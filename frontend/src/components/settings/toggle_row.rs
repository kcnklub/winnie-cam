use leptos::prelude::*;

/// An On/Off button row, used for the horizontal and vertical flips.
///
/// Hidden on the v4l2 backend — flipping is an rpicam capture option.
/// The pressed styling comes from `.settings-toggle[aria-pressed="true"]`,
/// so the ARIA state and the visual state can never disagree.
#[component]
pub fn ToggleRow(label: &'static str, value: RwSignal<bool>, visible: Memo<bool>) -> impl IntoView {
    view! {
        <div class="settings-row" hidden={move || !visible.get()}>
            <span class="settings-label">{label}</span>
            <button
                class="settings-toggle"
                type="button"
                aria-pressed={move || if value.get() { "true" } else { "false" }}
                on:click={move |_| value.update(|v| *v = !*v)}
            >
                {move || if value.get() { "On" } else { "Off" }}
            </button>
        </div>
    }
}
