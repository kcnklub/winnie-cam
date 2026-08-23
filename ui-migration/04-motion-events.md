# Phase 4 — Motion Events + Alerts

**Objective:** Port the SSE-based motion event stream, the motion panel
(recent activity list), the motion alert banner (with WebAudio chime), and
the mute toggle to Leptos.

**Dependencies:** Phase 3 complete (detection overlay works, detection SSE
hook established — motion SSE follows the same EventSource pattern).

## What Gets Ported

From `src/index.html` JS:
- `startEvents()` / `stopEvents()` — EventSource for `/events` with
  `snapshot` and `motion` event handlers
- `handleMotionEvent()` — pushes events to list, updates moving/still state,
  triggers alert for `started` events
- `renderMotionList()` — caps at MAX_MOTION_ITEMS (6), newest first
- `setMotionState()` — "Moving" / "Still" pill indicator with amber dot
- `showAlert()` / `clearAlert()` — motion alert banner over the stage
- `chime()` — WebAudio two-tone chime with 10s minimum gap
- `unlockAudio()` / `ensureAudioCtx()` — AudioContext lifecycle
- `setTitleFlash()` — "(!) Motion —" title prefix
- Alert mute toggle (two buttons: one in panel, one on banner)
- `localStorage` for `winnie-alert-sound` preference
- `MAX_MOTION_ITEMS`, `ALERT_CHIME_MIN_GAP_MS`, `ORIGINAL_TITLE` constants
- Sequence number bookkeeping (seeded alerts, reconnection dedup)

From CSS:
- `.motion-panel`, `.motion-panel-head`, `.motion-panel-title`
- `.motion-btn`, `.motion-btn.active`, `.motion-btn .dot`
- `.motion-alert`, `.motion-alert-dot`, `.motion-alert-text`,
  `.motion-alert-btn`, `.motion-alert-mute`
- `.motion-total`, `#motion-list`, `li.motion-empty`
- `@keyframes alert-in`
- `.alert-sound-btn`

## Task List

### 4.1 — Motion events SSE hook (`use_events`)

- [ ] Create `frontend/src/hooks/use_events.rs`:
  ```rust
  pub fn use_events(
      detect_is_open: ReadSignal<bool>,  // detection must be on for motion
  ) -> MotionState { ... }
  ```
  where `MotionState` bundles:
  - `events: RwSignal<Vec<MotionEvent>>` — capped at 50 internally
  - `is_moving: ReadSignal<bool>` — derived from last event's `kind`
  - `event_count: ReadSignal<u64>` — total events seen
  - `recent_text: Memo<Vec<String>>` — formatted display lines (newest 6)
  - `alert_active: RwSignal<bool>`
  - `alert_time_text: ReadSignal<String>` — formatted clock time of alert
  - `sound_on: RwSignal<bool>`
  - `toggle_sound: Callback<(), ()>`

- [ ] EventSource on `/events`:
  - On `snapshot` event: parse `MotionSnapshot`, seed `events` list,
    set `is_moving` from last event, handle seq-based dedup
  - On `motion` event: parse `MotionEvent`, push to events vec,
    call `handle_motion_event()`
  - On error: EventSource auto-reconnects — nothing to do

- [ ] `handle_motion_event(ev: MotionEvent)`:
  - Push to events vec (cap at 50)
  - Update `is_moving` (true if `kind == "started"`)
  - If `kind == "started"`:
    - Update `last_alerted_seq`
    - Set `alert_active = true`
    - Set title to "(!) Motion — Winnie"
    - Trigger chime

- [ ] Sequence number bookkeeping (port the existing JS logic):
  - `seeded_alerts: bool` — first snapshot after page load absorbs history
    silently
  - `last_alerted_seq: i64` — tracks highest seq alerted on
  - On snapshot after reconnect: find unseen `started` events, alert on
    the newest one
  - If max seq < last_alerted_seq → server restarted, reset last_alerted_seq

- [ ] Start/stop tied to `detect_is_open`:
  - When detection opens → start EventSource
  - When detection closes → close EventSource, clear alert

### 4.2 — WebAudio chime (`utils/audio.rs`)

- [ ] Create `frontend/src/utils/audio.rs`:
  ```rust
  pub fn use_chime(sound_on: ReadSignal<bool>) -> Callback<(), ()> { ... }
  ```
- [ ] `AudioContext` creation and lifecycle:
  - Eagerly create `AudioContext` on first user gesture (click/keydown/touchstart)
  - Resume if suspended
  - `audio_ctx: Option<AudioContext>` stored in a `Rc<RefCell<...>>` or
    behind a signal
