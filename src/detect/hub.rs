//! Publishes the latest detection pass to any number of overlay viewers.
//!
//! This is deliberately not [`crate::hub::FrameHub`]'s `broadcast` channel:
//! detections are a "latest value" thing, not a stream nobody can afford to
//! miss a frame of, so a `watch` channel is the better fit - a new viewer
//! gets the current result immediately instead of waiting for the next pass.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

use tokio::sync::watch;

use crate::detect::nms::BBox;

/// Lifecycle state, surfaced on `/healthz` so the frontend knows whether to
/// show the overlay toggle at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetectState {
    /// Model is still loading/optimizing on the worker thread.
    Loading,
    Ready,
    /// The model failed to load; detection is permanently disabled for this
    /// run, but video is unaffected.
    Error,
}

impl DetectState {
    pub fn as_str(self) -> &'static str {
        match self {
            DetectState::Loading => "loading",
            DetectState::Ready => "ready",
            DetectState::Error => "error",
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            1 => DetectState::Ready,
            2 => DetectState::Error,
            _ => DetectState::Loading,
        }
    }
}

/// One detection pass's result, pre-serialized so publishing and fanning
/// out to N SSE viewers doesn't re-encode JSON per viewer. Doubles as the
/// backing store for the `/healthz` timing fields, via [`DetectionHub`]'s
/// `last_inference_ms`/`seconds_since_last_pass` - `at`/`inference_ms` are
/// read directly off whatever the watch channel currently holds instead of
/// being tracked a second time.
pub struct DetectionFrame {
    pub seq: u64,
    pub json: String,
    pub inference_ms: f32,
    at: Instant,
}

struct Inner {
    tx: watch::Sender<Arc<DetectionFrame>>,
    state: AtomicU8,
}

/// Cheaply cloneable handle to the shared detection result.
#[derive(Clone)]
pub struct DetectionHub {
    inner: Arc<Inner>,
}

impl DetectionHub {
    pub fn new() -> Self {
        let (tx, _) = watch::channel(Arc::new(DetectionFrame {
            seq: 0,
            json: empty_json(),
            inference_ms: 0.0,
            at: Instant::now(),
        }));
        Self {
            inner: Arc::new(Inner {
                tx,
                state: AtomicU8::new(0),
            }),
        }
    }

    pub fn set_state(&self, state: DetectState) {
        let v = match state {
            DetectState::Loading => 0,
            DetectState::Ready => 1,
            DetectState::Error => 2,
        };
        self.inner.state.store(v, Ordering::Relaxed);
    }

    pub fn state(&self) -> DetectState {
        DetectState::from_u8(self.inner.state.load(Ordering::Relaxed))
    }

    /// Publish one pass's boxes, already in source-frame pixel coordinates
    /// (`src_w`/`src_h` give the frame those pixels are relative to) -
    /// serialization normalizes and clamps them, see
    /// [`detections_json`].
    pub fn publish(&self, src_w: u32, src_h: u32, boxes: &[BBox], inference_ms: f32) {
        // seq is derived from the previous published value rather than a
        // separate atomic counter - one less piece of state to keep in
        // sync, and there's no concurrent-publisher case to race on: only
        // the single detection worker thread ever calls `publish`.
        let seq = self.inner.tx.borrow().seq + 1;
        let json = detections_json(src_w, src_h, seq, inference_ms, boxes);
        let frame = Arc::new(DetectionFrame {
            seq,
            json,
            inference_ms,
            at: Instant::now(),
        });
        // send_replace, not send: stores the value even with zero
        // receivers, so a viewer connecting between passes still gets the
        // most recent result instead of nothing.
        self.inner.tx.send_replace(frame);
    }

    pub fn subscribe(&self) -> watch::Receiver<Arc<DetectionFrame>> {
        self.inner.tx.subscribe()
    }

    pub fn subscriber_count(&self) -> usize {
        self.inner.tx.receiver_count()
    }

    /// `None` until the first pass has actually run (seq 0 is a placeholder
    /// published by `new`, not a real pass).
    pub fn last_inference_ms(&self) -> Option<f32> {
        let frame = self.inner.tx.borrow();
        (frame.seq > 0).then_some(frame.inference_ms)
    }

    pub fn seconds_since_last_pass(&self) -> Option<f64> {
        let frame = self.inner.tx.borrow();
        (frame.seq > 0).then(|| frame.at.elapsed().as_secs_f64())
    }
}

impl Default for DetectionHub {
    fn default() -> Self {
        Self::new()
    }
}

