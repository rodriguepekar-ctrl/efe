"""Dependency bag for the M11 HTTP app."""

from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING

if TYPE_CHECKING:  # pragma: no cover
    from efe_security.audit.store import InMemoryAuditStore
    from efe_security.authn.jwt_verifier import JwtVerifier
    from efe_security.authz.pdp import PolicyDecisionPoint
    from efe_security.consent.manager import ConsentManager
    from efe_security.consent.dsar import DsarService
    from efe_security.incident.triage import IncidentService
    from efe_security.lifecycle.erasure import ErasureService
    from efe_security.tenancy.provisioner import TenantProvisioner

__all__ = ["SecurityDeps"]


@dataclass(slots=True)
class SecurityDeps:
    jwt_verifier: "JwtVerifier"
    pdp: "PolicyDecisionPoint"
    audit_store: "InMemoryAuditStore"
    consent_manager: "ConsentManager"
    dsar: "DsarService"
    incidents: "IncidentService"
    erasure: "ErasureService"
    tenants: "TenantProvisioner"
