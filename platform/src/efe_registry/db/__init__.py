"""Persistence layer for efe_registry — SQLAlchemy 2.0 async."""

from __future__ import annotations

from efe_registry.db.engine import (
    build_engine,
    build_sessionmaker,
    create_all,
)
from efe_registry.db.models import (
    SBOM,
    Artifact,
    ArtifactContract,
    ArtifactPromotion,
    ArtifactSignature,
    ArtifactStateTransition,
    Base,
    CanaryRun,
    Contract,
    Endpoint,
    RollbackEvent,
    RoutingTableRow,
    ShadowTap,
    SLSAProvenance,
)

__all__ = [
    "SBOM",
    "Artifact",
    "ArtifactContract",
    "ArtifactPromotion",
    "ArtifactSignature",
    "ArtifactStateTransition",
    "Base",
    "CanaryRun",
    "Contract",
    "Endpoint",
    "RollbackEvent",
    "RoutingTableRow",
    "SLSAProvenance",
    "ShadowTap",
    "build_engine",
    "build_sessionmaker",
    "create_all",
]
