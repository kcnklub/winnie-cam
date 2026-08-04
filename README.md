# winnie-cam

A small Rust web app that forwards a camera's video to any browser on the
local network - v1 of a baby monitor. It serves an MJPEG stream (plain
`multipart/x-mixed-replace`, no plugins or JS frameworks needed) from either:

- a **Raspberry Pi CSI camera module**, via the `rpicam-vid` subprocess, or
- a **USB webcam** over V4L2 - which is also how this runs during
  development on a regular laptop with no CSI camera at all.

Motion/sound detection, recording, auth, and HTTPS are not part of this
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
  and seconds since the last frame (useful for confirming the camera is
  actually alive without opening a browser).

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

- `cargo test` runs the JPEG frame-splitter's unit tests, which are the
  trickiest part of this codebase - see the doc comments in `src/jpeg.rs`
  for why a naive "scan for `FF D9`" approach isn't safe.
- Both capture backends publish into a single `FrameHub`
  (`src/hub.rs`), so the camera is opened exactly once no matter how many
  browser tabs are watching.
