use leptos::prelude::*;

/// Top-level application component. Phase 1 is just a stub to prove the
/// Leptos + Trunk pipeline works end to end; real components arrive in
/// Phase 2+.
#[component]
pub fn App() -> impl IntoView {
    view! {
        <div class="app">
            <header class="bar">
                <div class="brand">
                    <svg class="mark" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                        <path d="M15.5 12.5A6 6 0 1 1 11.5 4a4.7 4.7 0 0 0 4 8.5z"/>
                    </svg>
                    <span>"Winnie"</span>
                </div>
            </header>
            <p style="text-align: center; color: var(--text-dim);">
                "Leptos frontend loading..."
            </p>
        </div>
    }
}
