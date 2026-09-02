use components::bar::Bar;
use components::controls::{DetectToggle, SettingsButton};
use components::footer::Footer;
use components::motion_alert::MotionAlert;
use components::motion_panel::MotionPanel;
use components::settings_panel::SettingsPanel;
use components::stage::Stage;
use hooks::use_config::use_config;
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

    // ── Detection + overlay ──────────────────────────────────────────
    let det = use_detections(detect_available);

    // ── Motion events ──────────────────────────────────────────────
    // Tied to detection: events SSE opens/closes with the detect toggle.
    let motion = use_events(det.is_open.read_only());
    let motion_panel_expanded: RwSignal<bool> = RwSignal::new(false);

    use_overlay(canvas_ref, img_ref, stage_ref, det.latest_payload);

    // ── Camera settings ──────────────────────────────────────────────
    let settings_open: RwSignal<bool> = RwSignal::new(false);
    let config = use_config(Callback::new(move |()| settings_open.set(false)));

    // Port of `openSettings()` / `closeSettings()`. Opening refetches so
    // the form always starts from the server's truth, but doesn't wait on
    // that fetch — a slow or dead server must not block the panel.
    let toggle_settings = Callback::new(move |()| {
        if settings_open.get_untracked() {
            settings_open.set(false);
            return;
        }

        config.fetch.run(());
        config.error.set(None);
        config.status.set(None);
        settings_open.set(true);
    });

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
                <SettingsButton is_open={settings_open} on_toggle={toggle_settings} />
            </div>
            <SettingsPanel is_open={settings_open} config={config} />
            <div
                class="settings-panel-status"
                hidden={move || config.status.get().is_none()}
            >
                {move || config.status.get().unwrap_or_default()}
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
