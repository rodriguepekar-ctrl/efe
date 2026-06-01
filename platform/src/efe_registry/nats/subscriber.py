"""NATS subscriber — in-memory implementation for tests."""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable
from dataclasses import dataclass, field
from typing import Any, Protocol

from efe_registry.nats.schema import NatsEvent, verify_envelope

Handler = Callable[[NatsEvent], Awaitable[None]]


class NatsSubscriber(Protocol):
    async def subscribe(
        self, topic: str, handler: Handler,
    ) -> None:  # pragma: no cover
        ...

    async def close(self) -> None:  # pragma: no cover
        ...


@dataclass
class InMemoryNatsSubscriber:
    hmac_key: bytes
    handlers: dict[str, list[Handler]] = field(default_factory=dict)

    async def subscribe(self, topic: str, handler: Handler) -> None:
        self.handlers.setdefault(topic, []).append(handler)

    async def close(self) -> None:
        self.handlers.clear()

    async def deliver(self, topic: str, envelope: dict[str, Any]) -> None:
        event = verify_envelope(envelope, key=self.hmac_key)
        handlers = self.handlers.get(topic, [])
        if not handlers:
            return
        await asyncio.gather(*(h(event) for h in handlers))


__all__ = ["Handler", "InMemoryNatsSubscriber", "NatsSubscriber"]
