//! The `mindgraph-core` server process: the composition root.
//!
//! ```text
//! config → PostgreSQL pool → migrations → local workspace
//!        → PostgresMetadataStore → GraphProjectService → router → serve
//! ```
//!
//! Startup fails fast when PostgreSQL is unreachable; Docker's restart policy
//! restarts the process. The Python AI service is not needed (ADR-007).

use std::error::Error;
use std::process::ExitCode;
use std::sync::Arc;

use mindgraph_api::{AppState, Config, SERVICE_NAME, router};
use mindgraph_core::{GraphProjectService, MetadataStore};
use mindgraph_metadata::{PostgresMetadataStore, PostgresOptions};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(error = %error_chain(err.as_ref()), "{SERVICE_NAME} failed to start");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn Error>> {
    let config = Config::from_env()?;

    let options = PostgresOptions::new(config.database.url())
        .with_max_connections(config.database.max_connections);
    let store = PostgresMetadataStore::connect(&options)
        .await
        .map_err(|err| {
            format!(
                "cannot connect to PostgreSQL (check MINDGRAPH_DATABASE_URL): {}",
                error_chain(&err)
            )
        })?;
    store.migrate().await?;
    tracing::info!("database migrations applied");

    let metadata: Arc<dyn MetadataStore> = Arc::new(store);
    let graph_projects = GraphProjectService::bootstrap(metadata).await?;
    let app = router(AppState::new(graph_projects));

    let listener = TcpListener::bind(config.bind_addr).await?;
    tracing::info!(address = %listener.local_addr()?, "{SERVICE_NAME} listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    tracing::info!("{SERVICE_NAME} stopped");
    Ok(())
}

/// `error: cause: cause` on one line, for logs.
fn error_chain(err: &dyn Error) -> String {
    let mut message = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
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
