//! Motion events, built on top of person detection: a person box tells us
//! *where* to look, and a cheap downscaled frame-diff masked to that region
//! tells us whether it's moving. Turned into discrete `motion_started`/
//! `motion_stopped` events (`motion::detector::MotionDetector`) and fanned
//! out over `/events` (`motion::hub::MotionHub`).
//!
//! A submodule of `detect`, not a sibling of it: motion has no meaning
//! without a person box to mask against, so it belongs under the module
//! that produces one, not next to it. That nesting is also why the
//! `decode_rgb`/`Throttle`/`fps_to_interval` helpers this module reuses
//! (see `pump_loop`/`worker_loop` below) can stay private in `detect`'s
//! top level instead of needing `pub(crate)` - a private item is visible
//! to descendants of its defining module, and this module is one.
//!
//! There is no separate `--motion` flag - motion runs whenever `--detect`
//! does (see `MotionConfig::from_config`). `detect::start` spawns this
//! module from the very `DetectionHub` that spawning the detector just
//! produced, so the two literally cannot come apart at runtime.
//!
//! # Never subscribing to `DetectionHub`
//!
//! `detect::pump_loop` decides its own sample rate from
//! `DetectionHub::subscriber_count()` - any code that calls
//! `DetectionHub::subscribe()` pins detection to `--detect-fps` forever
//! (see that function's doc comment). This module instead calls
//! `DetectionHub::latest()`, a borrow-only peek that doesn't affect that
//! count, every time it samples a frame.
//!
//! # Same async-pump + OS-thread split as `detect`
//!
//! For the same reasons documented on `detect`'s module doc comment: JPEG
//! decode is blocking, the previous grid needs to persist across passes,
//! and a dedicated thread keeps this from sharing a blocking pool with the
//! V4L2 capture backend or the detector.

pub mod detector;
pub mod grid;
pub mod hub;

use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::config::Config;
use crate::hub::FrameHub;
use detector::{DebounceConfig, MotionDetector, MotionKind};
use hub::MotionHub;
// These three are private in `super` (`detect`) - reachable as `super::X`
// only because this module is nested inside it. See this module's doc
// comment for why that's deliberate, not a shortcut.
use super::hub::DetectionHub;
use super::nms::BBox;
use super::{Throttle, decode_rgb, fps_to_interval};

/// Motion settings extracted from [`Config`]. Like `detect::DetectConfig`,
/// kept separate from `Config` itself, which doesn't survive past startup.
#[derive(Clone, Debug)]
pub(super) struct MotionConfig {
    pub fps: f32,
    pub grid_side: u32,
    pub pixel_delta: u8,
    pub threshold: f32,
    pub sustain_ms: u64,
    pub quiet_ms: u64,
    pub cooldown_ms: u64,
    pub box_ttl_ms: u64,
    pub box_margin: f32,
}

impl MotionConfig {
    /// Unconditional: unlike `DetectConfig::from_config`, there's no
    /// "motion disabled" case to report here. `detect::start` is the
    /// only caller, and it only reaches this after already confirming
    /// `--detect` is on - motion has no separate switch, see this module's
    /// doc comment.
    pub(super) fn from_config(cfg: &Config) -> Self {
        Self {
            fps: cfg.motion_fps,
            grid_side: cfg.motion_grid,
            pixel_delta: cfg.motion_pixel_delta,
            threshold: cfg.motion_threshold,
            sustain_ms: cfg.motion_sustain_ms,
            quiet_ms: cfg.motion_quiet_ms,
            cooldown_ms: cfg.motion_cooldown_ms,
            box_ttl_ms: cfg.motion_box_ttl_ms,
            box_margin: cfg.motion_box_margin,
        }
    }

    fn debounce(&self) -> DebounceConfig {
        DebounceConfig {
            sustain: Duration::from_millis(self.sustain_ms),
            quiet: Duration::from_millis(self.quiet_ms),
            cooldown: Duration::from_millis(self.cooldown_ms),
        }
    }
}

