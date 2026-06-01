# SPDX-License-Identifier: UNLICENSED
"""Sprint **D19 (P1-Foundation) — audit fix A3** : pin tests pour
`eval/scripts/check_slo.py`.

Cf. `docs/EFE_P1_SURFACE_ARCHITECTURE.md` § 5.3.1 (workflow gate).
"""

from __future__ import annotations

import json
import subprocess
import sys
import textwrap
from pathlib import Path

import pytest


ROOT = Path(__file__).resolve().parents[2]
CHECK_SLO = ROOT / "eval" / "scripts" / "check_slo.py"


def _run(args: list[str]) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(CHECK_SLO), *args],
        capture_output=True,
        text=True,
    )


def _write_baseline(tmp_path: Path, body: str) -> Path:
    p = tmp_path / "baseline.yaml"
    p.write_text(textwrap.dedent(body))
    return p


def _write_measure(tmp_path: Path, body: dict) -> Path:
    p = tmp_path / "measure.json"
    p.write_text(json.dumps(body))
    return p


def test_pass_when_p99_under_threshold(tmp_path: Path) -> None:
    baseline = _write_baseline(
        tmp_path,
        """
        version: "0.2.0"
        slos:
          - id: SLO-C-2
            description: "Mode-1 CPU p99 latency"
            metric: 'histogram_quantile(0.99, …)'
            quantile: 0.99
            comparator: lte
            threshold_ns: 800_000_000
            status: ACTIVE
        """,
    )
    # 187 ms p99 → PASS (≤ 800 ms)
    measure = _write_measure(
        tmp_path,
        {
            "count": 1000,
            "latencyDistribution": [
                {"percentage": 50, "latency": 0.025},
                {"percentage": 99, "latency": 0.187},
            ],
        },
    )
    r = _run(["--baseline", str(baseline), "--measure", str(measure)])
    assert r.returncode == 0, r.stdout + r.stderr
    assert "PASS" in r.stdout


def test_fail_when_p99_over_threshold(tmp_path: Path) -> None:
    baseline = _write_baseline(
        tmp_path,
        """
        slos:
          - id: SLO-C-1
            description: "Mode-1 GPU p99 latency"
            quantile: 0.99
            comparator: lte
            threshold_ns: 200_000_000
            status: ACTIVE
        """,
    )
    # 250 ms p99 → FAIL (> 200 ms)
    measure = _write_measure(
        tmp_path,
        {
            "latencyDistribution": [
                {"percentage": 99, "latency": 0.250},
            ],
        },
    )
    r = _run(["--baseline", str(baseline), "--measure", str(measure)])
    assert r.returncode == 1, r.stdout + r.stderr
    assert "FAIL" in r.stdout


def test_skip_pending_gpu_runner_when_flag(tmp_path: Path) -> None:
    baseline = _write_baseline(
        tmp_path,
        """
        slos:
          - id: SLO-C-1
            description: "GPU SLO"
            quantile: 0.99
            comparator: lte
            threshold_ns: 200_000_000
            status: PENDING_GPU_RUNNER
        """,
    )
    measure = _write_measure(tmp_path, {"latencyDistribution": []})
    # Avec --skip-pending-gpu-runner : SKIPPED, exit 0
    r = _run(
        [
            "--baseline",
            str(baseline),
            "--measure",
            str(measure),
            "--skip-pending-gpu-runner",
        ]
    )
    assert r.returncode == 0, r.stdout + r.stderr
    assert "SKIPPED" in r.stdout


def test_strict_skip_fails_when_any_skip(tmp_path: Path) -> None:
    baseline = _write_baseline(
        tmp_path,
        """
        slos:
          - id: SLO-X
            description: "Will skip"
            quantile: 0.99
            comparator: lte
            threshold_ns: 100_000_000
            status: PENDING_GPU_RUNNER
        """,
    )
    measure = _write_measure(tmp_path, {"latencyDistribution": []})
    r = _run(
        [
            "--baseline",
            str(baseline),
            "--measure",
            str(measure),
            "--skip-pending-gpu-runner",
            "--strict-skip",
        ]
    )
    assert r.returncode == 1, r.stdout + r.stderr


