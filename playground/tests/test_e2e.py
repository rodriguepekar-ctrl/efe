"""EFE Playground — end-to-end smoke tests.

Tests the Flask backend in-process via Werkzeug test client.
No live network ; runs in CI offline.

Coverage :
  - health + info endpoints
  - auth flow (token issue + verify)
  - catalog register + classify + delete
  - encode endpoint
  - audit log + integrity verify
  - sample loading
  - multi-tenant isolation (tenant A cannot read tenant B catalog)
"""
from __future__ import annotations

import sys
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
BACKEND = REPO / "playground" / "backend"
sys.path.insert(0, str(BACKEND))

from app import app, CATALOG, AUDIT, EMBEDDER  # noqa: E402


@pytest.fixture
def client():
    app.config["TESTING"] = True
    with app.test_client() as c:
        # Reset state between tests
        CATALOG._store.clear()
        AUDIT._events.clear()
        AUDIT._last_hmac = "0" * 64
        yield c


def get_token(client, tenant_id="acme", role="admin"):
    resp = client.post("/auth/token", json={"tenant_id": tenant_id, "role": role})
    assert resp.status_code == 200
    return resp.get_json()["token"]


def auth_headers(token):
    return {"Authorization": f"Bearer {token}"}


# ─────────────────────────────────────────────────────────────────────
# Health + info
# ─────────────────────────────────────────────────────────────────────


def test_health(client):
    resp = client.get("/health")
    assert resp.status_code == 200
    data = resp.get_json()
    assert data["status"] == "ok"
    assert data["service"] == "efe-playground"


def test_info(client):
    resp = client.get("/info")
    assert resp.status_code == 200
    data = resp.get_json()
    assert "embedder" in data
    assert data["embedder"]["dim"] == 384
    assert data["embedder"]["backend"] in ("hash", "sentence-transformers")


# ─────────────────────────────────────────────────────────────────────
# Auth
# ─────────────────────────────────────────────────────────────────────


def test_auth_token_issue_demo_tenant(client):
    token = get_token(client, tenant_id="acme")
    assert token
    # Verify token
    resp = client.get("/auth/me", headers=auth_headers(token))
    assert resp.status_code == 200
    me = resp.get_json()
    assert me["tenant_id"] == "acme"
    assert me["is_admin"] is True


def test_auth_token_unknown_tenant_refused(client):
    resp = client.post("/auth/token", json={"tenant_id": "unknown-tenant"})
    assert resp.status_code == 400


def test_auth_no_token_rejected(client):
    resp = client.get("/catalog")
    assert resp.status_code == 401


def test_auth_invalid_token_rejected(client):
    resp = client.get("/catalog", headers={"Authorization": "Bearer xxxinvalid"})
    assert resp.status_code == 401


def test_list_demo_tenants(client):
    resp = client.get("/auth/tenants")
    assert resp.status_code == 200
    tenants = resp.get_json()["tenants"]
    assert len(tenants) >= 3
    ids = [t["tenant_id"] for t in tenants]
    assert "acme" in ids
    assert "beta-corp" in ids
    assert "demo-fr" in ids


# ─────────────────────────────────────────────────────────────────────
# Catalog register + list + classify
# ─────────────────────────────────────────────────────────────────────


def test_catalog_register(client):
    token = get_token(client)
    payload = {
        "name": "test-catalog",
        "classes": [
            {"name": "class-a", "examples": ["hello", "hi"], "confidence_threshold": 0.5},
            {"name": "class-b", "examples": ["bye", "goodbye"], "confidence_threshold": 0.5},
        ],
    }
    resp = client.post("/catalog/register", json=payload, headers=auth_headers(token))
    assert resp.status_code == 201
    data = resp.get_json()
    assert data["version"] == 1
    assert data["name"] == "test-catalog"
    assert len(data["fingerprint"]) == 64  # sha256 hex