/// Handle to a spawned motion pipeline: the async pump task and the OS
/// thread doing decode+diff. Mirrors `detect::DetectHandle` exactly,
/// including the join timeout - see that type's doc comments for why.
pub(super) struct MotionHandle {
    pump: JoinHandle<()>,
    worker: std::thread::JoinHandle<()>,
}

const WORKER_JOIN_TIMEOUT: Duration = Duration::from_secs(3);

impl MotionHandle {
    pub(super) async fn join(self) -> anyhow::Result<()> {
        self.pump.await?;

        // Deliberately a bare `std::thread`, not `spawn_blocking` - see
        // `detect::DetectHandle::join`'s doc comment for why.
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(self.worker.join());
        });
        match tokio::time::timeout(WORKER_JOIN_TIMEOUT, rx).await {
            Ok(Ok(result)) => {
                result.map_err(|_| anyhow::anyhow!("motion worker thread panicked"))?;
            }
            Ok(Err(_)) | Err(_) => tracing::warn!(
                timeout_secs = WORKER_JOIN_TIMEOUT.as_secs(),
                "motion worker didn't finish its in-flight pass in time; \
                 shutting down without waiting for it"
            ),
        }
        Ok(())
    }
}

/// Spawns the motion pump task and worker thread. Called from
/// `detect::start` with the same `DetectionHub` its own call to
/// `detect::spawn` just returned - see this module's doc comment on why
/// that pairing is enforced at the call site rather than checked here.
pub(super) fn spawn(
    cfg: MotionConfig,
    frame_hub: FrameHub,
    det_hub: DetectionHub,
    shutdown: CancellationToken,
) -> (MotionHub, MotionHandle) {
    let motion_hub = MotionHub::new();
    let (work_tx, work_rx) = mpsc::channel::<MotionInput>(1);

    let worker = {
        let hub = motion_hub.clone();
        let cfg = cfg.clone();
        std::thread::Builder::new()
            .name("winnie-motion".into())
            .spawn(move || worker_loop(cfg, hub, work_rx))
            .expect("spawning the motion worker thread")
    };

    let pump = tokio::spawn(pump_loop(cfg, frame_hub, det_hub, work_tx, shutdown));

    (motion_hub, MotionHandle { pump, worker })
}

/// What one sample hands to the worker thread: either a frame to actually
/// diff (a fresh-enough person box exists) or `NoSubject`, which still
/// ticks the debounce state machine so an in-flight episode can close out
/// cleanly instead of lingering once the person leaves frame.
enum MotionInput {
    Frame { jpeg: Bytes, boxes: Vec<BBox>, src_w: u32, src_h: u32 },
    NoSubject,
}

/// Async side: throttles frames from the [`FrameHub`] to `--motion-fps`,
/// pairs the newest one with the latest person box from `det_hub`, and
/// hands it to the worker thread without ever blocking on it. Mirrors
/// `detect::pump_loop`'s coalesce/throttle/try_send shape.
async fn pump_loop(
    cfg: MotionConfig,
    frame_hub: FrameHub,
    det_hub: DetectionHub,
    work_tx: mpsc::Sender<MotionInput>,
    shutdown: CancellationToken,
) {
    let mut rx = frame_hub.subscribe();
    let mut throttle = Throttle::new();
    let interval = fps_to_interval(cfg.fps);
    let box_ttl = Duration::from_millis(cfg.box_ttl_ms);

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            r = rx.recv() => match r {
                Ok(mut frame) => {
                    // Coalesce to the newest frame, same as detect::pump_loop.
                    loop {
                        match rx.try_recv() {
                            Ok(f) => frame = f,
                            Err(TryRecvError::Lagged(_)) => continue,
                            Err(_) => break,
                        }
                    }

                    if !throttle.should_run(Instant::now(), interval) {
                        continue;
                    }

                    // Borrow-only: see this module's doc comment on why
                    // this must never be `det_hub.subscribe()`.
                    let latest = det_hub.latest();
                    let has_subject =
                        latest.seq > 0 && !latest.boxes.is_empty() && latest.age() <= box_ttl;
                    let input = if has_subject {
                        MotionInput::Frame {
                            jpeg: frame,
                            boxes: latest.boxes.clone(),
                            src_w: latest.src_w,
                            src_h: latest.src_h,
                        }
                    } else {
                        MotionInput::NoSubject
                    };

                    // Non-blocking hand-off, identical rationale to
                    // detect::pump_loop: if the worker is still busy, this
                    // sample is simply dropped.
                    match work_tx.try_send(input) {
                        Ok(()) => {}
                        Err(TrySendError::Full(_)) => {
                            tracing::trace!("motion worker still busy; skipping sample");
                        }
                        Err(TrySendError::Closed(_)) => {
                            tracing::warn!("motion worker is gone; stopping the pump");
                            break;
                        }
                    }
                }
                Err(RecvError::Lagged(n)) => tracing::trace!(n, "motion pump fell behind capture"),
                Err(RecvError::Closed) => break,
            }
        }
    }
    // Dropping work_tx here is what tells worker_loop to stop.
}

