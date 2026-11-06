# MindGraph in Codex

> **Not tested with Codex yet.** This page follows the Codex MCP
> documentation as read on 2026-11-05. The TOML below was checked with a
> TOML parser, and the server itself passed protocol tests with the official
> MCP Rust client and a real session with Claude Code. Please report what
> differs in your Codex version.

General reference: [MCP server README](README.md).

## 1. Start MindGraph and build the server

```bash
docker compose up                          # Core on http://127.0.0.1:8080
cargo build -p mindgraph-mcp --release     # target/release/mindgraph-mcp
```

Copy the graph id from **MCP** in the web app's sidebar, which shows this
configuration filled in.

## 2. Add the server

With the CLI:

```bash
codex mcp add mindgraph \
  --env MINDGRAPH_MCP_CORE_URL=http://127.0.0.1:8080 \
  --env MINDGRAPH_MCP_GRAPH_ID=<graph id> \
  -- /absolute/path/to/MindGraph-Open/target/release/mindgraph-mcp
```

Or in `~/.codex/config.toml`. Codex also reads `.codex/config.toml` in
projects you trust.

```toml
[mcp_servers.mindgraph]
command = "/absolute/path/to/MindGraph-Open/target/release/mindgraph-mcp"

[mcp_servers.mindgraph.env]
MINDGRAPH_MCP_CORE_URL = "http://127.0.0.1:8080"
MINDGRAPH_MCP_GRAPH_ID = "<graph id>"
```

To enable GraphRAG, add `MINDGRAPH_MCP_ENABLE_ASK = "true"`, and read
[`ask_graph`](README.md#ask_graph-opt-in) first: it sends graph data to
Core's model provider. Codex's documented default tool timeout
(`tool_timeout_sec`) is 60 seconds, and a GraphRAG answer can take longer,
so raise it:

```toml
[mcp_servers.mindgraph]
command = "/absolute/path/to/MindGraph-Open/target/release/mindgraph-mcp"
tool_timeout_sec = 180

[mcp_servers.mindgraph.env]
MINDGRAPH_MCP_CORE_URL = "http://127.0.0.1:8080"
MINDGRAPH_MCP_GRAPH_ID = "<graph id>"
MINDGRAPH_MCP_ENABLE_ASK = "true"
```

`mindgraph-mcp` starts without waiting for Core, well within Codex's
documented `startup_timeout_sec` default of 10 seconds.

## 3. Discover the tools

Start a new Codex session. `codex mcp list` lists the configured servers,
and `/mcp` in the Codex TUI shows the active ones. Then try:

> Find Customer C001 and show its relationships.

## Troubleshooting

Run the configured command with the same environment and `--check`:

```bash
MINDGRAPH_MCP_GRAPH_ID=<graph id> /absolute/path/to/mindgraph-mcp --check
```

More symptoms: [MCP README › Troubleshooting](README.md#troubleshooting).
