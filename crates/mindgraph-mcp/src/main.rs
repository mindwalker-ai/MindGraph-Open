//! `mindgraph-mcp`: the MindGraph MCP server binary (stdio transport).

use std::process::ExitCode;

fn main() -> ExitCode {
    mindgraph_mcp::cli::run(std::env::args().skip(1))
}
