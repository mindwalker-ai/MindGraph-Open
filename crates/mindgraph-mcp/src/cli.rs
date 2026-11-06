//! Command-line entry point shared by `mindgraph-mcp` and `mindgraph mcp`.
//!
//! Without arguments the process speaks MCP over stdio: that is how MCP
//! clients launch it, so nothing but protocol messages may reach stdout.
//! `--help`, `--version` and `--check` are for people in a terminal; they
//! never start the transport.

use std::process::ExitCode;
use std::time::Duration;

use tracing_subscriber::EnvFilter;

use crate::client::CoreClient;
use crate::config::Config;
use crate::tools::{ASK_TOOL, GRAPH_TOOLS};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Usage text for `--help`.
pub const HELP: &str = "\
mindgraph-mcp: read-only MindGraph tools for AI agents over the Model Context Protocol

Usage: mindgraph-mcp [OPTIONS]

Without options, serves MCP over stdin/stdout. Your MCP client (Claude Code,
Codex, Cursor, ...) starts this process; you do not run it by hand.

Options:
      --check    Check the configuration and that MindGraph Core and the graph are reachable
  -h, --help     Print help
  -V, --version  Print version

Environment:
  MINDGRAPH_MCP_GRAPH_ID             Graph project to expose (required, a UUIDv7)
  MINDGRAPH_MCP_CORE_URL             MindGraph Core on a loopback address [default: http://127.0.0.1:8080]
  MINDGRAPH_MCP_TIMEOUT_SECONDS      Deadline of one Core request, 1-300 [default: 30]
  MINDGRAPH_MCP_MAX_RESULT_BYTES     Largest tool result, 16384-4194304 [default: 262144]
  MINDGRAPH_MCP_ENABLE_ASK           Register ask_graph (GraphRAG; sends graph data to the
                                     model provider configured in Core) [default: false]
  MINDGRAPH_MCP_ASK_TIMEOUT_SECONDS  Deadline of one ask_graph request, 1-600 [default: 150]
  RUST_LOG                           Log filter for stderr [default: mindgraph_mcp=info,rmcp=warn]

Docs: docs/mcp/README.md";

/// Runs the command line `args` (without the program name) and returns the
/// process exit code: 0 on success, 1 when serving or `--check` fails, 2 for
/// usage or configuration errors.
#[must_use]
pub fn run(args: impl IntoIterator<Item = String>) -> ExitCode {
    let args: Vec<String> = args.into_iter().collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => serve(),
        ["-h" | "--help" | "help"] => {
            println!("{HELP}");
            ExitCode::SUCCESS
        }
        ["-V" | "--version" | "version"] => {
            println!("mindgraph-mcp {VERSION}");
            ExitCode::SUCCESS
        }
        ["--check"] => check(),
        _ => {
            eprintln!(
                "mindgraph-mcp: unexpected arguments {args:?}; configuration comes from MINDGRAPH_MCP_* environment variables\n\n{HELP}"
            );
            ExitCode::from(2)
        }
    }
}

/// Logs to stderr only, without colours (clients store stderr in log files).
fn init_logging() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("mindgraph_mcp=info,rmcp=warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_target(false)
        .try_init();
}

fn load_config() -> Result<Config, ExitCode> {
    Config::from_env().map_err(|error| {
        eprintln!("mindgraph-mcp: invalid configuration: {error}");
        ExitCode::from(2)
    })
}

fn runtime() -> Result<tokio::runtime::Runtime, ExitCode> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|error| {
            eprintln!("mindgraph-mcp: cannot start the async runtime: {error}");
            ExitCode::FAILURE
        })
}

fn serve() -> ExitCode {
    init_logging();
    let config = match load_config() {
        Ok(config) => config,
        Err(code) => return code,
    };
    let runtime = match runtime() {
        Ok(runtime) => runtime,
        Err(code) => return code,
    };
    let result = runtime.block_on(crate::serve_stdio(config));
    // Tokio reads stdin on a blocking thread that may still wait for input
    // after a signal; do not let it keep the process alive.
    runtime.shutdown_timeout(Duration::from_millis(500));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = %error, "mindgraph-mcp stopped with an error");
            ExitCode::FAILURE
        }
    }
}

/// `--check`: validates the configuration, then asks Core for its health,
/// the graph and its ontology. Prints a short report to stdout.
fn check() -> ExitCode {
    let config = match load_config() {
        Ok(config) => config,
        Err(code) => return code,
    };
    let runtime = match runtime() {
        Ok(runtime) => runtime,
        Err(code) => return code,
    };
    let code = runtime.block_on(async {
        let client = match CoreClient::new(&config) {
            Ok(client) => client,
            Err(error) => {
                eprintln!("mindgraph-mcp: cannot initialise the HTTP client: {error}");
                return ExitCode::FAILURE;
            }
        };
        let core = config.core_url.as_str().trim_end_matches('/');
        match client.health().await {
            Ok(health) if health.service == "mindgraph-core" => {
                println!(
                    "core      {core}: {} {} ({})",
                    health.service, health.version, health.status
                );
            }
            Ok(health) => {
                eprintln!(
                    "mindgraph-mcp: {core} is `{}`, not mindgraph-core",
                    health.service
                );
                return ExitCode::FAILURE;
            }
            Err(error) => {
                eprintln!("mindgraph-mcp: {error}");
                return ExitCode::FAILURE;
            }
        }
        match client.graph().await {
            Ok(graph) => println!(
                "graph     {}: \"{}\" ({})",
                graph.id, graph.name, graph.status
            ),
            Err(error) => {
                eprintln!("mindgraph-mcp: graph {}: {error}", config.graph_id);
                return ExitCode::FAILURE;
            }
        }
        match client.ontology().await {
            Ok(ontology) => println!(
                "ontology  version {}: {} entity types, {} relationship types",
                ontology.version,
                ontology.entity_types.len(),
                ontology.relationship_types.len()
            ),
            Err(error) => println!("ontology  unavailable: {error}"),
        }
        let ask = if config.enable_ask {
            format!(", {ASK_TOOL}")
        } else {
            format!(" ({ASK_TOOL} disabled)")
        };
        println!("tools     {}{ask}", GRAPH_TOOLS.join(", "));
        ExitCode::SUCCESS
    });
    runtime.shutdown_timeout(Duration::from_millis(500));
    code
}
