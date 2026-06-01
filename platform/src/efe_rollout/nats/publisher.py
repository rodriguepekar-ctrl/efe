"""NATS publisher abstraction — Protocol-based, in-memory for tests.

Real deployments replace :class:`InMemoryPublisher` with a
JetStream-backed implementation. Callers depend on the Protocol, never
on a concrete class.
"""

from __future__ import annotations

from typing import Any, Protocol


class Publisher(Protocol):
    async def publish(
        self, subject: str, payload: dict[str, Any],
    ) -> None:  # pragma: no cover
        ...


class InMemoryPublisher:
    """Records every published message for unit-test introspection."""

    __slots__ = ("_messages",)

    def __init__(self) -> None:
        self._messages: list[tuple[str, dict[str, Any]]] = []

    async def publish(self, subject: str, payload: dict[str, Any]) -> None:
        self._messages.append((subject, dict(payload)))

    def messages(
        self, *, subject: str | None = None,
    ) -> tuple[tuple[str, dict[str, Any]], ...]:
        if subject is None:
            return tuple(self._messages)
        return tuple(m for m in self._messages if m[0] == subject)

    def clear(self) -> None:
        self._messages.clear()


__all__ = ["InMemoryPublisher", "Publisher"]
