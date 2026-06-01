"""Audit JSONL sink.

Append-only log suitable for shipping to S3 / object store. Each line is
a canonical JSON of an :class:`~efe_scheduling.telemetry.events.Event`,
plus a hex-encoded seal. PII fields are expected to have been replaced
with digests upstream.
"""

from __future__ import annotations

import asyncio
import json
from dataclasses import dataclass, field
from typing import Any, Protocol

from efe_scheduling.telemetry.events import Event

__all__ = ["AuditSink", "InMemoryAuditSink", "write_audit"]


class AuditSink(Protocol):
    async def write(self, line: bytes) -> None: ...  # pragma: no cover


@dataclass(slots=True)
class InMemoryAuditSink:
    lines: list[bytes] = field(default_factory=list)
    _lock: asyncio.Lock = field(default_factory=asyncio.Lock, repr=False)

    async def write(self, line: bytes) -> None:
        async with self._lock:
            self.lines.append(line)

    async def read_all(self) -> list[dict[str, Any]]:
        async with self._lock:
            return [json.loads(line.decode("utf-8")) for line in self.lines]


async def write_audit(sink: AuditSink, event: Event) -> None:
    line = event.to_json() + b"\n"
    await sink.write(line)
