"""Tamper-evident audit trail."""

from __future__ import annotations

from efe_security.audit.chain import ChainVerifier, seal_record
from efe_security.audit.emitter import AuditEmitter, AuditSink
from efe_security.audit.records import (
    AuditDraft,
    AuditOutcome,
    AuditRecord,
    AuditSeverity,
)
from efe_security.audit.search import AuditQuery, InMemoryAuditSearch
from efe_security.audit.store import AuditStore, InMemoryAuditStore
from efe_security.audit.verifier import VerifyOutcome, verify_range

__all__ = [
    "AuditDraft",
    "AuditEmitter",
    "AuditOutcome",
    "AuditQuery",
    "AuditRecord",
    "AuditSeverity",
    "AuditSink",
    "AuditStore",
    "ChainVerifier",
    "InMemoryAuditSearch",
    "InMemoryAuditStore",
    "VerifyOutcome",
    "seal_record",
    "verify_range",
]
