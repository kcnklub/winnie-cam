use crate::state::{ConnectionState, Placeholder};
use leptos::prelude::*;
use wasm_bindgen::JsCast;

const RECONNECT_DELAY_MS: u32 = 2000;

/// Return bundle from [`use_mjpeg`].
pub struct UseMjpegReturn {
    /// `src` attribute for the `<img>` element.
    pub img_src: RwSignal<String>,
    /// Current connection state for the status indicator.
    pub connection: ReadSignal<ConnectionState>,
    /// True once at least one MJPEG frame has loaded.
    pub had_frames: ReadSignal<bool>,
    /// Current placeholder text to show over the stage.
    pub placeholder: ReadSignal<Placeholder>,
    /// Call to force a reconnect (also exposed so healthz can trigger it).
    pub do_reconnect: Callback<()>,
    /// Call this from `on:load` on the `<img>` element.
    pub on_load: Callback<web_sys::Event>,
    /// Call this from `on:error` on the `<img>` element.
    pub on_error: Callback<web_sys::Event>,
}

/// Reactive MJPEG stream lifecycle.
pub fn use_mjpeg() -> UseMjpegReturn {
    let img_src = RwSignal::new(String::new());
    let (connection, set_connection) = signal(ConnectionState::Connecting);
    let (had_frames, set_had_frames) = signal(false);
    let (placeholder, set_placeholder) = signal(Placeholder::WAKING);

    // Monotonically-increasing cache-busting seed.
    let ts_seed: RwSignal<u64> = RwSignal::new(0);

    let do_reconnect = {
        let img_src = img_src;
        let set_connection = set_connection;
        let had_frames_rc = had_frames;
        let set_placeholder = set_placeholder;
        let ts_seed = ts_seed;

        Callback::new(move |()| {
            set_connection.set(ConnectionState::Reconnecting);
            let had = had_frames_rc.get_untracked();
            set_placeholder.set(if had {
                Placeholder::LOST
            } else {
                Placeholder::WAKING
            });

            ts_seed.update(|s| *s = s.wrapping_add(1));

            // Schedule reconnect after delay.
            let new_src = format!("/stream.mjpeg?ts={}", ts_seed.get_untracked());
            let img_src = img_src;
            wasm_bindgen_futures::spawn_local(async move {
                gloo_timers::future::TimeoutFuture::new(RECONNECT_DELAY_MS)
                    .await;
                img_src.set(new_src);
            });
        })
    };

    // On load: set live state, read aspect ratio.
    let on_load = {
        let set_connection = set_connection;
        let set_had_frames = set_had_frames;

        Callback::new(move |ev: web_sys::Event| {
            set_connection.set(ConnectionState::Live);
            set_had_frames.set(true);

            // Read natural dimensions and set CSS custom properties on
            // the parent .stage element.
            if let Some(img) = ev
                .target()
                .and_then(|t| t.dyn_into::<web_sys::HtmlImageElement>().ok())
            {
                let nw = img.natural_width();
                let nh = img.natural_height();
                if nw > 0 && nh > 0 {
                    if let Some(parent) = img.parent_element() {
                        let html_parent: &web_sys::HtmlElement =
                            parent.unchecked_ref();
                        let style =
                            web_sys::HtmlElement::style(html_parent);
                        let _ = style
                            .set_property("--ar", &format!("{nw} / {nh}"));
                        let _ = style.set_property(
                            "--ar-num",
                            &format!("{}", nw as f64 / nh as f64),
                        );
                    }
                }
            }
        })
    };

    // On error: trigger reconnect.
    let on_error = {
        let do_reconnect = do_reconnect;
        Callback::new(move |_: web_sys::Event| {
            do_reconnect.run(());
        })
    };

    // Kick off first connection after a microtask delay so the DOM is
    // already built by the time we set the src.
    let first_src = format!("/stream.mjpeg?ts={}", ts_seed.get_untracked());
    wasm_bindgen_futures::spawn_local(async move {
        img_src.set(first_src);
    });

    UseMjpegReturn {
        img_src,
        connection: connection.into(),
        had_frames: had_frames.into(),
        placeholder: placeholder.into(),
        do_reconnect,
        on_load,
        on_error,
    }
}