# Architecture overview

A summary of how the code is organised. The [ADRs](../adr/) remain the
source of truth. This page only maps them onto the repository.

## Runtime processes (ADR-002)

| Process          | Source                          | Notes                             |
| ---------------- | ------------------------------- | --------------------------------- |
| `mindgraph-web`  | `apps/web`                      | Next.js, port 3000                |
| `mindgraph-core` | `crates/mindgraph-api` (binary) | Rust modular monolith, port 8080  |
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
