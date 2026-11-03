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
(one ontology + mapping suggestion per dataset) together with `GET /ready`.
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