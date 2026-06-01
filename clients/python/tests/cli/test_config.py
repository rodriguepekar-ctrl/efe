"""Tests for :mod:`efe_tool.cli.config`.

Exhaustive precedence and validation checks.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from efe_tool.cli.config import (
    MISSING,
    ResolvedConfig,
    harmonise_budget_and_timeout,
    resolve_config,
    validate_config,
)
from efe_tool.cli.errors import UsageError
from efe_tool.cli.parser import build_parser


# ---------------------------------------------------------------------------
#  Helpers
# ---------------------------------------------------------------------------


def _parse(*argv: str):
    return build_parser().parse_args(list(argv))


# ---------------------------------------------------------------------------
#  Precedence
# ---------------------------------------------------------------------------


class TestPrecedence:
    def test_flag_beats_env(self) -> None:
        ns = _parse("classify", "--text", "x",
                    "--catalog", "c.yaml",
                    "--endpoint", "from-flag:2222")
        cfg = resolve_config(ns, env={"EFE_ENDPOINT": "from-env:1111"})
        assert cfg.endpoint == "from-flag:2222"

    def test_env_beats_default(self) -> None:
        ns = _parse("classify", "--text", "x", "--catalog", "c.yaml")
        cfg = resolve_config(ns, env={"EFE_ENDPOINT": "from-env:3333"})
        assert cfg.endpoint == "from-env:3333"

    def test_default_when_nothing(self) -> None:
        ns = _parse("classify", "--text", "x", "--catalog", "c.yaml")
        cfg = resolve_config(ns, env={})
        assert cfg.endpoint == "127.0.0.1:50051"

    def test_bool_env_true(self) -> None:
        ns = _parse("classify", "--text", "x", "--catalog", "c.yaml")
        cfg = resolve_config(ns, env={"EFE_STRICT": "1"})
        assert cfg.strict is True

    def test_bool_env_false(self) -> None:
        ns = _parse("classify", "--text", "x", "--catalog", "c.yaml")
        cfg = resolve_config(ns, env={"EFE_STRICT": "0"})
        assert cfg.strict is False

    def test_int_env(self) -> None:
        ns = _parse("classify", "--text", "x", "--catalog", "c.yaml")
        cfg = resolve_config(ns, env={"EFE_BUDGET_MS": "9999"})
        assert cfg.budget_ms == 9999

    def test_float_env(self) -> None:
        ns = _parse("classify", "--text", "x", "--catalog", "c.yaml")
        cfg = resolve_config(ns, env={"EFE_TIMEOUT_S": "3.5"})
        assert cfg.timeout_s == 3.5

    def test_int_env_invalid_raises_usage(self) -> None:
        ns = _parse("classify", "--text", "x", "--catalog", "c.yaml")
        with pytest.raises(UsageError):
            resolve_config(ns, env={"EFE_BUDGET_MS": "abc"})


# ---------------------------------------------------------------------------
#  NO_COLOR
# ---------------------------------------------------------------------------


class TestNoColor:
    def test_no_color_env_presence(self) -> None:
        ns = _parse("version")
        cfg = resolve_config(ns, env={"NO_COLOR": ""})
        assert cfg.no_color is True

    def test_no_color_flag(self) -> None:
        ns = _parse("--no-color", "version")
        cfg = resolve_config(ns, env={})
        assert cfg.no_color is True

    def test_no_color_off(self) -> None:
        ns = _parse("version")
        cfg = resolve_config(ns, env={})
        assert cfg.no_color is False


# ---------------------------------------------------------------------------
#  Quiet shortcut
# ---------------------------------------------------------------------------


class TestQuiet:
    def test_quiet_flag(self) -> None:
        ns = _parse("--quiet", "version")
        cfg = resolve_config(ns, env={})
        assert cfg.quiet is True
        assert cfg.effective_log_level == "error"

    def test_no_quiet(self) -> None:
        ns = _parse("version")
        cfg = resolve_config(ns, env={})
        assert cfg.quiet is False
        assert cfg.effective_log_level == cfg.log_level


# ---------------------------------------------------------------------------
#  Validation
# ---------------------------------------------------------------------------


class TestValidation:
    def test_invalid_log_level_raises(self) -> None:
        cfg = _build_valid_cfg(log_level="loud")
        with pytest.raises(UsageError):
            validate_config(cfg)

    def test_invalid_log_format_raises(self) -> None:
        cfg = _build_valid_cfg(log_format="xml")
        with pytest.raises(UsageError):
            validate_config(cfg)

    def test_invalid_output_format_raises(self) -> None:
        cfg = _build_valid_cfg(format="yaml")
        with pytest.raises(UsageError):
            validate_config(cfg)

    def test_invalid_progress_raises(self) -> None:
        cfg = _build_valid_cfg(progress="sometimes")
        with pytest.raises(UsageError):
            validate_config(cfg)

    def test_budget_must_be_positive(self) -> None:
        cfg = _build_valid_cfg(budget_ms=0)
        with pytest.raises(UsageError):
            validate_config(cfg)

    def test_timeout_must_be_positive(self) -> None:
        cfg = _build_valid_cfg(timeout_s=0.0)
        with pytest.raises(UsageError):
            validate_config(cfg)

    def test_max_rounds_bounded(self) -> None:
        cfg = _build_valid_cfg(max_rounds=0)
        with pytest.raises(UsageError):
            validate_config(cfg)
        cfg = _build_valid_cfg(max_rounds=257)
        with pytest.raises(UsageError):
            validate_config(cfg)

    def test_endpoint_no_scheme(self) -> None:
        cfg = _build_valid_cfg(endpoint="http://127.0.0.1:50051")
        with pytest.raises(UsageError):
            validate_config(cfg)

    def test_endpoint_requires_port(self) -> None:
        cfg = _build_valid_cfg(endpoint="127.0.0.1")
        with pytest.raises(UsageError):
            validate_config(cfg)


# ---------------------------------------------------------------------------
#  harmonise_budget_and_timeout
# ---------------------------------------------------------------------------


class TestHarmonise:
    def test_bumps_timeout_when_below_budget(self) -> None:
        cfg = _build_valid_cfg(budget_ms=5000, timeout_s=2.0)
        out = harmonise_budget_and_timeout(cfg)
        # 5000 / 1000 + 0.5 = 5.5
        assert out.timeout_s == 5.5

    def test_does_not_touch_if_already_enough(self) -> None:
        cfg = _build_valid_cfg(budget_ms=1000, timeout_s=10.0)
        out = harmonise_budget_and_timeout(cfg)
        assert out.timeout_s == 10.0


# ---------------------------------------------------------------------------
#  Positional path & legacy text folding
# ---------------------------------------------------------------------------


class TestPositionalFolding:
    """Verify the resolver folds positional args into typed fields."""

    def test_compile_catalog_positional_becomes_catalog_path(self) -> None:
        ns = _parse("compile-catalog", "/tmp/x.yaml")
        cfg = resolve_config(ns, env={})
        assert cfg.catalog_path is not None
        assert str(cfg.catalog_path) == "/tmp/x.yaml"

    def test_validate_positional_becomes_catalog_path(self) -> None:
        ns = _parse("validate", "/tmp/x.yaml")
        cfg = resolve_config(ns, env={})
        assert str(cfg.catalog_path) == "/tmp/x.yaml"

    def test_explicit_catalog_flag_wins_over_positional(self) -> None:
        # compile-catalog has no --catalog flag, but validate doesn't
        # either — and classify has no positional. So this scenario is
        # impossible in practice; we still assert the resolver rule.
        ns = _parse("validate", "/tmp/positional.yaml")
        # Simulate a post-parse override by mutating namespace
        # (not normally possible, but verifies the algorithm).
        ns.catalog_path = "/tmp/flag.yaml"
        cfg = resolve_config(ns, env={})
        # Flag value wins because the resolver only folds when
        # ``catalog_path is None``.
        assert str(cfg.catalog_path) == "/tmp/flag.yaml"


class TestLegacyTextFolding:
    def test_positional_text_populates_legacy_and_text(self) -> None:
        ns = _parse("classify", "some legacy text",
                    "--catalog", "c.yaml")
        cfg = resolve_config(ns, env={})
        assert cfg.legacy_text == "some legacy text"
        assert cfg.text == "some legacy text"

    def test_explicit_text_flag_trumps_positional(self) -> None:
        ns = _parse("classify", "legacy",
                    "--text", "modern",
                    "--catalog", "c.yaml")
        cfg = resolve_config(ns, env={})
        # legacy_text is still recorded (for the deprecation warning)
        assert cfg.legacy_text == "legacy"
        # But the effective text is the --text value
        assert cfg.text == "modern"

    def test_no_positional_no_legacy_marker(self) -> None:
        ns = _parse("classify", "--text", "hi",
                    "--catalog", "c.yaml")
        cfg = resolve_config(ns, env={})
        assert cfg.legacy_text is None
        assert cfg.text == "hi"


# ---------------------------------------------------------------------------
#  Internals
# ---------------------------------------------------------------------------


def _build_valid_cfg(**overrides) -> ResolvedConfig:
    base = dict(
        command="version",
        endpoint="127.0.0.1:50051",
        tls=False,
        tls_ca=None,
        jwt=None,
        tenant=None,
        catalog_path=None,
        embedder_model="dummy",
        budget_ms=5000,
        timeout_s=10.0,
        max_rounds=16,
        strict=False,
        format="json",
        pretty=False,
        with_diagnostics=False,
        log_level="info",
        log_format="text",
        quiet=False,
        no_color=False,
        request_id="rid",
        progress="auto",
    )
    base.update(overrides)
    return ResolvedConfig(**base)
