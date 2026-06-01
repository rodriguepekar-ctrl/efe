"""Rollback package — the <60s flow.

Public surface:

* :class:`RollbackExecutor` — orchestrator, SLO-bound.
* :class:`RollbackReceipt` — return value of ``execute()``.
* :class:`PreWarmedStandbyManager` — keeps a standby pod warm.
"""

from __future__ import annotations

from efe_registry.rollback.executor import RollbackExecutor
from efe_registry.rollback.phases import (
    PhaseName,
    PhaseRecorder,
    PhaseTimeout,
)
from efe_registry.rollback.receipt import RollbackReceipt
from efe_registry.rollback.standby import (
    InMemoryStandbyProbe,
    PreWarmedStandbyManager,
    StandbyProbe,
)

__all__ = [
    "InMemoryStandbyProbe",
    "PhaseName",
    "PhaseRecorder",
    "PhaseTimeout",
    "PreWarmedStandbyManager",
    "RollbackExecutor",
    "RollbackReceipt",
    "StandbyProbe",
]
