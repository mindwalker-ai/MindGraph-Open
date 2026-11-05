# MindGraph in Claude Code

This setup was tested with Claude Code 2.1.318 on macOS against a running
MindGraph Core. See [Verified](#verified) for what was and was not run.
General reference: [MCP server README](README.md).

## 1. Start MindGraph and load a graph

```bash
docker compose up          # Core on http://127.0.0.1:8080
```

Create a graph in the web app (http://localhost:3000), or load the fraud
example with the curl steps in the main README.

## 2. Build `mindgraph-mcp`

```bash
cargo build -p mindgraph-mcp --release
realpath target/release/mindgraph-mcp      # the absolute path for step 3
```

## 3. Add the server

Copy the graph id from **MCP** in the web app's sidebar, which also shows
these commands filled in. Run this in the project where you use Claude
Code:

```bash
claude mcp add --scope project mindgraph \
  --env MINDGRAPH_MCP_CORE_URL=http://127.0.0.1:8080 \
  --env MINDGRAPH_MCP_GRAPH_ID=<graph id> \
  -- /absolute/path/to/MindGraph-Open/target/release/mindgraph-mcp
```

`--scope project` writes `.mcp.json` in the project:

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

The `.mcp.json` holds your local binary path and graph id, so think before
committing it. Other scopes:

- `--scope local` (the default) keeps the server private to you in this
  project.
- `--scope user` makes it available in all your projects.

To expose a second graph, add another entry such as `mindgraph-cyber` with
its own `MINDGRAPH_MCP_GRAPH_ID`. To enable GraphRAG, add
`--env MINDGRAPH_MCP_ENABLE_ASK=true`, and read [`ask_graph`](README.md#ask_graph-opt-in)
first: it sends graph data to Core's model provider.

## 4. Start a session and discover the tools

Start `claude` in the project. Claude Code asks you to approve servers from
a project's `.mcp.json` before it starts them. Until you do,
`claude mcp list` shows `⏸ Pending approval`. After approval, `/mcp` lists
`mindgraph` and its seven tools: `get_graph_info`, `get_graph_schema`,
`search_entities`, `get_entity`, `get_neighbors`, `find_path` and
`get_subgraph`.

Try:

> Find Customer C001 and show its relationships.

> Which customers share device DEVICE-921, and what is the shortest
> connection between C001 and C002?

Claude Code asks before each tool call unless you allow them, for example
with `/permissions` or `--allowedTools mcp__mindgraph`.

## Non-interactive use

To use a config file without the approval step, for example in scripts:

```bash
claude -p "Find Customer C001 and show its relationships." \
  --mcp-config .mcp.json --strict-mcp-config --allowedTools mcp__mindgraph
```

## Troubleshooting

- `claude mcp get mindgraph` shows the command, the environment and the
  connection status.
- `✘ Failed to connect`: run the same command with the same environment and
  `--check`, which names the problem:
  ```bash
  MINDGRAPH_MCP_GRAPH_ID=<graph id> /absolute/path/to/mindgraph-mcp --check
  ```
- Remove the server with `claude mcp remove mindgraph -s project`.
- More symptoms: [MCP README › Troubleshooting](README.md#troubleshooting).

## Verified

- **Configuration.** On 2026-11-05, `claude mcp add --scope project`
  (both `-e` and `--env`) wrote the `.mcp.json` above.
  `claude mcp list` then reported the server as "Pending approval".
- **Agent session.** A real agent session ran with `claude -p
  --mcp-config .mcp.json --strict-mcp-config --allowedTools mcp__mindgraph`
  against Core with PostgreSQL and HugeGraph and the fraud example loaded:
  - Claude Code connected and listed the seven tools.
  - It called `get_graph_info`, `get_graph_schema`, `search_entities`,
    `get_neighbors` and `find_path`.
  - Its answers matched the CSV files: C001 owns A1001 and A1002 and uses
    DEVICE-921 and DEVICE-109; C001 and C002 share DEVICE-921; the
    shortest connection is the 2-hop path
    C001 → DEVICE-921 → C002.
- **Not verified.** The interactive approval dialog and the `/mcp` view
  were not exercised in that run.
