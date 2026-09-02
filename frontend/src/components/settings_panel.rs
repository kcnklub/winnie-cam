use crate::components::settings::dual_input::DualInput;
use crate::components::settings::quality_slider::QualitySlider;
use crate::components::settings::toggle_row::ToggleRow;
use crate::hooks::use_config::ConfigState;
use leptos::prelude::*;
use shared_types::VideoSettingsUpdate;

const WIDTH_PRESETS: &[u32] = &[320, 640, 800, 1280, 1920];
const HEIGHT_PRESETS: &[u32] = &[240, 480, 600, 720, 1080];
const FPS_PRESETS: &[u32] = &[5, 10, 15, 20, 30];

/// Backend kind for which `quality`, `hflip` and `vflip` apply.
const RPICAM: &str = "rpicam";

/// Camera settings form. Owns the six field signals plus the three
/// preset/custom mode flags; `config` supplies the values to seed them
/// with and carries the apply flow.
#[component]
pub fn SettingsPanel(is_open: RwSignal<bool>, config: ConfigState) -> impl IntoView {
    let width = RwSignal::new(0u32);
    let height = RwSignal::new(0u32);
    let fps = RwSignal::new(0u32);
    let quality = RwSignal::new(0u8);
    let hflip = RwSignal::new(false);
    let vflip = RwSignal::new(false);

    let width_custom = RwSignal::new(false);
    let height_custom = RwSignal::new(false);
    let fps_custom = RwSignal::new(false);

    // Port of `populateForm()`: refill every field whenever the server
    // sends a config — on open, and again from the PUT response. Because
    // a plain `set` always notifies, cancelling and reopening re-runs this
    // and discards the user's unapplied edits.
    let _populate = Effect::new(move |_| {
        let Some(cfg) = config.config.get() else {
            return;
        };

        width.set(cfg.width);
        height.set(cfg.height);
        fps.set(cfg.fps);
        quality.set(cfg.quality);
        hflip.set(cfg.hflip);
        vflip.set(cfg.vflip);

        width_custom.set(!WIDTH_PRESETS.contains(&cfg.width));
        height_custom.set(!HEIGHT_PRESETS.contains(&cfg.height));
        fps_custom.set(!FPS_PRESETS.contains(&cfg.fps));
    });

    // Before the first fetch lands the backend is unknown; show the
    // rpicam-only rows, as the old markup did.
    let is_rpicam = Memo::new(move |_| {
        config
            .config
            .get()
            .map(|cfg| cfg.backend == RPICAM)
            .unwrap_or(true)
    });

    let backend_tag = Memo::new(move |_| {
        config
            .config
            .get()
            .map(|cfg| cfg.backend)
            .unwrap_or_default()
    });

    // Port of `readForm()`.
    let on_apply = move |_| {
        config.apply.run(VideoSettingsUpdate {
            width: Some(width.get_untracked()),
            height: Some(height.get_untracked()),
            fps: Some(fps.get_untracked()),
            quality: Some(quality.get_untracked()),
            hflip: Some(hflip.get_untracked()),
            vflip: Some(vflip.get_untracked()),
        });
    };

    view! {
        <div
            class="settings-panel"
            role="region"
            aria-label="Camera settings"
            hidden={move || !is_open.get()}
        >
            <div class="settings-head">
                <p class="settings-title">"Camera settings"</p>
                <span class="settings-tag">{backend_tag}</span>
            </div>

            <div class="settings-body">
                <DualInput
                    label="Width"
                    input_id="s-width"
                    presets={WIDTH_PRESETS}
                    value={width}
                    is_custom={width_custom}
                />
                <DualInput
                    label="Height"
                    input_id="s-height"
                    presets={HEIGHT_PRESETS}
                    value={height}
                    is_custom={height_custom}
                />
                <DualInput
                    label="FPS"
                    input_id="s-fps"
                    presets={FPS_PRESETS}
                    value={fps}
                    is_custom={fps_custom}
                />
                <QualitySlider value={quality} visible={is_rpicam} />
                <ToggleRow label="Flip horizontal" value={hflip} visible={is_rpicam} />
                <ToggleRow label="Flip vertical" value={vflip} visible={is_rpicam} />
            </div>

            <div class="settings-actions">
                <span class="settings-error" hidden={move || config.error.get().is_none()}>
                    {move || config.error.get().unwrap_or_default()}
                </span>
                <button
                    class="btn"
                    type="button"
                    on:click={move |_| is_open.set(false)}
                >
                    "Cancel"
                </button>
                <button class="btn btn-wide" type="button" on:click=on_apply>
                    "Apply"
                </button>
            </div>
        </div>
    }
}
