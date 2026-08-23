# Phase 6 — Controls + Polish

**Objective:** Port the remaining controls and polish features: dim/brightness
cycling, fullscreen/immersive fallback, theme toggle, snapshot download, and
global event listeners (keyboard, resize, etc.).

**Dependencies:** Phases 2-5 complete. This phase is mostly independent of
the others — each control is a self-contained feature — but assumes the
`Stage`, `Bar`, and `Footer` components exist to wire into.

## What Gets Ported

From `src/index.html` JS:
- `toggleImmersive()` / `syncFullBtn()` — fullscreen API with iOS
  `.immersive` CSS fallback
- Dim cycle (three levels: 1 → 0.6 → 0.35), localStorage persistence,
  `applyDim()`
- Theme toggle (dark / light / system preference), `setTheme()`,
  `resolvedIsLight()`, `syncThemeColor()`, `prefers-color-scheme` listener
- Snapshot download: cache-busted `/snapshot.jpg` URL with timestamped
  filename (`winnie-YYYYMMDD-HHmmss.jpg`)
- `exit-immersive` button inside stage (X button in top-right)

From CSS:
- `.btn-icon`, `.btn.on` — icon-only buttons, active state
- `body.immersive` — CSS-only fullscreen fallback for iOS Safari
- `#exit-immersive` — close button in fullscreen
- `.stage:fullscreen` — fullscreen stage behavior
- `.ghost` — theme toggle button style
- `[data-theme="light"]` and `prefers-color-scheme: light` variants

## Task List

### 6.1 — Dim/brightness cycling

- [ ] Create `frontend/src/utils/dim.rs`:
  ```rust
  pub fn use_dim() -> (RwSignal<usize>, Callback<(), ()>, ReadSignal<bool>) { ... }
  ```
- [ ] Three levels: `1.0` (off), `0.6` (low), `0.35` (lowest)
- [ ] `dim_idx: RwSignal<usize>` — 0, 1, or 2
- [ ] On change: set `document.documentElement.style.setProperty("--dim", ...)`
- [ ] `is_dimmed: ReadSignal<bool>` — `dim_idx != 0`
- [ ] Button label cycles: "Dim screen" → "Dim: low" → "Dim: lowest"
- [ ] `aria-pressed` when dimmed
- [ ] `localStorage.setItem("winnie-dim", String(dim_idx))`
- [ ] Read initial value from localStorage on mount
- [ ] Moon icon button in the controls row
- [ ] Effect: the `--dim` CSS custom property applies `filter: brightness()`
  to the entire `.app` container

### 6.2 — Fullscreen + immersive fallback

- [ ] Create `frontend/src/utils/fullscreen.rs`:
  ```rust
  pub fn use_fullscreen(stage_ref: NodeRef<Div>) -> (
      ReadSignal<bool>,      // is_fullscreen
      Callback<(), ()>,      // toggle
      Callback<(), ()>,      // exit (used by the X button)
  ) { ... }
  ```
- [ ] `toggle()`:
  - If already fullscreen or immersive → exit
  - Try `stage.requestFullscreen()` with fallback:
    - On success → browser handles it
    - On `.catch()` → add `classList.add("immersive")` on body
    - If `requestFullscreen` doesn't exist → go straight to immersive
  - Also try `webkitRequestFullscreen` for older Safari
- [ ] `exit()`:
  - If `document.fullscreenElement` → `document.exitFullscreen()`
  - If `document.webkitFullscreenElement` → `document.webkitExitFullscreen()`
  - If `body.classList.contains("immersive")` → remove class
- [ ] `sync_state()`:
  - `is_fullscreen` signal tracks actual fullscreen state
  - Listen for `fullscreenchange` and `webkitfullscreenchange` events
  - Check `body.classList.contains("immersive")`
- [ ] Update `<Stage>` to show/hide the exit-immersive X button
- [ ] On fullscreen exit: trigger canvas overlay redraw (via a callback)

### 6.3 — Theme toggle

- [ ] Create `frontend/src/utils/theme.rs`:
  ```rust
  pub fn use_theme() -> (ReadSignal<bool>, Callback<(), ()>) { ... }
  // is_light: ReadSignal<bool>, toggle: Callback<(), ()>
  ```
- [ ] `resolved_is_light()`:
  - Check `document.documentElement.getAttribute("data-theme")`
  - If set: "light" → true, "dark" → false
  - If not set: `matchMedia("(prefers-color-scheme: light)").matches`
- [ ] `set_theme(mode: Option<&str>)`:
  - If `Some("light")` → set `data-theme="light"`, save to localStorage
  - If `Some("dark")` → set `data-theme="dark"`, save to localStorage
  - If `None` → remove `data-theme`, remove from localStorage (system
    preference)