/// Blocking side: owns the previous sample's grid and the debounce state
/// machine, and runs one decode+diff pass per input handed to it.
fn worker_loop(cfg: MotionConfig, hub: MotionHub, mut work_rx: mpsc::Receiver<MotionInput>) {
    let mut rgb: Vec<u8> = Vec::new();
    let mut prev_grid: Vec<u8> = Vec::new();
    let mut cur_grid: Vec<u8> = Vec::new();
    let mut have_baseline = false;
    let mut last_sample_at: Option<Instant> = None;
    // A gap in samples this long (camera reconnect, or an extended stretch
    // with nothing in frame) makes the previous grid meaningless to diff
    // against - the next real frame becomes a fresh baseline instead of
    // being scored, so a stale baseline can't produce a false spike.
    let stale_after = fps_to_interval(cfg.fps).saturating_mul(3);
    let mut detector = MotionDetector::new(cfg.debounce());

    while let Some(input) = work_rx.blocking_recv() {
        let now = Instant::now();
        let stale = match last_sample_at {
            Some(t) => now.duration_since(t) > stale_after,
            None => true,
        };
        last_sample_at = Some(now);

        match input {
            MotionInput::NoSubject => {
                if let Some(kind) = detector.close(now) {
                    emit(&hub, kind, 0.0);
                }
                // The next real frame gets to set a fresh baseline rather
                // than diff against whatever was in frame before the
                // subject left.
                have_baseline = false;
            }
            MotionInput::Frame { jpeg, boxes, src_w, src_h } => {
                let (w, h) = match decode_rgb(&jpeg, &mut rgb) {
                    Ok(dims) => dims,
                    Err(err) => {
                        tracing::warn!(error = ?err, "motion frame decode failed");
                        continue;
                    }
                };
                grid::downscale(&rgb, w, h, cfg.grid_side, &mut cur_grid);

                if stale || !have_baseline {
                    std::mem::swap(&mut prev_grid, &mut cur_grid);
                    have_baseline = true;
                    continue;
                }

                let mask = grid::mask(&boxes, src_w, src_h, cfg.grid_side, cfg.box_margin);
                let sample = grid::score(&prev_grid, &cur_grid, &mask, cfg.pixel_delta);
                std::mem::swap(&mut prev_grid, &mut cur_grid);

                let (moving, score) = match sample {
                    // Global-lighting guard: auto-exposure hunting and IR
                    // cut-filter switching light up the *whole* frame, not
                    // just the masked region - real movement shouldn't
                    // light up the unmasked region nearly as much.
                    Some(s) => (s.inside >= cfg.threshold && s.outside < s.inside * 0.6, s.inside),
                    None => (false, 0.0),
                };

                if let Some(kind) = detector.observe(now, moving) {
                    emit(&hub, kind, score);
                }
            }
        }
    }
    tracing::info!("motion worker shutting down");
}

fn emit(hub: &MotionHub, kind: MotionKind, score: f32) {
    match kind {
        MotionKind::Started => tracing::info!(score, "motion started"),
        MotionKind::Stopped { duration_ms } => tracing::info!(duration_ms, score, "motion stopped"),
    }
    hub.publish(kind, score);
}
