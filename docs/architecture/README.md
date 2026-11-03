# Architecture overview

A summary of how the code is organised. The [ADRs](../adr/) remain the
source of truth. This page only maps them onto the repository.

## Runtime processes (ADR-002)

| Process          | Source                          | Notes                             |
| ---------------- | ------------------------------- | --------------------------------- |
| `mindgraph-web`  | `apps/web`                      | Next.js, port 3000                |
| `mindgraph-core` | `crates/mindgraph-api` (binary) | Rust modular monolith, port 8080; composition root |
| `mindgraph-ai`   | `services/ai`                   | FastAPI, internal only, port 8000 |
| `mindgraph-mcp`  | `crates/mindgraph-mcp`          | Additional binary (M8)            |
| `mindgraph`      | `crates/mindgraph-cli`          | Developer CLI                     |

The `mindgraph-core` *process* is built from the `mindgraph-api` crate,
because the binary must sit in the outermost layer. The `mindgraph-core`
*crate* is the application layer and must not depend on the API.

## Crate dependency direction

Dependencies always point inward:

```text
 mindgraph-api   mindgraph-mcp   mindgraph-cli        (entry points)
        \              |              /
         └──────► mindgraph-core ◄───┘               (application / use cases)
                        │
                        ▼
 mindgraph-graph, -ingestion, -ontology,              (capabilities)
 -entity-resolution, -graphrag
                        │
                        ▼
                mindgraph-domain                      (pure domain model)
```

`mindgraph-domain` may depend only on small, infrastructure-free libraries
(`serde`, `serde_json`, `uuid`, `chrono`, `thiserror`). CI enforces this with
`make check-domain-boundary`, which fails if `axum`, `sqlx`, `reqwest`,
`hyper`, `http`, `tokio`, `tower` or `tonic` show up in its dependency tree.

## Domain model (ADR-003)

| Type               | Key fields                                                                               | Invariants enforced                                          |
| ------------------ | ---------------------------------------------------------------------------------------- | ------------------------------------------------------------ |
| `Workspace`        | `id`, `name`, `created_at`, `updated_at`                                                 | name 1–120 chars, `updated_at ≥ created_at`                  |
| `GraphProject`     | `id`, `workspace_id`, `name`, `description`, `status`, timestamps                        | name 1–120 chars, description ≤ 2,000 chars                  |
| `EntityType`       | `id`, `graph_id`, `name`, `display_name`, `description`, `properties_schema`             | `name` is a valid `TypeName`                                 |
| `RelationshipType` | `id`, `graph_id`, `name`, `source_entity_type`, `target_entity_type`, `properties_schema` | endpoints must exist in the ontology                         |
| `Ontology`         | `graph_id`, `version`, entity types, relationship types                                  | unique names, same graph, endpoints exist, immutable         |
| `Entity`           | `id`, `graph_id`, `entity_type`, `properties`                                            | checked against an ontology with `Ontology::validate_entity` |
| `Relationship`     | `id`, `graph_id`, `relationship_type`, `source_id`, `target_id`, `properties`            | checked with `Ontology::validate_relationship`               |

Conventions:

- **Identifiers** are newtypes over UUIDv7. Nil and non-v7 UUIDs are rejected
  on parse and on deserialisation.
- **Machine names** (`TypeName`, `PropertyKey`) are 1–64 characters, start
  with an ASCII letter, and contain only ASCII letters, digits and `_`, so they
  are safe as graph-store labels. Human labels go in `display_name`.
- **Properties** support string, integer, float (finite), boolean, datetime
  (UTC) and string array. JSON input is decoded against a `PropertiesSchema`.
  Undeclared properties are rejected.
- **Source identifiers** such as `CUST-10291` are ordinary properties, never
  MindGraph IDs.
- **Ontology versions** start at `v1`. `Ontology::next_version` creates a new
  immutable version and leaves the previous one unchanged.

## Graph storage (ADR-004, M2)

`crates/mindgraph-graph` owns all graph persistence. Application code only
ever sees the `GraphStore` trait (`Send + Sync`, usable as
`Arc<dyn GraphStore>`):

```text
application (mindgraph-core, M3+)
        │  Arc<dyn GraphStore>
        ▼
   GraphStore ── MemoryGraphStore   reference implementation, tests, dev
              └─ HugeGraphStore     Apache HugeGraph 1.7 over REST
        │
        ▼
 mindgraph-domain (Entity, Relationship, ids, Properties)
```

| Operation | Semantics |
| --------- | --------- |
| `create_entity` / `create_entities` | Never overwrites. Duplicate → `Conflict`. A batch must be one graph (`InvalidInput`) and is validated as a whole before writing. |
| `create_relationship` / `create_relationships` | Both endpoints must exist in the relationship's graph (`EntityNotFound` otherwise, including endpoints that only exist in another graph). Duplicate id → `Conflict`. Self-loops allowed. |
| `get_entity` | `Ok(None)` when missing. |
| `search_entities` | Optional entity type + exact property filters, ordered by `EntityId`, limit 1–1,000. |
| `get_neighbors` | `Subgraph` with the entity, the matching relationships (direction, optional type, limit 1–1,000, smallest ids first) and their other endpoints. |
| `find_path` | Shortest path (fewest hops), `max_depth` 1–6, direction-aware. `None` if no path. Parallel relationships: the smallest `RelationshipId` is used. |
| `get_subgraph` | Requested entities (≤ 1,000, missing ids ignored) plus the relationships among them. |
| `delete_entity` | Also removes incident relationships. Returns whether the entity existed. |
| `clear_graph` | Removes one graph project's data only. Never wipes the database. |

