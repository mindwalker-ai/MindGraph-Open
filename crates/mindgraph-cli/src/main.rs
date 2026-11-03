//! The `mindgraph` command-line interface (ADR-001 §13).
//!
//! Status: skeleton. Only `--help` and `--version` work today. Commands such
//! as `start`, `import`, `demo` and `mcp` arrive in later milestones.

use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
MindGraph Open command-line interface

Usage: mindgraph <COMMAND>

Options:
  -h, --help     Print help
  -V, --version  Print version

Planned commands (not yet implemented):
  start | stop | status   Manage the local MindGraph stack
  import <file>           Import a CSV or JSON dataset
  demo <name>             Load a demo graph (fraud, cyber, knowledge)
  mcp                     Run the MCP server over stdio

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
        Some(other) => {
            eprintln!("mindgraph: unknown or not yet implemented command `{other}`\n\n{HELP}");
            ExitCode::from(2)
        }
    }
}
