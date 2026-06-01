"""Enumeration of the 12 run states + the transition matrix.

The state graph (see M03 architecture doc §3):

              DRAFT ──submit──► QUEUED ──dispatch──► SCHEDULED
                                  │                      │
                                  │                      └─► TRAINING
                                  │                             │
                                  │             ┌───────────────┤
                                  │             ▼               ▼
                                  │        EVALUATING      RETRYING ──retry──► QUEUED
                                  │             │               │
                                  │             ▼               ▼
                                  │        UPLOADING            LOST
                                  │             │               │
                                  │             ▼               ▼
                                  │         SUCCEEDED         FAILED
                                  │
                                  └──cancel──► CANCELLING ──► CANCELLED

Terminal states: SUCCEEDED, FAILED, CANCELLED, LOST.

Invariants checked by the state machine:

* ``attempt`` increases monotonically on every QUEUED entry after the first.
* ``worker_id`` is only set while the state is SCHEDULED/TRAINING/
  EVALUATING/UPLOADING/RETRYING (cleared otherwise).
* Terminal states never transition away (an ``InvalidTransition`` is
  raised if a caller tries).
"""

from __future__ import annotations

from enum import Enum


class RunState(str, Enum):
    """The 12 named states of a fine-tune run."""

    DRAFT = "draft"
    QUEUED = "queued"
    SCHEDULED = "scheduled"
    TRAINING = "training"
    EVALUATING = "evaluating"
    UPLOADING = "uploading"
    SUCCEEDED = "succeeded"
    FAILED = "failed"
    CANCELLING = "cancelling"
    CANCELLED = "cancelled"
    RETRYING = "retrying"
    LOST = "lost"


# ---------------------------------------------------------------------------
#  Valid transitions (27 edges) — closed set
#
#  Composition:
#      6 happy-path edges      : DRAFT→QUEUED→SCHEDULED→TRAINING→EVALUATING
#                               →UPLOADING→SUCCEEDED
#      6 retry edges           : {SCHEDULED,TRAINING,EVALUATING,UPLOADING}
#                               →RETRYING; RETRYING→QUEUED; RETRYING→FAILED
#      7 cancellation edges    : DRAFT/QUEUED→CANCELLED (direct);
#                               {SCHEDULED,TRAINING,EVALUATING,UPLOADING}
#                               →CANCELLING; CANCELLING→CANCELLED
#      4 heartbeat-loss edges  : {SCHEDULED,TRAINING,EVALUATING,UPLOADING}
#                               →LOST
#      4 hard-failure edges    : {SCHEDULED,TRAINING,EVALUATING,UPLOADING}
#                               →FAILED  (non-retryable errors)
#
#  Terminal states never appear as a source.
# ---------------------------------------------------------------------------


VALID_TRANSITIONS: frozenset[tuple[RunState, RunState]] = frozenset({
    # Happy path
    (RunState.DRAFT, RunState.QUEUED),
    (RunState.QUEUED, RunState.SCHEDULED),
    (RunState.SCHEDULED, RunState.TRAINING),
    (RunState.TRAINING, RunState.EVALUATING),
    (RunState.EVALUATING, RunState.UPLOADING),
    (RunState.UPLOADING, RunState.SUCCEEDED),

    # Retry path — retries are initiated from any of the 4 active phases
    (RunState.SCHEDULED, RunState.RETRYING),
    (RunState.TRAINING, RunState.RETRYING),
    (RunState.EVALUATING, RunState.RETRYING),
    (RunState.UPLOADING, RunState.RETRYING),
    (RunState.RETRYING, RunState.QUEUED),
    (RunState.RETRYING, RunState.FAILED),

    # Cancellation
    (RunState.DRAFT, RunState.CANCELLED),
    (RunState.QUEUED, RunState.CANCELLED),
    (RunState.SCHEDULED, RunState.CANCELLING),
    (RunState.TRAINING, RunState.CANCELLING),
    (RunState.EVALUATING, RunState.CANCELLING),
    (RunState.UPLOADING, RunState.CANCELLING),
    (RunState.CANCELLING, RunState.CANCELLED),

    # Heartbeat loss
    (RunState.SCHEDULED, RunState.LOST),
    (RunState.TRAINING, RunState.LOST),
    (RunState.EVALUATING, RunState.LOST),
    (RunState.UPLOADING, RunState.LOST),

    # Hard failure (non-retryable, from any active phase)
    (RunState.SCHEDULED, RunState.FAILED),
    (RunState.TRAINING, RunState.FAILED),
    (RunState.EVALUATING, RunState.FAILED),
    (RunState.UPLOADING, RunState.FAILED),
})


TERMINAL_STATES: frozenset[RunState] = frozenset({
    RunState.SUCCEEDED,
    RunState.FAILED,
    RunState.CANCELLED,
    RunState.LOST,
})


ACTIVE_STATES: frozenset[RunState] = frozenset({
    RunState.SCHEDULED,
    RunState.TRAINING,
    RunState.EVALUATING,
    RunState.UPLOADING,
})


WAITING_STATES: frozenset[RunState] = frozenset({
    RunState.DRAFT,
    RunState.QUEUED,
    RunState.RETRYING,
    RunState.CANCELLING,
})


RETRYABLE_STATES: frozenset[RunState] = ACTIVE_STATES


def is_terminal(state: RunState | str) -> bool:
    return RunState(state) in TERMINAL_STATES


def is_active(state: RunState | str) -> bool:
    return RunState(state) in ACTIVE_STATES


def is_waiting(state: RunState | str) -> bool:
    return RunState(state) in WAITING_STATES


__all__ = [
    "ACTIVE_STATES",
    "RETRYABLE_STATES",
    "TERMINAL_STATES",
    "VALID_TRANSITIONS",
    "WAITING_STATES",
    "RunState",
    "is_active",
    "is_terminal",
    "is_waiting",
]
