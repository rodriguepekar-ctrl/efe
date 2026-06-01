"""Core scaffolding tests — version, errors, clock, ids, hashing, config."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone

import pytest

from efe_gate.clock import FakeClock, SystemClock
from efe_gate.config import GateSettings, SignatureSettings
from efe_gate.errors import (
    ConfigError,
    GateError,
    InvalidTransition,
    NotFound,
    TenantMismatch,
)
from efe_gate.hashing import (
    content_hash,
    hmac_sign,
    hmac_verify,
    sha256_bytes,
    sha256_hex,
)
from efe_gate.ids import (
    SequentialIdGen,
    new_uuid,
    reset_id_gen,
    set_id_gen,
)
from efe_gate.version import VERSION, __version__, version_tuple


# ---------------- version ----------------


def test_version_pair() -> None:
    assert __version__ == VERSION
    maj, mn, patch = version_tuple()
    assert (maj, mn, patch) >= (1, 0, 0)


def test_version_tuple_malformed(monkeypatch: pytest.MonkeyPatch) -> None:
    import efe_gate.version as mod
    monkeypatch.setattr(mod, "VERSION", "bad")
    with pytest.raises(ValueError):
        mod.version_tuple()


# ---------------- errors ----------------


def test_gate_error_shape() -> None:
    e = GateError("oops", detail={"x": 1})
    assert e.code == "gate_error"
    assert e.to_dict()["detail"] == {"x": 1}
    assert "gate_error" in repr(e)


def test_error_subclasses_have_codes() -> None:
    assert NotFound("x").http_status == 404
    assert TenantMismatch("x").http_status == 403
    assert InvalidTransition("x").http_status == 409


# ---------------- clock ----------------


def test_system_clock_returns_utc() -> None:
    ts = SystemClock().now()
    assert ts.tzinfo is not None
    assert ts.utcoffset() == timedelta(0)


def test_fake_clock_advance_and_set() -> None:
    c = FakeClock()
    t0 = c.now()
    c.advance(30)
    assert (c.now() - t0).total_seconds() == 30
    c.set(datetime(2030, 6, 1, tzinfo=timezone.utc))
    assert c.now().year == 2030


def test_fake_clock_rejects_naive() -> None:
    with pytest.raises(ValueError):
        FakeClock(epoch=datetime(2030, 1, 1))
    with pytest.raises(ValueError):
        FakeClock().set(datetime(2030, 1, 1))


def test_fake_clock_rejects_backwards() -> None:
    with pytest.raises(ValueError):
        FakeClock().advance(-1.0)


# ---------------- ids ----------------


def test_system_id_shaped_like_uuid() -> None:
    reset_id_gen()
    u = new_uuid()
    assert len(u) == 36 and u.count("-") == 4


def test_sequential_id_gen() -> None:
    set_id_gen(SequentialIdGen(start=99))
    try:
        assert new_uuid() == "00000000-0000-0000-0000-000000000100"
    finally:
        reset_id_gen()


# ---------------- hashing ----------------


def test_sha256_basic() -> None:
    assert len(sha256_bytes(b"x")) == 32
    assert len(sha256_hex("x")) == 64


def test_content_hash_sort_invariant() -> None:
    a = content_hash({"a": 1, "b": 2})
    b = content_hash({"b": 2, "a": 1})
    assert a == b


def test_hmac_roundtrip() -> None:
    key = bytes(32)
    sig = hmac_sign(b"hello", key)
    assert hmac_verify(b"hello", key, sig) is True
    assert hmac_verify(b"tampered", key, sig) is False


# ---------------- config ----------------


def test_settings_defaults() -> None:
    cfg = GateSettings.from_env()
    assert cfg.environment in ("dev", "test")
    assert cfg.database.url.startswith("sqlite")


def test_settings_override_from_env(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("EFE_GATE_ENVIRONMENT", "staging")
    monkeypatch.setenv("EFE_GATE_STATS__BOOTSTRAP_RESAMPLES", "1000")
    cfg = GateSettings.from_env()
    assert cfg.environment == "staging"
    assert cfg.stats.bootstrap_resamples == 1000


def test_settings_env_conflict(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("EFE_GATE_DATABASE", "scalar")
    monkeypatch.setenv("EFE_GATE_DATABASE__URL", "sqlite:///x")
    with pytest.raises(ConfigError):
        GateSettings.from_env()


def test_settings_rejects_extra() -> None:
    with pytest.raises(ConfigError):
        GateSettings.from_env(overrides={"unknown": 1})


def test_signing_key_validation() -> None:
    with pytest.raises(Exception):
        SignatureSettings(key_hex="xx")
    with pytest.raises(Exception):
        SignatureSettings(key_hex="zz" * 32)
    ok = SignatureSettings(key_hex="a" * 64)
    assert len(bytes.fromhex(ok.key_hex)) == 32
