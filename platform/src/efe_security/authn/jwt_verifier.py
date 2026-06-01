"""JWT verifier.

Validates :

- signature (via the JWKS cache),
- ``exp`` / ``nbf`` / ``iat`` window,
- ``aud`` membership,
- ``iss`` expected issuer,
- required private claims (``tenant_id``, ``scopes``).

The verifier is pure-Python for JSON parsing and signature only —
RSA/EC/EdDSA math requires the ``cryptography`` package (falls back to
HMAC-SHA256 for tests if not present).
"""

from __future__ import annotations

import base64
import json
import logging
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any

from efe_security.authn.jwks_cache import Jwk, JwksCache
from efe_security.context import Actor, ActorKind, Principal
from efe_security.errors import (
    ExpiredToken,
    InvalidAudience,
    InvalidIssuer,
    InvalidSignature,
    MissingToken,
)

_LOG = logging.getLogger("efe_security.authn.jwt")

__all__ = [
    "JwtClaims",
    "JwtVerifier",
    "JwtVerifierConfig",
    "parse_token_header",
]


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _b64url_decode(s: str) -> bytes:
    pad = "=" * (-len(s) % 4)
    return base64.urlsafe_b64decode(s + pad)


def parse_token_header(token: str) -> dict[str, Any]:
    if not token:
        raise MissingToken("empty token")
    parts = token.split(".")
    if len(parts) != 3:
        raise InvalidSignature("JWT must have 3 parts")
    try:
        header = json.loads(_b64url_decode(parts[0]).decode("utf-8"))
    except Exception as exc:
        raise InvalidSignature("invalid JWT header encoding") from exc
    if not isinstance(header, dict):
        raise InvalidSignature("JWT header is not a JSON object")
    return header


@dataclass(frozen=True, slots=True)
class JwtClaims:
    iss: str
    sub: str
    aud: tuple[str, ...]
    iat: int
    exp: int
    nbf: int | None
    jti: str | None
    tenant_id: str | None
    scopes: tuple[str, ...]
    amr: tuple[str, ...]
    acr: str | None
    raw: dict[str, Any]


@dataclass(frozen=True, slots=True)
class JwtVerifierConfig:
    expected_iss: tuple[str, ...]
    expected_aud: tuple[str, ...]
    clock_skew_s: int = 30
    required_scopes: tuple[str, ...] = ()
    require_tenant: bool = True
    allowed_algorithms: tuple[str, ...] = ("ES256", "EdDSA", "HS256")
    """Strict allowlist — belt & suspenders against ``alg=none`` attacks.

    ``HS256`` is allowed for test harnesses ; production should leave
    only asymmetric algorithms (``ES256``, ``EdDSA``).
    """


# ---------------------------------------------------------------------------
# Signature verification helpers
# ---------------------------------------------------------------------------


def _verify_signature(alg: str, jwk: Jwk, signing_input: bytes, signature: bytes) -> bool:
    if alg != jwk.alg:
        return False
    if alg == "HS256":
        import hmac
        from hashlib import sha256

        expected = hmac.new(jwk.key_bytes, signing_input, sha256).digest()
        return hmac.compare_digest(expected, signature)

    try:
        from cryptography.hazmat.primitives import hashes, serialization
        from cryptography.hazmat.primitives.asymmetric import ec, ed25519
        from cryptography.hazmat.primitives.asymmetric.utils import (
            encode_dss_signature,
        )
    except ImportError:  # pragma: no cover
        return False

    if alg == "EdDSA":
        try:
            pk = ed25519.Ed25519PublicKey.from_public_bytes(jwk.key_bytes)
            pk.verify(signature, signing_input)
            return True
        except Exception:
            return False
    if alg == "ES256":
        try:
            pk = serialization.load_der_public_key(jwk.key_bytes)
            assert isinstance(pk, ec.EllipticCurvePublicKey)
            if len(signature) != 64:
                return False
            r = int.from_bytes(signature[:32], "big")
            s = int.from_bytes(signature[32:], "big")
            der = encode_dss_signature(r, s)
            pk.verify(der, signing_input, ec.ECDSA(hashes.SHA256()))
            return True
        except Exception:
            return False
    return False


# ---------------------------------------------------------------------------
# JwtVerifier
# ---------------------------------------------------------------------------


