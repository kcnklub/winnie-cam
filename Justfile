# Convenience tasks for winnie-cam. Install: cargo install just

default:
    @just --list

# Build backend + frontend for production (frontend lands in frontend/dist/)
build:
    cargo build --release
    cd frontend && trunk build --release --public-url /v2

# Run backend + frontend dev server together (frontend proxies API to :8080)
dev:
    cargo run --release &
    cd frontend && trunk serve --proxy-backend=http://127.0.0.1:8080

# Run backend only (serves the prebuilt frontend/dist/ at /v2)
serve:
    cargo run --release

# Format + lint + test everything
check:
    cargo fmt --all
    cargo clippy --all-targets
    cargo test
    cd frontend && cargo fmt && cargo clippy --all-targets