"""Subprocess-level integration tests for ``efe-admin``.

These tests exercise the entry point in a fresh Python process, which
is the ultimate smoke test: they catch import-time regressions,
entry-point wiring issues, and exit-code discipline bugs that a
direct ``main(argv)`` call would hide.

They do NOT require a running ``efe-server``. A separate end-to-end
test suite (gated on ``EFE_E2E=1``) exercises that path.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Sequence

import pytest


def _run(argv: Sequence[str], *, stdin: str | None = None,
         env: dict[str, str] | None = None, timeout: float = 30.0
         ) -> subprocess.CompletedProcess:
    full_env = os.environ.copy()
    # Ensure a deterministic PYTHONPATH so the test runner can import
    # the in-tree efe_tool package even if we're not inside the
    # project venv. Strip existing EFE_* vars.
    for k in list(full_env.keys()):
        if k.startswith("EFE_") or k == "NO_COLOR":
            full_env.pop(k, None)
    if env:
        full_env.update(env)

    return subprocess.run(
        [sys.executable, "-m", "efe_tool.cli", *argv],
        input=stdin,
        capture_output=True,
        text=True,
        env=full_env,
        timeout=timeout,
        check=False,
    )


class TestSmoke:
    def test_help_exit_zero(self):
        r = _run(["--help"])
        assert r.returncode == 0
        assert "efe-admin" in r.stdout or "efe-admin" in r.stderr

    def test_version_exit_zero(self):
        r = _run(["--version"])
        assert r.returncode == 0

    def test_version_subcommand_json(self):
        r = _run(["version", "--format", "json"])
        assert r.returncode == 0
        doc = json.loads(r.stdout)
        assert doc["program"] == "efe-admin"
        assert doc["schema_version"] >= 1

    def test_version_subcommand_pretty(self):
        r = _run(["version", "--format", "json", "--pretty"])
        assert r.returncode == 0
        assert "\n  " in r.stdout


class TestExitCodeDiscipline:
    """Each exit code in the taxonomy must be hit by at least one shell
    invocation (see LETTER_A_ARCHITECTURE.md §21.1 G-A5)."""

    def test_code_2_usage_error(self):
        r = _run([])
        assert r.returncode == 2

    def test_code_3_catalog_not_found(self, tmp_path: Path):
        bad = tmp_path / "nope.yaml"
        r = _run(["validate", str(bad)])
        assert r.returncode == 3

    def test_code_4_catalog_invalid(self, tmp_path: Path):
        p = tmp_path / "bad.yaml"
        p.write_text("this: [is, not, a, catalog]", encoding="utf-8")
        r = _run(["validate", str(p)])
        assert r.returncode == 4

    def test_completions_bash_exit_zero(self):
        r = _run(["completions", "bash"])
        assert r.returncode == 0
        assert "_efe_admin_completions" in r.stdout


class TestJsonErrorDocuments:
    def test_usage_error_json(self, tmp_path: Path):
        r = _run(["--log-format", "json", "validate",
                  str(tmp_path / "nope.yaml")])
        assert r.returncode == 3
        # The error document is written to stderr in JSON format.
        # The dispatcher may emit both the error doc and a final
        # ``cli_failed`` log line — parse the first ndjson object.
        first_line = r.stderr.strip().splitlines()[0]
        doc = json.loads(first_line)
        assert doc["error"]["code"] == "CATALOG_NOT_FOUND"
        assert doc["error"]["exit_code"] == 3


class TestStreamSeparation:
    """stdout must remain pristine — no log leaks."""

    def test_help_is_stdout_not_stderr(self):
        r = _run(["--help"])
        assert "efe-admin" in r.stdout
        # argparse writes --help to stdout; stderr should be empty
        assert r.stderr == ""

    def test_error_goes_to_stderr_only(self, tmp_path: Path):
        r = _run(["validate", str(tmp_path / "nope.yaml")])
        assert r.returncode == 3
        assert r.stdout == "", f"stdout leaked: {r.stdout!r}"
        assert r.stderr != ""


class TestNoColorConvention:
    def test_no_color_env(self):
        r = _run(["validate", "/nope.yaml"], env={"NO_COLOR": "1"})
        # stderr should not contain ANSI color codes.
        assert "\x1b[" not in r.stderr


class TestDeprecatedPositionalText:
    """The AS-IS form ``efe-admin classify TEXT --catalog ...`` is kept
    working for one more minor version. Each use must emit a clearly
    visible deprecation warning on stderr.
    """

    def test_positional_text_warning(self, tmp_path: Path):
        bad = tmp_path / "nope.yaml"
        r = _run([
            "classify", "this is the legacy form",
            "--catalog", str(bad),
            "--log-format", "json",
        ])
        # The catalog path doesn't exist → exit 3, BUT the deprecation
        # warning fires *before* the catalog check, so it must be
        # present in stderr.
        assert r.returncode == 3, r.stderr
        assert "deprecated_positional_text" in r.stderr

    def test_explicit_text_no_warning(self, tmp_path: Path):
        bad = tmp_path / "nope.yaml"
        r = _run([
            "classify", "--text", "modern form",
            "--catalog", str(bad),
            "--log-format", "json",
        ])
        assert r.returncode == 3
        assert "deprecated_positional_text" not in r.stderr


class TestGlobalFlagPositioning:
    """Global flags must be accepted both BEFORE and AFTER the subcommand."""

    def test_log_format_before_subcommand(self, tmp_path: Path):
        r = _run(["--log-format", "json",
                  "validate", str(tmp_path / "nope.yaml")])
        assert r.returncode == 3
        # JSON log was requested; the error line must be valid JSON.
        first_err_line = r.stderr.strip().splitlines()[0]
        json.loads(first_err_line)

    def test_log_format_after_subcommand(self, tmp_path: Path):
        r = _run(["validate", str(tmp_path / "nope.yaml"),
                  "--log-format", "json"])
        assert r.returncode == 3
        first_err_line = r.stderr.strip().splitlines()[0]
        json.loads(first_err_line)

    def test_quiet_suppresses_info_logs(self, tmp_path: Path):
        """--quiet == log-level error; no INFO events must be emitted."""
        ok_file = tmp_path / "c.yaml"
        ok_file.write_text(
            "apiVersion: efe.mode1/v1\n"
            "kind: AnchorCatalog\n"
            "metadata:\n  name: x\n  version: 0.0.1\n"
            "embedder:\n  family: st\n  model: y\n  dim: 4\n"
            "classes:\n"
            "  - name: a\n    canonical_sentences: [hello]\n"
            "  - name: b\n    canonical_sentences: [bye]\n",
            encoding="utf-8",
        )
        r = _run(["validate", str(ok_file), "--quiet"])
        assert r.returncode == 0
        # No "validate_start" / "validate_done" info events
        assert "validate_start" not in r.stderr
        assert "validate_done" not in r.stderr


class TestRequestId:
    def test_explicit_request_id_propagates(self, tmp_path: Path):
        """--request-id value must appear in both the error document and
        the log lines emitted on the same run."""
        r = _run(["--request-id", "abc-123",
                  "--log-format", "json",
                  "validate", str(tmp_path / "nope.yaml")])
        assert r.returncode == 3
        assert "abc-123" in r.stderr

    def test_auto_request_id_is_uuid(self, tmp_path: Path):
        """Absent --request-id, a fresh UUID must be generated."""
        r = _run(["--log-format", "json",
                  "validate", str(tmp_path / "nope.yaml")])
        assert r.returncode == 3
        first = json.loads(r.stderr.strip().splitlines()[0])
        rid = first.get("request_id", "")
        # Simple UUID v4 shape check: 8-4-4-4-12 hex
        import re
        assert re.match(
            r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$",
            rid,
        ), rid
