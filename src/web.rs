//! HTTP layer: the viewer page, the live MJPEG stream, a single-frame
//! snapshot, a health check, and (when `--detect` is enabled) a live
//! detection-box stream for the overlay.

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::{FromRef, State};
use axum::http::{StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use bytes::Bytes;
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

use shared_types::{ErrorResponse, HealthzResponse, VideoSettingsUpdate};

use crate::config::{SharedVideoConfig, SourceKind};
use crate::detect::hub::DetectionHub;
use crate::detect::motion::hub::MotionHub;
use crate::hub::FrameHub;

/// Multipart boundary for the MJPEG stream. Arbitrary, but must match
/// between the `Content-Type` header and each part's leading marker line.
const BOUNDARY: &str = "winniecamframe";

/// Everything a request handler might need. `FrameHub` is always present;
/// `DetectionHub` is `None` when the server was started without `--detect`.
#[derive(Clone)]
pub struct AppState {
    pub hub: FrameHub,
    pub detect: Option<DetectionHub>,
    /// `None` whenever `detect` is - motion has no separate switch, see
    /// `motion`'s module doc comment.
    pub motion: Option<MotionHub>,
    /// Runtime-changeable camera settings shared with the capture backend.
    pub video_config: Arc<SharedVideoConfig>,
    /// Counts browser tabs watching `/stream.mjpeg`, separately from
    /// `FrameHub`'s own broadcast receiver count. Those two used to be the
    /// same number, but once detection exists the detector *also* holds a
    /// `FrameHub` subscription (see `detect::pump_loop`) to pull frames to
    /// run inference on - so `FrameHub::subscriber_count()` alone would
    /// overcount by one whenever `--detect` is on, which would make
    /// `/healthz`'s `subscribers` field lie about whether anyone's actually
    /// watching.
    viewers: Arc<AtomicUsize>,
    /// `/stream.mjpeg` and `/detections` are both infinite streams that
    /// otherwise only end when the *client* disconnects. `axum::serve`'s
    /// graceful shutdown stops accepting new connections but waits for
    /// existing ones to finish on their own - so without this, leaving a
    /// browser tab open would make the server hang forever on shutdown
    /// (`Ctrl-C`/`systemctl stop` would appear to do nothing until the tab
    /// was closed). Both streaming handlers race their channel against
    /// this token so the server always terminates client connections
    /// itself, rather than waiting to be let go.
    shutdown: CancellationToken,
}

impl AppState {
    pub fn new(
        hub: FrameHub,
        detect: Option<DetectionHub>,
        motion: Option<MotionHub>,
        video_config: Arc<SharedVideoConfig>,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            hub,
            detect,
            motion,
            video_config,
            viewers: Arc::new(AtomicUsize::new(0)),
            shutdown,
        }
    }
}

// Lets handlers that only care about the frame hub keep the exact
// `State<FrameHub>` signature they had before detection existed.
impl FromRef<AppState> for FrameHub {
    fn from_ref(state: &AppState) -> FrameHub {
        state.hub.clone()
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/stream.mjpeg", get(stream_mjpeg))
        .route("/snapshot.jpg", get(snapshot))
        .route("/healthz", get(healthz))
        .route("/detections", get(detections))
        .route("/events", get(events))
        .route("/events.json", get(events_json))
        .route("/api/config", get(get_config).put(update_config))
        .with_state(state)
}

async fn index() -> Html<&'static str> {
    Html(include_str!("index.html"))
}

async fn snapshot(State(hub): State<FrameHub>) -> Response {
    match hub.latest() {
        Some(frame) => ([(header::CONTENT_TYPE, "image/jpeg")], frame).into_response(),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "no frame captured yet - is the camera connected?",
        )
            .into_response(),
    }
}

async fn healthz(State(state): State<AppState>) -> Response {
    let stats = state.hub.stats();

    let (detect_state, detect_ms, detect_since) = match &state.detect {
        None => ("off".to_string(), None, None),
        Some(det_hub) => {
            let ms = det_hub.last_inference_ms().map(|ms| ms as f64);
            let since = det_hub.seconds_since_last_pass();
            (det_hub.state().as_str().to_string(), ms, since)
        }
    };

    let (motion_state, motion_events) = match &state.motion {
        None => ("off".to_string(), 0),
        Some(motion_hub) => (motion_hub.state().as_str().to_string(), motion_hub.event_count()),
    };

    let body = HealthzResponse {
        uptime_secs: stats.uptime.as_secs_f64(),
        frames_captured: stats.frames_captured,
        subscribers: state.viewers.load(Ordering::Relaxed) as u64,
        seconds_since_last_frame: stats.since_last_frame.map(|d| d.as_secs_f64()),
        detect: detect_state,
        detect_ms,
        seconds_since_last_detection: detect_since,
        motion: motion_state,
        motion_events,
    };

    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&body).expect("HealthzResponse serialization is infallible"),
    )
        .into_response()
}

