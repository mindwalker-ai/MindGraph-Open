//! The `mindgraph-core` server process.

use std::process::ExitCode;

use mindgraph_api::{Config, SERVICE_NAME, router};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(error = %err, "{SERVICE_NAME} failed to run");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;
    let listener = TcpListener::bind(config.bind_addr).await?;
    tracing::info!(address = %listener.local_addr()?, "{SERVICE_NAME} listening");

    axum::serve(listener, router())
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("{SERVICE_NAME} stopped");
    Ok(())
}

/// Resolves on Ctrl-C or SIGTERM (sent by `docker compose stop`).
///
/// If a handler cannot be installed, that branch waits forever instead of
/// triggering an immediate shutdown.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::warn!(error = %err, "cannot listen for Ctrl-C");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(err) => {
                tracing::warn!(error = %err, "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutdown signal received");
}
