"""Tests for AuthN (JWT/mTLS/session) and AuthZ (PDP/RBAC/ABAC)."""

from __future__ import annotations

import asyncio
import base64
import json
from datetime import datetime, timedelta, timezone

import hashlib
import hmac as _hmac
import pytest

from efe_security.authn import (
    Jwk,
    JwksCache,
    JwtClaims,
    JwtVerifier,
    JwtVerifierConfig,
    MtlsVerifier,
    OpaqueSession,
    SessionStore,
    StaticJwksSource,
    new_session_token,
    parse_spiffe_id,
    parse_token_header,
)
from efe_security.authn.factor import MfaRequirement, require_mfa
from efe_security.authz import (
    AbacCondition,
    AbacRule,
    AbacRuleSet,
    PolicyDecision,
    PolicyDecisionPoint,
    PolicyRequest,
    RbacRegistry,
    RoleGrant,
    requires,
)
from efe_security.authz.pdp import ACTION_SCOPES
from efe_security.context import (
    Actor,
    ActorKind,
    Obligation,
    Principal,
    Resource,
    SecurityContext,
)
from efe_security.errors import (
    ExpiredToken,
    InvalidIssuer,
    InvalidSignature,
    MfaRequired,
    MissingToken,
    ScopeMissing,
    SessionRevoked,
)


# ---------------------------------------------------------------------------
# JWT helpers (HS256 for tests)
# ---------------------------------------------------------------------------


def _b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def _make_hs256_jwt(claims: dict, *, kid: str, secret: bytes) -> str:
    header = {"alg": "HS256", "kid": kid, "typ": "JWT"}
    header_b = _b64url(json.dumps(header).encode())
    payload_b = _b64url(json.dumps(claims).encode())
    signing_input = f"{header_b}.{payload_b}".encode()
    sig = _hmac.new(secret, signing_input, hashlib.sha256).digest()
    return f"{header_b}.{payload_b}.{_b64url(sig)}"


@pytest.fixture()
def now() -> datetime:
    return datetime(2026, 4, 21, 12, 0, 0, tzinfo=timezone.utc)


@pytest.fixture()
def jwks_cache() -> JwksCache:
    jwk = Jwk(kid="k1", alg="HS256", key_bytes=b"supersecret-32bytes-key-000000")
    source = StaticJwksSource(keys=(jwk,))
    return JwksCache(source=source)


@pytest.fixture()
def verifier(jwks_cache: JwksCache) -> JwtVerifier:
    return JwtVerifier(
        jwks=jwks_cache,
        config=JwtVerifierConfig(
            expected_iss=("https://idp.test",),
            expected_aud=("efe.api.v1",),
        ),
    )


# ---------------------------------------------------------------------------
# JWT tests
# ---------------------------------------------------------------------------


def test_parse_token_header_ok() -> None:
    claims = {"iss": "x", "sub": "u"}
    token = _make_hs256_jwt(claims, kid="k1", secret=b"supersecret-32bytes-key-000000")
    header = parse_token_header(token)
    assert header["kid"] == "k1"


def test_jwt_verifier_accepts_valid(verifier: JwtVerifier, now: datetime) -> None:
    claims = {
        "iss": "https://idp.test",
        "sub": "u1",
        "aud": "efe.api.v1",
        "iat": int(now.timestamp()),
        "exp": int((now + timedelta(hours=1)).timestamp()),
        "tenant_id": "t1",
        "scopes": "drift:read sched:read",
        "jti": "j1",
        "amr": ["pwd"],
    }
    token = _make_hs256_jwt(claims, kid="k1", secret=b"supersecret-32bytes-key-000000")
    parsed, principal = asyncio.run(verifier.verify(token, now=now))
    assert principal.tenant_id == "t1"
    assert principal.jti == "j1"
    assert "drift:read" in principal.scopes


def test_jwt_verifier_rejects_wrong_issuer(verifier: JwtVerifier, now: datetime) -> None:
    claims = {
        "iss": "https://evil.example",
        "sub": "u", "aud": "efe.api.v1",
        "iat": int(now.timestamp()),
        "exp": int((now + timedelta(hours=1)).timestamp()),
        "tenant_id": "t1", "scopes": "",
    }
    token = _make_hs256_jwt(claims, kid="k1", secret=b"supersecret-32bytes-key-000000")
    with pytest.raises(InvalidIssuer):
        asyncio.run(verifier.verify(token, now=now))


def test_jwt_verifier_rejects_expired(verifier: JwtVerifier, now: datetime) -> None:
    claims = {
        "iss": "https://idp.test", "sub": "u", "aud": "efe.api.v1",
        "iat": int((now - timedelta(hours=2)).timestamp()),
        "exp": int((now - timedelta(hours=1)).timestamp()),
        "tenant_id": "t1", "scopes": "",
    }
    token = _make_hs256_jwt(claims, kid="k1", secret=b"supersecret-32bytes-key-000000")
    with pytest.raises(ExpiredToken):
        asyncio.run(verifier.verify(token, now=now))


