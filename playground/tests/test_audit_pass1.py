"""EFE Playground — audit pass 1 anti-régression tests.

Cf. CHANGELOG.md `### M15 audit pass 1` for the 6 defects.

Audit-fix tests anchored on structural invariants of each fix.

  * **A1** (HIGH) Evolve RPC (D12) — POST /catalog/<name>/evolve
  * **A2** (HIGH) Catalog quotas enforced (D12) — 4 limits
  * **A3** (MED)  Rate limit per tenant (M11)
  * **A4** (MED)  PII hash mode (D13)
  * **A5** (MED)  Bundle info admin endpoint (D27) + admin quotas
  * **A6** (MED)  Idempotency cache (M11)
"""
from __future__ import annotations

import os
import sys
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
BACKEND = REPO / "playground" / "backend"
sys.path.insert(0, str(BACKEND))

# Reset the global PII_HASH_ENABLED before importing app for predictability
os.environ["EFE_PLAYGROUND_PII_HASH"] = "true"  # for test_a4

from app import app, CATALOG, AUDIT, RATE_LIMITER, IDEMPOTENCY, PII_HASH_ENABLED  # noqa: E402


@pytest.fixture
def client():
    app.config["TESTING"] = True
    with app.test_client() as c:
        CATALOG._store.clear()
        AUDIT._events.clear()
        AUDIT._last_hmac = "0" * 64
        RATE_LIMITER._buckets.clear()
        IDEMPOTENCY._store.clear()
        yield c


def get_token(client, tenant_id="acme", role="admin"):
    resp = client.post("/auth/token", json={"tenant_id": tenant_id, "role": role})
    return resp.get_json()["token"]


def auth_headers(token, **extra):
    h = {"Authorization": f"Bearer {token}"}
    h.update(extra)
    return h


# ─────────────────────────────────────────────────────────────────────
# A1 — Evolve RPC (D12)
# ─────────────────────────────────────────────────────────────────────


def test_audit_a1_evolve_appends_examples_increments_version(client):
    token = get_token(client)
    # Register v1
    payload = {
        "name": "evolving-cat",
        "classes": [
            {"name": "greeting", "examples": ["hello", "hi"], "confidence_threshold": 0.5},
        ],
    }
    r = client.post("/catalog/register", json=payload, headers=auth_headers(token))
    assert r.status_code == 201
    assert r.get_json()["version"] == 1

    # Evolve : add 2 more examples to "greeting" + create new class "farewell"
    evolve_payload = {
        "additional_examples": {
            "greeting": ["good morning", "howdy"],
            "farewell": ["bye", "goodbye"],
        }
    }
    r2 = client.post(
        "/catalog/evolving-cat/evolve",
        json=evolve_payload,
        headers=auth_headers(token),
    )
    assert r2.status_code == 201, f"Evolve failed : {r2.get_json()}"
    data = r2.get_json()
    assert data["version"] == 2
    assert data["class_count"] == 2  # greeting + farewell

    # Verify v2 has 4 examples in greeting
    cat = client.get("/catalog/evolving-cat", headers=auth_headers(token)).get_json()
    greeting = next(c for c in cat["classes"] if c["name"] == "greeting")
    assert len(greeting["examples"]) == 4
    assert "good morning" in greeting["examples"]


def test_audit_a1_evolve_unknown_catalog_404(client):
    token = get_token(client)
    r = client.post(
        "/catalog/non-existent/evolve",
        json={"additional_examples": {"x": ["a"]}},
        headers=auth_headers(token),
    )
    assert r.status_code == 404


def test_audit_a1_evolve_emits_audit_event(client):
    token = get_token(client)
    client.post("/catalog/register", json={
        "name": "ev-audit", "classes": [{"name": "x", "examples": ["a"]}]
    }, headers=auth_headers(token))

    client.post(
        "/catalog/ev-audit/evolve",
        json={"additional_examples": {"x": ["b"]}},
        headers=auth_headers(token),
    )
    events = client.get("/audit", headers=auth_headers(token)).get_json()["events"]
    assert any(e["action"] == "catalog.evolve" for e in events)


# ─────────────────────────────────────────────────────────────────────
# A2 — D12 quotas enforced
# ─────────────────────────────────────────────────────────────────────


