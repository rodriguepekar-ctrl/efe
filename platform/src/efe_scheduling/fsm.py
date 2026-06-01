"""Finite-state-machine helpers.

Transitions are expressed as a ``frozenset[tuple[State, State]]``. The
:func:`guard` helper asserts a proposed transition is whitelisted and
raises :class:`~efe_scheduling.errors.InvalidStateTransition` otherwise.

This keeps the per-aggregate FSM logic tiny and readable :

    from efe_scheduling.fsm import guard
    from efe_scheduling.types import ScheduleState

    guard(ScheduleState.disabled, ScheduleState.enabled, SCHEDULE_ALLOWED)
    # raises InvalidStateTransition if that pair is not in SCHEDULE_ALLOWED
"""

from __future__ import annotations

from typing import TypeVar

from efe_scheduling.errors import InvalidStateTransition

__all__ = [
    "JOB_ALLOWED",
    "RUN_ALLOWED",
    "SCHEDULE_ALLOWED",
    "TRIGGER_ALLOWED",
    "guard",
    "is_allowed",
]

S = TypeVar("S")


def guard(
    current: S,
    target: S,
    allowed: frozenset[tuple[S, S]],
    *,
    aggregate: str = "aggregate",
) -> None:
    """Raise :class:`InvalidStateTransition` if ``(current, target)`` ∉ ``allowed``."""
    if (current, target) not in allowed:
        raise InvalidStateTransition(
            f"{aggregate}: transition {current} → {target} is not allowed",
            detail={"current": str(current), "target": str(target)},
        )


def is_allowed(current: S, target: S, allowed: frozenset[tuple[S, S]]) -> bool:
    return (current, target) in allowed


# ---------------------------------------------------------------------------
# FSM tables (string-valued so they can be compared across process boundaries).
# ---------------------------------------------------------------------------

SCHEDULE_ALLOWED: frozenset[tuple[str, str]] = frozenset(
    {
        ("nil", "disabled"),
        ("nil", "enabled"),
        ("disabled", "enabled"),
        ("enabled", "disabled"),
        ("enabled", "completed"),
        ("enabled", "errored"),
        ("disabled", "errored"),
    }
)
"""Allowed transitions for :class:`~efe_scheduling.types.ScheduleState`."""

TRIGGER_ALLOWED: frozenset[tuple[str, str]] = frozenset(
    {
        ("nil", "enabled"),
        ("nil", "disabled"),
        ("enabled", "disabled"),
        ("disabled", "enabled"),
        ("enabled", "errored"),
        ("disabled", "errored"),
    }
)
"""Allowed transitions for a trigger."""

JOB_ALLOWED: frozenset[tuple[str, str]] = frozenset(
    {
        ("nil", "queued"),
        ("queued", "running"),
        ("running", "succeeded"),
        ("running", "queued"),  # retry
        ("running", "failed_permanent"),
        ("running", "cancelled"),
        ("queued", "cancelled"),
        ("failed_permanent", "dead_letter"),
        ("dead_letter", "queued"),  # operator requeue
    }
)
"""Allowed transitions for :class:`~efe_scheduling.types.JobState`."""

RUN_ALLOWED: frozenset[tuple[str, str]] = frozenset(
    {
        ("nil", "running"),
        ("running", "succeeded"),
        ("running", "failed"),
        ("running", "cancelled"),
        ("running", "timeout"),
    }
)
"""Allowed transitions for :class:`~efe_scheduling.types.JobRunStatus`."""
