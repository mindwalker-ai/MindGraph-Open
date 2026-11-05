# API

The public API is versioned JSON REST (ADR-006). The authoritative contract is
[`openapi.yaml`](openapi.yaml) (OpenAPI 3.1).

| Method | Path | Purpose |
| ------ | ---- | ------- |
| `GET`  | `/health` | Liveness. Never touches the database. |
| `GET`  | `/ready` | Readiness. `200` when PostgreSQL is reachable, `503 NOT_READY` otherwise. |
| `POST` | `/api/v1/graphs` | Create a graph project (`201`, `Location` header) with an empty ontology v1. |
| `GET`  | `/api/v1/graphs?limit=50` | List graph projects, newest first (`limit` 1–100). |
| `GET`  | `/api/v1/graphs/{graph_id}` | Read a graph project. |
| `GET`  | `/api/v1/graphs/{graph_id}/ontology` | Read its active ontology (read-only in v0.1). |
| `POST` | `/api/v1/graphs/{graph_id}/entities/search` | Exact entity search by type and property values (M6). |
| `GET`  | `/api/v1/graphs/{graph_id}/entities/{entity_id}` | Read one entity (M6). |
| `GET`  | `/api/v1/graphs/{graph_id}/entities/{entity_id}/neighbors` | Bounded neighbourhood: `direction`, `relationship_type`, `limit` (M6). |
| `POST` | `/api/v1/graphs/{graph_id}/paths` | Shortest path within `max_depth` (1–6) hops (M6). |
| `POST` | `/api/v1/graphs/{graph_id}/subgraph` | Given entities and the relationships among them (M6). |
| `POST` | `/api/v1/graphs/{graph_id}/ask` | Ask MindGraph: a question answered from bounded graph evidence, with checked citations; follow-ups carry up to 3 earlier questions in `history` (M7, needs a model). |
| `GET`  | `/api/v1/graphrag/status` | Whether a model is configured for Ask MindGraph (`ready`, `not_configured`, `unavailable`). |

Dataset (M4) and AI Graph Builder (M5) endpoints are described in
[`openapi.yaml`](openapi.yaml).

## Request ids

Every response carries `x-request-id`. Send your own (1–128 visible ASCII
characters) to correlate logs, or let the server generate a UUIDv7.

## Errors

Every error, including unknown routes and wrong methods, uses one envelope:

```json
{ "error": { "code": "GRAPH_NOT_FOUND", "message": "Graph was not found", "request_id": "..." } }
```

| Status | Codes |
| ------ | ----- |
| 400 | `INVALID_JSON`, `INVALID_QUERY`, `INVALID_REQUEST`, `INVALID_GRAPH_ID`, `INVALID_ENTITY_ID`, `INVALID_QUESTION`, `INVALID_SCOPE`, `INVALID_HISTORY` |
| 404 | `GRAPH_NOT_FOUND`, `ENTITY_NOT_FOUND`, `ROUTE_NOT_FOUND` |
| 405 | `METHOD_NOT_ALLOWED` |
| 409 | `CONFLICT` |
| 415 | `UNSUPPORTED_MEDIA_TYPE` |
| 422 | `VALIDATION_FAILED` (a domain rule, e.g. an empty graph name), `INVALID_RETRIEVAL_PLAN`, `RETRIEVAL_LIMIT_EXCEEDED` (the model's GraphRAG plan does not fit the graph or the limits) |
| 500 | `INTERNAL_ERROR` (details are logged, never returned) |
| 502 | `AI_INVALID_RESPONSE` (the model did not return a valid answer) |
| 503 | `SERVICE_UNAVAILABLE`, `NOT_READY`, `AI_NOT_CONFIGURED`, `AI_UNAVAILABLE` |
| 504 | `AI_TIMEOUT` (the provider or the whole question exceeded its deadline) |

A GraphRAG question without matching evidence is not an error: it is a
`200` with `"status": "insufficient_evidence"`.

The AI service is internal and never part of the public API (ADR-007).
