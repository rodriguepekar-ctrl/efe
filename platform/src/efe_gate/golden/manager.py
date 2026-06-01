"""Golden-set manager.

Golden sets are *created locked*: `snapshot_uri` + `content_hash`
+ `n_samples` + `label_balance` are all frozen at insert time and
**never mutated**. The only allowed later operation is
``deprecate`` (a soft flag that excludes the set from automatic
pick-up by new evaluations but keeps it queryable for reproducing
historical runs).

Content-hash scheme (same as M01 snapshots): the ID-tuple of the
labelled samples, sorted and SHA-256'd. Callers must supply the
pre-computed hash so the manager can verify uniqueness across the
tenant/task pair.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime
from typing import Any

from sqlalchemy import select, update
from sqlalchemy.ext.asyncio import AsyncSession

from efe_gate.clock import Clock, SystemClock
from efe_gate.db.models import GoldenSet
from efe_gate.errors import (
    GoldenSetDeprecated,
    GoldenSetError,
    GoldenSetLocked,
    NotFound,
    ValidationError,
)


def _coerce_bytes(v: Any) -> bytes:
    """Accept hex str, bytes, or bytearray → return 32-byte digest."""
    if isinstance(v, bytes):
        if len(v) != 32:
            raise ValidationError(
                f"content_hash must be 32 raw bytes, got {len(v)}",
            )
        return bytes(v)
    if isinstance(v, bytearray):
        return _coerce_bytes(bytes(v))
    if isinstance(v, str):
        if len(v) != 64:
            raise ValidationError(
                f"content_hash hex must be 64 chars, got {len(v)}",
            )
        try:
            return bytes.fromhex(v)
        except ValueError as exc:
            raise ValidationError(
                f"content_hash is not valid hex: {exc}",
            ) from exc
    raise ValidationError(
        f"unsupported content_hash type: {type(v).__name__}",
    )


@dataclass(frozen=True, slots=True)
class GoldenSetInfo:
    id: str
    tenant_id: str
    task_id: str
    snapshot_uri: str
    content_hash: str  # hex
    n_samples: int
    label_balance: dict[str, int]
    fairness_attrs: dict[str, Any]
    locked_at: datetime
    created_by: str
    deprecated: bool

    @classmethod
    def from_row(cls, row: GoldenSet) -> GoldenSetInfo:
        return cls(
            id=row.id,
            tenant_id=row.tenant_id,
            task_id=row.task_id,
            snapshot_uri=row.snapshot_uri,
            content_hash=row.content_hash.hex(),
            n_samples=row.n_samples,
            label_balance=dict(row.label_balance or {}),
            fairness_attrs=dict(row.fairness_attrs or {}),
            locked_at=row.locked_at,
            created_by=row.created_by,
            deprecated=row.deprecated,
        )


class GoldenSetManager:
    """CRUD + validation on :class:`GoldenSet` rows."""

    def __init__(
        self,
        session: AsyncSession,
        *,
        clock: Clock | None = None,
    ) -> None:
        self._session = session
        self._clock: Clock = clock or SystemClock()

    async def create(
        self,
        *,
        tenant_id: str,
        task_id: str,
        snapshot_uri: str,
        content_hash: Any,
        n_samples: int,
        label_balance: dict[str, int] | None = None,
        fairness_attrs: dict[str, Any] | None = None,
        created_by: str,
    ) -> GoldenSetInfo:
        if not tenant_id or not task_id:
            raise ValidationError("tenant_id and task_id are required")
        if not snapshot_uri:
            raise ValidationError("snapshot_uri is required")
        if n_samples < 1:
            raise ValidationError("n_samples must be >= 1")
        if not created_by:
            raise ValidationError("created_by is required")
        ch = _coerce_bytes(content_hash)
        row = GoldenSet(
            tenant_id=tenant_id,
            task_id=task_id,
            snapshot_uri=snapshot_uri,
            content_hash=ch,
            n_samples=int(n_samples),
            label_balance=dict(label_balance or {}),
            fairness_attrs=dict(fairness_attrs or {}),
            locked_at=self._clock.now(),
            created_by=created_by,
            deprecated=False,
        )
        self._session.add(row)
        try:
            await self._session.flush()
        except Exception as exc:  # integrity error on unique key
            raise GoldenSetError(
                f"golden set creation failed: {exc}",
            ) from exc
        return GoldenSetInfo.from_row(row)

    async def get(self, gs_id: str) -> GoldenSetInfo:
        row = await self._session.get(GoldenSet, gs_id)
        if row is None:
            raise NotFound(f"golden set {gs_id!r} not found")
        return GoldenSetInfo.from_row(row)

    async def latest_for(
        self, tenant_id: str, task_id: str,
        *, include_deprecated: bool = False,
    ) -> GoldenSetInfo | None:
        stmt = (
            select(GoldenSet)
            .where(GoldenSet.tenant_id == tenant_id)
            .where(GoldenSet.task_id == task_id)
        )
        if not include_deprecated:
            stmt = stmt.where(GoldenSet.deprecated.is_(False))
        stmt = stmt.order_by(GoldenSet.locked_at.desc()).limit(1)
        row = (await self._session.execute(stmt)).scalar_one_or_none()
        return GoldenSetInfo.from_row(row) if row is not None else None

    async def deprecate(self, gs_id: str, *, actor: str) -> GoldenSetInfo:
        if not actor:
            raise ValidationError("actor is required")
        row = await self._session.get(GoldenSet, gs_id)
        if row is None:
            raise NotFound(f"golden set {gs_id!r} not found")
        if row.deprecated:
            raise GoldenSetDeprecated(
                f"golden set {gs_id!r} already deprecated",
            )
        row.deprecated = True
        await self._session.flush()
        return GoldenSetInfo.from_row(row)

    async def assert_mutation_blocked(
        self, gs_id: str, *, field: str,
    ) -> None:
        """Fail loudly when any caller tries to mutate a locked column."""
        _ = await self.get(gs_id)
        raise GoldenSetLocked(
            f"cannot mutate {field!r} on golden set {gs_id!r}: "
            f"content-hashed and locked at creation",
        )

    async def list_for(
        self,
        tenant_id: str,
        task_id: str | None = None,
        *,
        include_deprecated: bool = False,
    ) -> list[GoldenSetInfo]:
        stmt = (
            select(GoldenSet)
            .where(GoldenSet.tenant_id == tenant_id)
        )
        if task_id is not None:
            stmt = stmt.where(GoldenSet.task_id == task_id)
        if not include_deprecated:
            stmt = stmt.where(GoldenSet.deprecated.is_(False))
        stmt = stmt.order_by(GoldenSet.locked_at.desc())
        rows = list((await self._session.execute(stmt)).scalars())
        return [GoldenSetInfo.from_row(r) for r in rows]


# Re-export helper for test convenience
coerce_hex_like_input = _coerce_bytes


__all__ = [
    "GoldenSetInfo",
    "GoldenSetManager",
    "coerce_hex_like_input",
]


# Silence unused imports used only for runtime helpers
_ = (update, )
