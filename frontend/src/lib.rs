use components::bar::Bar;
use components::controls::{DetectToggle, ListenToggle};
use components::footer::Footer;
use components::motion_alert::MotionAlert;
use components::motion_panel::MotionPanel;
use components::stage::Stage;
use hooks::use_audio::use_audio;
use hooks::use_detections::use_detections;
use hooks::use_events::use_events;
use hooks::use_healthz::use_healthz;
use hooks::use_mjpeg::use_mjpeg;
use hooks::use_overlay::use_overlay;
use leptos::html::{Canvas, Div, Img};
use leptos::prelude::*;

mod components;
mod hooks;
mod state;
pub mod utils;

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
    let audio_available = healthz.audio_available;

    // ── Microphone ───────────────────────────────────────────────────
    // Independent of detection: audio comes from its own device and its
    // own endpoint, so the two toggles never gate each other. The format
    // tells the hook whether MSE playback is possible.
    let audio = use_audio(healthz.audio_format);

    // ── Detection + overlay ──────────────────────────────────────────
    let det = use_detections(detect_available);

    // ── Motion events ──────────────────────────────────────────────
    // Tied to detection: events SSE opens/closes with the detect toggle.
    let motion = use_events(det.is_open.read_only());
    let motion_panel_expanded: RwSignal<bool> = RwSignal::new(false);

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
            >
                <MotionAlert
                    alert_active={motion.alert_active}
                    alert_time_text={motion.alert_time_text}
                    sound_on={motion.sound_on}
                    toggle_sound={motion.toggle_sound}
                />
            </Stage>
            <div class="controls">
                <DetectToggle
                    available={detect_available.into()}
                    is_open={det.is_open}
                    on_toggle={det.toggle}
                />
                <ListenToggle
                    available={audio_available.into()}
                    is_on={audio.is_on}
                    on_toggle={audio.toggle}
                    audio_ref={audio.audio_ref}
                    on_interrupted={audio.on_interrupted}
                />
            </div>
            <Footer
                healthz={healthz}
                presence_text={presence_text}
                motion_is_moving={motion.is_moving}
                motion_panel_expanded={motion_panel_expanded}
                detect_available={detect_available}
            />
            <MotionPanel motion={motion} expanded={motion_panel_expanded} available={detect_available} />
        </div>
    }
}
