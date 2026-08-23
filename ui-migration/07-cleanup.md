# Phase 7 — Cleanup + Final Verification

**Objective:** Remove the old `index.html`, make the Leptos app the default
at `/`, clean up dead code, add a build script, and perform end-to-end
verification on all target devices.

**Dependencies:** All previous phases complete and verified.

## Task List

### 7.1 — Swap the default route

- [ ] In `src/web.rs`:
  - Remove `route("/", get(index))` 
  - Remove the `index()` handler function
  - Change `ServeDir` from `/v2` to `/`:
    ```rust
    .nest_service("/", ServeDir::new("frontend/dist"))
    ```
  - Keep all API routes (`/stream.mjpeg`, `/snapshot.jpg`, `/healthz`,
    `/detections`, `/events`, `/events.json`, `/api/config`) — they must
    take precedence over the static file service, so register them
    *before* the `nest_service` fallback
- [ ] Verify that the Axum router order is correct:
  ```rust
  Router::new()
      .route("/stream.mjpeg", get(stream_mjpeg))
      .route("/snapshot.jpg", get(snapshot))
      .route("/healthz", get(healthz))
      .route("/detections", get(detections))
      .route("/events", get(events))
      .route("/events.json", get(events_json))
      .route("/api/config", get(get_config).put(update_config))
      .fallback_service(ServeDir::new("frontend/dist"))
  ```
  Using `fallback_service` instead of `nest_service` ensures API routes
  are matched first, and the SPA catches everything else (including
  client-side routing if added later).

### 7.2 — Remove old frontend artifacts

- [ ] Delete `src/index.html` — the old monolithic file
- [ ] Grep for any remaining references:
  ```bash
  rg "index\.html" src/
  rg "include_str!" src/
  ```
  Both should return no results.
- [ ] Check that `src/web.rs` no longer imports anything HTML-related
  (no `Html` response type from axum unless used elsewhere)
- [ ] Remove `Html` import if unused

### 7.3 — Build script

- [ ] Create or update a build script at the repo root. Options:
  - **`Justfile`** (recommended for simplicity):
    ```makefile
    default:
        @just --list

    build:
        cd frontend && trunk build --release
        cargo build --release

    dev:
        cargo run --release &
        cd frontend && trunk serve --proxy-backend=http://127.0.0.1:8080

    # For deployment: builds both and copies frontend to a known location
    dist: build
        mkdir -p dist
        cp target/release/winnie-cam dist/
        cp -r frontend/dist dist/static
    ```
  - Or a shell script `build.sh`
- [ ] Document the build process in `README.md` (update the "Building"
  section to mention Trunk and `wasm32-unknown-unknown`)
- [ ] Add `.gitignore` entries:
  ```
  frontend/dist/
  frontend/target/
  ```

### 7.4 — Update `deploy.md`

- [ ] Update `deploy.md` with the new build steps:
  - Install `wasm32-unknown-unknown` target on the build machine (or Pi)
  - Install Trunk
  - Build frontend: `cd frontend && trunk build --release`
  - Build backend: `cargo build --release`
  - The `frontend/dist/` directory must be deployed alongside the binary
  - Update systemd service if it references file paths

### 7.5 — Add a "both UIs" verification mode (optional safety net)

- [ ] Consider keeping the old UI available at `/classic` during the
  transition period:
  - Keep `src/index.html` for one release cycle
  - Add `route("/classic", get(index_classic))` that serves it
  - After a bake period (1-2 weeks with no issues), remove completely
- [ ] This is optional — only if you're risk-averse about the migration

### 7.6 — Production build size check

- [ ] Run `trunk build --release` in `frontend/`
- [ ] Check WASM binary size:
  ```bash
  ls -lh frontend/dist/*.wasm
  gzip -c frontend/dist/*.wasm | wc -c
  ```
  Target: < 300KB uncompressed, < 150KB gzipped
- [ ] Check total `dist/` size:
  ```bash
  du -sh frontend/dist/
  ```
  Should be under 500KB total
