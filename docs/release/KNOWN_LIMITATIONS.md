# Known limitations (v0.1.0-alpha.1)

MindGraph-OSS v0.1 is an alpha for local exploration and evaluation. These
are the limits you will meet, grouped by area. Details per milestone are in
[docs/architecture/](../architecture/README.md) (sections "Known M2…M9
limitations").

## Security and deployment

- **No authentication, users, roles or API keys.** Anyone who can reach the
  web app or the Core API can read and change every graph. Run it on your own
  machine; do not expose it to a network (see [SECURITY.md](../../SECURITY.md)).
- No CSRF protection and no full Content-Security-Policy yet.
- Single node only: no high availability, clustering, replication or
  sharding; no Kubernetes manifests.
- No published container images or binaries: images are built locally with
  `docker compose up --build` (a manual GHCR workflow exists but has not
  published anything), and the MCP binary is built with Cargo.
- Requires about 4 GB of memory for Docker or Podman and several GB of disk
  for the first build.

## Data and graphs

- Input formats: CSV with a header row and JSON arrays of flat objects only.
  No database connectors, streaming, CDC or scheduled imports. Uploads are
  limited to 50 MiB and 100,000 records per file by default.
- **No deletion**: graphs, entities, relationships and data sources cannot
  be deleted or edited through the UI or API. Start over with a fresh volume
  (see [data and backups](../operations/backup-restore.md)).
- Ingestion writes are not transactional across batches: a failed job may
  leave earlier batches written.
- Ontologies are append-only versions; existing types cannot be renamed or
  removed in place.
- One HugeGraph version (1.7.0) with one generic physical schema; changing
  the HugeGraph image needs a fresh volume and a re-import. Backups stop
  writes (no hot backup).

## Graph Explorer and queries

- Exact search only (type + property value or entity id): no full-text,
  fuzzy or semantic search.
- The browser holds at most 500 entities and 1,500 relationships; the canvas
  shows what has been loaded, not the whole graph, and counts are of loaded
  data. The home page shows ontology type counts, not graph-wide entity
  counts.
- Shortest path: one path, up to 6 hops, no filter by relationship type. On
  HugeGraph a search stops after expanding 2,000 entities.
- No query language (Gremlin/Cypher) and no graph algorithms beyond
  neighbours and shortest path.
- Requires WebGL in the browser.

## AI features (optional)

- One OpenAI-compatible provider at a time. Quality depends on the model;
  suggestions can be wrong and must be reviewed.
- Ask MindGraph retrieves bounded evidence (at most 150 entities and 300
  relationships per question) with four operations; "whole graph" questions
  inspect at most 100 candidates and say so. Conversations live in the
  browser only. A valid citation proves the cited object was retrieved, not
  that every sentence is correct.
- No vector search or embeddings.
- Data sent to a configured provider is described in the
  [AI Graph Builder](../tutorials/ai-graph-builder.md) and
  [Ask MindGraph](../tutorials/ask-mindgraph.md) guides.

## Entity resolution

- Deterministic rules only (exact identifier, normalised email, phone and
  text, fuzzy name); no machine learning.
- A scan examines at most 999 entities of one type and compares at most
  20,000 pairs; larger types are scanned partially and the coverage says so.
- Linking creates a logical identity only: records are never merged, and the
  Explorer, Ask MindGraph and MCP still show the original records. Decisions
  cannot be undone in v0.1.

## MCP server

- Read-only, one graph per server process, stdio transport only.
- Tested with Claude Code as a real client; the Codex and Cursor guides follow
  their documented configuration formats but were not run against those
  clients.
- `ask_graph` is off by default because it sends graph evidence to the model
  provider.

## Demos

- The three demo datasets are small and synthetic. They demonstrate
  patterns; nothing in them is a confirmed fraud case or security incident.
- `make demo-*` reuses a graph with the demo's exact name; it finds it among
  the 100 most recent graphs.

## Testing coverage

- The full-stack smoke test and the browser journeys run on Linux in CI and
  were run locally on macOS with Podman. Windows has not been tested.
- Browser journeys run in Chromium only, with software WebGL in headless CI.
- No load or concurrency testing; see the [benchmarks](../benchmarks/v0.1.0.md)
  for what was measured.

## Licensing

- The licensing split (AGPL-3.0 server, Apache-2.0 MCP package and examples)
  is **pending legal approval**; see
  [LICENSE_AUDIT.md](../legal/LICENSE_AUDIT.md).
