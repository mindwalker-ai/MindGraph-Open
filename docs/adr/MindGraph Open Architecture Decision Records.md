# MindGraph Open Architecture Decision Records

**Product:** MindGraph Open  
**Target:** v0.1  
**Status:** Architecture baseline for implementation  
**Date:** 2026-11-03

## ADR Index

| ADR | Decision | Status |
|---|---|---|
| ADR-001 | Technology Stack and Backend Architecture | Accepted |
| ADR-002 | Monorepo and Modular Monolith Structure | Accepted |
| ADR-003 | Core Domain Model and Identifier Strategy | Accepted |
| ADR-004 | Graph Storage Abstraction and HugeGraph | Accepted |
| ADR-005 | PostgreSQL Metadata Model | Accepted |
| ADR-006 | HTTP API and Background Job Model | Accepted |
| ADR-007 | AI Service Boundary and Model Provider Abstraction | Accepted |
| ADR-008 | Dataset Ingestion and AI Graph Builder Pipeline | Accepted |
| ADR-009 | Entity Resolution v0.1 | Accepted |
| ADR-010 | GraphRAG Architecture | Accepted |
| ADR-011 | MCP Server Architecture | Accepted |
| ADR-012 | Frontend and Graph Explorer Architecture | Accepted |
| ADR-013 | Configuration, Deployment, Observability, and Security | Accepted |
| ADR-014 | Testing and Quality Gates | Accepted |
| ADR-015 | Open Source Licensing and Enterprise Boundary | Accepted |

---

# ADR-001: Technology Stack and Backend Architecture

**Status:** Accepted

## Decision

MindGraph Open uses:

```text
Frontend
Next.js + TypeScript
Tailwind CSS
shadcn/ui
Sigma.js
Graphology

Core Backend
Rust
Axum
Tokio
Tower
Serde
Reqwest
SQLx

AI
Python
FastAPI

Graph Storage
Apache HugeGraph

Metadata
PostgreSQL

MCP
Rust + rmcp

Deployment
Docker
Docker Compose

Observability
tracing
OpenTelemetry
```

Rust owns the graph platform.

Python owns AI-specific intelligence.

MindGraph Core will initially use a modular monolith.

---

# ADR-002: Monorepo and Modular Monolith Structure

**Status:** Accepted

## Context

MindGraph needs clear boundaries without creating unnecessary microservices.

The codebase also needs to be understandable for open-source contributors.

## Decision

Use one Git monorepo.

```text
mindgraph/
│
├── apps/
│   └── web/
│
├── crates/
│   ├── mindgraph-domain/
│   ├── mindgraph-core/
│   ├── mindgraph-api/
│   ├── mindgraph-graph/
│   ├── mindgraph-metadata/
│   ├── mindgraph-ingestion/
│   ├── mindgraph-ontology/
│   ├── mindgraph-entity-resolution/
│   ├── mindgraph-graphrag/
│   ├── mindgraph-mcp/
│   └── mindgraph-cli/
│
├── services/
│   └── ai/
│
├── examples/
│   ├── fraud/
│   ├── cyber/
│   └── knowledge-graph/
│
├── docs/
│   ├── adr/
│   ├── architecture/
│   └── api/
│
├── migrations/
│
├── docker/
│
├── Cargo.toml
├── docker-compose.yml
├── Makefile
└── README.md
```

## Dependency Direction

Dependencies must point inward.

```text
API
 │
 ▼
Core / Application
 │
 ▼
Domain

Infrastructure
Graph
PostgreSQL
AI
```

`mindgraph-domain` must not depend on:

- Axum
- HugeGraph
- PostgreSQL
- FastAPI
- HTTP clients

Domain objects must remain infrastructure-independent.

## Runtime Processes

v0.1 has three primary runtime processes:

```text
mindgraph-web
mindgraph-core
mindgraph-ai
```

`mindgraph-mcp` and `mindgraph-cli` are additional Rust binaries using shared crates.

## Refinement (M3, approved)

`crates/mindgraph-metadata/` was added during M3. It is an infrastructure
crate, not a runtime service:

```text
mindgraph-core      MetadataStore port and application use cases
mindgraph-metadata  PostgreSQL implementation (SQLx, persistence codecs,
                    embedded migrations)
mindgraph-api       HTTP transport, configuration, composition root
```

`mindgraph-metadata` depends on `mindgraph-core` (for the port) and
`mindgraph-domain`. Neither of those depends on SQLx or PostgreSQL.

## Refinement (M5, approved)

`crates/mindgraph-ai-client/` was added during M5. Like `mindgraph-metadata`
it is an infrastructure adapter crate, not a runtime service:

```text
mindgraph-core       GraphBuilderAi port, GraphBuilderService, validation
mindgraph-ai-client  HTTP adapter to the internal AI service (reqwest)
mindgraph-api        composition root: HttpGraphBuilderAi, or AI disabled
```

