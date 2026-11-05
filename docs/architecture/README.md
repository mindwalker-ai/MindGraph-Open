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

## AI Graph Builder (ADR-007, ADR-008, M5)

AI suggests. Rust validates. The user reviews. M4 executes. The AI layer
never writes graph data, ontologies or mappings, and there is no second
ingestion pipeline.

```text
Web (Graph Builder) ──/api/v1 proxy──► mindgraph-api
                                          │
                                          ▼
                          GraphBuilderService (mindgraph-core)
                          │  ai_suggest job: pinned ontology, Idempotency-Key
                          ▼
            GraphBuilderAi (port) ◄── HttpGraphBuilderAi (mindgraph-ai-client, reqwest)
                                          │ POST /internal/v1/graph-builder/suggest
                                          ▼
                       mindgraph-ai (FastAPI): prompt graph_builder_v2
                          LLMProvider ◄── OpenAICompatibleProvider (httpx2)
                                          │ POST {base_url}/chat/completions
                                          ▼
                              any OpenAI-compatible model server
        result: strict JSON ─► Pydantic ─► Rust validation ─► job result (a draft)

User reviews/edits the draft ─► POST /graphs/{id}/ontology ─► POST .../ingestions (per file) ─► M4 engine
```

### Boundaries

| Component | Owns | Does not |
| --- | --- | --- |
| `mindgraph-core` | `GraphBuilderAi` port, `GraphBuilderService`, authoritative validation, job | depend on reqwest, FastAPI or provider types |
| `mindgraph-ai-client` | HTTP to the AI service: one `reqwest::Client`, timeout, `x-request-id`, DTO decoding, 1 MiB response cap, error mapping | expose `reqwest` types |
| `services/ai` | provider interface, OpenAI-compatible adapter, prompt, Pydantic schemas | read files, call PostgreSQL or HugeGraph, persist anything |
| `apps/web` | review draft state, editing, the generate sequence | parse files, call the AI service directly |

`mindgraph-ai-client` is an infrastructure adapter crate, not a runtime
service (ADR-007 addendum). Core's dependency check still forbids HTTP
clients in `mindgraph-core`.

### What the model receives

One request models 1–10 analysed files together. Per file: its filename as
a label (made unique with ` (n)`) and the bounded M4 profile; plus the
pinned ontology:

- field names, inferred types, nullability and non-null counts;
- up to 5 sample values per field (each at most 200 characters);
- up to 10 sample rows (fewer if a profile would exceed its share of a
  320 KiB budget, at most 96 KiB per file; then fewer sample values);
- the active ontology's types (names, properties, endpoints), without ids.

Never the source file, other records, filesystem paths, graph contents or
credentials. Requests are user-triggered only (the AI Model step or the API).

### Prompt

`graph_builder_v2` (returned as `prompt_version` with every result; v1
modelled a single file). The system prompt tells the model to treat
everything inside `<context>` as untrusted data, to model all files as one
ontology (one type per real-world thing across files), to return exactly
one mapping per file, to put a cross-file relationship in the file whose
records hold both identifiers, to model entities conservatively (no entity
per field),
to prefer `*_id` identifiers typed as strings (leading zeros), to keep the
existing ontology intact, to use `match_only` for references to existing
types, to infer relationships only from evidence, and to output exactly one
JSON object of the documented shape: no reasoning, no markdown. Dataset JSON
is embedded with `<` and `>` escaped, so sample text cannot close the block.

### Validation (two layers)

1. **AI service (Pydantic)**: strict JSON only (no fences, no prose); unknown
   fields rejected; machine-name patterns; confidences in `0..=1`; at most
   25 entity types, 50 relationship types, 100 properties per type, 20
   warnings of 300 characters; 256 KiB of model output.
