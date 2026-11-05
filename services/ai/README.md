# MindGraph AI Service

Python + FastAPI service that owns AI-specific intelligence (ADR-001,
ADR-007). It is an **internal coprocessor**: only MindGraph Core calls it,
over the Docker network. It is never published, and it never reads source
files or writes PostgreSQL, HugeGraph or anything else. It turns the bounded
profiles of one or more datasets into one validated **suggestion**; MindGraph Core validates it
again, the user reviews it, and the deterministic M4 engine executes it.

For GraphRAG (M7) it does two things only: it interprets a question into a
structured **retrieval intent** (never a query language), and it phrases an
**answer** from the bounded evidence MindGraph Core retrieved. It never
reaches graph storage; Core validates the intent, retrieves the evidence and
checks every citation.

## Endpoints

| Endpoint | Purpose |
| --- | --- |
| `GET /health` | Liveness. Healthy even when AI is not configured. |
| `GET /ready` | `200 {"status": "ready", "configured": true, "provider", "model"}` or `503 {"status": "not_configured", "configured": false}`. Never URLs or keys. |
| `POST /internal/v1/graph-builder/suggest` | One AI Graph Builder suggestion (M5). |
| `POST /internal/v1/graphrag/interpret` | Question + ontology + selected entities (+ earlier questions and their entities for a follow-up) → one retrieval intent (M7). |
| `POST /internal/v1/graphrag/synthesize` | Question + bounded evidence context (+ earlier questions for a follow-up) → a cited answer (M7). |

Errors use `{"error": {"code", "message"}}` with fixed, client-safe messages:
`AI_NOT_CONFIGURED` (503), `PROVIDER_TIMEOUT` (504), `PROVIDER_RATE_LIMITED`
(429), `PROVIDER_UNAVAILABLE` (503), `PROVIDER_REJECTED` (502),
`INVALID_PROVIDER_OUTPUT` (502), `INVALID_REQUEST` (422, never echoing the
input), `PAYLOAD_TOO_LARGE` (413, requests over 512 KiB).

## Layout

```text
mindgraph_ai/
├── main.py                  app factory (settings and provider are injectable)
├── config.py                MINDGRAPH_LLM_* settings; invalid settings disable AI
├── errors.py                error categories and HTTP mapping
├── schemas.py               Pydantic request/suggestion schemas (bounded, strict)
├── observability.py         request ids, key=value logs (no payloads)
├── providers/
│   ├── base.py              LLMProvider protocol, strict JSON parsing
│   └── openai_compatible.py POST {base_url}/chat/completions with bounded retries
├── graph_builder/
│   ├── prompts.py           prompt graph_builder_v2, untrusted-data handling
│   └── service.py           provider → strict JSON → Pydantic → response
└── graph_rag/
    ├── schemas.py           retrieval intent, interpret / synthesize request and response
    ├── prompts.py           graphrag_interpret_v1, graphrag_synthesize_v1
    ├── validation.py        intent vs ontology and limits; citations vs evidence
    └── service.py           provider → strict JSON → Pydantic → checks → response
```

## GraphRAG (M7)

**Interpret.** The model returns one intent: `entity_lookup`, `neighbors`,
`find_path`, `shared_neighbor` or `unsupported`, with anchors that name a
selected entity (`S1`), an entity id, or an entity type with exact property
values. Unknown fields (for example a `gremlin` or `sql` key), unknown
operations, types, relationship types or properties, references to
unselected entities, missing anchors and depths beyond the request's limits
are rejected as `INVALID_PROVIDER_OUTPUT`. MindGraph Core validates the
intent again before any graph read.

**Synthesize.** The model receives the evidence context (`entities`,
`relationships`, `paths`, `shared_groups` under references such as `E1`,
plus what retrieval covered) and returns `{answer, evidence_refs,
insufficient_evidence, warnings}`. Citations of references that are not in
the evidence are removed from the list and from the text, with a warning;
with no evidence at all, no model is called and the answer is marked
insufficient. The prompt forbids inventing objects, asks for citations,
requires acknowledging bounded retrieval and forbids verdicts such as
fraud or guilt.

**What the model receives:** the question, the ontology, the selected
entities' types and bounded properties (interpret), and the bounded evidence
context (synthesize). For a follow-up, also the earlier questions (at most
3): with the references (`H1`, …) and bounded properties of their entities
for the interpretation, and as plain context for the synthesis, which is
told they are not evidence. No MindGraph ids, earlier answers, other graph
data or credentials. Questions and property values are untrusted: they are
passed as escaped JSON inside `<context>`, `<question>`, `<conversation>`
and `<evidence>` blocks. Logs contain
counts, sizes, outcomes and durations only.

## Configuration

| Variable | Default | Meaning |
| --- | --- | --- |
| `MINDGRAPH_LLM_PROVIDER` | empty | `openai_compatible`, or empty to disable AI |
| `MINDGRAPH_LLM_BASE_URL` | — | e.g. `https://api.openai.com/v1`, `http://host.docker.internal:11434/v1` (Ollama) |
| `MINDGRAPH_LLM_MODEL` | — | model name |
| `MINDGRAPH_LLM_API_KEY` | empty | optional `Bearer` token; never logged or returned |
| `MINDGRAPH_LLM_TIMEOUT_SECONDS` | `60` | per attempt (1–600) |
| `MINDGRAPH_LLM_MAX_OUTPUT_TOKENS` | `4096` | 256–32768 |
| `MINDGRAPH_LLM_MAX_RETRIES` | `2` | retries for 429 / 5xx / dropped connections (0–2) |
| `MINDGRAPH_LLM_JSON_MODE` | `true` | request `response_format: json_object` |

Invalid settings (an unknown provider, a missing URL or model, a URL with
credentials) do not crash the service: AI is disabled, the reason is logged
once, and `/ready` reports `not_configured` with `reason:
invalid_configuration`.

## What the model receives

Only what MindGraph Core sends: 1–10 datasets, each with its filename as a
label and its profile (field names, inferred types, nullability, non-null
counts, up to 5 sample values per field, up to 10 sample rows), and the
active ontology (types without ids). The suggestion has one ontology and one
`mappings` entry per dataset (`{"dataset": <label>, "mapping": {...}}`). The prompt marks this as untrusted
data, escapes `<`/`>` inside it, and asks for one JSON object without
reasoning or markdown. The model has no tools. Output that is not exactly one
JSON object, or that fails the schema (unknown fields, invalid names,
confidences outside `0..=1`, more than 25 entity types, 50 relationship
types, 100 properties per type, 10 mappings or 20 warnings), is rejected.

Logs contain request ids, provider, model, prompt version, field and record
counts, durations, retry counts and outcomes, never prompts, samples,
provider responses or keys.

## Development

```bash
uv sync                        # create .venv with dev tools
uv run uvicorn mindgraph_ai.main:app --reload --port 8000
uv run ruff check .
uv run ruff format --check .
uv run pytest                  # fake provider and mock transport: no network, no key
```

### Optional live check against a real model

Not part of CI. With any OpenAI-compatible server (for example
`ollama serve` and a pulled model):

```bash
MINDGRAPH_LLM_PROVIDER=openai_compatible \
MINDGRAPH_LLM_BASE_URL=http://localhost:11434/v1 \
MINDGRAPH_LLM_MODEL=qwen3:8b \
uv run uvicorn mindgraph_ai.main:app --port 8000
curl -s localhost:8000/ready
```

Then request a suggestion through MindGraph Core (see the root README).
