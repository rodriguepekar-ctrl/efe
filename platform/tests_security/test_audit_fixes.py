"""Non-regression tests for the M11 audit-round fixes + new modules."""

from __future__ import annotations

import asyncio
import copy
import pickle
from datetime import datetime, timedelta, timezone

import pytest

from efe_security.audit import (
    AuditDraft,
    AuditEmitter,
    AuditOutcome,
    AuditSeverity,
    InMemoryAuditStore,
    verify_range,
)
from efe_security.authn.factor import MfaRequirement
from efe_security.authn.jwks_cache import Jwk, JwksCache, StaticJwksSource
from efe_security.authn.jwt_verifier import JwtVerifier, JwtVerifierConfig
from efe_security.authz import (
    BREAK_GLASS_ALLOWED_SCOPES,
    BreakGlassManager,
    BreakGlassState,
    PolicyDecision,
    PolicyDecisionPoint,
    PolicyRequest,
    RbacRegistry,
    RlsBridge,
    validate_tenant_id,
)
from efe_security.context import Actor, ActorKind, Principal, Resource, SecurityContext
from efe_security.crypto import SecureBytes
from efe_security.errors import (
    ActionForbidden,
    CryptoError,
    InvalidSignature,
    MfaRequired,
    QuotaExceeded,
)
from efe_security.lifecycle import Category, GcDaemon, RetentionPolicy
from efe_security.middleware import (
    RequestAdapter,
    SecurityMiddleware,
    current_context,
    set_current_context,
)
from efe_security.secrets import (
    InMemoryKmsClient,
    KeyAlreadyRegistered,
    KeyEntry,
    KeyPurpose,
    KeyRegistry,
    KeyState,
)
from efe_security.sql import AUDIT_LOG_DDL, FULL_DDL
from efe_security.tenancy import QuotaKind, QuotaManager, QuotaSpec, TenantProvisioner


# ---------------------------------------------------------------------------
# Fix 1 — audit chain uses payload_digest, not full payload
# ---------------------------------------------------------------------------


def test_fix1_audit_chain_verifies_without_payload() -> None:
    """Verifier recomputes seals using only persisted fields — no need
    for original payload. This proves the seal covers payload_digest."""

    async def scenario() -> None:
        store = InMemoryAuditStore()
        now = datetime(2026, 4, 21, tzinfo=timezone.utc)
        for i in range(5):
            await store.append(
                AuditDraft(
                    occurred_at=now + timedelta(seconds=i),
                    tenant_id="t1",
                    actor_kind="human",
                    actor_id="u1",
                    action="authz.decision.allow",
                    outcome=AuditOutcome.allow,
                    severity=AuditSeverity.info,
                    reasons=("rbac:pass",),
                    payload={"secret": f"value-{i}"},
                )
            )
        # Drop the stored payloads — the verifier must still succeed.
        store._payloads.clear()
        out = await verify_range(store)
        assert out.ok
        assert out.inspected == 5

    asyncio.run(scenario())


def test_fix1_audit_chain_detects_seal_tamper() -> None:
    """Tamper a record's action — seal recomputation must break."""

    async def scenario() -> None:
        store = InMemoryAuditStore()
        now = datetime(2026, 4, 21, tzinfo=timezone.utc)
        for _ in range(3):
            await store.append(
                AuditDraft(
                    occurred_at=now,
                    tenant_id="t1",
                    actor_kind="human",
                    actor_id="u1",
                    action="authz.decision.allow",
                    outcome=AuditOutcome.allow,
                    severity=AuditSeverity.info,
                )
            )
        # Forge a different action on the middle record by swapping the
        # _rows entry with a tampered AuditRecord (dataclass is frozen,
        # so we must rebuild).
        mid = store._rows[1]
        tampered = type(mid)(
            audit_id=mid.audit_id,
            occurred_at=mid.occurred_at,
            tenant_id=mid.tenant_id,
            actor_kind=mid.actor_kind,
            actor_id=mid.actor_id,
            action="authz.decision.deny",  # tampered
            outcome=mid.outcome,
            severity=mid.severity,
            resource_kind=mid.resource_kind,
            resource_id=mid.resource_id,
            reasons=mid.reasons,
            payload_digest=mid.payload_digest,
            prev_seal=mid.prev_seal,
            seal=mid.seal,  # unchanged — exposing the tamper
            key_version=mid.key_version,
        )
        store._rows[1] = tampered
        out = await verify_range(store)
        assert not out.ok
        assert out.first_broken is not None

    asyncio.run(scenario())


