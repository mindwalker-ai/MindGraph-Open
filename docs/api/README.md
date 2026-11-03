# API

## Available now (M0)

| Service          | Endpoint      | Response                                                        |
| ---------------- | ------------- | --------------------------------------------------------------- |
| `mindgraph-core` | `GET /health` | `{"status":"ok","service":"mindgraph-core","version":"0.1.0"}` |
| `mindgraph-ai`   | `GET /health` | `{"status":"ok","service":"mindgraph-ai","version":"0.1.0"}`   |

The AI service is internal. It is reachable only from inside the Docker
network and is never part of the public API (ADR-007).

## Planned (ADR-006)

The public API is versioned JSON REST under `/api/v1`. It arrives in M3
(projects and metadata) and grows with each milestone:

```text
POST /api/v1/graphs
GET  /api/v1/graphs
GET  /api/v1/graphs/:id
POST /api/v1/graphs/:id/data-sources
GET  /api/v1/graphs/:id/entities
GET  /api/v1/graphs/:id/entities/:entity_id
GET  /api/v1/graphs/:id/entities/:entity_id/neighbors
POST /api/v1/graphs/:id/paths
POST /api/v1/graphs/:id/query
GET  /api/v1/jobs/:id
```

All errors will use one stable envelope:

```json
{ "error": { "code": "ENTITY_NOT_FOUND", "message": "Entity was not found", "request_id": "..." } }
```

An OpenAPI specification will be maintained alongside the backend from M3.
