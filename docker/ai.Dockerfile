# syntax=docker/dockerfile:1
# Builds the internal Python AI service. Build context: services/ai.

FROM python:3.13-slim-bookworm
COPY --from=ghcr.io/astral-sh/uv:0.13.5 /uv /uvx /bin/
WORKDIR /app
ENV UV_COMPILE_BYTECODE=1 \
    UV_LINK_MODE=copy \
    UV_PYTHON_DOWNLOADS=never \
    PYTHONUNBUFFERED=1
COPY pyproject.toml uv.lock .python-version ./
RUN --mount=type=cache,target=/root/.cache/uv \
    uv sync --locked --no-dev
COPY mindgraph_ai ./mindgraph_ai
RUN useradd --system --uid 10001 --no-create-home mindgraph
USER mindgraph
ENV PATH="/app/.venv/bin:$PATH"
EXPOSE 8000
CMD ["uvicorn", "mindgraph_ai.main:app", "--host", "0.0.0.0", "--port", "8000"]