def test_audit_a2_max_classes_quota_enforced(client):
    """Quota : MAX_CLASSES_PER_CATALOG = 256 → registering 257 classes refused."""
    from catalog import MAX_CLASSES_PER_CATALOG
    token = get_token(client)
    too_many_classes = [
        {"name": f"class-{i}", "examples": ["ex"]}
        for i in range(MAX_CLASSES_PER_CATALOG + 1)
    ]
    r = client.post(
        "/catalog/register",
        json={"name": "too-big", "classes": too_many_classes},
        headers=auth_headers(token),
    )
    assert r.status_code == 429, f"Expected 429 quota_exceeded, got {r.status_code}"
    body = r.get_json()
    assert body["error"] == "quota_exceeded"
    assert "max_classes_per_catalog" in body["message"]


def test_audit_a2_max_sentences_per_class_quota_enforced(client):
    """Quota : MAX_SENTENCES_PER_CLASS = 64 → 65 examples in a class refused."""
    from catalog import MAX_SENTENCES_PER_CLASS
    token = get_token(client)
    payload = {
        "name": "wide-class",
        "classes": [{
            "name": "x",
            "examples": [f"example {i}" for i in range(MAX_SENTENCES_PER_CLASS + 1)],
        }],
    }
    r = client.post(
        "/catalog/register", json=payload, headers=auth_headers(token)
    )
    assert r.status_code == 429
    body = r.get_json()
    assert body["error"] == "quota_exceeded"
    assert "max_sentences_per_class" in body["message"]


def test_audit_a2_admin_quotas_endpoint(client):
    """Audit fix M15-A5 + A2 — /admin/quotas endpoint surfaces usage."""
    token = get_token(client)
    # Create one catalog
    client.post(
        "/catalog/register",
        json={"name": "cat1", "classes": [{"name": "x", "examples": ["a"]}]},
        headers=auth_headers(token),
    )
    r = client.get("/admin/quotas", headers=auth_headers(token))
    assert r.status_code == 200
    data = r.get_json()
    assert data["tenant_id"] == "acme"
    assert data["current_usage"]["names_count"] == 1
    assert data["limits"]["max_classes_per_catalog"] == 256
    assert data["limits"]["max_sentences_per_class"] == 64


def test_audit_a2_info_exposes_correct_limits(client):
    """`/info` should expose the actual D12 quotas not stale fake values."""
    r = client.get("/info")
    data = r.get_json()
    assert data["limits"]["max_names_per_tenant"] == 100
    assert data["limits"]["max_versions_per_name"] == 1000
    assert data["limits"]["max_classes_per_catalog"] == 256
    assert data["limits"]["max_sentences_per_class"] == 64


# ─────────────────────────────────────────────────────────────────────
# A3 — Rate limit per tenant (M11)
# ─────────────────────────────────────────────────────────────────────


def test_audit_a3_rate_limit_blocks_after_capacity(client):
    """Bucket capacity = 50 ; the 51st request should be rate-limited.

    Note : tokens refill over time, so we send rapidly to deplete.
    """
    token = get_token(client)
    # Set up a catalog to classify against
    client.post(
        "/catalog/register",
        json={"name": "rl-cat", "classes": [{"name": "x", "examples": ["a"]}]},
        headers=auth_headers(token),
    )
    # Burn through bucket : capacity 50 → 50 classify calls should succeed,
    # subsequent should be rate-limited (until tokens refill).
    success_count = 0
    rate_limited_count = 0
    for _ in range(60):
        r = client.post(
            "/classify",
            json={"catalog": "rl-cat", "text": "hello"},
            headers=auth_headers(token),
        )
        if r.status_code == 200:
            success_count += 1
        elif r.status_code == 429:
            rate_limited_count += 1
            assert r.get_json()["error"] == "rate_limited"
            assert "rate_limit_info" in r.get_json()
    # We should have hit the limit at least once
    assert rate_limited_count >= 1, (
        f"A3 regression : 60 rapid requests should hit rate limit ; "
        f"got {success_count} success / {rate_limited_count} rate-limited"
    )


def test_audit_a3_rate_limit_info_in_info_endpoint(client):
    r = client.get("/info")
    data = r.get_json()
    assert "rate_limit" in data
    assert data["rate_limit"]["default_capacity"] == 50
    assert data["rate_limit"]["default_refill_per_sec"] == 10.0


# ─────────────────────────────────────────────────────────────────────
# A4 — PII hash mode (D13)
# ─────────────────────────────────────────────────────────────────────


