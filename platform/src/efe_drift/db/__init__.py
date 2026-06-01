"""Persistence layer for :mod:`efe_drift` — SQLAlchemy 2.0 async."""

from __future__ import annotations

from efe_drift.db.engine import build_engine, build_sessionmaker, create_all
from efe_drift.db.models import (
    AlertIncidentRow,
    Base,
    BaselineRow,
    BaselineStateTransitionRow,
    DriftEvaluationRow,
    FeatureCatalogRow,
    FeatureSnapshotRow,
    RawEventRow,
)

__all__ = [
    "AlertIncidentRow",
    "Base",
    "BaselineRow",
    "BaselineStateTransitionRow",
    "DriftEvaluationRow",
    "FeatureCatalogRow",
    "FeatureSnapshotRow",
    "RawEventRow",
    "build_engine",
    "build_sessionmaker",
    "create_all",
]
