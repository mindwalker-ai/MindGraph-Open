"""Shared test fixtures: a deterministic fake provider and sample payloads.

No test calls a real model or needs network access or an API key.
"""

from __future__ import annotations

import copy
from typing import Any

import pytest
from fastapi.testclient import TestClient

from mindgraph_ai.config import Settings
from mindgraph_ai.errors import AIServiceError
from mindgraph_ai.main import create_app
from mindgraph_ai.providers.base import StructuredGenerationRequest

CONFIGURED = Settings(
    provider="openai_compatible",
    base_url="http://llm.test/v1",
    model="fake-model",
    api_key="sk-test-secret",
)


def customers_suggestion() -> dict[str, Any]:
    """A valid suggestion for the fraud example's customers.csv."""
    return {
        "ontology": {
            "entity_types": [
                {
                    "name": "Customer",
                    "display_name": "Customer",
                    "properties": [
                        {"name": "customer_id", "type": "string", "required": True},
                        {"name": "name", "type": "string", "required": False},
                    ],
                }
            ],
            "relationship_types": [],
        },
        "mappings": [
            {
                "dataset": "customers.csv",
                "mapping": {
                    "entities": [
                        {
                            "alias": "customer",
                            "entity_type": "Customer",
                            "mode": "create_or_match",
                            "identity": [
                                {"source_field": "customer_id", "property": "customer_id"}
                            ],
                            "properties": [
                                {"source_field": "customer_id", "property": "customer_id"},
                                {"source_field": "name", "property": "name"},
                            ],
                        }
                    ],
                    "relationships": [],
                },
            }
        ],
        "confidence": 0.92,
        "annotations": {
            "entities": [
                {"name": "Customer", "confidence": 0.97, "reason": "customer_id is unique"}
            ],
            "relationships": [],
        },
        "warnings": [],
    }


def suggest_request(**overrides: Any) -> dict[str, Any]:
    """A valid internal request body (a bounded profile, an empty ontology)."""
    body: dict[str, Any] = {
        "datasets": [{"name": "customers.csv", "profile": customers_profile()}],
        "active_ontology": {"version": 1, "entity_types": [], "relationship_types": []},
    }
    body.update(overrides)
    return body


def customers_profile() -> dict[str, Any]:
    """The bounded profile of the fraud example's customers.csv."""
    return {
        "record_count": 5,
        "fields": [
            {
                "name": "customer_id",
                "inferred_type": "string",
                "nullable": False,
                "non_null_count": 5,
                "sample_values": ["C001", "C002"],
            },
            {
                "name": "name",
                "inferred_type": "string",
                "nullable": False,
                "non_null_count": 5,
                "sample_values": ["Alice Tan"],
            },
        ],
        "sample_rows": [{"customer_id": "C001", "name": "Alice Tan"}],
    }


class FakeProvider:
    """Returns scripted outputs (or raises scripted errors) in order and
    records every request it receives."""

    name = "fake"
    model = "fake-model"

    def __init__(self, *outputs: dict[str, Any] | AIServiceError) -> None:
        self.outputs = list(outputs) or [customers_suggestion()]
        self.requests: list[StructuredGenerationRequest] = []
        self.closed = False

    async def generate_structured(self, request: StructuredGenerationRequest) -> dict[str, Any]:
        self.requests.append(request)
        output = self.outputs.pop(0) if len(self.outputs) > 1 else self.outputs[0]
        if isinstance(output, AIServiceError):
            raise output
        return copy.deepcopy(output)

    async def aclose(self) -> None:
        self.closed = True


@pytest.fixture
def fake_provider() -> FakeProvider:
    return FakeProvider()


@pytest.fixture
def client(fake_provider: FakeProvider) -> TestClient:
    return TestClient(create_app(settings=CONFIGURED, provider=fake_provider))


@pytest.fixture
def disabled_client() -> TestClient:
    return TestClient(create_app(settings=Settings()))
