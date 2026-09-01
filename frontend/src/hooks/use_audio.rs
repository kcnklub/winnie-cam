//! Live microphone playback — drives the hidden `<audio>` element.
//!
//! Two transports, chosen per connection:
//!
//! - MSE (`utils::mse`): fetches `/audio` as a stream into a MediaSource and
//!   pins playback to the live edge. This is what keeps audio synchronized
//!   with the video; the direct transport below drifts behind.
//! - Direct `<audio src>`: the legacy progressive stream. Still used when
//!   MSE is unavailable (e.g. iOS Safari) or the server serves a format MSE
//!   can't demux (ADTS AAC). A browser element buffers a progressive stream
//!   without bound and never skips ahead after a stall, so audio lags —
//!   but it plays.
//!
//! Deliberately imperative rather than a reactive `src` binding, for two
//! reasons the reactive version gets wrong:
//!
//! - Browsers only allow `play()` from inside a user gesture. A signal write
//!   applies to the DOM in a later effect run, by which time the gesture is
//!   over and playback is blocked. So the toggle touches the element itself.
//! - Clearing `src` and calling `load()` is what actually aborts the HTTP
//!   request. That matters here more than it does for video: the server
//!   closes the microphone when the last listener disconnects, so a stream
//!   left open silently holds the mic on the Pi.

use leptos::html::Audio;
use leptos::prelude::*;
use std::sync::{Arc, Mutex};
use wasm_bindgen::JsCast;

use crate::utils::mse;

/// Matches `use_mjpeg`'s delay: long enough not to hammer a server that is
/// still starting the microphone, short enough not to be noticed.
const RECONNECT_DELAY_MS: u32 = 2000;

/// The `audio_format` healthz value for which the MSE transport is used.
const FORMAT_WEBM_OPUS: &str = "webm-opus";

/// Return bundle from [`use_audio`].
pub struct UseAudioReturn {
    /// Attach to the `<audio>` element.
    pub audio_ref: NodeRef<Audio>,
    /// Whether the user has Listen switched on.
    pub is_on: RwSignal<bool>,
    /// Toggle listening. Must be called from a click handler — see the
    /// module doc comment.
    pub toggle: Callback<(), ()>,
    /// Call this from `on:error` and `on:ended` on the `<audio>` element
    /// (and it is fired synthetically when an MSE session's stream ends).
    pub on_interrupted: Callback<web_sys::Event>,
}

/// Reactive microphone playback lifecycle.
///
/// `server_format` is healthz's `audio_format` value; `None` until the first
/// poll (or from a server too old to report it), which selects the direct
/// transport.
pub fn use_audio<S>(server_format: S) -> UseAudioReturn
where
    S: GetUntracked<Value = Option<String>> + Clone + Send + Sync + 'static,
{
    let audio_ref = NodeRef::<Audio>::new();
    let is_on = RwSignal::new(false);

    // Monotonically-increasing cache-busting seed, same as `use_mjpeg`'s:
    // without it a reconnect can be served from the browser's copy of the
    // stream it just lost. Doubles as the reconnect guard: each start() is
    // tagged with the seed it set, and a pending reconnect only fires if
    // the seed hasn't moved on since.
    let ts_seed: RwSignal<u64> = RwSignal::new(0);

    // The live MSE session, if any. `None` on the direct transport.
    // `Arc<Mutex<_>>` rather than `Rc<RefCell<_>>` because Callback captures
    // must be `Send + Sync`.
    let session: Arc<Mutex<Option<mse::Session>>> = Arc::new(Mutex::new(None));

    // Stops whatever is currently running, releasing the microphone
    // server-side. Dropping the source and reloading is what aborts the
    // request on the direct transport; Session::stop does the MSE part.
    let stop_running = {
        let session = Arc::clone(&session);

        move || {
            if let Ok(mut slot) = session.lock()
                && let Some(s) = slot.take()
            {
                s.stop();
            }
            if let Some(el) = audio_ref.get_untracked() {
                let _ = el.pause();
                let _ = el.remove_attribute("src");
                el.load();
            }
        }
    };

    let start = {
        let session = Arc::clone(&session);
        let stop_running = stop_running.clone();

        move || {
            let Some(el) = audio_ref.get_untracked() else {
                return;
            };
            stop_running();

            ts_seed.update(|seed| *seed = seed.wrapping_add(1));
            let seed = ts_seed.get_untracked();
            let url = format!("/audio?ts={seed}");

            // MSE only for the format it can demux; anything else (unknown,
            // ADTS AAC, MSE missing) gets the direct progressive stream.
            let use_mse = mse::supported()
                && server_format.get_untracked().as_deref() == Some(FORMAT_WEBM_OPUS);

            let attached = if use_mse {
                let media: &web_sys::HtmlMediaElement = el.unchecked_ref();
                let media = media.clone();
                let on_end = {
                    // Ends of the stream reach the reconnect path the same
                    // way element errors do: a synthetic `ended` event. The
                    // seed guard in `on_interrupted` drops stale reconnects.
                    let el = media.clone();
                    move || {
                        if let Ok(ev) = web_sys::Event::new("ended") {
                            let _ = el.dispatch_event(&ev);
                        }
                    }
                };
                match mse::Session::start(media, url.clone(), on_end) {
                    Ok(s) => {
                        if let Ok(mut slot) = session.lock() {
                            *slot = Some(s);
                        }
                        true
                    }
                    Err(err) => {
                        web_sys::console::error_1(
                            &format!("winnie-cam audio: MSE start failed: {err:?}").into(),
                        );
                        false
                    }
                }
            } else {
                false
            };

            if !attached {
                el.set_src(&url);
            }

            let _ = el.play();
        }
    };

    let stop = {
        let stop_running = stop_running.clone();

        move || {
            is_on.set(false);
            stop_running();
        }
    };

    let toggle = {
        let start = start.clone();
        Callback::new(move |()| {
            let next = !is_on.get_untracked();
            is_on.set(next);

            if next {
                start();
            } else {
                stop();
            }
        })
    };

    // Fires for element events (direct transport) and for MSE stream ends
    // (synthetic). Reconnect only if still wanted and the session hasn't
    // been replaced since — the seed at fire time identifies it.
    let on_interrupted = {
        let start = start.clone();

        Callback::new(move |_: web_sys::Event| {
            if !is_on.get_untracked() {
                return;
            }

            let seed = ts_seed.get_untracked();
            wasm_bindgen_futures::spawn_local({
                let start = start.clone();

                async move {
                    gloo_timers::future::TimeoutFuture::new(RECONNECT_DELAY_MS).await;
                    if is_on.get_untracked() && ts_seed.get_untracked() == seed {
                        start();
                    }
                }
            });
        })
    };

    UseAudioReturn {
        audio_ref,
        is_on,
        toggle,
        on_interrupted,
    }
}
