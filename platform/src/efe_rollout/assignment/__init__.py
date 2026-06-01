"""Variant assignment — hot-path hash routing."""

from __future__ import annotations

from efe_rollout.assignment.assigner import AssignmentDecision, HashAssigner
from efe_rollout.assignment.cache import AssignmentCache, InMemoryAssignmentCache
from efe_rollout.assignment.forced import (
    ForcedAssignment,
    ForcedAssignmentStore,
    InMemoryForcedAssignmentStore,
)
from efe_rollout.assignment.holdout import HoldoutFilter

__all__ = [
    "AssignmentCache",
    "AssignmentDecision",
    "ForcedAssignment",
    "ForcedAssignmentStore",
    "HashAssigner",
    "HoldoutFilter",
    "InMemoryAssignmentCache",
    "InMemoryForcedAssignmentStore",
]
