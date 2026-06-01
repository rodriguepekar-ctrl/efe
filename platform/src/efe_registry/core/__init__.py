"""Core registry abstractions — artifact FSM, registry façade, promoter."""

from __future__ import annotations

from efe_registry.core.artifact import (
    Artifact,
    ArtifactRole,
    ArtifactState,
    artifact_role_values,
    artifact_state_values,
)
from efe_registry.core.promoter import ArtifactPromoter, PromoteResult
from efe_registry.core.registry import Registry
from efe_registry.core.state_machine import (
    ARTIFACT_TRANSITIONS,
    ArtifactTransitionValidator,
)

__all__ = [
    "ARTIFACT_TRANSITIONS",
    "Artifact",
    "ArtifactPromoter",
    "ArtifactRole",
    "ArtifactState",
    "ArtifactTransitionValidator",
    "PromoteResult",
    "Registry",
    "artifact_role_values",
    "artifact_state_values",
]
