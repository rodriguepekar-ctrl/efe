"""Per-task class-balance invariant check.

The rule: ``max(class_counts) / min(class_counts) ≤ balance_skew_max``.
Violating batches are rejected by default; a tenant may override with
an explicit ``X-Accept-Skew: true`` header — at which point the audit
logs the acceptance under ``kind='balance_skew_overridden'`` (opt in).
"""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass

from sqlalchemy import func, select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_ft.db.models import TrainingSample


@dataclass(frozen=True, slots=True)
class BalanceReport:
    """Snapshot of the label distribution at check time."""

    counts: Mapping[str, int]
    skew: float
    n_classes: int
    n_samples: int

    def is_healthy(self, threshold: float) -> bool:
        return self.n_samples == 0 or self.skew <= threshold


async def balance_skew_check(
    session: AsyncSession,
    tenant_id: str,
    task_id: str,
) -> BalanceReport:
    """Compute the current label distribution + skew for a task."""
    stmt = (
        select(TrainingSample.label, func.count(TrainingSample.id))
        .where(
            TrainingSample.tenant_id == tenant_id,
            TrainingSample.task_id == task_id,
            TrainingSample.label.is_not(None),
            TrainingSample.status != "archived",
        )
        .group_by(TrainingSample.label)
    )
    result = await session.execute(stmt)
    counts: dict[str, int] = {
        row[0]: int(row[1]) for row in result.all()
    }
    if not counts:
        return BalanceReport(counts={}, skew=0.0, n_classes=0, n_samples=0)
    values = list(counts.values())
    skew = max(values) / max(1, min(values))
    return BalanceReport(
        counts=counts,
        skew=skew,
        n_classes=len(counts),
        n_samples=sum(values),
    )


__all__ = ["BalanceReport", "balance_skew_check"]
