# Deploying winnie-cam to the Pi

Step-by-step instructions for getting this code onto a Raspberry Pi, building
it there, and running it as a systemd service. This assumes the plain
Raspberry Pi OS (Bookworm or later) with SSH already enabled, and covers the
initial deploy plus how to push later updates.

Throughout, replace `pi@raspberrypi.local` with your actual user@host. If
you'll be running these commands more than once, it's worth exporting it
once per shell session:

```bash
export PI_HOST=pi@raspberrypi.local
```

## 1. One-time Pi setup

These only need to happen once per Pi.

### Enable and check the camera

If using the CSI camera module, confirm it's detected before anything else
- this rules out a ribbon-cable problem early, rather than after chasing it
through application logs:

```bash
ssh "$PI_HOST" rpicam-hello --list-cameras
```

You should see your camera model listed. If nothing shows up: reseat the
ribbon cable (blue side facing the correct way per your Pi model), reboot,
and try again. `raspi-config` isn't needed on current Pi OS - the camera
connector is auto-detected.

If instead you're using a USB webcam, just confirm it enumerates:

```bash
ssh "$PI_HOST" v4l2-ctl --list-devices
```

### Install Rust via rustup

Raspberry Pi OS's packaged `rustc` is too old for this crate's 2024
edition, so install a current toolchain directly:

```bash
ssh "$PI_HOST" 'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y'
```

Then make sure `cargo` is on `PATH` for future non-interactive SSH commands
(the installer adds it to `~/.profile`, which only login shells source):

```bash
ssh "$PI_HOST" 'echo export PATH="\$HOME/.cargo/bin:\$PATH" >> ~/.bashrc'
```

Verify:

```bash
ssh "$PI_HOST" '~/.cargo/bin/cargo --version'
```

### Add the user to the `video` group

Needed either way (CSI or USB) so the app can open the camera device
without running as root:

```bash
ssh "$PI_HOST" sudo usermod -aG video "$(whoami)"
```

This requires a fresh login to take effect - log out and back in (or just
reboot) before relying on it.

## 2. Send the code to the Pi

From your development machine, in this project's directory, `rsync` is the
right tool here: unlike `scp -r`, re-running it only transfers what
changed, which matters once you're iterating. `--exclude` keeps the local
`target/` build cache and `.git` history off the Pi - the Pi builds its own
`target/` from source, and doesn't need your git history to do it.

```bash
rsync -avz --delete \
  --exclude target \
  --exclude .git \
  ./ "$PI_HOST":~/winnie-cam/
```

`--delete` keeps the Pi's copy in sync if you've removed files locally;
drop it for the first run if you'd rather be conservative.

If you're using `--detect` (see "Person detection" in the README), the
model file needs to go over too - it's `.gitignore`d, so a fresh sync of
the repo alone won't include it:

```bash
rsync -avz ./models/ "$PI_HOST":~/winnie-cam/models/
```

## 3. Build on the Pi

```bash
ssh "$PI_HOST" 'cd ~/winnie-cam && ~/.cargo/bin/cargo build --release'
```

**Without `--detect`** the first build compiles the full dependency tree and
can take several minutes on a Pi Zero 2 W (faster on a Pi 4/5). Subsequent
builds after small code changes are much quicker.

**With detection enabled** (i.e. `tract-onnx` in `Cargo.toml`), the first
build is much bigger - roughly **30-50 minutes on a Pi 4** and **15-25
minutes on a Pi 5**, since `tract`'s dependency tree is large and one of its
crates (`tract-linalg`) generates and compiles aarch64 NEON assembly at
build time. This is a one-time cost per Pi: `rsync` above already excludes
`target/`, so incremental rebuilds after that stay fast. See the README's
"Person detection" section for the tradeoff we're accepting here (and the
future option of cross-compiling from a faster machine instead).

## 4. Smoke-test it manually

Before wiring up systemd, run it in the foreground once to confirm the
camera actually works end-to-end:

```bash
ssh -t "$PI_HOST" '~/winnie-cam/target/release/winnie-cam --width 1280 --height 720 --fps 15'
```

Leave that running, and from another machine on the LAN, open
`http://<pi-hostname>.local:8080` (or `http://<pi-ip>:8080`) in a browser -
you should see the live feed. Check `/healthz` too:

```bash
curl http://raspberrypi.local:8080/healthz
```

`seconds_since_last_frame` should stay near zero. Once you've confirmed it
works, `Ctrl-C` it and move on to installing the service.

If the camera is mounted upside down (common once it's actually attached
somewhere useful, like a crib rail), add `--hflip`/`--vflip` - you'll bake
these into the systemd unit in the next step.

