"""Multi-tenancy subsystem."""

from __future__ import annotations

from efe_security.tenancy.context import (
    TenantContext,
    current_tenant,
    set_current_tenant,
)
from efe_security.tenancy.isolation import NetworkPolicyHint, NetworkPolicyKind
from efe_security.tenancy.provisioner import (
    OffboardMode,
    Tenant,
    TenantProvisioner,
    TenantTier,
)
from efe_security.tenancy.quota import (
    QuotaBucket,
    QuotaKind,
    QuotaManager,
    QuotaSpec,
)

__all__ = [
    "NetworkPolicyHint",
    "NetworkPolicyKind",
    "OffboardMode",
    "QuotaBucket",
    "QuotaKind",
    "QuotaManager",
    "QuotaSpec",
    "Tenant",
    "TenantContext",
    "TenantProvisioner",
    "TenantTier",
    "current_tenant",
    "set_current_tenant",
]
