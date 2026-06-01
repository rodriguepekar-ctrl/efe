"""Enumeration of the 9 eval states + transitions matrix.

The state graph (see M05 architecture doc §5):

    QUEUED → RUNNING → MEASURED → DECIDED → PROMOTED / REJECTED /
                                              HUMAN_REVIEW
    HUMAN_REVIEW → PROMOTED / REJECTED
    PROMOTED → ROLLED_BACK (within 60-min canary window)
    {QUEUED, RUNNING, MEASURED, DECIDED, HUMAN_REVIEW} → FAILED

Terminal states: REJECTED, FAILED, ROLLED_BACK. PROMOTED is
*near-terminal*: it can still transition to ROLLED_BACK while the
rollback daemon is watching.
"""

from __future__ import annotations

from enum import Enum


class EvalState(str, Enum):
    QUEUED = "queued"
    RUNNING = "running"
    MEASURED = "measured"
    DECIDED = "decided"
    PROMOTED = "promoted"
    REJECTED = "rejected"
    HUMAN_REVIEW = "human_review"
    FAILED = "failed"
    ROLLED_BACK = "rolled_back"


VALID_TRANSITIONS: frozenset[tuple[EvalState, EvalState]] = frozenset({
    # happy path
    (EvalState.QUEUED, EvalState.RUNNING),
    (EvalState.RUNNING, EvalState.MEASURED),
    (EvalState.MEASURED, EvalState.DECIDED),
    (EvalState.DECIDED, EvalState.PROMOTED),
    (EvalState.DECIDED, EvalState.REJECTED),
    (EvalState.DECIDED, EvalState.HUMAN_REVIEW),
    # human review outcomes
    (EvalState.HUMAN_REVIEW, EvalState.PROMOTED),
    (EvalState.HUMAN_REVIEW, EvalState.REJECTED),
    # rollback
    (EvalState.PROMOTED, EvalState.ROLLED_BACK),
    # failure paths
    (EvalState.QUEUED, EvalState.FAILED),
    (EvalState.RUNNING, EvalState.FAILED),
    (EvalState.MEASURED, EvalState.FAILED),
    (EvalState.DECIDED, EvalState.FAILED),
    (EvalState.HUMAN_REVIEW, EvalState.FAILED),
})


TERMINAL_STATES: frozenset[EvalState] = frozenset({
    EvalState.REJECTED,
    EvalState.FAILED,
    EvalState.ROLLED_BACK,
})


ACTIVE_STATES: frozenset[EvalState] = frozenset({
    EvalState.RUNNING, EvalState.MEASURED, EvalState.DECIDED,
})


WAITING_STATES: frozenset[EvalState] = frozenset({
    EvalState.QUEUED, EvalState.HUMAN_REVIEW,
})


def is_terminal(state: EvalState | str) -> bool:
    return EvalState(state) in TERMINAL_STATES


def is_active(state: EvalState | str) -> bool:
    return EvalState(state) in ACTIVE_STATES


def is_waiting(state: EvalState | str) -> bool:
    return EvalState(state) in WAITING_STATES


__all__ = [
    "ACTIVE_STATES",
    "TERMINAL_STATES",
    "VALID_TRANSITIONS",
    "WAITING_STATES",
    "EvalState",
    "is_active",
    "is_terminal",
    "is_waiting",
]
