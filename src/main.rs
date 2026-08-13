mod capture;
mod config;
mod detect;
mod hub;
mod jpeg;
mod json;
mod web;

use anyhow::Context;
use clap::Parser;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use config::Config;
use hub::FrameHub;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cfg = Config::parse();

    let hub = FrameHub::new();
    let shutdown = CancellationToken::new();

    tokio::spawn({
        let shutdown = shutdown.clone();
        async move {
            wait_for_shutdown_signal().await;
            shutdown.cancel();
        }
    });

    let source = capture::build(&cfg)?;
    let capture_task = tokio::spawn(capture::supervise(source, hub.clone(), shutdown.clone()));

    // Spawned before the listener binds: model loading happens on the
    // detector's own worker thread, so it never delays the video stream
    // coming up (see `detect::spawn`'s doc comment). `spawn_all` is the
    // single entry point for everything detection-derived - person
    // detection and motion events both - so there's no separate motion
    // config to extract or gate on here; see `detect`'s module doc comment.
    let detection = detect::spawn_all(&cfg, hub.clone(), shutdown.clone())?;
    let (detect_hub, motion_hub) = match &detection {
        Some(d) => (Some(d.detect_hub.clone()), Some(d.motion_hub.clone())),
        None => (None, None),
    };

    let listener = tokio::net::TcpListener::bind(cfg.bind)
        .await
        .with_context(|| format!("binding to {}", cfg.bind))?;
    tracing::info!(bind = %cfg.bind, "winnie-cam listening");

    let app = web::router(web::AppState::new(hub, detect_hub, motion_hub, shutdown.clone()));
    let server_shutdown = shutdown.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move { server_shutdown.cancelled().await })
        .await
        .context("http server error")?;

    // The server only returns after shutdown was triggered; make sure
    // capture (and detection, if enabled) wind down too - they may already
    // be stopping on their own.
    shutdown.cancel();
    capture_task
        .await
        .context("capture supervisor task panicked")?;
    if let Some(d) = detection {
        d.join().await.context("detection task panicked")?;
    }

    Ok(())
}

/// Waits for Ctrl-C or, on Unix, SIGTERM - covers both an interactive
/// `cargo run` and a `systemctl stop` in production.
async fn wait_for_shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sigterm) => {
                sigterm.recv().await;
            }
            Err(err) => {
                tracing::warn!(error = %err, "failed to install SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received");
}
