use crate::state::ConnectionState;
use leptos::prelude::*;

/// Top bar: brand mark + "Winnie" label and a connection-status pill.
#[component]
pub fn Bar(connection: ReadSignal<ConnectionState>) -> impl IntoView {
    let status_class = Memo::new(move |_| {
        let cs = connection.get();
        format!("status {}", cs.css_class())
    });

    let status_label = Memo::new(move |_| connection.get().label().to_string());

    view! {
        <header class="bar">
            <div class="brand">
                <svg
                    class="mark"
                    width="20"
                    height="20"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.6"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    aria-hidden="true"
                >
                    <path d="M15.5 12.5A6 6 0 1 1 11.5 4a4.7 4.7 0 0 0 4 8.5z"/>
                </svg>
                <span>"Winnie"</span>
            </div>
            <p class={status_class} role="status" aria-live="polite">
                <span class="dot"></span>
                <span>{status_label}</span>
            </p>
        </header>
    }
}
