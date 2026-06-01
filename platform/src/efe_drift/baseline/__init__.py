"""Baseline lifecycle : FSM, candidate, validator, seal, rotation, manager."""

from __future__ import annotations

from efe_drift.baseline.candidate import BaselineCandidate
from efe_drift.baseline.manager import BaselineManager
from efe_drift.baseline.rotation import (
    RotationPlan,
    is_rotation_due,
    plan_rotation,
)
from efe_drift.baseline.seal import (
    SealedBaseline,
    seal_baseline,
    verify_baseline,
)
from efe_drift.baseline.state import (
    TERMINAL_STATES,
    BaselineFsm,
    iter_transitions,
)
from efe_drift.baseline.store import (
    BaselineStore,
    InMemoryBaselineStore,
    TransitionAuditEntry,
)
from efe_drift.baseline.validator import (
    BaselineValidator,
    ValidationIssue,
    ValidationReport,
)

__all__ = [
    "TERMINAL_STATES",
    "BaselineCandidate",
    "BaselineFsm",
    "BaselineManager",
    "BaselineStore",
    "BaselineValidator",
    "InMemoryBaselineStore",
    "RotationPlan",
    "SealedBaseline",
    "TransitionAuditEntry",
    "ValidationIssue",
    "ValidationReport",
    "is_rotation_due",
    "iter_transitions",
    "plan_rotation",
    "seal_baseline",
    "verify_baseline",
]