2. **Rust (authoritative)**: the same bounds; the ontology is built through
   the domain constructors on top of the pinned base version; the base must
   be preserved (no removed types or relationship types, no changed property
   types, requiredness or endpoints, new properties of existing types
   optional); every file needs exactly one mapping, and each must pass M4's
   `validate_mapping` against that file's fields; a `match_only` type must
   exist already or be created by one of the files; files must not only
   match what each other creates (a cycle). Rust sets each annotation's
   `existing` flag itself, links each mapping to its data source and orders
   the mappings safely (files that create a type before files that only
   match it).

If Rust rejects a suggestion, the concise errors (never stack traces, SQL,
paths, URLs or secrets) and the previous suggestion are sent back for at
most `MINDGRAPH_AI_MAX_REPAIR_ATTEMPTS` (default 1, at most 2) repairs. Then
the job fails with `AI_VALIDATION_FAILED`.

### Provider calls

- One provider kind, `openai_compatible` (hosted APIs, vLLM, Ollama's `/v1`,
  llama.cpp server, gateways). `Authorization: Bearer` only when a key is
  set; the key is never logged, returned or shown in `repr`.
- Per-attempt timeout `MINDGRAPH_LLM_TIMEOUT_SECONDS` (60). Retries (at most
  2, backoff 0.5 s, 1 s; `Retry-After` honoured up to 5 s) only for 429, 500,
  502, 503, 504 and dropped connections. 400/401/403/404 and timeouts are not
  retried.
- `response_format: json_object` is requested when `MINDGRAPH_LLM_JSON_MODE`
  is on, but correctness never depends on it.

### Jobs

`ai_suggest` reuses the M4 job table, executor code and API:

- **Executor**: M4's dispatcher became a generic `JobExecutor` with one
  handler per job kind. Analysis and ingestion share one instance
  (`MINDGRAPH_JOB_CONCURRENCY`); AI jobs run on their own instance with its
  own queue (64) and `MINDGRAPH_AI_JOB_CONCURRENCY` (2, at most 8), so slow
  model calls never delay dataset jobs. A result the job store refuses fails
  the job with `RESULT_NOT_RECORDED` instead of leaving it `running`.
- **Data sources**: `POST /graphs/{id}/graph-builder/suggestions` with
  `{"data_source_ids": [...]}` (1–10, distinct, all analysed); the
  per-data-source endpoint is the one-file shorthand. The first id is the
  job's `data_source_id`; all of them, in order, are stored in
  `ingestion_job_data_sources` (migration
  `20261104120000_multi_dataset_suggestions.sql`, foreign keys to the job
  and to data sources of the same graph) and returned as `data_source_ids`.
- **Idempotency**: required `Idempotency-Key`; the same key with the same
  data sources in the same order returns the original job without another
  provider call; anything else with that key is `409`.
- **Pinning**: the active ontology version is stored on the job; the worker
  loads exactly that version, and the result records it as
  `ontology_version_base`.
- **Result**: prompt version, provider, model, base version, repair count
  and the validated suggestion. Never the prompt, raw provider output or
  reasoning (migration `20261103140000_m5_graph_builder.sql`, result capped
  at 512 KiB; the worker fails larger suggestions with `AI_RESULT_TOO_LARGE`
  before storing them).
- **Repair**: the errors sent back with a repair request are bounded to the
  AI service schema (50 errors of at most 500 characters).
- **Restart**: `ai_suggest` jobs left `queued` or `running` fail with
  `PROCESS_RESTARTED`, like every other kind.
- **Preflight** (`503 AI_NOT_CONFIGURED` / `AI_UNAVAILABLE`, `409` if a data
  source is not analysed) runs before a job is created.
- **Earlier results**: suggestions stored before multi-file support (one
  `mapping`) still decode, as one mapping of the job's data source.

### Web UI

The screens follow the MindGraph Open design (Titillium Web, IBM Plex
Mono, the dark token palette, Lucide icons, a 220px sidebar or a 52px
rail). Destinations of later milestones (MCP, templates, the demo) are
shown disabled with the milestone in their tooltip.

- `/`: the welcome screen until a graph exists, then "Your Graphs" (cards
  drawn from each graph's ontology; an active graph opens in the Graph
  Explorer, a draft one on the upload step; graph-wide counts are not
  computed).
