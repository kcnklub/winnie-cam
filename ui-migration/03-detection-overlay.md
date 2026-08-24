# Phase 3 — Detection Overlay

**Objective:** Port the SSE-based detection overlay, the detect toggle,
visibility-change handling, and the imperative Canvas 2D bounding-box
drawing to Leptos.

**Dependencies:** Phase 2 complete (video feed renders, healthz poll
provides `detect_available` signal).

## What Gets Ported

From `src/index.html` JS:
- `startDetect()` / `stopDetect()` — opening/closing the `/detections` SSE
  connection, localStorage persistence
- `EventSource.onmessage` — parsing detection JSON, triggering redraw
- `drawOverlay()` / `syncCanvas()` / `imageRect()` — Canvas 2D bounding box
  rendering with object-fit:contain coordinate mapping
- `lastPayload` / `lastPayloadAt` / `DETECTION_STALE_MS` — stale detection
  timeout clearing the overlay
- `detectOpen()` — whether the detection SSE connection is active
- Detect toggle button state management
- `visibilitychange` listener — close SSE when tab is hidden, reopen on return
- `resize` / `ResizeObserver` — redraw on size/orientation change
- `readPaintTokens()` — reading CSS custom properties for drawing colors

From CSS:
- `#overlay` — absolutely positioned canvas over the video
- `.btn.on` — active toggle button state
- `#detect-toggle` — the detect button

## Task List

### 3.1 — Detection SSE hook (`use_detections`)

- [ ] Create `frontend/src/hooks/use_detections.rs`:
  ```rust
  pub fn use_detections(
      detect_available: ReadSignal<bool>,
  ) -> (
      RwSignal<bool>,                    // is_open
      ReadSignal<Option<DetectionPayload>>,  // latest payload
      Callback<(), ()>,                  // toggle function
  )
  ```
- [ ] `EventSource::new("/detections")` — typed via `web_sys::EventSource`
- [ ] `set_onmessage` callback:
  - Parse `event.data()` as JSON → `shared_types::DetectionPayload`
  - Update `latest_payload` signal
  - Reset stale timer (set `last_payload_at` to now)
- [ ] `set_onerror`: clear `latest_payload` (stops showing stale boxes)
- [ ] `toggle()`: if open → close EventSource, clear payload; if closed →
  create new EventSource
- [ ] Auto-start if `localStorage.getItem("winnie-detect") === "1"` on mount
- [ ] Persist toggle state to `localStorage`
- [ ] Cleanup on drop: close EventSource

### 3.2 — Canvas overlay hook (`use_overlay`)

- [ ] Create `frontend/src/utils/canvas.rs` (or `hooks/use_overlay.rs`):
  ```rust
  pub fn use_overlay(
      canvas_ref: NodeRef<Canvas>,
      img_ref: NodeRef<Img>,
      stage_ref: NodeRef<Div>,
      detection: ReadSignal<Option<DetectionPayload>>,
  )
  ```
- [ ] `create_effect` that redraws whenever:
  - `detection` signal changes
  - Window resize event fires
  - Fullscreen change event fires
- [ ] `image_rect()` function — computes the painted-image rect within the
  stage in local coordinates (object-fit:contain math):
  ```
  img client rect   →   subtract stage offset   →   contain letterbox math
  ```
  This is the most subtle coordinate math in the entire app. It must
  account for:
  - `img.getBoundingClientRect()` (viewport space)
  - `stage.getBoundingClientRect()` (viewport space)
  - `stage.clientLeft` / `stage.clientTop` (border offset)
  - `object-fit: contain` letterboxing (centered within stage)
  - `devicePixelRatio` for sharp canvas rendering
- [ ] `sync_canvas()` — positions/sizes the canvas over the image, handles
  DPR scaling, hides canvas when no image rect is available
- [ ] `draw_overlay()`:
  - Clear canvas
  - Check `detection` payload and `dets` array
  - For each box: map normalized (0..1) coordinates to rendered pixels
  - Draw rounded rectangle stroke in `--ok` color
  - Draw label + confidence text with background pill
  - Use `ctx.roundRect()` when available, fall back to `ctx.rect()`
