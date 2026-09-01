//! MSE playback session for the live WebM/Opus stream.
//!
//! A direct `<audio src="/audio">` is a progressive stream with no live-edge
//! management: the element starts at t=0, buffers as far ahead as it likes,
//! and after any stall resumes where it was — which is how audio ends up
//! seconds behind video. This module replaces that for WebM/Opus:
//!
//! - fetches `/audio` as a stream and splits it into init + clusters with
//!   [`super::webm::WebmChunker`],
//! - feeds a sequence-mode `SourceBuffer` on a `MediaSource`,
//! - keeps playback pinned near the live edge with [`super::live_edge`].
//!
//! Fallback to the direct-src path happens one level up
//! ([`crate::hooks::use_audio`]) when MSE is unsupported or the server
//! serves a format MSE can't demux (ADTS AAC).
//!
//! ```text
//! fetch("/audio") ──▶ WebmChunker ──▶ append queue ──▶ SourceBuffer
//!                                                    (updateend pumps)
//!        keep-up tick ──▶ decide(lag) ──▶ seek to live edge / trim
//! ```

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use wasm_bindgen::{JsCast, JsValue};
use web_sys::{
    HtmlMediaElement, MediaSource, ReadableStreamDefaultReader, SourceBuffer,
    SourceBufferAppendMode, TimeRanges,
};

use super::live_edge::{self, Decision, KEEP_BEHIND_SECS};
use super::webm::{Chunk, WebmChunker};

/// The MIME type the SourceBuffer is created for. Must match what ffmpeg
/// produces for the server's `AudioFormat::WebmOpus`.
const MIME_WEBM_OPUS: &str = "audio/webm; codecs=\"opus\"";

/// How often the keep-up tick runs.
const KEEPUP_TICK_MS: u32 = 500;

/// State shared between the fetch loop, the append pump and the keep-up
/// tick. `Send + Sync` throughout so a `Session` can live inside Leptos
/// `Callback` captures.
struct Shared {
    el: HtmlMediaElement,
    media_source: MediaSource,
    url: String,
    buffer: Mutex<Option<SourceBuffer>>,
    queue: Mutex<VecDeque<Vec<u8>>>,
    stopped: AtomicBool,
    on_end: Arc<dyn Fn() + Send + Sync>,
}

impl Shared {
    /// Runs the end-of-stream callback unless the session was stopped on
    /// purpose (a manual Stop toggle must not trigger a reconnect).
    fn fire_on_end(&self) {
        if !self.stopped.load(Ordering::SeqCst) {
            (self.on_end)();
        }
    }

    /// Appends the next queued chunk if the buffer is idle. The next pump
    /// happens on the buffer's `updateend` event.
    fn pump(&self) {
        if self.stopped.load(Ordering::SeqCst) {
            return;
        }
        let Ok(buffer_guard) = self.buffer.lock() else {
            return;
        };
        let Some(buffer) = buffer_guard.as_ref() else {
            return;
        };
        if buffer.updating() {
            return;
        }
        let chunk = {
            let mut queue = self.queue.lock().expect("audio queue lock");
            queue.pop_front()
        };
        let Some(mut chunk) = chunk else {
            return;
        };
        if buffer.append_buffer_with_u8_array(&mut chunk).is_err() {
            // Most likely quota: the trim in the keep-up tick will make
            // room; put the chunk back so it isn't lost.
            if let Ok(mut queue) = self.queue.lock() {
                queue.push_front(chunk);
            }
        }
    }
}

/// True when this browser can play the stream through MSE.
pub fn supported() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    if !js_sys::Reflect::has(&window, &"MediaSource".into()).unwrap_or(false) {
        return false;
    }
    MediaSource::is_type_supported(MIME_WEBM_OPUS)
}

/// A running playback session. Call [`Session::stop`] to tear it down.
/// The event/interval closures live until the page does (they no-op once
/// stopped), so dropping the handle alone also keeps the session running.
pub struct Session {
    shared: Arc<Shared>,
    object_url: String,
}

impl Session {
    /// Starts fetching `url` into `el` through MSE. `on_end` runs when the
    /// server stream ends or the connection dies (the caller reconnects).
    pub fn start(
        el: HtmlMediaElement,
        url: String,
        on_end: impl Fn() + Send + Sync + 'static,
    ) -> Result<Session, JsValue> {
        let media_source = MediaSource::new()?;
        let object_url = web_sys::Url::create_object_url_with_source(&media_source)?;
        el.set_src(&object_url);

        let shared = Arc::new(Shared {
            el,
            media_source: media_source.clone(),
            url,
            buffer: Mutex::new(None),
            queue: Mutex::new(VecDeque::new()),
            stopped: AtomicBool::new(false),
            on_end: Arc::new(on_end),
        });

        // The SourceBuffer can only be created once the element has opened
        // the MediaSource; the browser fires `sourceopen` for that. The
        // closure is forgotten, but guards itself on `stopped`.
        let source_open_cb = {
            let shared = Arc::clone(&shared);
            wasm_bindgen::prelude::Closure::wrap(Box::new(move || {
                if let Err(err) = attach_buffer(&shared) {
                    shared.fire_on_end();
                    log_err(&err);
                }
            }) as Box<dyn FnMut()>)
        };
        media_source.set_onsourceopen(Some(source_open_cb.as_ref().unchecked_ref()));
        source_open_cb.forget();

        let tick = {
            let shared = Arc::clone(&shared);
            move || keep_up(&shared)
        };
        gloo_timers::callback::Interval::new(KEEPUP_TICK_MS, tick).forget();

        Ok(Session { shared, object_url })
    }

