use crate::hooks::use_healthz::HealthzState;
use leptos::prelude::*;

/// Footer row: uptime stat, FPS, viewer count, presence stat, motion
/// toggle, and a theme-toggle placeholder button.
///
/// Motion props are `Option<...>` so the component works without motion
/// support (just hidden). The caller always passes them in Phase 4+.
#[component]
pub fn Footer(
    healthz: HealthzState,
    #[prop(optional)] presence_text: Option<Memo<String>>,
    #[prop(optional)] motion_is_moving: Option<ReadSignal<bool>>,
    #[prop(optional)] motion_panel_expanded: Option<RwSignal<bool>>,
    #[prop(optional)] detect_available: Option<Memo<bool>>,
) -> impl IntoView {
    let motion_label = Memo::new(move |_| {
        match motion_is_moving {
            Some(ref s) if s.get() => "Moving",
            _ => "Still",
        }
    });

    let motion_btn_class = Memo::new(move |_| {
        match motion_is_moving {
            Some(ref s) if s.get() => "motion-btn active",
            _ => "motion-btn",
        }
    });

    let motion_visible = Memo::new(move |_| {
        detect_available.as_ref().map(|m| m.get()).unwrap_or(false)
    });

    let on_motion_click = {
        let expanded = motion_panel_expanded;
        move |_| {
            if let Some(ref expanded) = expanded {
                expanded.update(|v| *v = !*v);
            }
        }
    };

    view! {
        <footer class="stats">
            <span class="stat">{move || healthz.since_text.get()}</span>
            <span class="stat" hidden={move || healthz.fps_text.get().is_empty()}>
                {move || healthz.fps_text.get()}
            </span>
            <span class="stat">{move || healthz.viewers_text.get()}</span>
            {move || {
                presence_text.as_ref().map(|m| {
                    let text = m.get();
                    let is_empty = text.is_empty();
                    view! {
                        <span
                            class="stat"
                            id="stat-presence"
                            hidden={is_empty}
                        >
                            {text}
                        </span>
                    }
                })
            }}
            <span class="stat" hidden={move || !motion_visible.get()}>
                <button
                    class={motion_btn_class}
                    type="button"
                    aria-expanded={move || {
                        motion_panel_expanded
                            .as_ref()
                            .map(|e| if e.get() { "true" } else { "false" })
                            .unwrap_or("false")
                    }}
                    aria-controls="motion-panel"
                    on:click=on_motion_click
                >
                    <span class="dot"></span>
                    <span>{motion_label}</span>
                </button>
            </span>
            // Theme toggle — placeholder shell for Phase 6.
            <button
                class="ghost"
                type="button"
                aria-label="Switch to light theme"
                title="Theme toggle (coming in Phase 6)"
            >
                <svg
                    width="16"
                    height="16"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.8"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    aria-hidden="true"
                >
                    <path d="M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z"/>
                </svg>
            </button>
        </footer>
    }
}