# Development guide

How MindGraph-OSS is built and how to work on it. Start with
[CONTRIBUTING.md](../../CONTRIBUTING.md).

## Architecture

> **Rust owns the graph platform. Python owns AI-specific intelligence.**

```text
        Next.js + TypeScript + Tailwind + Sigma.js / Graphology (WebGL)
                                  │  REST / SSE
                                  ▼
                 mindgraph-core  (Rust · Axum · Tokio)  ── modular monolith
                  │                    │                     │
                  ▼                    ▼                     ▼
          Apache HugeGraph        PostgreSQL           mindgraph-ai
            graph data          app metadata      (Python · FastAPI, internal)

   AI agents ──stdio──► mindgraph-mcp ──REST (loopback)──► mindgraph-core
                        (read-only MCP tools, one graph; no database access)
```

The architecture is defined by the Architecture Decision Records in
[`docs/adr/`](../adr/), the highest technical source of truth. The crate
map and dependency rules are summarised in
[`docs/architecture/`](../architecture/README.md). Current and planned
endpoints are listed in [`docs/api/`](../api/README.md).

## Repository layout

```text
apps/web/                   Next.js frontend: home, create, upload, AI Graph Builder
crates/
  mindgraph-domain/         Infrastructure-free domain model
  mindgraph-core/           Application layer: use cases, MetadataStore port
  mindgraph-api/            HTTP API + the `mindgraph-core` server binary
  mindgraph-graph/          GraphStore trait, Memory + HugeGraph adapters
  mindgraph-metadata/       PostgreSQL metadata store, SQLx, migrations
  mindgraph-ingestion/      CSV / JSON ingestion
  mindgraph-ontology/       Ontology workflows (M3, M5)
  mindgraph-entity-resolution/  Deterministic duplicate detection, pure
  mindgraph-graphrag/       Graph-first retrieval
  mindgraph-mcp/            MCP server over Core's REST API (M8, Apache-2.0)
  mindgraph-cli/            `mindgraph` CLI
services/ai/                Python AI service (FastAPI, uv)
docker/                     Dockerfiles for web, core and ai
docs/                       ADRs, architecture, API, guides, release and legal docs
examples/                   Demo scenarios (fraud, cyber, knowledge-graph) with demo.json manifests
scripts/                    smoke.sh (running stack), smoke-stack.sh (isolated stack), check_openapi.py
assets/                     Logo and the README screenshots
migrations/                 PostgreSQL migrations, applied at startup
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
| `make hugegraph-test-up`     | Start a throwaway HugeGraph 1.7 on `127.0.0.1:18080`          |
| `make test-hugegraph`        | Run the `GraphStore` contract and the M4 ingestion flow against that live HugeGraph |
| `make postgres-test-up`      | Start a throwaway PostgreSQL 17 on `127.0.0.1:15432`          |
| `make test-postgres`         | Run the metadata store tests against that live PostgreSQL     |

The default test run never needs PostgreSQL, HugeGraph, Python services or
network access. Live HugeGraph and PostgreSQL tests are opt-in:

```bash
make hugegraph-test-up      # add CONTAINER=podman to use Podman
make test-hugegraph
make hugegraph-test-down

make postgres-test-up
make test-postgres
make postgres-test-down
```

Running `mindgraph-core` outside Docker (`make dev-core`) needs
`MINDGRAPH_DATABASE_URL` pointing at a PostgreSQL 17 database and
`MINDGRAPH_HUGEGRAPH_URL` pointing at HugeGraph 1.7. Migrations and the
HugeGraph schema are applied automatically at startup; no `sqlx` CLI is
needed. Uploaded files go to `./data` unless `MINDGRAPH_DATA_DIR` is set.

A 10,000-row performance sanity check (not part of CI) runs with
`cargo test --release -p mindgraph-api --test ingestion_perf -- --ignored --nocapture`.

### Beyond `make check`

| What | Command | Needs |
|---|---|---|
| Load a demo through the REST API | `make demo-fraud` (or `cargo run -p mindgraph-cli -- demo fraud --api http://127.0.0.1:8080`) | a running stack |
| Smoke-test a running stack | `make smoke` (`scripts/smoke.sh`, read-only, curl only) | stack + `make demo-fraud` |
| Isolated full-stack test | `make smoke-stack` (`scripts/smoke-stack.sh [--e2e] [--keep]`) | Docker or `COMPOSE="podman compose"` |
| Browser journeys (Playwright) | `cd apps/web && npx playwright install chromium && npm run test:e2e` | stack + fraud demo, `MINDGRAPH_E2E_WEB_URL` |
| Dependency audit | `make audit` (cargo-deny, npm audit, pip-audit) | `cargo install cargo-deny --locked` |
| OpenAPI vs router | `make check-openapi` | uv |
| MCP protocol tests on the release binary | `cargo test -p mindgraph-mcp --release --locked --test mcp_protocol` | none |

