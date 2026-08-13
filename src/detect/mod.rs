//! Person detection: decodes captured JPEG frames, runs them through a
//! YOLOv8/YOLO11 ONNX model via `tract`, and publishes normalized boxes for
//! the live overlay.
//!
//! # Why detection can never stall capture
//!
//! [`crate::hub::FrameHub::publish`] sends on a `broadcast` channel, which
//! is non-blocking by construction - a slow subscriber just falls behind
//! and drops frames (`RecvError::Lagged`). So simply *subscribing* normally
//! already makes this module structurally incapable of blocking capture.
//! On top of that, this module runs its own throttle (only look at a frame
//! every so often) and coalescing (skip straight to the newest frame when
//! catching up), and hands work to a worker thread through a
//! capacity-1 channel with `try_send` - if the worker is still busy on the
//! previous frame, the new one is simply dropped rather than queued.
//!
//! # Why a dedicated OS thread, not `spawn_blocking`
//!
//! The worker keeps its decode/tensor scratch buffers alive across passes
//! (nothing reallocates per frame), and `spawn_blocking`'s thread pool is
//! already used by the V4L2 capture backend (`src/capture/v4l2.rs`) - a
//! dedicated thread keeps the two blocking workloads from ever interacting
//! through a shared pool.
//!
//! # Idle vs active throttling
//!
//! Detection runs continuously once `--detect` is passed - even with no
//! viewer watching - so there's a standing signal to build alerting on. In
//! fact `--detect` now drives exactly that: the `motion` module reads the
//! latest person box off [`hub::DetectionHub::latest`] to power motion
//! events (`/events`), so the idle rate below isn't just a CPU-saving
//! measure anymore - it's also what keeps a person box fresh enough for
//! motion to mask against. Detection runs much slower when nobody has the
//! overlay open (`--detect-idle-fps`, default 0.2/s) and speeds up to
//! `--detect-fps` only while at least one `/detections` SSE connection is
//! open. Sustained inference is this app's dominant CPU/heat cost on a Pi,
//! so idling it still matters.

pub mod hub;
pub mod letterbox;
pub mod model;
pub mod nms;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tract_onnx::prelude::{IntoTValue, Tensor};

use crate::config::Config;
use crate::hub::FrameHub;
use hub::{DetectState, DetectionHub};
use letterbox::Letterbox;
use model::PERSON_CLASS;

/// Detection settings extracted from [`Config`]. Kept separate because
/// `Config` itself doesn't survive past startup (see `src/main.rs`), while
/// this needs to live on in the worker thread and the pump task.
#[derive(Clone, Debug)]
pub struct DetectConfig {
    pub model_path: PathBuf,
    pub side: u32,
    pub active_fps: f32,
    pub idle_fps: f32,
    pub threshold: f32,
    pub iou: f32,
}

impl DetectConfig {
    /// `None` if `--detect` wasn't passed; an error if it was passed
    /// without `--model` (clap's `requires = "model"` on `--detect` already
    /// prevents this at parse time, but a config built by hand - e.g. in a
    /// test - might not go through clap).
    pub fn from_config(cfg: &Config) -> anyhow::Result<Option<Self>> {
        if !cfg.detect {
            return Ok(None);
        }
        let model_path = cfg
            .model
            .clone()
            .ok_or_else(|| anyhow::anyhow!("--detect requires --model <path to .onnx>"))?;
        Ok(Some(Self {
            model_path,
            side: cfg.detect_size,
            active_fps: cfg.detect_fps,
            idle_fps: cfg.detect_idle_fps,
            threshold: cfg.detect_threshold,
            iou: cfg.detect_iou,
        }))
    }
}

/// Handle to a spawned detector: the async pump task and the OS thread
/// doing decode+inference. Both must be joined for a clean shutdown.
pub struct DetectHandle {
    pump: JoinHandle<()>,
    worker: std::thread::JoinHandle<()>,
}

/// How long shutdown waits for an in-flight inference pass to finish on its
/// own before giving up on a clean join. Generous for a release build on
/// even a slow Pi (see the README's latency table), but nowhere near
/// `tract`'s debug-build slowdown (30-100x - a 300ms pass becomes 20-30s),
/// so a debug build with `--detect` still shuts down promptly instead of
/// making `Ctrl-C` look hung.
const WORKER_JOIN_TIMEOUT: Duration = Duration::from_secs(3);

