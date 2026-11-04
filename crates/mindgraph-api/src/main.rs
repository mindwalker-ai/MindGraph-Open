//! The `mindgraph-core` server process: the composition root.
//!
//! ```text
//! config → PostgreSQL pool → migrations ─► PostgresMetadataStore ─┬► GraphProjectService
//!        → HugeGraph schema (ensure_schema) ─► HugeGraphStore ──┐  ├► OntologyService
//!        → <data dir>/sources ─► LocalSourceStore ──────────────┴──┴► IngestionService
//!        → MINDGRAPH_AI_URL ─► HttpGraphBuilderAi (or AI disabled) ─► GraphBuilderService
//!        → restart recovery (fail cut-off jobs) → router → serve
//! ```
//!
//! Startup fails fast when PostgreSQL or HugeGraph is unreachable; Docker's
//! restart policy restarts the process. The Python AI service is optional
//! (ADR-007): it is never contacted at startup, and every manual flow works
//! without it.

use std::error::Error;
use std::process::ExitCode;
use std::sync::Arc;

use mindgraph_ai_client::HttpGraphBuilderAi;
use mindgraph_api::{AppState, Config, SERVICE_NAME, router};
use mindgraph_core::{
    DisabledGraphBuilderAi, GraphBuilderAi, GraphBuilderConfig, GraphBuilderService,
    GraphExplorerService, GraphProjectService, IngestionPorts, IngestionService, MetadataStore,
    OntologyService,
};
use mindgraph_graph::{GraphStore, HugeGraphConfig, HugeGraphStore};
use mindgraph_ingestion::LocalSourceStore;
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

    // One HugeGraph adapter, shared by ingestion (writes) and the explorer (reads).
    let graph: Arc<dyn GraphStore> = Arc::new(connect_hugegraph(&config).await?);
    tracing::info!("HugeGraph schema ready");
    let sources = LocalSourceStore::new(&config.data_dir)
        .await
        .map_err(|err| format!("cannot prepare MINDGRAPH_DATA_DIR: {}", error_chain(&err)))?;

    let graph_projects = GraphProjectService::bootstrap(Arc::clone(&metadata)).await?;
    let workspace_id = graph_projects.workspace_id();
    let ontology = OntologyService::new(Arc::clone(&metadata), workspace_id);
    let graph_explorer =
        GraphExplorerService::new(Arc::clone(&metadata), Arc::clone(&graph), workspace_id);
    let ingestion = IngestionService::start(
        IngestionPorts {
            metadata,
            graph,
            sources: Arc::new(sources),
        },
        workspace_id,
        config.ingestion,
    )?;
    let ai: Arc<dyn GraphBuilderAi> = match config.ai.url() {
        Some(url) => {
            tracing::info!("AI Graph Builder enabled (internal AI service)");
            Arc::new(HttpGraphBuilderAi::new(url, config.ai.timeout)?)
        }
        None => {
            tracing::info!("AI Graph Builder disabled (MINDGRAPH_AI_URL is not set)");
            Arc::new(DisabledGraphBuilderAi)
        }
    };
    let graph_builder = GraphBuilderService::new(
        ai,
        &ingestion,
        GraphBuilderConfig {
            max_repair_attempts: config.ai.max_repair_attempts,
            job_concurrency: config.ai.job_concurrency,
        },
    )?;
    // Before serving: jobs of a previous process can never finish.
    ingestion.recover_incomplete_jobs().await?;
    let app = router(
        AppState::new(graph_projects, ontology, ingestion)
            .with_graph_builder(graph_builder)
            .with_graph_explorer(graph_explorer),
    );

    let listener = TcpListener::bind(config.bind_addr).await?;
    tracing::info!(address = %listener.local_addr()?, "{SERVICE_NAME} listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    tracing::info!("{SERVICE_NAME} stopped");
    Ok(())
}

/// Builds the HugeGraph adapter and creates its physical schema (M2's
/// `ensure_schema`, idempotent). Fails when HugeGraph is unreachable.
async fn connect_hugegraph(config: &Config) -> Result<HugeGraphStore, Box<dyn Error>> {
    let hint = |err: &dyn Error| {
        format!(
            "cannot initialise HugeGraph (check MINDGRAPH_HUGEGRAPH_URL): {}",
            error_chain(err)
        )
    };
    // Not `hint`: this error would quote the URL.
    let mut settings = HugeGraphConfig::new(config.hugegraph.url()).map_err(
        |_| "MINDGRAPH_HUGEGRAPH_URL must be an http:// URL such as http://hugegraph:8080",
    )?;
    if let Some(graphspace) = &config.hugegraph.graphspace {
        settings = settings
            .with_graphspace(graphspace)
            .map_err(|err| hint(&err))?;
    }
    if let Some(graph) = &config.hugegraph.graph {
        settings = settings.with_graph_name(graph).map_err(|err| hint(&err))?;
    }
    let store = HugeGraphStore::new(settings).map_err(|err| hint(&err))?;
    store.ensure_schema().await.map_err(|err| hint(&err))?;
    Ok(store)
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
