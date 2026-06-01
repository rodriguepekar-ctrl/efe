"""Data lifecycle : retention, erasure, anonymisation, GC daemon."""

from __future__ import annotations

from efe_security.lifecycle.anonymizer import anonymize_text
from efe_security.lifecycle.erasure import (
    ErasurePhase,
    ErasureRecord,
    ErasureService,
)
from efe_security.lifecycle.gc_daemon import GcDaemon, GcOutcome
from efe_security.lifecycle.retention import (
    Category,
    RetentionPolicy,
    default_retention_policies,
)

__all__ = [
    "Category",
    "ErasurePhase",
    "ErasureRecord",
    "ErasureService",
    "GcDaemon",
    "GcOutcome",
    "RetentionPolicy",
    "anonymize_text",
    "default_retention_policies",
]