def test_catalog_register_requires_classes(client):
    token = get_token(client)
    resp = client.post(
        "/catalog/register",
        json={"name": "empty-cat", "classes": []},
        headers=auth_headers(token),
    )
    assert resp.status_code == 400


def test_catalog_register_versioning(client):
    token = get_token(client)
    payload = {
        "name": "versioned",
        "classes": [{"name": "x", "examples": ["one"]}],
    }
    r1 = client.post("/catalog/register", json=payload, headers=auth_headers(token))
    assert r1.get_json()["version"] == 1
    r2 = client.post("/catalog/register", json=payload, headers=auth_headers(token))
    assert r2.get_json()["version"] == 2


def test_catalog_list(client):
    token = get_token(client)
    client.post(
        "/catalog/register",
        json={"name": "cat1", "classes": [{"name": "x", "examples": ["a"]}]},
        headers=auth_headers(token),
    )
    resp = client.get("/catalog", headers=auth_headers(token))
    assert resp.status_code == 200
    catalogs = resp.get_json()["catalogs"]
    assert len(catalogs) == 1
    assert catalogs[0]["name"] == "cat1"


def test_classify_top_class_returned(client):
    token = get_token(client)
    client.post(
        "/catalog/register",
        json={
            "name": "classify-test",
            "classes": [
                {"name": "greeting", "examples": ["hello", "hi", "good morning"]},
                {"name": "farewell", "examples": ["goodbye", "bye", "see you later"]},
            ],
        },
        headers=auth_headers(token),
    )
    resp = client.post(
        "/classify",
        json={"catalog": "classify-test", "text": "hi there", "top_k": 2},
        headers=auth_headers(token),
    )
    assert resp.status_code == 200
    data = resp.get_json()
    assert "top_k" in data
    assert len(data["top_k"]) == 2
    # Each result has score + above_threshold
    for r in data["top_k"]:
        assert "score" in r
        assert "class_name" in r
        assert "above_threshold" in r


def test_classify_unknown_catalog_404(client):
    token = get_token(client)
    resp = client.post(
        "/classify",
        json={"catalog": "non-existent", "text": "hello"},
        headers=auth_headers(token),
    )
    assert resp.status_code == 404


def test_catalog_delete(client):
    token = get_token(client)
    client.post(
        "/catalog/register",
        json={"name": "to-delete", "classes": [{"name": "x", "examples": ["a"]}]},
        headers=auth_headers(token),
    )
    resp = client.delete("/catalog/to-delete", headers=auth_headers(token))
    assert resp.status_code == 200
    # Verify gone
    list_resp = client.get("/catalog", headers=auth_headers(token))
    assert len(list_resp.get_json()["catalogs"]) == 0


# ─────────────────────────────────────────────────────────────────────
# Encode
# ─────────────────────────────────────────────────────────────────────


def test_encode_returns_384_dim(client):
    token = get_token(client)
    resp = client.post(
        "/encode", json={"text": "hello world"}, headers=auth_headers(token)
    )
    assert resp.status_code == 200
    data = resp.get_json()
    assert data["dim"] == 384
    assert len(data["embedding"]) == 384
    # L2-normalized (within tolerance)
    assert 0.99 <= data["norm"] <= 1.01


# ─────────────────────────────────────────────────────────────────────
# Audit log + integrity
# ─────────────────────────────────────────────────────────────────────


def test_audit_events_recorded(client):
    token = get_token(client)
    client.post(
        "/catalog/register",
        json={"name": "audit-test", "classes": [{"name": "x", "examples": ["a"]}]},
        headers=auth_headers(token),
    )
    resp = client.get("/audit", headers=auth_headers(token))
    assert resp.status_code == 200
    events = resp.get_json()["events"]
    assert any(e["action"] == "catalog.register" for e in events)
    assert any(e["action"] == "auth.issue_token" for e in events)