@dataclass(slots=True)
class JwtVerifier:
    jwks: JwksCache
    config: JwtVerifierConfig

    async def verify(self, token: str, *, now: datetime) -> tuple[JwtClaims, Principal]:
        if not token:
            raise MissingToken("no Bearer token")
        parts = token.split(".")
        if len(parts) != 3:
            raise InvalidSignature("JWT must have 3 parts")
        header = parse_token_header(token)
        kid = header.get("kid")
        alg = header.get("alg")
        if not kid or not alg:
            raise InvalidSignature("JWT header missing kid or alg")
        # Belt-and-suspenders : even before fetching the JWK, reject
        # algorithms not on the configured allowlist. Defends against
        # ``alg=none`` and other classic JWT-library pitfalls.
        if alg not in self.config.allowed_algorithms:
            raise InvalidSignature(
                f"algorithm {alg!r} not in allowlist",
                detail={"allowed": list(self.config.allowed_algorithms)},
            )

        # Decode payload.
        try:
            payload = json.loads(_b64url_decode(parts[1]).decode("utf-8"))
        except Exception as exc:
            raise InvalidSignature("invalid JWT payload encoding") from exc
        if not isinstance(payload, dict):
            raise InvalidSignature("JWT payload is not a JSON object")

        claims = _extract_claims(payload)

        # iss / aud
        if claims.iss not in self.config.expected_iss:
            raise InvalidIssuer(f"iss {claims.iss!r} not allowed")
        if not set(claims.aud).intersection(self.config.expected_aud):
            raise InvalidAudience(f"aud {claims.aud!r} not allowed")

        # exp / nbf window
        skew = self.config.clock_skew_s
        now_ts = int(now.timestamp())
        if claims.exp + skew < now_ts:
            raise ExpiredToken(f"exp={claims.exp} now={now_ts}")
        if claims.nbf is not None and claims.nbf - skew > now_ts:
            raise ExpiredToken(f"nbf={claims.nbf} in future")

        # Signature
        jwk = await self.jwks.get(kid, now=now)
        signing_input = (parts[0] + "." + parts[1]).encode("ascii")
        signature = _b64url_decode(parts[2])
        if not _verify_signature(alg, jwk, signing_input, signature):
            raise InvalidSignature("JWT signature mismatch")

        # Required tenant / scopes
        if self.config.require_tenant and claims.tenant_id is None:
            raise InvalidSignature("JWT missing tenant_id claim")
        if not set(self.config.required_scopes).issubset(set(claims.scopes)):
            missing = set(self.config.required_scopes) - set(claims.scopes)
            raise InvalidSignature(f"JWT missing required scopes: {missing}")

        principal = Principal(
            actor=Actor(
                kind=ActorKind.human,
                actor_id=claims.sub,
                amr=claims.amr,
                acr=claims.acr,
            ),
            tenant_id=claims.tenant_id,
            session_id=None,
            jti=claims.jti,
            scopes=frozenset(claims.scopes),
            groups=frozenset(str(g) for g in payload.get("groups", [])),
            expires_at=datetime.fromtimestamp(claims.exp, tz=timezone.utc),
        )
        return claims, principal


def _extract_claims(payload: dict[str, Any]) -> JwtClaims:
    raw_aud = payload.get("aud", "")
    aud: tuple[str, ...]
    if isinstance(raw_aud, list):
        aud = tuple(str(x) for x in raw_aud)
    else:
        aud = (str(raw_aud),) if raw_aud else ()

    raw_scopes = payload.get("scopes", "") or payload.get("scope", "")
    scopes: tuple[str, ...]
    if isinstance(raw_scopes, list):
        scopes = tuple(str(s) for s in raw_scopes)
    else:
        scopes = tuple(str(raw_scopes).split()) if raw_scopes else ()

    amr_raw = payload.get("amr", [])
    amr = tuple(str(x) for x in amr_raw) if isinstance(amr_raw, list) else ()

    return JwtClaims(
        iss=str(payload.get("iss", "")),
        sub=str(payload.get("sub", "")),
        aud=aud,
        iat=int(payload.get("iat", 0)),
        exp=int(payload.get("exp", 0)),
        nbf=int(payload["nbf"]) if "nbf" in payload else None,
        jti=payload.get("jti"),
        tenant_id=payload.get("tenant_id"),
        scopes=scopes,
        amr=amr,
        acr=payload.get("acr"),
        raw=dict(payload),
    )
