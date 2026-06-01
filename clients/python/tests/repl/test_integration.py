"""Subprocess-level integration tests for ``efe-admin repl``.

These tests drive a fresh Python process in batch mode (stdin piped),
validating that the entry points, CLI plumbing, and REPL loop cooperate
end-to-end. They do NOT require a running ``efe-server`` — every test
either uses commands that avoid RPC or expects a clean, typed error.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Sequence

import pytest


def _run(
    argv: Sequence[str],
    *,
    stdin: str | None = None,
    env: dict[str, str] | None = None,
    timeout: float = 60.0,
) -> subprocess.CompletedProcess:
    full_env = os.environ.copy()
    for k in list(full_env.keys()):
        if k.startswith("EFE_") or k in {"NO_COLOR", "XDG_DATA_HOME"}:
            full_env.pop(k, None)
    full_env["NO_COLOR"] = "1"  # keep outputs deterministic
    if env:
        full_env.update(env)

    return subprocess.run(
        [sys.executable, "-m", "efe_tool.cli", *argv],
        input=stdin, capture_output=True, text=True,
        env=full_env, timeout=timeout, check=False,
    )


# ---------------------------------------------------------------------------
#  Boot and shutdown
# ---------------------------------------------------------------------------


class TestBootShutdown:
    def test_quit_immediately(self):
        r = _run(["repl", "--no-history"], stdin=":quit\n")
        assert r.returncode == 0

    def test_ctrl_d_equivalent(self):
        r = _run(["repl", "--no-history"], stdin="")
        assert r.returncode == 0

    def test_version_inside(self):
        r = _run(["repl", "--no-history"], stdin=":version\n:quit\n")
        assert r.returncode == 0
        assert "efe-tool" in r.stdout


# ---------------------------------------------------------------------------
#  Command dispatch via subprocess
# ---------------------------------------------------------------------------


class TestCommandsInSubprocess:
    def test_help_prints_groups(self):
        r = _run(["repl", "--no-history"], stdin=":help\n:quit\n")
        assert r.returncode == 0
        assert "[session]" in r.stdout
        assert "[tuning]" in r.stdout
        assert ":reload" in r.stdout

    def test_info_lists_fields(self):
        r = _run(["repl", "--no-history"], stdin=":info\n:quit\n")
        assert r.returncode == 0
        assert "session" in r.stdout
        assert "endpoint" in r.stdout

    def test_budget_set_and_show(self):
        r = _run(
            ["repl", "--no-history"],
            stdin=":budget 7000\n:budget\n:quit\n",
        )
        assert r.returncode == 0
        assert "7000" in r.stdout

    def test_format_switch_json(self):
        r = _run(
            ["repl", "--no-history"],
            stdin=":format json\n:format\n:quit\n",
        )
        assert r.returncode == 0
        assert "json" in r.stdout

    def test_unknown_command_suggestion(self):
        r = _run(
            ["repl", "--no-history"],
            stdin=":relod\n:quit\n",
        )
        assert r.returncode == 0
        # Suggestion printed on stderr
        assert "reload" in r.stderr


# ---------------------------------------------------------------------------
#  Error safety
# ---------------------------------------------------------------------------


class TestErrorSafety:
    def test_classify_without_catalog_does_not_crash(self):
        r = _run(
            ["repl", "--no-history"],
            stdin="hello\n:quit\n",
        )
        assert r.returncode == 0
        # One CLI error visible on stderr
        assert "no catalog" in r.stderr.lower() or \
               "catalog" in r.stderr.lower()

    def test_malformed_command_does_not_crash(self):
        r = _run(
            ["repl", "--no-history"],
            stdin=':save "unterminated\n:quit\n',
        )
        assert r.returncode == 0

    def test_catalog_not_found_does_not_crash(self, tmp_path):
        bad = tmp_path / "nope.yaml"
        r = _run(
            ["repl", "--no-history"],
            stdin=f":catalog {bad}\n:quit\n",
        )
        assert r.returncode == 0
        # CATALOG_NOT_FOUND rendered on stderr but session survived
        assert "CATALOG_NOT_FOUND" in r.stderr


# ---------------------------------------------------------------------------
#  Transcript saving
# ---------------------------------------------------------------------------


class TestSaveTranscript:
    def test_save_produces_valid_json(self, tmp_path):
        target = tmp_path / "session.json"
        r = _run(
            ["repl", "--no-history"],
            stdin=f":info\n:save {target}\n:quit\n",
        )
        assert r.returncode == 0
        assert target.exists()
        doc = json.loads(target.read_text(encoding="utf-8"))
        assert doc["schema_version"] == 1
        assert "entries" in doc


# ---------------------------------------------------------------------------
#  History persistence
# ---------------------------------------------------------------------------


class TestHistoryPersistence:
    def test_history_file_written(self, tmp_path):
        hist = tmp_path / "hist"
        r = _run(
            ["repl", "--history", str(hist)],
            stdin=":info\n:version\n:quit\n",
        )
        assert r.returncode == 0
        assert hist.exists()
        text = hist.read_text(encoding="utf-8")
        assert ":info" in text
        assert ":version" in text

    def test_no_history_skips_write(self, tmp_path):
        hist = tmp_path / "hist"
        r = _run(
            ["repl", "--history", str(hist), "--no-history"],
            stdin=":info\n:quit\n",
        )
        assert r.returncode == 0
        assert not hist.exists()


# ---------------------------------------------------------------------------
#  Global flag positioning
# ---------------------------------------------------------------------------


class TestGlobalFlagPositioning:
    def test_quiet_before_subcommand(self):
        r = _run(
            ["--quiet", "repl", "--no-history"],
            stdin=":info\n:quit\n",
        )
        assert r.returncode == 0
        # Quiet suppresses INFO logs on stderr
        assert "repl_boot" not in r.stderr
        assert "repl_ready" not in r.stderr

    def test_log_format_after_subcommand(self):
        r = _run(
            ["repl", "--no-history", "--log-format", "json"],
            stdin=":quit\n",
        )
        assert r.returncode == 0


# ---------------------------------------------------------------------------
#  stdout / stderr separation (discipline carried over from Lettre A)
# ---------------------------------------------------------------------------


class TestStreamSeparation:
    def test_info_on_stdout(self):
        r = _run(["repl", "--no-history", "--quiet"],
                 stdin=":info\n:quit\n")
        assert "endpoint" in r.stdout
        # No log leakage
        assert "repl_boot" not in r.stdout

    def test_error_on_stderr(self):
        r = _run(["repl", "--no-history"],
                 stdin=":unknowncmd\n:quit\n")
        assert "unknown" in r.stderr.lower()