`mindgraph-ai-client` depends on `mindgraph-core` (for the port). The
application layer never depends on an HTTP client.

## Rejected

Microservice-per-domain architecture.

Reason:

Too much operational overhead for v0.1.

---

# ADR-003: Core Domain Model and Identifier Strategy

**Status:** Accepted

## Decision

The main hierarchy is:

```text
Workspace
   │
   └── GraphProject
          │
          ├── DataSource
          ├── Ontology
          ├── Entity
          ├── Relationship
          └── SavedView
```

v0.1 may expose only one local workspace.

The domain should still support workspace identifiers for future multi-tenancy.

## GraphProject

Represents one logical graph.

Example:

```text
Fraud Investigation
Company Knowledge Graph
Cyber Investigation
```

Fields:

```text
id
workspace_id
name
description
status
created_at
updated_at
```

## EntityType

Example:

```text
Customer
Account
Transaction
Device
Merchant
```

Fields:

```text
id
graph_id
name
display_name
description
properties_schema
```

## RelationshipType

Example:

```text
OWNS
USES
SENT
TRANSFERRED_TO
```

Fields:

```text
id
graph_id
name
source_entity_type
target_entity_type
properties_schema
```

## Entity

Conceptual representation:

```text
Entity {
    id
    graph_id
    entity_type
    properties
}
```

## Relationship

```text
Relationship {
    id
    graph_id
    relationship_type
    source_id
    target_id
    properties
}
```

## IDs

Use UUIDv7 for MindGraph-owned identifiers.

Reasons:

- globally unique
- sortable by creation time
- suitable for distributed evolution
- does not expose sequential internal IDs

External source identifiers remain properties.

Example:

```text
MindGraph ID:
019...

Source identifier:
CUST-10291
```

Do not use source identifiers as primary MindGraph IDs.

## Property Representation

Properties use JSON-compatible values.

Supported v0.1 primitive types:

```text
string
integer
float
boolean
datetime
string[]
```

---

# ADR-004: Graph Storage Abstraction and HugeGraph

**Status:** Accepted

## Decision

Graph-specific infrastructure must be hidden behind a Rust trait.

Conceptually:

```rust
trait GraphStore {
    create_entity(...)
    create_entities(...)

    create_relationship(...)
    create_relationships(...)

    get_entity(...)
    search_entities(...)

    get_neighbors(...)

    find_path(...)

    get_subgraph(...)

    delete_entity(...)

    clear_graph(...)
}
```

Actual interfaces may evolve during implementation.

## Initial Adapters

Implement:

```text
HugeGraphStore
MemoryGraphStore
```

`MemoryGraphStore` exists for:

- unit tests
- local domain tests
- development
- benchmarks

## HugeGraph Responsibilities

HugeGraph stores:

```text
entities
relationships
graph properties
graph topology
```

HugeGraph must not become the storage location for ordinary application metadata.

## Adapter Rule

Application code must never call HugeGraph directly.

Invalid:

```text
mindgraph-api
    ↓
HugeGraph REST API
```

Correct:

```text
mindgraph-api
    ↓
mindgraph-core
    ↓
GraphStore
    ↓
HugeGraphStore
```

## Future

A native implementation may later be introduced:

```text
MindGraphDBStore
```

without changing application logic.

---

# ADR-005: PostgreSQL Metadata Model

**Status:** Accepted

## Decision

PostgreSQL stores control-plane and application metadata.

Examples:

```text
workspaces
graph_projects
data_sources
ontology_versions
ingestion_jobs
ai_jobs
saved_views
settings
```

Graph topology remains in the GraphStore.

## Initial Tables

Minimum:

```text
workspaces

graph_projects

data_sources

ontology_versions

ingestion_jobs

saved_views
```

## Ontology Versioning

Ontology must be versioned.

Example:

```text
Graph
 ├ Ontology v1
 ├ Ontology v2
 └ Ontology v3
```

Each graph has:

```text
active_ontology_version
```

v0.1 does not require advanced migration between ontology versions.

## Database Access

Use SQLx.

Prefer explicit SQL over a heavy ORM.

Migrations live under:

```text
/migrations
```

Migrations are append-only after public release.

---

# ADR-006: HTTP API and Background Job Model

**Status:** Accepted

## API Style

Use versioned JSON REST APIs.

Base path:

```text
/api/v1
```

Examples:

```text
POST   /api/v1/graphs
GET    /api/v1/graphs
GET    /api/v1/graphs/:id

POST   /api/v1/graphs/:id/data-sources

GET    /api/v1/graphs/:id/entities
GET    /api/v1/graphs/:id/entities/:entity_id

GET    /api/v1/graphs/:id/entities/:entity_id/neighbors

POST   /api/v1/graphs/:id/paths

POST   /api/v1/graphs/:id/query
```

