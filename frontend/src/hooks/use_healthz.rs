use gloo_timers::callback::Interval;
use leptos::prelude::*;

const HEALTH_POLL_MS: u32 = 5000;
const STALE_AFTER_SECS: f64 = 6.0;
const NO_FRAMES_TIMEOUT_MS: u64 = 15_000;
/// Bundle of reactive signals driven by periodic `/healthz` polling.
pub struct HealthzState {
    /// Raw response from the last successful poll. `None` means no poll has
    /// completed yet (or the server is unreachable).
    pub last: ReadSignal<Option<shared_types::HealthzResponse>>,
    /// Formatted "Watching since …" / "Just started" text.
    pub since_text: Memo<String>,
    /// Formatted FPS text (empty when unknown).
    pub fps_text: Memo<String>,
    /// Formatted viewer count text.
    pub viewers_text: Memo<String>,
    /// True when `seconds_since_last_frame > STALE_AFTER_SECS`.
    pub is_stale: Memo<bool>,
    /// True when `detect == "ready"`.
    pub detect_available: Memo<bool>,
    /// True when `motion == "active"`.
    pub motion_active: Memo<bool>,
    /// True when the server was started with `--audio`. Unlike detection
    /// there is no "loading" state to wait out — the microphone opens on
    /// demand, so anything other than `"off"` means Listen will work.
    pub audio_available: Memo<bool>,
    /// What `/audio` serves (`"webm-opus" | "adts-aac" | ...`), `None` before
    /// the first poll or from a server too old to report it. Lets the audio
    /// hook pick a transport instead of guessing.
    pub audio_format: Memo<Option<String>>,
    /// Whether the server is currently unreachable (last poll failed).
    pub offline: ReadSignal<bool>,
}