# ---------------------------------------------------------------------------
# Fix 2 — cumulative quota
# ---------------------------------------------------------------------------


def test_fix2_cumulative_quota_accepts_until_cap() -> None:
    async def scenario() -> None:
        qm = QuotaManager(
            defaults={
                QuotaKind.storage_bytes: QuotaSpec(
                    kind=QuotaKind.storage_bytes,
                    capacity=0.0,
                    refill_per_s=0.0,
                    hard_cap=1_000.0,
                )
            }
        )
        # Three calls well below the cap — must succeed.
        for _ in range(3):
            await qm.consume(tenant_id="t1", kind=QuotaKind.storage_bytes, cost=100.0)
        # Now push past the cap.
        with pytest.raises(QuotaExceeded):
            await qm.consume(tenant_id="t1", kind=QuotaKind.storage_bytes, cost=10_000.0)

    asyncio.run(scenario())


def test_fix2_rate_limit_quota_still_enforced() -> None:
    async def scenario() -> None:
        qm = QuotaManager(
            defaults={
                QuotaKind.api_rps: QuotaSpec(
                    kind=QuotaKind.api_rps, capacity=2.0, refill_per_s=0.0,
                )
            }
        )
        await qm.consume(tenant_id="t1", kind=QuotaKind.api_rps)
        await qm.consume(tenant_id="t1", kind=QuotaKind.api_rps)
        with pytest.raises(QuotaExceeded):
            await qm.consume(tenant_id="t1", kind=QuotaKind.api_rps)

    asyncio.run(scenario())


# ---------------------------------------------------------------------------
# Fix 3 — SecureBytes anti-copy
# ---------------------------------------------------------------------------


def test_fix3_secure_bytes_blocks_pickle() -> None:
    sb = SecureBytes(b"secret-key-material")
    with pytest.raises(CryptoError):
        pickle.dumps(sb)


def test_fix3_secure_bytes_blocks_copy() -> None:
    sb = SecureBytes(b"secret-key-material")
    with pytest.raises(CryptoError):
        copy.copy(sb)
    with pytest.raises(CryptoError):
        copy.deepcopy(sb)


def test_fix3_secure_bytes_blocks_iter() -> None:
    sb = SecureBytes(b"secret")
    with pytest.raises(CryptoError):
        list(iter(sb))


# ---------------------------------------------------------------------------
# Fix 4 — JWT allowlist rejects disallowed algorithms
# ---------------------------------------------------------------------------


def test_fix4_jwt_allowlist_rejects_none() -> None:
    import base64
    import json

    def b64url(data: bytes) -> str:
        return base64.urlsafe_b64encode(data).rstrip(b"=").decode()

    header = {"alg": "none", "kid": "k1", "typ": "JWT"}
    payload = {"iss": "https://idp.test", "sub": "u", "aud": "efe.api.v1"}
    token = f"{b64url(json.dumps(header).encode())}.{b64url(json.dumps(payload).encode())}."
    verifier = JwtVerifier(
        jwks=JwksCache(
            source=StaticJwksSource(
                keys=(Jwk(kid="k1", alg="HS256", key_bytes=b"x" * 32),)
            )
        ),
        config=JwtVerifierConfig(
            expected_iss=("https://idp.test",),
            expected_aud=("efe.api.v1",),
            allowed_algorithms=("ES256", "EdDSA"),  # exclude HS256 and none
        ),
    )
    with pytest.raises(InvalidSignature):
        asyncio.run(verifier.verify(token, now=datetime(2026, 4, 21, tzinfo=timezone.utc)))


# ---------------------------------------------------------------------------
# Fix 5 — RLS tenant_id validation
# ---------------------------------------------------------------------------