## API Specification

OpenAPI is generated or maintained as part of the backend.

API consistency is more important than automatically exposing every domain object.

## Error Format

All errors use a stable envelope.

```json
{
  "error": {
    "code": "ENTITY_NOT_FOUND",
    "message": "Entity was not found",
    "request_id": "..."
  }
}
```

## Jobs

Operations expected to take more than a few seconds run as jobs.

Examples:

```text
dataset ingestion
ontology generation
graph generation
embedding generation
bulk entity resolution
```

API:

```text
POST /...
       ↓
202 Accepted

{
  "job_id": "..."
}
```

Status:

```text
GET /api/v1/jobs/:id
```

States:

```text
queued
running
completed
failed
cancelled
```

## Progress Updates

Use SSE for v0.1.

Do not introduce WebSockets unless a concrete requirement needs bidirectional communication.

---

# ADR-007: AI Service Boundary and Model Provider Abstraction

**Status:** Accepted

## Decision

Python AI Service exposes internal APIs consumed only by MindGraph Core.

It is not the public application API.

Example internal endpoints:

```text
/internal/v1/ontology/suggest

/internal/v1/entities/extract

/internal/v1/relationships/extract

/internal/v1/embeddings

/internal/v1/graphrag/synthesize
```

## Model Provider Interface

AI components must not depend directly on one vendor.

Conceptual interface:

```text
LLMProvider

generate()
generate_structured()
embed()
```

Possible providers later:

```text
OpenAI-compatible
Anthropic
Gemini
Ollama
vLLM
BytePlus ModelArk
```

v0.1 only needs one or two implementations.

## Structured Output

Entity extraction and ontology generation must produce validated structured data.

Never parse arbitrary natural-language model output directly into the graph.

Use:

```text
LLM output
   ↓
Pydantic schema
   ↓
validation
   ↓
Rust validation
   ↓
persistence
```

## AI Failure Principle

Core graph functionality must continue to function when AI is disabled.

The user must still be able to:

```text
import data
define ontology manually
explore graph
run graph operations
use MCP graph tools
```

---

## Refinement (M5, approved)

The first internal endpoint is `POST /internal/v1/graph-builder/suggest`
(one ontology plus one mapping per dataset, for 1–10 datasets modelled
together; see the ADR-008 refinement) together with `GET /ready`.
The only provider implementation in v0.1 is a generic OpenAI-compatible
chat-completions adapter, which also covers local servers (Ollama, vLLM,
llama.cpp). The validation chain above is implemented as: strict JSON →
Pydantic schema (AI service) → explicit DTOs (`mindgraph-ai-client`) →
domain constructors, base-ontology preservation and M4 `validate_mapping`
(Rust, authoritative) → at most two repair attempts → a job result that the
user reviews before any ontology version or graph data is written (the
upload, its profile and the suggestion itself are stored as job state). The
reviewed ontology is applied with `expected_version` set to the suggestion's
base version, so a stale review cannot overwrite a newer ontology.

# ADR-008: Dataset Ingestion and AI Graph Builder Pipeline

**Status:** Accepted

## Supported Inputs for v0.1

```text
CSV
JSON
```

Do not implement database connectors in v0.1.

## Pipeline

```text
Upload
  ↓
Store temporary source
  ↓
Inspect schema
  ↓
Sample records
  ↓
Infer primitive field types
  ↓
Request AI ontology suggestion
  ↓
User reviews ontology
  ↓
Create ontology version
  ↓
Transform rows
  ↓
Create entities
  ↓
Create relationships
  ↓
Optional entity resolution
  ↓
Complete ingestion job
```

## AI Never Automatically Commits Ontology

AI Graph Builder suggestions remain drafts until the user confirms:

```text
Generate Graph
```

## File Processing

Rust owns:

```text
CSV parsing
JSON parsing
type detection
batching
validation
mapping
persistence
```

Python does not receive full large files by default.

Python receives:

```text
schema
column names
statistics
representative samples
```

This minimizes latency and token consumption.

## Batch Processing

Graph writes should be batched.

Do not insert graph entities one-by-one when the adapter supports bulk operations.

## Idempotency

Each ingestion run gets an ingestion ID.

Repeated requests with the same idempotency key must not silently duplicate graph records.

## Refinement (multi-dataset suggestions, approved)

Users upload several related files at once (customers, transactions,
devices), so one AI Graph Builder suggestion models 1–10 analysed data
sources together: one ontology for all of them and one M4 mapping per data
source. A cross-file relationship lives in the mapping of the file whose
records hold both identifiers (for example `customer_id` inside
`devices.csv`), with the other endpoint in `match_only` mode. Rust links
each mapping to its data source and returns the mappings in a safe
ingestion order (files that create a type before files that only match
it); a cycle is a validation error. Ingestion itself is unchanged: one M4
job per data source, applied in that order after the reviewed ontology is
saved. The job keeps its first data source in `data_source_id` and lists
all of them in `ingestion_job_data_sources`.

