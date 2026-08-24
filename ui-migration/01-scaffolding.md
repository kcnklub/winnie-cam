# Phase 1 — Project Scaffolding

**Objective:** Create the Leptos frontend crate, configure Trunk, and wire
Axum to serve the Trunk output alongside the existing API routes. The new UI
is served at `/v2` so both old and new can run side by side during
development.

**Dependencies:** Phase 0 (shared types) should be complete — the frontend
crate will depend on `shared-types`.

## Prerequisites

- [ ] Phase 0 complete (`shared-types` crate exists, backend uses it)
- [ ] Trunk installed, **pinned to `0.21.5`**: `cargo install trunk@0.21.5 --locked`
  (trunk `0.21.6`+ panics on root proxying — see `frontend/README.md`)
- [ ] `wasm32-unknown-unknown` target installed:
  `rustup target add wasm32-unknown-unknown`

## Task List

### 1.1 — Create the frontend crate

- [ ] Create `frontend/` at repo root as a Cargo workspace member
- [ ] Add to root `Cargo.toml`:
  ```toml
  [workspace]
  members = ["src/shared-types", "frontend"]
  ```
  (Adding the root crate itself as a member is optional — if it already
  has `[workspace]`, expand it; otherwise add it.)
- [ ] Create `frontend/Cargo.toml`:
  ```toml
  [package]
  name = "winnie-cam-frontend"
  version = "0.1.0"
  edition = "2024"

  [lib]
  crate-type = ["cdylib"]

  [dependencies]
  shared-types = { path = "../src/shared-types" }
  leptos = { version = "0.7", features = ["csr"] }
  wasm-bindgen = "0.2"
  web-sys = { version = "0.3", features = [
    "console",
    "Window",
    "Document",
    "Element",
    "HtmlElement",
    "HtmlImageElement",
    "HtmlCanvasElement",
    "CanvasRenderingContext2d",
    "EventSource",
    "EventSourceInit",
    "ResizeObserver",
    "ResizeObserverEntry",
    "FullscreenOptions",
    "AudioContext",
    "OscillatorNode",
    "GainNode",
    "DocumentVisibilityState",
    "DomRect",
    "DomRectReadOnly",
  ]}
  gloo-net = "0.6"
  gloo-timers = { version = "0.3", features = ["futures"] }
  serde = { version = "1", features = ["derive"] }
  serde_json = "1"
  serde-wasm-bindgen = "0.6"
  ```

### 1.2 — Create the Trunk entry point

- [ ] Create `frontend/index.html` — a minimal Trunk shell:
  ```html
  <!doctype html>
  <html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
    <meta name="apple-mobile-web-app-capable" content="yes">
    <meta name="apple-mobile-web-app-status-bar-style" content="black-translucent">
    <meta id="theme-color-meta" name="theme-color" content="#14101a">
    <title>Winnie</title>
    <link rel="icon" href="data:image/svg+xml,..."> <!-- same favicon -->
    <link data-trunk rel="css" href="style.css">
  </head>
  <body></body>
  </html>
  ```
- [ ] Copy the existing CSS to `frontend/style.css` (keep it identical for
  now; will be refactored alongside component work in later phases)
- [ ] Create `frontend/src/lib.rs`:
  ```rust
  use leptos::*;

  #[component]
  pub fn App() -> impl IntoView {
      view! {
          <div class="app">
              <header><h1>Winnie v2</h1></header>
              <p>"Leptos frontend loading..."</p>
          </div>
      }
  }
  ```
- [ ] Create `frontend/src/main.rs`:
  ```rust
  use leptos::*;
  use winnie_cam_frontend::App;

  fn main() {
      mount_to_body(|| view! { <App/> });
  }
  ```

### 1.3 — Verify Trunk builds

- [ ] Run `cd frontend && trunk build` — succeeds with no errors
- [ ] Verify `frontend/dist/` contains:
  - `index.html`
  - `winnie_cam_frontend_bg.wasm` (the WASM binary)
  - `winnie_cam_frontend.js` (the JS glue)
  - `style.css`

### 1.4 — Wire Axum to serve the Trunk output

- [ ] Add `tower-http` to root `Cargo.toml`:
  ```toml
  tower-http = { version = "0.6", features = ["fs"] }
  ```
- [ ] In `src/web.rs`, add a route for the new frontend:
  ```rust
  use tower_http::services::ServeDir;

  // Add to router():
  .nest_service("/v2", ServeDir::new("frontend/dist"))
  ```
- [ ] Keep the existing `index()` handler at `/` unchanged — the old UI
  stays as-is

### 1.5 — Add a build convenience

- [ ] Document the full build command in `frontend/README.md`:
  ```markdown
  # Development
  cd frontend && trunk serve --port 8081 --proxy-backend=http://127.0.0.1:8080

  # Production
  cd frontend && trunk build --release
  ```
- [ ] Consider a `Justfile` or shell script at repo root:
  ```makefile
  dev:
      cargo run --release & sleep 2 && cd frontend && trunk serve --port 8081 --proxy-backend=http://127.0.0.1:8080

  build:
      cd frontend && trunk build --release
      cargo build --release
  ```

### 1.6 — Verification

- [ ] `cargo build` from repo root succeeds (all workspace members)
- [ ] Start the backend: `cargo run --release`
- [ ] In another terminal: `cd frontend && trunk build && cp -r dist ../`
- [ ] Visit `http://localhost:8080/` — the old UI loads unchanged
- [ ] Visit `http://localhost:8080/v2/` — the new Leptos app loads with
  "Winnie v2 — Leptos frontend loading..." text visible
- [ ] Open browser devtools — no console errors, no WASM load failures
- [ ] CSS loads and applies (fonts, colors match the old UI)
- [ ] Verify on a phone/tablet on the LAN that the old UI at `/` still
  works as before
- [ ] Trunk dev mode: `cd frontend && trunk serve --port 8081 --proxy-backend=http://127.0.0.1:8080` — visit `http://localhost:8081` — app auto-reloads on source changes