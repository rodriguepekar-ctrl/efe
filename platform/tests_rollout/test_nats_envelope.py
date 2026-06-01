"""NATS envelope HMAC + skew-window protection."""

from __future__ import annotations

import pytest


from efe_rollout.clock import FakeClock
from efe_rollout.errors import NatsEnvelopeStale, NatsMessageIntegrity
from efe_rollout.nats import EnvelopeSigner, EnvelopeVerifier


def test_signed_envelope_verifies():
    key = b"k" * 32
    clock = FakeClock()
    signer = EnvelopeSigner(hmac_key=key, clock=clock)
    verifier = EnvelopeVerifier(
        hmac_key=key, skew_window_s=300, clock=clock,
    )
    env = signer.sign(
        event_type="efe.rollout.aborted",
        tenant_id="acme",
        payload={"experiment_id": "e-1", "reason": "guardrail"},
    )
    # Publish → serialize to dict → verify
    verified = verifier.verify(env.to_dict())
    assert verified.event_type == "efe.rollout.aborted"
    assert verified.payload["experiment_id"] == "e-1"


def test_envelope_stale_when_outside_window():
    key = b"k" * 32
    clock_sign = FakeClock()
    signer = EnvelopeSigner(hmac_key=key, clock=clock_sign)
    env = signer.sign(
        event_type="x", tenant_id="t",
        payload={},
    )
    # Now verifier is 1 h in the future
    clock_verify = FakeClock()
    clock_verify.advance(3_600)
    verifier = EnvelopeVerifier(
        hmac_key=key, skew_window_s=300, clock=clock_verify,
    )
    with pytest.raises(NatsEnvelopeStale):
        verifier.verify(env.to_dict())


def test_envelope_integrity_failure_on_tampered_payload():
    key = b"k" * 32
    clock = FakeClock()
    signer = EnvelopeSigner(hmac_key=key, clock=clock)
    verifier = EnvelopeVerifier(
        hmac_key=key, skew_window_s=300, clock=clock,
    )
    env = signer.sign(
        event_type="x", tenant_id="t", payload={"v": 1},
    )
    tampered = env.to_dict()
    tampered["payload"]["v"] = 2
    with pytest.raises(NatsMessageIntegrity):
        verifier.verify(tampered)


def test_envelope_integrity_failure_on_wrong_key():
    key = b"k" * 32
    clock = FakeClock()
    signer = EnvelopeSigner(hmac_key=key, clock=clock)
    verifier = EnvelopeVerifier(
        hmac_key=b"w" * 32, skew_window_s=300, clock=clock,
    )
    env = signer.sign(
        event_type="x", tenant_id="t", payload={},
    )
    with pytest.raises(NatsMessageIntegrity):
        verifier.verify(env.to_dict())