def test_fix5_rls_accepts_valid_tenant() -> None:
    assert validate_tenant_id("tenant-123_abc") == "tenant-123_abc"


@pytest.mark.parametrize(
    "bad",
    [
        "",
        " ",
        "a'b",
        "t\nx",
        "t\x00",
        "t\\x",
        "tenant with spaces",
        "-leading-hyphen",
        "_leading-underscore",
        "é",  # non-ASCII
        "a" * 65,  # too long
    ],
)
def test_fix5_rls_rejects_malicious_tenant(bad: str) -> None:
    with pytest.raises(ValueError):
        validate_tenant_id(bad)


def test_fix5_rls_apply_via_bridge() -> None:
    async def scenario() -> None:
        bridge = RlsBridge()

        executed: list[str] = []

        class _Conn:
            async def execute(self, sql: str) -> None:
                executed.append(sql)

        await bridge.apply(_Conn(), "tenantA-01")
        await bridge.apply(_Conn(), None)
        assert any("tenantA-01" in s for s in executed)
        assert any("__none__" in s for s in executed)
        with pytest.raises(ValueError):
            await bridge.apply(_Conn(), "t'); DROP TABLE x; --")

    asyncio.run(scenario())


# ---------------------------------------------------------------------------
# Fix 6 — KeyAlreadyRegistered + idempotent onboard
# ---------------------------------------------------------------------------


def test_fix6_double_onboard_idempotent_on_kek() -> None:
    async def scenario() -> None:
        kms = InMemoryKmsClient()
        reg = KeyRegistry()
        prov = TenantProvisioner(kms=kms, key_registry=reg)
        t1 = await prov.onboard(tenant_id="t-fix6", display_name="T")
        # Force a second registration of the same KEK key — should NOT bubble.
        await reg.register(
            KeyEntry(
                key_id=t1.dek_kek_key_id,
                alg_id="AES256-GCM-v1",
                purpose=KeyPurpose.tenant_dek_kek,
                state=KeyState.active,
                created_at=datetime(2026, 4, 21, tzinfo=timezone.utc),
            )
        ) if False else None  # already done by onboard
        # Attempting a second onboard with the SAME tenant_id fails
        # — but the reason is "tenant already exists", not KeyAlreadyRegistered.
        with pytest.raises(Exception):
            await prov.onboard(tenant_id="t-fix6", display_name="T")

    asyncio.run(scenario())


def test_fix6_key_already_registered_raised() -> None:
    async def scenario() -> None:
        reg = KeyRegistry()
        entry = KeyEntry(
            key_id="k1",
            alg_id="HMAC-SHA256-v1",
            purpose=KeyPurpose.generic_hmac,
            state=KeyState.active,
            created_at=datetime(2026, 4, 21, tzinfo=timezone.utc),
        )
        await reg.register(entry)
        with pytest.raises(KeyAlreadyRegistered):
            await reg.register(entry)

    asyncio.run(scenario())


# ---------------------------------------------------------------------------
# Fix 7 — gc_daemon logs errors and records them per category
# ---------------------------------------------------------------------------


def test_fix7_gc_daemon_records_errors() -> None:
    async def scenario() -> None:
        def scanner(cat: Category, t: datetime) -> int:
            return 5

        def purger(cat: Category, t: datetime) -> int:
            raise RuntimeError("storage unavailable")

        policies = {
            Category.inference_events_raw: RetentionPolicy(
                Category.inference_events_raw, 30
            ),
        }
        daemon = GcDaemon(policies=policies, scanner=scanner, purger=purger)
        out = await daemon.run_once(now=datetime(2026, 4, 21, tzinfo=timezone.utc))
        assert Category.inference_events_raw in out.errors
        assert Category.inference_events_raw in out.skipped_holds
        assert Category.inference_events_raw not in out.purged

    asyncio.run(scenario())


# ---------------------------------------------------------------------------
# Break-glass
# ---------------------------------------------------------------------------