def test_fingerprint_mismatch_returns_exit_3(tmp_path: Path) -> None:
    baseline = _write_baseline(
        tmp_path,
        """
        host_fingerprint_baseline: "0a3b4cdeadbeef"
        slos:
          - id: SLO-CRIT
            description: "Critical"
            quantile: 0.99
            comparator: lte
            threshold_ns: 200_000_000
            status: ACTIVE
            host_fingerprint_required: true
        """,
    )
    measure = _write_measure(
        tmp_path,
        {"latencyDistribution": [{"percentage": 99, "latency": 0.1}]},
    )
    r = _run(
        [
            "--baseline",
            str(baseline),
            "--measure",
            str(measure),
            "--fingerprint",
            "different_fp_sha256",
        ]
    )
    assert r.returncode == 3, r.stdout + r.stderr
    assert "MISMATCH" in r.stdout
    assert "REFUSED_BY_FINGERPRINT_MISMATCH" in r.stdout


def test_fingerprint_placeholder_is_treated_as_no_baseline(tmp_path: Path) -> None:
    baseline = _write_baseline(
        tmp_path,
        """
        host_fingerprint_baseline: "PLACEHOLDER_compute_at_freeze"
        slos:
          - id: SLO-X
            description: "Test"
            quantile: 0.99
            comparator: lte
            threshold_ns: 200_000_000
            status: ACTIVE
            host_fingerprint_required: true
        """,
    )
    measure = _write_measure(
        tmp_path,
        {"latencyDistribution": [{"percentage": 99, "latency": 0.1}]},
    )
    r = _run(
        [
            "--baseline",
            str(baseline),
            "--measure",
            str(measure),
            "--fingerprint",
            "any_fp",
        ]
    )
    # PLACEHOLDER → considéré "pas de baseline signée" → no gating
    assert r.returncode == 0, r.stdout + r.stderr


def test_json_output_is_machine_readable(tmp_path: Path) -> None:
    baseline = _write_baseline(
        tmp_path,
        """
        slos:
          - id: SLO-X
            description: "Test"
            quantile: 0.99
            comparator: lte
            threshold_ns: 200_000_000
            status: ACTIVE
        """,
    )
    measure = _write_measure(
        tmp_path,
        {"latencyDistribution": [{"percentage": 99, "latency": 0.15}]},
    )
    r = _run(
        ["--baseline", str(baseline), "--measure", str(measure), "--json"]
    )
    assert r.returncode == 0
    parsed = json.loads(r.stdout)
    assert "results" in parsed
    assert parsed["summary"]["pass"] == 1
    assert parsed["summary"]["fail"] == 0


def test_realistic_p1_baseline_runs_clean(tmp_path: Path) -> None:
    """Run against the actual v0.2.0/p1_slos.yaml with a fully PASS
    measure. Le baseline livré ne doit faire crasher rien."""
    real_baseline = ROOT / "bench_baselines" / "v0.2.0" / "p1_slos.yaml"
    if not real_baseline.exists():
        pytest.skip("p1_slos.yaml absent — likely a freshly-cloned worktree.")
    # Synthèse un measure qui passe tous les SLOs CPU.
    measure = _write_measure(
        tmp_path,
        {
            "latencyDistribution": [
                {"percentage": 50, "latency": 0.060},
                {"percentage": 99, "latency": 0.180},
                {"percentage": 99.9, "latency": 0.310},
            ],
        },
    )
    r = _run(
        [
            "--baseline",
            str(real_baseline),
            "--measure",
            str(measure),
            "--skip-pending-gpu-runner",
            "--json",
        ]
    )
    # On accepte SKIP (multi-window BURN n'a pas de threshold_ns
    # canonique), mais aucun FAIL.
    assert r.returncode == 0, r.stdout + r.stderr
    summary = json.loads(r.stdout)["summary"]
    assert summary["fail"] == 0, summary