---

# ADR-009: Entity Resolution v0.1

**Status:** Accepted

## Scope

v0.1 provides deterministic and fuzzy candidate detection.

It is not an enterprise-grade identity resolution engine.

## Methods

Initial rules:

```text
exact source identifier

normalized email

normalized phone

normalized string

fuzzy name similarity
```

## Flow

```text
Entity A
   ↓
Candidate Generation
   ↓
Similarity Rules
   ↓
Candidate Match
   ↓
Confidence
   ↓
User Review
```

Example:

```text
Muhammad Ikhwan Nasution
M. Ikhwan Nasution

Potential duplicate
91%
```

## Merge Rule

Do not automatically merge uncertain fuzzy matches.

Automatic merge is allowed only for deterministic configured identifiers.

## Provenance

A merge operation must preserve:

```text
original entity IDs
source records
merge timestamp
merge strategy
```

This prepares the model for future enterprise explainability.

## Refinement (M9)

M9 implements entity resolution as **candidate detection, human review and
non-destructive logical identity**. It never rewrites the graph.

```text
GraphStore (bounded search) ─► mindgraph-entity-resolution ─► EntityResolutionService ─► EntityResolutionStore
   entities of one type          normalize · block · compare     (mindgraph-core)           (PostgreSQL / memory)
                                  · score (pure, no I/O)                                     scans · candidates ·
                                                                                             decisions · identity groups
```

- **Logical merge, never physical.** The GraphStore has no atomic
  "update entity + rewire relationships + delete duplicate" operation, and
  HugeGraph writes are not coordinated with PostgreSQL. An approved "same
  entity" decision therefore records metadata only:
  - a decision with its provenance;
  - a flat identity group in which every member maps directly to the
    canonical entity the reviewer chose, and the canonical entity maps to
    itself.

  No entity is deleted, no relationship is rewired and no property is
  overwritten. No `SAME_AS` relationship is created. The original records
  stay readable through the Graph Explorer, GraphRAG and MCP, which keep
  their M6–M8 semantics; no API substitutes canonical ids. Identity
  responses say `"physical_graph_merged": false`.
- **No automatic merge.** Every candidate, even an exact identifier match,
  waits for a person. Fuzzy name matches are weak evidence. This is a
  stricter subset of the original "automatic merge only for deterministic
  identifiers" rule.
- **Score, not confidence.** The example above ("91%") is replaced by a
  deterministic ranking score in `[0, 1)`: `1 − Π (1 − weight)` over
  agreeing signals, with fixed, documented weights. It orders review work
  and is never shown as a percentage or a probability.
- **Methods.** The five rules each name one explicit ontology property of
  the scanned type:
  - `exact_identifier`
  - `normalized_email`: domain lower-cased; the local part is preserved,
    and a case-only difference is a separate, weaker signal.
  - `normalized_phone`: libphonenumber E.164; national numbers need a
    default country.
  - `normalized_string`: NFKC, case folding, whitespace.
  - `fuzzy_name`: token-aware Jaro-Winkler with a 0.90 default threshold.

  Missing and invalid values never match. Entities are compared only within
  one entity type and one graph.
- **Bounded, synchronous scans.** A scan has these limits:
  - it reads at most 999 entities of one type through the existing
    GraphStore search (the first in id order, plus one to detect
    truncation);
  - it compares at most 20,000 blocked pairs and skips blocks of more than
    50 entities;
  - it keeps at most 2,000 candidates and has a 30 s deadline.

  Every limit it hits is reported, and `exhaustive` is true only when none
  was. This is not the "bulk entity resolution" job that ADR-006 lists. A
  graph-wide resolution job needs a dedicated resolution-run model and an
  indexed enumeration API, designed later, never a fabricated data-source
  job.
- **Persistence.** There is a separate `EntityResolutionStore` port
  (memory and PostgreSQL), not new `MetadataStore` methods. The append-only
  migration adds `entity_resolution_scans`, `entity_resolution_candidates`,
  `entity_resolution_decisions` and `entity_identity_aliases`:
  - Scans and decisions are immutable.
  - One pending candidate per pair and graph.
  - A deferred constraint trigger keeps groups flat and single-typed.
  - Writes per graph are serialised with a transaction-scoped advisory
    lock, so concurrent approvals cannot create conflicting canonical
    mappings.
  - No property value is stored: evidence names properties, rules and
    outcomes only.
