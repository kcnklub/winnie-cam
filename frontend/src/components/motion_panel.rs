use crate::hooks::use_events::MotionState;
use leptos::prelude::*;

/// Expandable panel listing recent motion activity (newest first, capped
/// at 6 entries). Hidden by default; toggled from the footer button.
///
/// Also carries the primary mute-toggle button (the alert banner has a
/// secondary copy so mute stays reachable in fullscreen / immersive mode).
#[component]
pub fn MotionPanel(
    motion: MotionState,
    expanded: RwSignal<bool>,
    available: Memo<bool>,
) -> impl IntoView {
    let panel_hidden = Memo::new(move |_| !available.get() || !expanded.get());

    let sound_icon = Memo::new(move |_| {
        if motion.sound_on.get() {
            // Speaker unmuted icon
            r#"<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M4 9v6h4l5 4V5L8 9H4z"/><path d="M17 8a5 5 0 0 1 0 8" opacity=".8"/>
            </svg>"#
        } else {
            // Speaker muted icon
            r#"<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M4 9v6h4l5 4V5L8 9H4z"/><path d="M17 8a5 5 0 0 1 0 8" opacity=".5"/><line x1="22" y1="3" x2="2" y2="21"/>
            </svg>"#
        }
    });

    let mute_label = Memo::new(move |_| {
        if motion.sound_on.get() {
            "Mute motion alert sound"
        } else {
            "Unmute motion alert sound"
        }
    });

    view! {
        <div
            class="motion-panel"
            id="motion-panel"
            role="region"
            aria-label="Recent motion activity"
            hidden={panel_hidden}
        >
            <div class="motion-panel-head">
                <p class="motion-panel-title">"Recent activity"</p>
                <span class="motion-total">
                    {move || {
                        let n = motion.total_count.get();
                        if n > 0 { format!("{n} total") } else { String::new() }
                    }}
                </span>
                <button
                    class="alert-sound-btn"
                    type="button"
                    aria-pressed={move || if motion.sound_on.get() { "true" } else { "false" }}
                    aria-label={mute_label}
                    on:click={move |_| motion.toggle_sound.run(())}
                >
                    <span inner_html={sound_icon}></span>
                </button>
            </div>
            <ul id="motion-list">
                {move || {
                    let lines = motion.recent_text.get();
                    if lines.is_empty() {
                        view! {
                            <li class="motion-empty">"No motion yet"</li>
                        }.into_any()
                    } else {
                        lines
                            .iter()
                            .cloned()
                            .map(|l| {
                                view! { <li>{l}</li> }
                            })
                            .collect::<Vec<_>>()
                            .into_any()
                    }
                }}
            </ul>
        </div>
    }
}
