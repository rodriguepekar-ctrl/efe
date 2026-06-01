"""Tests for :mod:`efe_orch.config`."""

from __future__ import annotations

import pytest

from efe_orch.config import OrchSettings, PoolSettings, SchedulerSettings
from efe_orch.errors import ConfigError


def test_defaults() -> None:
    cfg = OrchSettings.from_env()
    assert cfg.environment == "dev"
    assert cfg.database.url.startswith("sqlite")
    assert cfg.k8s.dry_run is True
    names = [p.name for p in cfg.scheduler.pools]
    assert names == ["cpu-small", "cpu-large", "gpu-a10", "gpu-a100"]


def test_frozen_settings() -> None:
    cfg = OrchSettings.from_env()
    with pytest.raises(Exception):
        cfg.environment = "prod"  # type: ignore[misc]


def test_env_override(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("EFE_ORCH_ENVIRONMENT", "staging")
    monkeypatch.setenv("EFE_ORCH_HEARTBEAT__INTERVAL_S", "45")
    monkeypatch.setenv("EFE_ORCH_DATABASE__ECHO", "true")
    cfg = OrchSettings.from_env()
    assert cfg.environment == "staging"
    assert cfg.heartbeat.interval_s == 45
    assert cfg.database.echo is True


def test_overrides_win(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("EFE_ORCH_ENVIRONMENT", "dev")
    cfg = OrchSettings.from_env(overrides={"environment": "prod"})
    assert cfg.environment == "prod"


def test_env_conflict_raises(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("EFE_ORCH_DATABASE", "scalar_val")
    monkeypatch.setenv("EFE_ORCH_DATABASE__URL", "sqlite:///x")
    with pytest.raises(ConfigError, match="env conflict"):
        OrchSettings.from_env()


def test_extra_fields_rejected() -> None:
    with pytest.raises(ConfigError):
        OrchSettings.from_env(overrides={"unknown_key": 42})


def test_empty_pools_rejected() -> None:
    with pytest.raises(ConfigError):
        OrchSettings.from_env(
            overrides={"scheduler": {"pools": []}},
        )


def test_duplicate_pool_names_rejected() -> None:
    # pydantic raises ValidationError at direct-constructor level; from_env
    # wraps it as ConfigError. Assert on *both* paths.
    with pytest.raises(Exception):
        SchedulerSettings(
            pools=(
                PoolSettings(name="dup"),
                PoolSettings(name="dup"),
            ),
        )
    with pytest.raises(ConfigError):
        OrchSettings.from_env(overrides={
            "scheduler": {
                "pools": [
                    {"name": "dup"},
                    {"name": "dup"},
                ],
            },
        })


def test_pool_requires_name() -> None:
    with pytest.raises(Exception):
        PoolSettings(name="")  # min_length=1 violated


def test_heartbeat_bounds() -> None:
    with pytest.raises(ConfigError):
        OrchSettings.from_env(
            overrides={"heartbeat": {"interval_s": 0}},
        )
