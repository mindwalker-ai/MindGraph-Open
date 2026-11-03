"""FastAPI application for the MindGraph AI service.

The AI service is an internal coprocessor consumed only by MindGraph Core
(ADR-007). Internal endpoints will live under ``/internal/v1`` and arrive with
M5 (AI Graph Builder) and M7 (GraphRAG). M0 only exposes a health check.
"""

from fastapi import FastAPI
from pydantic import BaseModel

from mindgraph_ai import __version__

SERVICE_NAME = "mindgraph-ai"


class Health(BaseModel):
    """Response body of ``GET /health``."""

    status: str
    service: str
    version: str


def create_app() -> FastAPI:
    """Build the FastAPI application."""
    app = FastAPI(title="MindGraph AI (internal)", version=__version__)

    @app.get("/health")
    async def health() -> Health:
        return Health(status="ok", service=SERVICE_NAME, version=__version__)

    return app


app = create_app()
