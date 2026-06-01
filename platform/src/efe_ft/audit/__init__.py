"""Tamper-evident audit chain.

Each audit event stores the sha256 of its canonical JSON payload
chained to the previous event's ``this_hash``. Recalculating the
chain detects any retroactive modification.
"""

from __future__ import annotations

from efe_ft.audit.chain import (
    AUDIT_KINDS,
    AuditChain,
    AuditEntry,
    verify_chain,
)

__all__ = ["AUDIT_KINDS", "AuditChain", "AuditEntry", "verify_chain"]
