"""NATS publisher — production uses nats-py; tests use in-memory."""

from __future__ import annotations

import uuid
from collections.abc import Mapping
from dataclasses import dataclass, field
from datetime import timezone
from typing import Any, Protocol

from efe_registry.clock import Clock, SystemClock
from efe_registry.nats.schema import NatsEvent, build_envelope


class NatsPublisher(Protocol):
    async def publish(
        self,
        topic: str,
        payload: Mapping[str, Any],
        *,
        tenant_id: str | None = None,
    ) -> None:  # pragma: no cover
        ...


@dataclass
class InMemoryNatsPublisher:
    """Collects every published envelope for assertions."""

    hmac_key: bytes
    clock: Clock = field(default_factory=SystemClock)
    sent: list[tuple[str, dict[str, Any]]] = field(default_factory=list)

    async def publish(
        self,
        topic: str,
        payload: Mapping[str, Any],
        *,
        tenant_id: str | None = None,
    ) -> None:
        event = NatsEvent(
            event_type=topic,
            event_id=str(uuid.uuid4()),
            occurred_at=self.clock.now().astimezone(timezone.utc),
            tenant_id=tenant_id or str(payload.get("tenant_id", "")),
            payload=dict(payload),
        )
        envelope = build_envelope(event, key=self.hmac_key)
        self.sent.append((topic, envelope))


__all__ = ["InMemoryNatsPublisher", "NatsPublisher"]
