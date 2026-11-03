# MindGraph AI Service

Python + FastAPI service that owns AI-specific intelligence (ADR-001,
ADR-007): LLM provider integration, embeddings, ontology suggestion, entity
extraction and GraphRAG synthesis.

It is an **internal coprocessor**, called only by MindGraph Core. It is not
part of the public API and is not published outside the Docker network. Core
graph features keep working when this service is disabled.

Status: M0 skeleton exposing only `GET /health`. The `/internal/v1/...`
endpoints arrive with M5 (AI Graph Builder) and M7 (GraphRAG).

```bash
uv sync                        # create .venv with dev tools
uv run uvicorn mindgraph_ai.main:app --reload --port 8000
uv run ruff check .
uv run ruff format --check .
uv run pytest
```
