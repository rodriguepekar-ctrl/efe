"""Sprint D13 — promtool check rules validation.

Validates the recording.yaml + alerts.yaml in
`deploy/observability/prometheus/rules.d13/` via promtool.

Skipped if `promtool` binary is not on PATH (e.g. local dev
machines without prometheus installed).

In CI, this runs against an official prom/prometheus container.
"""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

import pytest

_REPO = Path(__file__).resolve().parents[2]
_RULES_DIR = _REPO / "deploy" / "observability" / "prometheus" / "rules.d13"


def _has_promtool() -> bool:
    return shutil.which("promtool") is not None


@pytest.mark.skipif(not _has_promtool(), reason="promtool not installed")
def test_recording_rules_valid():
    p = _RULES_DIR / "recording.yaml"
    result = subprocess.run(
        ["promtool", "check", "rules", str(p)],
        capture_output=True, text=True,
    )
    assert result.returncode == 0, (
        f"promtool check rules {p} failed:\n"
        f"stdout: {result.stdout}\nstderr: {result.stderr}"
    )


@pytest.mark.skipif(not _has_promtool(), reason="promtool not installed")
def test_alert_rules_valid():
    p = _RULES_DIR / "alerts.yaml"
    result = subprocess.run(
        ["promtool", "check", "rules", str(p)],
        capture_output=True, text=True,
    )
    assert result.returncode == 0, (
        f"promtool check rules {p} failed:\n"
        f"stdout: {result.stdout}\nstderr: {result.stderr}"
    )


# ─────────────────────────────────────────────────────────────────────
# Structural checks that don't require promtool
# ─────────────────────────────────────────────────────────────────────

def test_rules_files_exist():
    assert (_RULES_DIR / "recording.yaml").exists()
    assert (_RULES_DIR / "alerts.yaml").exists()


def test_recording_rules_count_at_least_7():
    text = (_RULES_DIR / "recording.yaml").read_text()
    # Count `- record:` occurrences.
    n = text.count("- record:")
    assert n >= 7, f"expected ≥ 7 recording rules, got {n}"


def test_alert_rules_count_is_10():
    text = (_RULES_DIR / "alerts.yaml").read_text()
    n = text.count("- alert:")
    assert n == 10, f"expected 10 alert rules, got {n}"


def test_each_alert_has_runbook_url_in_yaml():
    """Cheap pre-check that doesn't parse YAML — just text-greps."""
    text = (_RULES_DIR / "alerts.yaml").read_text()
    n_alert = text.count("- alert:")
    n_runbook = text.count("runbook_url:")
    assert n_runbook == n_alert, (
        f"runbook_url count {n_runbook} != alert count {n_alert} ; "
        f"some alert lacks the annotation"
    )
