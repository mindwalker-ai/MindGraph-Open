"""HTTP behaviour of the AI service: health, readiness and the internal
Graph Builder endpoint, with a fake provider."""

from __future__ import annotations

from typing import Any

from conftest import CONFIGURED, FakeProvider, customers_suggestion, suggest_request
from fastapi.testclient import TestClient

from mindgraph_ai import __version__
from mindgraph_ai.config import Settings
from mindgraph_ai.errors import (
    InvalidProviderOutputError,
    ProviderRateLimitedError,
    ProviderRejectedError,
    ProviderTimeoutError,
    ProviderUnavailableError,
)
from mindgraph_ai.graph_builder import PROMPT_VERSION
from mindgraph_ai.main import create_app

SUGGEST = "/internal/v1/graph-builder/suggest"


def app_with(*outputs: Any) -> tuple[TestClient, FakeProvider]:
    provider = FakeProvider(*outputs)
    return TestClient(create_app(settings=CONFIGURED, provider=provider)), provider


def test_health_is_ok_even_without_ai(disabled_client: TestClient) -> None:
    response = disabled_client.get("/health")
    assert response.status_code == 200
    assert response.json() == {"status": "ok", "service": "mindgraph-ai", "version": __version__}


def test_ready_when_configured(client: TestClient) -> None:
    response = client.get("/ready")
    assert response.status_code == 200
    assert response.json() == {
        "status": "ready",
        "configured": True,
        "provider": "fake",
        "model": "fake-model",
    }
    assert "secret" not in response.text and "llm.test" not in response.text


def test_ready_when_not_configured(disabled_client: TestClient) -> None:
    response = disabled_client.get("/ready")
    assert response.status_code == 503
    assert response.json() == {"status": "not_configured", "configured": False}


def test_ready_reports_invalid_configuration_without_details() -> None:
    settings = Settings.from_env({"MINDGRAPH_LLM_PROVIDER": "openai_compatible"})
    response = TestClient(create_app(settings=settings)).get("/ready")
    assert response.status_code == 503
    assert response.json() == {
        "status": "not_configured",
        "configured": False,
        "reason": "invalid_configuration",
    }


def test_valid_suggestions_are_returned_with_metadata(client: TestClient) -> None:
    response = client.post(SUGGEST, json=suggest_request(), headers={"x-request-id": "job-1"})
    assert response.status_code == 200, response.text
    body = response.json()
    assert body["metadata"] == {
        "prompt_version": PROMPT_VERSION,
        "provider": "fake",
        "model": "fake-model",
    }
    assert body["suggestion"]["ontology"]["entity_types"][0]["name"] == "Customer"
    mapping = body["suggestion"]["mappings"][0]
    assert mapping["dataset"] == "customers.csv"
    assert mapping["mapping"]["entities"][0]["mode"] == "create_or_match"
    assert response.headers["x-request-id"] == "job-1"


def test_the_request_id_reaches_the_provider() -> None:
    client, provider = app_with()
    client.post(SUGGEST, json=suggest_request(), headers={"x-request-id": "job-42"})
    assert provider.requests[0].request_id == "job-42"


def test_suggestions_are_rejected_when_ai_is_not_configured(disabled_client: TestClient) -> None:
    response = disabled_client.post(SUGGEST, json=suggest_request())
    assert response.status_code == 503
    assert response.json()["error"]["code"] == "AI_NOT_CONFIGURED"