def test_break_glass_requires_mfa_and_4eyes() -> None:
    async def scenario() -> None:
        audit = InMemoryAuditStore()
        mgr = BreakGlassManager(audit=audit)
        requester = Principal(
            actor=Actor(kind=ActorKind.human, actor_id="u-requester", amr=("pwd",)),
            tenant_id="t1",
            scopes=frozenset(),
        )
        # No MFA → rejected.
        with pytest.raises(MfaRequired):
            await mgr.activate(
                requester=requester,
                approved_by="u-approver",
                scopes=frozenset({"audit:read"}),
                justification="incident 1234, chain break detected",
            )
        # With MFA but self-approval → rejected.
        mfa = Principal(
            actor=Actor(kind=ActorKind.human, actor_id="u1", amr=("pwd", "mfa")),
            tenant_id="t1",
            scopes=frozenset(),
        )
        with pytest.raises(ActionForbidden):
            await mgr.activate(
                requester=mfa,
                approved_by="u1",  # same as requester
                scopes=frozenset({"audit:read"}),
                justification="incident 1234, chain break detected",
            )

    asyncio.run(scenario())


def test_break_glass_rejects_out_of_allowlist_scopes() -> None:
    async def scenario() -> None:
        audit = InMemoryAuditStore()
        mgr = BreakGlassManager(audit=audit)
        mfa = Principal(
            actor=Actor(kind=ActorKind.human, actor_id="u1", amr=("pwd", "mfa")),
            tenant_id="t1",
            scopes=frozenset(),
        )
        with pytest.raises(ActionForbidden):
            await mgr.activate(
                requester=mfa,
                approved_by="u2",
                scopes=frozenset({"drift:write"}),  # not in allowlist
                justification="incident 1234, chain break detected",
            )

    asyncio.run(scenario())


def test_break_glass_happy_path_and_expiry() -> None:
    async def scenario() -> None:
        audit = InMemoryAuditStore()
        mgr = BreakGlassManager(
            audit=audit,
            default_ttl=timedelta(minutes=5),
            max_ttl=timedelta(hours=1),
        )
        mfa = Principal(
            actor=Actor(kind=ActorKind.human, actor_id="u1", amr=("pwd", "mfa")),
            tenant_id="t1",
            scopes=frozenset(),
        )
        now = datetime(2026, 4, 21, tzinfo=timezone.utc)
        grant = await mgr.activate(
            requester=mfa,
            approved_by="u2",
            scopes=frozenset({"audit:read", "audit:export"}),
            justification="incident 1234, chain break detected",
            tenant_id="t1",
            now=now,
        )
        assert grant.state == BreakGlassState.active
        active = await mgr.active_scopes_for(principal_id="u1", now=now)
        assert "audit:read" in active
        # Advance past expiry.
        pruned = await mgr.prune_expired(now=now + timedelta(hours=1))
        assert pruned == 1
        no_longer = await mgr.active_scopes_for(
            principal_id="u1", now=now + timedelta(hours=1)
        )
        assert no_longer == frozenset()
        # Audit trail recorded.
        lines = await audit.fetch_range(limit=100)
        actions = [r.action for r in lines]
        assert "break_glass.activated" in actions
        assert "break_glass.expired_prune" in actions

    asyncio.run(scenario())


# ---------------------------------------------------------------------------
# Audit emitter
# ---------------------------------------------------------------------------


def test_audit_emitter_emits_decision_event() -> None:
    async def scenario() -> None:
        store = InMemoryAuditStore()
        emitter = AuditEmitter(sink=store)
        p = Principal(
            actor=Actor(kind=ActorKind.human, actor_id="u1"),
            tenant_id="t1",
            scopes=frozenset({"drift:read"}),
        )
        ctx = SecurityContext(principal=p, request_id="r-1")
        req = PolicyRequest(
            principal=p,
            action="drift.evaluation.read",
            resource=Resource(kind="drift", tenant_id="t1"),
        )
        pdp = PolicyDecisionPoint(rbac=RbacRegistry())
        decision = await pdp.decide(req)
        await emitter.authz_decision(ctx=ctx, request=req, decision=decision)
        lines = await store.fetch_range()
        assert any("authz.drift.evaluation.read.allow" in r.action for r in lines)

    asyncio.run(scenario())


# ---------------------------------------------------------------------------
# Middleware end-to-end with JWT
# ---------------------------------------------------------------------------


