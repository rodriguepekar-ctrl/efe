"""HTTP API smoke tests via FastAPI TestClient."""

from __future__ import annotations

import pytest

try:
    from fastapi.testclient import TestClient
except ImportError:   # pragma: no cover
    TestClient = None   # type: ignore[assignment,misc]

from efe_drift import DriftClient
from efe_drift.api import AppContext, build_app


pytestmark = pytest.mark.skipif(
    TestClient is None, reason="fastapi / TestClient not installed",
)


def _make_app() -> TestClient:  # type: ignore[valid-type]
    client = DriftClient.in_memory()
    ctx = AppContext(
        client=client,
        evaluator=client.evaluator,
        router=client.router,
        rules=client.rule_store,
        incidents=client.incident_store,
    )
    return TestClient(build_app(ctx))  # type: ignore[misc]


def test_health_endpoint():
    app = _make_app()
    response = app.get("/healthz")
    assert response.status_code == 200
    assert response.json() == {"status": "ok"}


def test_register_and_list_feature():
    app = _make_app()
    # Empty list.
    r = app.get("/v1/drift/features")
    assert r.status_code == 200
    assert r.json() == []
    # Register.
    payload = {
        "tenant_id": "acme",
        "task_id": "summarize",
        "name": "prompt_length_tokens",
        "kind": "numeric",
        "extractor": "token_length",
        "binning_strategy": "equifrequency",
    }
    r = app.post("/v1/drift/features", json=payload)
    assert r.status_code == 200
    body = r.json()
    assert body["name"] == "prompt_length_tokens"
    # List again.
    r = app.get("/v1/drift/features")
    assert len(r.json()) == 1


def test_ingest_event_without_features_returns_zero_samples():
    app = _make_app()
    r = app.post("/v1/drift/samples", json={
        "event_id": "e1",
        "tenant_id": "unregistered",
        "task_id": "t",
        "user_id": "u",
        "observed_at": "2026-04-10T10:00:00+00:00",
    })
    assert r.status_code == 200
    assert r.json() == {"ok": True, "samples_written": 0, "features_matched": 0}


def test_register_alert_rule_and_list():
    app = _make_app()
    payload = {
        "tenant_id": "acme",
        "method": "psi",
        "thresholds": {"warning": 0.1, "major": 0.25, "critical": 0.4},
        "channels": [{"channel": "slack", "min_severity": "warning"}],
        "consecutive_breaches": 3,
        "suppression_s": 1800,
    }
    r = app.post("/v1/drift/rules", json=payload)
    assert r.status_code == 200
    rule = r.json()
    assert rule["tenant_id"] == "acme"
    r = app.get("/v1/drift/rules", params={"tenant_id": "acme"})
    assert len(r.json()) == 1
    # Delete.
    rule_id = rule["rule_id"]
    r = app.delete(f"/v1/drift/rules/{rule_id}")
    assert r.status_code == 200
    r = app.get("/v1/drift/rules", params={"tenant_id": "acme"})
    assert r.json() == []


def test_list_incidents_empty_by_default():
    app = _make_app()
    r = app.get("/v1/drift/incidents")
    assert r.status_code == 200
    assert r.json() == []


def test_evaluate_unknown_feature_returns_404_via_error_handler():
    app = _make_app()
    r = app.post("/v1/drift/evaluate/does-not-exist")
    assert r.status_code == 404
    assert r.json()["code"] == "feature_not_found"
