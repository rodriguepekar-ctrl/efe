"""Run state machine for :mod:`efe_orch`.

Public surface:

    RunState              — enum of the 12 states
    VALID_TRANSITIONS     — frozenset of legal (from, to) pairs
    TransitionCtx         — data-class attached to every transition
    is_terminal / is_active / is_waiting — predicates
    StateMachine.transition — atomic, advisory-locked mutation
    AdvisoryLock / LockBackend — portable locking abstraction
"""

from __future__ import annotations

from efe_orch.state_machine.locks import (
    AdvisoryLock,
    InMemoryLockBackend,
    LockBackend,
    PostgresLockBackend,
    SqliteLockBackend,
)
from efe_orch.state_machine.states import (
    ACTIVE_STATES,
    RETRYABLE_STATES,
    TERMINAL_STATES,
    VALID_TRANSITIONS,
    WAITING_STATES,
    RunState,
    is_active,
    is_terminal,
    is_waiting,
)
from efe_orch.state_machine.transitions import StateMachine, TransitionCtx

__all__ = [
    "ACTIVE_STATES",
    "RETRYABLE_STATES",
    "TERMINAL_STATES",
    "VALID_TRANSITIONS",
    "WAITING_STATES",
    "AdvisoryLock",
    "InMemoryLockBackend",
    "LockBackend",
    "PostgresLockBackend",
    "RunState",
    "SqliteLockBackend",
    "StateMachine",
    "TransitionCtx",
    "is_active",
    "is_terminal",
    "is_waiting",
]
