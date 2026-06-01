"""Per-tenant PII policy."""

from __future__ import annotations

import enum
from dataclasses import dataclass, field

from efe_security.pii.classifier import PiiKind

__all__ = ["PiiPolicy", "PiiProfile", "default_policy_for"]


class PiiProfile(str, enum.Enum):
    default = "default"
    strict = "strict"
    hipaa = "hipaa"


@dataclass(frozen=True, slots=True)
class PiiPolicy:
    tenant_id: str
    profile: PiiProfile
    enabled_kinds: frozenset[PiiKind]
    retention_days: int
    phi_extra_encryption: bool = False
    audit_phi_access: bool = False
    field_overrides: tuple[tuple[str, str], ...] = ()


_DEFAULT_KINDS: frozenset[PiiKind] = frozenset(
    {
        PiiKind.email,
        PiiKind.phone,
        PiiKind.iban,
        PiiKind.ssn_us,
        PiiKind.credit_card,
        PiiKind.ipv4,
        PiiKind.ipv6,
        PiiKind.dob,
    }
)

_STRICT_KINDS: frozenset[PiiKind] = _DEFAULT_KINDS | {PiiKind.name}


def default_policy_for(tenant_id: str, profile: PiiProfile = PiiProfile.default) -> PiiPolicy:
    if profile == PiiProfile.default:
        return PiiPolicy(
            tenant_id=tenant_id,
            profile=profile,
            enabled_kinds=_DEFAULT_KINDS,
            retention_days=30,
        )
    if profile == PiiProfile.strict:
        return PiiPolicy(
            tenant_id=tenant_id,
            profile=profile,
            enabled_kinds=_STRICT_KINDS,
            retention_days=30,
        )
    # hipaa
    return PiiPolicy(
        tenant_id=tenant_id,
        profile=PiiProfile.hipaa,
        enabled_kinds=_STRICT_KINDS,
        retention_days=30,
        phi_extra_encryption=True,
        audit_phi_access=True,
    )
