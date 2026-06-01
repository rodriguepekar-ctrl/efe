"""Tests for :mod:`efe_tool.cli.metadata`.

Validate the invariants that lock DEFAULTS and ENV_KEYS together.
"""

from __future__ import annotations

import os

import pytest

from efe_tool.cli.metadata import (
    DEFAULTS,
    ENV_KEYS,
    LOG_FORMATS,
    LOG_LEVELS,
    OUTPUT_FORMATS,
    PROGRESS_MODES,
    SCHEMA_VERSION,
    no_color_from_env,
    parse_bool_env,
)


class TestStructuralInvariants:
    def test_defaults_and_env_keys_have_same_keyset(self) -> None:
        assert set(DEFAULTS.keys()) == set(ENV_KEYS.keys()), (
            "DEFAULTS and ENV_KEYS must stay in lockstep; a key was "
            "added to one table and not the other."
        )

    def test_schema_version_is_positive_int(self) -> None:
        assert isinstance(SCHEMA_VERSION, int)
        assert SCHEMA_VERSION >= 1

    def test_log_levels_are_sorted(self) -> None:
        assert LOG_LEVELS == ("debug", "info", "warn", "error")

    def test_log_formats(self) -> None:
        assert LOG_FORMATS == ("text", "json")

    def test_output_formats(self) -> None:
        assert OUTPUT_FORMATS == ("json", "text", "table")

    def test_progress_modes(self) -> None:
        assert PROGRESS_MODES == ("auto", "always", "never")


class TestDefaultsConsistency:
    def test_log_level_default_is_in_domain(self) -> None:
        assert DEFAULTS["log_level"] in LOG_LEVELS

    def test_log_format_default_is_in_domain(self) -> None:
        assert DEFAULTS["log_format"] in LOG_FORMATS

    def test_format_default_is_in_domain(self) -> None:
        assert DEFAULTS["format"] in OUTPUT_FORMATS

    def test_budget_is_positive(self) -> None:
        assert isinstance(DEFAULTS["budget_ms"], int)
        assert DEFAULTS["budget_ms"] > 0

    def test_timeout_is_positive(self) -> None:
        assert isinstance(DEFAULTS["timeout_s"], float)
        assert DEFAULTS["timeout_s"] > 0.0


class TestParseBoolEnv:
    @pytest.mark.parametrize("raw", ["1", "true", "TRUE", "Yes", "on", "y", "t"])
    def test_truthy(self, raw: str) -> None:
        assert parse_bool_env(raw) is True

    @pytest.mark.parametrize("raw", ["0", "false", "FALSE", "No", "off", "n", "f"])
    def test_falsy(self, raw: str) -> None:
        assert parse_bool_env(raw) is False

    @pytest.mark.parametrize("raw", [None, ""])
    def test_unset(self, raw: str | None) -> None:
        assert parse_bool_env(raw) is None

    def test_ambiguous_raises(self) -> None:
        with pytest.raises(ValueError):
            parse_bool_env("maybe")


class TestNoColorConvention:
    def test_absent_means_false(self, monkeypatch: pytest.MonkeyPatch) -> None:
        monkeypatch.delenv("NO_COLOR", raising=False)
        assert no_color_from_env() is False

    def test_present_empty_means_true(self, monkeypatch: pytest.MonkeyPatch) -> None:
        monkeypatch.setenv("NO_COLOR", "")
        assert no_color_from_env() is True

    def test_present_any_value_means_true(
        self, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        monkeypatch.setenv("NO_COLOR", "1")
        assert no_color_from_env() is True
