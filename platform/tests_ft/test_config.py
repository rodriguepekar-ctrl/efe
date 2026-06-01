"""Tests for :mod:`efe_ft.config` — env parsing + defaults."""

from __future__ import annotations

import pytest

from efe_ft.config import FtSettings
from efe_ft.errors import ConfigError


def test_defaults() -> None:
    cfg = FtSettings.from_env()
    assert cfg.environment == "dev"
    assert cfg.database.url.startswith("sqlite")
    assert cfg.ingest.minhash_num_perm == 128
    assert cfg.ingest.minhash_threshold == 0.90
    assert cfg.ingest.balance_skew_max == 50.0


def test_frozen_settings() -> None:
    cfg = FtSettings.from_env()
    with pytest.raises(Exception):
        cfg.environment = "prod"  # type: ignore[misc]


def test_overrides_win(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("EFE_FT_ENVIRONMENT", "test")
    monkeypatch.setenv("EFE_FT_INGEST__BALANCE_SKEW_MAX", "25")
    cfg = FtSettings.from_env(overrides={"environment": "staging"})
    assert cfg.environment == "staging"  # explicit override wins
    assert cfg.ingest.balance_skew_max == 25.0  # from env


def test_env_coerce_types(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("EFE_FT_ENVIRONMENT", "test")
    monkeypatch.setenv("EFE_FT_DATABASE__ECHO", "true")
    monkeypatch.setenv("EFE_FT_DATABASE__POOL_SIZE", "50")
    monkeypatch.setenv("EFE_FT_INGEST__MINHASH_THRESHOLD", "0.85")
    cfg = FtSettings.from_env()
    assert cfg.database.echo is True
    assert cfg.database.pool_size == 50
    assert cfg.ingest.minhash_threshold == 0.85


def test_invalid_config_raises(monkeypatch: pytest.MonkeyPatch) -> None:
    # threshold must be in (0.5, 1.0); any value outside raises
    monkeypatch.setenv("EFE_FT_INGEST__MINHASH_THRESHOLD", "2.0")
    with pytest.raises(ConfigError):
        FtSettings.from_env()


def test_extra_fields_rejected() -> None:
    with pytest.raises(ConfigError):
        FtSettings.from_env(overrides={"unknown_key": 42})


def test_env_conflict_raises(monkeypatch: pytest.MonkeyPatch) -> None:
    # EFE_FT_DATABASE set as scalar, then EFE_FT_DATABASE__URL nested.
    monkeypatch.setenv("EFE_FT_DATABASE", "scalar_val")
    monkeypatch.setenv("EFE_FT_DATABASE__URL", "sqlite:///x")
    with pytest.raises(ConfigError, match="env conflict"):
        FtSettings.from_env()
