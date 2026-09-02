use leptos::prelude::*;

/// Gear icon, drawn inline so the button carries no external asset.
const GEAR_ICON: &str = r#"<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>
</svg>"#;

/// Detect toggle button — shown only when detection is available
/// (healthz `detect == "ready"`). The rest of the controls row (snapshot,
/// dim, fullscreen) comes in Phase 6.
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

/// Gear button that opens and closes the camera settings panel.
#[component]
pub fn SettingsButton(is_open: RwSignal<bool>, on_toggle: Callback<(), ()>) -> impl IntoView {
    let btn_class = Memo::new(move |_| {
        if is_open.get() {
            "btn btn-icon on"
        } else {
            "btn btn-icon"
        }
    });

    view! {
        <button
            class={btn_class}
            type="button"
            aria-pressed={move || if is_open.get() { "true" } else { "false" }}
            aria-label="Camera settings"
            on:click={move |_| on_toggle.run(())}
            inner_html={GEAR_ICON}
        >
        </button>
    }
}