- **Review rules.**
  - Repeating a decision is idempotent.
  - Changing it is a `409 DECISION_CONFLICT`.
  - Rescans skip pairs that are pending, rejected or already linked.
  - A rejected pair is reviewed again only with an explicit `reconsider`.
  - Linking entities that an earlier review marked different, or rejecting
    already-linked entities, is a `409 IDENTITY_CONFLICT`.
  - Accepted mappings cannot be undone in v0.1.
- **Provenance.** Each decision keeps:
  - the candidate's original entity ids and the canonical entity;
  - the canonical entities of the groups it consolidated, and the members
    whose mapping it wrote;
  - the decision, score, agreeing rules, scan id, timestamp and actor
    (`local-user`).

  M4 entities carry no source-record references, so source records are not
  part of M9 provenance and are never fabricated.
- **Ingestion is unchanged (ADR-008).** M4 exact identity matching
  (`create_or_match`, `match_only`) still decides which records become one
  entity at write time. M9 works on already-ingested entities and never
  feeds back into ingestion. The "optional entity resolution" step of the
  ADR-008 pipeline remains future work, after a separate design review, so
  there are never two conflicting identity authorities.

---

# ADR-010: GraphRAG Architecture

**Status:** Accepted

## Principle

MindGraph GraphRAG is graph-first, not vector-search-first.

GraphRAG must answer:

```text
How are these entities connected?
```

not only:

```text
Which documents are semantically similar?
```

## Query Pipeline

```text
User Question
    ↓
Intent Extraction
    ↓
Entity Identification
    ↓
Graph Retrieval
    ↓
Optional Semantic Retrieval
    ↓
Subgraph Construction
    ↓
Context Serialization
    ↓
LLM Synthesis
    ↓
Answer + Evidence
```

## Response

The frontend receives both:

```text
answer
```

and:

```text
evidence graph
```

Example:

```json
{
  "answer": "...",
  "entities": ["..."],
  "relationships": ["..."],
  "paths": ["..."]
}
```

The graph explorer highlights those entities and relationships.

## Hallucination Control

The generated answer should clearly distinguish between:

```text
facts supported by graph
model-generated interpretation
```

AI-generated relationships must never be persisted automatically from a GraphRAG answer.

## Vector Search

Vector storage is optional for v0.1.

If needed initially, PostgreSQL + pgvector is preferred over adding another standalone vector database.

## Refinement (M7)

The pipeline above is implemented without the optional semantic step (no
embeddings, no vector store):

```text
question ─► AI service /internal/v1/graphrag/interpret ─► RetrievalIntent (untrusted)
         ─► mindgraph-graphrag: validate_intent (ontology, ids, depths) ─► RetrievalPlan
         ─► retrieve over the GraphStore port (bounded) ─► EvidenceGraph
         ─► build_context (references E1/R1/P1, ≤ 48 KiB)
         ─► AI service /internal/v1/graphrag/synthesize ─► answer + cited references
         ─► GraphRagService keeps only citations of retrieved evidence
```

- **Crates.** `mindgraph-graphrag` is a capability crate over the
  `GraphStore` port (`mindgraph-graph` without the HugeGraph feature) and
  `mindgraph-domain`; it has no HTTP or provider types. `mindgraph-core`
  owns the `GraphRagAi` port and `GraphRagService` and reuses
  `GraphExplorerService` for workspace and graph isolation.
  `mindgraph-ai-client` implements the port over HTTP.
- **Intent extraction and entity identification** are one structured
  interpretation limited to four operations (`entity_lookup`, `neighbors`,
  `find_path`, `shared_neighbor`) plus `unsupported`. The model never writes
  a query language; Rust validates every type, property, id and depth.
- **Scopes.** Selected entities (1-5, verified to belong to the graph) or
  bounded discovery (at most 100 candidates). A bounded result is reported
  as `partial`, never as exhaustive.
- **Response.** `status` (`answered`, `partial`, `insufficient_evidence`),
  the answer, the evidence subgraph (entities, relationships, ordered paths
  with references), checked citations, warnings, the validated plan and
  retrieval counters. Missing evidence is a successful
  `insufficient_evidence` answer, not an error.
- **Hallucination control.** The evidence graph contains only objects read
  from the GraphStore (with both endpoints of every relationship); unknown or
  fabricated citations are removed; an answer that cites nothing is
  downgraded to `insufficient_evidence`. Citation checks prove that cited
  objects were retrieved, not that the model's interpretation is right; the
  UI shows the evidence next to the model text. Nothing from an answer is
  written to the graph.
- **Execution (refines ADR-006 for this endpoint).** `POST
  /api/v1/graphs/{id}/ask` is synchronous rather than a job: questions are
  interactive, there is no server-side conversation state, and the
  ingestion job schema is tied to data sources. It is bounded by a per-question deadline
  (`MINDGRAPH_GRAPHRAG_TIMEOUT_SECONDS`, default 120) and a concurrency
  limit (`MINDGRAPH_GRAPHRAG_CONCURRENCY`, default 2), and holds no database
  transaction while the model works. Streaming and persistent history are
  future work.
