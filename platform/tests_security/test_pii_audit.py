"""Tests for PII detection/masking and audit chain integrity."""

from __future__ import annotations

import asyncio
from datetime import datetime, timedelta, timezone

import pytest

from efe_security.audit import (
    AuditDraft,
    AuditOutcome,
    AuditSeverity,
    InMemoryAuditStore,
    verify_range,
)
from efe_security.pii import (
    Classification,
    FieldDescriptor,
    FieldTransform,
    MaskMode,
    Masker,
    PiiClassifier,
    PiiFieldRegistry,
    PiiKind,
    pseudonymize,
    pseudonymize_hashed,
    scrub,
)
from efe_security.errors import FieldNotClassified


# ---------------------------------------------------------------------------
# Classifier
# ---------------------------------------------------------------------------


def test_classifier_detects_email() -> None:
    c = PiiClassifier()
    hits = c.find("Contact me at alice@example.com please.")
    assert any(h.kind == PiiKind.email for h in hits)


def test_classifier_detects_luhn_valid_cc_only() -> None:
    c = PiiClassifier()
    # A known Luhn-valid test card number
    hits = c.find("card 4242 4242 4242 4242 yay")
    assert any(h.kind == PiiKind.credit_card for h in hits)
    # A Luhn-invalid long digit string is not flagged.
    hits2 = c.find("code 1234 5678 9012 3456")
    assert not any(h.kind == PiiKind.credit_card for h in hits2)


def test_classifier_nfkc_detects_full_width_email() -> None:
    c = PiiClassifier()
    # Full-width "@" + full-width letters
    text = "Contact: alice@example.com"
    hits = c.find(text)
    assert len(hits) >= 1


def test_classifier_walks_nested_dict() -> None:
    c = PiiClassifier()
    obj = {
        "prompt": "Call 555-123-4567 and email bob@test.org",
        "nested": {"field": ["192.168.1.1"]},
    }
    paths = [p for p, _ in c.walk(obj)]
    # At least one of the paths should be nested.
    assert any("nested" in p for p in paths)


# ---------------------------------------------------------------------------
# Masker
# ---------------------------------------------------------------------------


def test_masker_redacts_email() -> None:
    m = Masker(classifier=PiiClassifier(), mode=MaskMode.redact)
    out = m.mask_text("send to alice@example.com")
    assert "[PII:email]" in out
    assert "alice@example.com" not in out


def test_masker_partial_mode_keeps_edges() -> None:
    m = Masker(classifier=PiiClassifier(), mode=MaskMode.partial, keep_chars=2)
    out = m.mask_text("alice@example.com")
    assert out.startswith("al") and out.endswith("om")
    assert "*" in out


def test_scrub_convenience_walks_object() -> None:
    obj = {"msg": "email alice@e.com", "meta": {"phone": "+33 6 12 34 56 78"}}
    out = scrub(obj)
    assert "PII" in out["msg"]


# ---------------------------------------------------------------------------
# Pseudonymizer
# ---------------------------------------------------------------------------


def test_pseudonymize_deterministic() -> None:
    a = pseudonymize("u1", tenant_id="t1")
    b = pseudonymize("u1", tenant_id="t1")
    c = pseudonymize("u1", tenant_id="t2")
    assert a == b
    assert a != c


def test_pseudonymize_hashed_is_salted() -> None:
    a = pseudonymize_hashed("1.2.3.4", tenant_id="t1")
    b = pseudonymize_hashed("1.2.3.4", tenant_id="t2")
    assert a != b


# ---------------------------------------------------------------------------
# Field registry
# ---------------------------------------------------------------------------


def test_registry_requires_classified_field() -> None:
    reg = PiiFieldRegistry()
    asyncio.run(
        reg.register(
            FieldDescriptor(
                table_name="events",
                column_name="user_id",
                classification=Classification.pii,
                detector="regex:none",
                transform=FieldTransform.pseudonymize,
                retention_days=365,
            )
        )
    )
    # Present → OK.
    desc = asyncio.run(reg.require("events", "user_id"))
    assert desc.classification == Classification.pii
    # Absent → raises.
    with pytest.raises(FieldNotClassified):
        asyncio.run(reg.require("events", "prompt_text"))


# ---------------------------------------------------------------------------
# Audit chain
# ---------------------------------------------------------------------------


@pytest.fixture()
def now() -> datetime:
    return datetime(2026, 4, 21, 12, 0, tzinfo=timezone.utc)


def _draft(
    now: datetime,
    *,
    tenant: str | None = "t1",
    action: str = "authz.decision.allow",
    outcome: AuditOutcome = AuditOutcome.allow,
) -> AuditDraft:
    return AuditDraft(
        occurred_at=now,
        tenant_id=tenant,
        actor_kind="human",
        actor_id="u1",
        action=action,
        outcome=outcome,
        severity=AuditSeverity.info,
        reasons=("test",),
        payload={"k": "v"},
    )


def test_audit_append_builds_chain(now: datetime) -> None:
    store = InMemoryAuditStore()
    r1 = asyncio.run(store.append(_draft(now)))
    r2 = asyncio.run(store.append(_draft(now + timedelta(seconds=1))))
    assert r2.prev_seal == r1.seal
    assert r1.prev_seal == b"GENESIS"


def test_audit_verify_range_ok(now: datetime) -> None:
    store = InMemoryAuditStore()
    for i in range(10):
        asyncio.run(store.append(_draft(now + timedelta(seconds=i))))
    out = asyncio.run(verify_range(store))
    assert out.ok
    assert out.inspected == 10


def test_audit_tamper_detected(now: datetime) -> None:
    """Audit Fix 1 : the seal covers the record's own fields including
    payload_digest. Tampering a record's action surfaces immediately ;
    tampering the side-store of payloads is irrelevant because the
    chain is self-verifying from persisted fields only."""
    store = InMemoryAuditStore()
    for i in range(5):
        asyncio.run(store.append(_draft(now + timedelta(seconds=i))))
    # Rebuild record #2 with a tampered action but keep the original seal.
    target = store._rows[2]
    store._rows[2] = type(target)(
        audit_id=target.audit_id,
        occurred_at=target.occurred_at,
        tenant_id=target.tenant_id,
        actor_kind=target.actor_kind,
        actor_id=target.actor_id,
        action="authz.decision.deny",  # tampered field
        outcome=target.outcome,
        severity=target.severity,
        resource_kind=target.resource_kind,
        resource_id=target.resource_id,
        reasons=target.reasons,
        payload_digest=target.payload_digest,
        prev_seal=target.prev_seal,
        seal=target.seal,  # unchanged — surfaces on recompute
        key_version=target.key_version,
    )
    out = asyncio.run(verify_range(store))
    assert not out.ok
    assert out.first_broken is not None


def test_audit_search_filters() -> None:
    from efe_security.audit import AuditQuery, InMemoryAuditSearch

    now = datetime(2026, 4, 21, 12, 0, tzinfo=timezone.utc)
    store = InMemoryAuditStore()
    asyncio.run(store.append(_draft(now, action="a.b")))
    asyncio.run(store.append(_draft(now + timedelta(seconds=1), action="c.d")))
    search = InMemoryAuditSearch(store=store)
    out = asyncio.run(search.query(AuditQuery(action_prefix="a.")))
    assert len(out) == 1
