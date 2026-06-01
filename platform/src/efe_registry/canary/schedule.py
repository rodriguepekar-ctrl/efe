"""Canary progression schedules."""

from __future__ import annotations

from dataclasses import dataclass

from efe_registry.errors import ValidationError


@dataclass(frozen=True, slots=True)
class Step:
    index: int
    percent: int
    min_duration_s: int


Schedule = tuple[Step, ...]


SCHEDULE_GEOMETRIC: Schedule = (
    Step(0, 1, 600),
    Step(1, 5, 600),
    Step(2, 25, 1_200),
    Step(3, 100, 0),
)

SCHEDULE_LINEAR: Schedule = (
    Step(0, 10, 600),
    Step(1, 25, 600),
    Step(2, 50, 600),
    Step(3, 75, 600),
    Step(4, 100, 0),
)


def schedule_for(kind: str, *, step_duration_s: int | None = None) -> Schedule:
    if kind == "geometric":
        base = SCHEDULE_GEOMETRIC
    elif kind == "linear":
        base = SCHEDULE_LINEAR
    elif kind == "manual":
        base = (Step(0, 1, 0),)
    else:
        raise ValidationError(f"unknown canary schedule: {kind!r}")
    if step_duration_s is None:
        return base
    # Override per-step duration, keep terminal step at 0.
    return tuple(
        Step(
            index=s.index,
            percent=s.percent,
            min_duration_s=0 if s.percent == 100 else step_duration_s,
        )
        for s in base
    )


def find_step_index(schedule: Schedule, current_percent: int) -> int:
    for s in schedule:
        if s.percent == current_percent:
            return s.index
    # Interpolate — pick the latest step with percent <= current.
    best = 0
    for s in schedule:
        if s.percent <= current_percent:
            best = s.index
    return best


__all__ = [
    "SCHEDULE_GEOMETRIC",
    "SCHEDULE_LINEAR",
    "Schedule",
    "Step",
    "find_step_index",
    "schedule_for",
]
