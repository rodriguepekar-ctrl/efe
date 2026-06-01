"""AuthN : JWT + JWKS + mTLS + session + service_account + factor."""

from __future__ import annotations

from efe_security.authn.factor import MfaRequirement, require_mfa
from efe_security.authn.jwks_cache import Jwk, JwksCache, StaticJwksSource
from efe_security.authn.jwt_verifier import (
    JwtClaims,
    JwtVerifier,
    JwtVerifierConfig,
    parse_token_header,
)
from efe_security.authn.mtls_verifier import MtlsVerifier, SpiffeId, parse_spiffe_id
from efe_security.authn.service_account import (
    ServiceAccount,
    ServiceAccountRegistry,
)
from efe_security.authn.session import (
    OpaqueSession,
    SessionStore,
    new_session_token,
)

__all__ = [
    "Jwk",
    "JwksCache",
    "JwtClaims",
    "JwtVerifier",
    "JwtVerifierConfig",
    "MfaRequirement",
    "MtlsVerifier",
    "OpaqueSession",
    "ServiceAccount",
    "ServiceAccountRegistry",
    "SessionStore",
    "SpiffeId",
    "StaticJwksSource",
    "new_session_token",
    "parse_spiffe_id",
    "parse_token_header",
    "require_mfa",
]
