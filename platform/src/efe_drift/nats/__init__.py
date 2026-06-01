"""NATS eventing layer — HMAC-signed envelopes + publisher."""

from __future__ import annotations

from efe_drift.nats.envelope import (
    EnvelopeSigner,
    EnvelopeVerifier,
    NatsEnvelope,
)
from efe_drift.nats.publisher import InMemoryPublisher, Publisher

__all__ = [
    "EnvelopeSigner",
    "EnvelopeVerifier",
    "InMemoryPublisher",
    "NatsEnvelope",
    "Publisher",
]
