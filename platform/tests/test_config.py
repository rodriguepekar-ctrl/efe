"""Tests for :mod:`efe_platform.config` — hierarchical loading + env parsing."""

from __future__ import annotations

from pathlib import Path

import pytest

from efe_platform.config import (
    AuditSettings,
    BudgetSettings,
    LlmSettings,
    PlatformConfig,
    SwarmSettings,
    _coerce,
    _from_environ,
    _merge,
)
from efe_platform.errors import ConfigError

# ---------------------------------------------------------------------------
#  Defaults
# ---------------------------------------------------------------------------


def test_defaults_are_valid() -> None:
    cfg = PlatformConfig()
    assert cfg.tenant_default == "default"
    assert cfg.swarm.endpoint == "localhost:50051"
    assert cfg.llm.provider == "mock"
    assert cfg.budget.max_wall_clock_ms > 0


def test_frozen_config_is_immutable() -> None:
    cfg = PlatformConfig()
    # pydantic frozen → attribute assignment raises
    with pytest.raises(Exception):
        cfg.tenant_default = "other"  # type: ignore[misc]


def test_empty_tenant_rejected() -> None:
    with pytest.raises(ConfigError):
        PlatformConfig.from_env(overrides={"tenant_default": ""})


def test_swarm_bounds() -> None:
    with pytest.raises(Exception):
        SwarmSettings(default_deadline_ms=-1)
    with pytest.raises(Exception):
        SwarmSettings(max_retries=-1)


def test_budget_bounds() -> None:
    with pytest.raises(Exception):
        BudgetSettings(max_wall_clock_ms=0)


def test_audit_defaults() -> None:
    a = AuditSettings()
    assert a.enabled is True
    assert a.fsync_each is True


def test_llm_temperature_bounds() -> None:
    with pytest.raises(Exception):
        LlmSettings(temperature=3.0)


# ---------------------------------------------------------------------------
#  Loaders
# ---------------------------------------------------------------------------


def test_overrides_win_over_defaults() -> None:
    cfg = PlatformConfig.from_env(overrides={"tenant_default": "explicit"})
    assert cfg.tenant_default == "explicit"


def test_env_parsing(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("EFE_PLATFORM_TENANT_DEFAULT", "acme")
    monkeypatch.setenv("EFE_PLATFORM_SWARM__ENDPOINT", "swarm.local:50051")
    monkeypatch.setenv("EFE_PLATFORM_SWARM__DEFAULT_DEADLINE_MS", "7500")
    monkeypatch.setenv("EFE_PLATFORM_SWARM__TLS", "true")
    cfg = PlatformConfig.from_env()
    assert cfg.tenant_default == "acme"
    assert cfg.swarm.endpoint == "swarm.local:50051"
    assert cfg.swarm.default_deadline_ms == 7500
    assert cfg.swarm.tls is True


def test_env_overrides_then_explicit_wins(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("EFE_PLATFORM_TENANT_DEFAULT", "from-env")
    cfg = PlatformConfig.from_env(overrides={"tenant_default": "explicit"})
    assert cfg.tenant_default == "explicit"


def test_toml_loader(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    p = tmp_path / "cfg.toml"
    p.write_text(
        """\
tenant_default = "toml-tenant"

[swarm]
endpoint = "from.toml:50052"
""",
        encoding="utf-8",
    )
    monkeypatch.setenv("EFE_PLATFORM_CONFIG", str(p))
    cfg = PlatformConfig.from_env()
    assert cfg.tenant_default == "toml-tenant"
    assert cfg.swarm.endpoint == "from.toml:50052"


def test_toml_not_found(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("EFE_PLATFORM_CONFIG", "/nonexistent/path.toml")
    with pytest.raises(ConfigError, match="TOML config not found"):
        PlatformConfig.from_env()


def test_unknown_field_rejected() -> None:
    # extra="forbid" on the pydantic model
    with pytest.raises(ConfigError):
        PlatformConfig.from_env(overrides={"mystery_key": 42})


# ---------------------------------------------------------------------------
#  Low-level helpers
# ---------------------------------------------------------------------------


def test_merge_deep() -> None:
    a = {"x": {"y": 1, "z": 2}, "k": "old"}
    b = {"x": {"y": 99}, "k": "new"}
    assert _merge(a, b) == {"x": {"y": 99, "z": 2}, "k": "new"}


def test_merge_second_wins_on_type_conflict() -> None:
    a = {"x": {"y": 1}}
    b = {"x": 42}
    assert _merge(a, b) == {"x": 42}


def test_coerce_types() -> None:
    assert _coerce("true") is True
    assert _coerce("false") is False
    assert _coerce("42") == 42
    assert _coerce("3.14") == 3.14
    assert _coerce("hello") == "hello"


def test_from_environ_conflict(monkeypatch: pytest.MonkeyPatch) -> None:
    """A scalar then a nested descent under the same key is a hard error."""
    monkeypatch.setenv("EFE_PLATFORM_SWARM", "scalar")
    monkeypatch.setenv("EFE_PLATFORM_SWARM__ENDPOINT", "x")
    with pytest.raises(ConfigError, match="env conflict"):
        _from_environ("EFE_PLATFORM_")
