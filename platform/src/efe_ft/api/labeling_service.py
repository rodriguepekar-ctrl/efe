"""Labeling façade — wraps LabelerService with a transport-agnostic API."""

from __future__ import annotations

from collections.abc import Iterable
from dataclasses import dataclass

import numpy as np
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from efe_ft.db.engine import session_for_tenant
from efe_ft.labeling.active_learning import ALBatch
from efe_ft.labeling.escalation import EscalationService, PendingEscalation
from efe_ft.labeling.iaa import (
    pairwise_kappas,
)
from efe_ft.labeling.labeler_service import (
    FlagReason,
    LabelerService,
    LabelerSession,
    LabelSubmission,
)
from efe_ft.labeling.session import SessionStore


@dataclass(frozen=True, slots=True)
class AnnotatorStats:
    annotator_id: str
    labels_submitted: int
    avg_time_ms: float
    kappa_vs_peers: float


class LabelingFacade:
    """High-level labeling API."""

    def __init__(
        self,
        sessionmaker: async_sessionmaker[AsyncSession],
        *,
        session_store: SessionStore,
        class_set: Iterable[str],
        session_ttl_s: int = 1_800,
    ) -> None:
        self._sm = sessionmaker
        self._store = session_store
        self._class_set = frozenset(class_set)
        self._ttl = session_ttl_s

    async def start_session(
        self, *, annotator_id: str, tenant_id: str, task_id: str,
    ) -> LabelerSession:
        async with session_for_tenant(self._sm, tenant_id) as db:
            service = LabelerService(
                db, store=self._store,
                class_set=self._class_set, session_ttl_s=self._ttl,
            )
            return await service.start_session(
                annotator_id=annotator_id,
                tenant_id=tenant_id,
                task_id=task_id,
            )

    async def end_session(
        self, *, tenant_id: str, session_id: str,
    ) -> None:
        async with session_for_tenant(self._sm, tenant_id) as db:
            service = LabelerService(
                db, store=self._store, class_set=self._class_set,
                session_ttl_s=self._ttl,
            )
            await service.end_session(session_id)

    async def get_next_batch(
        self,
        *,
        tenant_id: str,
        session_id: str,
        pool_probs: np.ndarray,
        pool_embeddings: np.ndarray,
        pool_sample_ids: list[str],
        batch_size: int = 10,
        alpha: float = 0.7,
        rng_seed: int = 0,
    ) -> ALBatch:
        async with session_for_tenant(self._sm, tenant_id) as db:
            service = LabelerService(
                db, store=self._store, class_set=self._class_set,
                session_ttl_s=self._ttl,
            )
            return await service.get_next_batch(
                session_id=session_id,
                pool_probs=pool_probs,
                pool_embeddings=pool_embeddings,
                pool_sample_ids=pool_sample_ids,
                batch_size=batch_size, alpha=alpha, rng_seed=rng_seed,
            )

    async def submit_label(
        self,
        *,
        tenant_id: str,
        session_id: str,
        sample_id: str,
        label: str,
        time_ms: int = 0,
    ) -> LabelSubmission:
        async with session_for_tenant(self._sm, tenant_id) as db:
            service = LabelerService(
                db, store=self._store, class_set=self._class_set,
                session_ttl_s=self._ttl,
            )
            result = await service.submit_label(
                session_id=session_id, sample_id=sample_id,
                label=label, time_ms=time_ms,
            )
            await db.commit()
            return result

    async def flag_sample(
        self,
        *,
        tenant_id: str,
        session_id: str,
        sample_id: str,
        reason: FlagReason,
    ) -> LabelSubmission:
        async with session_for_tenant(self._sm, tenant_id) as db:
            service = LabelerService(
                db, store=self._store, class_set=self._class_set,
                session_ttl_s=self._ttl,
            )
            result = await service.flag(
                session_id=session_id, sample_id=sample_id, reason=reason,
            )
            await db.commit()
            return result

    async def escalation_queue(
        self, *, tenant_id: str, task_id: str, limit: int = 50,
    ) -> list[PendingEscalation]:
        async with session_for_tenant(self._sm, tenant_id) as db:
            esc = EscalationService(db)
            return await esc.queue(
                tenant_id=tenant_id, task_id=task_id, limit=limit,
            )

    async def resolve_escalation(
        self,
        *,
        tenant_id: str,
        sample_id: str,
        expert_id: str,
        label: str,
    ) -> str:
        async with session_for_tenant(self._sm, tenant_id) as db:
            esc = EscalationService(db)
            ref = await esc.resolve(
                tenant_id=tenant_id, sample_id=sample_id,
                expert_id=expert_id, label=label,
            )
            await db.commit()
            return ref

    async def annotator_stats(
        self, *, tenant_id: str, annotator_id: str,
    ) -> AnnotatorStats:
        from sqlalchemy import func, select

        from efe_ft.db.models import LabelHistory

        async with session_for_tenant(self._sm, tenant_id) as db:
            count_stmt = select(
                func.count(LabelHistory.id),
                func.coalesce(func.avg(LabelHistory.time_ms), 0.0),
            ).where(LabelHistory.annotator_id == annotator_id)
            row = (await db.execute(count_stmt)).one()
            # Compute kappa vs peers if overlap exists.
            peer_stmt = select(
                LabelHistory.sample_id,
                LabelHistory.annotator_id,
                LabelHistory.label_value,
            )
            rows = list((await db.execute(peer_stmt)).all())
        by_ann: dict[str, dict[str, str]] = {}
        for sid, ann, lbl in rows:
            by_ann.setdefault(ann, {})[sid] = lbl
        pair = pairwise_kappas(by_ann)
        peer_kappas = [
            r.kappa for (a, b), r in pair.items()
            if annotator_id in (a, b) and r.n_samples >= 5
        ]
        k = float(sum(peer_kappas) / len(peer_kappas)) if peer_kappas else 0.0
        return AnnotatorStats(
            annotator_id=annotator_id,
            labels_submitted=int(row[0]),
            avg_time_ms=float(row[1]),
            kappa_vs_peers=k,
        )


__all__ = ["AnnotatorStats", "LabelingFacade"]
