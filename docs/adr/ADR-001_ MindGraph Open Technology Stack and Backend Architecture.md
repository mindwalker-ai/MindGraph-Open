# ADR-001: MindGraph Open Technology Stack and Backend Architecture

**Status:** Accepted  
**Date:** 2026-11-03  
**Decision Owner:** Mindwalker Engineering  
**Product:** MindGraph Open  
**Scope:** MindGraph Open v0.1 and forward-compatible foundation for MindGraph Enterprise

## 1. Context

MindGraph Open is an open-source Graph Intelligence platform designed to let developers turn structured data into explorable knowledge graphs, investigate relationships, run GraphRAG, and expose graph context to AI agents through MCP.

The initial release must optimize for:

- strong developer experience
- simple local deployment
- high-performance graph operations
- modularity
- maintainability
- open-source contributor friendliness
- future migration toward a native MindGraph graph engine
- compatibility with Python-centric AI ecosystems
- Product Hunt and GitHub launch readiness

The architecture must avoid premature microservices while keeping clear boundaries between graph infrastructure and AI-specific workloads.

---

## 2. Decision

MindGraph Open will use a **hybrid Rust + Python architecture**.

Rust will serve as the primary backend and platform runtime.

Python will be isolated to AI-specific workloads such as LLM orchestration, embeddings, ontology generation, and GraphRAG synthesis.

The initial stack will be:

| Layer | Technology |
|---|---|
| Frontend | Next.js + TypeScript |
| UI | Tailwind CSS + shadcn/ui |
| Graph Visualization | Sigma.js + Graphology |
| Core Backend | Rust |
| HTTP Framework | Axum |
| Async Runtime | Tokio |
| Middleware | Tower |
| Serialization | Serde |
| HTTP Client | Reqwest |
| Metadata Database | PostgreSQL |
| PostgreSQL Client | SQLx |
| Graph Backend | Apache HugeGraph |
| AI Service | Python + FastAPI |
| MCP Server | Rust + rmcp |
| Observability | tracing + OpenTelemetry |
| Deployment | Docker + Docker Compose |
| CI/CD | GitHub Actions |

---

# 3. High-Level Architecture

```text
                    MindGraph Open

┌───────────────────────────────────────────┐
│                  Web UI                   │
│                                           │
│ Next.js + TypeScript                      │
│ Tailwind + shadcn/ui                      │
│ Sigma.js + Graphology                     │
└────────────────────┬──────────────────────┘
                     │
                REST / SSE
                     │
                     ▼
┌───────────────────────────────────────────┐
│              MindGraph Core               │
│                   Rust                    │
│                                           │
│ Axum + Tokio                              │
│                                           │
│ Graph API                                 │
│ Dataset Ingestion                         │
│ Ontology                                  │
│ Entity Resolution                         │
│ Investigation                             │
│ Search                                    │
│ Graph Algorithms                          │
│ Project Management                        │
│ Graph Store Abstraction                   │
└───────────────┬──────────────┬────────────┘
                │              │
                ▼              ▼
        Apache HugeGraph   PostgreSQL
          Graph Data       App Metadata

                ▲
                │
┌───────────────┴───────────────────────────┐
│               AI Service                  │
│                  Python                   │
│                                           │
│ FastAPI                                   │
│ GraphRAG                                  │
│ LLM Integration                           │
│ Embeddings                                │
│ Entity Extraction                         │
│ Ontology Suggestion                       │
│ Reranking                                 │
└───────────────────────────────────────────┘


┌───────────────────────────────────────────┐
│             MindGraph MCP                 │
│                  Rust                     │
│                                           │
│ rmcp                                      │
│                                           │
│ search_entities                           │
│ get_entity                                │
│ get_neighbors                             │
│ find_path                                 │
│ query_graph                               │
│ get_subgraph                              │
└───────────────────────────────────────────┘
```

---

# 4. Backend Architectural Style

MindGraph Core will initially use a **modular monolith** architecture.

The system will not be decomposed into separate microservices for:

- ontology
- ingestion
- entity management
- graph queries
- search
- investigation
- authentication

These will remain internal Rust modules or crates.

The initial runtime should consist of approximately:

```text
mindgraph-web

mindgraph-core

mindgraph-ai

mindgraph-mcp
```

