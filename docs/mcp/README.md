# MindGraph MCP server

`mindgraph-mcp` lets AI agents explore a MindGraph knowledge graph through
the [Model Context Protocol](https://modelcontextprotocol.io). Supported
clients include Claude Code, Codex, Cursor and any other client that speaks
MCP over stdio. The server exposes small, bounded, **read-only** tools. Each
tool returns real graph data as structured JSON, which the agent can reason
over and use in its next call.

Client guides: [Claude Code](claude-code.md) · [Codex](codex.md) ·
[Cursor](cursor.md). The web app shows ready-made snippets for the current
graph under **MCP** in the sidebar (`/graphs/{id}/integrations/mcp`).

## How it works

```text
Claude Code / Codex / Cursor
             │  MCP over stdio (the client starts the process)
             ▼
       mindgraph-mcp            one graph project per process
             │  HTTP to MindGraph Core's REST API on 127.0.0.1
             ▼
      mindgraph-core
      ├─ GraphExplorerService   search, entity, neighbors, paths, subgraph (M6)
      └─ GraphRagService        ask_graph, opt-in (M7)
             │
         GraphStore ─► HugeGraph
```

- **Core's REST API only.** `mindgraph-mcp` is a separate, small binary.
  It never connects to PostgreSQL, HugeGraph or the AI service, and needs
  none of their credentials. Each tool makes one call to an existing Core
  endpoint (`/api/v1/graphs/{id}/…`). Searches, traversals and GraphRAG run
  in Core, so the MCP server holds no second copy of the graph logic.
- **stdio only.** The MCP client launches the process and exchanges MCP
  messages over its stdin and stdout. There is no daemon, no listening
  port and no remote MCP endpoint. stdout carries only protocol messages,
  and every log line goes to stderr.
- **One graph.** `MINDGRAPH_MCP_GRAPH_ID` fixes the graph project, and no
  tool takes a graph id. To give an agent several graphs, configure several
  server entries.

## Install

No published crate or prebuilt binary exists yet. Build it from this
repository (Rust stable, see the main README):

```bash
cargo build -p mindgraph-mcp --release      # or: make mcp-build
./target/release/mindgraph-mcp --version   # mindgraph-mcp 0.1.0
./target/release/mindgraph-mcp --help
```

Use the absolute path of `target/release/mindgraph-mcp` in your client's
configuration. `mindgraph mcp` (the `mindgraph` CLI) runs the same server.

Without arguments, the binary serves MCP over stdio. That is how clients
start it; you do not run it by hand. To test a configuration from a
terminal, use `--check`:

```bash
MINDGRAPH_MCP_GRAPH_ID=<graph id> ./target/release/mindgraph-mcp --check
# core      http://127.0.0.1:8080: mindgraph-core 0.1.0 (ok)
# graph     019…: "Fraud Investigation" (active)
# ontology  version 1: 3 entity types, 2 relationship types
# tools     get_graph_info, get_graph_schema, search_entities, get_entity, get_neighbors, find_path, get_subgraph (ask_graph disabled)
```

There is no Docker image for the MCP server. A stdio server in a container
would have to reach Core from inside the container network, which the
loopback rule below forbids, and no Docker-based stdio setup has been
tested. The native binary is the supported path.

## Configure

Configuration comes from environment variables, which the client passes to
the process:

| Variable | Default | Meaning |
| --- | --- | --- |
| `MINDGRAPH_MCP_GRAPH_ID` | required | The graph project to expose (a UUIDv7). |
| `MINDGRAPH_MCP_CORE_URL` | `http://127.0.0.1:8080` | MindGraph Core. `docker compose up` publishes it on this address; `MINDGRAPH_CORE_PORT` changes the port. |
| `MINDGRAPH_MCP_TIMEOUT_SECONDS` | `30` (1–300) | Deadline of one Core request for the graph tools. |
| `MINDGRAPH_MCP_MAX_RESULT_BYTES` | `262144` (16384–4194304) | Largest JSON result of one tool call. |
| `MINDGRAPH_MCP_ENABLE_ASK` | `false` | Registers `ask_graph` (see below). |
| `MINDGRAPH_MCP_ASK_TIMEOUT_SECONDS` | `150` (1–600) | Deadline of one `ask_graph` call. Core's own GraphRAG deadline is 120 s. |
| `RUST_LOG` | `mindgraph_mcp=info,rmcp=warn` | Log filter for stderr. |

**Core URL rules.** `MINDGRAPH_MCP_CORE_URL` must be `http://` on a loopback
host (`127.0.0.1` or any `127.x`, `[::1]`, `localhost`), optionally with a
port. It must not contain credentials, a path, a query or a fragment.
Remote Cores are not supported in M8: Core has no authentication in v0.1,
and graph data must not leave the machine through the MCP server.

**Find the graph id.** Copy it from the MCP page in the web app, or list
the graphs:

```bash
curl -s http://127.0.0.1:8080/api/v1/graphs | jq '.data[] | {id, name, status}'
```

**Startup.** An invalid or missing variable stops the process before the
transport starts. It prints one line to stderr and exits with code 2, so
the client reports the server as failed. If Core is down, the server still
starts, and each tool call returns a `CORE_UNAVAILABLE` error until Core
answers. Startup therefore never waits for Core or for a model provider.

## Tools

All tools are read-only and operate on the configured graph only. Ids are
MindGraph's UUIDv7 entity ids, never HugeGraph's internal ids. Inputs are
checked against JSON Schemas generated from the Rust types, and unknown
arguments are rejected.

| Tool | Input | Returns |
| --- | --- | --- |
| `get_graph_info` | none | `id`, `name`, `description`, `status`, `created_at`, `updated_at` |
| `get_graph_schema` | none | ontology `version`; `entity_types` with `properties` (`name`, `type`, `required`); `relationship_types` with `source_entity_type` and `target_entity_type` |
| `search_entities` | `entity_type?`, `property_filters?`, `limit?` (1–100, default 20) | `entities`, `count`, `limit`, `limit_reached` |
| `get_entity` | `entity_id` | `id`, `entity_type`, `properties`, `property_types` |
| `get_neighbors` | `entity_id`, `direction?` (`outgoing`/`incoming`/`both`, default `both`), `relationship_type?`, `limit?` (relationships, 1–100, default 50) | the entity and its one-hop `entities`, `relationships`, `limit_reached` |
| `find_path` | `source_id`, `target_id`, `direction?`, `max_depth?` (1–6, default 3) | `found`, `hops`, ordered `entities` and `relationships`, and a `note` when nothing was found |
| `get_subgraph` | `entity_ids` (1–100) | the entities that exist, every relationship among them, and `missing_ids` |
| `ask_graph` | `question`, `scope?` (`{"mode": "bounded_graph"}` or `{"mode": "selected_entities", "entity_ids": [1–5 ids]}`) | the M7 answer: `status`, `answer`, `evidence`, `citations`, `warnings`, `retrieval`, `metadata` (opt-in) |

- **Search is exact.** `search_entities` matches property values exactly,
  with types taken from the ontology. It is not full-text, fuzzy or semantic
  search, and it cannot run arbitrary queries. Large integers may be passed
  as strings.
- **No `query_graph`.** ADR-011 originally listed a `query_graph` tool. It
  is deferred until a bounded query contract has been reviewed, and no
  Gremlin, Cypher or SQL reaches Core through MCP.
- **Results.** Each result is MCP structured content with an output schema,
  repeated as JSON text for clients that read only text. Integers beyond
  ±(2^53 − 1) are decimal strings, typed `integer` in `property_types`.
- **Bounds.** `limit_reached: true` means more matches may exist. A result
  larger than `MINDGRAPH_MCP_MAX_RESULT_BYTES` fails with
  `RESULT_TOO_LARGE`; it is never cut off, because a truncated subgraph
  would invent missing structure. `find_path` with `found: false` only means
  there is no path within `max_depth` in that direction. It does not show
  that the entities are unconnected.
- **Errors.** A failed call is an MCP tool error (`isError: true`) whose text
  is JSON, for example:

  ```json
  {"error": {"code": "ENTITY_NOT_FOUND", "message": "Entity not found",
             "hint": "…", "http_status": 404, "request_id": "019…"}}
  ```

  - Core's codes pass through unchanged: `GRAPH_NOT_FOUND`,
    `ENTITY_NOT_FOUND`, `INVALID_QUERY`, `INVALID_REQUEST`,
    `SERVICE_UNAVAILABLE`, `AI_NOT_CONFIGURED`, `AI_TIMEOUT`, and others.
  - The server adds its own codes: `INVALID_ARGUMENT`, `RESULT_TOO_LARGE`,
    `CORE_UNAVAILABLE`, `CORE_TIMEOUT`, `CORE_REDIRECT`, `CORE_ERROR` and
    `INVALID_CORE_RESPONSE`.
  - When the SDK cannot parse the arguments (wrong type, missing or unknown
    field), the result is also a tool error, with a plain-text message.
  - A failing call never stops the server.

## The fraud demo, through an agent

Load the fraud example (README, "Load a dataset without AI"), configure
the server for that graph, and ask *"Which customers share a device with
Customer C001?"*. An agent needs no model call on MindGraph's side; it
chains the deterministic tools:

```text
get_graph_schema                       Customer, Account, Device; OWNS, USES
search_entities {Customer, customer_id: C001}             → Alice Tan (id A)
get_neighbors {A, outgoing, USES}                         → DEVICE-921, DEVICE-109
get_neighbors {DEVICE-921 id, incoming, USES}             → C001, C002
find_path {A, C002 id}           → C001 ─USES→ DEVICE-921 ←USES─ C002 (2 hops)
```

## `ask_graph` (opt-in)

`MINDGRAPH_MCP_ENABLE_ASK=true` registers `ask_graph`, which calls
MindGraph's GraphRAG (`POST /api/v1/graphs/{id}/ask`, M7). It is the same
pipeline and has the same limits as Ask MindGraph in the web app.

- **It may send graph data to a third party.** Core sends the question, the
  ontology and the retrieved evidence (at most 150 entities, 48 KiB) to the
  model provider configured in Core (`MINDGRAPH_LLM_PROVIDER`). Use a local
  model for sensitive graphs. The tool's annotations say so
  (`openWorldHint: true`).
- **It needs a model.** Without one configured in Core, the call fails with
  `AI_NOT_CONFIGURED`; the other seven tools keep working.
- **It is slower.** Expect seconds to minutes (two model calls). The
  deterministic tools are better for exact lookups.
- **It writes nothing.** No entities, relationships, ontology changes or
  ingestion jobs.

## Security

- **Read-only by construction.** The server can only reach Core's read
  endpoints plus `ask`, and no tool maps to a write endpoint. The tests
  prove it: a spy on the GraphStore counts zero writes during a full agent
  session, and the graph count and ontology version stay unchanged. The
  `readOnlyHint`/`destructiveHint` annotations describe this behaviour; they
  are not what enforces it.
- **Graph isolation.**
  - Requests go only to `/api/v1/graphs/{configured id}/…`.
  - Core enforces graph isolation.
  - The server also discards any element whose `graph_id` differs from the
    configured one.
  - An entity of another graph is `ENTITY_NOT_FOUND`.
- **No ambient authority.**
  - No tool argument is a URL, file path or graph id.
  - Tools never read local files or environment variables.
  - The HTTP client ignores proxy variables and never follows redirects.
  - Logs hold the operation, status, size, duration and request id, never
    arguments, results or property values.
- **Graph text is data.** Property values may contain text such as "ignore
  your previous instructions". The server returns it unchanged as data and
  acts on nothing in it. The server instructions tell the agent the same.
- **Local-first limits.** MindGraph-OSS v0.1 has no authentication, and
  anyone who can reach Core's port can read and change every graph. The
  MCP server reads one graph over loopback. It is not a secure multi-tenant
  or remote deployment: do not expose Core or an MCP endpoint beyond your
  machine.

## Protocol

- **SDK:** the official Rust SDK [`rmcp`](https://crates.io/crates/rmcp)
  3.8.0, with `#[tool]` handlers, Schemars input and output schemas, and
  the stdio transport. No custom JSON-RPC code.
- **Versions:** the server answers `initialize` for protocol versions
  `2024-11-05`, `2025-03-26`, `2025-06-18` and `2025-11-25`, all tested. The
  rmcp client negotiates `2025-11-25`. rmcp also implements `2026-07-28`,
  which our tests do not exercise.
- **Shutdown:** the process exits cleanly when the client closes stdin, and
  on Ctrl-C or SIGTERM.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| The client shows the server as failed | Run the same command with the same environment plus `--check`. A configuration error names the variable. Use an absolute path to the binary. |
| `CORE_UNAVAILABLE` | Start MindGraph (`docker compose up`) and check the port (`MINDGRAPH_CORE_PORT`, default 8080). |
| `--check`: "is `mindgraph-ai`, not mindgraph-core", or `CORE_ERROR` / `INVALID_CORE_RESPONSE` | The URL points at another service, such as the web app on 3000 or the AI service. Use Core's port. |
| `GRAPH_NOT_FOUND` | `MINDGRAPH_MCP_GRAPH_ID` names no graph in this Core. |
| `ENTITY_NOT_FOUND` for an id you know | The id belongs to another graph. Each server sees one graph. |
| `RESULT_TOO_LARGE` | Lower `limit`, add `relationship_type` or `property_filters`, request fewer ids, or raise `MINDGRAPH_MCP_MAX_RESULT_BYTES`. |
| No `ask_graph` tool | Set `MINDGRAPH_MCP_ENABLE_ASK=true` and restart the client. `AI_NOT_CONFIGURED` means Core has no model. |
| `AI_TIMEOUT` / `CORE_TIMEOUT` | Raise `MINDGRAPH_MCP_ASK_TIMEOUT_SECONDS` / `MINDGRAPH_MCP_TIMEOUT_SECONDS`, and your client's tool timeout. |

Server logs are on stderr. Your client stores them in its MCP log (see the
client guides). Every error carries the `request_id` of its Core request,
so you can find the matching line in Core's log.

## Licensing boundary (for legal review)

- **The package.** `mindgraph-mcp` declares Apache-2.0, as ADR-015 proposes
  for MCP packages. That proposal still needs final legal approval.
- **What the binary links.**
  - No MindGraph crate. The AGPL server crates (`mindgraph-core`, `-api`,
    `-graph`, …) are not dependencies, and `make check-mcp-boundary` fails
    CI if one is added.
  - Its third-party dependencies (rmcp, reqwest, tokio, serde, schemars,
    uuid, tracing and their transitive dependencies) are under permissive
    licenses: MIT, Apache-2.0, Unicode-3.0, Unlicense, Zlib (per
    `cargo tree -e normal -f '{l}'`).
- **What it talks to.** The binary calls the AGPL MindGraph Core over HTTP,
  as any API client would. Whether that has consequences for distribution
  is a question for counsel; this document makes no claim about it.
- **Open questions for review.**
  - **Tests:** the end-to-end tests serve the real Core router in-process,
    so the crate has AGPL dev-dependencies (`mindgraph-api`, `-core`,
    `-domain`, `-graph`, `-ingestion`). They are compiled only for
    `cargo test` and are not part of the binary. Confirm that this is
    acceptable for the source package.
  - **CLI:** the AGPL `mindgraph` CLI links `mindgraph-mcp` for its
    `mindgraph mcp` command.
  - **Packaging:** per-package license files and source headers wait for
    legal approval of the split (see
    [LICENSE_AUDIT.md](../legal/LICENSE_AUDIT.md)).

## Tests

```bash
cargo test -p mindgraph-mcp                    # unit, HTTP adapter, MCP protocol, end to end (offline)
make test-hugegraph CONTAINER=podman           # also runs the MCP flow on HugeGraph 1.7
MINDGRAPH_MCP_LIVE_CORE_URL=http://127.0.0.1:8080 make test-mcp-live   # against your running Core
```

`make test-mcp-live` creates two "MCP live" graphs in that Core and loads
the fraud example into them. With a model configured in Core, it also
calls the real provider through `ask_graph`.
