//! Camera capture backends.

pub mod rpicam;
pub mod v4l2;

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use crate::config::{Config, SourceKind};
use crate::hub::FrameHub;

/// A source of JPEG frames that publishes into a [`FrameHub`] until it
/// errors or `shutdown` is cancelled.
///
/// Implementors hold only configuration (device path, resolution, ...), not
/// live handles - `run` opens whatever it needs each time it's called, which
/// is what lets [`supervise`] retry a failed backend by simply calling it
/// again on the same instance.
pub trait CaptureSource: Send + Sync {
    fn run<'a>(
        &'a self,
        hub: FrameHub,
        shutdown: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;
}

const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(10);
/// A run that lasted at least this long before failing is treated as having
/// been healthy, so the next retry isn't penalized by whatever backoff a
/// much earlier, unrelated failure had built up.
const HEALTHY_RUN_THRESHOLD: Duration = Duration::from_secs(30);

/// Runs `source` until `shutdown` is cancelled, restarting it with capped
/// exponential backoff whenever it errors out (or exits without being
/// asked to) - a camera hiccup or a killed subprocess shouldn't take the
/// whole server down.
pub async fn supervise(source: Box<dyn CaptureSource>, hub: FrameHub, shutdown: CancellationToken) {
    let mut backoff = INITIAL_BACKOFF;

    while !shutdown.is_cancelled() {
        let started = Instant::now();
        let result = source.run(hub.clone(), shutdown.clone()).await;

        if shutdown.is_cancelled() {
            return;
        }

        match result {
            Ok(()) => tracing::warn!("capture backend exited without an error; restarting"),
            Err(err) => tracing::error!(error = %err, "capture backend failed; restarting"),
        }

        if started.elapsed() >= HEALTHY_RUN_THRESHOLD {
            backoff = INITIAL_BACKOFF;
        }

        tracing::info!(delay = ?backoff, "retrying capture after backoff");
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = tokio::time::sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// Builds the capture source configured by `cfg`, resolving
/// [`SourceKind::Auto`] by probing `PATH` for `rpicam-vid`/`libcamera-vid`.
pub fn build(cfg: &Config) -> anyhow::Result<Box<dyn CaptureSource>> {
    let kind = match cfg.source {
        SourceKind::Auto => {
            if find_in_path("rpicam-vid").is_some() || find_in_path("libcamera-vid").is_some() {
                tracing::info!(
                    "found rpicam-vid/libcamera-vid on PATH, using the Pi CSI camera backend"
                );
                SourceKind::Rpicam
            } else {
                tracing::info!(
                    device = %cfg.device.display(),
                    "no rpicam-vid/libcamera-vid on PATH, falling back to the V4L2 backend"
                );
                SourceKind::V4l2
            }
        }
        other => other,
    };

    match kind {
        SourceKind::Rpicam => Ok(Box::new(rpicam::RpicamCapture::from_config(cfg)?)),
        SourceKind::V4l2 => Ok(Box::new(v4l2::V4l2Capture::from_config(cfg))),
        SourceKind::Auto => unreachable!("resolved above"),
    }
}

/// Searches `PATH` for an executable named `bin`, the same lookup a shell
/// does, without pulling in a `which` crate for it.
pub(crate) fn find_in_path(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        let candidate = dir.join(bin);
        is_executable(&candidate).then_some(candidate)
    })
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_known_executable_on_path() {
        // `sh` is about as safe a bet as exists on any Unix PATH.
        assert!(find_in_path("sh").is_some());
    }

    #[test]
    fn does_not_find_a_nonexistent_binary() {
        assert!(find_in_path("winnie-cam-definitely-not-a-real-binary").is_none());
    }
}
