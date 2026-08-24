//! Capture backend for the Raspberry Pi CSI camera module.
//!
//! On Pi OS Bookworm and later, CSI cameras are driven through libcamera;
//! `/dev/video0` for a CSI camera exposes raw Bayer data from unicam, not
//! JPEG, and there is no V4L2 path that yields a usable image. The
//! supported route is `rpicam-vid` (named `libcamera-vid` on Bullseye and
//! earlier), which drives the ISP and JPEG encoder and writes MJPEG to
//! stdout - so this backend just runs it as a subprocess and splits its
//! stdout into frames.
//!
//! Config changes are detected via [`SharedVideoConfig::changed`]: when it
//! fires the subprocess is killed, `run()` returns `Ok(())`, and
//! [`supervise`] restarts it immediately with the new settings.

use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{Context, anyhow};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use super::CaptureSource;
use crate::config::SharedVideoConfig;
use crate::hub::FrameHub;
use crate::jpeg::JpegSplitter;

/// Size of each stdout read. Large enough to keep up with 720p MJPEG at
/// 15fps comfortably without excessive syscall overhead.
const READ_CHUNK_SIZE: usize = 64 * 1024;

pub struct RpicamCapture {
    /// Resolved at construction time: `rpicam-vid` if present, else
    /// `libcamera-vid`. Re-resolved on every retry via `from_config`-style
    /// construction isn't needed since the binary doesn't move at runtime.
    binary: String,
    /// Runtime-changeable settings; [`SharedVideoConfig::changed`] signals
    /// the capture loop to restart with fresh values.
    video_config: Arc<SharedVideoConfig>,
}

impl RpicamCapture {
    pub fn from_config(video_config: Arc<SharedVideoConfig>) -> anyhow::Result<Self> {
        let settings = video_config.settings.read().unwrap();
        anyhow::ensure!(
            settings.width > 0 && settings.height > 0,
            "width/height must be > 0"
        );
        anyhow::ensure!(settings.fps > 0, "fps must be > 0");
        drop(settings);

        let binary = super::find_in_path("rpicam-vid")
            .map(|_| "rpicam-vid".to_string())
            .or_else(|| super::find_in_path("libcamera-vid").map(|_| "libcamera-vid".to_string()))
            .unwrap_or_else(|| "rpicam-vid".to_string());

        Ok(Self {
            binary,
            video_config,
        })
    }

    /// Populate `cmd` with the arguments for the current settings, so both
    /// the first launch and config-change restarts go through one code path.
    fn build_command(&self, cmd: &mut Command) {
        let s = self.video_config.settings.read().unwrap();
        cmd.arg("-t")
            .arg("0") // run indefinitely
            .arg("--codec")
            .arg("mjpeg")
            .arg("-o")
            .arg("-") // write to stdout
            .arg("-n") // no preview window
            .arg("--flush") // push each frame out immediately
            .arg("--width")
            .arg(s.width.to_string())
            .arg("--height")
            .arg(s.height.to_string())
            .arg("--framerate")
            .arg(s.fps.to_string())
            .arg("--quality")
            .arg(s.quality.to_string());
        if s.hflip {
            cmd.arg("--hflip");
        }
        if s.vflip {
            cmd.arg("--vflip");
        }
    }

    async fn run_impl(&self, hub: FrameHub, shutdown: CancellationToken) -> anyhow::Result<()> {
        let settings = self.video_config.settings.read().unwrap().clone();
        drop(settings); // release the read lock before we start spawning

        let mut cmd = Command::new(&self.binary);
        self.build_command(&mut cmd);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn {}", self.binary))?;
        let mut stdout = child.stdout.take().expect("stdout was piped");
        let stderr = child.stderr.take().expect("stderr was piped");

        // libcamera failures (camera not detected, ribbon cable seated
        // wrong, resolution unsupported, ...) only ever show up on stderr,
        // so log it rather than discard it.
        let binary_name = self.binary.clone();
        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::warn!(target: "rpicam-vid", binary = %binary_name, "{line}");
            }
        });

        let mut splitter = JpegSplitter::new();
        let mut buf = [0u8; READ_CHUNK_SIZE];

        let result = loop {
            tokio::select! {
                _ = shutdown.cancelled() => break Ok(()),
                _ = self.video_config.changed() => {
                    // Config was updated — kill the subprocess and return
                    // cleanly so `supervise` restarts us with fresh args.
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    stderr_task.abort();
                    return Ok(());
                }
                read = stdout.read(&mut buf) => {
                    match read {
                        Ok(0) => break Err(anyhow!("{} exited (stdout closed)", self.binary)),
                        Ok(n) => {
                            for frame in splitter.push(&buf[..n]) {
                                hub.publish(frame);
                            }
                        }
                        Err(e) => {
                            break Err(anyhow::Error::from(e)
                                .context(format!("reading {} stdout", self.binary)));
                        }
                    }
                }
            }
        };

        // Whatever path got us here, make sure the child doesn't outlive us.
        let _ = child.start_kill();
        let _ = child.wait().await;
        stderr_task.abort();

        result
    }
}

impl CaptureSource for RpicamCapture {
    fn run<'a>(
        &'a self,
        hub: FrameHub,
        shutdown: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(self.run_impl(hub, shutdown))
    }
}
