"""Labeler service orchestrator.

Consumes a pool of candidate samples from M01, runs the active-
learning sampler, hands out batches, records labels back into M01,
and triggers escalation on dual-label disagreements.
"""

from __future__ import annotations

import time
import uuid
from collections.abc import Iterable
from dataclasses import dataclass
from typing import Literal

import numpy as np
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_ft.audit import AuditChain
from efe_ft.db.models import LabelHistory, TrainingSample
from efe_ft.errors import (
    LabelingError,
    SampleNotAssigned,
    UnknownLabel,
)
from efe_ft.labeling.active_learning import (
    ALBatch,
    Candidate,
    hybrid_select,
)
from efe_ft.labeling.session import (
    HistoryEntry,
    SessionState,
    SessionStore,
)

FlagReason = Literal["skip", "unclear", "harmful", "pii"]


@dataclass(frozen=True, slots=True)
class LabelerSession:
    """Return value of :meth:`LabelerService.start_session`."""

    session_id: str
    annotator_id: str
    tenant_id: str
    task_id: str


@dataclass(frozen=True, slots=True)
class LabelSubmission:
    """Server-side outcome of :meth:`LabelerService.submit_label`."""

    accepted: bool
    triggered_escalation: bool = False
    escalation_ref: str | None = None
    labels_submitted: int = 0
    next_sample_id: str | None = None