`scripts/smoke-stack.sh` builds and starts a throwaway copy of the stack
under the Compose project `mindgraph-smoke`, with its own volumes, ports
(23000 and 28180) and env file (`scripts/smoke.env`, which keeps your `.env`
and any model API key out of it). It loads the fraud demo, runs the smoke
test, re-runs the loader to prove it is idempotent, restarts with
`down` + `up` to prove the data persists, optionally runs the Playwright
journeys, and removes only its own containers and volumes. CI runs it with
`--e2e` on every push.

The Playwright journeys (`apps/web/e2e/`) run against the real stack with no
API mocks: first run (A), the fraud demo in the Graph Explorer (B), entity
resolution on a graph of their own (C) and the manual flow without AI (D).
Headless runners have no GPU, so Chromium renders WebGL in software
(SwiftShader flags in `playwright.config.ts`); Scenario B checks that the
canvas renders instead of its "WebGL is unavailable" fallback.

### Tooling

| Language   | Format    | Lint          | Test         |
| ---------- | --------- | ------------- | ------------ |
| Rust       | `rustfmt` | `clippy`      | `cargo test` |
| TypeScript | Prettier  | ESLint, `tsc` | Vitest       |
| Python     | Ruff      | Ruff          | pytest       |

CI ([`.github/workflows/ci.yml`](../../.github/workflows/ci.yml)) runs all of the
above on every pull request. It also runs the live HugeGraph contract suite
and the M4 ingestion flow against a `hugegraph/hugegraph:1.7.0` service
container, the metadata store suite against PostgreSQL 17, and validates and
builds the Docker images.

## Domain model

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

## Graph storage

`crates/mindgraph-graph` defines the `GraphStore` trait, the only way
application code reads or writes graph data (ADR-004):

- `MemoryGraphStore` is the in-memory reference implementation, used by
  tests and local development.
- `HugeGraphStore` is the Apache HugeGraph 1.7 adapter. It uses one generic
  physical schema (`mindgraph_entity` vertices, `mindgraph_relationship`
  edges) and stores domain properties in a lossless typed encoding.

Every operation is scoped to one `GraphId`: graphs never see or change each
other's data, and `clear_graph` clears one graph project, never the
database. Both backends pass the same contract test suite. Details,
the physical mapping and current limitations are in
[`docs/architecture/`](../architecture/README.md#graph-storage-adr-004-m2).

## Dataset ingestion

`crates/mindgraph-ingestion` is the deterministic dataset engine (ADR-008),
orchestrated by `IngestionService` in `mindgraph-core`:

- **Formats:** CSV with a header row, and JSON as a top-level array of flat
  objects. Files are streamed, never loaded whole into memory.
- **Analysis job:** record count and, per field, the inferred type, nullability,
  non-null count and up to 5 sample values (plus up to 20 sample rows).
  Inference is conservative: `00123` stays a string, JSON strings are never
  re-read as numbers, mixed integer/float widens to float, anything else
  mixed is a string.
- **Mapping:** several entities per record (by alias), `create_or_match` or
  `match_only` with exact identity fields, and relationships between
  aliases. Validated against the active ontology before a job is created;
  the job is pinned to that ontology version.
- **Ingestion job:** a preflight pass checks every record before the first
  write, then batched `GraphStore` writes with a job-scoped identity cache
  and batched identity lookups. Progress is persisted per batch and
  available over SSE. Jobs cut off by a restart are marked
  `PROCESS_RESTARTED`.

Uploaded files are stored by data source id under `MINDGRAPH_DATA_DIR`
(Compose: the `mindgraph-data` volume), never under the user's filename.
Graph writes are not transactional across batches. Details, bounds and known
limitations are in
[`docs/architecture/`](../architecture/README.md#dataset-ingestion-adr-008-m4).
