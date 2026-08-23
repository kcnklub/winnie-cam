# Phase 5 — Settings Panel

**Objective:** Port the camera settings form, its preset/custom dual-input
pattern, backend-dependent show/hide logic, validation, and the
GET/PUT `/api/config` fetch flow to Leptos.

**Dependencies:** Phase 2 complete (video feed working — the settings panel
changes the camera configuration which restarts capture, so the video must
be able to reconnect cleanly after the restart).

## What Gets Ported

From `src/index.html` JS:
- `fetchConfig()` / `applySettings()` — GET and PUT `/api/config`
- `populateForm()` — fill all six fields from a parsed config
- `readForm()` — read all six current values from the form
- `syncPreset()` — show/hide custom number input when "custom" is picked
- `setToggle()` — On/Off toggle button state
- Preset value sets (320/640/800/1280/1920, 240/480/600/720/1080,
  5/10/15/20/30)
- Quality slider (1-100) + live output display
- Backend tag showing "rpicam" or "v4l2"
- rpicam-only row show/hide (quality, hflip, vflip)
- `showSettingsError()` / `showSettingsStatus()` — inline feedback
- `openSettings()` / `closeSettings()` — panel visibility
- Settings button in the controls row

From CSS:
- `.settings-panel`, `.settings-head`, `.settings-title`, `.settings-tag`
- `.settings-body`, `.settings-row`, `.settings-label`
- `.settings-select`, `.settings-num`, `.settings-range`,
  `.settings-range-wrap`, `.settings-range-out`
- `.settings-toggle`, `.settings-toggle[aria-pressed="true"]`
- `.settings-actions`, `.settings-error`
- `.settings-panel-status`
- `@keyframes settings-in`

## Task List

### 5.1 — Settings config hook (`use_config`)

- [ ] Create `frontend/src/hooks/use_config.rs`:
  ```rust
  use shared_types::{CameraConfig, VideoSettingsUpdate, ErrorResponse};

  pub fn use_config() -> (
      RwSignal<Option<CameraConfig>>,  // current config
      RwSignal<Option<String>>,        // error message
      RwSignal<Option<String>>,        // status message
      Callback<(), ()>,                // fetch_config
      Callback<CameraConfig, ()>,      // apply_settings
      ReadSignal<bool>,                // is_loading
  ) { ... }
  ```
- [ ] `fetch_config()`:
  - GET `/api/config`
  - Parse into `CameraConfig`
  - Update config signal on success
  - Set error on failure
- [ ] `apply_settings(update: CameraConfig)`:
  - Validates fields: width > 0, height > 0, fps > 0, quality 1-100
  - PUT `/api/config` with JSON body (serialized `VideoSettingsUpdate`)
  - On success: parse response into `CameraConfig`, set status
    "Applied — restarting capture…", auto-clear status after 3s
  - On 422: parse `ErrorResponse`, show error
  - On network error: show "Failed to reach server"
- [ ] Client-side validation (done before the PUT so we don't waste a
  round-trip):
  - All values must be positive integers
  - Quality must be 1-100

### 5.2 — Settings panel component

- [ ] Create `frontend/src/components/settings_panel.rs`:
  ```rust
  #[component]
  pub fn SettingsPanel(
      is_open: RwSignal<bool>,
      config: RwSignal<Option<CameraConfig>>,
      on_apply: Callback<(), ()>,  // triggers the apply flow
  ) -> impl IntoView { ... }
  ```
- [ ] Panel structure:
  - Header: "Camera settings" title + backend tag ("rpicam" / "v4l2")
  - Body: rows for width, height, fps, quality, hflip, vflip
  - Actions: Cancel + Apply buttons, error text inline
- [ ] Status: external element below the panel — "Applied — restarting
  capture…" feedback

### 5.3 — Preset + custom dual inputs

- [ ] Create `frontend/src/components/settings/dual_input.rs`:
  ```rust
  #[component]
  pub fn DualInput(
      label: &'static str,
      preset_values: &'static [u32],
      value: RwSignal<u32>,
  ) -> impl IntoView { ... }
  ```
