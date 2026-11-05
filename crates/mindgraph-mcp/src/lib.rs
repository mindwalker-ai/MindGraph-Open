//! MCP server for MindGraph Open (ADR-011, refined in M8).
//!
//! `mindgraph-mcp` is a standalone stdio process that an MCP client (Claude
//! Code, Codex, Cursor, …) launches. It exposes small, composable, read-only
//! tools over **one** graph project and gets every result from MindGraph
//! Core's public REST API:
//!
//! ```text
//! MCP client ──stdio──► mindgraph-mcp ──HTTP (loopback)──► mindgraph-core
//!                                                          ├─ GraphExplorerService
//!                                                          └─ GraphRagService (ask_graph)
//! ```
//!
//! It never connects to PostgreSQL, HugeGraph or the AI service, links no
//! MindGraph server crate, and duplicates no graph logic: paths,
//! neighbourhoods and searches are Core's. stdout carries MCP messages only;
//! every diagnostic goes to stderr.

pub mod cli;
pub mod client;
pub mod config;
pub mod dto;
pub mod errors;
pub mod tools;

use rmcp::ServiceExt;
use tracing::info;

pub use crate::config::{Config, ConfigError};
pub use crate::errors::ToolError;
pub use crate::tools::MindGraphMcp;

/// Why the server stopped with an error.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// The HTTP client could not be built.
    #[error("cannot initialise the HTTP client: {0}")]
    Client(#[from] reqwest::Error),
    /// The MCP handshake failed (for example, stdin closed before it).
    #[error("MCP initialization failed: {0}")]
    Initialize(String),
    /// The MCP session ended abnormally.
    #[error("MCP session failed: {0}")]
    Session(String),
}

/// Serves MCP over stdin/stdout until the client disconnects (EOF) or the
/// process receives Ctrl-C / SIGTERM.
///
/// # Errors
///
/// [`ServeError`] when the client cannot be built or the session fails.
pub async fn serve_stdio(config: Config) -> Result<(), ServeError> {
    let server = MindGraphMcp::new(&config)?;
    info!(
        graph_id = %config.graph_id,
        core_url = %config.core_url,
        tools = ?server.tool_names(),
        "mindgraph-mcp serving MCP over stdio"
    );
    let service = server
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|error| ServeError::Initialize(error.to_string()))?;
    let token = service.cancellation_token();
    let signals = tokio::spawn(async move {
        shutdown_signal().await;
        token.cancel();
    });
    let quit = service.waiting().await;
    signals.abort();
    let reason = quit.map_err(|error| ServeError::Session(error.to_string()))?;
    info!(reason = ?reason, "mindgraph-mcp stopped");
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = terminate.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
