# Phase 2 — Core Video + Status

**Objective:** Port the MJPEG video stream lifecycle, connection status
indicator, placeholder states, stats footer, and health-check polling from
the vanilla JS to Leptos components and hooks.

**Dependencies:** Phase 1 complete (scaffolding builds, `/v2` serves the
Leptos app, CSS is loaded).

## What Gets Ported

From `src/index.html` JS:
- `connect()` / `img.load` / `img.error` — MJPEG `<img>` lifecycle
- `setStatus()` — connection state indicator (dot + label)
- `setPlaceholder()` — four placeholder states (waking, lost, no camera,
  unreachable)
- `pollHealth()` — periodic `/healthz` fetch, stats display, stale detection
- `updateStats()` / `fmtSince()` / `fmtFps()` — footer stat computations
- `RECONNECT_DELAY_MS`, `STALE_AFTER_SECS`, `HEALTH_POLL_MS` — constants

From CSS:
- `.status`, `.dot`, `.is-live`, `.is-wait`, `.is-bad` — status indicator
- `.placeholder`, `.ph-title`, `.ph-sub` — placeholder overlay
- `.stage`, `#feed` — video stage and feed image
- `.stats`, `.stat` — footer stats row

## Task List

### 2.1 — Connection state model

- [ ] Create `frontend/src/state.rs` (or `connection.rs`):
  ```rust
  #[derive(Clone, Copy, PartialEq, Debug)]
  pub enum ConnectionState {
      Connecting,
      Live,
      Reconnecting,
      Stale,
      Offline,
  }

  #[derive(Clone, Debug)]
  pub struct Placeholder {
      pub title: &'static str,
      pub sub: &'static str,
  }
  ```
- [ ] Define the four placeholder variants as constants or as methods on
  `Placeholder`

### 2.2 — MJPEG stream hook (`use_mjpeg`)

- [ ] Create `frontend/src/hooks/use_mjpeg.rs`:
  - `pub fn use_mjpeg() -> (RwSignal<String>, /* img src */)`
  - On mount: set `img.src = "/stream.mjpeg?ts=${Date.now()}"`
  - `on:load` → set state to `Live`, show the image, hide placeholder
  - `on:error` → set state to `Reconnecting`, show placeholder, set
    reconnect timeout (2s)
  - Expose the current connection state as a signal for the bar component
  - Set `--ar` and `--ar-num` CSS custom properties on the stage from
    `img.naturalWidth` / `img.naturalHeight`
- [ ] The hook returns:
  - `img_src: RwSignal<String>` — for the `<img>` element's `src` attr
  - `connection: ReadSignal<ConnectionState>` — for the status indicator
  - `had_frames: ReadSignal<bool>` — whether at least one frame ever loaded

### 2.3 — Healthz polling hook (`use_healthz`)

- [ ] Create `frontend/src/hooks/use_healthz.rs`:
  - `pub fn use_healthz() -> HealthzState` where `HealthzState` bundles
    reactive signals for every `/healthz` field
  - Poll every 5 seconds using `gloo_timers::callback::Interval`
  - Use `gloo_net::http::Request::get("/healthz")` (relative URL — works
    because we're served from the same origin)
  - Parse response into `shared_types::HealthzResponse`
  - Compute derived signals:
    - `since_text: Memo<String>` — formatted "Watching since ..." / "Just started"
    - `fps_text: Memo<String>` — computed from frame count delta
    - `viewers_text: Memo<String>` — "N viewer(s)"
    - `is_stale: Memo<bool>` — `seconds_since_last_frame > STALE_AFTER_SECS`
    - `detect_available: Memo<bool>` — `detect == "ready"`
    - `motion_active: Memo<bool>` — `motion == "active"`
  - On stale detection: trigger reconnection (call into the mjpeg hook's
    reconnect mechanism)
  - Track `zeroFramesSince` — if no frames for 15s, show "no camera"
    placeholder
- [ ] Constants:
  ```rust
  const HEALTH_POLL_MS: u32 = 5000;
  const STALE_AFTER_SECS: f64 = 6.0;
  const NO_FRAMES_TIMEOUT_MS: u64 = 15_000;
  ```

### 2.4 — Bar component (brand + status)

- [ ] Create `frontend/src/components/bar.rs`:
  ```rust
  #[component]
  pub fn Bar(connection: ReadSignal<ConnectionState>) -> impl IntoView { ... }
  ```
- [ ] Renders:
  - Brand SVG icon + "Winnie" text
  - Status pill: connected dot + label text
  - CSS classes: `is-live`, `is-wait`, `is-bad` mapped from `ConnectionState`
- [ ] The dot uses `@keyframes pulse` animation from existing CSS

### 2.5 — Stage component (video feed + placeholder)

- [ ] Create `frontend/src/components/stage.rs`:
  ```rust
  #[component]
  pub fn Stage(
      img_src: RwSignal<String>,
      had_frames: ReadSignal<bool>,
      connection: ReadSignal<ConnectionState>,
      on_reconnect: Callback<()>,
  ) -> impl IntoView { ... }
  ```
- [ ] Renders:
  - `<div class="stage">` with aspect-ratio CSS vars
  - `<img id="feed">` with `src={img_src}` — hidden when `connection != Live`
  - Placeholder `<div>` with title/sub text, hidden when feed is showing
  - Placeholder text changes based on `connection` and `had_frames`
- [ ] `node_ref` on the `<img>` for later phases (canvas overlay, fullscreen)

### 2.6 — Footer stats component

- [ ] Create `frontend/src/components/footer.rs`:
  ```rust
  #[component]
  pub fn Footer(healthz: HealthzState) -> impl IntoView { ... }
  ```
- [ ] Renders:
  - "Watching since ..." stat
  - FPS stat (hidden when no fps data)
  - Viewer count stat
  - Theme toggle button (placeholder for Phase 6 — just the button shell
    for now)

### 2.7 — Wire the App component

- [ ] Update `frontend/src/lib.rs` (or `app.rs`) to compose:
  ```rust
  #[component]
  pub fn App() -> impl IntoView {
      let (img_src, connection, had_frames) = use_mjpeg();
      let healthz = use_healthz();

      view! {
          <div class="app">
              <Bar connection=connection />
              <Stage img_src=img_src had_frames=had_frames connection=connection />
              // controls (placeholder for Phase 6)
              <Footer healthz=healthz />
          </div>
      }
  }
  ```

### 2.8 — Verification

- [ ] `cargo build` succeeds from workspace root
- [ ] `cd frontend && trunk build` succeeds
- [ ] Visit `http://localhost:8080/v2/`:
  - [ ] Status dot shows "Connecting…" (amber, pulsing) initially
  - [ ] Video feed appears and status changes to "Live" (green dot)
  - [ ] Placeholder disappears when video loads
  - [ ] Footer shows "Watching since ...", FPS, viewer count
  - [ ] When camera is unplugged: error state → reconnecting state →
    placeholder updates to "Lost the picture"
  - [ ] When server is stopped: "Can't reach the monitor" placeholder
  - [ ] Aspect ratio is correct (no stretching)
- [ ] Compare behavior side by side with old UI at `/` — same reconnection
  timing, same placeholder text transitions
- [ ] Test on a phone/tablet over LAN
- [ ] Open multiple browser tabs — viewer count increments/decrements
  correctly
- [ ] Browser devtools: no WASM panics, no unhandled promise rejections in
  console