- [ ] Behavior:
  - Dropdown shows preset list + "custom" option
  - When a preset is selected: dropdown visible, number input hidden,
    value = preset
  - When "custom" is selected: dropdown hidden, number input visible,
    value typed by user
  - `on:change` on dropdown → sync value, toggle visibility
  - `on:input` on number input → sync value
- [ ] Used three times: width (320/640/800/1280/1920), height
  (240/480/600/720/1080), fps (5/10/15/20/30)

### 5.4 — Quality slider

- [ ] Create `frontend/src/components/settings/quality_slider.rs`:
  ```rust
  #[component]
  pub fn QualitySlider(
      value: RwSignal<u8>,
      visible: ReadSignal<bool>,  // rpicam only
  ) -> impl IntoView { ... }
  ```
- [ ] `<input type="range" min="1" max="100">`
- [ ] Live `<output>` element showing current value
- [ ] `on:input` → update value signal and output text

### 5.5 — Toggle button (hflip/vflip)

- [ ] Create `frontend/src/components/settings/toggle_row.rs`:
  ```rust
  #[component]
  pub fn ToggleRow(
      label: &'static str,
      value: RwSignal<bool>,
      visible: ReadSignal<bool>,  // rpicam only
  ) -> impl IntoView { ... }
  ```
- [ ] Button shows "On" / "Off" with `aria-pressed`
- [ ] Click toggles the signal
- [ ] Active styling via `.settings-toggle[aria-pressed="true"]`

### 5.6 — Form population from fetched config

- [ ] When `openSettings()` is called:
  - Call `fetch_config()`
  - When config arrives, populate all five signals (width, height, fps,
    quality, hflip, vflip) and the backend tag
  - If fetch fails, keep whatever was in the form (user can still edit)
- [ ] `readForm()` — collect all six current values from the reactive
  signals into a `CameraConfig` struct
- [ ] `applySettings()`:
  - Read form
  - Client-side validate
  - Call API
  - On success: repopulate form from response, close panel
  - On error: show error text

### 5.7 — Wire into App

- [ ] Add settings button to the controls row:
  - Gear icon
  - `aria-pressed` when panel is open
  - `.on` class when open
- [ ] Add `<SettingsPanel>` below the controls (hidden when not open,
  `@keyframes settings-in` animation)
- [ ] Add status element below panel (hidden when no status message)
- [ ] Settings button click toggles panel

### 5.8 — Verification

- [ ] `cargo build` && `trunk build` succeed
- [ ] Start server:
  - [ ] Click settings gear → panel opens with slide-in animation
  - [ ] Backend tag shows correct value ("rpicam" or "v4l2")
- [ ] rpicam backend:
  - [ ] Quality slider row is visible
  - [ ] HFlip and VFlip toggle rows are visible
- [ ] V4L2 backend:
  - [ ] Quality, hflip, vflip rows are hidden
- [ ] Resolution presets:
  - [ ] Dropdowns show current values
  - [ ] Change a preset → number updates
  - [ ] Select "custom" → dropdown hides, number input appears
  - [ ] Type a custom value → value stored correctly
- [ ] Quality slider:
  - [ ] Range input works
  - [ ] Output text updates in real time as slider moves
- [ ] HFlip/VFlip toggles:
  - [ ] Click → toggles between "On" and "Off"
  - [ ] Style changes (accent color when on)
- [ ] Apply settings:
  - [ ] Change a value, click Apply
  - [ ] Panel closes, "Applied — restarting capture…" status appears
  - [ ] Status auto-clears after 3 seconds
  - [ ] Video reconnects with new settings (may see brief reconnect flash)
  - [ ] Reopen settings → values match what was applied
- [ ] Apply settings with invalid values:
  - [ ] Width = 0 → client-side validation catches it, error shown
    ("All values must be positive")
  - [ ] Quality = 0 or 101 → validation catches it
- [ ] Cancel:
  - [ ] Change values, click Cancel → panel closes, values don't apply
  - [ ] Reopen → original values are shown (fetched from server)
- [ ] Server unreachable:
  - [ ] Stop server, open settings → form stays as-is (no crash)
  - [ ] Try to apply → error shown
- [ ] Side-by-side with old UI at `/` — settings behavior matches:
  - [ ] Same preset values
  - [ ] Same validation messages
  - [ ] Same restart behavior