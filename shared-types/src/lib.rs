//! Shared types for the HTTP boundary between the winnie-cam server and its
//! web frontend. Every JSON shape the server sends or receives is a struct
//! here, so the Leptos frontend (and any other client) can depend on this
//! crate and get compile-time field-name / type checking instead of hoping
//! its `JSON.parse` keys match the server's `format!` strings.

use serde::{Deserialize, Serialize};

// ── /healthz ───────────────────────────────────────────────────────────

/// Response body for `GET /healthz`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthzResponse {
    pub uptime_secs: f64,
    pub frames_captured: u64,
    pub subscribers: u64,
    /// `None` when no frame has ever been captured.
    pub seconds_since_last_frame: Option<f64>,
    /// `"off" | "loading" | "ready" | "error"`.
    pub detect: String,
    /// `None` before the first detection pass has run, or when detection is
    /// disabled.
    pub detect_ms: Option<f64>,
    /// Time since the last detection pass, in seconds. `None` under the
    /// same conditions as `detect_ms`.
    pub seconds_since_last_detection: Option<f64>,
    /// `"off" | "idle" | "active"`.
    pub motion: String,
    pub motion_events: u64,
}

// ── /detections (SSE) ──────────────────────────────────────────────────

/// One SSE event on the `/detections` stream — a completed detection pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectionPayload {
    /// Source frame width in pixels (for debugging), never used by the
    /// overlay renderer.
    pub w: u32,
    /// Source frame height in pixels.
    pub h: u32,
    /// Monotonically increasing pass counter.
    pub seq: u64,
    /// Inference time for this pass, in milliseconds.
    pub ms: f32,
    /// Detected bounding boxes, in normalized (0..1) source-frame
    /// fractions. Empty when nothing was found.
    pub dets: Vec<DetectionBox>,
}

/// One person detection, in normalized 0..1 coordinates of the source
/// frame. The overlay maps these onto the rendered image without ever
/// knowing the capture resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectionBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub score: f32,
    pub label: String,
}

// ── /events (SSE) & /events.json ───────────────────────────────────────

/// One motion event on the `/events` SSE stream or in the `/events.json`
/// ring-buffer snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MotionEvent {
    pub seq: u64,
    /// `"started"` or `"stopped"`.
    pub kind: String,
    /// Wall-clock time of the event, milliseconds since the Unix epoch.
    pub at: u64,
    /// The masked-diff score that triggered the transition, 0..1.
    pub score: f32,
    /// Present only for `"stopped"` events: how long the motion episode
    /// lasted, in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// The `snapshot` SSE event (sent once on connect so a fresh viewer sees
/// recent history immediately) and the body of `GET /events.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MotionSnapshot {
    pub events: Vec<MotionEvent>,
}

// ── /api/config ────────────────────────────────────────────────────────

/// Body of `GET /api/config`, and of a successful `PUT /api/config`: the
/// current settings plus which capture backend is running. Clients need
/// the backend kind because `quality`, `hflip` and `vflip` only take
/// effect on `rpicam` (a USB webcam encodes MJPEG on-device).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CameraConfig {
    /// `"rpicam"` or `"v4l2"`.
    pub backend: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub quality: u8,
    pub hflip: bool,
    pub vflip: bool,
}

/// The mutable camera settings the server owns. Sent to clients wrapped in
/// a [`CameraConfig`], which adds the read-only `backend` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoSettings {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub quality: u8,
    pub hflip: bool,
    pub vflip: bool,
}

/// Partial update payload for `PUT /api/config` — every field is optional.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoSettingsUpdate {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<u32>,
    pub quality: Option<u8>,
    pub hflip: Option<bool>,
    pub vflip: Option<bool>,
}

impl VideoSettingsUpdate {
    /// Returns `Ok(())` if every present field passes range checks, or an
    /// error message suitable for a 422 body.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(w) = self.width {
            if w == 0 {
                return Err("width must be > 0".into());
            }
        }
        if let Some(h) = self.height {
            if h == 0 {
                return Err("height must be > 0".into());
            }
        }
        if let Some(f) = self.fps {
            if f == 0 {
                return Err("fps must be > 0".into());
            }
        }
        if let Some(q) = self.quality {
            if q < 1 || q > 100 {
                return Err("quality must be 1-100".into());
            }
        }
        Ok(())
    }

    /// Apply every `Some` field to `target`; fields left `None` are left
    /// alone. Returns `true` if at least one field actually changed.
    pub fn apply_to(&self, target: &mut VideoSettings) -> bool {
        let mut changed = false;
        if let Some(w) = self.width {
            if target.width != w {
                target.width = w;
                changed = true;
            }
        }
        if let Some(h) = self.height {
            if target.height != h {
                target.height = h;
                changed = true;
            }
        }
        if let Some(f) = self.fps {
            if target.fps != f {
                target.fps = f;
                changed = true;
            }
        }
        if let Some(q) = self.quality {
            if target.quality != q {
                target.quality = q;
                changed = true;
            }
        }
        if let Some(hf) = self.hflip {
            if target.hflip != hf {
                target.hflip = hf;
                changed = true;
            }
        }
        if let Some(vf) = self.vflip {
            if target.vflip != vf {
                target.vflip = vf;
                changed = true;
            }
        }
        changed
    }
}

// ── Error responses ────────────────────────────────────────────────────

/// Generic error body, used by several endpoints (config validation,
/// detection-unavailable, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

// ── NaN/inf safety helpers ─────────────────────────────────────────────
//
// `serde_json` will emit the bare word `NaN` or `inf` for non-finite f32
// values — one degenerate float from the model or a motion score would
// break `JSON.parse` for every viewer. Every float that reaches the wire
// must go through these first.

/// Clamps a value into `[0.0, 1.0]`, mapping non-finite values to `0.0`.
/// Use for detection scores, normalized coordinates, and motion scores
/// before stuffing them into a wire struct.
pub fn clamp_fraction(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Clamps a pixel coordinate into `[0.0, extent]`, mapping non-finite
/// values to `0.0`. Use for bounding-box corners in frame-pixel space
/// before normalizing them to 0..1 fractions.
pub fn clamp_coord(v: f32, extent: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, extent)
    } else {
        0.0
    }
}
