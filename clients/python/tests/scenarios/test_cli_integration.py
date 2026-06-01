"""Subprocess-level tests for ``efe-admin scenario``.

These tests drive a fresh process and validate CLI plumbing: exit
codes, argument parsing, error rendering. They do NOT hit the live
Anthropic API — ``run`` and ``bench`` are not covered here (they need
an API key and network), only ``list``, ``show`` and ``scaffold``.
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
        if k.startswith(("EFE_", "ANTHROPIC_")) or k == "NO_COLOR":
            full_env.pop(k, None)
    full_env["NO_COLOR"] = "1"
    if env:
        full_env.update(env)
    return subprocess.run(
        [sys.executable, "-m", "efe_tool.cli", *argv],
        input=stdin, capture_output=True, text=True,
        env=full_env, timeout=timeout, check=False,
    )


class TestList:
    def test_exit_zero_json_valid(self):
        r = _run(["scenario", "list"])
        assert r.returncode == 0
        doc = json.loads(r.stdout)
        assert doc["schema_version"] == 1
        assert doc["count"] >= 3
        names = {s["name"] for s in doc["scenarios"]}
        assert "support_triage_bilingual" in names
        assert "billing_escalation" in names
        assert "fraud_alert" in names


class TestShow:
    def test_show_support_triage(self):
        r = _run(["scenario", "show", "support_triage_bilingual"])
        assert r.returncode == 0
        doc = json.loads(r.stdout)
        assert doc["name"] == "support_triage_bilingual"
        assert doc["golden_cases"] == 8

    def test_show_unknown_exits_3(self):
        r = _run(["scenario", "show", "does_not_exist"])
        assert r.returncode == 3


class TestScaffold:
    def test_creates_files(self, tmp_path: Path):
        r = _run([
            "scenario", "scaffold", "smoke_scn",
            "--out", str(tmp_path),
        ])
        assert r.returncode == 0, r.stderr
        assert (tmp_path / "smoke_scn" / "prompt.md").is_file()
        assert (tmp_path / "smoke_scn" / "golden.json").is_file()
        assert (tmp_path / "smoke_scn" / "README.md").is_file()

    def test_rejects_invalid_name(self, tmp_path: Path):
        r = _run([
            "scenario", "scaffold", "Bad-Name",
            "--out", str(tmp_path),
        ])
        assert r.returncode == 2  # USAGE
        assert "invalid scenario name" in r.stderr.lower()

    def test_existing_without_force_fails(self, tmp_path: Path):
        _run(["scenario", "scaffold", "once",
              "--out", str(tmp_path)])
        r = _run(["scenario", "scaffold", "once",
                  "--out", str(tmp_path)])
        assert r.returncode == 2

    def test_force_overwrites(self, tmp_path: Path):
        _run(["scenario", "scaffold", "twice",
              "--out", str(tmp_path)])
        r = _run(["scenario", "scaffold", "twice",
                  "--out", str(tmp_path), "--force"])
        assert r.returncode == 0, r.stderr


class TestGlobalFlags:
    def test_quiet_suppresses_info_logs(self, tmp_path: Path):
        r = _run([
            "scenario", "scaffold", "quiet_scn",
            "--out", str(tmp_path), "--quiet",
        ])
        assert r.returncode == 0
        # No scenario_scaffold_start log line on stderr in quiet mode.
        assert "scenario_scaffold_start" not in r.stderr

    def test_log_format_json_after_subcommand(self, tmp_path: Path):
        r = _run([
            "scenario", "scaffold", "logjson_scn",
            "--out", str(tmp_path),
            "--log-format", "json",
        ])
        assert r.returncode == 0
        # Each log line on stderr should be NDJSON.
        stderr_lines = [ln for ln in r.stderr.strip().splitlines() if ln]
        for ln in stderr_lines:
            json.loads(ln)
