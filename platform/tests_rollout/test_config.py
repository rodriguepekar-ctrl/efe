"""Configuration loading + validation."""

from __future__ import annotations

import pytest

from efe_rollout.config import (
    AssignmentSettings,
    IngestSettings,
    NatsSettings,
    RolloutSettings,
)
from efe_rollout.errors import ConfigError


def test_defaults_are_valid():
    s = RolloutSettings()
    assert s.environment == "dev"
    assert s.assignment.holdout_basis_points == 100


def test_from_env_accepts_overrides(monkeypatch):
    monkeypatch.setenv("EFE_ROLLOUT_ENVIRONMENT", "prod")
    monkeypatch.setenv("EFE_ROLLOUT_ASSIGNMENT__HOLDOUT_BASIS_POINTS", "50")
    s = RolloutSettings.from_env()
    assert s.environment == "prod"
    assert s.assignment.holdout_basis_points == 50


def test_from_env_overrides_merge():
    s = RolloutSettings.from_env(
        overrides={"environment": "staging"},
    )
    assert s.environment == "staging"


def test_hmac_key_must_be_hex():
    with pytest.raises(Exception):
        AssignmentSettings(assignment_hmac_key_hex="not-hex")
    with pytest.raises(Exception):
        IngestSettings(exposure_hmac_key_hex="not-hex")
    with pytest.raises(Exception):
        NatsSettings(hmac_key_hex="abc")


def test_invalid_config_raises_config_error(monkeypatch):
    monkeypatch.setenv("EFE_ROLLOUT_ASSIGNMENT__HOLDOUT_BASIS_POINTS", "99999")
    with pytest.raises(ConfigError):
        RolloutSettings.from_env()