/// Formats a 0..1 fraction safely. JSON has no NaN/Infinity literal, and
/// `format!("{:.4}", f32::NAN)` prints the bare word `NaN` - one degenerate
/// box would break `JSON.parse` for every viewer, so every number that
/// reaches the wire goes through this first.
fn frac(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

fn empty_json() -> String {
    "{\"w\":0,\"h\":0,\"seq\":0,\"ms\":0.0,\"dets\":[]}".to_string()
}

/// Hand-rolled JSON, matching the existing style in `src/web.rs`'s
/// `healthz` handler - see that function's doc comment for why this
/// project doesn't pull in `serde` for a payload this small. `boxes` are in
/// source-frame pixel coordinates; this is what converts them to the 0..1
/// fractions clients actually get, so the browser never needs to know the
/// capture resolution.
pub fn detections_json(src_w: u32, src_h: u32, seq: u64, ms: f32, boxes: &[BBox]) -> String {
    let mut out = String::with_capacity(64 + boxes.len() * 72);
    out.push_str(&format!(
        "{{\"w\":{src_w},\"h\":{src_h},\"seq\":{seq},\"ms\":{:.1},\"dets\":[",
        if ms.is_finite() { ms.max(0.0) } else { 0.0 }
    ));
    for (i, b) in boxes.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let x = frac(b.x1 / src_w as f32);
        let y = frac(b.y1 / src_h as f32);
        let w = frac((b.x2 - b.x1) / src_w as f32);
        let h = frac((b.y2 - b.y1) / src_h as f32);
        let score = frac(b.score);
        out.push_str(&format!(
            "{{\"x\":{x:.4},\"y\":{y:.4},\"w\":{w:.4},\"h\":{h:.4},\
             \"score\":{score:.4},\"label\":\"person\"}}"
        ));
    }
    out.push_str("]}");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bbox(x1: f32, y1: f32, x2: f32, y2: f32, score: f32) -> BBox {
        BBox { x1, y1, x2, y2, score }
    }

    #[test]
    fn serializes_an_empty_detection_list() {
        assert_eq!(
            detections_json(1280, 720, 1, 12.5, &[]),
            "{\"w\":1280,\"h\":720,\"seq\":1,\"ms\":12.5,\"dets\":[]}"
        );
    }

    #[test]
    fn serializes_one_box_to_four_decimals() {
        // 320-wide box at x=32 in a 1280-wide frame -> x=0.1, w=0.25.
        let boxes = [bbox(32.0, 36.0, 352.0, 108.0, 0.876_54)];
        assert_eq!(
            detections_json(1280, 720, 7, 100.0, &boxes),
            "{\"w\":1280,\"h\":720,\"seq\":7,\"ms\":100.0,\"dets\":[\
             {\"x\":0.0250,\"y\":0.0500,\"w\":0.2500,\"h\":0.1000,\
             \"score\":0.8765,\"label\":\"person\"}]}"
        );
    }

    #[test]
    fn clamps_out_of_range_and_non_finite_coordinates() {
        let boxes = [
            bbox(f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -100.0, f32::NAN),
            bbox(-500.0, -500.0, 5000.0, 5000.0, 2.0),
        ];
        let json = detections_json(1280, 720, 1, f32::NAN, &boxes);
        assert!(!json.contains("NaN"), "must not emit the literal NaN: {json}");
        assert!(!json.contains("inf"), "must not emit inf/Infinity: {json}");
        // Every numeric field *inside a box* must land in [0, 1] - scoped to
        // the "dets" array so this doesn't also match the top-level "w"/"h"
        // fields, which are the source resolution in pixels, not fractions.
        let dets_start = json.find("\"dets\":[").expect("payload has a dets array");
        let dets = &json[dets_start..];
        for tok in ["\"x\":", "\"y\":", "\"w\":", "\"h\":", "\"score\":"] {
            for part in dets.split(tok).skip(1) {
                let end = part.find([',', '}']).unwrap();
                let v: f32 = part[..end].parse().expect("valid float");
                assert!((0.0..=1.0).contains(&v), "{tok}{v} out of range in {json}");
            }
        }
    }

    #[test]
    fn last_inference_ms_is_none_before_any_pass() {
        let hub = DetectionHub::new();
        assert_eq!(hub.last_inference_ms(), None);
        assert_eq!(hub.seconds_since_last_pass(), None);
    }

    #[test]
    fn last_inference_ms_reflects_the_most_recent_publish() {
        let hub = DetectionHub::new();
        hub.publish(1280, 720, &[], 42.5);
        assert_eq!(hub.last_inference_ms(), Some(42.5));
        assert!(hub.seconds_since_last_pass().unwrap() < 1.0);
    }

    #[test]
    fn seq_increments_across_publishes() {
        let hub = DetectionHub::new();
        hub.publish(1280, 720, &[], 1.0);
        hub.publish(1280, 720, &[], 1.0);
        let mut rx = hub.subscribe();
        assert_eq!(rx.borrow_and_update().seq, 2);
    }
}
