"""Event bus for structured events.

Every domain action emits one :class:`Event` that carries :

* a stable ``kind`` (``efe.sched.job.enqueued``, ...),
* a canonical JSON payload,
* an HMAC seal (``DOMAIN_TAG_SCHED_EVENT_V1``) for integrity across the
  wire.

Implementations (in-memory here, NATS in production) satisfy the
:class:`EventBus` Protocol.
"""

from __future__ import annotations

import asyncio
import json
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any, Protocol

from efe_scheduling.hashing import (
    DOMAIN_TAG_SCHED_EVENT_V1,
    canonical_json,
    hmac_seal,
)

__all__ = ["Event", "EventBus", "InMemoryEventBus", "seal_event"]


@dataclass(frozen=True, slots=True)
class Event:
    kind: str
    tenant_id: str | None
    payload: dict[str, Any]
    occurred_at: datetime = field(default_factory=lambda: datetime.now(tz=timezone.utc))
    schema_version: int = 1
    seal: bytes = b""

    def to_dict(self) -> dict[str, Any]:
        return {
            "kind": self.kind,
            "tenant_id": self.tenant_id,
            "payload": self.payload,
            "occurred_at": self.occurred_at.astimezone(timezone.utc).isoformat(),
            "schema_version": self.schema_version,
            "seal": self.seal.hex(),
        }

    def to_json(self) -> bytes:
        return canonical_json(self.to_dict())


def seal_event(kind: str, tenant_id: str | None, payload: dict[str, Any]) -> Event:
    base = {
        "kind": kind,
        "tenant_id": tenant_id,
        "payload": payload,
        "schema_version": 1,
    }
    seal = hmac_seal(DOMAIN_TAG_SCHED_EVENT_V1, base)
    return Event(
        kind=kind,
        tenant_id=tenant_id,
        payload=dict(payload),
        schema_version=1,
        seal=seal,
    )


class EventBus(Protocol):
    async def publish(self, event: Event) -> None: ...  # pragma: no cover


class InMemoryEventBus:
    def __init__(self) -> None:
        self._events: list[Event] = []
        self._lock = asyncio.Lock()

    async def publish(self, event: Event) -> None:
        async with self._lock:
            self._events.append(event)

    async def drain(self) -> list[Event]:
        async with self._lock:
            out = list(self._events)
            self._events.clear()
            return out

    async def count(self) -> int:
        async with self._lock:
            return len(self._events)


_ = json  # keep import for json-encoding audit
