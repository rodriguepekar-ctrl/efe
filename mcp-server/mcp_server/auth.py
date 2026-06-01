"""OIDC authentication bridge (M16).

The MCP server runs *as* the user — it doesn't maintain its own credentials.
The OIDC bearer token is read once from `EFE_OIDC_TOKEN` env var, validated
syntactically, and forwarded as-is to the EFE backend on every call.

Tenant identity is **derived from the JWT** by the EFE server (D11 zero-trust
isolation) — never from MCP tool arguments. This eliminates an entire class
of confused-deputy attacks (TM-M16-S-01 in the threat model).

The token's payload is parsed for **logging only** (tenant_id + sub + exp).
The MCP server does NOT validate the JWT signature — that is the EFE
backend's responsibility, single source of truth (D15).
"""
from __future__ import annotations

import base64
import json
import os
import time
from dataclasses import dataclass
from typing import Optional


class AuthError(Exception):
    """Raised when the OIDC token is missing, malformed, or expired."""


@dataclass(frozen=True)
class AuthContext:
    """Decoded token claims used for logging + redaction.

    The MCP server never trusts these claims for authorization — they are
    informational. The EFE backend is the source of truth (D15).
    """

    raw_token: str
    sub: Optional[str] = None
    tenant_id: Optional[str] = None
    exp: Optional[int] = None
    issuer: Optional[str] = None

    def is_expired(self, *, now: Optional[float] = None) -> bool:
        if self.exp is None:
            return False
        now = now if now is not None else time.time()
        return now >= float(self.exp)

    def redacted_token(self) -> str:
        """Return a redacted form for logs : `eyJh***...***xyz` (first 4 + last 4)."""
        if not self.raw_token or len(self.raw_token) < 16:
            return "***"
        return f"{self.raw_token[:4]}***{self.raw_token[-4:]}"


def _b64url_decode(segment: str) -> bytes:
    # Pad to multiple of 4 for base64 ; JWT uses URL-safe alphabet.
    pad = "=" * (-len(segment) % 4)
    return base64.urlsafe_b64decode(segment + pad)


def parse_jwt_unverified(token: str) -> dict:
    """Parse a JWT *without* signature verification.

    Used solely for logging + redaction. The EFE backend validates the
    signature ; trusting it here would be a separate trust boundary
    (TM-M16-S-02). Returns the claims dict, or raises AuthError.
    """
    parts = token.split(".")
    if len(parts) != 3:
        raise AuthError(f"malformed JWT: expected 3 segments, got {len(parts)}")
    try:
        payload_bytes = _b64url_decode(parts[1])
        return json.loads(payload_bytes)
    except (ValueError, json.JSONDecodeError) as exc:
        raise AuthError(f"malformed JWT payload: {exc}") from exc


def load_auth_from_env(
    *,
    env_var: str = "EFE_OIDC_TOKEN",
    allow_opaque: bool = True,
) -> AuthContext:
    """Read + parse the OIDC token from env.

    `allow_opaque=True` accepts non-JWT bearer tokens (e.g. playground demo
    tokens like `playground-acme-admin-...`) — they are passed through as
    `raw_token` with no claims extraction.
    """
    token = os.environ.get(env_var)
    if not token:
        raise AuthError(
            f"missing required environment variable {env_var!r} ; "
            f"please export it before starting the MCP server"
        )
    token = token.strip()
    if not token:
        raise AuthError(f"environment variable {env_var!r} is empty")

    # Heuristic : JWTs have exactly 2 dots and base64url body.
    if token.count(".") == 2:
        try:
            claims = parse_jwt_unverified(token)
        except AuthError:
            if not allow_opaque:
                raise
            return AuthContext(raw_token=token)
        return AuthContext(
            raw_token=token,
            sub=claims.get("sub"),
            tenant_id=claims.get("tenant_id") or claims.get("tid"),
            exp=claims.get("exp"),
            issuer=claims.get("iss"),
        )
    if not allow_opaque:
        raise AuthError("token is not a JWT and allow_opaque=False")
    return AuthContext(raw_token=token)


def authorization_header(ctx: AuthContext) -> dict[str, str]:
    """Return the `Authorization` header dict for HTTP transport."""
    return {"Authorization": f"Bearer {ctx.raw_token}"}
