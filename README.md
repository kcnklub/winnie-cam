# winnie-cam

A small Rust web app that forwards a camera's video to any browser on the
local network - v1 of a baby monitor. It serves an MJPEG stream (plain
`multipart/x-mixed-replace`, no plugins or JS frameworks needed) from either:

- a **Raspberry Pi CSI camera module**, via the `rpicam-vid` subprocess, or
- a **USB webcam** over V4L2 - which is also how this runs during
  development on a regular laptop with no CSI camera at all.

It can also run a simple person detector against the live feed, overlay the
boxes on the stream, and turn sustained movement into `motion_started`/
`motion_stopped` events - see "Person detection" and "Motion events" below.
Sound detection, recording, auth, and HTTPS are still not part of this
version; see "Out of scope" in the plan this was built from if you're
picking this back up later.

## Building

### On the Pi (recommended for now)

The Pi needs a recent Rust toolchain via [rustup](https://rustup.rs) -
Debian/Raspberry Pi OS's packaged `rustc` is too old for this crate's 2024
edition.

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# copy or clone this project onto the Pi, then:
cd winnie-cam
cargo build --release
```

A first build on a Pi Zero 2 W can take several minutes; later builds are
faster since only changed code gets recompiled.

### On a laptop, for development

```bash
cargo build
```

No extra system packages are required: the `v4l` crate talks to V4L2
directly via raw ioctls (no `libv4l-dev` dependency).

## Running

```bash
# Pi, CSI camera (rpicam-vid must be on PATH - it is by default on Pi OS)
cargo run --release -- --source rpicam --width 1280 --height 720 --fps 15

# Laptop or Pi, USB webcam
cargo run -- --source v4l2 --device /dev/video0 --width 1280 --height 720 --fps 15

# Let it auto-detect (default): picks rpicam-vid if found on PATH, else V4L2
cargo run
```

Then open `http://<host>:8080` in a browser - `http://localhost:8080` on
the same machine, or `http://<pi-hostname>.local:8080` from another device
on the LAN (Pi OS ships avahi, so `.local` resolution works without hunting
for an IP).

If the camera ends up mounted upside down (a common outcome on a crib
rail), add `--hflip`/`--vflip`.

Run `cargo run -- --help` for the full list of flags (resolution, fps, JPEG
quality, bind address, ...).

### Endpoints

- `/` - the viewer page.
- `/stream.mjpeg` - the live MJPEG stream directly, if you want to point
  `ffmpeg`, VLC, or another client at it instead of a browser.
- `/snapshot.jpg` - the single most recent frame.
- `/healthz` - JSON status: uptime, frames captured, current viewer count,
  seconds since the last frame, and (with `--detect`) detection state,
  latest inference time, and seconds since the last detection pass.
- `/detections` - `text/event-stream` (SSE) of the latest detection pass,
  boxes normalized to a 0..1 fraction of the source frame so the client
  never needs to know the capture resolution. Returns 503 if `--detect`
  wasn't passed. This is what the viewer page's overlay toggle subscribes
  to - see "Person detection" below.
- `/events` - `text/event-stream` (SSE) of motion events. On connect, sends
  one `snapshot` event with recent history, then a `motion` event per
  `motion_started`/`motion_stopped` transition as it happens. Returns 503
  under the same condition as `/detections` - see "Motion events" below.
- `/events.json` - the same recent-history ring buffer `/events`' `snapshot`
  sends, as a one-shot JSON document, for polling instead of holding a
  connection open.

## Person detection (optional)

`--detect` runs a person detector (YOLOv8n, filtered to the COCO "person"
class) against the live feed and makes the results available for a
toggleable bounding-box overlay on the viewer page - useful for eyeballing
whether the detector is actually finding the baby. It's meant as a "is
there a person-shaped thing in the crib" sanity signal, **not a safety
mechanism**: a swaddled infant seen from directly overhead, especially
under IR/night vision, looks very different from the upright, daylight,
limbs-visible people the model was trained on. Expect frequent misses at
night, lower confidence scores than you'd get from a normal security
camera, and occasional false positives on blankets or stuffed animals.

Detection is CPU-only and is the most expensive thing this program can do
by a wide margin - it's **Raspberry Pi 4/5 only**. Don't enable it on a Pi
Zero 2 W; the model alone needs ~60-100MB more RAM than this app otherwise
uses, on top of the CPU cost.

### 1. Export a model

You'll need a YOLOv8n or YOLO11n model exported to ONNX with a **fixed
square input** and **without built-in NMS** (this app does its own
non-maximum suppression). In a Python environment with `ultralytics`
installed:

```bash
pip install ultralytics onnx
yolo export model=yolov8n.pt format=onnx imgsz=320 opset=12 simplify=True dynamic=False half=False
```

That produces `yolov8n.onnx` (~12MB). Put it wherever you're running the
server from - `models/yolov8n.onnx` is a reasonable convention, and
`.gitignore` already excludes `models/` since a ~12MB binary file doesn't
belong in git history. `imgsz=320` matches this app's `--detect-size`
default; if you export at a different size, pass a matching `--detect-size`.

### 2. Run with detection enabled

```bash
cargo build --release   # --release is mandatory, see below
./target/release/winnie-cam --detect --model models/yolov8n.onnx
```

**`--release` is not optional here.** `tract` (the pure-Rust ONNX runtime
this uses) is 30-100x slower in a debug build - an inference pass that
takes 300ms in release can take 20-30 *seconds* in debug, which will look
like the app has hung. `cargo run` without `--release` will emit a warning
if `--detect` is passed in a debug build, but it's easy to miss in
scrollback.

Flags:

| Flag | Default | |
|---|---|---|
| `--detect` | off | Enables detection. Requires `--model`. |
| `--model <path>` | - | Path to the exported ONNX model. |
| `--detect-size` | 320 | Model input side length; must match the export. |
| `--detect-fps` | 1.0 | Passes/sec while a viewer has the overlay open. |
| `--detect-idle-fps` | 0.2 | Passes/sec when nobody's watching - detection keeps running at a low rate rather than stopping entirely, so there's a standing signal to build alerting on later, but slowly since sustained inference is the Pi's dominant heat source. |
| `--detect-threshold` | 0.35 | Minimum confidence to report a box. |
| `--detect-iou` | 0.45 | Non-maximum suppression IoU threshold. |

Once running, open the viewer page and click the toggle in the top-left
corner to turn the overlay on/off. **`--detect` also turns on motion
events** (see "Motion events" below) - there's no separate flag for that,
since motion is derived from the person box detection already produces.

### Measured latency

One inference pass at the default 320x320:

| Hardware | Time/pass | Notes |
|---|---|---|
| Pi 5 Model B (Cortex-A76), CSI Camera Module 2 | ~145-180ms | Measured via `/healthz`'s `detect_ms`. `tract` auto-detected ARMv8.2 and activated fp16 NEON kernels (`mmm_f16`/`sigmoid_f16`/`tanh_f16`) - see the `tract_linalg::arm64` startup log lines. Comfortably supports `--detect-fps` well above the 1.0 default; 3-5 passes/sec is realistic if you want a snappier overlay. |
| Pi 4 (Cortex-A72) | _(untested)_ | No dot-product NEON extension, so expect meaningfully slower than the Pi 5 numbers above - plausibly 600ms-1.2s, which would land under 1 detection/sec even at the default `--detect-fps 1.0`. If that's too slow, try `--detect-size 256` or `192` before switching models. |

`tract` runs a single pass on one core - it doesn't parallelize a single
`run()` call, so these numbers won't improve just from having more cores
free.

`/healthz`'s `detect_ms` field reports the actual measured time on your
hardware once it's running - use that instead of guessing.

### A build-time tradeoff worth knowing about

Adding `tract-onnx` roughly doubles this project's dependency count and
pushes a *clean* release build on the Pi from a few minutes to
**~30-50 minutes on a Pi 4** / **~15-25 minutes on a Pi 5** (see
`deploy.md`), mostly because one of tract's crates generates and compiles
aarch64 NEON assembly at build time. We're accepting that for now to keep
this project's "just `cargo build --release` on the Pi" deploy story
unchanged - no cross-compilation toolchain, no Docker, nothing extra to set
up. Incremental rebuilds after the first one stay fast either way.

**If build times become annoying**, the natural fix later is cross-compiling
from a faster machine (e.g. an x86_64 laptop, targeting
`aarch64-unknown-linux-gnu`) and `rsync`-ing the built binary to the Pi
instead of the source - builds would drop from tens of minutes to seconds.
That's a real chunk of work on its own (cross toolchain setup, and the `v4l`
crate's `bindgen`/`clang` step needs to target aarch64 too), so it's
deliberately not part of this change - just flagging it as the option to
reach for if the on-Pi build time stops being worth it.

## Motion events

`--detect` also turns on motion events - there's no `--motion` flag to
enable separately. Rather than diffing the whole frame (which fires on
lighting changes, curtains, a pet, or the camera getting bumped), motion is
scored only inside the most recent person box: the frame is downsampled to
a small grayscale grid, masked to the region the box covers, and compared
against the previous sample. A debounced state machine turns that into
discrete events - motion has to *sustain* before `motion_started` fires,
and stillness has to sustain before `motion_stopped` fires - so a flickering
score doesn't spam events.

Because it needs a person box, motion is only as fresh as the last
detection pass - at the idle rate (`--detect-idle-fps`, default 0.2/s) that
means up to ~5s before a box is available to mask against, which is also
why `--motion-box-ttl-ms` defaults to 10s (roughly twice that period, so a
box survives one missed pass without motion going quiet for no reason).

Events show up three ways:
- The viewer page's **Activity panel** (bottom-left, appears whenever
  detection is ready) - a still/moving pill plus a short recent-events list.