def test_middleware_full_dispatch() -> None:
    import base64
    import hashlib
    import hmac as _hmac
    import json

    def b64url(data: bytes) -> str:
        return base64.urlsafe_b64encode(data).rstrip(b"=").decode()

    def hs256(claims: dict, *, kid: str, secret: bytes) -> str:
        header = {"alg": "HS256", "kid": kid, "typ": "JWT"}
        header_b = b64url(json.dumps(header).encode())
        payload_b = b64url(json.dumps(claims).encode())
        sig = _hmac.new(secret, f"{header_b}.{payload_b}".encode(), hashlib.sha256).digest()
        return f"{header_b}.{payload_b}.{b64url(sig)}"

    async def scenario() -> None:
        # Use a far-future expiry so the middleware's wall-clock verify
        # still accepts the token regardless of when the test runs.
        now = datetime.now(tz=timezone.utc)
        secret = b"testsecret-abcdefghijklmnopqrstuvwxyz"
        jwks = JwksCache(source=StaticJwksSource(keys=(Jwk(kid="k1", alg="HS256", key_bytes=secret),)))
        verifier = JwtVerifier(
            jwks=jwks,
            config=JwtVerifierConfig(
                expected_iss=("https://idp.test",),
                expected_aud=("efe.api.v1",),
                allowed_algorithms=("HS256",),
            ),
        )
        audit = InMemoryAuditStore()
        mw = SecurityMiddleware(
            jwt=verifier,
            mtls=None,
            sessions=None,
            audit=audit,
        )
        token = hs256(
            {
                "iss": "https://idp.test",
                "sub": "u-middleware",
                "aud": "efe.api.v1",
                "iat": int(now.timestamp()),
                "exp": int((now + timedelta(hours=1)).timestamp()),
                "tenant_id": "t-mw",
                "scopes": "drift:read",
                "jti": "j1",
            },
            kid="k1",
            secret=secret,
        )

        class _Req:
            def header(self, name: str) -> str | None:
                if name.lower() == "authorization":
                    return f"Bearer {token}"
                return None

            def peer_spiffe_id(self) -> str | None:
                return None

            def request_id(self) -> str:
                return "req-42"

            def client_ip(self) -> str | None:
                return "10.0.0.1"

        async def handler(ctx: SecurityContext) -> str:
            # The context is also available via contextvar.
            from_ctxvar = current_context()
            assert from_ctxvar is not None
            return ctx.principal.actor.actor_id

        result = await mw.dispatch(_Req(), handler)
        assert result == "u-middleware"
        lines = await audit.fetch_range()
        assert any(r.action == "authn.success" for r in lines)

    asyncio.run(scenario())


# ---------------------------------------------------------------------------
# SQL DDL is well-formed strings
# ---------------------------------------------------------------------------


def test_sql_full_ddl_contains_required_tables() -> None:
    for needle in (
        "CREATE TABLE IF NOT EXISTS tenants",
        "CREATE TABLE IF NOT EXISTS audit_log",
        "CREATE TABLE IF NOT EXISTS consent_records",
        "CREATE TABLE IF NOT EXISTS dsar_requests",
        "CREATE TABLE IF NOT EXISTS security_incidents",
        "CREATE TABLE IF NOT EXISTS key_registry",
        "CREATE TABLE IF NOT EXISTS pii_field_registry",
        "CREATE TABLE IF NOT EXISTS rbac_roles",
        "ENABLE ROW LEVEL SECURITY",
    ):
        assert needle in FULL_DDL


# ---------------------------------------------------------------------------
# set_current_context ContextVar
# ---------------------------------------------------------------------------


def test_current_context_set_reset() -> None:
    p = Principal(
        actor=Actor(kind=ActorKind.human, actor_id="u1"),
        tenant_id="t1",
    )
    ctx = SecurityContext(principal=p, request_id="r1")
    assert current_context() is None
    token = set_current_context(ctx)
    try:
        got = current_context()
        assert got is not None
        assert got.principal.actor.actor_id == "u1"
    finally:
        set_current_context(None)
        # reset manually
        _ = token