impl DetectHandle {
    /// Waits for both halves to finish, up to [`WORKER_JOIN_TIMEOUT`] for
    /// the worker thread. If that elapses, logs a warning and returns
    /// anyway rather than blocking process exit - the worker holds no
    /// external state (no open files, no locks anything else needs) that a
    /// clean finish would protect, so abandoning it mid-pass is safe; the
    /// OS reclaims the thread when the process exits.
    pub async fn join(self) -> anyhow::Result<()> {
        self.pump.await?;

        // Deliberately a bare `std::thread`, not `spawn_blocking`: tokio's
        // `Runtime::drop` blocks indefinitely for outstanding blocking
        // tasks, so a `spawn_blocking` join here would still hang the
        // process for the full in-flight pass once `#[tokio::main]` drops
        // the runtime after this function returns - the exact hang
        // `WORKER_JOIN_TIMEOUT` exists to bound. A detached `std::thread`
        // is invisible to that, so the timeout below actually bounds
        // shutdown; the thread itself is simply abandoned to the OS
        // (see the doc comment above for why that's safe).
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(self.worker.join());
        });
        match tokio::time::timeout(WORKER_JOIN_TIMEOUT, rx).await {
            Ok(Ok(result)) => {
                result.map_err(|_| anyhow::anyhow!("detection worker thread panicked"))?;
            }
            // The joiner thread hung up without sending - can only happen
            // if it panicked, which `std::thread::JoinHandle::join` itself
            // doesn't do; treated the same as a timeout.
            Ok(Err(_)) | Err(_) => tracing::warn!(
                timeout_secs = WORKER_JOIN_TIMEOUT.as_secs(),
                "detection worker didn't finish its in-flight pass in time; \
                 shutting down without waiting for it"
            ),
        }
        Ok(())
    }
}

/// Spawns the detection pump task and worker thread. Returns immediately -
/// model loading happens on the worker thread so the HTTP listener and
/// video stream are never delayed by it (see `worker_loop`).
///
/// Returns the [`DetectionHub`] (for the web layer to hold in `AppState`
/// and hand out to `/detections` subscribers) alongside the [`DetectHandle`]
/// (for `main` to join at shutdown).
pub fn spawn(
    cfg: DetectConfig,
    hub: FrameHub,
    shutdown: CancellationToken,
) -> (DetectionHub, DetectHandle) {
    let det_hub = DetectionHub::new();
    let (work_tx, work_rx) = mpsc::channel::<Bytes>(1);

    let worker = {
        let det_hub = det_hub.clone();
        let cfg = cfg.clone();
        std::thread::Builder::new()
            .name("winnie-detect".into())
            .spawn(move || worker_loop(cfg, det_hub, work_rx))
            .expect("spawning the detection worker thread")
    };

    let pump = tokio::spawn(pump_loop(cfg, hub, det_hub.clone(), work_tx, shutdown));

    (det_hub, DetectHandle { pump, worker })
}

/// Async side: throttles and coalesces frames from the [`FrameHub`], and
/// hands the newest one to the worker thread without ever blocking on it.
async fn pump_loop(
    cfg: DetectConfig,
    hub: FrameHub,
    det_hub: DetectionHub,
    work_tx: mpsc::Sender<Bytes>,
    shutdown: CancellationToken,
) {
    let mut rx = hub.subscribe();
    let mut throttle = Throttle::new();
    let active_iv = fps_to_interval(cfg.active_fps);
    let idle_iv = fps_to_interval(cfg.idle_fps);

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            r = rx.recv() => match r {
                Ok(mut frame) => {
                    // Coalesce: only the newest frame is worth inferring on.
                    loop {
                        match rx.try_recv() {
                            Ok(f) => frame = f,
                            Err(TryRecvError::Lagged(_)) => continue,
                            Err(_) => break,
                        }
                    }

                    let iv = if det_hub.subscriber_count() > 0 { active_iv } else { idle_iv };
                    if !throttle.should_run(Instant::now(), iv) {
                        continue;
                    }

                    // Non-blocking hand-off: if the worker is mid-inference,
                    // this frame is simply dropped, so detection latency
                    // never compounds.
                    match work_tx.try_send(frame) {
                        Ok(()) => {}
                        Err(TrySendError::Full(_)) => {
                            tracing::trace!("detector still busy; skipping frame");
                        }
                        // The worker thread returned (most commonly: model
                        // load failed) and dropped its receiver. Without this
                        // arm the match above never matches `Closed`, so the
                        // pump would spin forever on every capture frame
                        // instead of noticing the worker is gone. Mark
                        // detection as errored too, in case the worker died
                        // *after* reaching `Ready` (e.g. a panic mid-pass) -
                        // otherwise `/healthz` and the overlay toggle would
                        // keep reporting detection as working.
                        Err(TrySendError::Closed(_)) => {
                            tracing::warn!("detection worker is gone; stopping the pump");
                            det_hub.set_state(DetectState::Error);
                            break;
                        }
                    }
                }
                Err(RecvError::Lagged(n)) => tracing::trace!(n, "detector fell behind capture"),
                Err(RecvError::Closed) => break,
            }
        }
    }
    // Dropping work_tx here (end of scope) is what tells worker_loop to
    // stop: its `blocking_recv()` returns `None` once the channel closes.
}

