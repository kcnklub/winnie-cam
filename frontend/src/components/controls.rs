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
