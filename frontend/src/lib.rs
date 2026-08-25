use components::bar::Bar;
use components::controls::DetectToggle;
use components::footer::Footer;
use components::stage::Stage;
use hooks::use_detections::use_detections;
use hooks::use_healthz::use_healthz;
use hooks::use_mjpeg::use_mjpeg;
use hooks::use_overlay::use_overlay;
use leptos::html::{Canvas, Div, Img};
use leptos::prelude::*;

mod components;
mod hooks;
mod state;

/// Top-level application component.
#[component]
pub fn App() -> impl IntoView {
    // ── Node refs for overlay coordinate mapping ─────────────────────
    let canvas_ref = NodeRef::<Canvas>::new();
    let img_ref = NodeRef::<Img>::new();
    let stage_ref = NodeRef::<Div>::new();

    // ── Core hooks ───────────────────────────────────────────────────
    let mjpeg = use_mjpeg();
    let do_reconnect = mjpeg.do_reconnect;

    let healthz = use_healthz({
        let do_reconnect = do_reconnect;
        move || do_reconnect.run(())
    });

    let detect_available = healthz.detect_available;

    // ── Detection + overlay ──────────────────────────────────────────
    let det = use_detections(detect_available);

    use_overlay(canvas_ref, img_ref, stage_ref, det.latest_payload);

    // ── Presence text ────────────────────────────────────────────────
    let presence_text: Memo<String> = Memo::new(move |_| {
        if !det.is_open.get() {
            return String::new();
        }
        match det.latest_payload.get() {
            None => "checking\u{2026}".into(),
            Some(ref payload) => {
                if payload.dets.is_empty() {
                    "no person detected".into()
                } else {
                    "person detected".into()
                }
            }
        }
    });

    view! {
        <div class="app">
            <Bar connection={mjpeg.connection} />
            <Stage
                mjpeg={mjpeg}
                canvas_ref={canvas_ref}
                img_ref={img_ref}
                stage_ref={stage_ref}
            />
            <div class="controls">
                <DetectToggle
                    available={detect_available.into()}
                    is_open={det.is_open}
                    on_toggle={det.toggle}
                />
            </div>
            <Footer healthz={healthz} presence_text={presence_text} />
        </div>
    }
}
