<p align="center">
  <img src="assets/logo.png" alt="MindGraph" width="360">
</p>

<p align="center"><strong>Open-source Graph Intelligence for humans and AI agents.</strong></p>

MindGraph Open turns messy, structured data into an explorable knowledge graph.
You can then investigate relationships visually, ask questions with GraphRAG, and
connect your AI agents through MCP.

```text
Data → Graph → Explore → Ask → Investigate → Connect to AI Agents
```

> **Status: pre-alpha.** Milestones **M0** (repository bootstrap) and **M1**
> (domain foundation) are complete. The graph store, ingestion, graph explorer,
> GraphRAG and MCP arrive in the next milestones. See the [roadmap](#roadmap).

---

## Quick start

The only requirement is Docker with Compose v2.

```bash
git clone https://github.com/mindwalker-ai/MindGraph-Open
cd MindGraph-Open
docker compose up --build
```

Then open **http://localhost:3000**.

| Service     | Role                                 | Address                      |
| ----------- | ------------------------------------ | ---------------------------- |
| `web`       | Next.js UI                           | http://localhost:3000        |
| `core`      | Rust platform API (`mindgraph-core`) | http://localhost:8080/health |
| `ai`        | Python AI service (FastAPI)          | internal only                |
| `postgres`  | Application metadata                 | internal only                |
| `hugegraph` | Graph storage (Apache HugeGraph)     | internal only                |

- Published ports bind to `127.0.0.1` only. Postgres, HugeGraph and the AI
  service stay on the internal Docker network (ADR-013).
- On first start, HugeGraph initialises its storage, which can take a minute
  or two. Run `docker compose ps` to watch service health.
- Every setting has a local default. To override one, copy `.env.example` to
  `.env`. Never commit `.env`.
- To wipe all local data, run `docker compose down -v`.

## Architecture

> **Rust owns the graph platform. Python owns AI-specific intelligence.**

```text
           Next.js + TypeScript + Tailwind + shadcn/ui (+ Sigma.js in M6)
                                  │  REST / SSE
                                  ▼
                 mindgraph-core  (Rust · Axum · Tokio)  ── modular monolith
                  │                    │                     │
                  ▼                    ▼                     ▼
          Apache HugeGraph        PostgreSQL           mindgraph-ai
            graph data          app metadata      (Python · FastAPI, internal)

          mindgraph-mcp and mindgraph-cli: additional Rust binaries on the same core
```

The architecture is defined by the Architecture Decision Records in
[`docs/adr/`](docs/adr/), the highest technical source of truth. The crate
map and dependency rules are summarised in
[`docs/architecture/`](docs/architecture/README.md). Current and planned
endpoints are listed in [`docs/api/`](docs/api/README.md).

## Repository layout

```text
apps/web/                   Next.js frontend (placeholder until M6)
crates/
  mindgraph-domain/         Infrastructure-free domain model (M1)
  mindgraph-core/           Application layer / use cases
  mindgraph-api/            HTTP API + the `mindgraph-core` server binary
  mindgraph-graph/          GraphStore trait, Memory + HugeGraph adapters (M2)
  mindgraph-ingestion/      CSV / JSON ingestion (M4)
  mindgraph-ontology/       Ontology workflows (M3, M5)
  mindgraph-entity-resolution/  Duplicate detection (M9)
  mindgraph-graphrag/       Graph-first retrieval (M7)
  mindgraph-mcp/            MCP server (M8)
  mindgraph-cli/            `mindgraph` CLI
services/ai/                Python AI service (FastAPI, uv)
docker/                     Dockerfiles for web, core and ai
docs/                       ADRs, architecture and API docs
examples/                   Demo datasets (M10)
migrations/                 PostgreSQL migrations (M3)
```

## Development

To work on the code you need [Rust](https://rustup.rs) stable, Node.js 22+,
and [uv](https://docs.astral.sh/uv/) (it installs Python 3.13
for you). Docker is only needed for the full stack.

```bash
make setup     # npm ci (apps/web) + uv sync (services/ai)
make check     # run every check CI runs
make help      # list all targets
```

| Command                      | What it does                                                  |
| ---------------------------- | ------------------------------------------------------------- |
| `make up` / `make down`      | Start and stop the Docker Compose stack                       |
| `make dev-core`              | Run `mindgraph-core` on `127.0.0.1:8080` with `cargo run`     |
| `make dev-web`               | Run the Next.js dev server on `localhost:3000`                |
| `make dev-ai`                | Run the AI service on `127.0.0.1:8000` with reload            |
| `make rust-check`            | `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` |
| `make web-check`             | TypeScript typecheck, ESLint, Prettier check, Vitest          |
| `make ai-check`              | Ruff lint, Ruff format check, pytest                          |
| `make check-domain-boundary` | Fail if `mindgraph-domain` depends on infrastructure crates   |
| `make fmt`                   | Format Rust, TypeScript and Python                            |

Tests never need PostgreSQL, HugeGraph, Python services or network access.
They run fully in-process.

### Tooling

| Language   | Format    | Lint          | Test         |
| ---------- | --------- | ------------- | ------------ |
| Rust       | `rustfmt` | `clippy`      | `cargo test` |
| TypeScript | Prettier  | ESLint, `tsc` | Vitest       |
| Python     | Ruff      | Ruff          | pytest       |

CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) runs all of the
above on every pull request, and also validates and builds the Docker images.

## Domain model (M1)

`crates/mindgraph-domain` holds the core vocabulary and has no dependency on
Axum, SQLx, HugeGraph or any HTTP client:

```text
Workspace
  └─ GraphProject                 (draft | active | archived)
       ├─ Ontology  vN            immutable, versioned (v1, v2, …)
       │    ├─ EntityType         Customer { customer_id: string, … }
       │    └─ RelationshipType   Customer ──OWNS──> Account
       ├─ Entity                  typed properties, validated against an ontology
       └─ Relationship            source → target, validated against an ontology
```

- **Typed UUIDv7 identifiers:** `WorkspaceId`, `GraphId`, `EntityId`,
  `RelationshipId`, and so on. Nil and non-v7 UUIDs are rejected, so a
  relationship endpoint can never be "empty".
- **Properties** are JSON-compatible: string, integer, float, boolean,
  datetime (UTC, RFC 3339) and string array. Conversion from JSON is guided by
  the schema.
- **Validation** is deterministic and returns a `DomainError`. Examples:
  empty graph names, invalid type names, duplicate types, relationship types
  whose endpoints do not exist, and endpoint type mismatches.

## Configuration

Configuration uses environment variables (ADR-013). See
[`.env.example`](.env.example) for the full list.

| Variable                      | Used by   | Default (Compose)                 |
| ----------------------------- | --------- | --------------------------------- |
| `MINDGRAPH_CORE_BIND_ADDR`    | core      | `0.0.0.0:8080` in the container   |
| `MINDGRAPH_DATABASE_URL`      | core (M3) | built from `MINDGRAPH_POSTGRES_*` |
| `MINDGRAPH_HUGEGRAPH_URL`     | core (M2) | `http://hugegraph:8080`           |
| `MINDGRAPH_AI_URL`            | core (M5) | `http://ai:8000`                  |
| `MINDGRAPH_LLM_PROVIDER`      | ai (M5)   | empty (AI disabled)               |
| `MINDGRAPH_LLM_API_KEY`       | ai (M5)   | empty                             |
| `MINDGRAPH_TELEMETRY_ENABLED` | all       | `false`                           |

Telemetry is off by default. MindGraph never collects graph contents,
uploaded datasets, prompts or entity data. Next.js telemetry is disabled in
the Docker image and in CI.

## Roadmap

| Milestone | Scope                                                    | Status  |
| --------- | -------------------------------------------------------- | ------- |
| M0        | Repository bootstrap: monorepo, Compose, CI              | ✅ Done |
| M1        | Domain foundation: types, UUIDv7 IDs, validation, errors | ✅ Done |
| M2        | GraphStore trait, `MemoryGraphStore`, `HugeGraphStore`   | Next    |
| M3        | PostgreSQL migrations, project and metadata APIs         | Planned |
| M4        | CSV / JSON ingestion without AI                          | Planned |
| M5        | AI Graph Builder (ontology suggestion, review, generate) | Planned |
| M6        | Visual Graph Explorer (Sigma.js, Graphology)             | Planned |
| M7        | GraphRAG with evidence graphs                            | Planned |
| M8        | MCP server for Claude, Codex and Cursor                  | Planned |
| M9        | Basic entity resolution                                  | Planned |
| M10       | Launch hardening and demos                               | Planned |

## Contributing

1. Read the ADRs in [`docs/adr/`](docs/adr/). When they conflict with code
   conventions, the ADRs win.
2. Keep `mindgraph-domain` free of infrastructure dependencies. Graph access
   always goes through `GraphStore`, never directly to HugeGraph.
3. New infrastructure or services need an ADR first.
4. Run `make check` before opening a pull request.

## License

MindGraph Open is licensed under the [GNU AGPL-3.0](LICENSE).

ADR-015 also proposes Apache-2.0 for SDKs, MCP packages and examples. That
split is **pending final legal approval**. The `mindgraph-mcp` crate manifest
already declares Apache-2.0. Per-package license files and source headers
arrive with M10.
