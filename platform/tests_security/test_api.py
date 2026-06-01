"""API smoke — routes wired + exception handler."""

from __future__ import annotations

import asyncio

from efe_security.api import SecurityDeps, build_app
from efe_security.audit import InMemoryAuditStore
from efe_security.authn.jwks_cache import Jwk, JwksCache, StaticJwksSource
from efe_security.authn.jwt_verifier import JwtVerifier, JwtVerifierConfig
from efe_security.authz import PolicyDecisionPoint, RbacRegistry
from efe_security.consent import ConsentManager, DsarService, default_purpose_catalog
from efe_security.incident import IncidentService
from efe_security.lifecycle import ErasureService
from efe_security.secrets import InMemoryKmsClient, KeyRegistry
from efe_security.tenancy import TenantProvisioner


def _build_deps() -> SecurityDeps:
    async def _init() -> SecurityDeps:
        jwks = JwksCache(source=StaticJwksSource(keys=()))
        return SecurityDeps(
            jwt_verifier=JwtVerifier(
                jwks=jwks,
                config=JwtVerifierConfig(
                    expected_iss=("https://idp.test",),
                    expected_aud=("efe.api.v1",),
                ),
            ),
            pdp=PolicyDecisionPoint(rbac=RbacRegistry()),
            audit_store=InMemoryAuditStore(),
            consent_manager=ConsentManager(purposes=await default_purpose_catalog()),
            dsar=DsarService(),
            incidents=IncidentService(),
            erasure=ErasureService(),
            tenants=TenantProvisioner(kms=InMemoryKmsClient(), key_registry=KeyRegistry()),
        )

    return asyncio.run(_init())


def test_app_registers_routes() -> None:
    deps = _build_deps()
    app = build_app(deps)
    paths: list[str] = []
    for r in getattr(app, "routes", []):
        if isinstance(r, tuple):
            if r[0]:
                paths.append(str(r[0][0]))
        else:
            p = getattr(r, "path", None)
            if p is not None:
                paths.append(str(p))
    for required in [
        "/security/health",
        "/v1/tenants",
        "/v1/consent",
        "/v1/dsar",
        "/v1/audit/verify",
        "/v1/compliance/evidence",
    ]:
        assert any(p == required for p in paths), f"route not registered: {required}"
