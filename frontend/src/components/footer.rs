use crate::hooks::use_healthz::HealthzState;
use leptos::prelude::*;

/// Footer row: uptime stat, FPS, viewer count, and a theme-toggle
/// placeholder button.
#[component]
pub fn Footer(healthz: HealthzState) -> impl IntoView {
    view! {
        <footer class="stats">
            <span class="stat">{move || healthz.since_text.get()}</span>
            <span class="stat" hidden={move || healthz.fps_text.get().is_empty()}>
                {move || healthz.fps_text.get()}
            </span>
            <span class="stat">{move || healthz.viewers_text.get()}</span>
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
