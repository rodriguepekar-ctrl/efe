"""Expert-adjudication escalation service.

When two non-expert annotators disagree on a dual-label pass, the
system stores a reference (via audit chain) and the expert UI picks
them up. :func:`resolve` writes a round=3 ``is_expert=True`` label,
which becomes the canonical answer.
"""

from __future__ import annotations

import uuid
from dataclasses import dataclass

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_ft.audit import AuditChain
from efe_ft.db.models import LabelHistory, TrainingSample
from efe_ft.errors import LabelingError


@dataclass(frozen=True, slots=True)
class PendingEscalation:
    sample_id: str
    tenant_id: str
    task_id: str
    labels_proposed: tuple[tuple[str, str], ...]  # (annotator_id, label_value)
    text_preview: str

    def to_dict(self) -> dict[str, object]:
        return {
            "sample_id": self.sample_id,
            "tenant_id": self.tenant_id,
            "task_id": self.task_id,
            "labels_proposed": [list(p) for p in self.labels_proposed],
            "text_preview": self.text_preview,
        }


class EscalationService:
    """Discover disagreements and apply expert resolutions."""

    def __init__(self, session: AsyncSession) -> None:
        self._session = session

    async def queue(
        self, *, tenant_id: str, task_id: str, limit: int = 50,
    ) -> list[PendingEscalation]:
        """Return samples with 2+ non-expert labels and ≥1 disagreement.

        Implementation uses a portable two-phase approach: fetch all
        non-expert labels and filter in Python. This avoids a complex
        SQL ``HAVING COUNT(DISTINCT ...) > 1`` that SQLite + SQLAlchemy
        typing do not render cleanly. For Postgres-scale volumes a
        phase-2 optimisation with a dedicated index + CTE is planned.
        """
        # Load ALL history rows (expert + non-expert) so we can detect
        # samples that have already been resolved. Filter to non-expert
        # entries in Python for the "proposals" list.
        stmt = (
            select(LabelHistory)
            .order_by(LabelHistory.created_at.asc())
        )
        rows = list((await self._session.execute(stmt)).scalars())
        by_sample: dict[str, list[LabelHistory]] = {}
        for r in rows:
            by_sample.setdefault(r.sample_id, []).append(r)

        out: list[PendingEscalation] = []
        for sample_id, all_entries in by_sample.items():
            # Already resolved by an expert → no longer pending.
            if any(e.is_expert for e in all_entries):
                continue
            entries = [e for e in all_entries if not e.is_expert]
            # Need at least 2 DISTINCT non-expert annotators to have
            # a disagreement. Filter by unique annotator_id to ignore
            # re-labels by the same person.
            by_ann: dict[str, LabelHistory] = {}
            for e in entries:
                # Keep the latest label per annotator.
                by_ann[e.annotator_id] = e
            if len(by_ann) < 2:
                continue
            labels = {e.label_value for e in by_ann.values()}
            if len(labels) < 2:
                continue
            sample = await self._session.get(TrainingSample, sample_id)
            if sample is None:
                continue
            if sample.tenant_id != tenant_id or sample.task_id != task_id:
                continue
            proposals = tuple(
                (e.annotator_id, e.label_value) for e in by_ann.values()
            )
            out.append(PendingEscalation(
                sample_id=sample_id,
                tenant_id=sample.tenant_id,
                task_id=sample.task_id,
                labels_proposed=proposals,
                text_preview=sample.text[:160],
            ))
            if len(out) >= limit:
                break
        return out

    async def resolve(
        self,
        *,
        tenant_id: str,
        sample_id: str,
        expert_id: str,
        label: str,
    ) -> str:
        """Write the expert decision and update the canonical label."""
        sample = await self._session.get(TrainingSample, sample_id)
        if sample is None:
            raise LabelingError(f"sample {sample_id!r} not found")
        if sample.tenant_id != tenant_id:
            raise LabelingError("cross-tenant expert resolution blocked")
        prev_label = sample.label

        hist = LabelHistory(
            id=str(uuid.uuid4()),
            sample_id=sample_id,
            annotator_id=expert_id,
            label_value=label,
            confidence=1.0,
            round=3,
            is_expert=True,
            time_ms=0,
        )
        self._session.add(hist)
        sample.label = label
        sample.status = "labelled"

        chain = AuditChain(self._session, tenant_id)
        await chain.record(
            kind="label_overridden",
            actor_id=expert_id,
            subject_id=sample_id,
            payload={
                "prev_label": prev_label,
                "new_label": label,
                "expert_id": expert_id,
            },
        )
        await self._session.flush()
        return hist.id


__all__ = ["EscalationService", "PendingEscalation"]
