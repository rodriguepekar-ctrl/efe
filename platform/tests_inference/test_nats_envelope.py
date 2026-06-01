"""NATS envelope HMAC."""

from __future__ import annotations

import uuid
from datetime import datetime, timezone

import pytest

from efe_inference.nats.schema import (
    NatsEvent,
    NatsMessageIntegrity,
    build_envelope,
    verify_envelope,
)


def test_roundtrip():
    key = b"\x01" * 32
    now = datetime(2026, 4, 20, 10, 0, 0, tzinfo=timezone.utc)
    evt = NatsEvent(
        event_type="efe.inference.completed",
        event_id=str(uuid.uuid4()),
        occurred_at=now,
        tenant_id="acme",
        payload={"request_id": "r-1", "backend": "mock"},
    )
    env = build_envelope(evt, key=key)
    out = verify_envelope(env, key=key, now=now)
    assert out.event_type == "efe.inference.completed"
    assert out.tenant_id == "acme"


def test_tamper_detected():
    key = b"\x02" * 32
    now = datetime(2026, 4, 20, tzinfo=timezone.utc)
    evt = NatsEvent(
        event_type="efe.inference.completed",
        event_id="1",
        occurred_at=now,
        tenant_id="acme", payload={},
    )
    env = build_envelope(evt, key=key)
    env["payload"]["injected"] = "tamper"
    with pytest.raises(NatsMessageIntegrity):
        verify_envelope(env, key=key, now=now)


def test_wrong_key_rejected():
    now = datetime(2026, 4, 20, tzinfo=timezone.utc)
    evt = NatsEvent(
        event_type="x", event_id="1",
        occurred_at=now,
        tenant_id="t", payload={},
    )
    env = build_envelope(evt, key=b"\x03" * 32)
    with pytest.raises(NatsMessageIntegrity):
        verify_envelope(env, key=b"\x04" * 32, now=now)