## 5. Install as a systemd service

This is what makes it a real baby monitor rather than a terminal window you
have to remember to leave open: it starts on boot and restarts itself if it
crashes or the Pi loses power briefly.

First, edit `deploy/winnie-cam.service` (locally, then re-sync, or directly
on the Pi) so `ExecStart` matches your actual username and any flags you
need:

```ini
ExecStart=/home/pi/winnie-cam/target/release/winnie-cam --width 1280 --height 720 --fps 15 --hflip
```

With detection enabled, add `--detect` and `--model`:

```ini
ExecStart=/home/pi/winnie-cam/target/release/winnie-cam --width 1280 --height 720 --fps 15 --hflip \
  --detect --model /home/pi/winnie-cam/models/yolov8n.onnx
```

No changes to the unit's hardening are needed for this: `ProtectHome=read-only`
makes `/home` read-only, not inaccessible, so the process can still read the
model file from there - it just can't write anywhere under it.

(The `User=pi` / `Group=pi` lines in the unit need to match too, if your Pi
user isn't literally named `pi`.)

Then install it:

```bash
ssh "$PI_HOST" '
  sudo cp ~/winnie-cam/deploy/winnie-cam.service /etc/systemd/system/ &&
  sudo systemctl daemon-reload &&
  sudo systemctl enable --now winnie-cam
'
```

Confirm it's up:

```bash
ssh "$PI_HOST" systemctl status winnie-cam
curl http://raspberrypi.local:8080/healthz
```

### Useful commands going forward

```bash
# Tail logs live
ssh "$PI_HOST" journalctl -u winnie-cam -f

# Restart after a config/flag change
ssh "$PI_HOST" sudo systemctl restart winnie-cam

# Stop it (e.g. to free the camera for `rpicam-hello` while debugging)
ssh "$PI_HOST" sudo systemctl stop winnie-cam
```

## 6. Redeploying an update

Once the service is installed, shipping a code change is: sync, rebuild,
restart.

```bash
rsync -avz --delete --exclude target --exclude .git ./ "$PI_HOST":~/winnie-cam/
ssh "$PI_HOST" '
  cd ~/winnie-cam &&
  ~/.cargo/bin/cargo build --release &&
  sudo systemctl restart winnie-cam
'
```

Worth turning into a local shell script or Makefile target once you're
doing it often.

## Troubleshooting

- **`rpicam-hello --list-cameras` shows nothing**: ribbon cable issue -
  reseat it (contacts facing the right way for your Pi model) and reboot.
  The app can't fix a camera the OS can't see.
- **Service is `active` but `/healthz` never gets a frame**
  (`seconds_since_last_frame` stays `null`)**: check
  `journalctl -u winnie-cam -f` - rpicam-vid failures only show up in
  stderr, which winnie-cam logs at `warn` level with
  `target: "rpicam-vid"`.
- **`Device or resource busy` opening the camera**: something else has it
  open - most likely a leftover foreground smoke-test process from step 4,
  or `rpicam-hello` left running. `sudo systemctl stop winnie-cam` before
  running any manual camera command, and vice versa.
- **Can't reach `http://<host>.local:8080` from another device**: try the
  Pi's IP address directly (`hostname -I` on the Pi) - `.local` resolution
  depends on mDNS/avahi working on both ends, which some routers or client
  devices don't support well.
- **Permission denied opening `/dev/video0`**: the user running the service
  isn't in the `video` group yet, or hasn't re-logged-in since being added
  (see step 1). Check with `groups pi`.
- **`unsupported operator` or the process exits right after "loading
  detection model" when using `--detect`**: `tract` couldn't compile the
  ONNX graph - almost always means the model wasn't exported the way this
  app expects. Re-check the `yolo export` command in the README (fixed
  input shape, no built-in NMS) rather than assuming it's a Pi-specific
  problem; this fails the same way on a laptop. Video keeps working either
  way - a failed model load only disables detection.
- **`/healthz` shows `"detect":"loading"` for a long time**: normal on
  first start - loading and optimizing the model takes a few seconds. If it
  never moves past `"loading"`, check `journalctl -u winnie-cam -f` for the
  load error.
- **The Pi got noticeably hotter / video got choppier after enabling
  `--detect`**: expected - sustained CPU inference is this app's biggest
  power/heat cost by far, and can thermal-throttle an un-heatsinked Pi
  enough to also affect the camera's hardware encode. Lower `--detect-fps`
  and/or `--detect-size` (e.g. 256 instead of 320), and check
  `vcgencmd measure_temp`.