def test_jwt_verifier_rejects_missing_tenant(verifier: JwtVerifier, now: datetime) -> None:
    claims = {
        "iss": "https://idp.test", "sub": "u", "aud": "efe.api.v1",
        "iat": int(now.timestamp()),
        "exp": int((now + timedelta(hours=1)).timestamp()),
        "scopes": "",
    }
    token = _make_hs256_jwt(claims, kid="k1", secret=b"supersecret-32bytes-key-000000")
    with pytest.raises(InvalidSignature):
        asyncio.run(verifier.verify(token, now=now))


def test_missing_token_raises() -> None:
    with pytest.raises(MissingToken):
        parse_token_header("")


# ---------------------------------------------------------------------------
# mTLS
# ---------------------------------------------------------------------------


def test_parse_spiffe_id_ok() -> None:
    sid = parse_spiffe_id("spiffe://efe.local/prod/ingester")
    assert sid.trust_domain == "efe.local"
    assert sid.service_account == "ingester"


def test_parse_spiffe_id_bad() -> None:
    with pytest.raises(InvalidIssuer):
        parse_spiffe_id("http://not.spiffe/x/y")


def test_mtls_verifier_builds_principal() -> None:
    v = MtlsVerifier(
        trusted_trust_domain="efe.local",
        default_scopes_by_sa={"ingester": frozenset({"ingest:write"})},
    )
    p = v.build_principal(spiffe_uri="spiffe://efe.local/prod/ingester", tenant_id="t1")
    assert p.actor.kind == ActorKind.service
    assert "ingest:write" in p.scopes


# ---------------------------------------------------------------------------
# Session
# ---------------------------------------------------------------------------


def test_opaque_session_issue_and_resolve(now: datetime) -> None:
    store = SessionStore()
    p = Principal(
        actor=Actor(kind=ActorKind.human, actor_id="u1"),
        tenant_id="t1",
    )
    token, session = asyncio.run(store.issue(p, ttl=timedelta(minutes=10), now=now))
    got = asyncio.run(store.resolve(token, now=now))
    assert got.session_id == session.session_id


def test_opaque_session_revoke(now: datetime) -> None:
    store = SessionStore()
    p = Principal(
        actor=Actor(kind=ActorKind.human, actor_id="u1"),
        tenant_id="t1",
    )
    token, session = asyncio.run(store.issue(p, ttl=timedelta(minutes=10), now=now))
    asyncio.run(store.revoke(session.session_id, now=now))
    with pytest.raises(SessionRevoked):
        asyncio.run(store.resolve(token, now=now))


# ---------------------------------------------------------------------------
# MFA
# ---------------------------------------------------------------------------


def test_require_mfa_ok() -> None:
    p = Principal(
        actor=Actor(kind=ActorKind.human, actor_id="u1", amr=("pwd", "mfa")),
        tenant_id="t1",
    )
    require_mfa(p, MfaRequirement(required_factors=frozenset({"mfa"})))


def test_require_mfa_missing() -> None:
    p = Principal(
        actor=Actor(kind=ActorKind.human, actor_id="u1", amr=("pwd",)),
        tenant_id="t1",
    )
    with pytest.raises(MfaRequired):
        require_mfa(p, MfaRequirement(required_factors=frozenset({"mfa"})))


# ---------------------------------------------------------------------------
# PDP / RBAC
# ---------------------------------------------------------------------------


def test_pdp_denies_anonymous() -> None:
    pdp = PolicyDecisionPoint(rbac=RbacRegistry())
    p = Principal(
        actor=Actor(kind=ActorKind.anonymous, actor_id="anon"),
        tenant_id=None,
    )
    req = PolicyRequest(principal=p, action="schedule.create")
    dec = asyncio.run(pdp.decide(req))
    assert not dec.allow


def test_pdp_allows_with_scope() -> None:
    rbac = RbacRegistry()
    pdp = PolicyDecisionPoint(rbac=rbac)
    p = Principal(
        actor=Actor(kind=ActorKind.human, actor_id="u1"),
        tenant_id="t1",
        scopes=frozenset({"drift:read"}),
    )
    req = PolicyRequest(
        principal=p,
        action="drift.evaluation.read",
        resource=Resource(kind="drift", tenant_id="t1"),
    )
    dec = asyncio.run(pdp.decide(req))
    assert dec.allow


