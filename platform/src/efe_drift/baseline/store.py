"""Baseline store — Protocol + in-memory implementation.

The store is the persistence boundary for L3. It speaks in
:class:`~efe_drift.records.Baseline` objects, never in ORM rows. The
manager layer (:mod:`efe_drift.baseline.manager`) sits on top and
composes the store with the FSM, validator, seal, and rotation
policy.

Concrete implementations :

* :class:`InMemoryBaselineStore` — dict of dicts, used in tests and
  single-process dev.
* :class:`SqlBaselineStore` (post-MVP) — SQLAlchemy implementation
  mapping to :class:`BaselineRow` + :class:`BaselineStateTransitionRow`.

Concurrency : every mutation holds an :class:`asyncio.Lock` so
concurrent promotions / rotations cannot race on the "≤ 1 active
per scope" invariant.
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, field, replace
from datetime import datetime
from typing import Any, Protocol

from efe_drift.errors import (
    BaselineImmutable,
    BaselineNotFound,
    BaselineStateVersionConflict,
)
from efe_drift.ids import new_uuid
from efe_drift.records import Baseline, BaselineSpec, BaselineState

# ---------------------------------------------------------------------------
# Store Protocol
# ---------------------------------------------------------------------------


class BaselineStore(Protocol):
    async def create(
        self, *, spec: BaselineSpec, state: BaselineState, now: datetime,
    ) -> Baseline:  # pragma: no cover
        ...

    async def get(self, baseline_id: str) -> Baseline:  # pragma: no cover
        ...

    async def get_active(
        self, *, tenant_id: str, task_id: str, feature_name: str,
    ) -> Baseline | None:  # pragma: no cover
        ...

    async def list(
        self,
        *,
        tenant_id: str | None = None,
        task_id: str | None = None,
        state: BaselineState | None = None,
    ) -> tuple[Baseline, ...]:  # pragma: no cover
        ...

    async def transition(
        self,
        baseline_id: str,
        *,
        from_state: BaselineState,
        to_state: BaselineState,
        expected_version: int,
        now: datetime,
    ) -> Baseline:  # pragma: no cover
        ...

    async def seal(
        self,
        baseline_id: str,
        *,
        spec_hash: bytes,
        signing_key_id: str,
        signature: bytes,
        expected_version: int,
    ) -> Baseline:  # pragma: no cover
        ...

    async def set_planned_end(
        self,
        baseline_id: str,
        *,
        planned_end_at: datetime,
        expected_version: int,
    ) -> Baseline:  # pragma: no cover
        ...

    async def record_transition(
        self,
        *,
        baseline_id: str,
        from_state: BaselineState,
        to_state: BaselineState,
        actor: str,
        reason: str | None,
        now: datetime,
    ) -> None:  # pragma: no cover
        ...

    async def transitions_for(
        self, baseline_id: str,
    ) -> tuple[TransitionAuditEntry, ...]:  # pragma: no cover
        ...


@dataclass(frozen=True, slots=True)
class TransitionAuditEntry:
    transition_id: str
    baseline_id: str
    from_state: BaselineState
    to_state: BaselineState
    actor: str
    reason: str | None
    occurred_at: datetime


# ---------------------------------------------------------------------------
# InMemoryBaselineStore
# ---------------------------------------------------------------------------


@dataclass(slots=True)
class _Cell:
    baseline: Baseline
    transitions: list[TransitionAuditEntry] = field(default_factory=list)


class InMemoryBaselineStore:
    """Async-lock-guarded in-memory implementation."""

    __slots__ = ("_cells", "_lock")

    def __init__(self) -> None:
        self._cells: dict[str, _Cell] = {}
        self._lock = asyncio.Lock()

    # ---- CRUD ----------------------------------------------------------

    async def create(
        self, *, spec: BaselineSpec, state: BaselineState, now: datetime,
    ) -> Baseline:
        async with self._lock:
            baseline_id = new_uuid()
            baseline = Baseline(
                baseline_id=baseline_id,
                spec=spec,
                state=state,
                state_version=0,
                created_at=now,
            )
            self._cells[baseline_id] = _Cell(baseline=baseline)
            return baseline

    async def get(self, baseline_id: str) -> Baseline:
        cell = self._cells.get(baseline_id)
        if cell is None:
            raise BaselineNotFound(
                f"unknown baseline_id {baseline_id!r}",
            )
        return cell.baseline

    async def get_active(
        self, *, tenant_id: str, task_id: str, feature_name: str,
    ) -> Baseline | None:
        # Newest ACTIVE (should be at most one per scope, but we
        # defend against drift by picking the most-recently promoted).
        candidates = [
            cell.baseline
            for cell in self._cells.values()
            if cell.baseline.state == "active"
            and cell.baseline.spec.tenant_id == tenant_id
            and cell.baseline.spec.task_id == task_id
            and cell.baseline.spec.feature_name == feature_name
        ]
        if not candidates:
            return None
        return max(
            candidates,
            key=lambda b: (b.promoted_at or b.created_at or datetime.min),
        )

    async def list(
        self,
        *,
        tenant_id: str | None = None,
        task_id: str | None = None,
        state: BaselineState | None = None,
    ) -> tuple[Baseline, ...]:
        rows = tuple(cell.baseline for cell in self._cells.values())
        if tenant_id is not None:
            rows = tuple(b for b in rows if b.spec.tenant_id == tenant_id)
        if task_id is not None:
            rows = tuple(b for b in rows if b.spec.task_id == task_id)
        if state is not None:
            rows = tuple(b for b in rows if b.state == state)
        return rows

    async def transition(
        self,
        baseline_id: str,
        *,
        from_state: BaselineState,
        to_state: BaselineState,
        expected_version: int,
        now: datetime,
    ) -> Baseline:
        async with self._lock:
            cell = self._cells.get(baseline_id)
            if cell is None:
                raise BaselineNotFound(
                    f"unknown baseline_id {baseline_id!r}",
                )
            current = cell.baseline
            if current.state != from_state:
                raise BaselineStateVersionConflict(
                    f"expected state {from_state!r}, got {current.state!r}",
                )
            if current.state_version != expected_version:
                raise BaselineStateVersionConflict(
                    f"expected version {expected_version}, "
                    f"got {current.state_version}",
                )
            updates: dict[str, Any] = {
                "state": to_state,
                "state_version": current.state_version + 1,
            }
            if to_state == "active" and current.promoted_at is None:
                updates["promoted_at"] = now
            if to_state == "retired":
                updates["retired_at"] = now
            updated = replace(current, **updates)
            cell.baseline = updated
            return updated

    async def seal(
        self,
        baseline_id: str,
        *,
        spec_hash: bytes,
        signing_key_id: str,
        signature: bytes,
        expected_version: int,
    ) -> Baseline:
        async with self._lock:
            cell = self._cells.get(baseline_id)
            if cell is None:
                raise BaselineNotFound(
                    f"unknown baseline_id {baseline_id!r}",
                )
            current = cell.baseline
            if current.state == "active":
                raise BaselineImmutable(
                    "cannot re-seal an ACTIVE baseline (invariant I4)",
                )
            if current.state_version != expected_version:
                raise BaselineStateVersionConflict(
                    f"expected version {expected_version}, "
                    f"got {current.state_version}",
                )
            updated = replace(
                current,
                spec_hash=spec_hash,
                signing_key_id=signing_key_id,
                signature=signature,
                state_version=current.state_version + 1,
            )
            cell.baseline = updated
            return updated

    async def set_planned_end(
        self,
        baseline_id: str,
        *,
        planned_end_at: datetime,
        expected_version: int,
    ) -> Baseline:
        async with self._lock:
            cell = self._cells.get(baseline_id)
            if cell is None:
                raise BaselineNotFound(
                    f"unknown baseline_id {baseline_id!r}",
                )
            current = cell.baseline
            if current.state_version != expected_version:
                raise BaselineStateVersionConflict(
                    f"expected version {expected_version}, "
                    f"got {current.state_version}",
                )
            updated = replace(
                current,
                planned_end_at=planned_end_at,
                state_version=current.state_version + 1,
            )
            cell.baseline = updated
            return updated

    async def record_transition(
        self,
        *,
        baseline_id: str,
        from_state: BaselineState,
        to_state: BaselineState,
        actor: str,
        reason: str | None,
        now: datetime,
    ) -> None:
        async with self._lock:
            cell = self._cells.get(baseline_id)
            if cell is None:
                raise BaselineNotFound(
                    f"unknown baseline_id {baseline_id!r}",
                )
            cell.transitions.append(TransitionAuditEntry(
                transition_id=new_uuid(),
                baseline_id=baseline_id,
                from_state=from_state,
                to_state=to_state,
                actor=actor,
                reason=reason,
                occurred_at=now,
            ))

    async def transitions_for(
        self, baseline_id: str,
    ) -> tuple[TransitionAuditEntry, ...]:
        cell = self._cells.get(baseline_id)
        if cell is None:
            raise BaselineNotFound(
                f"unknown baseline_id {baseline_id!r}",
            )
        return tuple(cell.transitions)


__all__ = [
    "BaselineStore",
    "InMemoryBaselineStore",
    "TransitionAuditEntry",
]