- [ ] Verify no unexpected large dependencies were pulled in (run
  `cargo tree -p winnie-cam-frontend` and review)

### 7.7 — End-to-end verification matrix

Run through every feature on every target device:

| Feature | Laptop (Linux) | Laptop (macOS) | Phone (iOS) | Phone (Android) | Pi (local) |
|---------|---------------|----------------|-------------|-----------------|------------|
| Video loads | ☐ | ☐ | ☐ | ☐ | ☐ |
| Reconnect on camera unplug | ☐ | ☐ | ☐ | ☐ | ☐ |
| Status indicator states | ☐ | ☐ | ☐ | ☐ | ☐ |
| Detection overlay | ☐ | N/A | ☐ | ☐ | ☐ |
| Detection stale timeout | ☐ | N/A | ☐ | ☐ | ☐ |
| Motion events + alert | ☐ | N/A | ☐ | ☐ | ☐ |
| WebAudio chime | ☐ | ☐ | ☐ | ☐ | ☐ |
| Chime mute persistence | ☐ | ☐ | ☐ | ☐ | ☐ |
| Settings panel | ☐ | ☐ | ☐ | ☐ | ☐ |
| Settings apply + restart | ☐ | ☐ | ☐ | ☐ | ☐ |
| Dim cycling | ☐ | ☐ | ☐ | ☐ | ☐ |
| Fullscreen | ☐ | ☐ | ☐ | ☐ | ☐ |
| iOS immersive fallback | N/A | N/A | ☐ | N/A | N/A |
| Theme toggle | ☐ | ☐ | ☐ | ☐ | ☐ |
| System theme follows OS | ☐ | ☐ | ☐ | ☐ | ☐ |
| Snapshot download | ☐ | ☐ | ☐ | ☐ | ☐ |
| Multiple tabs (viewer count) | ☐ | ☐ | ☐ | ☐ | ☐ |
| localStorage persistence | ☐ | ☐ | ☐ | ☐ | ☐ |
| No console errors | ☐ | ☐ | ☐ | ☐ | ☐ |
| Page reload during stream | ☐ | ☐ | ☐ | ☐ | ☐ |

### 7.8 — Final cleanup

- [ ] Run `cargo clippy --all-targets` — fix any warnings
- [ ] Run `cargo fmt` on all crates
- [ ] Run `cargo test` — all tests pass
- [ ] Remove unused dependencies from `Cargo.toml` (both root and frontend)
- [ ] Remove `shared-types` path dependency from root if it was only used
  by the old JS (it's needed by both, so likely keep it)
- [ ] Ensure `frontend/Cargo.toml` doesn't pull in unnecessary `web-sys`
  features — prune to exactly what's used
- [ ] Update `README.md`:
  - Mention Leptos in the tech stack
  - Update the "Building" section with Trunk steps
  - Add a "Development" section with `trunk serve` instructions
- [ ] Tag the release: `git tag v1.0.0-leptos` (or whatever version convention)

### 7.9 — Rollback plan

If something is broken in production:

- [ ] Keep the last pre-Leptos binary available
- [ ] The rollback is: swap the binary back, no frontend build needed
- [ ] If only the frontend is broken: `git revert` the route change in
  `web.rs` to serve the old `index.html` at `/` again, keep `/v2` for
  debugging

## Verification Commands

```bash
# Build everything
cargo build --release
cd frontend && trunk build --release

# Run
./target/release/winnie-cam --device /dev/video0

# Or with detection
./target/release/winnie-cam --detect --model models/yolov8n.onnx

# Check healthz
curl -s http://localhost:8080/healthz | jq

# Check config
curl -s http://localhost:8080/api/config | jq

# Check detection SSE (Ctrl-C to stop)
curl -N http://localhost:8080/detections

# Check motion events SSE (Ctrl-C to stop)
curl -N http://localhost:8080/events

# Check the UI loads
curl -s http://localhost:8080/ | head -20
# Should return the Trunk-built index.html (not the old one)
```