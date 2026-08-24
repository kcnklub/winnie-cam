# Phase 0 — Shared API Types

**Objective:** Extract every JSON shape crossing the HTTP boundary into a shared
Rust crate with serde derives. This gives compile-time verification that the
frontend and backend agree on field names, types, and optionality.

**Rationale:** The server's JSON shapes are currently constructed with ad-hoc
`format!()` strings or `serde_json::json!()` macros. The JS parses them with
bare `JSON.parse()` into untyped objects. A typo in a field name on either
side is a runtime bug. This phase gives us typed structs both sides can use
*before* any Leptos work begins, and the server gets cleaner JSON construction
as a side benefit.

## Prerequisites

- None (standalone phase, no frontend tooling needed)

## Task List

### 0.1 — Create the shared-types crate

- [ ] Create `src/shared-types/Cargo.toml` with:
  ```toml
  [package]
  name = "shared-types"
  version = "0.1.0"
  edition = "2024"

  [dependencies]
  serde = { version = "1", features = ["derive"] }
  serde_json = "1"
  ```
- [ ] Add `shared-types` as a path dependency in the root `Cargo.toml`:
  ```toml
  [dependencies]
  shared-types = { path = "src/shared-types" }
  ```
- [ ] Create `src/shared-types/src/lib.rs`

### 0.2 — Define `/healthz` response type

- [ ] Add `HealthzResponse` struct with every field the server currently emits:
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct HealthzResponse {
      pub uptime_secs: f64,
      pub frames_captured: u64,
      pub subscribers: u64,
      pub seconds_since_last_frame: Option<f64>,
      pub detect: String,
      pub detect_ms: Option<f64>,
      pub seconds_since_last_detection: Option<f64>,
      pub motion: String,
      pub motion_events: u64,
  }
  ```

### 0.3 — Define detection SSE payload type

- [ ] Add `DetectionPayload` and `DetectionBox` structs:
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct DetectionPayload {
      pub w: u32,
      pub h: u32,
      pub seq: u64,
      pub ms: f64,
      pub dets: Vec<DetectionBox>,
  }

  #[derive(Debug, Clone, Copy, Serialize, Deserialize)]
  pub struct DetectionBox {
      pub x: f32,
      pub y: f32,
      pub w: f32,
      pub h: f32,
      pub label: (to fill in from the actual wire format),
      pub score: f32,
  }
  ```
  **Note:** Inspect the actual JSON emitted by `DetectionHub::publish` in
  `src/detect/hub.rs` to determine the exact field names — the payload
  shown above uses the JS-side field names but should match the server
  exactly.

### 0.4 — Define motion event SSE payload type

- [ ] Add `MotionEvent` struct matching the wire format from
  `src/detect/motion/hub.rs`'s `event_json()`:
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct MotionEvent {
      pub kind: String,        // "started" | "stopped"
      pub at: u64,             // unix ms
      pub seq: u64,
      pub duration_ms: Option<u64>,
  }
  ```
- [ ] Add `MotionSnapshot` for the `snapshot` SSE event:
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct MotionSnapshot {
      pub events: Vec<MotionEvent>,
  }
  ```

### 0.5 — Define `/api/config` types

- [ ] Add `CameraConfig` (GET response):
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct CameraConfig {
      pub backend: String,    // "rpicam" | "v4l2"
      pub width: u32,
      pub height: u32,
      pub fps: u32,
      pub quality: u8,
      pub hflip: bool,
      pub vflip: bool,
  }
  ```
- [ ] Add `VideoSettingsUpdate` (PUT request body) — this already exists in
  `src/config.rs`. Move it to `shared-types` instead.
- [ ] Add an error response type for the settings endpoint:
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct ErrorResponse {
      pub error: String,
  }
  ```

### 0.6 — Refactor backend to use shared types

- [ ] Replace the ad-hoc `format!()` in `web::healthz` with
  `serde_json::to_string(&HealthzResponse { ... })`
- [ ] Replace `motion_hub.recent_json()` calls with typed equivalents
  (or keep the helper but have it go through the shared type)
- [ ] Move `VideoSettingsUpdate` from `src/config.rs` to `shared-types`
  and re-export or use the shared one
- [ ] Ensure all existing tests still pass

### 0.7 — Verification

- [ ] `cargo build` succeeds from workspace root
- [ ] `cargo test` passes
- [ ] Run the server (`cargo run --release`) and verify every endpoint
  still returns equivalent JSON:
  - [ ] `curl http://localhost:8080/healthz | jq` — all fields present
  - [ ] `curl http://localhost:8080/api/config | jq` — all fields present
  - [ ] Open the current UI in a browser — everything works unchanged