- `/graphs/new`: name and how to start (CSV, JSON, or an empty ontology).
- `/graphs/{id}/upload`: drop several files; each is uploaded and analysed
  at once (rows, columns, headers, preview). The ids live in `?sources=`,
  so a reload keeps them.
- `/graphs/{id}/builder?sources=…`: the AI Graph Builder. "Generate Graph
  Model" asks for one suggestion over all files (or "Build manually"). The
  review shows entity types (left), the ontology preview canvas with a YAML
  view (centre) and an inspector (right: definition, properties, source
  fields per file, keys, sample matches from the profiles, the cross-file
  join). The draft keeps the ontology and one mapping per file, linked by
  stable keys. Without `?sources=` it edits the ontology alone.

**Generate Graph** validates the draft locally and locks it, posts the
ontology with `expected_version` set to the draft's base version (skipped
when unchanged; the server rejects a stale base atomically with `409`), then
ingests each mapped file in a safe order, one idempotency key per file. A
file that cannot start keeps its key for **Retry ingestion**; files already
ingested are never ingested twice. The success screen links to the Graph
Explorer (M6, below).

### Known M5 limitations

- One provider kind (OpenAI-compatible); no streaming of model output.
- Suggestions are not stored as reusable drafts beyond the job result; the
  review draft lives in the browser tab.
- Relationship cardinality and match statistics are not computed yet (shown
  disabled); sample matches come from the bounded profiles only.
- There is no API to list a graph's data sources yet: the upload page
  restores files from `?sources=` only.
- Confidence values are the model's own estimates, not calibrated
  probabilities.
- `/ready` of core does not include the AI service (AI is optional); see
  `GET /api/v1/graph-builder/status`.

## Graph Explorer (ADR-012, M6)

A read-only investigation workspace at `/graphs/{id}/explore`. The browser
holds a bounded, growing subgraph; MindGraph Core and the `GraphStore` remain
the source of truth and own every traversal.

```text
Sigma.js (WebGL canvas)          renders, never fetches
   ▲ reducers: selection, hover, hidden, filters, path highlight
Graphology MultiDirectedGraph    the loaded subgraph (node = EntityId, edge = RelationshipId)
   ▲ mergeSubgraph (dedup, budget, placement)        investigation state (React reducer)
lib/api/client.ts                same-origin /api/v1 (Next.js proxy)
   ▲
mindgraph-api  explorer handlers  →  GraphExplorerService (core)  →  Arc<dyn GraphStore>
                                                                     HugeGraph / memory
```

### Backend

`GraphExplorerService` (`mindgraph-core`) wraps five bounded reads of the
shared `GraphStore` (the same `Arc` the ingestion uses). Each call first
checks that the graph exists in the workspace, so an unknown graph is
`404 GRAPH_NOT_FOUND`, and an entity of another graph is
`404 ENTITY_NOT_FOUND`. It logs the graph id, the operation, counts and
the duration, never property values.

| Endpoint | Store call | Bounds |
| -------- | ---------- | ------ |
| `POST /graphs/{id}/entities/search` | `search_entities` | exact match; at most 20 filters; `limit` 1–1,000 (default 100) |
| `GET /graphs/{id}/entities/{entity_id}` | `get_entity` | one entity |
| `GET /graphs/{id}/entities/{entity_id}/neighbors` | `get_neighbors` | `direction`, `relationship_type`, `limit` 1–1,000 relationships (default 100) |
| `POST /graphs/{id}/paths` | `find_path` | fewest hops, `max_depth` 1–6 (default 3), `direction` |
| `POST /graphs/{id}/subgraph` | `get_subgraph` | at most 1,000 entity ids |

Search filters are typed with the active ontology (`PropertyValue::from_json`):
with `entity_type`, by that type's schema; without it, only properties whose
type is the same in every entity type. Integers may be sent as decimal
strings. Responses render properties as plain JSON plus a `property_types`
map; datetimes are RFC 3339 and integers beyond 2^53 - 1 are decimal strings,
so JavaScript loses no digits. The HTTP handlers only parse and render; they
contain no traversal logic. There is no endpoint that returns a whole graph.

