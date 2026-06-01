"""NATS envelope — HMAC + skew + missing-field hardening."""

from __future__ import annotations

import pytest

from efe_drift.clock import FakeClock
from efe_drift.errors import NatsEnvelopeStale, NatsMessageIntegrity
from efe_drift.nats import EnvelopeSigner, EnvelopeVerifier


def test_signed_envelope_verifies():
    key = b"k" * 32
    clock = FakeClock()
    signer = EnvelopeSigner(hmac_key=key, clock=clock)
    verifier = EnvelopeVerifier(hmac_key=key, clock=clock)
    env = signer.sign(
        event_type="x", tenant_id="acme", payload={"a": 1},
    )
    verified = verifier.verify(env.to_dict())
    assert verified.event_type == "x"


def test_envelope_rejects_missing_required_fields():
    key = b"k" * 32
    clock = FakeClock()
    signer = EnvelopeSigner(hmac_key=key, clock=clock)
    verifier = EnvelopeVerifier(hmac_key=key, clock=clock)
    good = signer.sign(
        event_type="x", tenant_id="acme", payload={},
    ).to_dict()
    for field in ("schema_version", "event_type", "event_id",
                  "occurred_at", "tenant_id", "hmac"):
        stripped = {k: v for k, v in good.items() if k != field}
        with pytest.raises(NatsMessageIntegrity):
            verifier.verify(stripped)


def test_envelope_stale_outside_window():
    key = b"k" * 32
    clock_sign = FakeClock()
    signer = EnvelopeSigner(hmac_key=key, clock=clock_sign)
    env = signer.sign(
        event_type="x", tenant_id="acme", payload={},
    )
    clock_verify = FakeClock()
    clock_verify.advance(10_000)
    verifier = EnvelopeVerifier(
        hmac_key=key, skew_window_s=300, clock=clock_verify,
    )
    with pytest.raises(NatsEnvelopeStale):
        verifier.verify(env.to_dict())