/// Blocking side: owns the tract plan and reusable scratch buffers, and
/// runs one decode+infer pass per frame handed to it.
fn worker_loop(cfg: DetectConfig, det_hub: DetectionHub, mut work_rx: mpsc::Receiver<Bytes>) {
    #[cfg(debug_assertions)]
    tracing::warn!(
        "detection is enabled in a DEBUG build; tract will be 30-100x slower than a \
         release build and inference may appear to hang. Use `cargo build --release`."
    );

    let plan = match model::load(&cfg.model_path, cfg.side) {
        Ok(p) => {
            tracing::info!(model = %cfg.model_path.display(), side = cfg.side, "detection model ready");
            det_hub.set_state(DetectState::Ready);
            p
        }
        Err(err) => {
            tracing::error!(
                error = ?err,
                model = %cfg.model_path.display(),
                "failed to load detection model; detection disabled (video is unaffected)"
            );
            det_hub.set_state(DetectState::Error);
            return;
        }
    };

    let mut rgb: Vec<u8> = Vec::new();
    let side = cfg.side as usize;
    // Allocated once and reused every pass via `Arc::get_mut` in
    // `run_one_pass` - see that function's doc comment.
    let mut scratch: Arc<Tensor> = match Tensor::zero::<f32>(&[1, 3, side, side]) {
        Ok(t) => Arc::new(t),
        Err(err) => {
            tracing::error!(error = ?err, "failed to allocate detection input tensor");
            det_hub.set_state(DetectState::Error);
            return;
        }
    };

    while let Some(jpeg) = work_rx.blocking_recv() {
        let started = Instant::now();
        match run_one_pass(&plan, &cfg, &jpeg, &mut rgb, &mut scratch) {
            Ok((src_w, src_h, boxes)) => {
                let ms = started.elapsed().as_secs_f32() * 1000.0;
                tracing::debug!(count = boxes.len(), ms, "detection pass complete");
                det_hub.publish(src_w, src_h, &boxes, ms);
            }
            Err(err) => tracing::warn!(error = ?err, "detection pass failed"),
        }
    }
    tracing::info!("detection worker shutting down");
}

/// Decode -> letterbox -> tensor fill -> inference -> NMS, for one frame.
/// `rgb`/`scratch` are reused across calls so nothing allocates per pass.
fn run_one_pass(
    plan: &model::Plan,
    cfg: &DetectConfig,
    jpeg: &[u8],
    rgb: &mut Vec<u8>,
    scratch: &mut Arc<Tensor>,
) -> anyhow::Result<(u32, u32, Vec<nms::BBox>)> {
    let (src_w, src_h) = decode_rgb(jpeg, rgb)?;
    let lb = Letterbox::new(src_w, src_h, cfg.side);

    // Fill `scratch` in place rather than building a fresh Tensor from
    // `nchw` every pass. `Arc::get_mut` only returns `None` if something
    // else is still holding a clone of the *previous* pass's input (tract
    // shouldn't retain one past `plan.run` returning, but this falls back
    // to a fresh allocation instead of panicking if that's ever untrue).
    let side = cfg.side as usize;
    {
        let tensor = match Arc::get_mut(scratch) {
            Some(t) => t,
            None => {
                *scratch = Arc::new(Tensor::zero::<f32>(&[1, 3, side, side])?);
                Arc::get_mut(scratch).expect("just replaced with a fresh, uniquely-owned Arc")
            }
        };
        lb.fill_tensor(rgb, tensor.as_slice_mut::<f32>()?);
    }

    let model_boxes = model::infer(plan, scratch.clone().into_tvalue(), PERSON_CLASS, cfg.threshold)?;
    let suppressed = nms::nms(model_boxes, cfg.iou, 50);

    // Map each surviving box from model-input pixels back to source-frame
    // pixels before NMS's caller-facing output; hub::publish normalizes
    // those to 0..1 fractions for the wire format.
    let mapped = suppressed
        .into_iter()
        .map(|b| {
            let (x1, y1) = lb.to_source(b.x1, b.y1);
            let (x2, y2) = lb.to_source(b.x2, b.y2);
            nms::BBox { x1, y1, x2, y2, score: b.score }
        })
        .collect();

    Ok((src_w, src_h, mapped))
}