- **Follow-up questions (session only).** The browser keeps the
  conversation of the Ask panel and sends up to three earlier questions in
  `history`, each with the entity ids it was about (its scope and cited
  entities). The server stores nothing. `GraphRagService` checks the ids
  against the graph again (missing ones are ignored with a warning), keeps
  at most 10 of them (newest question first) and gives them to the model as
  references `H1`, `H2`, … next to the selected `S1`; `validate_intent`
  resolves both. Earlier questions reach the synthesis only as context for
  what the question refers to: evidence is retrieved afresh for every
  question and an earlier answer is never evidence. Persisting
  conversations across sessions needs its own decision (privacy,
  retention).
- **Internal endpoints (refines ADR-007).** `/internal/v1/graphrag/interpret`
  and `/internal/v1/graphrag/synthesize`, prompts `graphrag_interpret_v1` and
  `graphrag_synthesize_v1`, reusing the M5 OpenAI-compatible provider, retry
  policy and error categories.

---

# ADR-011: MCP Server Architecture

**Status:** Accepted

## Decision

Implement MCP in Rust.

MCP uses the same application/domain capabilities as the HTTP API.

Do not duplicate graph logic.

Architecture:

```text
Claude / Codex / Cursor
          ↓
      MindGraph MCP
          ↓
    Application Layer
          ↓
       GraphStore
```

## Initial Tools

```text
search_entities

get_entity

get_neighbors

find_path

get_subgraph

query_graph
```

## Tool Philosophy

Tools should be small and composable.

Avoid:

```text
do_everything_with_graph()
```

Prefer deterministic operations.

## Mutation

v0.1 MCP is read-heavy.

Graph mutation through MCP should either:

- be excluded initially
- or require explicit opt-in configuration

This reduces accidental destructive agent operations.

## Transport

Support stdio first.

Streamable HTTP may be added after local MCP experience is stable.

## Refinement (M8)

M8 implements the server as a standalone Rust binary that reaches the
application layer through MindGraph Core's public REST API, not in-process:

```text
Claude Code / Codex / Cursor ─stdio─► mindgraph-mcp ─HTTP (loopback)─► mindgraph-core
                                                                       ├─ GraphExplorerService
                                                                       └─ GraphRagService
```

- **Transport boundary.** `mindgraph-mcp` calls only Core's existing
  `/api/v1` endpoints. It never connects to PostgreSQL, HugeGraph or the AI
  service, needs none of their credentials, and links no other MindGraph
  crate (`make check-mcp-boundary` enforces this in CI). The application
  logic stays single: every tool is one call to an M6 explorer endpoint or
  the M7 `ask` endpoint. A developer can therefore run the server without
  database configuration, and it stays separately distributable.
- **Transport.** stdio only, through the official Rust SDK `rmcp`. stdout
  carries only MCP messages and diagnostics go to stderr. The process ends
  on EOF or a signal. There is no daemon, Compose service or remote
  endpoint, and Streamable HTTP is still deferred.
- **Scope.** One graph project per process (`MINDGRAPH_MCP_GRAPH_ID`). No
  tool accepts a graph id. The server also rejects any element Core returns
  for another graph. Several graphs mean several server entries.
- **Tools.**
  - The read-only graph tools: `get_graph_info`, `get_graph_schema`,
    `search_entities` (exact, typed matching only), `get_entity`,
    `get_neighbors`, `find_path` and `get_subgraph`.
  - Inputs come from Schemars JSON Schemas with unknown fields rejected.
    The server checks ids, directions, limits (search 1–100, neighbours
    1–100, subgraph 1–100 ids) and depths (1–6) before Core is called.
  - Results are structured content with output schemas, bounded by
    `MINDGRAPH_MCP_MAX_RESULT_BYTES` (default 256 KiB). An oversized result
    is an error, never a truncated graph.
- **`query_graph` is deferred.** The name suggests arbitrary query
  execution. No free-form query, Gremlin or Cypher tool is exposed, and the
  deterministic tools above cover the M8 needs. A generic query tool needs
  its own reviewed, bounded query contract first.
- **`ask_graph` is opt-in.** It is registered only with
  `MINDGRAPH_MCP_ENABLE_ASK=true` and annotated `openWorldHint: true`. It
  calls the M7 GraphRAG endpoint with its limits, so the question and
  retrieved evidence may reach the model provider configured in Core. It
  writes nothing.
- **Mutation.** No mutation tool exists: nothing creates, deletes, ingests
  or changes the ontology. Annotations (`readOnlyHint`) describe this
  behaviour but do not enforce it; what enforces it is that no tool maps to
  a write endpoint.
