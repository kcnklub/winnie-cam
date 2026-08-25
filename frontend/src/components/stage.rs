use crate::hooks::use_mjpeg::UseMjpegReturn;
use crate::state::ConnectionState;
use leptos::html::{Canvas, Div, Img};
use leptos::prelude::*;

/// Video stage: the MJPEG `<img>`, a placeholder overlay, the detection
/// canvas, an exit-immersive button, and (later) a motion alert.
///
/// `NodeRef` props are created by the parent (`App`) and passed through so
/// the overlay hook can use them; `Stage` only assigns them to DOM elements.
#[component]
pub fn Stage(
    mjpeg: UseMjpegReturn,
    canvas_ref: NodeRef<Canvas>,
    img_ref: NodeRef<Img>,
    stage_ref: NodeRef<Div>,
) -> impl IntoView {
    let connection = mjpeg.connection;
    let had_frames = mjpeg.had_frames;
    let placeholder = mjpeg.placeholder;
    let img_src = mjpeg.img_src;
    let on_load = mjpeg.on_load;
    let on_error = mjpeg.on_error;

    let feed_hidden = Memo::new(move |_| connection.get() != ConnectionState::Live);
    let placeholder_hidden =
        Memo::new(move |_| {
            connection.get() == ConnectionState::Live && had_frames.get()
        });

    let ph = Memo::new(move |_| placeholder.get());

    view! {
        <div class="stage" id="stage" node_ref={stage_ref}>
            <img
                id="feed"
                src={img_src}
                node_ref={img_ref}
                on:load={move |ev| on_load.run(ev)}
                on:error={move |ev| on_error.run(ev.into())}
                alt="Live view of Winnie's room"
                hidden={feed_hidden}
            />
            <canvas
                id="overlay"
                aria-hidden="true"
                node_ref={canvas_ref}
                hidden
            ></canvas>
            <div class="placeholder" hidden={placeholder_hidden}>
                <svg
                    width="56"
                    height="56"
                    viewBox="0 0 64 64"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.5"
                    stroke-linecap="round"
                    aria-hidden="true"
                >
                    <path d="M40 34A14 14 0 1 1 28 12a11 11 0 0 0 12 22z" opacity=".8"/>
                    <path d="M14 46a24 24 0 0 1 36 0" opacity=".35"/>
                </svg>
                <p class="ph-title">{move || ph.get().title}</p>
                <p class="ph-sub">{move || ph.get().sub}</p>
            </div>
            <button
                id="exit-immersive"
                class="stage-btn"
                type="button"
                aria-label="Exit fullscreen"
                hidden
            >
                <svg
                    width="18"
                    height="18"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    aria-hidden="true"
                >
                    <path d="M6 6l12 12M18 6L6 18"/>
                </svg>
            </button>
        </div>
    }
}