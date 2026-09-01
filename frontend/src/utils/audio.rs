//! WebAudio chime for motion alerts — ports `chime()`, `unlockAudio()`,
//! and `ensureAudioCtx()` from the vanilla JS, with the mute + localStorage
//! toggle.
//!
//! The chime is two sine tones (660 Hz then 880 Hz, 160 ms apart), each
//! lasting ~150 ms with a fast-attack/slow-decay gain envelope. A 10-second
//! minimum gap prevents overlapping chimes when motion starts and stops
//! repeatedly.

use leptos::prelude::*;
use std::sync::{Arc, Mutex};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::Closure;

const CHIME_MIN_GAP_MS: u64 = 10_000;

/// Bundle returned by [`use_chime`].
pub struct UseChimeReturn {
    /// Whether the alert sound is currently enabled.
    pub sound_on: RwSignal<bool>,
    /// Toggle sound on/off (writes through to localStorage).
    pub toggle_sound: Callback<(), ()>,
    /// Trigger the two-tone chime. No-op when sound is muted or the
    /// minimum gap hasn't elapsed.
    pub chime: Callback<(), ()>,
}

/// Reactive WebAudio chime lifecycle.
///
/// Creates an `AudioContext` on the first user gesture (click / keydown /
/// touchstart, `once` + `capture`), resumes it if suspended, and exposes a
/// `chime()` callback that is gated by `sound_on` and a 10-second minimum gap.
///
/// The mute toggle is persisted in `localStorage` under
/// `"winnie-alert-sound"` (defaults to `true` / `"1"`).
pub fn use_chime() -> UseChimeReturn {
    // ── Mute state (persisted to localStorage) ────────────────────────

    let sound_on: RwSignal<bool> = RwSignal::new(read_sound_on());

    let toggle_sound = {
        let sound_on = sound_on;
        Callback::new(move |()| {
            let next = !sound_on.get_untracked();
            sound_on.set(next);
            persist_sound_on(next);
        })
    };

    // ── AudioContext handle ───────────────────────────────────────────
    //
    // `Arc<Mutex<...>>` is `Send + Sync` (unlike `Rc<RefCell<...>>`),
    // which is required by `Callback::new`.  Wasm is single-threaded so
    // the Mutex never actually contends.

    let audio_ctx: Arc<Mutex<Option<web_sys::AudioContext>>> = Arc::new(Mutex::new(None));
    let last_chime_at: Arc<Mutex<u64>> = Arc::new(Mutex::new(0));

    // ── Helpers ───────────────────────────────────────────────────────

    let unlock_audio = {
        let audio_ctx = Arc::clone(&audio_ctx);
        move || {
            let mut guard = audio_ctx.lock().unwrap();
            if guard.is_none() {
                if let Ok(ctx) = web_sys::AudioContext::new() {
                    let _ = ctx.resume();
                    *guard = Some(ctx);
                }
            } else if let Some(ref ctx) = *guard {
                if ctx.state() == web_sys::AudioContextState::Suspended {
                    let _ = ctx.resume();
                }
            }
        }
    };

    // ── Global gesture listeners (once + capture) ─────────────────────

    let _ = {
        let unlock = unlock_audio.clone();
        let cb = Closure::wrap(Box::new(move || {
            unlock();
        }) as Box<dyn FnMut()>);

        let doc = document();
        let cb_ref = cb.as_ref().unchecked_ref();
        let _ = doc.add_event_listener_with_callback_and_bool("click", cb_ref, true);
        let _ = doc.add_event_listener_with_callback_and_bool("keydown", cb_ref, true);
        let _ = doc.add_event_listener_with_callback_and_bool("touchstart", cb_ref, true);

        // Idempotent on re-entrancy — harmless to keep alive forever.
        cb.forget();
    };

    // ── Chime callback ────────────────────────────────────────────────

    let chime = {
        let audio_ctx = Arc::clone(&audio_ctx);
        let last_chime_at = Arc::clone(&last_chime_at);
        let sound_on = sound_on;

        Callback::new(move |()| {
            if !sound_on.get_untracked() {
                return;
            }
            let now = js_sys::Date::now() as u64;
            {
                let mut guard = last_chime_at.lock().unwrap();
                if now.saturating_sub(*guard) < CHIME_MIN_GAP_MS {
                    return;
                }
                *guard = now;
            }

            let guard = audio_ctx.lock().unwrap();
            let ctx = match guard.as_ref() {
                Some(c) => c,
                None => return,
            };

            if ctx.state() == web_sys::AudioContextState::Suspended {
                let _ = ctx.resume();
            }

            let t0 = ctx.current_time();

            let frequencies: [f64; 2] = [660.0, 880.0];
            for (i, freq) in frequencies.iter().enumerate() {
                let osc = match ctx.create_oscillator() {
                    Ok(o) => o,
                    Err(_) => continue,
                };
                let gain = match ctx.create_gain() {
                    Ok(g) => g,
                    Err(_) => continue,
                };

                osc.set_type(web_sys::OscillatorType::Sine);
                osc.frequency().set_value(*freq as f32);

                let start = t0 + (i as f64) * 0.16;
                let gain_param = gain.gain();
                let _ = gain_param.set_value_at_time(0.0, start);
                let _ = gain_param.linear_ramp_to_value_at_time(0.2, start + 0.02);
                let _ = gain_param.linear_ramp_to_value_at_time(0.0, start + 0.14);

                let dest = ctx.destination();
                let _ = osc.connect_with_audio_node(&gain);
                let _ = gain.connect_with_audio_node(&dest);

                let _ = osc.start_with_when(start);
                let _ = osc.stop_with_when(start + 0.15);
            }
        })
    };

    UseChimeReturn {
        sound_on,
        toggle_sound,
        chime,
    }
}

// ── Helpers ────────────────────────────────────────────────────────────

fn document() -> web_sys::Document {
    web_sys::window()
        .expect("no window in browser")
        .document()
        .expect("no document")
}

fn read_sound_on() -> bool {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item("winnie-alert-sound").ok())
        .flatten()
        .as_deref()
        != Some("0")
}

fn persist_sound_on(on: bool) {
    let _ = web_sys::window().and_then(|w| {
        w.local_storage()
            .ok()
            .flatten()
            .map(|storage| storage.set_item("winnie-alert-sound", if on { "1" } else { "0" }))
    });
}