- **Security.**
  - The Core URL must be `http://` on a loopback host, without credentials
    or a path. Remote Cores are not supported, because Core has no
    authentication in v0.1.
  - The HTTP client follows no redirects and uses no proxy.
  - Requests have deadlines and bounded bodies, and carry an `x-request-id`.
  - Tool errors are machine-readable MCP tool errors, and Core's error
    codes pass through unchanged.
- **Licensing (ADR-015).** The binary depends only on permissive
  third-party crates, which keeps the proposed Apache-2.0 MCP package
  separate from the AGPL server. Two points remain for legal review: the
  AGPL dev-dependencies used by the end-to-end tests, and the AGPL CLI
  linking the MCP crate. See `docs/mcp/README.md`.

---

# ADR-012: Frontend and Graph Explorer Architecture

**Status:** Accepted

## Stack

```text
Next.js
TypeScript
Tailwind CSS
shadcn/ui
Sigma.js
Graphology
```

## Frontend Responsibility

Frontend owns:

```text
visualization state
selected nodes
selected edges
filters
layouts
graph camera
temporary investigation state
```

Backend remains source of truth for persistent graph data.

## State

Use local React state for local component state.

Use a lightweight global state library only where needed for application workspace state.

Do not introduce Redux by default.

## Graph Loading

Do not transfer the entire graph automatically.

Use subgraph retrieval.

Example:

```text
Search entity
   ↓
Load entity
   ↓
Load neighbors
   ↓
Expand
   ↓
Load additional neighborhood
```

This allows large underlying graphs while maintaining manageable browser state.

## Graph Interaction

Required:

```text
pan
zoom
select node
select edge
expand neighbors
collapse
filter
find path
fit graph
highlight evidence
```

## GraphRAG

The graph stays visible while the AI panel is open.

GraphRAG must not become a full-screen chatbot.

---

# ADR-013: Configuration, Deployment, Observability, and Security

**Status:** Accepted

## Local Deployment

Canonical local start:

```bash
docker compose up
```

Services:

```text
web
core
ai
postgres
hugegraph
```

## Configuration

Configuration uses environment variables.

Example:

```text
MINDGRAPH_DATABASE_URL

MINDGRAPH_HUGEGRAPH_URL

MINDGRAPH_AI_URL

MINDGRAPH_LLM_PROVIDER

MINDGRAPH_LLM_API_KEY

MINDGRAPH_TELEMETRY_ENABLED
```

Provide:

```text
.env.example
```

No secrets committed to Git.

## Default Security

v0.1 is local-first.

Default network exposure should be conservative.

Where practical, services not intended for direct external access should remain internal to Docker networking.

## Authentication

Open v0.1 does not require enterprise authentication.

Initial choices:

```text
single local user
or
optional simple local authentication
```

SSO, LDAP, AD, and advanced RBAC remain Enterprise concerns.

## Observability

Rust:

```text
tracing
structured logs
request IDs
OpenTelemetry hooks
```

Python:

```text
structured logging
request IDs
OpenTelemetry hooks
```

Every cross-service request should propagate a correlation/request ID.

## Telemetry

Open-source telemetry must be:

```text
documented
privacy-conscious
easy to disable
```

Do not collect graph contents, uploaded datasets, prompts, or customer entity data.

---

# ADR-014: Testing and Quality Gates

**Status:** Accepted

## Testing Layers

### Rust unit tests

Test:

```text
domain logic
ontology validation
entity resolution rules
mapping
graph algorithms
```

Use `MemoryGraphStore` where possible.

### Adapter integration tests

Test:

```text
HugeGraph adapter
PostgreSQL repositories
AI service client
```

### Python tests

Use pytest for:

```text
AI schemas
provider adapters
ontology prompts
GraphRAG serialization
```

AI tests should not require live external LLM calls by default.

### API tests

Test core public API contracts.

### Frontend tests

Focus on:

```text
critical components
graph interaction logic
upload flow
GraphRAG evidence highlighting
```

### End-to-end tests

Minimum critical flow:

```text
Create Graph
   ↓
Upload fraud CSV
   ↓
Analyze
   ↓
Confirm ontology
   ↓
Generate Graph
   ↓
Open Graph
   ↓
Expand Entity
   ↓
Find Path
```

## CI Required Checks

Pull requests must pass:

```text
cargo fmt --check

cargo clippy

cargo test

Python lint

Python tests

TypeScript typecheck

Frontend lint

Frontend tests
```

## Formatting

Rust:

```text
rustfmt
clippy
```

Python:

```text
ruff
```

TypeScript:

```text
eslint
prettier
```

---

# ADR-015: Open Source Licensing and Enterprise Boundary

**Status:** Accepted

## Product Principle

MindGraph Open must be genuinely useful.

Enterprise value comes from operational and organizational requirements, not artificially broken basic functionality.