- `/events` and `/events.json` - see "Endpoints" above.
- `tracing` logs (`motion started` / `motion stopped`, at `info` level) -
  useful for tuning thresholds without a browser open at all.

Tuning flags (defaults are meant to be reasonable out of the box - most
setups shouldn't need to touch these):

| Flag | Default | |
|---|---|---|
| `--motion-fps` | 2.0 | Motion samples per second. |
| `--motion-grid` | 64 | Diff grid side length (grid x grid grayscale cells). |
| `--motion-pixel-delta` | 12 | Per-cell grayscale change (0-255) that counts as "changed". |
| `--motion-threshold` | 0.08 | Fraction of in-box cells that must change to count as motion. |
| `--motion-sustain-ms` | 700 | How long motion must persist before `motion_started` fires. |
| `--motion-quiet-ms` | 4000 | How long stillness must persist before `motion_stopped` fires. |
| `--motion-cooldown-ms` | 3000 | Minimum gap between a stop and the next start. |
| `--motion-box-ttl-ms` | 10000 | Ignore person boxes older than this. |
| `--motion-box-margin` | 0.10 | Dilate each box by this fraction of its own size before masking. |

If motion is either too twitchy or too slow to notice real movement, watch
the logged `score` field against `--motion-threshold` (`RUST_LOG=info`
already shows it) before reaching for a different grid size or pixel delta.

## Deploying as a service

See [`deploy.md`](deploy.md) for the full step-by-step runbook (getting the
code onto the Pi, first build, systemd install, and redeploying updates).

`deploy/winnie-cam.service` is a systemd unit that restarts the app if it
crashes or the Pi loses power briefly - the point of a baby monitor is that
it comes back on its own.

```bash
sudo cp deploy/winnie-cam.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now winnie-cam
```

Edit the `ExecStart` path in the unit file to match where you built the
binary, and add any flags you need (orientation, resolution, etc.) there.
The `pi` user needs to be in the `video` group to open the camera:

```bash
sudo usermod -aG video pi
```

## Development notes

- `cargo test` runs everything except one test: `src/jpeg.rs`'s JPEG
  frame-splitter tests (see its doc comments for why a naive "scan for
  `FF D9`" approach isn't safe), plus the detection module's letterbox
  coordinate math, NMS, YOLO output parsing, and JSON serialization tests
  under `src/detect/`, and motion's grid-diff, debounce state-machine, and
  event-hub tests under `src/detect/motion/` - a submodule of `detect`, not
  a sibling of it, since motion has no meaning without the person box
  `detect` produces (see that module's doc comment).
- One test is `#[ignore]`d by default: `detect::model::tests::
  the_model_loads_optimizes_and_runs` actually loads an ONNX model through
  `tract` and runs inference on it, so it needs a real exported model file
  to point at. Run it with:
  ```bash
  WINNIE_CAM_TEST_MODEL=models/yolov8n.onnx cargo test --release -- --ignored
  ```
  (`--release` matters here too - see the `--release` note above.) Run this
  after any change to model loading or after re-exporting the model with
  different settings, on both your dev machine and the Pi - `tract`'s op
  support can differ across `imgsz`/`opset` export choices in ways that are
  much cheaper to catch here than by staring at an empty overlay.
- Both capture backends publish into a single `FrameHub`
  (`src/hub.rs`), so the camera is opened exactly once no matter how many
  browser tabs are watching. `src/detect/hub.rs`'s `DetectionHub` is the
  equivalent for detection results - a `watch` channel instead of a
  `broadcast` one, since only the latest detection pass ever matters.
  `src/detect/motion/hub.rs`'s `MotionHub` goes back to `broadcast` (plus a
  small ring buffer): unlike detections, individual motion events are
  discrete occurrences that shouldn't be coalesced away by a later one
  overwriting an unread earlier one.
- `detect::motion::pump_loop` reads the current person box via
  `DetectionHub::latest()`, a borrow-only accessor - never
  `DetectionHub::subscribe()`, which would pin `detect::pump_loop` to its
  active sample rate forever (see that function's doc comment).
- `detect::start` is the single entry point `main.rs` calls: it spawns the
  person detector and, since motion has no switch of its own, spawns motion
  right alongside it from the `DetectionHub` that produces. Motion living as
  a submodule of `detect` (not a top-level module next to it) is also what
  lets the `decode_rgb`/`Throttle`/`fps_to_interval` helpers `detect::motion`
  reuses stay private to `detect` - a private item is visible to descendants
  of its defining module, so no `pub(crate)` is
  needed just to share them with a submodule.
