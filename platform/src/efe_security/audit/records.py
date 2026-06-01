"""Audit record model."""

from __future__ import annotations

import enum
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any

from efe_security.crypto import canonical_json

__all__ = [
    "AuditDraft",
    "AuditOutcome",
    "AuditRecord",
    "AuditSeverity",
]


class AuditOutcome(str, enum.Enum):
    allow = "allow"
    deny = "deny"
    success = "success"
    failure = "failure"


class AuditSeverity(str, enum.Enum):
    info = "info"
    warning = "warning"
    critical = "critical"


@dataclass(frozen=True, slots=True)
class AuditDraft:
    """Inbound audit event — unsealed."""

    occurred_at: datetime
    tenant_id: str | None
    actor_kind: str
    actor_id: str
    action: str
    outcome: AuditOutcome
    severity: AuditSeverity = AuditSeverity.info
    resource_kind: str | None = None
    resource_id: str | None = None
    reasons: tuple[str, ...] = ()
    payload: dict[str, Any] = field(default_factory=dict)

    def __post_init__(self) -> None:
        if self.occurred_at.tzinfo is None:
            raise ValueError("occurred_at must be tz-aware")

    def canonical_payload(self) -> bytes:
        return canonical_json(
            {
                "occurred_at": self.occurred_at.astimezone(timezone.utc).isoformat(),
                "tenant_id": self.tenant_id,
                "actor_kind": self.actor_kind,
                "actor_id": self.actor_id,
                "action": self.action,
                "outcome": self.outcome.value,
                "severity": self.severity.value,
                "resource_kind": self.resource_kind,
                "resource_id": self.resource_id,
                "reasons": list(self.reasons),
                "payload": self.payload,
            }
        )


@dataclass(frozen=True, slots=True)
class AuditRecord:
    """Sealed audit record — one line on disk."""

    audit_id: str
    occurred_at: datetime
    tenant_id: str | None
    actor_kind: str
    actor_id: str
    action: str
    outcome: AuditOutcome
    severity: AuditSeverity
    resource_kind: str | None
    resource_id: str | None
    reasons: tuple[str, ...]
    payload_digest: bytes
    prev_seal: bytes
    seal: bytes
    key_version: int

    def to_dict(self) -> dict[str, Any]:
        return {
            "audit_id": self.audit_id,
            "occurred_at": self.occurred_at.astimezone(timezone.utc).isoformat(),
            "tenant_id": self.tenant_id,
            "actor_kind": self.actor_kind,
            "actor_id": self.actor_id,
            "action": self.action,
            "outcome": self.outcome.value,
            "severity": self.severity.value,
            "resource_kind": self.resource_kind,
            "resource_id": self.resource_id,
            "reasons": list(self.reasons),
            "payload_digest": self.payload_digest.hex(),
            "prev_seal": self.prev_seal.hex(),
            "seal": self.seal.hex(),
            "key_version": self.key_version,
        }
