"""NATS publisher — in-memory for tests, prod uses nats-py."""

from __future__ import annotations

import uuid
from collections.abc import Mapping
from dataclasses import dataclass, field
from datetime import timezone
from typing import Any, Protocol

from efe_inference.clock import Clock, SystemClock
from efe_inference.nats.schema import NatsEvent, build_envelope


class NatsPublisher(Protocol):
    async def publish(
        self, topic: str, payload: Mapping[str, Any], *,
        tenant_id: str | None = None,
    ) -> None:  # pragma: no cover
        ...


@dataclass
class InMemoryNatsPublisher:
    hmac_key: bytes
    clock: Clock = field(default_factory=SystemClock)
    sent: list[tuple[str, dict[str, Any]]] = field(default_factory=list)

    async def publish(
        self, topic: str, payload: Mapping[str, Any], *,
        tenant_id: str | None = None,
    ) -> None:
        event = NatsEvent(
            event_type=topic,
            event_id=str(uuid.uuid4()),
            occurred_at=self.clock.now().astimezone(timezone.utc),
            tenant_id=tenant_id or str(payload.get("tenant_id", "")),
            payload=dict(payload),
        )
        self.sent.append((topic, build_envelope(event, key=self.hmac_key)))


__all__ = ["InMemoryNatsPublisher", "NatsPublisher"]