- [ ] `toggle()`:
  - If currently light → set dark
  - If currently dark → set light
  - (Three-way toggle with system preference could be done as a future
    enhancement; for now match the existing two-way behavior)
- [ ] `sync_theme_color()`:
  - Update `<meta name="theme-color">` — `#f6f1ea` (light) or
    `#14101a` (dark)
- [ ] On mount:
  - Read localStorage, apply saved theme
  - Listen for `matchMedia("(prefers-color-scheme: light)").change` —
    when system preference changes and no manual override is set,
    update theme color and trigger repaint
- [ ] Theme toggle button in footer stats row:
  - Sun/moon icon
  - `aria-label` changes: "Switch to light theme" / "Switch to dark theme"
- [ ] When theme changes:
  - Re-read paint tokens (CSS `--ok`, `--text`) for canvas overlay
  - Redraw overlay if visible

### 6.4 — Snapshot download

- [ ] In the controls row, add a snapshot button:
  ```rust
  #[component]
  pub fn SnapshotButton(frames_available: ReadSignal<bool>) -> impl IntoView { ... }
  ```
- [ ] `<a>` tag with `href` and `download` attributes
- [ ] `href` is `/snapshot.jpg?ts=<timestamp>` (cache-busted)
- [ ] `download` is `winnie-YYYYMMDD-HHmmss.jpg`
- [ ] `aria-disabled="true"` when no frames available
- [ ] `preventDefault` on click when disabled
- [ ] Camera icon + "Snapshot" label
- [ ] Timestamp slug: `Date.now()` formatted as `YYYYMMDD-HHmmss`
  at click time
- [ ] Note: the `<a>` approach means the browser handles the download
  natively — no `fetch` → `Blob` → `createObjectURL` needed. Keep the
  existing pattern.

### 6.5 — Exit immersive button

- [ ] Inside the `<Stage>` component, add the X button:
  ```html
  <button class="stage-btn" hidden={!is_fullscreen} on:click={on_exit}>
    <!-- X icon SVG -->
  </button>
  ```
- [ ] Positioned top-right of the stage
- [ ] Only visible when fullscreen or immersive
- [ ] Semi-transparent with backdrop blur (matches existing style)

### 6.6 — Wire the controls row

- [ ] Update the controls row in `App` to include all buttons:
  ```
  [Detect Toggle] [Snapshot] [Dim] [Fullscreen] [Settings]
  ```
- [ ] Each is a separate component so they can be rearranged easily
- [ ] All buttons use the `.btn` base class with specific variants
  (`.btn-wide`, `.btn-icon`)

### 6.7 — Verification

- [ ] `cargo build` && `trunk build` succeed
- [ ] Dim:
  - [ ] Click dim button → screen dims to 60% brightness
  - [ ] Click again → 35% brightness
  - [ ] Click again → back to 100%
  - [ ] Button label and `aria-pressed` update correctly
  - [ ] Dim state persists across page reload
- [ ] Fullscreen:
  - [ ] Click fullscreen button → video fills screen
  - [ ] X button appears in top-right
  - [ ] Click X or fullscreen button again → exits fullscreen
  - [ ] Detection overlay stays aligned in fullscreen
  - [ ] Motion alert banner visible in fullscreen
  - [ ] On iOS Safari: `.immersive` CSS class applied, video fills screen,
    controls bar hidden
- [ ] Theme:
  - [ ] Click theme button → switches to light theme
  - [ ] All colors update (background, text, accents)
  - [ ] Theme color meta tag updates (browser chrome)
  - [ ] Detection overlay colors update to light-theme tokens
  - [ ] Theme persists across page reload
  - [ ] Click again → switches to dark theme
- [ ] Snapshot:
  - [ ] Click snapshot button → browser downloads `winnie-YYYYMMDD-HHmmss.jpg`
  - [ ] The JPEG is the most recent frame from the camera
  - [ ] Button disabled when no frames available
- [ ] Global keyboard:
  - [ ] (Future enhancement — not in current JS, but noted: Escape to exit
    fullscreen, F for fullscreen, D for detect toggle, M for mute)
- [ ] Side-by-side with old UI at `/`:
  - [ ] Dim levels cycle the same way
  - [ ] Fullscreen behaves the same
  - [ ] Theme toggle behaves the same
  - [ ] Snapshot naming matches
- [ ] Test on iOS Safari:
  - [ ] Fullscreen fallback works
  - [ ] Safe area insets respected
  - [ ] Status bar transparency correct