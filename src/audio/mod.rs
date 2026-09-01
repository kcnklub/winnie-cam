//! Microphone capture and fan-out.
//!
//! Structurally parallel to [`crate::capture`] - a source trait, a driver,
//! a hub - but with one deliberate difference: video runs from boot whether
//! or not anyone is watching, and audio does not. The microphone is opened
//! only while somebody is listening, because on a Pi that is already
//! spending its CPU budget on ONNX inference there is no reason to encode
//! Opus into a channel with no receivers.
//!
//! ```text
//!  USB mic ──> ffmpeg (alsa in, Opus/WebM out) ──> WebmChunker
//!                                                       │
//!                                           Init(Bytes) / Cluster(Bytes)
//!                                                       v
//!                                                   AudioHub
//!                                       (watch<init> + broadcast<cluster>)
//!                                                       │
//!                                              GET /audio (chunked)
//!                                                       v
//!                                             <audio> in the browser
//! ```
//!
//! Audio is a separate transport from video, not a second track of it:
//! `multipart/x-mixed-replace` carries no audio, and an `<img>` cannot play
//! sound. The two streams are therefore not synchronized, which for a baby
//! monitor is not worth the machinery it would cost to fix.

pub mod alsa;
pub mod hub;
pub mod webm;

use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use crate::config::Config;
use hub::AudioHub;

/// A source of encoded audio that publishes into an [`AudioHub`] until it
/// errors or `shutdown` is cancelled.
///
/// Mirrors [`crate::capture::CaptureSource`]: implementors hold only
/// configuration, and `run` opens whatever it needs each time it is called,
/// so [`supervise_on_demand`] can start and stop the same instance
/// repeatedly as listeners come and go.
pub trait AudioSource: Send + Sync {
    fn run<'a>(
        &'a self,
        hub: AudioHub,
        shutdown: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;
}

const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(10);
/// A run that lasted at least this long before failing is treated as having
/// been healthy, so the next retry isn't penalized by an unrelated earlier
/// failure's accumulated backoff.
const HEALTHY_RUN_THRESHOLD: Duration = Duration::from_secs(30);
/// How long the microphone stays open after the last listener leaves.
/// Reloading the page or flipping the Listen button off and on again should
/// not cost a subprocess restart, and ALSA devices are slow to reopen.
const LINGER: Duration = Duration::from_secs(5);

/// Builds the audio source, or `None` when `--audio` wasn't passed.
///
/// Fails rather than returning `None` if audio was asked for but can't be
/// provided (no `ffmpeg`, nonsense rate/bitrate): a silently missing baby
/// monitor microphone is worse than a startup error.
pub fn build(cfg: &Config) -> anyhow::Result<Option<Box<dyn AudioSource>>> {
    if !cfg.audio {
        return Ok(None);
    }

    let source = alsa::AlsaCapture::from_config(cfg)?;
    tracing::info!(
        device = %cfg.audio_device,
        format = ?cfg.audio_format,
        "audio enabled; microphone opens on the first listener"
    );

    Ok(Some(Box::new(source)))
}

