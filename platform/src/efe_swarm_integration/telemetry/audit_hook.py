"""AuditHook — thin adapter over :class:`efe_security.audit.AuditStore`.

When M11 isn't imported, the hook degrades to a no-op.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Protocol

__all__ = ["AuditHook", "NoopAuditHook"]


class AuditHook(Protocol):
    async def emit(self, draft: Any) -> None: ...  # pragma: no cover


@dataclass(slots=True)
class NoopAuditHook:
    """Fallback when no audit sink is available."""

    async def emit(self, draft: Any) -> None:
        return None
