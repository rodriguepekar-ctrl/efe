"""FeatureStore service façade — mirrors the 8 RPCs of the proto.

This is a plain async class, not a real gRPC servicer — it is easy to
wrap for gRPC or HTTP. Keeping it transport-agnostic keeps it
trivially testable from Python.
"""

from __future__ import annotations

from collections.abc import AsyncIterator, Iterable
from dataclasses import dataclass
from typing import Literal

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from efe_ft.audit import AuditChain
from efe_ft.db.engine import session_for_tenant
from efe_ft.db.models import Snapshot, SnapshotMember, TrainingSample
from efe_ft.errors import FtError
from efe_ft.ingest.dedup import Deduper
from efe_ft.ingest.pipeline import IngestInput, IngestOutcome, IngestPipeline
from efe_ft.ingest.schemas import SampleIngestPayload
from efe_ft.snapshots import MaterialisationSpec, Partitioning, materialise


@dataclass(frozen=True, slots=True)
class SampleBrief:
    id: str
    tenant_id: str
    task_id: str
    text: str
    label: str | None
    status: str


class FeatureStoreFacade:
    """High-level async API consumed by HTTP handlers and tests."""

    def __init__(
        self,
        sessionmaker: async_sessionmaker[AsyncSession],
        *,
        deduper: Deduper,
        pii_policy: Literal["mask", "reject"] = "mask",
        balance_skew_max: float = 50.0,
    ) -> None:
        self._sm = sessionmaker
        self._deduper = deduper
        self._pii_policy = pii_policy
        self._balance_skew_max = balance_skew_max

    # ------------------------------------------------------- ingestion

    async def ingest_sample(
        self, payload: SampleIngestPayload,
    ) -> IngestOutcome:
        return await self.ingest_batch([payload])

    async def ingest_batch(
        self, payloads: Iterable[SampleIngestPayload],
    ) -> IngestOutcome:
        items = list(payloads)
        if not items:
            raise FtError("empty ingest batch")
        first = items[0]
        # Ensure all items belong to the same tenant/task — API layer typically
        # dispatches one tenant at a time.
        for p in items:
            if p.tenant_id != first.tenant_id or p.task_id != first.task_id:
                raise FtError("mixed tenant/task in a single batch")
        inputs = [IngestInput(payload=p) for p in items]
        async with session_for_tenant(self._sm, first.tenant_id,
                                      enable_rls=True) as session:
            pipeline = IngestPipeline(
                session,
                deduper=self._deduper,
                pii_policy=self._pii_policy,
                balance_skew_max=self._balance_skew_max,
            )
            outcome = await pipeline.run(
                inputs,
                tenant_id=first.tenant_id,
                task_id=first.task_id,
                actor_id=first.actor_id,
                batch_source=first.source,
            )
            await session.commit()
        return outcome

    # ------------------------------------------------------- listing

    async def list_samples(
        self,
        *,
        tenant_id: str,
        task_id: str,
        status: str | None = None,
        page_size: int = 100,
        page_token: str | None = None,
    ) -> tuple[list[SampleBrief], str | None]:
        async with session_for_tenant(self._sm, tenant_id,
                                      enable_rls=True) as session:
            stmt = select(TrainingSample).where(
                TrainingSample.tenant_id == tenant_id,
                TrainingSample.task_id == task_id,
            )
            if status is not None:
                stmt = stmt.where(TrainingSample.status == status)
            if page_token is not None:
                stmt = stmt.where(TrainingSample.id > page_token)
            stmt = stmt.order_by(TrainingSample.id.asc()).limit(page_size + 1)
            rows = list((await session.execute(stmt)).scalars())
        next_token: str | None = None
        if len(rows) > page_size:
            next_token = rows[page_size].id
            rows = rows[:page_size]
        briefs = [
            SampleBrief(
                id=r.id, tenant_id=r.tenant_id, task_id=r.task_id,
                text=r.text, label=r.label, status=r.status,
            )
            for r in rows
        ]
        return briefs, next_token

    # ------------------------------------------------------- snapshots

    async def materialise_snapshot(
        self,
        *,
        tenant_id: str,
        task_id: str,
        actor_id: str = "system",
        status_filter: Literal["labelled", "pending", "any"] = "labelled",
        partitioning: Partitioning | None = None,
        min_samples: int = 10,
    ) -> Snapshot:
        spec = MaterialisationSpec(
            tenant_id=tenant_id, task_id=task_id, actor_id=actor_id,
            status_filter=status_filter,
            partitioning=partitioning,
            min_samples=min_samples,
        )
        async with session_for_tenant(self._sm, tenant_id,
                                      enable_rls=True) as session:
            snap = await materialise(session, spec)
            await session.commit()
            # Detach from session to allow post-commit access.
            session.expunge(snap)
        return snap

    async def get_snapshot(
        self, *, snapshot_id: str,
    ) -> Snapshot | None:
        async with self._sm() as session:
            snap = await session.get(Snapshot, snapshot_id)
            if snap is not None:
                session.expunge(snap)
        return snap

    async def stream_snapshot_members(
        self, *, snapshot_id: str,
    ) -> AsyncIterator[SnapshotMember]:
        async with self._sm() as session:
            stmt = (
                select(SnapshotMember)
                .where(SnapshotMember.snapshot_id == snapshot_id)
                .order_by(SnapshotMember.sample_id.asc())
            )
            result = await session.execute(stmt)
            for member in result.scalars():
                session.expunge(member)
                yield member

    # ------------------------------------------------------- delete

    async def delete_sample(
        self,
        *,
        tenant_id: str,
        sample_id: str,
        actor_id: str,
        reason: str,
    ) -> bool:
        async with session_for_tenant(self._sm, tenant_id,
                                      enable_rls=True) as session:
            sample = await session.get(TrainingSample, sample_id)
            if sample is None or sample.tenant_id != tenant_id:
                return False
            # Refuse if referenced by a non-archived snapshot (RESTRICT FK).
            # In SQLite, an FK violation raises only at commit time; we pre-
            # check to emit a clean error.
            members_stmt = select(SnapshotMember).where(
                SnapshotMember.sample_id == sample_id,
            )
            member = (await session.execute(members_stmt)).scalars().first()
            if member is not None:
                raise FtError(
                    f"sample {sample_id!r} is referenced by snapshot "
                    f"{member.snapshot_id!r}; cannot delete",
                )
            chain = AuditChain(session, tenant_id)
            await chain.record(
                kind="sample_deleted",
                actor_id=actor_id,
                subject_id=sample_id,
                payload={"reason": reason},
            )
            await session.delete(sample)
            await session.commit()
        return True


__all__ = ["FeatureStoreFacade", "SampleBrief"]
