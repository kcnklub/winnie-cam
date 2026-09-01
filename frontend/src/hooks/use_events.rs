//! Motion events SSE lifecycle — ports `startEvents()` / `stopEvents()`
//! from the vanilla JS, including the `snapshot` / `motion` event handlers,
//! sequence-number bookkeeping for reconnection dedup, and alert + chime
//! triggering.
//!
//! Start/stop is tied to `detect_is_open`: when detection opens, the motion
//! EventSource is opened; when detection closes, it is torn down and any
//! active alert is cleared.

use crate::utils::audio::{UseChimeReturn, use_chime};
use leptos::prelude::*;
use shared_types::{MotionEvent, MotionSnapshot};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::Closure;

/// Maximum number of raw events kept in memory (the display list is capped
/// separately by the `recent_text` `Memo`).
const MAX_EVENTS: usize = 50;
/// Number of formatted display lines shown in the panel.
const MAX_DISPLAY_ITEMS: usize = 6;
/// Original page title, captured once.
const ORIGINAL_TITLE: &str = "Winnie";

/// Return bundle from [`use_events`].
pub struct MotionState {
    /// Whether there is currently a motion episode in progress (derived
    /// from the last event's `kind`).
    pub is_moving: ReadSignal<bool>,
    /// Formatted display lines for the motion panel (newest first, at most
    /// 6). Each entry is a single string like `"10:23:45 — started moving"`.
    pub recent_text: Memo<Vec<String>>,
    /// Whether the alert banner should be visible right now.
    pub alert_active: RwSignal<bool>,
    /// Formatted clock time of the alert that is currently showing (or
    /// empty when no alert is active).
    pub alert_time_text: ReadSignal<String>,
    /// Whether the alert chime sound is currently enabled.
    pub sound_on: RwSignal<bool>,
    /// Toggle the sound on/off (persisted to localStorage).
    pub toggle_sound: Callback<(), ()>,
    /// Total raw event count (for the counter in the panel header).
    pub total_count: Memo<u64>,
}

