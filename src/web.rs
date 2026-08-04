//! HTTP layer: the viewer page, the live MJPEG stream, a single-frame
//! snapshot, and a health check.

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use bytes::Bytes;
use tokio::sync::broadcast::error::RecvError;

use crate::hub::FrameHub;

/// Multipart boundary for the MJPEG stream. Arbitrary, but must match
/// between the `Content-Type` header and each part's leading marker line.
const BOUNDARY: &str = "winniecamframe";

pub fn router(hub: FrameHub) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/stream.mjpeg", get(stream_mjpeg))
        .route("/snapshot.jpg", get(snapshot))
        .route("/healthz", get(healthz))
        .with_state(hub)
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

async fn healthz(State(hub): State<FrameHub>) -> Response {
    let stats = hub.stats();
    let since_last_frame = match stats.since_last_frame {
        Some(d) => d.as_secs_f64().to_string(),
        None => "null".to_string(),
    };
    let body = format!(
        "{{\"uptime_secs\":{},\"frames_captured\":{},\"subscribers\":{},\"seconds_since_last_frame\":{}}}",
        stats.uptime.as_secs(),
        stats.frames_captured,
        stats.subscribers,
        since_last_frame,
    );
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

/// Logs when a viewer's stream ends, whichever way that happens (client
/// disconnect drops the response body mid-poll; the loop otherwise only
/// exits if the hub itself is torn down). Dropping the async generator's
/// locals - including this guard - is the only reliable "connection over"
/// signal available here.
struct DisconnectLog {
    hub: FrameHub,
}

impl Drop for DisconnectLog {
    fn drop(&mut self) {
        // This guard is still alive (and thus still "subscribed" as far as
        // the hub's receiver count goes) at the moment it drops, so the
        // count it reads is one higher than what remains once it's gone.
        tracing::info!(
            subscribers = self.hub.subscriber_count().saturating_sub(1),
            "viewer disconnected"
        );
    }
}

async fn stream_mjpeg(State(hub): State<FrameHub>) -> Response {
    let mut rx = hub.subscribe();
    tracing::info!(subscribers = hub.subscriber_count(), "viewer connected");
    let first_frame = hub.latest();

    let body_stream = async_stream::stream! {
        let _log_on_drop = DisconnectLog { hub: hub.clone() };

        // Paint something immediately instead of waiting on the next capture.
        if let Some(frame) = first_frame {
            yield Ok::<Bytes, std::io::Error>(encode_part(&frame));
        }

        loop {
            match rx.recv().await {
                Ok(frame) => yield Ok(encode_part(&frame)),
                Err(RecvError::Lagged(skipped)) => {
                    tracing::debug!(skipped, "viewer fell behind; dropping older frames");
                }
                Err(RecvError::Closed) => break,
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