def test_audit_chain_integrity_ok(client):
    token = get_token(client)
    # Generate some events
    for i in range(5):
        client.post(
            "/encode", json={"text": f"text-{i}"}, headers=auth_headers(token)
        )
    resp = client.get("/audit/integrity", headers=auth_headers(token))
    assert resp.status_code == 200
    data = resp.get_json()
    assert data["ok"] is True
    assert data["broken_at"] is None
    assert data["count"] >= 5


# ─────────────────────────────────────────────────────────────────────
# Multi-tenant isolation
# ─────────────────────────────────────────────────────────────────────


def test_tenant_isolation_acme_vs_beta_corp(client):
    token_acme = get_token(client, tenant_id="acme")
    token_beta = get_token(client, tenant_id="beta-corp")

    # acme creates a catalog
    client.post(
        "/catalog/register",
        json={"name": "acme-only", "classes": [{"name": "x", "examples": ["a"]}]},
        headers=auth_headers(token_acme),
    )

    # beta-corp lists catalogs : should be empty
    resp = client.get("/catalog", headers=auth_headers(token_beta))
    assert resp.status_code == 200
    catalogs = resp.get_json()["catalogs"]
    assert len(catalogs) == 0, "beta-corp must NOT see acme's catalog (D11 isolation)"

    # beta-corp tries to classify against acme's catalog : 404
    resp = client.post(
        "/classify",
        json={"catalog": "acme-only", "text": "hi"},
        headers=auth_headers(token_beta),
    )
    assert resp.status_code == 404, (
        "beta-corp must NOT be able to classify against acme's catalog"
    )

    # acme still sees its catalog
    resp = client.get("/catalog", headers=auth_headers(token_acme))
    assert len(resp.get_json()["catalogs"]) == 1


def test_audit_log_tenant_scoped(client):
    token_acme = get_token(client, tenant_id="acme")
    token_beta = get_token(client, tenant_id="beta-corp")

    client.post(
        "/encode",
        json={"text": "acme text"},
        headers=auth_headers(token_acme),
    )
    client.post(
        "/encode",
        json={"text": "beta text"},
        headers=auth_headers(token_beta),
    )

    # acme audit log : only acme events
    resp_a = client.get("/audit", headers=auth_headers(token_acme))
    events_a = resp_a.get_json()["events"]
    assert all(e["tenant_id"] == "acme" for e in events_a)

    # beta audit log : only beta events
    resp_b = client.get("/audit", headers=auth_headers(token_beta))
    events_b = resp_b.get_json()["events"]
    assert all(e["tenant_id"] == "beta-corp" for e in events_b)


# ─────────────────────────────────────────────────────────────────────
# Samples
# ─────────────────────────────────────────────────────────────────────


def test_samples_list(client):
    resp = client.get("/samples")
    assert resp.status_code == 200
    samples = resp.get_json()["samples"]
    # Expect at least our 4 samples
    sample_ids = [s["id"] for s in samples]
    assert "support_routing" in sample_ids
    assert "content_moderation" in sample_ids
    assert "intent_recognition" in sample_ids
    assert "multilingual_intents" in sample_ids


def test_samples_load_creates_catalog(client):
    token = get_token(client)
    resp = client.post(
        "/samples/support_routing/load", headers=auth_headers(token)
    )
    assert resp.status_code == 201
    data = resp.get_json()
    assert data["loaded"] is True
    assert data["version"] == 1
    # Verify catalog exists
    list_resp = client.get("/catalog", headers=auth_headers(token))
    catalogs = list_resp.get_json()["catalogs"]
    assert any(c["name"] == "support-routing" for c in catalogs)


def test_samples_load_requires_admin(client):
    """Sample load is admin-only — verify role check."""
    token_user = get_token(client, role="user")
    resp = client.post(
        "/samples/support_routing/load", headers=auth_headers(token_user)
    )
    assert resp.status_code == 403, "user role should not load samples"
