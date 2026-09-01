# Convenience tasks for winnie-cam. Install: cargo install just

# Microphone streaming. `--audio` needs ffmpeg on PATH and fails at startup
# without it, so both are overridable per-run:
#   just audio_device=plughw:1,0 dev
#   just audio=off dev
# `just devices` lists the card/device numbers to use here.
audio := "on"
audio_device := "default"
audio_flags := if audio == "on" { "--audio --audio-device " + audio_device } else { "" }

default:
    @just --list

# Fail fast if trunk is not the pinned version. trunk 0.21.6+ (including the
# 0.22.0-beta.x releases) panics at startup when proxying to a backend with a
# bare "/" path: axum 0.8 removed root-level `nest_service` ("Nesting at the
# root is no longer supported"), and trunk's proxy still calls it for
# `--proxy-backend=http://127.0.0.1:8080`. 0.21.5 is the last version whose
# proxy works at the root.
check-trunk:
    #!/usr/bin/env bash
    set -euo pipefail
    required="0.21.5"
    if ! command -v trunk >/dev/null 2>&1; then
        echo "error: trunk not found. Install with: cargo install trunk@${required} --locked" >&2
        exit 1
    fi
    actual="$(trunk --version | awk '{print $2}')"
    if [ "${actual}" != "${required}" ]; then
        echo "error: trunk ${required} required, found ${actual}." >&2
        echo "       trunk 0.21.6+ panics on root proxying (axum 0.8 regression)." >&2
        echo "       Install with: cargo install trunk@${required} --locked --force" >&2
        exit 1
    fi

# Build backend + frontend for production (frontend lands in frontend/dist/)
build: check-trunk
    cargo build --release
    cd frontend && trunk build --release --public-url /v2

# Run backend + frontend dev server together (frontend proxies API to :8080).
# Press Ctrl-C once to stop both.
dev: check-trunk
    #!/usr/bin/env bash
    set -euo pipefail

    if ss -ltn 2>/dev/null | grep -qE ':8080[[:space:]]'; then
        echo "error: port 8080 is already in use (another winnie-cam running?)" >&2
        echo "       find it with: ss -tlnp | grep :8080" >&2
        exit 1
    fi

    # Build once, then run the binary directly so $! is the server's own PID.
    # (cargo run would background *cargo*, not the server, and leave an orphan
    # holding :8080 if the shell dies.)
    cargo build --release -p winnie-cam
    ./target/release/winnie-cam --detect --model models/yolov8n.onnx {{audio_flags}} &
    backend=$!
    cleanup() {
        kill "$backend" 2>/dev/null || true
    }
    trap cleanup INT TERM EXIT

    # Wait for the backend to answer /healthz (bounded, in case it crashes on boot).
    ready=""
    for _ in $(seq 1 60); do
        if curl -sf http://127.0.0.1:8080/healthz >/dev/null 2>&1; then
            ready=1
            break
        fi
        if ! kill -0 "$backend" 2>/dev/null; then
            echo "error: backend exited before becoming ready (see output above)" >&2
            exit 1
        fi
        sleep 1
    done
    if [ -z "$ready" ]; then
        echo "error: backend did not become ready within 60s" >&2
        exit 1
    fi

    cd frontend
    trunk serve --dist dist-dev --port 8081 --proxy-backend=http://127.0.0.1:8080

# Run backend only (serves the prebuilt frontend/dist/ at /v2)
serve:
    cargo run --release -- --detect --model models/yolov8n.onnx {{audio_flags}}

# List ALSA capture devices, to pick a value for `audio_device`.
devices:
    @arecord -l

# Confirm a microphone works before involving the app: records 3 seconds
# through the same ffmpeg pipeline the server uses, then plays it back.
check-mic:
    #!/usr/bin/env bash
    set -euo pipefail

    if ! command -v ffmpeg >/dev/null 2>&1; then
        echo "error: ffmpeg not found. Debian/Pi OS: sudo apt install ffmpeg" >&2
        exit 1
    fi

    out="$(mktemp -t winnie-mic-XXXXXX.webm)"
    trap 'rm -f "${out}"' EXIT

    echo "recording 3s from {{audio_device}} ..."
    ffmpeg -hide_banner -loglevel warning \
        -f alsa -i {{audio_device}} -t 3 \
        -ac 1 -ar 48000 -c:a libopus -b:a 32k -application voip \
        -f webm -y "${out}"

    echo "playing it back ..."
    ffplay -hide_banner -loglevel warning -autoexit -nodisp "${out}"

# Listen to the running server's stream from the terminal, no browser
# involved - proves the endpoint independently of any frontend bug.
listen port="8080":
    ffplay -hide_banner -loglevel warning -autoexit -nodisp http://127.0.0.1:{{port}}/audio

# Format + lint + test everything
check:
    cargo fmt --all
    cargo clippy --all-targets
    cargo test
    cd frontend && cargo fmt && cargo clippy --all-targets
