"""Champion registry — read/write the ``champions`` + ``champion_history`` tables.

The registry is the **authoritative** answer to "which model is
currently served for (tenant, task)?". M06 pulls it at startup +
every hint.

A promotion path goes:

    1. Upsert ``champions`` row (version += 1, metrics_snapshot refreshed).
    2. Append ``champion_history`` row (action=promote, from=prev).
    3. Record an AuditChain event (kind=decision_promote, signed).

A rollback reverses that:

    1. Read history, find the previous ``to_run_id``.
    2. Upsert ``champions`` back to that run.
    3. Append ``champion_history`` (action=rollback, from=current, to=prev).
    4. Record AuditChain event (kind=rollback).
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime
from typing import Any

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_gate.champion.audit import AuditChain, AuditKind
from efe_gate.clock import Clock, SystemClock
from efe_gate.db.models import Champion, ChampionHistory
from efe_gate.errors import (
    ConcurrentPromotion,
    NoChampion,
    NoPreviousChampion,
    ValidationError,
)
from efe_gate.ids import new_uuid


@dataclass(frozen=True, slots=True)
class ChampionInfo:
    tenant_id: str
    task_id: str
    artifact_uri: str
    run_id: str
    eval_run_id: str
    promoted_at: datetime
    promoted_by: str
    metrics_snapshot: dict[str, Any]
    version: int

    @classmethod
    def from_row(cls, row: Champion) -> ChampionInfo:
        return cls(
            tenant_id=row.tenant_id,
            task_id=row.task_id,
            artifact_uri=row.artifact_uri,
            run_id=row.run_id,
            eval_run_id=row.eval_run_id,
            promoted_at=row.promoted_at,
            promoted_by=row.promoted_by,
            metrics_snapshot=dict(row.metrics_snapshot or {}),
            version=row.version,
        )

    def as_dict(self) -> dict[str, Any]:
        return {
            "tenant_id": self.tenant_id,
            "task_id": self.task_id,
            "artifact_uri": self.artifact_uri,
            "run_id": self.run_id,
            "eval_run_id": self.eval_run_id,
            "promoted_at": self.promoted_at.isoformat(),
            "promoted_by": self.promoted_by,
            "metrics_snapshot": dict(self.metrics_snapshot),
            "version": self.version,
        }


class ChampionRegistry:
    """Promote / rollback / get operations."""

    def __init__(
        self,
        session: AsyncSession,
        *,
        audit_chain: AuditChain,
        clock: Clock | None = None,
    ) -> None:
        self._session = session
        self._audit = audit_chain
        self._clock: Clock = clock or SystemClock()

    # ---- read side -------------------------------------------------

    async def get(self, tenant_id: str, task_id: str) -> ChampionInfo:
        row = await self._fetch(tenant_id, task_id)
        if row is None:
            raise NoChampion(
                f"no champion for {tenant_id}/{task_id}",
            )
        return ChampionInfo.from_row(row)

    async def get_or_none(
        self, tenant_id: str, task_id: str,
    ) -> ChampionInfo | None:
        row = await self._fetch(tenant_id, task_id)
        return ChampionInfo.from_row(row) if row is not None else None

    async def history(
        self, tenant_id: str, task_id: str, *, limit: int = 50,
    ) -> list[ChampionHistory]:
        stmt = (
            select(ChampionHistory)
            .where(ChampionHistory.tenant_id == tenant_id)
            .where(ChampionHistory.task_id == task_id)
            .order_by(ChampionHistory.at.desc())
            .limit(limit)
        )
        return list((await self._session.execute(stmt)).scalars())

    # ---- write side ------------------------------------------------

    async def promote(
        self,
        *,
        tenant_id: str, task_id: str,
        run_id: str, artifact_uri: str,
        eval_run_id: str,
        metrics_snapshot: dict[str, Any],
        promoted_by: str,
        expected_version: int | None = None,
    ) -> ChampionInfo:
        """Replace the champion atomically.

        ``expected_version`` guards against concurrent promotions: if
        the currently-persisted version differs from
        ``expected_version``, we raise :class:`ConcurrentPromotion`
        (the caller should re-read and retry or escalate).
        """
        if not promoted_by:
            raise ValidationError("promoted_by is required")
        now = self._clock.now()
        row = await self._fetch(tenant_id, task_id)
        prev_run_id: str | None = None
        if row is None:
            if expected_version is not None and expected_version != 0:
                raise ConcurrentPromotion(
                    f"expected version {expected_version} but no champion "
                    f"exists for {tenant_id}/{task_id}",
                )
            row = Champion(
                tenant_id=tenant_id, task_id=task_id,
                artifact_uri=artifact_uri, run_id=run_id,
                eval_run_id=eval_run_id,
                promoted_at=now, promoted_by=promoted_by,
                metrics_snapshot=dict(metrics_snapshot),
                version=1,
            )
            self._session.add(row)
        else:
            if (
                expected_version is not None
                and row.version != expected_version
            ):
                raise ConcurrentPromotion(
                    f"version mismatch: expected {expected_version}, "
                    f"observed {row.version}",
                )
            prev_run_id = row.run_id
            row.artifact_uri = artifact_uri
            row.run_id = run_id
            row.eval_run_id = eval_run_id
            row.promoted_at = now
            row.promoted_by = promoted_by
            row.metrics_snapshot = dict(metrics_snapshot)
            row.version = row.version + 1

        hist = ChampionHistory(
            id=new_uuid(),
            tenant_id=tenant_id, task_id=task_id,
            from_run_id=prev_run_id, to_run_id=run_id,
            action="promote", actor=promoted_by,
            reason="M05 decision PROMOTE",
            at=now,
        )
        self._session.add(hist)

        entry = await self._audit.record(
            kind=AuditKind.DECISION_PROMOTE,
            actor=promoted_by,
            subject_id=run_id,
            payload={
                "tenant_id": tenant_id, "task_id": task_id,
                "from_run_id": prev_run_id, "to_run_id": run_id,
                "artifact_uri": artifact_uri,
                "metrics_snapshot": metrics_snapshot,
                "eval_run_id": eval_run_id,
                "version": row.version,
            },
        )
        hist.audit_hash = entry.this_hash
        await self._session.flush()
        return ChampionInfo.from_row(row)

    async def rollback(
        self,
        *,
        tenant_id: str, task_id: str,
        actor: str, reason: str = "auto-rollback",
    ) -> ChampionInfo:
        """Restore the previous champion.

        Raises :class:`NoPreviousChampion` when history has only the
        current run (nothing to fall back to).
        """
        if not actor:
            raise ValidationError("actor is required")
        row = await self._fetch(tenant_id, task_id)
        if row is None:
            raise NoChampion(
                f"no champion for {tenant_id}/{task_id}",
            )
        now = self._clock.now()
        # Find the most recent "promote" action whose from_run_id is
        # not None and differs from the current run.
        stmt = (
            select(ChampionHistory)
            .where(ChampionHistory.tenant_id == tenant_id)
            .where(ChampionHistory.task_id == task_id)
            .where(ChampionHistory.action == "promote")
            .order_by(ChampionHistory.at.desc())
            .limit(1)
        )
        last_promote = (
            await self._session.execute(stmt)
        ).scalar_one_or_none()
        if last_promote is None or last_promote.from_run_id is None:
            raise NoPreviousChampion(
                f"no previous champion to fall back to for "
                f"{tenant_id}/{task_id}",
            )
        prev_run_id = last_promote.from_run_id
        # Find the last promote that pointed to prev_run_id to recover
        # the metrics_snapshot + artifact_uri.
        stmt2 = (
            select(ChampionHistory)
            .where(ChampionHistory.tenant_id == tenant_id)
            .where(ChampionHistory.task_id == task_id)
            .where(ChampionHistory.to_run_id == prev_run_id)
            .where(ChampionHistory.action == "promote")
            .order_by(ChampionHistory.at.desc())
            .limit(1)
        )
        prev_promote = (
            await self._session.execute(stmt2)
        ).scalar_one_or_none()
        if prev_promote is None:
            raise NoPreviousChampion(
                f"history lacks a promote for run {prev_run_id!r}",
            )

        previous_run_artifact = getattr(
            prev_promote, "reason", "",  # placeholder
        )
        _ = previous_run_artifact
        # The actual artifact_uri + metrics of the previous champion
        # must live somewhere — we fetch it from the AuditEvent payload
        # of the original promote (audit chain is the authoritative
        # source of cold facts).
        from efe_gate.db.models import AuditEvent
        stmt3 = (
            select(AuditEvent)
            .where(AuditEvent.tenant_id == tenant_id)
            .where(AuditEvent.subject_id == prev_run_id)
            .where(AuditEvent.kind == AuditKind.DECISION_PROMOTE.value)
            .order_by(AuditEvent.created_at.desc())
            .limit(1)
        )
        audit_row = (await self._session.execute(stmt3)).scalar_one_or_none()
        if audit_row is None:
            raise NoPreviousChampion(
                f"no audit event for previous promote of "
                f"{prev_run_id!r}",
            )
        payload = dict(audit_row.payload or {})
        prev_metrics = dict(payload.get("metrics_snapshot") or {})
        # The original artifact URI must have been recorded on the
        # previous Champion row. We recover it from the last time
        # prev_run_id was active by querying audit payload —
        # payload may or may not include artifact_uri. As a fallback
        # we re-derive it from the history row's reason or raise.
        # To keep things correct we add a ``artifact_uri`` key in the
        # audit payload of promote — see promote() above. We extend:
        prev_artifact = payload.get("artifact_uri") or ""
        if not prev_artifact:
            # Extremely defensive — older rows may lack it. Block.
            raise NoPreviousChampion(
                "previous audit event missing artifact_uri; "
                "cannot rollback safely",
            )

        from_run_id = row.run_id
        row.artifact_uri = str(prev_artifact)
        row.run_id = str(prev_run_id)
        row.eval_run_id = str(payload.get("eval_run_id", row.eval_run_id))
        row.promoted_at = now
        row.promoted_by = actor
        row.metrics_snapshot = prev_metrics
        row.version = row.version + 1

        hist = ChampionHistory(
            id=new_uuid(),
            tenant_id=tenant_id, task_id=task_id,
            from_run_id=from_run_id, to_run_id=prev_run_id,
            action="rollback", actor=actor, reason=reason,
            at=now,
        )
        self._session.add(hist)
        entry = await self._audit.record(
            kind=AuditKind.ROLLBACK,
            actor=actor,
            subject_id=prev_run_id,
            payload={
                "tenant_id": tenant_id, "task_id": task_id,
                "from_run_id": from_run_id, "to_run_id": prev_run_id,
                "reason": reason,
                "artifact_uri": prev_artifact,
                "metrics_snapshot": prev_metrics,
            },
        )
        hist.audit_hash = entry.this_hash
        await self._session.flush()
        return ChampionInfo.from_row(row)

    # ---- helpers ---------------------------------------------------

    async def _fetch(
        self, tenant_id: str, task_id: str,
    ) -> Champion | None:
        return await self._session.get(Champion, (tenant_id, task_id))


__all__ = ["ChampionInfo", "ChampionRegistry"]
