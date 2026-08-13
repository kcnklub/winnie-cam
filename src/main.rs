mod capture;
mod config;
mod detect;
mod hub;
mod jpeg;
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
    // Extracted before `cfg` goes out of scope at the end of this block -
    // `Config` itself isn't stored anywhere past startup (see config.rs).
    let detect_cfg = detect::DetectConfig::from_config(&cfg)?;

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
    // coming up (see `detect::spawn`'s doc comment).
    let detect_handle = detect_cfg
        .map(|cfg| detect::spawn(cfg, hub.clone(), shutdown.clone()));
    let (detect_hub, detect_task) = match detect_handle {
        Some((hub, task)) => (Some(hub), Some(task)),
        None => (None, None),
    };

    let listener = tokio::net::TcpListener::bind(cfg.bind)
        .await
        .with_context(|| format!("binding to {}", cfg.bind))?;
    tracing::info!(bind = %cfg.bind, "winnie-cam listening");

    let app = web::router(web::AppState::new(hub, detect_hub, shutdown.clone()));
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
    if let Some(task) = detect_task {
        task.join().await.context("detection task panicked")?;
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
