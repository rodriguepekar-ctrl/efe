"""OIDC auth bridge pin tests (M16)."""
from __future__ import annotations

import base64
import json
import os
import time

import pytest

from mcp_server.auth import (
    AuthError,
    authorization_header,
    load_auth_from_env,
    parse_jwt_unverified,
)


def _make_jwt(claims: dict) -> str:
    """Build an unsigned JWT for testing — signature is the literal 'sig'."""
    header = base64.urlsafe_b64encode(json.dumps({"alg": "RS256"}).encode()).decode().rstrip("=")
    payload = base64.urlsafe_b64encode(json.dumps(claims).encode()).decode().rstrip("=")
    return f"{header}.{payload}.sig"


def test_load_auth_missing_env_var_raises(monkeypatch):
    monkeypatch.delenv("EFE_OIDC_TOKEN", raising=False)
    with pytest.raises(AuthError):
        load_auth_from_env()


def test_load_auth_empty_env_var_raises(monkeypatch):
    monkeypatch.setenv("EFE_OIDC_TOKEN", "   ")
    with pytest.raises(AuthError):
        load_auth_from_env()


def test_load_auth_jwt_extracts_claims(monkeypatch):
    token = _make_jwt({"sub": "alice@example.com", "tenant_id": "acme", "exp": 9999999999})
    monkeypatch.setenv("EFE_OIDC_TOKEN", token)
    ctx = load_auth_from_env()
    assert ctx.tenant_id == "acme"
    assert ctx.sub == "alice@example.com"
    assert ctx.exp == 9999999999
    assert ctx.is_expired() is False


def test_load_auth_opaque_token_passes_through(monkeypatch):
    monkeypatch.setenv("EFE_OIDC_TOKEN", "playground-acme-admin-1234567890")
    ctx = load_auth_from_env()
    assert ctx.raw_token == "playground-acme-admin-1234567890"
    assert ctx.tenant_id is None  # not a JWT
    assert ctx.is_expired() is False


def test_redacted_token_format(monkeypatch):
    monkeypatch.setenv("EFE_OIDC_TOKEN", "abcdefghijklmnopqrstuv1234")
    ctx = load_auth_from_env()
    redacted = ctx.redacted_token()
    assert redacted.startswith("abcd")
    assert redacted.endswith("1234")
    assert "***" in redacted


def test_authorization_header_uses_bearer(monkeypatch):
    monkeypatch.setenv("EFE_OIDC_TOKEN", "tok-xyz-abcdefghij")
    ctx = load_auth_from_env()
    headers = authorization_header(ctx)
    assert headers["Authorization"] == "Bearer tok-xyz-abcdefghij"


def test_expired_jwt_detected(monkeypatch):
    expired = _make_jwt({"sub": "x", "tenant_id": "t", "exp": int(time.time()) - 100})
    monkeypatch.setenv("EFE_OIDC_TOKEN", expired)
    ctx = load_auth_from_env()
    assert ctx.is_expired() is True


def test_malformed_jwt_falls_back_to_opaque(monkeypatch):
    """A 3-segment string with a non-JSON payload should be treated as opaque
    (allow_opaque=True default)."""
    bogus = "AA.BB.CC"  # parts but BB is not JSON
    monkeypatch.setenv("EFE_OIDC_TOKEN", bogus)
    ctx = load_auth_from_env(allow_opaque=True)
    assert ctx.raw_token == bogus
    assert ctx.tenant_id is None


def test_parse_jwt_unverified_rejects_two_segment_input():
    with pytest.raises(AuthError):
        parse_jwt_unverified("only.two")
