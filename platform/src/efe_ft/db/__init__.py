"""SQLAlchemy engine + ORM models.

Submodules:
    engine     — Async engine + session maker + RLS helper
    models     — Declarative mapped classes for all 6 tables
"""

from __future__ import annotations

from efe_ft.db.engine import (
    drop_schema,
    get_engine,
    get_sessionmaker,
    init_schema,
    session_for_tenant,
)
from efe_ft.db.models import (
    AuditEvent,
    Base,
    EmbeddingCache,
    IngestBatch,
    LabelHistory,
    Snapshot,
    SnapshotMember,
    TrainingSample,
)

__all__ = [
    "AuditEvent",
    "Base",
    "EmbeddingCache",
    "IngestBatch",
    "LabelHistory",
    "Snapshot",
    "SnapshotMember",
    "TrainingSample",
    "drop_schema",
    "get_engine",
    "get_sessionmaker",
    "init_schema",
    "session_for_tenant",
]
