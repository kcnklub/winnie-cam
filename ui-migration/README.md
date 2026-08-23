# UI Migration — Overview

This directory tracks migrating `src/index.html` (a ~1,576-line monolithic
file containing all CSS and JS inline) to a **Leptos** client-side-rendered
SPA, compiled to WebAssembly via **Trunk** and served as static files
alongside the existing Axum backend.

## Why

The current frontend works but has zero compile-time verification:

- No type checking across the HTTP boundary. Server JSON shapes (`/healthz`,
  `/api/config`, SSE event payloads) drift independently from the JS that
  parses them.
- No component organization. One file with ~1,100 lines of imperative JS
  and ~450 lines of CSS — no separation of concerns.
- No Rust compiler verification. A typo in a DOM query selector or a
  misnamed JSON field is a runtime bug discovered at 3 AM.

## What Changes

| Layer | Before | After |
|-------|--------|-------|
| Frontend | Single `src/index.html` with inline CSS + JS, served via `include_str!()` | Leptos CSR app compiled to WASM, served as static files from `dist/` |
| Backend API routes | Unchanged | Unchanged |
| `/` route | `get(index)` returning `Html<&'static str>` | Static file serving via `tower_http::services::ServeDir` |
| CSS | Inline in `<style>` | Separate `style.css` (Trunk `data-trunk` link or imported) |
| JS runtime | Vanilla JS (browser-interpreted) | Rust → WASM (ahead-of-time compiled) |

## Framework Choice: Leptos

**Leptos** was chosen over Dioxus and Yew for three reasons specific to this
app:

1. **Fine-grained signals** — the app has many independent reactive streams
   (MJPEG frame arrival, SSE detection data, SSE motion events, healthz
   poll, settings form state). Leptos signals update exactly the DOM nodes
   that depend on them, matching the current pattern of surgically updating
   individual elements without re-rendering whole components.

2. **No virtual DOM** — the detection overlay requires imperative Canvas 2D
   drawing. Leptos's `create_effect` + `NodeRef<Canvas>` allows this without
   a VDOM layer sitting between the reactive system and the imperative API.

3. **`web-sys` first-class** — Canvas 2D, WebAudio, Fullscreen API,
   ResizeObserver, EventSource, and `visibilitychange` are all available
   through typed `web-sys` bindings rather than raw `wasm_bindgen`.

4. **CSR-only mode** — this is purely a single-page app connecting to
   SSE/MJPEG streams. No SSR hydration needed, keeping the WASM bundle
   small (~150KB gzipped) and the mental model simple.

## Architecture

```
Trunk output (dist/)                Axum server
┌─────────────────────┐             ┌──────────────────────┐
│ index.html (shell)  │ served as   │ GET  /               │  static files
│ frontend_bg.wasm    │ ◄────────── │ GET  /stream.mjpeg   │  MJPEG broadcast
│ frontend.js         │ static      │ GET  /snapshot.jpg   │  single JPEG
│ style.css           │ files       │ GET  /healthz        │  JSON stats
│                     │             │ GET  /detections     │  SSE (person boxes)
│                     │  SSE/       │ GET  /events         │  SSE (motion events)
│                     │  MJPEG      │ GET  /events.json    │  JSON (motion buffer)
│                     │  streams    │ GET  /api/config     │  current settings
│                     │ ◄────────── │ PUT  /api/config     │  update settings
└─────────────────────┘             └──────────────────────┘
```

The WASM app communicates with the backend through the **same API contracts**
the current JS uses. None of the backend routes change their wire format.

## Phases

Each phase is a self-contained document with concrete, checkboxed task lists.
Phases are ordered to build on each other — each one's verification
checklist assumes all previous phases are complete.

| # | Phase | File |
|---|-------|------|
| 0 | Shared API types | [`00-shared-types.md`](00-shared-types.md) |
| 1 | Project scaffolding | [`01-scaffolding.md`](01-scaffolding.md) |
| 2 | Core video + status | [`02-core-video.md`](02-core-video.md) |
| 3 | Detection overlay | [`03-detection-overlay.md`](03-detection-overlay.md) |
| 4 | Motion events + alerts | [`04-motion-events.md`](04-motion-events.md) |
| 5 | Settings panel | [`05-settings-panel.md`](05-settings-panel.md) |
| 6 | Controls + polish | [`06-controls.md`](06-controls.md) |
| 7 | Cleanup + verification | [`07-cleanup.md`](07-cleanup.md) |

## Conventions

- **Each phase is a PR.** The checkboxes serve as both implementation
  checklist and reviewer's verification list.
- **Backward compatibility.** The old `index.html` remains served at `/`
  until Phase 7 (cleanup). During phases 1-6 the new UI is served at a
  separate path (e.g. `/ui` or `/v2`) so both can be tested side by side.
- **API types live in a shared crate** (`src/shared-types/` or a workspace
  member). Every JSON shape the server sends or receives gets a serde
  struct here. Phase 0 establishes this; every subsequent phase uses these
  types on both sides.
- **No new backend dependencies** beyond `tower-http` (for `ServeDir`),
  which is already in the Axum ecosystem.
- **Trunk** is the build tool. `trunk build --release` produces `dist/`.
  Development uses `trunk serve --proxy-backend=http://127.0.0.1:8080` so
  API calls proxy to the running Axum server.