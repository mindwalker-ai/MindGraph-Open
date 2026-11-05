"""FastAPI application for the MindGraph AI service.

The AI service is an internal coprocessor consumed only by MindGraph Core
(ADR-007). It is never published outside the Docker network and never
writes PostgreSQL, HugeGraph or files: it turns a bounded dataset profile
into a validated *suggestion*, a question into a validated retrieval
*intent*, and bounded graph evidence into a cited *answer*. It never reaches
graph storage; MindGraph Core retrieves the evidence and checks the results.

Endpoints:

- ``GET /health``: liveness; healthy even when AI is not configured.
- ``GET /ready``: ``ready`` or ``not_configured`` (no URLs, no keys).
- ``POST /internal/v1/graph-builder/suggest``: one Graph Builder suggestion.
- ``POST /internal/v1/graphrag/interpret``: a question → one retrieval intent.
- ``POST /internal/v1/graphrag/synthesize``: question + evidence → a cited answer.
"""

from __future__ import annotations

import logging
from collections.abc import AsyncIterator, Awaitable, Callable
from contextlib import asynccontextmanager

from fastapi import FastAPI, Request, Response
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse

from mindgraph_ai import __version__
from mindgraph_ai.config import OPENAI_COMPATIBLE, Settings
from mindgraph_ai.errors import AIServiceError
from mindgraph_ai.graph_builder import GraphBuilderService
from mindgraph_ai.graph_rag import GraphRagService
from mindgraph_ai.graph_rag.schemas import (
    InterpretRequest,
    InterpretResponse,
    SynthesizeRequest,
    SynthesizeResponse,
)
from mindgraph_ai.observability import accept_request_id, configure_logging, request_id_var
from mindgraph_ai.providers import LLMProvider, OpenAICompatibleProvider
from mindgraph_ai.schemas import Health, Readiness, SuggestRequest, SuggestResponse

SERVICE_NAME = "mindgraph-ai"
#: Largest accepted request body (a bounded profile is far smaller).
MAX_REQUEST_BYTES = 512 * 1024

logger = logging.getLogger("mindgraph_ai")


def build_provider(settings: Settings) -> LLMProvider | None:
    """The configured provider, or ``None`` when AI is disabled."""
    if not settings.configured:
        return None
    if settings.provider == OPENAI_COMPATIBLE and settings.base_url and settings.model:
        return OpenAICompatibleProvider(
            base_url=settings.base_url,
            model=settings.model,
            api_key=settings.api_key,
            timeout_seconds=settings.timeout_seconds,
            max_retries=settings.max_retries,
            json_mode=settings.json_mode,
        )
    return None


def _error(status_code: int, code: str, message: str) -> JSONResponse:
    return JSONResponse(
        status_code=status_code, content={"error": {"code": code, "message": message}}
    )


def create_app(
    settings: Settings | None = None,
    provider: LLMProvider | None = None,
) -> FastAPI:
    """Builds the application. Tests inject ``settings`` and a fake ``provider``."""
    configure_logging()
    settings = settings or Settings.from_env()
    provider = provider if provider is not None else build_provider(settings)
    graph_builder = GraphBuilderService(provider, max_output_tokens=settings.max_output_tokens)
    graph_rag = GraphRagService(provider, max_output_tokens=settings.max_output_tokens)
    if provider is None:
        logger.info("AI Graph Builder not configured; manual graph building stays available")
    else:
        logger.info("AI Graph Builder ready provider=%s model=%s", provider.name, provider.model)

    @asynccontextmanager
    async def lifespan(_: FastAPI) -> AsyncIterator[None]:
        yield
        if provider is not None:
            await provider.aclose()

    app = FastAPI(title="MindGraph AI (internal)", version=__version__, lifespan=lifespan)

    @app.middleware("http")
    async def request_context(
        request: Request, call_next: Callable[[Request], Awaitable[Response]]
    ) -> Response:
        request_id = accept_request_id(request.headers.get("x-request-id"))
        token = request_id_var.set(request_id)
        try:
            length = request.headers.get("content-length")
            if length is not None and length.isdigit() and int(length) > MAX_REQUEST_BYTES:
                response: Response = _error(413, "PAYLOAD_TOO_LARGE", "the request is too large")
            else:
                response = await call_next(request)
            response.headers["x-request-id"] = request_id
            return response
        finally:
            request_id_var.reset(token)

    @app.exception_handler(AIServiceError)
    async def ai_error(_: Request, error: AIServiceError) -> JSONResponse:
        return _error(error.status_code, error.code, error.message)

    @app.exception_handler(RequestValidationError)
    async def invalid_request(_: Request, error: RequestValidationError) -> JSONResponse:
        # Never echo the input: it contains dataset samples.
        logger.warning("request rejected by the schema errors=%d", len(error.errors()))
        return _error(422, "INVALID_REQUEST", "the request does not match the schema")

    @app.get("/health")
    async def health() -> Health:
        return Health(status="ok", service=SERVICE_NAME, version=__version__)

    @app.get("/ready", response_model_exclude_none=True)
    async def ready(response: Response) -> Readiness:
        if provider is None:
            response.status_code = 503
            return Readiness(
                status="not_configured",
                configured=False,
                reason="invalid_configuration" if settings.config_error else None,
            )
        return Readiness(
            status="ready", configured=True, provider=provider.name, model=provider.model
        )

    @app.post("/internal/v1/graph-builder/suggest")
    async def suggest(body: SuggestRequest) -> SuggestResponse:
        return await graph_builder.suggest(body, request_id_var.get())

    @app.post("/internal/v1/graphrag/interpret", response_model_exclude_none=True)
    async def interpret(body: InterpretRequest) -> InterpretResponse:
        return await graph_rag.interpret(body, request_id_var.get())

    @app.post("/internal/v1/graphrag/synthesize")
    async def synthesize(body: SynthesizeRequest) -> SynthesizeResponse:
        return await graph_rag.synthesize(body, request_id_var.get())

    return app


app = create_app()
