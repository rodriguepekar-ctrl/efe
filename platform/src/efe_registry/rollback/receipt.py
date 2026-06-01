"""Receipts returned by :class:`RollbackExecutor`."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal


@dataclass(frozen=True, slots=True)
class RollbackReceipt:
    rollback_id: str
    tenant_id: str
    task_id: str
    from_artifact_id: str
    to_artifact_id: str
    latency_ms: int
    final_state: Literal["completed", "partial", "failed"]
    phases: dict[str, float]


__all__ = ["RollbackReceipt"]