Both backends are checked by one shared contract suite
(`crates/mindgraph-graph/tests/contract`): `MemoryGraphStore` on every
`cargo test`, and `HugeGraphStore` against a live server on demand.

### HugeGraph physical mapping

MindGraph uses one generic physical schema for every graph project. User
ontologies are **not** mirrored into HugeGraph schema objects.

| MindGraph | HugeGraph |
| --------- | --------- |
| `Entity` | vertex, label `mindgraph_entity`, id `"{graph_id}:{entity_id}"` (custom string), properties `mg_graph_id`, `mg_type`, `mg_properties` |
| `Relationship` | edge, label `mindgraph_relationship` (`frequency: MULTIPLE`, sort key `mg_id`), properties `mg_id`, `mg_graph_id`, `mg_type`, `mg_properties` |
| graph isolation, type lookup | secondary index `mindgraph_entity_by_graph_type` on `[mg_graph_id, mg_type]` |
| relationship id lookup | secondary index `mindgraph_relationship_by_graph_id` on `[mg_graph_id, mg_id]` |

- The vertex id carries the graph id, so identity is exactly
  `GraphId + EntityId`. HugeGraph cannot overwrite or connect vertices of
  different graph projects, even when two graphs use the same `EntityId`.
- `MULTIPLE` + sort key `mg_id` keeps parallel relationships apart. With the
  default `SINGLE` frequency, HugeGraph would merge them silently.
- `HugeGraphStore::ensure_schema` creates the schema idempotently
  (`check_exist: false`). An incompatible existing definition is reported as
  `Unsupported`.

### Property encoding

`mg_properties` stores domain properties as typed JSON, because plain JSON
cannot tell a datetime from a string without an ontology:

```json
{"version":1,"properties":{"joined_at":{"type":"datetime","value":"2026-11-03T08:30:00Z"},"risk_score":{"type":"float","value":0.91}}}
```

All six property types round-trip losslessly (floats bit-for-bit, datetimes
to the nanosecond). Unreadable stored data surfaces as
`GraphStoreError::CorruptData`, never a panic.

### HugeGraph 1.7.0 specifics

- REST paths are `/graphspaces/{graphspace}/graphs/{graph}/...` (defaults
  `DEFAULT` / `hugegraph`), not the older `/graphs/{graph}/...`.
- List and traverser responses are always gzip-compressed.
- Writing an element with an existing id overwrites it, so the adapter looks
  ids up before every write.
- `traversers/edgeexist` returns nothing when `label` is passed for an edge
  label with sort keys, so the adapter filters the label itself.

### Live integration tests

```bash
make hugegraph-test-up            # or: make hugegraph-test-up CONTAINER=podman
make test-hugegraph               # runs the contract suite against 127.0.0.1:18080
make hugegraph-test-down
```

CI runs the same suite in the `hugegraph-integration` job. Plain
`cargo test --workspace` never needs HugeGraph, Docker or network access.

### Known M2 limitations

- Exact property search scans the graph's (or type's) candidates by index
  and filters `mg_properties` in Rust. Scans are capped at 100,000
  candidates.
- Generic physical schema: no per-ontology HugeGraph labels or property
  indexes.
- No pagination abstraction, full-text, fuzzy or vector search.
- No cross-backend transactions. Duplicate detection is check-then-write,
  so concurrent writers of the same id can race. A batch that fails midway
  may have written earlier chunks.
- `clear_graph` deletes vertices one request at a time, and `get_subgraph`
  reads the edges of each requested entity separately. The REST API offers
  no batch delete or multi-vertex edge read.
- Path search does not filter by relationship type. When several different
  shortest paths exist, the backends may pick different ones.
- HTTPS to HugeGraph is not enabled. It is an internal backend.

## Metadata (ADR-005, M3)

PostgreSQL is the control plane: workspaces, graph projects and immutable
ontology versions. It never stores entities or relationships.

```text
HTTP ── mindgraph-api ── GraphProjectService (mindgraph-core)
                               │ Arc<dyn MetadataStore>   (port, in mindgraph-core)
                               ▼
                    PostgresMetadataStore (mindgraph-metadata, SQLx) ── PostgreSQL
                    MemoryMetadataStore   (mindgraph-core, tests/dev)
```

- **Startup** (`crates/mindgraph-api/src/main.rs`): read config, connect
  (fail fast), apply the embedded migrations, ensure the local workspace,
  serve. No retry loop; Docker's restart policy restarts the process.
- **Local workspace**: one row with `system_key = 'local'`, created on first
  start with `INSERT … ON CONFLICT DO NOTHING`, so it is stable across
  restarts and concurrent starts.
- **Graph creation** is one transaction: graph row plus ontology v1 row.
  `graph_projects.active_ontology_version` is `NOT NULL` with a deferred
  foreign key to `ontology_versions`, so a graph without its ontology cannot
  exist even at the database level.
- **Ontology versions** are immutable (a trigger rejects `UPDATE`) and
  unique per `(graph_id, version)`. The schema is stored as a versioned JSONB
  document. Decoding rebuilds domain objects through their validated
  constructors, so bad data is reported as `CorruptData`.
- **Baseline tables** for M4+: `data_sources`, `ingestion_jobs` (states
  `queued|running|completed|failed|cancelled`, unique
  `(graph_id, idempotency_key)`), `saved_views`. They have no application
  logic yet.
- Timestamps are stored with microsecond precision. The application
  truncates to microseconds when it creates records, so API responses before
  and after a round trip are identical.

The `MetadataStore` contract is enforced by a shared suite
(`crates/mindgraph-metadata/tests/contract`) that runs against the memory
store on every `cargo test` and against PostgreSQL 17 with
`make test-postgres` and in CI. Each PostgreSQL test runs in a fresh database.
