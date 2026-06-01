"""Tests for M11 foundations : crypto, context, fsm."""

from __future__ import annotations

from datetime import datetime, timezone

import pytest

from efe_security.context import (
    Actor,
    ActorKind,
    Obligation,
    Principal,
    Resource,
    SecurityContext,
    empty_context,
)
from efe_security.crypto import (
    ALG_HMAC_SHA256_V1,
    DOMAIN_TAG_AUDIT_CHAIN,
    DOMAIN_TAG_CONSENT_SEAL,
    DOMAIN_TAG_USER_ID,
    DefaultCryptoProvider,
    SecureBytes,
    canonical_json,
    csprng_bytes,
    hmac_seal,
    is_algorithm_deprecated,
    mark_deprecated,
    verify_seal,
)
from efe_security.crypto.primitives import is_using_dev_key, reset_key, set_key
from efe_security.errors import CryptoError
from efe_security.fsm import KEY_ALLOWED, TENANT_ALLOWED, guard, is_allowed


# ---------------------------------------------------------------------------
# Crypto primitives
# ---------------------------------------------------------------------------


def test_canonical_json_sorted() -> None:
    assert canonical_json({"b": 1, "a": 2}) == canonical_json({"a": 2, "b": 1})


def test_canonical_json_rejects_nan() -> None:
    with pytest.raises(ValueError):
        canonical_json({"x": float("nan")})


def test_hmac_seal_roundtrip() -> None:
    payload = {"x": 1}
    seal = hmac_seal(DOMAIN_TAG_AUDIT_CHAIN, payload)
    assert len(seal) == 32
    assert verify_seal(DOMAIN_TAG_AUDIT_CHAIN, payload, seal)


def test_hmac_seal_tag_isolation() -> None:
    payload = {"x": 1}
    s1 = hmac_seal(DOMAIN_TAG_AUDIT_CHAIN, payload)
    s2 = hmac_seal(DOMAIN_TAG_CONSENT_SEAL, payload)
    assert s1 != s2
    assert not verify_seal(DOMAIN_TAG_CONSENT_SEAL, payload, s1)


def test_unknown_tag_raises() -> None:
    with pytest.raises(CryptoError):
        hmac_seal(b"unknown/tag\x00", {"k": 1})


def test_secure_bytes_destroys() -> None:
    sb = SecureBytes(b"secret123")
    assert bytes(sb) == b"secret123"
    sb.destroy()
    with pytest.raises(CryptoError):
        bytes(sb)


def test_secure_bytes_rejects_pickle() -> None:
    import pickle

    sb = SecureBytes(b"x" * 16)
    with pytest.raises(CryptoError):
        pickle.dumps(sb)


def test_csprng_refuses_huge_draws() -> None:
    with pytest.raises(ValueError):
        csprng_bytes(10**9)


def test_agility_marks_deprecated() -> None:
    # Register a new fake algorithm
    from efe_security.crypto.agility import AlgorithmRegistry

    reg = AlgorithmRegistry()
    reg.register("FAKE-v1", category="hmac", description="fake")
    assert not reg.is_deprecated("FAKE-v1")
    reg.mark_deprecated("FAKE-v1")
    assert reg.is_deprecated("FAKE-v1")
    # Unknown algos are deprecated (fail-closed)
    assert reg.is_deprecated("NOT-REGISTERED")


def test_default_hmac_keys_roundtrip_via_provider() -> None:
    p = DefaultCryptoProvider()
    mac = p.hmac(b"hello", tag=DOMAIN_TAG_USER_ID)
    assert p.verify_hmac(b"hello", mac, tag=DOMAIN_TAG_USER_ID)
    assert not p.verify_hmac(b"hello", mac, tag=DOMAIN_TAG_AUDIT_CHAIN)


def test_key_management_reset_and_flag() -> None:
    reset_key()
    # default env-less ; is_using_dev_key may be True
    set_key(b"x" * 32)
    assert not is_using_dev_key()


# ---------------------------------------------------------------------------
# Context
# ---------------------------------------------------------------------------


def test_principal_scope_checks() -> None:
    p = Principal(
        actor=Actor(kind=ActorKind.human, actor_id="u1"),
        tenant_id="t1",
        scopes=frozenset({"a:read", "b:write"}),
    )
    assert p.has_scope("a:read")
    assert not p.has_scope("a:write")
    assert p.has_any("x:y", "a:read")
    assert p.has_all("a:read", "b:write")


def test_security_context_immutable_and_derivable() -> None:
    p = Principal(
        actor=Actor(kind=ActorKind.human, actor_id="u1"),
        tenant_id="t1",
        scopes=frozenset({"audit:read"}),
    )
    ctx = SecurityContext(principal=p, request_id="req-1")
    assert ctx.is_anonymous() is False
    ctx2 = ctx.with_obligations(Obligation(kind="mask_pii"))
    assert ctx.obligations == ()
    assert ctx2.has_obligation("mask_pii")
    assert ctx.require_tenant() == "t1"


def test_with_tenant_rejects_cross_bind() -> None:
    p = Principal(
        actor=Actor(kind=ActorKind.human, actor_id="u1"),
        tenant_id="t1",
        scopes=frozenset(),
    )
    ctx = SecurityContext(principal=p, request_id="r")
    with pytest.raises(ValueError):
        ctx.with_tenant("t2")


def test_empty_context_is_anonymous() -> None:
    ctx = empty_context("req-1")
    assert ctx.is_anonymous()


# ---------------------------------------------------------------------------
# FSM
# ---------------------------------------------------------------------------


def test_tenant_fsm_allowed_onboard() -> None:
    assert is_allowed("nil", "provisioning", TENANT_ALLOWED)
    assert is_allowed("provisioning", "active", TENANT_ALLOWED)


def test_key_fsm_rejects_random() -> None:
    from efe_security.errors import SecurityError

    with pytest.raises(SecurityError):
        guard("retired", "active", KEY_ALLOWED, aggregate="key")
