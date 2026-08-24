use components::bar::Bar;
use components::footer::Footer;
use components::stage::Stage;
use hooks::use_healthz::use_healthz;
use hooks::use_mjpeg::use_mjpeg;
use leptos::prelude::*;

mod components;
mod hooks;
mod state;

/// Top-level application component.
#[component]
pub fn App() -> impl IntoView {
    let mjpeg = use_mjpeg();
    let do_reconnect = mjpeg.do_reconnect;

    let healthz = use_healthz({
        let do_reconnect = do_reconnect;
        move || do_reconnect.run(())
    });

    view! {
        <div class="app">
            <Bar connection={mjpeg.connection} />
            <Stage mjpeg={mjpeg} />
            // Controls row — placeholder for Phase 6.
            <Footer healthz={healthz} />
        </div>
    }
}
