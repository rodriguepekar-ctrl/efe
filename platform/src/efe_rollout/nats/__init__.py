"""NATS eventing layer — HMAC-signed envelopes + publisher abstraction."""

from __future__ import annotations

from efe_rollout.nats.envelope import (
    EnvelopeSigner,
    EnvelopeVerifier,
    NatsEnvelope,
)
from efe_rollout.nats.publisher import InMemoryPublisher, Publisher

__all__ = [
    "EnvelopeSigner",
    "EnvelopeVerifier",
    "InMemoryPublisher",
    "NatsEnvelope",
    "Publisher",
]