/// Reactive motion-event SSE lifecycle.
///
/// Opens an `EventSource` on `/events` whenever `detect_is_open` is `true`
/// and tears it down when detection stops (or the component unmounts).
pub fn use_events(detect_is_open: ReadSignal<bool>) -> MotionState {
    // ── Chime ─────────────────────────────────────────────────────────

    let UseChimeReturn {
        sound_on,
        toggle_sound,
        chime,
    } = use_chime();

    // ── Core signals ──────────────────────────────────────────────────

    let events: RwSignal<Vec<MotionEvent>> = RwSignal::new(Vec::new());
    let (is_moving, set_is_moving) = signal(false);
    let alert_active: RwSignal<bool> = RwSignal::new(false);
    let (alert_time_text, set_alert_time_text) = signal(String::new());

    // ── Seq bookkeeping (ported from JS) ──────────────────────────────

    let seeded_alerts: RwSignal<bool> = RwSignal::new(false);
    let last_alerted_seq: RwSignal<i64> = RwSignal::new(-1);

    // ── EventSource handle ────────────────────────────────────────────

    let es: RwSignal<Option<web_sys::EventSource>> = RwSignal::new(None);

    // ── Helpers ───────────────────────────────────────────────────────

    let total_count = Memo::new(move |_| events.get().len() as u64);

    let recent_text = Memo::new(move |_| {
        let evs = events.get();
        if evs.is_empty() {
            return Vec::new();
        }
        let start = if evs.len() > MAX_DISPLAY_ITEMS {
            evs.len() - MAX_DISPLAY_ITEMS
        } else {
            0
        };
        evs[start..]
            .iter()
            .rev()
            .map(|ev| {
                let time = format_clock_time(ev.at);
                match ev.kind.as_str() {
                    "stopped" => {
                        let dur = ev
                            .duration_ms
                            .map(format_duration)
                            .unwrap_or_else(|| "?".into());
                        format!("{time} — settled ({dur})")
                    }
                    _ => format!("{time} — started moving"),
                }
            })
            .collect()
    });

    let clear_alert = {
        let alert_active = alert_active;
        let set_alert_time_text = set_alert_time_text;
        move || {
            alert_active.set(false);
            set_alert_time_text.set(String::new());
            // Restore original title.
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                doc.set_title(ORIGINAL_TITLE);
            }
        }
    };

    let show_alert = {
        let alert_active = alert_active;
        let set_alert_time_text = set_alert_time_text;
        let chime = chime;
        move |ev: MotionEvent| {
            let time = format_clock_time(ev.at);
            alert_active.set(true);
            set_alert_time_text.set(time);
            // Title flash.
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                doc.set_title(&format!("(!) Motion — {ORIGINAL_TITLE}"));
            }
            chime.run(());
        }
    };

    // Single path for every live motion event (not snapshot replays).
    // Keeps pill/list/alert in sync with each other.
    let handle_motion_event = {
        let events = events;
        let set_is_moving = set_is_moving;
        let last_alerted_seq = last_alerted_seq;
        let show_alert = show_alert.clone();

        move |ev: MotionEvent| {
            events.update(|v| {
                v.push(ev.clone());
                if v.len() > MAX_EVENTS {
                    v.remove(0);
                }
            });

            let is_started = ev.kind == "started";
            set_is_moving.set(is_started);

            if is_started {
                let seq = ev.seq as i64;
                last_alerted_seq.update(|s| *s = (*s).max(seq));
                show_alert(ev);
            }
        }
    };

    // ── Snapshot handler ──────────────────────────────────────────────

    let handle_snapshot = {
        let events = events;
        let set_is_moving = set_is_moving;
        let seeded_alerts = seeded_alerts;
        let last_alerted_seq = last_alerted_seq;
        let show_alert = show_alert;

        move |snap: MotionSnapshot| {
            let evs = snap.events;
            // Update pill state from the last event in the snapshot.
            let last = evs.last();
            let moving = last.map(|e| e.kind == "started").unwrap_or(false);
            set_is_moving.set(moving);
            events.set(evs.clone());

            let max_seq = evs.iter().fold(-1i64, |m, e| {
                let s = e.seq as i64;
                if s > m { s } else { m }
            });

            if !seeded_alerts.get_untracked() {
                // First snapshot after page load: absorb history silently.
                seeded_alerts.set(true);
                last_alerted_seq.set(max_seq);
            } else if max_seq < last_alerted_seq.get_untracked() {
                // seq reset = server restart. Re-alert on the latest
                // started event.
                last_alerted_seq.set(-1);
                let started_events: Vec<_> = evs.iter().filter(|e| e.kind == "started").collect();
                if let Some(latest) = started_events.last() {
                    last_alerted_seq.set(latest.seq as i64);
                    show_alert((*latest).clone());
                }
            } else {
                // Reconnect after network blip: alert on unseen `started`
                // events, only the newest one.
                let cur = last_alerted_seq.get_untracked();
                let unseen: Vec<&MotionEvent> = evs
                    .iter()
                    .filter(|e| e.kind == "started" && (e.seq as i64) > cur)
                    .collect();
                if let Some(latest) = unseen.last() {
                    last_alerted_seq.set(latest.seq as i64);
                    show_alert((*latest).clone());
                }
            }
        }
    };

    // ── Open / close EventSource ──────────────────────────────────────

    let close_es = {
        let es = es;
        let clear_alert = clear_alert.clone();
        move || {
            if let Some(source) = es.get_untracked() {
                source.close();
            }
            es.set(None);
            clear_alert();
        }
    };

    let open_es = {
        let es = es;
        let handle_snapshot = handle_snapshot;
        let handle_motion_event = handle_motion_event;

        move || {
            // Close any existing connection first (idempotent).
            if let Some(source) = es.get_untracked() {
                source.close();
            }

            let source = web_sys::EventSource::new("/events")
                .expect("EventSource constructor should not fail");

            // `snapshot` event handler.
            let snapshot_cb = {
                let handle_snapshot = handle_snapshot.clone();
                Closure::wrap(Box::new(move |event: web_sys::MessageEvent| {
                    if let Some(data) = event.data().as_string() {
                        if let Ok(snap) = serde_json::from_str::<MotionSnapshot>(&data) {
                            handle_snapshot(snap);
                        }
                    }
                }) as Box<dyn FnMut(web_sys::MessageEvent)>)
            };
            let _ = source
                .add_event_listener_with_callback("snapshot", snapshot_cb.as_ref().unchecked_ref());
            snapshot_cb.forget();

            // `motion` event handler.
            let motion_cb = {
                let handle_motion_event = handle_motion_event.clone();
                Closure::wrap(Box::new(move |event: web_sys::MessageEvent| {
                    if let Some(data) = event.data().as_string() {
                        if let Ok(ev) = serde_json::from_str::<MotionEvent>(&data) {
                            handle_motion_event(ev);
                        }
                    }
                }) as Box<dyn FnMut(web_sys::MessageEvent)>)
            };
            let _ = source
                .add_event_listener_with_callback("motion", motion_cb.as_ref().unchecked_ref());
            motion_cb.forget();

            // EventSource auto-reconnects on its own — no need for an
            // onerror handler beyond letting it be.
            let onerror_cb = Closure::wrap(Box::new(move |_: web_sys::Event| {
                // Auto-reconnect — nothing to do.
            }) as Box<dyn FnMut(web_sys::Event)>);
            source.set_onerror(Some(onerror_cb.as_ref().unchecked_ref()));
            onerror_cb.forget();

            es.set(Some(source));
        }
    };

    // ── React to detect_is_open changes ───────────────────────────────

    let _open_close_effect = Effect::new({
        let open_es = open_es.clone();
        let close_es = close_es.clone();
        move |_| {
            if detect_is_open.get() {
                open_es();
            } else {
                close_es();
            }
        }
    });

    // ── Visibility change ─────────────────────────────────────────────
    //
    // `/detections` has its own visibility guard in `use_detections`;
    // `/events` stays open so the pill/list update live, but the server is
    // still running motion detection regardless (it rides on the detection
    // pipeline). On tab foreground the EventSource reconnects and replays
    // history via `snapshot` above.

    // ── Cleanup on drop ──────────────────────────────────────────────

    on_cleanup({
        let close_es = close_es.clone();
        move || {
            close_es();
        }
    });

    MotionState {
        is_moving: is_moving.into(),
        recent_text,
        alert_active,
        alert_time_text: alert_time_text.into(),
        sound_on,
        toggle_sound,
        total_count,
    }
}

// ── Formatting helpers ─────────────────────────────────────────────────

fn format_clock_time(unix_ms: u64) -> String {
    let ms = unix_ms as f64;
    let d = js_sys::Date::new(&ms.into());
    let hours = d.get_hours();
    let minutes = d.get_minutes();
    let seconds = d.get_seconds();
    let ampm = if hours >= 12 { "PM" } else { "AM" };
    let h12 = match hours {
        0 => 12,
        13..=23 => hours - 12,
        _ => hours,
    };
    format!("{h12}:{minutes:02}:{seconds:02} {ampm}")
}

fn format_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else {
        let s = ms as f64 / 1000.0;
        if s >= 10.0 {
            format!("{}s", s.round() as u64)
        } else {
            format!("{:.1}s", s)
        }
    }
}
