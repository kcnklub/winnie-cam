# winnie-cam frontend

Leptos client-side-rendered SPA, built with [Trunk](https://trunkrs.dev/).

## Prerequisites

```bash
rustup target add wasm32-unknown-unknown
cargo install trunk@0.21.5 --locked   # see version pin note below
```

> **Trunk version pin.** We pin trunk to `0.21.5` — the last release whose
> `trunk serve --proxy-backend=...` works when the backend is served at the
> root path. trunk `0.21.6` through `0.22.0-beta.x` panic at startup
> (`Nesting at the root is no longer supported`) because they call axum 0.8's
> root-level `nest_service`, which was removed. Do not `cargo install trunk`
> without a version until that regression is fixed upstream.

## Development

Start the Axum backend on port 8080 first, then run Trunk's dev server with
a proxy so API/SSE requests hit the backend:

```bash
# terminal 1 — backend
cargo run --release

# terminal 2 — frontend with hot reload
cd frontend
trunk serve --port 8081 --proxy-backend=http://127.0.0.1:8080
```

Open http://localhost:8081 — the frontend proxies `/stream.mjpeg`,
`/healthz`, `/detections`, `/events`, and `/api/config` to the backend.

## Production build

```bash
cd frontend
trunk build --release --public-url /v2
```

The `--public-url /v2` prefix is required while the app is served under the
`/v2` subpath (see `src/web.rs`'s `nest_service`). Output lands in
`frontend/dist/`, which the backend serves directly.

## Layout

```
index.html        Trunk entry point (shell + asset links)
style.css         All UI styles (ported from the old src/index.html)
src/main.rs       WASM entry point — mounts <App/>
src/lib.rs        Top-level <App/> component
```