/// Decodes a JPEG into `rgb` (resized in place as needed) and returns
/// `(width, height)`. `pub(crate)`: also used by `motion::worker_loop`,
/// which needs the same decode without the letterbox/tensor steps below.
pub(crate) fn decode_rgb(jpeg: &[u8], rgb: &mut Vec<u8>) -> anyhow::Result<(u32, u32)> {
    use zune_jpeg::JpegDecoder;
    use zune_jpeg::zune_core::bytestream::ZCursor;

    let mut decoder = JpegDecoder::new(ZCursor::new(jpeg));
    decoder
        .decode_headers()
        .map_err(|e| anyhow::anyhow!("reading JPEG headers: {e}"))?;
    let (w, h) = decoder
        .dimensions()
        .ok_or_else(|| anyhow::anyhow!("JPEG dimensions unavailable after decoding headers"))?;
    let size = decoder
        .output_buffer_size()
        .ok_or_else(|| anyhow::anyhow!("JPEG output size unavailable after decoding headers"))?;

    if rgb.len() != size {
        rgb.resize(size, 0);
    }
    decoder
        .decode_into(rgb)
        .map_err(|e| anyhow::anyhow!("decoding JPEG: {e}"))?;

    Ok((w as u32, h as u32))
}

/// `pub(crate)`: also used by `motion::pump_loop` for its own `--motion-fps`
/// throttle.
pub(crate) fn fps_to_interval(fps: f32) -> Duration {
    Duration::from_secs_f32(1.0 / fps.max(0.01))
}

/// Schedules "should I run now" decisions at a given interval, without
/// sleeping - the caller supplies `now` and the interval each call, so this
/// is trivially unit-testable.
///
/// The deadline is recomputed from `last_ran + interval` on every call
/// using *that call's* interval, rather than being fixed in advance. That's
/// what lets the pump switch between the idle and active rates (see this
/// module's doc comment) and have it take effect on the very next tick,
/// instead of waiting out whatever interval happened to be in effect when
/// the previous pass was scheduled.
///
/// `pub(crate)`: `motion::pump_loop` reuses this same throttle shape for its
/// own sampling rate.
pub(crate) struct Throttle {
    last_ran: Option<Instant>,
}

impl Throttle {
    pub(crate) fn new() -> Self {
        Self { last_ran: None }
    }

    pub(crate) fn should_run(&mut self, now: Instant, interval: Duration) -> bool {
        match self.last_ran {
            Some(t) if now < t + interval => false,
            // Recording `now`, not `t + interval`: after a long stall (e.g.
            // no frames while the camera reconnects) this makes the next
            // deadline `now + interval` again, so we get exactly one pass,
            // not a burst catching up on every missed slot.
            _ => {
                self.last_ran = Some(now);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_call_always_runs() {
        let mut t = Throttle::new();
        assert!(t.should_run(Instant::now(), Duration::from_secs(1)));
    }

    #[test]
    fn a_second_call_within_the_interval_is_skipped() {
        let mut t = Throttle::new();
        let start = Instant::now();
        assert!(t.should_run(start, Duration::from_secs(1)));
        assert!(!t.should_run(start + Duration::from_millis(500), Duration::from_secs(1)));
    }

    #[test]
    fn a_call_after_the_interval_runs() {
        let mut t = Throttle::new();
        let start = Instant::now();
        assert!(t.should_run(start, Duration::from_secs(1)));
        assert!(t.should_run(start + Duration::from_millis(1100), Duration::from_secs(1)));
    }

    #[test]
    fn does_not_burst_after_a_long_stall() {
        let mut t = Throttle::new();
        let start = Instant::now();
        assert!(t.should_run(start, Duration::from_secs(1)));
        // Jump forward by 10 missed intervals - only one more run should fire.
        let later = start + Duration::from_secs(11);
        assert!(t.should_run(later, Duration::from_secs(1)));
        assert!(!t.should_run(later, Duration::from_secs(1)));
    }

    #[test]
    fn switching_to_a_shorter_interval_takes_effect_immediately() {
        let mut t = Throttle::new();
        let start = Instant::now();
        assert!(t.should_run(start, Duration::from_secs(10))); // sets next_at = start + 10s
        // Switch to a 1s (active) interval - a call 2s later should run,
        // even though the old 10s schedule wouldn't have allowed it yet.
        assert!(t.should_run(start + Duration::from_secs(2), Duration::from_secs(1)));
    }
}