/// Polls `/healthz` every 5 seconds and drives the footer stats signals.
///
/// When the server is detected as stale or unreachable, the caller should
/// call the provided reconnect callback to force a fresh MJPEG connection.
pub fn use_healthz(on_reconnect: impl Fn() + Clone + 'static) -> HealthzState {
    let (last, set_last) = signal(None::<shared_types::HealthzResponse>);
    let (offline, set_offline) = signal(false);

    // Tracking for "no camera" placeholder detection
    let zero_frames_since: RwSignal<Option<f64>> = RwSignal::new(None);

    // FPS computation state: (prev_frames, prev_timestamp)
    let fps_prev: RwSignal<Option<(u64, f64)>> = RwSignal::new(None);

    // Poll every 5 seconds.
    let _ = Interval::new(HEALTH_POLL_MS, {
        let on_reconnect = on_reconnect.clone();
        let set_last = set_last;
        let set_offline = set_offline;
        let zero_frames_since = zero_frames_since;

        move || {
            wasm_bindgen_futures::spawn_local({
                let on_reconnect = on_reconnect.clone();
                let set_last = set_last;
                let set_offline = set_offline;
                let zero_frames_since = zero_frames_since;

                async move {
                    let result = gloo_net::http::Request::get("/healthz").send().await;

                    match result {
                        Ok(resp) => {
                            match resp.json::<shared_types::HealthzResponse>().await {
                                Ok(data) => {
                                    set_offline.set(false);
                                    set_last.set(Some(data.clone()));

                                    // Stale detection triggers reconnect.
                                    if let Some(since) = data.seconds_since_last_frame {
                                        if since > STALE_AFTER_SECS {
                                            on_reconnect();
                                        }
                                    }

                                    // Track zero-frames window.
                                    if data.frames_captured == 0 {
                                        if zero_frames_since.get_untracked().is_none() {
                                            zero_frames_since.set(Some(
                                                window()
                                                    .performance()
                                                    .map(|p| p.now())
                                                    .unwrap_or(0.0),
                                            ));
                                        }
                                    } else {
                                        zero_frames_since.set(None);
                                    }
                                }
                                Err(_) => {
                                    set_offline.set(true);
                                }
                            }
                        }
                        Err(_) => {
                            set_offline.set(true);
                        }
                    }
                }
            });
        }
    })
    .forget();

    // Force an immediate poll (the interval above fires after 5s; we want
    // data right away).
    wasm_bindgen_futures::spawn_local({
        let _on_reconnect = on_reconnect.clone();
        let set_last = set_last;
        let set_offline = set_offline;
        let _zero_frames_since = zero_frames_since;

        async move {
            let result = gloo_net::http::Request::get("/healthz").send().await;

            match result {
                Ok(resp) => match resp.json::<shared_types::HealthzResponse>().await {
                    Ok(data) => {
                        set_offline.set(false);
                        set_last.set(Some(data));
                    }
                    Err(_) => {
                        set_offline.set(true);
                    }
                },
                Err(_) => {
                    set_offline.set(true);
                }
            }
        }
    });

    // Derived signals.

    let since_text = Memo::new(move |_| {
        let Some(ref data) = last.get() else {
            return String::new();
        };
        fmt_since(data.uptime_secs)
    });

    let fps_text = Memo::new({
        let last = last;
        let fps_prev = fps_prev;
        move |_| {
            let Some(ref data) = last.get() else {
                return String::new();
            };
            let now = js_sys::Date::now() as f64;
            let prev = fps_prev.get_untracked();
            let result = match prev {
                Some((prev_frames, prev_t)) => {
                    let dt = (now - prev_t) / 1000.0;
                    let df = data.frames_captured.saturating_sub(prev_frames);
                    if dt > 0.0 && df > 0 {
                        format!("{} fps", (df as f64 / dt).round() as u64)
                    } else {
                        String::new()
                    }
                }
                None => String::new(),
            };
            fps_prev.set(Some((data.frames_captured, now)));
            result
        }
    });

    let viewers_text = Memo::new(move |_| {
        let Some(ref data) = last.get() else {
            return String::new();
        };
        let v = data.subscribers;
        if v == 1 {
            "1 viewer".into()
        } else {
            format!("{v} viewers")
        }
    });

    let is_stale = Memo::new(move |_| {
        last.get()
            .and_then(|d| d.seconds_since_last_frame)
            .map(|s| s > STALE_AFTER_SECS)
            .unwrap_or(false)
    });

    let detect_available =
        Memo::new(move |_| last.get().map(|d| d.detect == "ready").unwrap_or(false));

    let motion_active =
        Memo::new(move |_| last.get().map(|d| d.motion == "active").unwrap_or(false));

    let audio_available = Memo::new(move |_| last.get().map(|d| d.audio != "off").unwrap_or(false));

    let audio_format = Memo::new(move |_| {
        last.get()
            .map(|d| d.audio_format.clone())
            .filter(|f| f != "off")
    });

    HealthzState {
        last: last.into(),
        since_text,
        fps_text,
        viewers_text,
        is_stale,
        detect_available,
        motion_active,
        audio_available,
        audio_format,
        offline: offline.into(),
    }
}

// ── Helpers ────────────────────────────────────────────────────────────

fn window() -> web_sys::Window {
    web_sys::window().expect("no window in browser")
}

fn fmt_since(uptime_secs: f64) -> String {
    if uptime_secs < 60.0 {
        return "Just started".into();
    }
    if uptime_secs >= 86400.0 {
        let d = (uptime_secs / 86400.0) as u32;
        let h = ((uptime_secs % 86400.0) / 3600.0) as u32;
        return if h > 0 {
            format!("Watching for {d}d {h}h")
        } else {
            format!("Watching for {d}d")
        };
    }
    // Compute wall-clock time when watching started.
    let now_ms = js_sys::Date::now() as f64;
    let start_ms = now_ms - uptime_secs * 1000.0;
    let start = js_sys::Date::new(&(start_ms).into());
    let hours = start.get_hours();
    let minutes = start.get_minutes();
    let ampm = if hours >= 12 { "PM" } else { "AM" };
    let h12 = if hours == 0 {
        12
    } else if hours > 12 {
        hours - 12
    } else {
        hours
    };
    format!("Watching since {h12}:{minutes:02} {ampm}")
}
