"""SQLAlchemy engine + models for :mod:`efe_orch`.

Public surface:

    engine               — async engine + session helpers
    models               — declarative mapped classes (5 tables)
"""

from __future__ import annotations

from efe_orch.db.engine import (
    dispose_all,
    drop_schema,
    get_engine,
    get_sessionmaker,
    init_schema,
    session_for_tenant,
)
from efe_orch.db.models import (
    Base,
    FinetuneRun,
    JsonType,
    RunCheckpoint,
    RunStateTransition,
    TenantQuota,
    UUIDStr,
    WorkerHeartbeat,
)

__all__ = [
    "Base",
    "FinetuneRun",
    "JsonType",
    "RunCheckpoint",
    "RunStateTransition",
    "TenantQuota",
    "UUIDStr",
    "WorkerHeartbeat",
    "dispose_all",
    "drop_schema",
    "get_engine",
    "get_sessionmaker",
    "init_schema",
    "session_for_tenant",
]
