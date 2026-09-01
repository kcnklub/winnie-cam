use leptos::html::Audio;
use leptos::prelude::*;

/// Detect toggle button — shown only when detection is available
/// (healthz `detect == "ready"`). Full controls row (snapshot, dim,
/// fullscreen, settings) come in Phase 6.
#[component]
pub fn DetectToggle(
    available: Signal<bool>,
    is_open: RwSignal<bool>,
    on_toggle: Callback<(), ()>,
) -> impl IntoView {
    let btn_class = Memo::new(move |_| {
        if is_open.get() {
            "btn btn-wide on"
        } else {
            "btn btn-wide"
        }
    });

    let on_click = {
        let on_toggle = on_toggle;
        move |_| on_toggle.run(())
    };

    view! {
        <button
            id="detect-toggle"
            class={btn_class}
            type="button"
            aria-pressed={move || if is_open.get() { "true" } else { "false" }}
            title="Also enables motion alerts"
            hidden={move || !available.get()}
            on:click=on_click
        >
            <span class="dot"></span>
            <span>"Detect"</span>
        </button>
    }
}

/// Listen toggle button — shown only when the server was started with
/// `--audio` (healthz `audio != "off"`).
///
/// The hidden `<audio>` element lives here rather than in the stage: it has
/// no visual presence, and keeping it next to the button that drives it
/// makes the pairing obvious. `preload="none"` matters — without it the
/// browser would open the stream, and so the microphone, on page load.
#[component]
pub fn ListenToggle(
    available: Signal<bool>,
    is_on: RwSignal<bool>,
    on_toggle: Callback<(), ()>,
    audio_ref: NodeRef<Audio>,
    on_interrupted: Callback<web_sys::Event>,
) -> impl IntoView {
    let btn_class = Memo::new(move |_| {
        if is_on.get() {
            "btn btn-wide on"
        } else {
            "btn btn-wide"
        }
    });

    let on_click = {
        let on_toggle = on_toggle;
        move |_| on_toggle.run(())
    };

    view! {
        <button
            id="listen-toggle"
            class={btn_class}
            type="button"
            aria-pressed={move || if is_on.get() { "true" } else { "false" }}
            title="Play the microphone from the camera"
            hidden={move || !available.get()}
            on:click=on_click
        >
            <span class="dot"></span>
            <span>"Listen"</span>
        </button>
        <audio
            id="listen-audio"
            node_ref={audio_ref}
            preload="none"
            on:error={move |ev| on_interrupted.run(ev.into())}
            on:ended={move |ev| on_interrupted.run(ev)}
        ></audio>
    }
}
