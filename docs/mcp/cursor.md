# MindGraph in Cursor

> **Not tested with Cursor yet.** This page follows Cursor's MCP
> documentation as read on 2026-11-05. The server itself passed protocol
> tests with the official MCP Rust client and a real session with Claude
> Code. Please report what differs in your Cursor version.

General reference: [MCP server README](README.md).

## 1. Start MindGraph and build the server

```bash
docker compose up                          # Core on http://127.0.0.1:8080
cargo build -p mindgraph-mcp --release     # target/release/mindgraph-mcp
```

Copy the graph id from **MCP** in the web app's sidebar, which shows this
configuration filled in.

## 2. Add the server

Create `.cursor/mcp.json` in your project, or `~/.cursor/mcp.json` for all
projects. When both define `mindgraph`, the project file wins.

```json
{
  "mcpServers": {
    "mindgraph": {
      "type": "stdio",
      "command": "/absolute/path/to/MindGraph-Open/target/release/mindgraph-mcp",
      "args": [],
      "env": {
        "MINDGRAPH_MCP_CORE_URL": "http://127.0.0.1:8080",
        "MINDGRAPH_MCP_GRAPH_ID": "<graph id>"
      }
    }
  }
}
```

- `command` must be an absolute path, or a command on your `PATH`.
- To enable GraphRAG, add `"MINDGRAPH_MCP_ENABLE_ASK": "true"` to `env`.
  Read [`ask_graph`](README.md#ask_graph-opt-in) first: it sends graph data
  to Core's model provider.
- To give Cursor a second graph, add a second entry with its own graph id.

## 3. Start it and discover the tools

1. Save the file and restart Cursor.
2. Open **Customize** in the sidebar, then **MCPs**, and enable
   `mindgraph`. Its tools appear in the tools list at the top of the chat
   panel.
3. Cursor asks for approval before it uses an MCP tool.
4. Try:

   > Find Customer C001 and show its relationships.

## Troubleshooting

- Cursor writes MCP logs to the Output panel (Cmd+Shift+U, then
  "MCP Logs"). `mindgraph-mcp` logs to stderr, so its messages appear
  there.
- Run the configured command with the same environment and `--check`:
  ```bash
  MINDGRAPH_MCP_GRAPH_ID=<graph id> /absolute/path/to/mindgraph-mcp --check
  ```
- More symptoms: [MCP README › Troubleshooting](README.md#troubleshooting).