- [ ] `chime()` function:
  - Guard: return if `!sound_on` or `now - last_chime_at < 10000`
  - Create two oscillators at 660Hz and 880Hz, 160ms apart
  - Sine wave, gain envelope: ramp to 0.2 in 20ms, ramp to 0 at 140ms
  - `osc.start(t0 + offset)`, `osc.stop(t0 + offset + 0.15)`
  - Update `last_chime_at`
- [ ] `unlock_audio()` — called by global click/keydown/touchstart listeners
- [ ] Mute toggle:
  - Read initial value from `localStorage.getItem("winnie-alert-sound")`
  - Toggle writes to localStorage
  - Returns `sound_on` signal and `toggle_sound` callback

### 4.3 — Motion panel component

- [ ] Create `frontend/src/components/motion_panel.rs`:
  ```rust
  #[component]
  pub fn MotionPanel(motion: MotionState, available: ReadSignal<bool>) -> impl IntoView { ... }
  ```
- [ ] Panel header:
  - "Recent activity" title
  - Total event count
  - Mute toggle button (speaker icon, `aria-pressed` for mute state)
- [ ] Event list:
  - Hidden when no events ("No motion yet" placeholder)
  - Shows newest 6 events
  - Format: "HH:MM:SS — started moving" / "HH:MM:SS — settled (Xs)"
- [ ] Expand/collapse: panel hidden by default, toggle button in footer
  stats row shows/hides it
- [ ] When `available` becomes false: hide panel, collapse toggle

### 4.4 — Motion alert banner component

- [ ] Create `frontend/src/components/motion_alert.rs`:
  ```rust
  #[component]
  pub fn MotionAlert(
      alert_active: RwSignal<bool>,
      alert_time_text: ReadSignal<String>,
      sound_on: RwSignal<bool>,
      toggle_sound: Callback<(), ()>,
      on_acknowledge: Callback<(), ()>,
  ) -> impl IntoView { ... }
  ```
- [ ] Banner overlay inside the stage:
  - Hidden when `!alert_active`
  - Animated entrance (`@keyframes alert-in`)
  - Amber pulsing dot
  - "Motion detected HH:MM:SS" text
  - Mute button (speaker icon)
  - "Acknowledge" button → calls `on_acknowledge` (clears alert, restores
    title)
- [ ] `on_acknowledge`:
  - Set `alert_active = false`
  - Set `document.title` back to "Winnie"
- [ ] Title flash: when alert activates, set `document.title = "(!) Motion — Winnie"`
  via `web_sys::window().unwrap().document().unwrap().set_title(...)`

### 4.5 — Footer stats integration

- [ ] Update `Footer` component to include:
  - Motion toggle button: amber dot + "Moving" / "Still" label
  - Button click → expand/collapse motion panel
  - `aria-expanded` on toggle button
  - Hidden when `detect_available` is false (same gate as the detect toggle)

### 4.6 — Wire into App

- [ ] Update `App` component:
  - Call `use_events(detect_is_open)` 
  - Pass motion state to `<MotionPanel>` and `<MotionAlert>`
  - Wire alert banner inside `<Stage>` (it must be a child of `.stage` for
    fullscreen/immersive to work — see the CSS comment)
  - Wire footer motion toggle

### 4.7 — Verification

- [ ] `cargo build` && `trunk build` succeed
- [ ] Start server with `--detect --model <path>`:
  - [ ] Open new Leptos UI, toggle detection on
  - [ ] Walk in front of camera — the amber dot in the footer starts
    pulsing and shows "Moving"
  - [ ] The motion alert banner appears over the video with the time
  - [ ] A chime plays (two-tone, brief)
  - [ ] Document title changes to "(!) Motion — Winnie"
  - [ ] Click "Acknowledge" — banner disappears, title restores
  - [ ] Stop moving — after ~4s, dot shows "Still", a "settled" event
    appears in the panel
- [ ] Open the motion activity panel:
  - [ ] Shows recent events in reverse chronological order
  - [ ] Events persist across panel close/reopen
  - [ ] Total count increments correctly
- [ ] Mute toggle:
  - [ ] Click mute button in panel → chime stops playing
  - [ ] Click mute button on alert banner → same effect (they're synced)
  - [ ] State persists across page reload
- [ ] Alert chime minimum gap:
  - [ ] Trigger two motion events within 10s → only the first chimes
- [ ] Fullscreen/immersive:
  - [ ] Alert banner remains visible (it's inside `.stage`)
  - [ ] Mute button on banner works in fullscreen
- [ ] Tab visibility:
  - [ ] Hide tab while motion is active → come back → SSE reconnects,
    snapshot replays missed events, alert fires for any unseen `started`
  - [ ] Fresh page load with ongoing motion: no alert (history absorbed
    silently on first connect)
- [ ] Server restart while tab open:
  - [ ] SSE reconnects, snapshot replays, alert fires for active motion
- [ ] Side-by-side with old UI at `/` — motion behavior matches