class LabelerService:
    """Stateful service — holds a :class:`SessionStore` reference."""

    def __init__(
        self,
        session: AsyncSession,
        *,
        store: SessionStore,
        class_set: Iterable[str],
        session_ttl_s: int = 1800,
    ) -> None:
        self._session = session
        self._store = store
        self._class_set = frozenset(class_set)
        self._ttl = session_ttl_s

    # ------------------------------------------------------- session API

    async def start_session(
        self, *, annotator_id: str, tenant_id: str, task_id: str,
    ) -> LabelerSession:
        session_id = f"sess_{uuid.uuid4().hex[:22]}"
        state = SessionState(
            session_id=session_id,
            annotator_id=annotator_id,
            tenant_id=tenant_id,
            task_id=task_id,
            started_at=time.time(),
            labels_submitted=0,
            skipped=0,
            flagged=0,
            history=(),
            current_batch=(),
            batch_cursor=0,
            kappa_rolling=0.0,
        )
        await self._store.set(state, self._ttl)
        return LabelerSession(
            session_id=session_id, annotator_id=annotator_id,
            tenant_id=tenant_id, task_id=task_id,
        )

    async def end_session(self, session_id: str) -> None:
        await self._store.delete(session_id)

    # ------------------------------------------------------- batch API

    async def get_next_batch(
        self,
        *,
        session_id: str,
        pool_probs: np.ndarray,
        pool_embeddings: np.ndarray,
        pool_sample_ids: list[str],
        batch_size: int = 10,
        alpha: float = 0.7,
        rng_seed: int = 0,
    ) -> ALBatch:
        """Refresh the session's current_batch with AL-selected candidates."""
        state = await self._store.get(session_id)
        if len(pool_sample_ids) != pool_probs.shape[0]:
            raise LabelingError(
                "pool_sample_ids length != pool_probs rows",
            )
        rng = np.random.default_rng(rng_seed)
        chosen = hybrid_select(
            pool_probs=pool_probs,
            pool_embeddings=pool_embeddings,
            batch_size=batch_size,
            alpha=alpha,
            rng=rng,
        )
        candidates: list[Candidate] = []
        from efe_ft.labeling.active_learning import entropy

        for i in chosen:
            idx = int(i)
            candidates.append(Candidate(
                sample_id=pool_sample_ids[idx],
                uncertainty=entropy(pool_probs[idx]),
            ))
        new_state = state.with_batch(c.sample_id for c in candidates)
        await self._store.set(new_state, self._ttl)
        return ALBatch(
            candidates=tuple(candidates),
            alpha=alpha,
            batch_size=batch_size,
        )

    # ------------------------------------------------------- labeling API

    async def submit_label(
        self,
        *,
        session_id: str,
        sample_id: str,
        label: str,
        time_ms: int = 0,
    ) -> LabelSubmission:
        if label not in self._class_set:
            raise UnknownLabel(
                f"label {label!r} not in task class set",
                detail={"label": label},
            )
        state = await self._store.get(session_id)
        if state.next_sample_id() != sample_id:
            raise SampleNotAssigned(
                f"sample {sample_id!r} not current in session",
                detail={"current": state.next_sample_id()},
            )
        row = await self._session.get(TrainingSample, sample_id)
        if row is None:
            raise LabelingError(f"sample {sample_id!r} not found")
        if row.tenant_id != state.tenant_id:
            raise LabelingError("cross-tenant label submission blocked")

        # Determine round: 1 = first labeler on this sample, 2 = dual-
        # labeler (a DIFFERENT non-expert annotator). A re-label by the
        # SAME annotator keeps the same round — we count *distinct*
        # non-expert annotators that have touched this sample before.
        prior_stmt = select(LabelHistory).where(
            LabelHistory.sample_id == sample_id,
        )
        prior_rows = list((await self._session.execute(prior_stmt)).scalars())
        distinct_prior = {
            r.annotator_id for r in prior_rows
            if not r.is_expert and r.annotator_id != state.annotator_id
        }
        round_num = 1 + len(distinct_prior)
        round_num = min(round_num, 2)  # expert path uses round=3 explicitly

        hist = LabelHistory(
            id=str(uuid.uuid4()),
            sample_id=sample_id,
            annotator_id=state.annotator_id,
            label_value=label,
            confidence=1.0,
            round=round_num,
            is_expert=False,
            time_ms=time_ms,
        )
        self._session.add(hist)

        # Update canonical label: use latest non-expert label when no expert present.
        row.label = label
        row.status = "labelled"

        # Dual-label disagreement triggers escalation.
        triggered = False
        escalation_ref: str | None = None
        other = [r for r in prior_rows if r.annotator_id != state.annotator_id and not r.is_expert]
        if other and any(r.label_value != label for r in other):
            triggered = True
            escalation_ref = f"esc_{uuid.uuid4().hex[:10]}"
            # Pour la simplicité on stocke juste dans l'audit ;
            # le service Escalation dédié (M02 §12) consomme l'audit.

        chain = AuditChain(self._session, state.tenant_id)
        await chain.record(
            kind="label_written",
            actor_id=state.annotator_id,
            subject_id=sample_id,
            payload={
                "label": label,
                "round": round_num,
                "time_ms": time_ms,
                "escalated": triggered,
                "escalation_ref": escalation_ref,
            },
        )
        await self._session.flush()

        entry = HistoryEntry(
            sample_id=sample_id,
            label=label,
            submitted_at=time.time(),
            time_ms=time_ms,
        )
        new_state = state.with_label(entry)
        await self._store.set(new_state, self._ttl)

        return LabelSubmission(
            accepted=True,
            triggered_escalation=triggered,
            escalation_ref=escalation_ref,
            labels_submitted=new_state.labels_submitted,
            next_sample_id=new_state.next_sample_id(),
        )

    async def flag(
        self,
        *,
        session_id: str,
        sample_id: str,
        reason: FlagReason,
    ) -> LabelSubmission:
        state = await self._store.get(session_id)
        if state.next_sample_id() != sample_id:
            raise SampleNotAssigned(
                f"sample {sample_id!r} not current in session",
            )
        chain = AuditChain(self._session, state.tenant_id)
        if reason == "skip":
            new_state = state.with_skip()
            kind = "label_flagged_unclear"  # reuse category
        elif reason == "harmful" or reason == "pii":
            row = await self._session.get(TrainingSample, sample_id)
            if row is not None:
                row.status = "archived"
            new_state = state.with_flag()
            kind = "label_flagged_harmful"
        else:
            new_state = state.with_flag()
            kind = "label_flagged_unclear"
        await chain.record(
            kind=kind,
            actor_id=state.annotator_id,
            subject_id=sample_id,
            payload={"reason": reason},
        )
        await self._session.flush()
        await self._store.set(new_state, self._ttl)
        return LabelSubmission(
            accepted=True, next_sample_id=new_state.next_sample_id(),
            labels_submitted=new_state.labels_submitted,
        )


__all__ = [
    "FlagReason",
    "LabelSubmission",
    "LabelerService",
    "LabelerSession",
]
