"""Tests for :mod:`efe_tool.cli.parser`.

Validate the argparse tree shape and the MISSING sentinel discipline.
"""

from __future__ import annotations

import pytest

from efe_tool.cli.config import MISSING
from efe_tool.cli.errors import UsageError
from efe_tool.cli.parser import build_parser, parse_args


class TestShape:
    def test_empty_argv_requires_subcommand(self):
        parser = build_parser()
        with pytest.raises(UsageError):
            parser.parse_args([])

    def test_unknown_subcommand(self):
        parser = build_parser()
        with pytest.raises(UsageError):
            parser.parse_args(["nope"])

    def test_help_raises_system_exit(self):
        parser = build_parser()
        with pytest.raises(SystemExit):
            parser.parse_args(["--help"])

    def test_version_raises_system_exit(self):
        parser = build_parser()
        with pytest.raises(SystemExit):
            parser.parse_args(["--version"])


class TestClassifyFlags:
    def test_text_and_catalog(self):
        ns = parse_args(["classify", "--text", "hi", "--catalog", "c.yaml"])
        assert ns.text == "hi"
        assert ns.catalog_path == "c.yaml"
        assert ns.cmd == "classify"

    def test_missing_defaults(self):
        # When a flag isn't passed, its default is the MISSING sentinel.
        ns = parse_args(["classify", "--text", "hi", "--catalog", "c.yaml"])
        assert ns.endpoint is MISSING
        assert ns.budget_ms is MISSING
        assert ns.max_rounds is MISSING
        assert ns.strict is MISSING
        assert ns.pretty is MISSING

    def test_negative_budget_rejected(self):
        with pytest.raises(UsageError):
            parse_args(["classify", "--text", "x",
                        "--catalog", "c.yaml",
                        "--budget-ms", "-1"])

    def test_zero_timeout_rejected(self):
        with pytest.raises(UsageError):
            parse_args(["classify", "--text", "x",
                        "--catalog", "c.yaml",
                        "--timeout-s", "0"])

    def test_max_rounds_out_of_bounds(self):
        with pytest.raises(UsageError):
            parse_args(["classify", "--text", "x",
                        "--catalog", "c.yaml",
                        "--max-rounds", "300"])

    def test_output_format_choice(self):
        ns = parse_args(["classify", "--text", "x",
                         "--catalog", "c.yaml",
                         "--format", "table"])
        assert ns.format == "table"

    def test_invalid_format_choice_rejected(self):
        with pytest.raises(UsageError):
            parse_args(["classify", "--text", "x",
                        "--catalog", "c.yaml",
                        "--format", "csv"])


class TestGlobalFlags:
    def test_log_level_choice(self):
        ns = parse_args(["--log-level", "debug", "version"])
        assert ns.log_level == "debug"

    def test_log_format_choice(self):
        ns = parse_args(["--log-format", "json", "version"])
        assert ns.log_format == "json"

    def test_quiet_shortcut(self):
        ns = parse_args(["--quiet", "version"])
        assert ns.quiet is True

    def test_request_id(self):
        ns = parse_args(["--request-id", "abc", "version"])
        assert ns.request_id == "abc"


class TestCompletionsShell:
    def test_shell_required(self):
        with pytest.raises(UsageError):
            parse_args(["completions"])

    def test_bash(self):
        ns = parse_args(["completions", "bash"])
        assert ns.shell == "bash"

    def test_invalid_shell(self):
        with pytest.raises(UsageError):
            parse_args(["completions", "powershell"])


class TestValidateCmd:
    def test_requires_path(self):
        with pytest.raises(UsageError):
            parse_args(["validate"])

    def test_with_strict_thresholds(self):
        ns = parse_args(["validate", "x.yaml", "--strict-thresholds"])
        assert ns.path == "x.yaml"
        assert ns.strict_thresholds is True


class TestHealthcheck:
    def test_accepts_endpoint(self):
        ns = parse_args(["healthcheck", "--endpoint", "h:1"])
        assert ns.endpoint == "h:1"
