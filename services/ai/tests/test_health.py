from fastapi.testclient import TestClient

from mindgraph_ai import __version__
from mindgraph_ai.main import create_app


def test_health_reports_ok() -> None:
    client = TestClient(create_app())

    response = client.get("/health")

    assert response.status_code == 200
    assert response.json() == {
        "status": "ok",
        "service": "mindgraph-ai",
        "version": __version__,
    }


def test_internal_endpoints_are_not_implemented_yet() -> None:
    client = TestClient(create_app())

    response = client.post("/internal/v1/ontology/suggest", json={})

    assert response.status_code == 404
