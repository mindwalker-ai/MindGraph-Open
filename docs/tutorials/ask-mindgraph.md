# How Ask MindGraph (GraphRAG) works

GraphRAG in MindGraph is graph-first (ADR-010): **Python interprets and
phrases, Rust retrieves and verifies.** No vector database is involved.

```text
question ─► AI service: interpret ─► retrieval intent (untrusted JSON, no query language)
         ─► mindgraph-graphrag: validate against the ontology and limits ─► retrieval plan
         ─► bounded GraphStore reads ─► evidence graph (real entities/relationships, E1/R1/P1)
         ─► AI service: synthesize from the bounded evidence context (≤ 48 KiB)
         ─► Rust keeps only citations of retrieved evidence ─► answer + evidence + status
```

- **Operations:** `entity_lookup` (by id, or type + exact property values),
  `neighbors` (1–3 hops), `find_path` (shortest path, 1–6 hops) and
  `shared_neighbor` (entities connected through a shared entity, such as
  customers using the same device), plus `unsupported`. There is no
  natural-language-to-Gremlin engine: the model never writes or runs a query.
- **Scopes:** `selected_entities` (1–5 entity ids, checked to belong to the
  graph) or `bounded_graph` discovery, which inspects at most 100 candidates
  and says so when it stops early. A bounded search is never presented as
  exhaustive.
- **Limits per question:** 2,000-character question, 100 GraphStore
  operations, 150 evidence entities, 300 evidence relationships, 48 KiB of
  model context, `MINDGRAPH_GRAPHRAG_TIMEOUT_SECONDS` (120) end to end and
  `MINDGRAPH_GRAPHRAG_CONCURRENCY` (2) questions at a time.
- **Evidence:** every entity and relationship in an answer was read from the
  GraphStore for that graph; relationships always come with both endpoints;
  nothing is synthesised to fill gaps; nothing from an answer is ever written
  to the graph. Citations to references that are not in the evidence are
  removed (with a warning). A valid citation proves the cited object was
  retrieved, not that the model's reading of it is right.
- **Statuses:** `answered`, `partial` (a limit stopped retrieval) and
  `insufficient_evidence` (no matching evidence, an ambiguous or missing
  entity, no path within the depth, or the model judged the evidence
  insufficient). Missing evidence is a normal answer, not an error.
- **Follow-up questions (session only):** the request may carry `history`,
  up to 3 earlier questions with up to 10 entity ids each. The browser keeps
  the conversation; MindGraph stores nothing. Core checks the ids against
  the graph again (others are ignored with a warning) and the model refers
  to them as `H1`, `H2`, … (at most 10). Earlier questions only explain what
  "they" or "what about C003?" mean: evidence is retrieved again for every
  question, and an earlier answer is never used as evidence.

**What is sent to the model:** the question; the ontology (type, property
and relationship names); the selected entities' types and up to 24
properties (texts cut at 200 characters); for a follow-up, the earlier
questions and the same bounded view of the entities they cited; and, for the answer, the evidence
context: the retrieved entities' and relationships' types and properties
under references such as `E1`, plus what the retrieval covered. Never
MindGraph ids, other graph data, credentials or the whole graph. Property
values are copied from your data, so use a local model for sensitive
graphs. Questions, prompts, property values and model output are never
logged; logs hold ids, counts, sizes and durations.

**Without a model** (`MINDGRAPH_LLM_PROVIDER` empty), `POST .../ask` returns
`503 AI_NOT_CONFIGURED`, `GET /api/v1/graphrag/status` reports
`not_configured`, the panel explains how to configure one, and every other
feature keeps working. Details:
[`docs/architecture/`](../architecture/README.md#graphrag-adr-010-m7) and
[`services/ai/README.md`](../../services/ai/README.md).