- [ ] `read_paint_tokens()` — read CSS `--ok` and `--text` custom properties
  via `getComputedStyle`

### 3.3 — Detect toggle button

- [ ] Create `frontend/src/components/controls.rs` (skeleton — full controls
  come in Phase 6):
  ```rust
  #[component]
  pub fn DetectToggle(
      available: ReadSignal<bool>,
      is_open: RwSignal<bool>,
      on_toggle: Callback<(), ()>,
  ) -> impl IntoView { ... }
  ```
- [ ] Button shows/hides based on `available` (from healthz `detect == "ready"`)
- [ ] When available and toggled on: `.on` class, `aria-pressed="true"`
- [ ] Click → calls `on_toggle`
- [ ] If `available` becomes false while detection is open:
  auto-call `on_toggle` to close (safety net)

### 3.4 — Visibility change handling

- [ ] In `use_detections`, add event listener for `visibilitychange`:
  - When `document.hidden` → close EventSource (but keep `is_open` signal
    true so it reopens)
  - When `document.visible` and `is_open` → create new EventSource
  - This matches the existing JS behavior: SSE closes to save resources,
    but the toggle stays "on"
- [ ] Register listener on mount, deregister on cleanup

### 3.5 — Resize observers

- [ ] In `use_overlay`: add `ResizeObserver` on the stage element
- [ ] Add `window.addEventListener("resize", ...)` 
- [ ] Both trigger `draw_overlay()` via the same `create_effect`
- [ ] Clean up observers/listeners on drop

### 3.6 — Stale detection timeout

- [ ] In `use_detections`: `setInterval` every 1 second
- [ ] If `Date.now() - last_payload_at > DETECTION_STALE_MS` (5000ms):
  clear `latest_payload` signal → overlay clears itself
- [ ] This covers the case where SSE goes quiet without firing `onerror`

### 3.7 — Wire into App

- [ ] Update `App` component:
  - Add `NodeRef` for canvas, img, stage elements
  - Call `use_detections(detect_available)` from healthz hook
  - Call `use_overlay(...)` with the refs and detection signal
  - Add `<canvas id="overlay">` inside the stage div
  - Add `<DetectToggle>` in the controls row
- [ ] Add presence stat: "person detected" / "no person detected" / "checking…"
  in the footer stats row (reads from detection signal)

### 3.8 — Verification

- [ ] `cargo build` && `trunk build` succeed
- [ ] Start server with `--detect --model <path>`:
  - [ ] Detect toggle appears once model is loaded (healthz `detect: "ready"`)
  - [ ] Toggle off by default
- [ ] Click toggle on:
  - [ ] Overlay canvas appears over the video
  - [ ] If a person is in frame: bounding boxes drawn in green with labels
  - [ ] Boxes track movement (no flickering, no desync with video)
  - [ ] Presence stat in footer updates ("person detected" / "no person detected")
- [ ] Switch browser tab away and back:
  - [ ] Overlay reappears (SSE reconnects automatically)
  - [ ] Canvas redraws correctly after being hidden
- [ ] Resize browser window / rotate phone:
  - [ ] Canvas resizes with the video
  - [ ] Boxes remain aligned with the person
- [ ] Fullscreen the video (if implemented yet, or test via browser devtools):
  - [ ] Canvas overlay stays aligned
- [ ] Toggle off:
  - [ ] Overlay disappears
  - [ ] SSE connection closes (check devtools Network tab)
  - [ ] Toggle state persists across page reload
- [ ] Side-by-side with old UI at `/` — overlay behavior matches
- [ ] Verify at different camera resolutions (640x480, 1280x720, 1920x1080):
  - [ ] Box coordinates stay correct (normalized 0..1 mapping works)
- [ ] Verify stale detection: stop the camera — overlay should clear
  within 5 seconds of detection data stopping