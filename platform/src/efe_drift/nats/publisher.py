"""NATS publisher abstraction — Protocol + in-memory test fixture."""

from __future__ import annotations

from typing import Any, Protocol


class Publisher(Protocol):
    async def publish(
        self, subject: str, payload: dict[str, Any],
    ) -> None:  # pragma: no cover
        ...


class InMemoryPublisher:
    """Records every published message for inspection in tests."""

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

    def __len__(self) -> int:
        return len(self._messages)


__all__ = ["InMemoryPublisher", "Publisher"]
