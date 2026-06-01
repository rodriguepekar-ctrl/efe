"""Chain verifier — recompute seals and detect tampering."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import TYPE_CHECKING

from efe_security.audit.chain import GENESIS, ChainVerifier
from efe_security.audit.records import AuditRecord
from efe_security.errors import ChainBroken

if TYPE_CHECKING:  # pragma: no cover
    from efe_security.audit.store import InMemoryAuditStore

__all__ = ["VerifyOutcome", "verify_range"]


@dataclass(frozen=True, slots=True)
class VerifyOutcome:
    inspected: int
    ok: bool
    first_broken: str | None = None
    errors: tuple[str, ...] = field(default_factory=tuple)


async def verify_range(store: "InMemoryAuditStore", *, limit: int = 10_000) -> VerifyOutcome:
    """Re-walk the chain and confirm every seal.

    Returns :class:`VerifyOutcome` summarising the check. Never raises
    on mismatch — callers use the outcome to decide escalation.
    """
    records: list[AuditRecord] = await store.fetch_range(limit=limit)
    verifier = ChainVerifier()
    prev = GENESIS
    errors: list[str] = []
    first_broken: str | None = None
    for rec in records:
        # The seal covers persisted fields only — no payload read needed.
        expected = verifier.recompute_seal(record=rec, prev_seal=prev)
        if expected != rec.seal:
            errors.append(f"seal_mismatch audit_id={rec.audit_id}")
            first_broken = first_broken or rec.audit_id
            break
        if rec.prev_seal != prev:
            errors.append(f"prev_seal_mismatch audit_id={rec.audit_id}")
            first_broken = first_broken or rec.audit_id
            break
        prev = rec.seal
    return VerifyOutcome(
        inspected=len(records),
        ok=not errors,
        first_broken=first_broken,
        errors=tuple(errors),
    )


_ = ChainBroken  # retain import
