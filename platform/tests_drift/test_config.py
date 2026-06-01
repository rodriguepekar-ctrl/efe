"""Configuration loading + validation."""

from __future__ import annotations

import pytest

from efe_drift.config import DriftSettings, NatsSettings
from efe_drift.errors import ConfigError


def test_defaults_valid():
    s = DriftSettings()
    assert s.environment == "dev"
    assert s.aggregation.default_histogram_bins == 10


def test_from_env_overrides(monkeypatch):
    monkeypatch.setenv("EFE_DRIFT_ENVIRONMENT", "prod")
    monkeypatch.setenv("EFE_DRIFT_AGGREGATION__DEFAULT_HISTOGRAM_BINS", "20")
    s = DriftSettings.from_env()
    assert s.environment == "prod"
    assert s.aggregation.default_histogram_bins == 20


def test_hmac_hex_length_validation():
    with pytest.raises(Exception):
        NatsSettings(hmac_key_hex="abc")


def test_invalid_env_surfaces_as_config_error(monkeypatch):
    monkeypatch.setenv("EFE_DRIFT_AGGREGATION__DEFAULT_HISTOGRAM_BINS", "999999")
    with pytest.raises(ConfigError):
        DriftSettings.from_env()
