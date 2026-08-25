use leptos::prelude::*;

/// Alert banner that appears over the video stage when motion starts.
/// Must be a child of `.stage` so it survives fullscreen / immersive mode
/// (those modes hide `.bar` / `.controls` / `.stats` / `.motion-panel`
/// but not `.stage`'s own children).
#[component]
pub fn MotionAlert(
    alert_active: RwSignal<bool>,
    alert_time_text: ReadSignal<String>,
    sound_on: RwSignal<bool>,
    toggle_sound: Callback<(), ()>,
) -> impl IntoView {
    let sound_icon = Memo::new(move |_| {
        if sound_on.get() {
            r#"<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M4 9v6h4l5 4V5L8 9H4z"/><path d="M17 8a5 5 0 0 1 0 8" opacity=".8"/>
            </svg>"#
        } else {
            r#"<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M4 9v6h4l5 4V5L8 9H4z"/><path d="M17 8a5 5 0 0 1 0 8" opacity=".5"/><line x1="22" y1="3" x2="2" y2="21"/>
            </svg>"#
        }
    });

    let mute_label = Memo::new(move |_| {
        if sound_on.get() {
            "Mute motion alert sound"
        } else {
            "Unmute motion alert sound"
        }
    });

    let on_acknowledge = {
        let alert_active = alert_active;
        move |_| {
            alert_active.set(false);
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                doc.set_title("Winnie");
            }
        }
    };

    view! {
        <div
            class="motion-alert"
            id="motion-alert"
            role="alert"
            hidden={move || !alert_active.get()}
        >
            <span class="motion-alert-dot" aria-hidden="true"></span>
            <p class="motion-alert-text">
                "Motion detected "
                <span id="motion-alert-time">{move || alert_time_text.get()}</span>
            </p>
            <button
                class="motion-alert-mute"
                type="button"
                aria-pressed={move || if sound_on.get() { "true" } else { "false" }}
                aria-label={mute_label}
                on:click={move |_| toggle_sound.run(())}
            >
                <span inner_html={sound_icon}></span>
            </button>
            <button class="motion-alert-btn" type="button" on:click=on_acknowledge>
                "Acknowledge"
            </button>
        </div>
    }
}