External infrastructure:

```text
HugeGraph

PostgreSQL
```

This minimizes operational complexity while preserving clear internal boundaries.

---

# 5. Rust Responsibilities

Rust is the source of truth for MindGraph platform behavior.

Rust will own:

```text
Graph CRUD

Graph traversal

Graph search

Shortest-path operations

Entity management

Relationship management

Ontology management

Dataset ingestion

CSV parsing

JSON parsing

Basic entity resolution

Project management

Graph export

REST API

MCP server

Job orchestration

Graph storage abstraction

Graph algorithm execution
```

Rust should not depend on Python for ordinary graph operations.

---

# 6. Python Responsibilities

Python will be limited to capabilities where the Python AI ecosystem provides significant advantages.

Python owns:

```text
LLM provider integration

Embeddings

LLM-based entity extraction

LLM-based relationship extraction

AI-assisted ontology generation

GraphRAG response synthesis

Reranking

Semantic similarity

Optional local model inference
```

Python is treated as an **AI coprocessor**, not as the main application backend.

---

# 7. AI Graph Builder Flow

The AI Graph Builder will follow this separation of responsibility:

```text
Dataset
   │
   ▼
Rust
   │
   ├ Parse files
   ├ Inspect schema
   ├ Validate columns
   └ Sample records
   │
   ▼
Python AI
   │
   ├ Suggest entities
   ├ Suggest relationships
   └ Suggest ontology
   │
   ▼
Rust
   │
   ├ Validate ontology
   ├ Apply mappings
   ├ Generate entities
   ├ Generate edges
   └ Persist graph
   │
   ▼
HugeGraph
```

All AI-generated structures must be validated by MindGraph Core before persistence.

---

# 8. Graph Storage Abstraction

MindGraph business logic must not directly depend on HugeGraph-specific APIs.

A graph abstraction layer will be introduced.

Conceptually:

```rust
trait GraphStore {
    create_node(...)
    create_edge(...)

    get_node(...)
    get_neighbors(...)

    find_path(...)
    query_graph(...)

    delete_node(...)
}
```

Initial implementations:

```text
GraphStore
   │
   ├ HugeGraphAdapter
   │
   └ MemoryGraphAdapter
```

Future:

```text
GraphStore
   │
   ├ HugeGraphAdapter
   │
   └ MindGraphDBAdapter
```

This allows MindGraph Open to migrate to a native MindGraph database without rewriting application-level functionality.

---

# 9. Graph Visualization

The main graph canvas will use:

**Sigma.js + Graphology**

instead of using a generic node-editor library.

Reasons:

- WebGL-based graph rendering
- suitable for large graph visualization
- graph-oriented interaction model
- efficient node and edge rendering
- compatible with graph algorithms and graph data structures
- appropriate for investigative exploration

React Flow may still be used in the future for workflow or pipeline interfaces, but not as the main graph visualization engine.

---

# 10. MCP Architecture

MCP will be implemented in Rust using the official Rust MCP ecosystem.

Initial tools:

```text
search_entities

get_entity

get_neighbors

find_path

query_graph

get_subgraph

search_related_entities
```

The MCP server should communicate with the same MindGraph Core/domain layer rather than implementing graph logic independently.

Target integrations include:

```text
Claude

Codex

Cursor

Custom MCP clients
```

MindGraph MCP should also be distributable independently in the future.

Example:

```bash
mindgraph mcp
```

or:

```bash
cargo install mindgraph-mcp
```

---

# 11. Repository Structure

MindGraph Open should use a monorepo.

Recommended structure:

```text
mindgraph/
│
├── apps/
│   └── web/
│
├── crates/
│   ├── mindgraph-core/
│   ├── mindgraph-domain/
│   ├── mindgraph-api/
│   ├── mindgraph-graph/
│   ├── mindgraph-ingestion/
│   ├── mindgraph-ontology/
│   ├── mindgraph-entity-resolution/
│   └── mindgraph-mcp/
│
├── services/
│   └── ai/
│       ├── extraction/
│       ├── graphrag/
│       ├── embeddings/
│       └── main.py
│
├── examples/
│   ├── fraud/
│   ├── cyber/
│   └── knowledge-graph/
│
├── docs/
│
├── docker/
│
├── Cargo.toml
│
├── docker-compose.yml
│
└── README.md
```

