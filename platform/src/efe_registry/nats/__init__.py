"""NATS integration — publisher, subscriber, schema."""

from __future__ import annotations

from efe_registry.nats.publisher import (
    InMemoryNatsPublisher,
    NatsPublisher,
)
from efe_registry.nats.schema import (
    NatsEvent,
    build_envelope,
    verify_envelope,
)
from efe_registry.nats.subscriber import (
    InMemoryNatsSubscriber,
    NatsSubscriber,
)

__all__ = [
    "InMemoryNatsPublisher",
    "InMemoryNatsSubscriber",
    "NatsEvent",
    "NatsPublisher",
    "NatsSubscriber",
    "build_envelope",
    "verify_envelope",
]