## Proposed Licensing

Initial recommendation:

```text
MindGraph Open Server
AGPLv3

SDKs
Apache 2.0

MCP packages
Apache 2.0

Examples
Apache 2.0

Enterprise modules
Commercial license
```

Final legal approval is required before release.

## Open Features

```text
CSV
JSON
ontology
graph visualization
GraphRAG
MCP
REST API
basic entity resolution
local deployment
basic algorithms
```

## Enterprise Features

```text
enterprise database connectors
Kafka
CDC
pipeline scheduling
SSO
LDAP
AD
advanced RBAC
audit logs
advanced entity resolution
case management
collaboration
HA
clustering
replication
sharding
Kubernetes operator
backup / DR automation
enterprise support
```

## Repository Rule

Do not place enterprise-only proprietary implementation in the public repository unless explicitly approved.

Open code must not contain hidden checks intended solely to disable ordinary open-source capabilities.

---

# Implementation Milestones

With these ADRs accepted, implementation proceeds in this order.

## M0: Repository Bootstrap

Deliver:

```text
monorepo structure
Cargo workspace
Next.js app
Python AI service
Docker Compose
PostgreSQL
HugeGraph
CI
.env.example
basic README
```

Success:

```text
docker compose up
```

starts all required services.

---

## M1: Domain Foundation

Implement:

```text
Workspace
GraphProject
EntityType
RelationshipType
Entity
Relationship
Ontology
```

Add:

```text
UUIDv7 IDs
validation
domain errors
```

No UI complexity yet.

---

## M2: GraphStore

Implement:

```text
GraphStore trait

MemoryGraphStore

HugeGraphStore
```

Operations:

```text
create entities
create relationships
get entity
neighbors
search
find path
get subgraph
```

Create integration tests.

---

## M3: Project and Metadata APIs

Implement:

```text
PostgreSQL migrations

GraphProject repository

Ontology repository

DataSource repository

Job repository
```

Expose:

```text
POST /graphs
GET /graphs
GET /graphs/:id
```

---

## M4: Dataset Ingestion

Implement:

```text
CSV upload
JSON upload

schema inspection
sample extraction
primitive type inference
mapping
batch graph writes
job progress
```

At the end of M4:

```text
CSV
 ↓
Rust
 ↓
Graph
 ↓
HugeGraph
```

works without AI.

---

## M5: AI Graph Builder

Implement:

```text
Python AI service
LLM provider abstraction
ontology suggestion schema
Rust AI client
ontology review API
Generate Graph
```

Flow:

```text
Upload
 ↓
Analyze
 ↓
AI suggests ontology
 ↓
User confirms
 ↓
Generate Graph
```

---

## M6: Graph Explorer

Implement:

```text
Sigma.js
Graphology

graph canvas
entity search
entity details
neighbor expansion
filters
path highlighting
```

At the end of M6 the product should already be visually demoable.

---

## M7: GraphRAG

Implement:

```text
question analysis
entity identification
graph retrieval
subgraph construction
context serialization
LLM synthesis
evidence graph response
```

Frontend:

```text
Ask MindGraph
Show on Graph
```

---

## M8: MCP

Implement:

```text
search_entities
get_entity
get_neighbors
find_path
get_subgraph
query_graph
```

Provide Claude/Codex/Cursor documentation.

---

## M9: Entity Resolution

Implement:

```text
exact ID
email
phone
normalized names
fuzzy names
candidate review
merge provenance
```

---

## M10: Launch Hardening

Deliver:

```text
Fraud demo
Knowledge Graph demo
Cyber demo

README GIF
quickstart
documentation
sample datasets
screenshots
Product Hunt assets

performance tests
security review
license headers
contributor guide
```

---

# Definition of v0.1

MindGraph Open v0.1 is ready when a new developer can:

```text
git clone
      ↓
docker compose up
      ↓
open localhost
      ↓
upload CSV
      ↓
generate ontology
      ↓
generate graph
      ↓
explore relationships
      ↓
ask GraphRAG question
      ↓
highlight evidence
      ↓
connect an MCP client
```

without requiring manual installation of Rust, Python, Node.js, PostgreSQL, or HugeGraph.

---

# Architecture Rule for AI Coding Agents

When Claude Code or another coding agent encounters ambiguity:

1. ADRs are the highest technical source of truth.
2. PRD defines product behavior.
3. Existing code conventions take precedence over new abstractions.
4. Prefer the simplest implementation satisfying the current milestone.
5. Do not introduce new infrastructure without an ADR.
6. Do not create microservices unless explicitly approved.
7. Do not couple application logic directly to HugeGraph.
8. Do not move core graph logic into Python.
9. Do not add Enterprise-only scope to v0.1.
10. Do not optimize speculative future use cases at the expense of current simplicity.