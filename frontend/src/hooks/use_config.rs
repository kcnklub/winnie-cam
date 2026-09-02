//! Camera settings API hook — ports `fetchConfig()` / `applySettings()`
//! from the vanilla JS.
//!
//! This is the transport layer only: it owns the last config the server
//! sent plus the inline error / status strings. The form's own field
//! signals live in `SettingsPanel`, which repopulates them whenever
//! `config` changes.

use gloo_net::http::Request;
use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use shared_types::{CameraConfig, ErrorResponse, VideoSettingsUpdate};
use web_sys::RequestCache;

const CONFIG_URL: &str = "/api/config";
/// Shown after a successful PUT. The capture pipeline restarts, so the
/// video feed drops and reconnects a moment later.
const APPLIED_STATUS: &str = "Applied \u{2014} restarting capture\u{2026}";
/// How long the "Applied" status stays on screen.
const STATUS_CLEAR_MS: u32 = 3000;
/// Client-side validation failure. Note the en dash, matching the old UI.
const VALIDATION_MSG: &str = "All values must be positive; quality must be 1\u{2013}100.";
/// Used when the server rejects the PUT without a parseable `ErrorResponse`
/// body — axum's own extractor rejections are plain text, not JSON.
const FALLBACK_ERROR: &str = "Failed to apply settings.";
const NETWORK_ERROR: &str = "Failed to reach server";

/// Return bundle from [`use_config`]. `Copy` because the App reads it
/// after handing it to the panel — every field is a handle, not data.
#[derive(Clone, Copy)]
pub struct ConfigState {
    /// Last config the server sent. `None` until the first successful
    /// fetch; left untouched by a failed one, so the form keeps whatever
    /// the user had typed.
    pub config: RwSignal<Option<CameraConfig>>,
    /// Inline error text under the panel's action row.
    pub error: RwSignal<Option<String>>,
    /// Transient status text below the panel.
    pub status: RwSignal<Option<String>>,
    /// GET `/api/config`.
    pub fetch: Callback<(), ()>,
    /// PUT `/api/config`, after client-side validation.
    pub apply: Callback<VideoSettingsUpdate, ()>,
}

/// Reactive `/api/config` access.
///
/// `on_applied` fires only after a successful PUT — the App uses it to
/// close the settings panel, which keeps view state out of this hook.
pub fn use_config(on_applied: Callback<(), ()>) -> ConfigState {
    let config: RwSignal<Option<CameraConfig>> = RwSignal::new(None);
    let error: RwSignal<Option<String>> = RwSignal::new(None);
    let status: RwSignal<Option<String>> = RwSignal::new(None);

    let fetch = Callback::new(move |()| {
        wasm_bindgen_futures::spawn_local(async move {
            let Some(cfg) = get_config().await else {
                // Server unreachable: keep the current form contents so the
                // user can still edit and retry.
                return;
            };

            config.set(Some(cfg));
            error.set(None);
        });
    });

    let apply = Callback::new(move |update: VideoSettingsUpdate| {
        // Validate before the round-trip, reusing the same checks the
        // server runs so the messages can never drift apart.
        if update.validate().is_err() {
            error.set(Some(VALIDATION_MSG.to_string()));
            status.set(None);
            return;
        }

        wasm_bindgen_futures::spawn_local(async move {
            match put_config(&update).await {
                Ok(cfg) => {
                    config.set(Some(cfg));
                    error.set(None);
                    status.set(Some(APPLIED_STATUS.to_string()));
                    clear_status_later(status);
                    on_applied.run(());
                }
                Err(msg) => {
                    error.set(Some(msg));
                    status.set(None);
                }
            }
        });
    });

    ConfigState {
        config,
        error,
        status,
        fetch,
        apply,
    }
}

// ── Transport ──────────────────────────────────────────────────────────

/// GET the current config. `None` on any transport or parse failure —
/// callers treat every failure the same way.
async fn get_config() -> Option<CameraConfig> {
    let resp = Request::get(CONFIG_URL)
        .cache(RequestCache::NoStore)
        .send()
        .await
        .ok()?;

    resp.json::<CameraConfig>().await.ok()
}

/// PUT an update. `Err` carries the message to show inline.
async fn put_config(update: &VideoSettingsUpdate) -> Result<CameraConfig, String> {
    let request = Request::put(CONFIG_URL)
        .cache(RequestCache::NoStore)
        .json(update)
        .map_err(|_| FALLBACK_ERROR.to_string())?;

    let resp = request
        .send()
        .await
        .map_err(|_| NETWORK_ERROR.to_string())?;

    if !resp.ok() {
        // A 422 carries an ErrorResponse; extractor rejections don't.
        let msg = resp
            .json::<ErrorResponse>()
            .await
            .map(|body| body.error)
            .unwrap_or_else(|_| FALLBACK_ERROR.to_string());
        return Err(msg);
    }

    resp.json::<CameraConfig>()
        .await
        .map_err(|_| FALLBACK_ERROR.to_string())
}

// ── Helpers ────────────────────────────────────────────────────────────

/// Hide the "Applied" status after a few seconds, but only if it is still
/// the message we set — a newer status must not be clobbered by an older
/// timer.
fn clear_status_later(status: RwSignal<Option<String>>) {
    wasm_bindgen_futures::spawn_local(async move {
        TimeoutFuture::new(STATUS_CLEAR_MS).await;

        if status.get_untracked().as_deref() == Some(APPLIED_STATUS) {
            status.set(None);
        }
    });
}