---

# 12. Local Development and Deployment

The primary installation path must remain:

```bash
git clone https://github.com/mindwalker-ai/mindgraph

cd mindgraph

docker compose up
```

Expected output:

```text
MindGraph is running.

http://localhost:3000
```

The standard local deployment should not require users to manually install:

- Rust
- Python
- PostgreSQL
- HugeGraph
- Node.js

Docker Compose should provide the complete default environment.

---

# 13. CLI

A native Rust CLI should eventually expose common developer workflows.

Target commands:

```bash
mindgraph start

mindgraph stop

mindgraph status

mindgraph import transactions.csv

mindgraph demo fraud

mindgraph demo cyber

mindgraph demo knowledge

mindgraph mcp
```

A simple CLI improves both local developer experience and open-source distribution.

---

# 14. Rationale

## Why Rust

Rust was selected for MindGraph Core because the platform contains substantial non-LLM workloads:

- graph traversal
- graph algorithms
- high-volume ingestion
- parsing
- path finding
- concurrent API workloads
- MCP execution
- future storage-engine development

Rust provides strong performance, memory safety, predictable runtime characteristics, and a suitable foundation for future graph infrastructure.

It also aligns strategically with the possibility of building a native MindGraph graph engine.

---

## Why not Go

Go remains a strong alternative and would reduce implementation complexity.

However, Rust was preferred because MindGraph is expected to evolve closer to:

```text
graph infrastructure

query execution

graph algorithms

storage

developer tooling
```

rather than remaining primarily a conventional business API.

Rust therefore provides greater strategic alignment with the long-term MindGraph architecture.

---

## Why not Python for the entire backend

Python would accelerate early AI development but would make the application layer overly dependent on the AI ecosystem.

Core graph functionality should remain usable independently of:

- LLM providers
- embedding models
- Python inference stacks

Separating the AI layer also improves:

- performance
- reliability
- architectural clarity
- future portability

---

# 15. Consequences

## Positive

- High-performance core backend
- clean separation between graph and AI workloads
- strong open-source developer positioning
- reusable Rust crates
- native MCP and CLI capabilities
- reduced dependence on Python runtime for core functionality
- future path toward MindGraph DB
- ability to replace HugeGraph through adapters
- simpler scaling path for graph-intensive workloads

## Negative

- Rust development velocity may be slower initially
- contributor pool may be smaller than TypeScript or Python
- cross-language integration adds complexity
- team requires stronger Rust engineering capability
- debugging AI flows may cross service boundaries
- build times may increase as the Rust workspace grows

These costs are accepted in exchange for long-term platform quality and infrastructure extensibility.

---

# 16. Explicit Non-Decisions

This ADR does not decide:

- the final MindGraph DB storage engine
- distributed consensus technology
- sharding strategy
- enterprise deployment topology
- enterprise event bus
- Kubernetes architecture
- final GraphRAG framework
- final LLM provider
- authentication implementation
- multi-tenancy strategy

These should be covered by separate ADRs when required.

---

# 17. Future Evolution

Expected architecture evolution:

```text
v0.1

Next.js
   ↓
Rust Core
   ├ HugeGraph
   ├ PostgreSQL
   └ Python AI


        ↓


Future

Next.js
   ↓
Rust MindGraph Platform
   │
   ├ MindGraph DB
   ├ Distributed Graph Engine
   ├ MCP
   ├ Enterprise Security
   ├ Enterprise DataHub
   └ Python / Model Runtime
```

The architecture chosen in this ADR must allow this evolution without requiring a complete application rewrite.

---

# 18. Decision Summary

MindGraph Open will use **Rust as its primary backend platform**, Python as an isolated AI service, Next.js for the frontend, Sigma.js for graph visualization, HugeGraph as the initial graph storage backend, and PostgreSQL for application metadata.

The core architecture will use a modular monolith with a GraphStore abstraction.

The primary principle is:

> **Rust owns the graph platform. Python owns AI-specific intelligence.**

This architecture provides a practical v0.1 foundation while preserving the option to evolve MindGraph into a native graph infrastructure platform in the future.