    pub fn stop(&self) {
        self.shared.stopped.store(true, Ordering::SeqCst);
        let _ = web_sys::Url::revoke_object_url(&self.object_url);
        let _ = self.shared.el.remove_attribute("src");
        self.shared.el.load();
    }
}

/// Creates the SourceBuffer, wires its update events, and starts fetching.
fn attach_buffer(shared: &Arc<Shared>) -> Result<(), JsValue> {
    shared.media_source.set_duration(f64::INFINITY);
    let buffer = shared.media_source.add_source_buffer(MIME_WEBM_OPUS)?;
    buffer.set_mode(SourceBufferAppendMode::Sequence);
    *shared.buffer.lock().expect("audio buffer lock") = Some(buffer.clone());

    // Pump more after every completed append/removal. Forgotten: the
    // closure lives until the page does; it no-ops once `stopped` is set.
    let update_end_cb = {
        let shared = Arc::clone(shared);
        wasm_bindgen::prelude::Closure::wrap(Box::new(move || {
            shared.pump();
        }) as Box<dyn FnMut()>)
    };
    buffer.set_onupdateend(Some(update_end_cb.as_ref().unchecked_ref()));
    update_end_cb.forget();

    spawn_fetch(Arc::clone(shared));

    Ok(())
}

/// Streams `/audio` through the chunker into the append queue for as long
/// as the server keeps writing. Fires `on_end` when it stops.
fn spawn_fetch(shared: Arc<Shared>) {
    wasm_bindgen_futures::spawn_local(async move {
        let result = run_fetch(&shared).await;

        if let Err(err) = result {
            log_err(&err);
        }
        shared.fire_on_end();
    });
}

async fn run_fetch(shared: &Arc<Shared>) -> Result<(), JsValue> {
    let Some(window) = web_sys::window() else {
        return Err(JsValue::from_str("no window"));
    };

    let resp = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(&shared.url))
        .await?
        .dyn_into::<web_sys::Response>()?;
    if !resp.ok() {
        return Err(JsValue::from_str(&format!(
            "/audio returned {}",
            resp.status()
        )));
    }

    let stream = resp.body().ok_or_else(|| JsValue::from_str("no body"))?;
    let reader = stream
        .get_reader()
        .dyn_into::<ReadableStreamDefaultReader>()
        .map_err(|_| JsValue::from_str("no stream reader"))?;

    let mut chunker = WebmChunker::new();
    loop {
        if shared.stopped.load(Ordering::SeqCst) {
            return Ok(());
        }

        let item = wasm_bindgen_futures::JsFuture::from(reader.read()).await?;
        if is_done(&item)? {
            return Ok(());
        }

        let value = js_sys::Reflect::get(&item, &"value".into())?;
        let bytes = js_sys::Uint8Array::unchecked_from_js(value).to_vec();

        // Queue in stream order; the chunker guarantees each queued append
        // starts on an init/cluster boundary even when reads don't.
        let chunks = chunker.push(&bytes);
        {
            let mut queue = shared.queue.lock().expect("audio queue lock");
            for chunk in chunks {
                match chunk {
                    Chunk::Init(bytes) | Chunk::Cluster(bytes) => queue.push_back(bytes),
                }
            }
        }
        shared.pump();
    }
}

fn is_done(item: &JsValue) -> Result<bool, JsValue> {
    Ok(js_sys::Reflect::get(item, &"done".into())?
        .dyn_into::<js_sys::Boolean>()?
        .value_of())
}

/// The keep-up tick: pin playback to the live edge and trim stale buffer.
fn keep_up(shared: &Arc<Shared>) {
    if shared.stopped.load(Ordering::SeqCst) {
        return;
    }

    let buffered_end = buffered_end(&shared.el);
    match live_edge::decide(shared.el.current_time(), buffered_end) {
        Decision::Hold => {}
        Decision::Seek(at) => shared.el.set_current_time(at),
    }

    trim_behind(shared);
}

/// End of the last buffered range — the newest decodable position.
fn buffered_end(el: &HtmlMediaElement) -> Option<f64> {
    let ranges: TimeRanges = el.buffered();
    let last = ranges.length().checked_sub(1)?;
    ranges.end(last).ok()
}

/// Drops buffered audio well behind playback so the SourceBuffer never hits
/// its quota. Removal is itself async; the next `updateend` pumps the queue.
fn trim_behind(shared: &Arc<Shared>) {
    let Ok(buffer_guard) = shared.buffer.lock() else {
        return;
    };
    let Some(buffer) = buffer_guard.as_ref() else {
        return;
    };
    if buffer.updating() {
        return;
    }

    let ranges = match buffer.buffered() {
        Ok(r) => r,
        Err(_) => return,
    };
    if ranges.length() == 0 {
        return;
    }

    let Ok(start) = ranges.start(0) else {
        return;
    };
    let cutoff = shared.el.current_time() - KEEP_BEHIND_SECS;
    if start < cutoff {
        let _ = buffer.remove(start, cutoff);
    }
}

fn log_err(err: &JsValue) {
    web_sys::console::error_1(&format!("winnie-cam audio: {err:?}").into());
}
