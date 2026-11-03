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
  candidates. Dataset ingestion therefore batches identity lookups with
  `EntitySearch::with_any_of` (one scan per type and batch, see M4 below).
- HugeGraph 1.7 repeats the last vertex of a page as the first vertex of
  the next page when paging a property-filtered query. The adapter keeps
  each vertex once, by id (fixed in M4; covered by the contract case
  `search_pages_through_large_types_exactly_once`).
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
  `(graph_id, idempotency_key)`), `saved_views`. M4 extends the first two
  with an append-only migration (see below); `saved_views` is still unused.
- Timestamps are stored with microsecond precision. The application
  truncates to microseconds when it creates records, so API responses before
  and after a round trip are identical.

The `MetadataStore` contract is enforced by a shared suite
(`crates/mindgraph-metadata/tests/contract`) that runs against the memory
store on every `cargo test` and against PostgreSQL 17 with
`make test-postgres` and in CI. Each PostgreSQL test runs in a fresh database.

## Dataset ingestion (ADR-008, M4)

The deterministic dataset engine: upload a CSV or flat JSON file, profile
it, define an ontology by hand, map fields to it, and ingest. No LLM and no
Python service is involved; M5's AI Graph Builder will suggest ontologies
and mappings and then call these same services.

```text
HTTP (mindgraph-api, transport only: multipart, headers, SSE)
  │
  ▼
OntologyService ── create_and_activate_ontology ──┐
IngestionService (mindgraph-core)                 │  MetadataStore (PostgreSQL):
  │ upload ─► SourceStore (LocalSourceStore)      ├─ data_sources, ingestion_jobs,
  │ jobs   ─► bounded queue ─► semaphore workers  │  ontology_versions
  │              │                                │
  │              ▼                                │
  │   mindgraph-ingestion: read ─► profile        │
  │                        mapping ─► validate ─► preflight ─► batched writes
  │                                                              │
  ▼                                                              ▼
SourceStore (<data dir>/sources/<id>/source)          GraphStore (HugeGraphStore)
```

Ownership stays with the ports: `MetadataStore` owns control-plane state,
`GraphStore` owns graph data, `SourceStore` owns file bytes. Handlers never
see a database pool, a HugeGraph client or a filesystem path.

### `mindgraph-ingestion`

| Module      | Contents                                                                 |
| ----------- | ------------------------------------------------------------------------ |
| `record`    | Streaming CSV (header row, strict column count) and JSON (top-level array of flat objects, read element by element) readers; `Limits` |
| `profile`   | `DatasetProfile` / `FieldProfile`, type inference and widening           |
| `mapping`   | The `IngestionMapping` contract, `validate_mapping`, value conversion    |
| `source`    | `SourceStore` / `SourceWriter`, `LocalSourceStore`, `MemorySourceStore`, `UploadDigest` (SHA-256) |
| `engine`    | `run_ingestion`: preflight pass, identity resolution, batched writes     |
| `error`     | `IngestionError` with stable codes; messages never contain values        |

It depends on `mindgraph-domain` and the `GraphStore` port of
`mindgraph-graph` only (built without the `hugegraph` feature, so no HTTP
client). `csv::Error`, `std::io::Error` and `serde_json::Error` are kept as
sources, never exposed.

### Type inference

| Input           | Rule                                                                         |
| --------------- | ---------------------------------------------------------------------------- |
| CSV blank       | null (never `""`, `0` or `false`)                                            |
| CSV text        | `true`/`false` (any case) → boolean; leading-zero number (`00123`) → **string**; `i64` → integer; finite decimal → float; RFC 3339 → datetime; else string |
| JSON values     | boolean, integer, float, string (RFC 3339 → datetime), array of strings → string_array; strings are never re-read as numbers; nested objects or non-string arrays are rejected |
| Widening        | null never changes the type; integer + float → float; any other mix → string; no value at all → string |

The profile keeps at most 20 sample rows and 5 distinct sample values per
field (each truncated to 200 characters), because the UI needs a preview.
This is the only place dataset values are stored in PostgreSQL.

### Mapping contract

```json
{"entities": [{"alias": "account", "entity_type": "Account", "mode": "create_or_match",
               "identity": [{"source_field": "account_id", "property": "account_id"}],
               "properties": [{"source_field": "account_id", "property": "account_id"}]},
              {"alias": "customer", "entity_type": "Customer", "mode": "match_only",
               "identity": [{"source_field": "customer_id", "property": "customer_id"}],
               "properties": []}],
 "relationships": [{"relationship_type": "OWNS", "source": "customer", "target": "account",
                    "properties": []}]}
```

Validation runs before a job exists: unique aliases, existing source
fields, entity and relationship types of the active ontology, existing
target properties, at least one identity field, identity properties also
mapped (for `create_or_match`), every required property mapped, no
properties on `match_only`, and relationship endpoints whose alias types
match the ontology. The ontology property type decides conversion; CSV
arrays are not split (string arrays only come from JSON).

### Jobs

- **States** `queued → running → completed | failed`, and `queued → failed`
  (restart recovery). Guarded `UPDATE … WHERE status = …` statements make
  other transitions a conflict; database constraints back them up
  (`failed` ⇔ error, `completed` ⇒ progress 1, ingest ⇔ pinned version).
- **Executor**: in-process Tokio. A bounded queue (256) feeds a dispatcher
  that runs at most `MINDGRAPH_JOB_CONCURRENCY` jobs at a time (semaphore).
  The queue slot is reserved before the job row is written, so a full queue
  stores nothing (`503`). A panicking job is recorded as failed.