def test_schema_violations_in_model_output_are_rejected() -> None:
    def broken(mutate: Any) -> dict[str, Any]:
        suggestion = customers_suggestion()
        mutate(suggestion)
        return suggestion

    cases = [
        broken(lambda s: s.update(confidence=1.4)),
        broken(lambda s: s["annotations"]["entities"][0].update(confidence=-0.1)),
        broken(lambda s: s.update(chain_of_thought="first I thought...")),
        broken(lambda s: s["ontology"]["entity_types"][0].update(name="Customer Record")),
        broken(lambda s: s["ontology"]["entity_types"][0]["properties"][0].update(type="decimal")),
        broken(lambda s: s["mappings"][0]["mapping"]["entities"][0].update(mode="fuzzy")),
        broken(lambda s: s["mappings"][0]["mapping"]["entities"][0].update(identity=[])),
        broken(lambda s: s["mappings"][0].update(data_source_id="x")),
        broken(lambda s: s.update(mappings=[])),
        broken(lambda s: s.update(warnings=["w"] * 21)),
        broken(lambda s: s.update(warnings=["x" * 301])),
        broken(lambda s: s["ontology"].update(entity_types=s["ontology"]["entity_types"] * 26)),
        broken(lambda s: s.pop("mappings")),
    ]
    for case in cases:
        client, _ = app_with(case)
        response = client.post(SUGGEST, json=suggest_request())
        assert response.status_code == 502, case
        assert response.json()["error"]["code"] == "INVALID_PROVIDER_OUTPUT"
        assert "Customer Record" not in response.text


def test_provider_errors_map_to_stable_codes() -> None:
    cases = [
        (ProviderTimeoutError("t"), 504, "PROVIDER_TIMEOUT"),
        (ProviderRateLimitedError("r"), 429, "PROVIDER_RATE_LIMITED"),
        (ProviderUnavailableError("u"), 503, "PROVIDER_UNAVAILABLE"),
        (ProviderRejectedError("x"), 502, "PROVIDER_REJECTED"),
        (InvalidProviderOutputError("j"), 502, "INVALID_PROVIDER_OUTPUT"),
    ]
    for error, status, code in cases:
        client, _ = app_with(error)
        response = client.post(SUGGEST, json=suggest_request())
        assert response.status_code == status
        assert response.json() == {"error": {"code": code, "message": error.message}}


def test_invalid_requests_are_rejected_without_echoing_input(client: TestClient) -> None:
    body = suggest_request()
    body["datasets"][0]["profile"]["fields"][0]["inferred_type"] = "secret-sample-value"
    response = client.post(SUGGEST, json=body)
    assert response.status_code == 422
    assert response.json()["error"]["code"] == "INVALID_REQUEST"
    assert "secret-sample-value" not in response.text

    extra = suggest_request(file_contents="customer_id,name\n...")
    assert client.post(SUGGEST, json=extra).status_code == 422


def test_oversized_requests_are_rejected(client: TestClient) -> None:
    response = client.post(
        SUGGEST,
        content=b"{" + b" " * (600 * 1024) + b"}",
        headers={"content-type": "application/json"},
    )
    assert response.status_code == 413
    assert response.json()["error"]["code"] == "PAYLOAD_TOO_LARGE"


def test_repair_requests_are_accepted() -> None:
    client, provider = app_with()
    body = suggest_request(
        repair={
            "previous": customers_suggestion(),
            "errors": ["identity property `customer_id` of `customer` must also be mapped"],
        }
    )
    assert client.post(SUGGEST, json=body).status_code == 200
    assert "validation_errors" in provider.requests[0].user_prompt


def test_active_ontologies_may_exceed_suggestion_bounds() -> None:
    client, _ = app_with(customers_suggestion())
    wide = {
        "name": "Customer",
        "properties": [{"name": f"attr_{i}", "type": "string"} for i in range(150)],
    }
    body = suggest_request(
        active_ontology={"version": 2, "entity_types": [wide], "relationship_types": []}
    )
    assert client.post(SUGGEST, json=body).status_code == 200


def test_several_datasets_are_accepted_and_bounded() -> None:
    client, provider = app_with(customers_suggestion())
    datasets = [
        {"name": f"part-{i}.csv", "profile": suggest_request()["datasets"][0]["profile"]}
        for i in range(3)
    ]
    assert client.post(SUGGEST, json=suggest_request(datasets=datasets)).status_code == 200
    assert '"name":"part-2.csv"' in provider.requests[0].user_prompt
    for bad in ([], datasets * 4):
        response = client.post(SUGGEST, json=suggest_request(datasets=bad))
        assert response.status_code == 422
        assert response.json()["error"]["code"] == "INVALID_REQUEST"


def test_unknown_internal_routes_do_not_exist(client: TestClient) -> None:
    assert client.post("/internal/v1/ontology/suggest", json={}).status_code == 404
