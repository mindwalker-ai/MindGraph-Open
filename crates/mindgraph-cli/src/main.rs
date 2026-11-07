//! The `mindgraph` command-line interface (ADR-001 §13).
//!
//! Status: `mcp` (M8) runs the MCP server and `demo` (M10) loads a demo
//! scenario through the REST API; `--help` and `--version` work. Commands
//! such as `start` and `import` are planned.

use std::process::ExitCode;

mod demo;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
MindGraph Open command-line interface

Usage: mindgraph <COMMAND>

Commands:
  demo <SCENARIO>         Load a demo scenario (fraud, cyber, knowledge-graph)
                          through the Core REST API (see `mindgraph demo --help`)
  mcp [--check]           Run the MCP server over stdio (same as `mindgraph-mcp`;
                          see `mindgraph mcp --help`)

Options:
  -h, --help     Print help
  -V, --version  Print version

Planned commands (not yet implemented):
  start | stop | status   Manage the local MindGraph stack
  import <file>           Import a CSV or JSON dataset

For now, start MindGraph with `docker compose up`.";

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        None | Some("-h" | "--help" | "help") => {
            println!("{HELP}");
            ExitCode::SUCCESS
        }
        Some("-V" | "--version" | "version") => {
            println!("mindgraph {VERSION}");
            ExitCode::SUCCESS
        }
        // A thin launcher: argument handling, stdio and logging all belong
        // to mindgraph-mcp, so both entry points behave identically.
        Some("mcp") => mindgraph_mcp::cli::run(std::env::args().skip(2)),
        Some("demo") => demo::run(std::env::args().skip(2)),
        Some(other) => {
            eprintln!("mindgraph: unknown or not yet implemented command `{other}`\n\n{HELP}");
            ExitCode::from(2)
        }
    }
}
