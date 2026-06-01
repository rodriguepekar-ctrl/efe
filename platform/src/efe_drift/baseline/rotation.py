"""Baseline rotation policy (M09 §13.3).

On every promotion of a new baseline, the previous ACTIVE baseline
(if any) for the same ``(tenant, task, feature_name)`` scope is moved
to RETIRING and scheduled to RETIRED after ``overlap_days``.

This module does NOT perform DB writes directly — it returns a
:class:`RotationPlan` (pure data) that the :class:`BaselineManager`
applies transactionally.

Invariants enforced :

* **I8** : ``overlap_days ≥ 1`` (configurable, default 3). A zero
  overlap is not allowed.
* **One-active-at-a-time** : after a rotation, there is exactly one
  baseline in ACTIVE state per scope. During the overlap window both
  the old (RETIRING) and the new (ACTIVE) are queryable for
  detection, but the pointer "active baseline" resolves to the newest.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timedelta

from efe_drift.errors import ValidationError
from efe_drift.records import Baseline


@dataclass(frozen=True, slots=True)
class RotationPlan:
    """Pure description of a rotation outcome."""

    previous_baseline_id: str | None   # None on first-ever promotion
    previous_state_version: int | None
    previous_planned_end_at: datetime | None
    new_baseline_id: str
    overlap_days: int

    @property
    def has_predecessor(self) -> bool:
        return self.previous_baseline_id is not None


def plan_rotation(
    *,
    incoming: Baseline,
    currently_active: Baseline | None,
    promotion_time: datetime,
    overlap_days: int,
) -> RotationPlan:
    """Compute a :class:`RotationPlan` for the given inputs.

    ``incoming`` is the candidate baseline about to be promoted ;
    ``currently_active`` is the ACTIVE baseline for the same scope
    (``None`` if this is the first promotion for the feature).
    """
    if overlap_days < 1:
        raise ValidationError(
            f"overlap_days must be ≥ 1, got {overlap_days}",
        )
    if overlap_days > 14:
        raise ValidationError(
            f"overlap_days must be ≤ 14, got {overlap_days}",
        )
    if promotion_time.tzinfo is None:
        raise ValidationError("promotion_time must be tz-aware")

    if currently_active is None:
        return RotationPlan(
            previous_baseline_id=None,
            previous_state_version=None,
            previous_planned_end_at=None,
            new_baseline_id=incoming.baseline_id,
            overlap_days=overlap_days,
        )
    if currently_active.state != "active":
        raise ValidationError(
            f"currently_active must be in ACTIVE state, got "
            f"{currently_active.state!r}",
        )
    if (
        currently_active.spec.tenant_id != incoming.spec.tenant_id
        or currently_active.spec.task_id != incoming.spec.task_id
        or currently_active.spec.feature_name != incoming.spec.feature_name
    ):
        raise ValidationError(
            "incoming and currently_active must share "
            "(tenant, task, feature_name)",
        )
    planned_end = promotion_time + timedelta(days=overlap_days)
    return RotationPlan(
        previous_baseline_id=currently_active.baseline_id,
        previous_state_version=currently_active.state_version,
        previous_planned_end_at=planned_end,
        new_baseline_id=incoming.baseline_id,
        overlap_days=overlap_days,
    )


def is_rotation_due(
    baseline: Baseline, *, now: datetime,
) -> bool:
    """Return ``True`` iff a RETIRING baseline should move to RETIRED."""
    if baseline.state != "retiring":
        return False
    if baseline.planned_end_at is None:
        return False
    return now >= baseline.planned_end_at


__all__ = ["RotationPlan", "is_rotation_due", "plan_rotation"]
