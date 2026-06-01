"""NATS envelope HMAC + verify."""

from __future__ import annotations

import uuid
from datetime import datetime, timezone

import pytest

from efe_registry.errors import NatsMessageIntegrity
from efe_registry.nats.schema import NatsEvent, build_envelope, verify_envelope


def test_roundtrip():
    key = b"\x01" * 32
    evt = NatsEvent(
        event_type="efe.registry.artifact.signed",
        event_id=str(uuid.uuid4()),
        occurred_at=datetime(2026, 4, 20, 10, 0, 0, tzinfo=timezone.utc),
        tenant_id="acme",
        payload={"artifact_id": "a-1"},
    )
    env = build_envelope(evt, key=key)
    out = verify_envelope(env, key=key)
    assert out.event_type == evt.event_type
    assert out.tenant_id == "acme"
    assert out.payload["artifact_id"] == "a-1"


def test_tampered_payload_fails():
    key = b"\x02" * 32
    evt = NatsEvent(
        event_type="efe.registry.rollback.completed",
        event_id="abc",
        occurred_at=datetime(2026, 4, 20, tzinfo=timezone.utc),
        tenant_id="acme",
        payload={"latency_ms": 42},
    )
    env = build_envelope(evt, key=key)
    env["payload"]["latency_ms"] = 999_999  # tamper
    with pytest.raises(NatsMessageIntegrity):
        verify_envelope(env, key=key)


def test_wrong_key_fails():
    k1 = b"\x03" * 32
    k2 = b"\x04" * 32
    evt = NatsEvent(
        event_type="x", event_id="1",
        occurred_at=datetime(2026, 4, 20, tzinfo=timezone.utc),
        tenant_id="t", payload={},
    )
    env = build_envelope(evt, key=k1)
    with pytest.raises(NatsMessageIntegrity):
        verify_envelope(env, key=k2)


def test_missing_hmac_fails():
    with pytest.raises(NatsMessageIntegrity):
        verify_envelope({"tenant_id": "t"}, key=b"\x00" * 32)