def test_pdp_denies_on_tenant_mismatch() -> None:
    pdp = PolicyDecisionPoint(rbac=RbacRegistry())
    p = Principal(
        actor=Actor(kind=ActorKind.human, actor_id="u1"),
        tenant_id="t1",
        scopes=frozenset({"drift:read"}),
    )
    req = PolicyRequest(
        principal=p,
        action="drift.evaluation.read",
        resource=Resource(kind="drift", tenant_id="t2"),
    )
    dec = asyncio.run(pdp.decide(req))
    assert not dec.allow
    assert any("tenant_mismatch" in r for r in dec.reasons)


def test_pdp_denies_unknown_action() -> None:
    pdp = PolicyDecisionPoint(rbac=RbacRegistry())
    p = Principal(
        actor=Actor(kind=ActorKind.human, actor_id="u1"),
        tenant_id="t1",
        scopes=frozenset({"audit:read"}),
    )
    req = PolicyRequest(principal=p, action="unknown.action")
    dec = asyncio.run(pdp.decide(req))
    assert not dec.allow


def test_pdp_missing_scope() -> None:
    pdp = PolicyDecisionPoint(rbac=RbacRegistry())
    p = Principal(
        actor=Actor(kind=ActorKind.human, actor_id="u1"),
        tenant_id="t1",
        scopes=frozenset(),
    )
    req = PolicyRequest(principal=p, action="audit.export")
    dec = asyncio.run(pdp.decide(req))
    assert not dec.allow
    assert any("rbac:missing" in r for r in dec.reasons)


def test_rbac_scopes_merge_from_registry() -> None:
    rbac = RbacRegistry()
    asyncio.run(
        rbac.grant(
            RoleGrant(
                tenant_id="t1",
                principal_id="u1",
                role="admin",
                granted_at=datetime(2026, 4, 1, tzinfo=timezone.utc),
                granted_by="system",
            )
        )
    )
    scopes = asyncio.run(rbac.scopes_for(tenant_id="t1", principal_id="u1"))
    assert "audit:read" in scopes
    assert "sched:admin" in scopes


# ---------------------------------------------------------------------------
# ABAC
# ---------------------------------------------------------------------------


def test_abac_owner_only_rule() -> None:
    rule = AbacRule(
        name="owner_only",
        action_pattern="schedule.*",
        conditions=(
            AbacCondition(
                path="resource.owner_id",
                comparator="eq",
                value="u1",
            ),
        ),
        allow=True,
    )
    rs = AbacRuleSet(rules=(rule,))
    rbac = RbacRegistry()
    asyncio.run(
        rbac.grant(
            RoleGrant(
                tenant_id="t1", principal_id="u1", role="operator",
                granted_at=datetime(2026, 4, 1, tzinfo=timezone.utc), granted_by="system",
            )
        )
    )
    pdp = PolicyDecisionPoint(rbac=rbac, abac=rs)
    p = Principal(
        actor=Actor(kind=ActorKind.human, actor_id="u1"),
        tenant_id="t1",
        scopes=frozenset({"sched:write"}),
    )
    dec_allow = asyncio.run(
        pdp.decide(
            PolicyRequest(
                principal=p,
                action="schedule.create",
                resource=Resource(kind="schedule", tenant_id="t1", owner_id="u1"),
            )
        )
    )
    assert dec_allow.allow


# ---------------------------------------------------------------------------
# @requires decorator
# ---------------------------------------------------------------------------


def test_requires_decorator_denies_then_allows() -> None:
    rbac = RbacRegistry()
    pdp = PolicyDecisionPoint(rbac=rbac)

    @requires(action="drift.evaluation.read", resource_kind="drift")
    async def handler(ctx: SecurityContext, pdp: PolicyDecisionPoint) -> str:
        return f"ok-{ctx.principal.actor.actor_id}"

    p_no = Principal(
        actor=Actor(kind=ActorKind.human, actor_id="u1"),
        tenant_id="t1",
        scopes=frozenset(),
    )
    ctx_no = SecurityContext(principal=p_no, request_id="r1")
    with pytest.raises(ScopeMissing):
        asyncio.run(handler(ctx=ctx_no, pdp=pdp))

    p_yes = Principal(
        actor=Actor(kind=ActorKind.human, actor_id="u1"),
        tenant_id="t1",
        scopes=frozenset({"drift:read"}),
    )
    ctx_yes = SecurityContext(principal=p_yes, request_id="r2")
    assert asyncio.run(handler(ctx=ctx_yes, pdp=pdp)) == "ok-u1"


# ---------------------------------------------------------------------------
# Mapping completeness
# ---------------------------------------------------------------------------


def test_action_scopes_map_not_empty() -> None:
    assert "drift.evaluation.read" in ACTION_SCOPES
    assert "audit.export" in ACTION_SCOPES