def test_audit_a4_pii_hash_scrubs_email_in_classify(client):
    """When PII_HASH_ENABLED=true, classify scrubs PII before encoding."""
    if not PII_HASH_ENABLED:
        pytest.skip("PII hash not enabled in this test run")
    token = get_token(client)
    client.post(
        "/catalog/register",
        json={"name": "pii-test", "classes": [{"name": "x", "examples": ["a"]}]},
        headers=auth_headers(token),
    )
    r = client.post(
        "/classify",
        json={
            "catalog": "pii-test",
            "text": "Contact john.doe@example.com or call 555-123-4567",
        },
        headers=auth_headers(token),
    )
    assert r.status_code == 200
    data = r.get_json()
    # pii_scrub field present with counts
    assert "pii_scrub" in data
    assert data["pii_scrub"]["email"] >= 1
    assert data["pii_scrub"]["phone"] >= 1


def test_audit_a4_pii_hash_endpoint_status(client):
    r = client.get("/info")
    data = r.get_json()
    assert "pii_hash_enabled" in data
    assert isinstance(data["pii_hash_enabled"], bool)


def test_audit_a4_scrub_pii_helper_works():
    """Test the scrub_pii helper directly."""
    sys.path.insert(0, str(BACKEND))
    from middleware import scrub_pii
    text = "Email me at test@example.com phone 555-987-6543 SSN 123-45-6789"
    scrubbed, info = scrub_pii(text)
    assert "test@example.com" not in scrubbed
    assert "555-987-6543" not in scrubbed
    assert "123-45-6789" not in scrubbed
    assert "<PII_HASH:" in scrubbed
    assert info["email"] == 1
    assert info["phone"] == 1
    assert info["ssn"] == 1


# ─────────────────────────────────────────────────────────────────────
# A5 — Bundle info admin endpoint (D27)
# ─────────────────────────────────────────────────────────────────────


def test_audit_a5_admin_bundle_info_endpoint(client):
    token = get_token(client)
    r = client.get("/admin/bundle-info", headers=auth_headers(token))
    assert r.status_code == 200
    data = r.get_json()
    # Required D27 contract fields
    assert "bundle_slug" in data
    assert "bundle_id" in data
    assert "manifestSha256" in data
    assert "framework" in data
    assert "dim" in data
    assert "is_dev_unsigned" in data
    # bundle_id matches D16 §3.2 canonical pattern (loosely)
    assert "@1.0.0" in data["bundle_id"]
    # Stable fingerprint (32 hex chars or 64)
    assert len(data["manifestSha256"]) == 64
    assert all(c in "0123456789abcdef" for c in data["manifestSha256"])


def test_audit_a5_admin_bundle_info_requires_auth(client):
    r = client.get("/admin/bundle-info")
    assert r.status_code == 401


# ─────────────────────────────────────────────────────────────────────
# A6 — Idempotency cache (M11)
# ─────────────────────────────────────────────────────────────────────


def test_audit_a6_idempotency_replays_cached_response(client):
    """Same Idempotency-Key + body → same response with X-EFE-Idempotency-Replay header."""
    token = get_token(client)
    client.post(
        "/catalog/register",
        json={"name": "idem-cat", "classes": [{"name": "greeting", "examples": ["hi"]}]},
        headers=auth_headers(token),
    )
    body = {"catalog": "idem-cat", "text": "hello world"}
    # First call : no cache, runs normally
    r1 = client.post(
        "/classify", json=body,
        headers=auth_headers(token, **{"Idempotency-Key": "test-key-001"}),
    )
    assert r1.status_code == 200
    assert "X-EFE-Idempotency-Replay" not in r1.headers

    # Second call : same key + body → cached replay
    r2 = client.post(
        "/classify", json=body,
        headers=auth_headers(token, **{"Idempotency-Key": "test-key-001"}),
    )
    assert r2.status_code == 200
    assert r2.headers.get("X-EFE-Idempotency-Replay") == "true", (
        "Expected X-EFE-Idempotency-Replay header on cached replay"
    )
    # Responses byte-identical
    assert r1.get_json() == r2.get_json()


def test_audit_a6_idempotency_no_key_no_cache(client):
    token = get_token(client)
    client.post(
        "/catalog/register",
        json={"name": "no-idem", "classes": [{"name": "x", "examples": ["a"]}]},
        headers=auth_headers(token),
    )
    body = {"catalog": "no-idem", "text": "test"}
    # Without Idempotency-Key header, no caching happens
    r1 = client.post("/classify", json=body, headers=auth_headers(token))
    r2 = client.post("/classify", json=body, headers=auth_headers(token))
    assert "X-EFE-Idempotency-Replay" not in r1.headers
    assert "X-EFE-Idempotency-Replay" not in r2.headers


def test_audit_a6_idempotency_stats_in_info(client):
    r = client.get("/info")
    data = r.get_json()
    assert "idempotency" in data
    assert "ttl_seconds" in data["idempotency"]
    assert "capacity" in data["idempotency"]
