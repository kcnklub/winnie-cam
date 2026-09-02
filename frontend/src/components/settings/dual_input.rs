use leptos::prelude::*;

/// Sentinel `<option>` value that swaps the dropdown for a free-text
/// number box.
const CUSTOM: &str = "custom";

/// A settings row whose value is either one of a few presets or a
/// hand-typed number. The dropdown and the number box are mutually
/// exclusive — exactly one is visible at a time.
///
/// ```text
///  preset mode            custom mode
///  ┌──────────┐           ┌──────────┐
///  │ 1280   ▾ │           │ 1000     │   <- type:number
///  └──────────┘           └──────────┘
/// ```
///
/// `is_custom` is owned by the caller, not by this component. Deriving it
/// here from `value` would be a bug: typing a preset number like `640`
/// into the box would snap the widget back to the dropdown mid-edit.
/// Only two things write it — the parent's populate effect, and the
/// dropdown's own `change` handler.
#[component]
pub fn DualInput(
    label: &'static str,
    input_id: &'static str,
    presets: &'static [u32],
    value: RwSignal<u32>,
    is_custom: RwSignal<bool>,
) -> impl IntoView {
    // The <select> is bound to the sentinel while custom mode is active,
    // so reopening the panel in preset mode shows the right option.
    let select_value = move || {
        if is_custom.get() {
            CUSTOM.to_string()
        } else {
            value.get().to_string()
        }
    };

    let on_select = move |ev| {
        let picked = event_target_value(&ev);

        if picked == CUSTOM {
            is_custom.set(true);
            return;
        }

        if let Ok(n) = picked.parse::<u32>() {
            value.set(n);
        }
    };

    // An empty or unparseable box reads as 0, which validation rejects —
    // matching the old UI's `Number('')`.
    let on_type = move |ev| {
        value.set(event_target_value(&ev).parse::<u32>().unwrap_or(0));
    };

    view! {
        <div class="settings-row">
            <label class="settings-label" for={input_id}>{label}</label>
            <div class="settings-input-group">
                <select
                    class="settings-select"
                    hidden={move || is_custom.get()}
                    prop:value={select_value}
                    on:change=on_select
                >
                    {presets
                        .iter()
                        .map(|p| {
                            view! { <option value={p.to_string()}>{p.to_string()}</option> }
                        })
                        .collect::<Vec<_>>()}
                    <option value={CUSTOM}>"custom"</option>
                </select>
                <input
                    id={input_id}
                    class="settings-num"
                    type="number"
                    min="1"
                    hidden={move || !is_custom.get()}
                    prop:value={move || value.get().to_string()}
                    on:input=on_type
                />
            </div>
        </div>
    }
}
