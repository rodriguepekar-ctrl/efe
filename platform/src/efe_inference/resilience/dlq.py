"""Dead-letter queue for requests exhausted by all backends."""

from __future__ import annotations

from collections import deque
from dataclasses import dataclass, field
from datetime import datetime
from typing import Protocol


@dataclass(frozen=True, slots=True)
class DlqEntry:
    request_id: str
    tenant_id: str
    task_id: str
    prompt_hash_hex: str
    last_error: str
    enqueued_at: datetime


class DeadLetterQueue(Protocol):
    async def push(
        self, entry: DlqEntry,
    ) -> None:  # pragma: no cover
        ...

    async def size(self) -> int:  # pragma: no cover
        ...

    async def drain(
        self, limit: int = 100,
    ) -> list[DlqEntry]:  # pragma: no cover
        ...


@dataclass
class InMemoryDeadLetterQueue:
    max_entries: int = 10_000
    _buf: deque[DlqEntry] = field(default_factory=deque)

    async def push(self, entry: DlqEntry) -> None:
        self._buf.append(entry)
        while len(self._buf) > self.max_entries:
            self._buf.popleft()

    async def size(self) -> int:
        return len(self._buf)

    async def drain(self, limit: int = 100) -> list[DlqEntry]:
        out: list[DlqEntry] = []
        while self._buf and len(out) < limit:
            out.append(self._buf.popleft())
        return out


__all__ = ["DeadLetterQueue", "DlqEntry", "InMemoryDeadLetterQueue"]