/// Runs `source` for exactly as long as somebody is listening, restarting it
/// with capped exponential backoff if it fails while listeners remain.
pub async fn supervise_on_demand(
    source: Box<dyn AudioSource>,
    hub: AudioHub,
    shutdown: CancellationToken,
) {
    let mut backoff = INITIAL_BACKOFF;

    while !shutdown.is_cancelled() {
        if !wait_for_listener(&hub, &shutdown).await {
            return;
        }

        let started = Instant::now();

        // Racing the run against silence is what makes this on-demand:
        // when the last listener leaves, the run future is dropped, and the
        // subprocess dies with it via `kill_on_drop`.
        let result = tokio::select! {
            result = source.run(hub.clone(), shutdown.clone()) => Some(result),
            _ = wait_for_silence(&hub) => None,
        };
        hub.end_stream();

        if shutdown.is_cancelled() {
            return;
        }

        match result {
            None => {
                tracing::info!("last listener left; microphone closed");
                backoff = INITIAL_BACKOFF;
                continue;
            }
            Some(Ok(())) => return,
            Some(Err(err)) => tracing::error!(error = %err, "audio backend failed"),
        }

        if started.elapsed() >= HEALTHY_RUN_THRESHOLD {
            backoff = INITIAL_BACKOFF;
        }

        tracing::info!(delay = ?backoff, "retrying audio after backoff");
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = tokio::time::sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// Resolves once at least one listener is connected. Returns `false` if
/// shutdown was requested first.
async fn wait_for_listener(hub: &AudioHub, shutdown: &CancellationToken) -> bool {
    let mut listeners = hub.listeners();

    loop {
        if *listeners.borrow_and_update() > 0 {
            return true;
        }

        tokio::select! {
            _ = shutdown.cancelled() => return false,
            changed = listeners.changed() => {
                if changed.is_err() {
                    return false;
                }
            }
        }
    }
}

/// Resolves once the listener count has been zero for a full [`LINGER`].
async fn wait_for_silence(hub: &AudioHub) {
    let mut listeners = hub.listeners();

    loop {
        while *listeners.borrow_and_update() > 0 {
            if listeners.changed().await.is_err() {
                return;
            }
        }

        // Nobody is listening. Give a reconnecting browser a moment to come
        // back before tearing the microphone down.
        tokio::select! {
            _ = tokio::time::sleep(LINGER) => return,
            changed = listeners.changed() => {
                if changed.is_err() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A source that records how many times it was started and then streams
    /// until asked to stop, so tests can observe the supervisor's start/stop
    /// decisions without a real subprocess.
    ///
    /// Honoring `shutdown` is not just realism - it is what the supervisor's
    /// `select!` needs in order to ever finish while a listener is still
    /// connected, exactly as [`alsa::AlsaCapture`] does.
    struct CountingSource {
        runs: Arc<AtomicUsize>,
    }

    impl AudioSource for CountingSource {
        fn run<'a>(
            &'a self,
            hub: AudioHub,
            shutdown: CancellationToken,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
            self.runs.fetch_add(1, Ordering::Relaxed);
            Box::pin(async move {
                hub.start_stream(Bytes::from_static(b"init"));
                shutdown.cancelled().await;
                Ok(())
            })
        }
    }

    #[tokio::test(start_paused = true)]
    async fn does_not_start_until_a_listener_connects() {
        let hub = AudioHub::new();
        let runs = Arc::new(AtomicUsize::new(0));
        let shutdown = CancellationToken::new();

        let task = tokio::spawn(supervise_on_demand(
            Box::new(CountingSource { runs: runs.clone() }),
            hub.clone(),
            shutdown.clone(),
        ));

        tokio::time::sleep(Duration::from_secs(60)).await;
        assert_eq!(runs.load(Ordering::Relaxed), 0);
        assert_eq!(hub.state(), hub::AudioState::Idle);

        let guard = hub.listen();
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(runs.load(Ordering::Relaxed), 1);
        assert_eq!(hub.state(), hub::AudioState::Streaming);

        drop(guard);
        shutdown.cancel();
        let _ = task.await;
    }

    #[tokio::test(start_paused = true)]
    async fn stops_after_the_last_listener_leaves() {
        let hub = AudioHub::new();
        let runs = Arc::new(AtomicUsize::new(0));
        let shutdown = CancellationToken::new();

        let task = tokio::spawn(supervise_on_demand(
            Box::new(CountingSource { runs: runs.clone() }),
            hub.clone(),
            shutdown.clone(),
        ));

        let guard = hub.listen();
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(hub.state(), hub::AudioState::Streaming);

        drop(guard);

        // Still running during the linger window.
        tokio::time::sleep(LINGER / 2).await;
        assert_eq!(hub.state(), hub::AudioState::Streaming);

        tokio::time::sleep(LINGER).await;
        assert_eq!(hub.state(), hub::AudioState::Idle);
        assert_eq!(runs.load(Ordering::Relaxed), 1, "should not have restarted");

        shutdown.cancel();
        let _ = task.await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_reconnect_within_the_linger_window_keeps_the_microphone_open() {
        let hub = AudioHub::new();
        let runs = Arc::new(AtomicUsize::new(0));
        let shutdown = CancellationToken::new();

        let task = tokio::spawn(supervise_on_demand(
            Box::new(CountingSource { runs: runs.clone() }),
            hub.clone(),
            shutdown.clone(),
        ));

        let guard = hub.listen();
        tokio::time::sleep(Duration::from_secs(1)).await;
        drop(guard);

        tokio::time::sleep(LINGER / 2).await;
        let _guard = hub.listen();
        tokio::time::sleep(LINGER * 2).await;

        assert_eq!(hub.state(), hub::AudioState::Streaming);
        assert_eq!(runs.load(Ordering::Relaxed), 1, "should not have restarted");

        shutdown.cancel();
        let _ = task.await;
    }
}