/// RAII pair for `viewers`: incrementing (and logging "connected") happens
/// in `new`, decrementing (and logging "disconnected") happens in `Drop`,
/// so the two can never come apart. This used to be split - increment in
/// the handler, decrement inside the `async_stream` generator body - which
/// broke for any response whose body is never polled (e.g. axum 0.8 routes
/// `HEAD /stream.mjpeg` to this same handler and then strips the body
/// without polling it), permanently leaking +1 per such request. Now the
/// guard is constructed once in the handler and simply moved into the
/// generator; a moved-but-never-polled generator still drops its locals
/// when the response is dropped, so the pairing holds on every path.
struct DisconnectLog {
    viewers: Arc<AtomicUsize>,
}

impl DisconnectLog {
    fn new(viewers: Arc<AtomicUsize>) -> Self {
        let count = viewers.fetch_add(1, Ordering::Relaxed) + 1;
        tracing::info!(subscribers = count, "viewer connected");
        Self { viewers }
    }
}

impl Drop for DisconnectLog {
    fn drop(&mut self) {
        let remaining = self.viewers.fetch_sub(1, Ordering::Relaxed).saturating_sub(1);
        tracing::info!(subscribers = remaining, "viewer disconnected");
    }
}

async fn stream_mjpeg(State(state): State<AppState>) -> Response {
    let hub = state.hub.clone();
    let mut rx = hub.subscribe();
    let log_on_drop = DisconnectLog::new(state.viewers.clone());
    let first_frame = hub.latest();
    let shutdown = state.shutdown.clone();

    let body_stream = async_stream::stream! {
        let _log_on_drop = log_on_drop;

        // Paint something immediately instead of waiting on the next capture.
        if let Some(frame) = first_frame {
            yield Ok::<Bytes, std::io::Error>(encode_part(&frame));
        }

        loop {
            // Racing against `shutdown` (rather than only `rx.recv()`) is
            // what lets the server close this connection itself on exit -
            // see `AppState::shutdown`'s doc comment for why that matters.
            tokio::select! {
                _ = shutdown.cancelled() => break,
                r = rx.recv() => match r {
                    Ok(frame) => yield Ok(encode_part(&frame)),
                    Err(RecvError::Lagged(skipped)) => {
                        tracing::debug!(skipped, "viewer fell behind; dropping older frames");
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/x-mixed-replace; boundary={BOUNDARY}"),
        )
        .header(header::CACHE_CONTROL, "no-store, no-cache, private")
        .body(Body::from_stream(body_stream))
        .expect("static header values are always valid")
}

/// Wraps one JPEG frame in its multipart part: boundary marker, headers,
/// blank line, the frame itself, then a trailing CRLF before the next part.
fn encode_part(frame: &Bytes) -> Bytes {
    let mut out = Vec::with_capacity(frame.len() + 96);
    out.extend_from_slice(b"--");
    out.extend_from_slice(BOUNDARY.as_bytes());
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(b"Content-Type: image/jpeg\r\n");
    out.extend_from_slice(format!("Content-Length: {}\r\n", frame.len()).as_bytes());
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(frame);
    out.extend_from_slice(b"\r\n");
    Bytes::from(out)
}

/// Mirrors [`DisconnectLog`] for `/detections` viewers: closing the overlay
/// (or losing the connection) drops `DetectionHub::subscriber_count()`,
/// which is what tells `detect::pump_loop` to slow back down to the idle
/// detection rate. Unlike video's `FrameHub`, nothing else ever calls
/// `DetectionHub::subscribe()`, so its own receiver count is accurate here
/// without needing a dedicated counter.
struct DetectDisconnectLog {
    hub: DetectionHub,
}

impl Drop for DetectDisconnectLog {
    fn drop(&mut self) {
        tracing::info!(
            subscribers = self.hub.subscriber_count().saturating_sub(1),
            "overlay disconnected"
        );
    }
}

async fn detections(State(state): State<AppState>) -> Response {
    let Some(det_hub) = state.detect.clone() else {
        let body =
            serde_json::to_string(&ErrorResponse { error: "detection is not enabled".into() })
                .expect("ErrorResponse serialization is infallible");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response();
    };

    let mut rx = det_hub.subscribe();
    tracing::info!(subscribers = det_hub.subscriber_count(), "overlay connected");
    let shutdown = state.shutdown.clone();

    let stream = async_stream::stream! {
        let _log_on_drop = DetectDisconnectLog { hub: det_hub.clone() };

        // Paint the most recent result immediately, same as stream_mjpeg
        // does for video - a viewer shouldn't wait out a detection pass
        // just to see the last one that already happened.
        let json = { rx.borrow_and_update().json.clone() };
        yield Ok::<Event, Infallible>(Event::default().data(json));

        loop {
            // Racing against `shutdown` is what lets the server close this
            // connection itself on exit - see `AppState::shutdown`'s doc
            // comment; without it an open overlay tab would hang shutdown
            // exactly like an open `/stream.mjpeg` tab would.
            tokio::select! {
                _ = shutdown.cancelled() => break,
                changed = rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    // Scoped deliberately: `borrow_and_update` returns a
                    // guard holding a non-Send RwLock read guard. It must
                    // be dropped before the next await, or the generator
                    // stops being Send and this function fails to compile
                    // with an opaque error pointing at `Sse::new` instead
                    // of here.
                    let json = { rx.borrow_and_update().json.clone() };
                    yield Ok(Event::default().data(json));
                }
            }
        }
    };

    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

/// Mirrors [`DetectDisconnectLog`] for `/events` viewers.
struct MotionDisconnectLog {
    hub: MotionHub,
}

impl Drop for MotionDisconnectLog {
    fn drop(&mut self) {
        tracing::info!(
            subscribers = self.hub.subscriber_count().saturating_sub(1),
            "events viewer disconnected"
        );
    }
}

/// SSE stream of motion events. Modeled directly on `detections`: a fresh
/// connection is first sent everything currently in the ring buffer (as one
/// `snapshot` event, so a viewer sees recent history immediately instead of
/// waiting out however long it takes for the next real event), then each
/// new event as it's published.
async fn events(State(state): State<AppState>) -> Response {
    let Some(motion_hub) = state.motion.clone() else {
        let body =
            serde_json::to_string(&ErrorResponse { error: "detection is not enabled".into() })
                .expect("ErrorResponse serialization is infallible");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response();
    };

    let mut rx = motion_hub.subscribe();
    tracing::info!(subscribers = motion_hub.subscriber_count(), "events viewer connected");
    let shutdown = state.shutdown.clone();

    let stream = async_stream::stream! {
        let _log_on_drop = MotionDisconnectLog { hub: motion_hub.clone() };

        yield Ok::<Event, Infallible>(
            Event::default().event("snapshot").data(motion_hub.recent_json())
        );

        loop {
            // Racing against `shutdown` is what lets the server close this
            // connection itself on exit - see `AppState::shutdown`'s doc
            // comment; without it an open tab would hang shutdown exactly
            // like an open `/stream.mjpeg` tab would.
            tokio::select! {
                _ = shutdown.cancelled() => break,
                r = rx.recv() => match r {
                    Ok(event) => {
                        yield Ok(Event::default().event("motion").data(crate::detect::motion::hub::event_json(&event)));
                    }
                    Err(RecvError::Lagged(skipped)) => {
                        tracing::debug!(skipped, "events viewer fell behind; some events were missed");
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        }
    };

    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

/// The motion ring buffer as a single JSON document, for polling instead of
/// holding an SSE connection open.
async fn events_json(State(state): State<AppState>) -> Response {
    let Some(motion_hub) = state.motion.clone() else {
        let body =
            serde_json::to_string(&ErrorResponse { error: "detection is not enabled".into() })
                .expect("ErrorResponse serialization is infallible");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response();
    };
    (
        [(header::CONTENT_TYPE, "application/json")],
        motion_hub.recent_json(),
    )
        .into_response()
}

// --- Runtime config endpoints -------------------------------------------

/// `GET /api/config` — current camera settings + backend kind.
async fn get_config(State(state): State<AppState>) -> Response {
    let settings = state.video_config.settings.read().unwrap().clone();
    let backend = match state.video_config.source_kind {
        SourceKind::Rpicam | SourceKind::Auto => "rpicam",
        SourceKind::V4l2 => "v4l2",
    };
    let body = serde_json::json!({
        "backend": backend,
        "width": settings.width,
        "height": settings.height,
        "fps": settings.fps,
        "quality": settings.quality,
        "hflip": settings.hflip,
        "vflip": settings.vflip,
    })
    .to_string();
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

/// `PUT /api/config` — apply a partial set of updated settings.
async fn update_config(
    State(state): State<AppState>,
    Json(update): Json<VideoSettingsUpdate>,
) -> Response {
    if let Err(msg) = update.validate() {
        let body =
            serde_json::to_string(&ErrorResponse { error: msg })
                .expect("ErrorResponse serialization is infallible");
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            [(header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response();
    }

    let changed = {
        let mut settings = state.video_config.settings.write().unwrap();
        update.apply_to(&mut settings)
    };

    if changed {
        state.video_config.mark_changed();
        tracing::info!(?update, "camera settings updated; restarting capture");
    }

    // Return the full config so the client doesn't need a follow-up GET.
    get_config(State(state)).await
}
