"""
EFE Playground — Authentication (mock OIDC for offline use).

Production EFE uses real OIDC issuers (Auth0 / Keycloak / Azure AD).
The playground ships a mock issuer so users can try multi-tenant
features without setting up an external identity provider.

Tokens : opaque base64 strings of form `playground-<tenant_id>-<role>`.
NOT cryptographically signed ; trivial to forge ; for playground only.

Production-grade JWT validation with JWKS rotation lives in efe-server
D15 (Rust).
"""
from __future__ import annotations

import base64
import os
from dataclasses import dataclass
from typing import Optional


# Pre-defined demo tenants for the playground
DEMO_TENANTS = {
    "acme": {
        "tenant_id": "acme",
        "display_name": "Acme Corp",
        "roles": ["admin", "user"],
    },
    "beta-corp": {
        "tenant_id": "beta-corp",
        "display_name": "Beta Corporation",
        "roles": ["admin", "user"],
    },
    "demo-fr": {
        "tenant_id": "demo-fr",
        "display_name": "Démo Français",
        "roles": ["admin", "user"],
    },
}


@dataclass
class AuthIdentity:
    tenant_id: str
    role: str
    actor: str       # subject claim equivalent
    is_admin: bool


def issue_demo_token(tenant_id: str, role: str = "admin") -> str:
    """Generate a demo token. NOT cryptographically signed."""
    if tenant_id not in DEMO_TENANTS:
        raise ValueError(f"unknown demo tenant : {tenant_id}")
    if role not in ("admin", "user"):
        raise ValueError(f"role must be admin|user, got {role!r}")
    raw = f"playground|{tenant_id}|{role}|{tenant_id}-actor".encode("utf-8")
    return base64.urlsafe_b64encode(raw).decode("ascii")


def parse_token(token: str) -> Optional[AuthIdentity]:
    """Parse a demo token. Returns None on invalid."""
    if not token:
        return None
    try:
        raw = base64.urlsafe_b64decode(token.encode("ascii")).decode("utf-8")
        parts = raw.split("|")
        if len(parts) != 4 or parts[0] != "playground":
            return None
        tenant_id, role, actor = parts[1], parts[2], parts[3]
        if tenant_id not in DEMO_TENANTS or role not in ("admin", "user"):
            return None
        return AuthIdentity(
            tenant_id=tenant_id,
            role=role,
            actor=actor,
            is_admin=(role == "admin"),
        )
    except Exception:
        return None


def list_demo_tenants() -> list[dict]:
    return list(DEMO_TENANTS.values())
