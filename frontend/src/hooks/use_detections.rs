//! Detection SSE lifecycle hook — ports `startDetect` / `stopDetect` from
//! the vanilla JS, including localStorage persistence, visibility-change
//! handling, and the stale-detection timeout.

use gloo_timers::callback::Interval;
use leptos::prelude::*;
use shared_types::DetectionPayload;
use wasm_bindgen::prelude::Closure;
use wasm_bindgen::JsCast;

/// Stale timeout: if no detection message arrives for this many ms, clear
/// the overlay so a phantom box never sits over live video.
const DETECTION_STALE_MS: u32 = 5000;
/// Stale check runs every second.
const STALE_CHECK_MS: u32 = 1000;

/// Return bundle from [`use_detections`].
pub struct UseDetectionsReturn {
    /// Whether the detection SSE connection is currently active. Controls
    /// the toggle's pressed state and the visibility-change reopen logic.
    pub is_open: RwSignal<bool>,
    /// Most recent parsed detection payload, or `None` when the connection
    /// is closed / in error / stale.
    pub latest_payload: ReadSignal<Option<DetectionPayload>>,
    /// Idempotent toggle: if open → close; if closed → open.
    pub toggle: Callback<(), ()>,
}

/// Reactive detection SSE lifecycle.
pub fn use_detections(
    detect_available: Memo<bool>,
) -> UseDetectionsReturn {
    let is_open = RwSignal::new(false);
    let (latest_payload, set_latest_payload) =
        signal(None::<DetectionPayload>);

    // Bookkeeping for stale detection.
    let last_payload_at: RwSignal<f64> = RwSignal::new(0.0);

    // The active EventSource handle, if any.
    let es: RwSignal<Option<web_sys::EventSource>> = RwSignal::new(None);

    // ── Helpers ──────────────────────────────────────────────────────

    let close_es = {
        let set_latest_payload = set_latest_payload;
        let es = es;
        move || {
            if let Some(source) = es.get_untracked() {
                source.close();
            }
            es.set(None);
            set_latest_payload.set(None);
        }
    };

    let open_es = {
        let set_latest_payload = set_latest_payload;
        let last_payload_at = last_payload_at;
        let es = es;
        move || {
            // Close any existing connection first (idempotent).
            if let Some(source) = es.get_untracked() {
                source.close();
            }

            let source = web_sys::EventSource::new("/detections")
                .expect("EventSource constructor should not fail");

            let onmessage_cb = {
                let set_latest_payload = set_latest_payload;
                let last_payload_at = last_payload_at;
                Closure::wrap(
                    Box::new(move |event: web_sys::MessageEvent| {
                        if let Some(data) = event.data().as_string() {
                            if let Ok(payload) =
                                serde_json::from_str::<DetectionPayload>(&data)
                            {
                                set_latest_payload.set(Some(payload));
                                last_payload_at
                                    .set(js_sys::Date::now() as f64);
                            }
                        }
                    })
                        as Box<dyn FnMut(web_sys::MessageEvent)>,
                )
            };

            source.set_onmessage(Some(onmessage_cb.as_ref().unchecked_ref()));
            onmessage_cb.forget();

            let onerror_cb = {
                let set_latest_payload = set_latest_payload;
                Closure::wrap(
                    Box::new(move |_event: web_sys::Event| {
                        set_latest_payload.set(None);
                    }) as Box<dyn FnMut(web_sys::Event)>,
                )
            };
            source
                .set_onerror(Some(onerror_cb.as_ref().unchecked_ref()));
            onerror_cb.forget();

            es.set(Some(source));
        }
    };

    // ── Toggle ───────────────────────────────────────────────────────

    let toggle = {
        let is_open = is_open;
        let open_es = open_es.clone();
        let close_es = close_es.clone();

        Callback::new(move |()| {
            if is_open.get_untracked() {
                close_es();
                is_open.set(false);
                try_persist("0");
            } else {
                open_es();
                is_open.set(true);
                try_persist("1");
            }
        })
    };

    // ── Auto-close when detect_available flips to false ──────────────

    let _auto_close_effect = Effect::new({
        let is_open = is_open;
        let close_es = close_es.clone();
        move |_| {
            if !detect_available.get() && is_open.get() {
                is_open.set(false);
                close_es();
                try_persist("0");
            }
        }
    });

    // ── Visibility change ────────────────────────────────────────────

    let _visibility_guard = {
        let is_open = is_open;
        let open_es = open_es.clone();
        let close_es = close_es.clone();

        let cb = Closure::wrap(
            Box::new(move || {
                let doc = document();
                if doc.hidden() {
                    if is_open.get_untracked() {
                        close_es();
                    }
                } else if is_open.get_untracked() {
                    open_es();
                }
            }) as Box<dyn FnMut()>,
        );

        document()
            .add_event_listener_with_callback(
                "visibilitychange",
                cb.as_ref().unchecked_ref(),
            )
            .expect("visibilitychange listener should register");

        cb.forget();
    };

    // ── Stale detection timeout ──────────────────────────────────────

    let _stale_interval = {
        let set_latest_payload = set_latest_payload;
        Interval::new(STALE_CHECK_MS, move || {
            let last = last_payload_at.get_untracked();
            if last > 0.0
                && (js_sys::Date::now() as f64 - last)
                    > DETECTION_STALE_MS as f64
            {
                set_latest_payload.set(None);
            }
        })
    };

    // ── Cleanup on drop ──────────────────────────────────────────────

    on_cleanup({
        let close_es = close_es.clone();
        move || {
            close_es();
        }
    });

    // ── Restore localStorage on mount ────────────────────────────────

    if try_restore() {
        is_open.set(true);
        open_es();
    }

    UseDetectionsReturn {
        is_open,
        latest_payload: latest_payload.into(),
        toggle,
    }
}

// ── Helpers ────────────────────────────────────────────────────────────

fn document() -> web_sys::Document {
    web_sys::window()
        .expect("no window in browser")
        .document()
        .expect("no document")
}

fn try_persist(value: &str) {
    let _ = web_sys::window()
        .and_then(|w| {
            w.local_storage()
                .ok()
                .flatten()
                .map(|storage| storage.set_item("winnie-detect", value))
        });
}

fn try_restore() -> bool {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item("winnie-detect").ok())
        .flatten()
        .as_deref()
        == Some("1")
}