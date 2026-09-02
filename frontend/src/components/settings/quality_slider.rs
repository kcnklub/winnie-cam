use leptos::prelude::*;

/// JPEG quality range accepted by the server (`VideoSettingsUpdate::validate`).
const QUALITY_MIN: u8 = 1;
const QUALITY_MAX: u8 = 100;

/// JPEG quality row: a range slider with a live numeric readout.
///
/// Hidden on the v4l2 backend — a USB webcam encodes MJPEG on-device, so
/// the quality setting has nothing to act on.
#[component]
pub fn QualitySlider(value: RwSignal<u8>, visible: Memo<bool>) -> impl IntoView {
    let on_slide = move |ev| {
        if let Ok(n) = event_target_value(&ev).parse::<u8>() {
            value.set(n);
        }
    };

    view! {
        <div class="settings-row" hidden={move || !visible.get()}>
            <label class="settings-label" for="s-quality">"Quality"</label>
            <div class="settings-range-wrap">
                <input
                    id="s-quality"
                    class="settings-range"
                    type="range"
                    min={QUALITY_MIN.to_string()}
                    max={QUALITY_MAX.to_string()}
                    prop:value={move || value.get().to_string()}
                    on:input=on_slide
                />
                <output class="settings-range-out" for="s-quality">
                    {move || value.get().to_string()}
                </output>
            </div>
        </div>
    }
}
