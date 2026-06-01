"""Session state for the labeling UI.

A session lives for the duration of one annotator's active browsing
of one ``(tenant, task)`` pair. It stores:

* the current batch of candidates served to the annotator,
* a cursor into that batch,
* running counters (labels submitted, skipped),
* a short history buffer for the "undo" keystroke,
* rolling Cohen's κ vs peers.

Backed by Redis in production (TTL), in-memory in tests.
"""

from __future__ import annotations

import json
import time
from collections.abc import Iterable
from dataclasses import dataclass, replace
from typing import Protocol

from efe_ft.errors import SessionExpired


@dataclass(frozen=True, slots=True)
class HistoryEntry:
    sample_id: str
    label: str
    submitted_at: float
    time_ms: int


@dataclass(frozen=True, slots=True)
class SessionState:
    """Immutable snapshot of a labeler session."""

    session_id: str
    annotator_id: str
    tenant_id: str
    task_id: str
    started_at: float
    labels_submitted: int
    skipped: int
    flagged: int
    history: tuple[HistoryEntry, ...]
    current_batch: tuple[str, ...]
    batch_cursor: int
    kappa_rolling: float

    def with_label(self, entry: HistoryEntry) -> SessionState:
        """Return a new state after a label submission."""
        new_history = (*self.history[-9:], entry)
        return replace(
            self,
            labels_submitted=self.labels_submitted + 1,
            history=new_history,
            batch_cursor=self.batch_cursor + 1,
        )

    def with_skip(self) -> SessionState:
        return replace(
            self,
            skipped=self.skipped + 1,
            batch_cursor=self.batch_cursor + 1,
        )

    def with_flag(self) -> SessionState:
        return replace(
            self,
            flagged=self.flagged + 1,
            batch_cursor=self.batch_cursor + 1,
        )

    def with_batch(self, batch: Iterable[str]) -> SessionState:
        return replace(
            self,
            current_batch=tuple(batch),
            batch_cursor=0,
        )

    def has_next(self) -> bool:
        return self.batch_cursor < len(self.current_batch)

    def next_sample_id(self) -> str | None:
        if not self.has_next():
            return None
        return self.current_batch[self.batch_cursor]

    def to_json(self) -> str:
        return json.dumps({
            "session_id": self.session_id,
            "annotator_id": self.annotator_id,
            "tenant_id": self.tenant_id,
            "task_id": self.task_id,
            "started_at": self.started_at,
            "labels_submitted": self.labels_submitted,
            "skipped": self.skipped,
            "flagged": self.flagged,
            "history": [
                {"sample_id": h.sample_id, "label": h.label,
                 "submitted_at": h.submitted_at, "time_ms": h.time_ms}
                for h in self.history
            ],
            "current_batch": list(self.current_batch),
            "batch_cursor": self.batch_cursor,
            "kappa_rolling": self.kappa_rolling,
        })

    @classmethod
    def from_json(cls, raw: str) -> SessionState:
        d = json.loads(raw)
        return cls(
            session_id=d["session_id"],
            annotator_id=d["annotator_id"],
            tenant_id=d["tenant_id"],
            task_id=d["task_id"],
            started_at=d["started_at"],
            labels_submitted=d["labels_submitted"],
            skipped=d["skipped"],
            flagged=d.get("flagged", 0),
            history=tuple(
                HistoryEntry(**h) for h in d.get("history", [])
            ),
            current_batch=tuple(d.get("current_batch", [])),
            batch_cursor=d.get("batch_cursor", 0),
            kappa_rolling=d.get("kappa_rolling", 0.0),
        )


class SessionStore(Protocol):
    """Minimal async API for session persistence."""

    async def get(self, session_id: str) -> SessionState: ...
    async def set(self, state: SessionState, ttl_s: int) -> None: ...
    async def delete(self, session_id: str) -> None: ...


class InMemorySessionStore:
    """Process-local backend — great for tests and single-pod dev."""

    def __init__(self) -> None:
        self._store: dict[str, tuple[float, SessionState]] = {}

    async def get(self, session_id: str) -> SessionState:
        pair = self._store.get(session_id)
        if pair is None:
            raise SessionExpired(f"session {session_id!r} not found")
        deadline, state = pair
        if deadline < time.monotonic():
            del self._store[session_id]
            raise SessionExpired(f"session {session_id!r} expired")
        return state

    async def set(self, state: SessionState, ttl_s: int) -> None:
        deadline = time.monotonic() + ttl_s
        self._store[state.session_id] = (deadline, state)

    async def delete(self, session_id: str) -> None:
        self._store.pop(session_id, None)


__all__ = [
    "HistoryEntry",
    "InMemorySessionStore",
    "SessionState",
    "SessionStore",
]