### Frontend

- `lib/graph/transform.ts`: payload validation (a malformed or cross-graph
  payload is rejected before anything changes), labels (a name-like
  property, else a domain identifier such as `customer_id`, else the short
  EntityId) and node / edge attributes. The API objects stay on the
  attributes for the details panels.
- `lib/graph/merge.ts`: `mergeSubgraph` adds entities and relationships by
  id, keeps existing positions, places new nodes around the expanded entity,
  sizes nodes by loaded degree (bounded) and curves parallel and opposite
  relationships apart. Self-loops are kept in the model and listed in the
  details panel, but Sigma 3 draws no loop.
- `lib/graph/state.ts`: the investigation state (selection, focus, hidden
  entities and relationships, entity and relationship type filters,
  highlights, the active path, the layout), kept apart from the graph data
  and never persisted. Highlighting takes any entities and relationships
  with a tone: blue for paths, orange for GraphRAG evidence (M7). Node and
  edge appearance are pure functions used as Sigma reducers.
- `lib/graph/layout.ts`: ForceAtlas2 with a bounded number of iterations
  (300 / 150 / 80 by size; never a background loop) and a circular layout
  grouped by entity type. After an expansion only the new nodes settle; the
  others stay fixed. A hierarchical layout is not provided.
- `components/explorer/`: `GraphExplorer` orchestrates; `GraphCanvas` (Sigma,
  loaded with `next/dynamic` and `ssr: false`) is the only WebGL code and
  disposes Sigma, its listeners and animations on unmount; `ExplorerSidebar`,
  `DetailsPanel`, `PathFinder` and `GraphToolbar` are plain React.

Loading is discovery first: the graph, its active ontology and one page of
25 entities, then the neighbourhood of one seed (the `?entity=` of the URL
or the first result). Everything after that is incremental: expand
(`neighbors`), connections among the loaded entities (`subgraph`) and paths
(`paths`, merged and highlighted, the camera fitted to them). A failed
request leaves the loaded graph as it was.

The view budget is 500 entities and 1,500 relationships in the browser.
Past it, a merge adds what fits and the explorer says how much was left out,
offering to unload hidden entities or reset the view. Hide, collapse (hides
the neighbours connected only to the entity; shared ones stay), type
filters and reset are client-side and never change the stored graph.

### Known M6 limitations

- Exact search only: no full-text, prefix, fuzzy or semantic search, and no
  graph-wide entity or relationship counts.
- Path search follows any relationship type; it cannot be limited to some
  types. When several shortest paths exist, one is returned.
- Self-loops are not drawn on the canvas (they are listed in the details
  panel). There is no hierarchical layout and no PNG export.
- Investigation views are not saved; a reload restores only the inspected
  entity (`?entity=`).
- The canvas needs WebGL; without it the explorer says so, and the panels
  still work.

## GraphRAG (ADR-010, M7)

Graph-first question answering with verifiable evidence. **Python
interprets and phrases; Rust validates, retrieves and verifies.** The model
never reaches the GraphStore, never writes or runs a query language, and
nothing from an answer is ever written to the graph. There is no vector
store in M7.

```text
POST /api/v1/graphs/{id}/ask ─► GraphRagService (mindgraph-core)
  1 graph in workspace · question ≤ 2000 chars · scope (1-5 selected ids, or bounded_graph)
    · history (follow-ups: ≤ 3 earlier questions, ≤ 10 entity ids each)
  2 active ontology · selected entities read through GraphExplorerService (404 if not in the graph)
    · entities of earlier questions read again (missing ones ignored, ≤ 10 kept as H1…)
  3 GraphRagAi::interpret ──HTTP──► AI service ─► RetrievalIntent (untrusted JSON)
  4 mindgraph-graphrag::validate_intent ─► RetrievalPlan     (422 INVALID_RETRIEVAL_PLAN /
                                                              RETRIEVAL_LIMIT_EXCEEDED)
  5 mindgraph-graphrag::retrieve ─► GraphStore (search, neighbours, path; bounded)
  6 EvidenceGraph ─► build_context (references only, ≤ 48 KiB)
  7 GraphRagAi::synthesize ──HTTP──► AI service ─► answer + cited references
  8 citations checked against the evidence ─► status, evidence, citations
```

