"""Audit bridge — hands off rollback events to M05 audit chain.

Concretely, :class:`AuditChannel` is expected to be fulfilled by a
thin adapter that calls ``efe_gate.champion.audit.AuditChain.record``.
To keep M06 independent of M05 at import time, we only define the
Protocol + two test doubles here.
"""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field
from typing import Any, Protocol


class AuditChannel(Protocol):
    async def record_rollback(
        self,
        *,
        tenant_id: str,
        rollback_id: str,
        from_artifact_id: str,
        to_artifact_id: str,
        latency_ms: int,
        trigger_kind: str,
        actor: str,
        reason: str,
    ) -> None:  # pragma: no cover
        ...

    async def record_emergency_override(
        self,
        *,
        tenant_id: str,
        actor: str,
        reason: str,
        metadata: Mapping[str, Any] | None = None,
    ) -> None:  # pragma: no cover
        ...


@dataclass
class NullAuditChannel:
    async def record_rollback(self, **_kw: Any) -> None:
        return None

    async def record_emergency_override(self, **_kw: Any) -> None:
        return None


@dataclass
class RecordingAuditChannel:
    """Test double: stores every call for assertions."""

    rollbacks: list[dict[str, Any]] = field(default_factory=list)
    overrides: list[dict[str, Any]] = field(default_factory=list)

    async def record_rollback(self, **kw: Any) -> None:
        self.rollbacks.append(dict(kw))

    async def record_emergency_override(self, **kw: Any) -> None:
        self.overrides.append(dict(kw))


__all__ = [
    "AuditChannel",
    "NullAuditChannel",
    "RecordingAuditChannel",
]
