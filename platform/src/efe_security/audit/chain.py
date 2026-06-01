"""Audit chain sealing.

For each record :

    seal(i) = HMAC(K_v, DOMAIN_TAG_AUDIT_CHAIN ||
                      canonical({record_i - seal, prev_seal.hex(),
                                 payload_digest.hex()}))

where ``record_i - seal`` means every field except the seal itself.
``prev_seal`` for record 0 is ``b"GENESIS"``.

The sealed view references only **payload_digest**, never the raw
payload. This is what lets the verifier re-walk the chain using only
data persisted in the ``audit_log`` table (no PII retention needed).
"""

from __future__ import annotations

import hashlib
from dataclasses import dataclass
from typing import Any

from efe_security.audit.records import AuditDraft, AuditRecord
from efe_security.crypto import (
    DOMAIN_TAG_AUDIT_CHAIN,
    canonical_json,
    hmac_seal,
)

__all__ = ["ChainVerifier", "GENESIS", "seal_record"]


GENESIS: bytes = b"GENESIS"


def _sealed_view(
    *,
    audit_id: str,
    draft: AuditDraft,
    prev_seal: bytes,
    payload_digest: bytes,
) -> dict[str, Any]:
    """The canonical dict that enters the HMAC.

    Deliberately excludes the raw payload — the digest suffices. This
    lets the chain be verified after payload retention has expired
    and guarantees invariant **I7** end-to-end.
    """
    from datetime import timezone as _tz

    return {
        "audit_id": audit_id,
        "occurred_at": draft.occurred_at.astimezone(_tz.utc).isoformat(),
        "tenant_id": draft.tenant_id,
        "actor_kind": draft.actor_kind,
        "actor_id": draft.actor_id,
        "action": draft.action,
        "outcome": draft.outcome.value,
        "severity": draft.severity.value,
        "resource_kind": draft.resource_kind,
        "resource_id": draft.resource_id,
        "reasons": list(draft.reasons),
        "payload_digest": payload_digest.hex(),
        "prev_seal": prev_seal.hex(),
    }


def seal_record(
    *,
    audit_id: str,
    draft: AuditDraft,
    prev_seal: bytes,
    key_version: int = 1,
) -> AuditRecord:
    payload_canon = draft.canonical_payload()
    payload_digest = hashlib.sha256(payload_canon).digest()
    view = _sealed_view(
        audit_id=audit_id,
        draft=draft,
        prev_seal=prev_seal,
        payload_digest=payload_digest,
    )
    seal = hmac_seal(DOMAIN_TAG_AUDIT_CHAIN, view)
    return AuditRecord(
        audit_id=audit_id,
        occurred_at=draft.occurred_at,
        tenant_id=draft.tenant_id,
        actor_kind=draft.actor_kind,
        actor_id=draft.actor_id,
        action=draft.action,
        outcome=draft.outcome,
        severity=draft.severity,
        resource_kind=draft.resource_kind,
        resource_id=draft.resource_id,
        reasons=draft.reasons,
        payload_digest=payload_digest,
        prev_seal=prev_seal,
        seal=seal,
        key_version=key_version,
    )


@dataclass(slots=True)
class ChainVerifier:
    """Recompute a record's seal from persisted fields alone."""

    def recompute_seal(
        self,
        *,
        record: AuditRecord,
        prev_seal: bytes,
        payload: dict[str, Any] | None = None,
    ) -> bytes:
        """Recompute the seal from the persisted record fields only.

        ``payload`` is accepted for API compatibility with earlier
        callers, but ignored : the seal covers ``payload_digest`` only.
        """
        del payload
        from datetime import timezone as _tz

        view = {
            "audit_id": record.audit_id,
            "occurred_at": record.occurred_at.astimezone(_tz.utc).isoformat(),
            "tenant_id": record.tenant_id,
            "actor_kind": record.actor_kind,
            "actor_id": record.actor_id,
            "action": record.action,
            "outcome": record.outcome.value,
            "severity": record.severity.value,
            "resource_kind": record.resource_kind,
            "resource_id": record.resource_id,
            "reasons": list(record.reasons),
            "payload_digest": record.payload_digest.hex(),
            "prev_seal": prev_seal.hex(),
        }
        return hmac_seal(DOMAIN_TAG_AUDIT_CHAIN, view)


_ = canonical_json  # keep import