### Crates

| Crate | Responsibility |
|---|---|
| `mindgraph-graphrag` | Capability crate over the `GraphStore` port (no HTTP, no provider types): `RetrievalIntent` (wire) → `validate_intent` → `RetrievalPlan`; `retrieve` (bounded GraphStore calls, deterministic id order); `EvidenceGraph`, `EvidenceBuilder` and references; `build_context`; `GraphRagLimits`. |
| `mindgraph-core` | `GraphRagAi` port (interpret, synthesize, status), `DisabledGraphRagAi`, `GraphRagService` (validation, deadline, concurrency, citation checks, statuses, logging). Reuses `GraphExplorerService` for workspace and graph isolation. |
| `mindgraph-ai-client` | `HttpGraphRagAi`: `POST /internal/v1/graphrag/{interpret,synthesize}`, strict DTOs, the M5 error mapping and readiness. |
| `mindgraph-api` | `POST /graphs/{id}/ask`, `GET /graphrag/status`, the error envelope; the request id is passed to the AI service. |
| `services/ai` | `mindgraph_ai.graph_rag`: Pydantic schemas, versioned prompts (`graphrag_interpret_v1`, `graphrag_synthesize_v1`), semantic checks, the shared OpenAI-compatible provider. |

### Operations

| Operation | Retrieval | Bounds |
|---|---|---|
| `entity_lookup` | `get_entity` by id, or `search_entities` by type and exact typed values | 100 matches (truncation reported) |
| `neighbors` | breadth-first `get_neighbors` from one entity; optional relationship type, neighbour type and direction | 1–3 hops, 100 relationships per call |
| `find_path` | `find_path` between two entities | 1–6 hops (M2 limit); no path = insufficient evidence, never proof of disconnection |
| `shared_neighbor` | from a selected member (its shared-type neighbours) or a shared entity, else search the shared type; then each candidate's members | 100 candidates; groups of ≥ 2 distinct members |
| `unsupported` | nothing | answered as insufficient evidence without a model call |

Anchors are a selected entity (`S1`…), an entity of an earlier question
(`H1`…, follow-ups), an entity id or an entity type with exact property
values typed by the ontology. An anchor that must name one
entity (a neighbourhood centre, path endpoints, a shared-neighbour start)
and matches several is reported as **ambiguous**: up to 10 candidates are
shown as evidence and none is chosen.

### Limits

`GraphRagLimits` (per question): 100 GraphStore operations, 100 candidates,
100 relationships per neighbourhood call, depth 3 (neighbours) and 6
(paths), 150 evidence entities, 300 evidence relationships, 48 KiB of model
context, property texts cut at 200 characters, 24 properties per item. None
of them raises a GraphStore bound (`sanitized` clamps them). Every question
runs under `MINDGRAPH_GRAPHRAG_TIMEOUT_SECONDS` (default 120: two model calls,
each bounded by the AI service's 60-second provider timeout, plus bounded
retrieval) including the wait for one of `MINDGRAPH_GRAPHRAG_CONCURRENCY`
(default 2) slots. No database transaction is held while the model works.
The API is synchronous: no job, no streaming, no stored conversation.

### Follow-up questions

The Ask panel keeps its conversation in the browser and sends the last
three questions with each follow-up, each with the entity ids it was about
(its scope, then the cited entities and the endpoints of cited
relationships and paths, at most 10). Nothing is stored on the server.

- `GraphRagService` checks the history (`INVALID_HISTORY` for more than 3
  turns, an empty or too long question, more than 10 ids or a malformed
  id), keeps at most 10 distinct earlier entities, newest question first,
  and reads each one again through the `GraphStore` for this graph. Missing
  ones (deleted, or ids of another graph) are ignored with a warning and
  never reach the model.
