"""Storage layer — Protocols and implementations.

Protocols express *what* the engine needs ; concrete stores express *how*
to persist it.
"""

from __future__ import annotations

from efe_scheduling.storage.inmemory import (
    InMemoryDeadLetterStore,
    InMemoryIdempotencyStore,
    InMemoryJobStore,
    InMemoryPolicyStore,
    InMemoryRunStore,
    InMemoryScheduleStore,
    InMemoryTriggerStore,
)
from efe_scheduling.storage.protocols import (
    DeadLetterStore,
    IdempotencyStore,
    JobStore,
    PolicyStore,
    RunStore,
    ScheduleStore,
    TriggerStore,
)

__all__ = [
    "DeadLetterStore",
    "IdempotencyStore",
    "InMemoryDeadLetterStore",
    "InMemoryIdempotencyStore",
    "InMemoryJobStore",
    "InMemoryPolicyStore",
    "InMemoryRunStore",
    "InMemoryScheduleStore",
    "InMemoryTriggerStore",
    "JobStore",
    "PolicyStore",
    "RunStore",
    "ScheduleStore",
    "TriggerStore",
]