- **Per-graph serialisation**: ingestion jobs of the same graph run one at a
  time, so two jobs cannot both miss an identity and create it twice.
- **Ontology pinning**: the active version is stored on the job at
  submission; the worker loads exactly that version.
- **Idempotency**: `Idempotency-Key` is unique per graph in PostgreSQL. The
  same key with the same data source and mapping returns the original job
  (also after a restart); with a different request it is `409`.
- **Progress** is persisted per batch (ingestion) or per 1,000 records
  (analysis, by bytes read), never per row, and never decreases. SSE polls
  the persisted state every 500 ms and emits only changes.
- **Restart recovery**: at startup, before serving, every `queued` or
  `running` job is marked `failed` with `PROCESS_RESTARTED`. Jobs are not
  resumed. This assumes one `mindgraph-core` process per database.

### Graph writes

1. **Preflight** streams the whole file and converts every record without
   writing. Missing required values, unconvertible values or malformed rows
   fail the job before the first graph mutation.
2. **Write** streams the file again in batches (`MINDGRAPH_INGESTION_BATCH_SIZE`,
   default 500). Uncached identities of a batch are resolved with one
   `search_entities` call per entity type and batch (chunks of up to 999
   identities, `EntitySearch::with_any_of`, exact matching by the
   GraphStore); a truncated result falls back to one exact search per
   identity. A type that
   was empty when the job started needs no lookups. Results go into a
   job-scoped identity cache, so a customer referenced by 1,000 rows is
   looked up once and created at most once.
3. Entities, then relationships, are written with `create_entities` /
   `create_relationships`. Identical relationships (type, endpoints,
   properties) are de-duplicated within a job; parallel relationships remain
   possible across jobs.
4. On success the graph moves `draft → active` (an archived graph is not
   reactivated), then the job completes.

Writes are **not transactional across batches or across PostgreSQL and
HugeGraph**. If HugeGraph fails midway, earlier batches remain, the job
fails with `GRAPH_UNAVAILABLE` / `GRAPH_STORE_ERROR`, and the message says
nothing was rolled back. The graph is never cleared to "undo" a job.

### Source storage

`LocalSourceStore` writes `<MINDGRAPH_DATA_DIR>/sources/<data_source_id>/source`.
Bytes stream into `.upload-<uuid>.tmp` in the same directory, are flushed and
fsynced, and are renamed on commit; an abandoned upload's temporary file is
removed. The user's filename is metadata only (validated, base name kept),
so `../` and collisions cannot affect paths. Docker Compose mounts the
`mindgraph-data` volume at `/data`. Failed analyses keep the file for
diagnosis.

### Bounds

| Resource              | Bound                                                        |
| --------------------- | ------------------------------------------------------------ |
| Upload size           | `MINDGRAPH_MAX_UPLOAD_BYTES` (50 MiB), enforced while streaming |
| Records per dataset   | `MINDGRAPH_MAX_INGESTION_RECORDS` (100,000)                   |
| Fields per record     | 500                                                          |
| Value size            | 64 KiB per value                                             |
| Samples               | 20 rows, 5 values per field, 200 characters per value        |
| Write batch           | `MINDGRAPH_INGESTION_BATCH_SIZE` (500, at most 5,000)        |
| Running / queued jobs | `MINDGRAPH_JOB_CONCURRENCY` (2, at most 16) / 256            |
| Job error messages    | 1,000 characters                                             |

### Security

Logs carry ids, kinds, sizes, record counts, codes and durations. They never
contain file contents, sample rows, identity values, graph properties, full
paths, database URLs or HugeGraph URLs (URLs with credentials are refused at
startup). Error messages name record numbers and fields, never values;
graph-store failures are reduced to their category.

### Performance sanity check

`crates/mindgraph-api/tests/ingestion_perf.rs` (ignored; run with
`cargo test --release -p mindgraph-api --test ingestion_perf -- --ignored --nocapture`)
ingests a generated 10,000-row `customers.csv` and a 10,000-row
`accounts.csv` whose every row matches a different existing customer.
Observed on a developer laptop (Apple Silicon, release build, HugeGraph 1.7
in Podman):

| Backend   | Analysis (10k rows) | customers.csv ingest | accounts.csv ingest (10k distinct lookups) |
| --------- | ------------------- | -------------------- | ------------------------------------------ |
| memory    | ~7–15 ms            | ~0.04 s              | ~1.2 s                                     |
| HugeGraph | ~6–15 ms            | ~0.53 s              | ~5.2 s                                     |

Before lookups were batched, 1,000 such rows took 119 s against HugeGraph
(one full type scan per row). Not an SLA.

### Known M4 limitations

- Local filesystem source storage only (no S3 / MinIO); CSV and flat JSON
  only (no nested objects, no CSV array splitting).
- In-process workers; jobs do not resume after a restart (they fail with
  `PROCESS_RESTARTED`); one `mindgraph-core` process per database.
- No public cancellation endpoint (`cancelled` exists in the schema only).
- Exact identity matching only: no fuzzy entity resolution (M9), no
  automatic ontology or mapping suggestions (M5), no GraphRAG (M7).
- No transaction across PostgreSQL and HugeGraph, nor across write batches.
- Identity lookups on HugeGraph still scan the whole type once per batch
  (properties are not indexed in M2's generic schema), so re-ingesting into
  a type with ~100,000 existing entities is slow; per-property indexes or a
  bulk identity index belong to a later milestone.
- `/ready` checks PostgreSQL only; HugeGraph is checked (and its schema
  created) at startup, which fails fast if it is unavailable.
- A new ontology version replaces the whole type set; existing graph data is
  not migrated or checked against it.