- The interpretation receives the earlier questions with the references of
  their entities (`S1` when selected, else `H1`, `H2`, …) and those entities'
  types and bounded properties. `validate_intent` resolves `H` references
  like `S` references, so the model never needs a MindGraph id.
- The synthesis receives the earlier questions only as context for what the
  question refers to. Evidence is retrieved afresh for every question; an
  earlier answer is never evidence, and earlier answers are not sent.
- "New conversation" in the panel forgets the conversation; leaving the
  page does too.

### Evidence and citations

- An `EvidenceGraph` holds only entities and relationships read from the
  GraphStore for the requested graph, deduplicated by MindGraph id (parallel
  relationships keep their own ids), with both endpoints of every
  relationship present and every path connected (`validate`). Additions are
  all-or-nothing within the budget, so truncation never leaves a dangling
  relationship.
- References are positional: `E1` is the first entity, `R1` the first
  relationship, `P1` the first path (shared-neighbour groups also produce
  member → shared → member paths).
- The context is compact JSON with sorted keys and references only (no
  MindGraph ids). When it exceeds 48 KiB it is reduced: shorter values and
  fewer properties first, then whole items, keeping paths and groups (and
  what they need) first. The reduced graph **is** the answer's evidence, so
  the model can never cite an object the response does not contain.
- After synthesis, Rust keeps only citations (listed or inline `[E1, R2]`)
  that resolve to the evidence; unknown or fabricated ones are removed with a
  warning. An answer that cites nothing is downgraded to
  `insufficient_evidence`. Citation checks prove that the cited objects were
  retrieved, not that the model interpreted them correctly; the answer is
  presented as model text over shown evidence.
- Status: `insufficient_evidence` when an anchor is missing or ambiguous, no
  path exists within the depth, the model says so, or nothing is cited;
  otherwise `partial` when retrieval or the context was truncated, else
  `answered`. With no evidence at all, the model is not asked and MindGraph
  answers itself (`answer_source: no_evidence`).

### Security and privacy

- Questions and property values are untrusted. Both prompts mark them as
  data, pass them as JSON with `<` and `>` escaped inside tagged blocks, and
  give the model no tools. Model output is parsed strictly, validated by
  Pydantic and semantic checks in the AI service, then by Rust.
- The model receives the question, the ontology, the selected entities'
  types and bounded properties, and the bounded evidence context. Never
  MindGraph ids, other graph data, credentials or the whole graph.
- Logs (Rust and Python) carry request ids, graph ids, operations, counts,
  sizes, truncation and durations; never questions, prompts, property values
  or model output. No telemetry is sent.

### Web UI

Ask MindGraph lives in the explorer's right panel (`AskPanel`, mockup 08);
the graph stays visible. `lib/graph/evidence.ts` verifies the response shape
(every relationship endpoint and path item inside the evidence, citations of
existing references) before anything reaches Graphology. The evidence is
merged with the M6 `mergeSubgraph` (same ids, view budget) and highlighted
with the `showEvidence` action (orange tone); clicking a reference highlights
that entity, relationship or path and moves the camera; *Clear Evidence
Highlight* resets the styling without removing data. The panel shows one
honest answering state (the API reports no intermediate phases), the
validated retrieval plan, limitations and warnings, and explains when no
model is configured. Selecting an entity scopes the question to it;
suggestions are derived from its type and the ontology's relationship types.

### Known M7 limitations

- Four operations only; no aggregates (counts, sums), filters on ranges or
  multi-step plans. Questions outside them are answered as unsupported.
- Exact matching only: a misspelt identifier finds nothing (no fuzzy,
  full-text or semantic retrieval).
- Path search cannot be restricted to relationship types (the filter is
  ignored, with a note), and one of several shortest paths is returned.
- Bounded discovery inspects the first 100 candidates by id order, not the
  most relevant ones.
- One question at a time in the panel; no conversation memory, history or
  streaming.
- Answers depend on the configured model's quality; citation checks do not
  verify the model's interpretation.

