"""Sprint D6 audit anti-regression tests for Python helpers.

Pins the 4 audit fixes :

1. `parse_criterion_report.py::evaluate_slos` propagates `display_as`
   and `comparator` to the compliance entry.
2. `parse_criterion_report.py::evaluate_slos` does not apply
   `ci_coefficient` to ratio targets.
3. `check_regression.py` inverts the sign of `delta` for `gte`
   metrics (eval_accuracy::*).
4. The numeric scaling convention `accuracy * 1e9` is preserved end
   to end.

Run with :
    python3 -m pytest scripts/tests/

Or standalone :
    python3 scripts/tests/test_d6_audit_fixes.py
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

# Allow importing parse_criterion_report from the scripts/ directory.
_HERE = Path(__file__).resolve().parent
_SCRIPTS = _HERE.parent
sys.path.insert(0, str(_SCRIPTS))

import parse_criterion_report as parser  # noqa: E402


def _make_results(**kwargs: float) -> dict:
    """Build a fake `results` dict with the criterion entry shape."""
    return {
        key: {
            "mean_ns": value,
            "median_ns": value,
            "std_dev_ns": 0.0,
            "mad_ns": 0.0,
            "approx_p95_ns": value,
        }
        for key, value in kwargs.items()
    }


def test_evaluate_slos_propagates_display_as():
    """The compliance entry must include `display_as` and `comparator`
    so downstream formatters can pick the right unit."""
    results = _make_results(**{
        "eval_accuracy::top1": 800_000_000,  # 0.80
    })
    compliance = parser.evaluate_slos(results, ci_coefficient=1.0)
    entry = compliance["SLO-D6-ACC-1"]
    assert entry["display_as"] == "ratio", entry
    assert entry["comparator"] == "gte", entry
    assert entry["passed"] is True, "0.80 >= 0.75 must pass"
    print("OK : evaluate_slos propagates display_as + comparator")


def test_evaluate_slos_no_ci_coefficient_on_ratio():
    """Ratio targets must NOT be inflated by ci_coefficient (a slow
    runner doesn't make accuracy worse)."""
    results = _make_results(**{
        "eval_accuracy::top1": 760_000_000,  # 0.76
    })
    compliance = parser.evaluate_slos(results, ci_coefficient=1.5)
    entry = compliance["SLO-D6-ACC-1"]
    # Target should remain 750M (= 0.75) regardless of ci_coefficient.
    assert entry["target_ns"] == 750_000_000, (
        f"Ratio target inflated by CI coefficient : got {entry['target_ns']}"
    )
    assert entry["passed"] is True
    print("OK : ratio target immune to ci_coefficient")


def test_evaluate_slos_ci_coefficient_on_latency():
    """Duration targets ARE inflated by ci_coefficient (slow runner
    needs more time)."""
    results = _make_results(**{
        "real_e2e_latency_naive::small": 500_000_000,  # 500 ms
    })
    compliance = parser.evaluate_slos(results, ci_coefficient=1.5)
    entry = compliance["SLO-RD1-1"]
    # Target should be inflated by 1.5×.
    assert entry["target_ns"] == 600_000_000, (
        f"Latency target NOT inflated : got {entry['target_ns']}"
    )
    assert entry["passed"] is True, "500 ms < 600 ms must pass"
    print("OK : latency target inflated by ci_coefficient")


def test_check_regression_detects_accuracy_drop():
    """A 20 % drop in accuracy from baseline must trigger a regression
    even though the absolute delta is negative.
    """
    baseline_data = {
        "version": "1.0",
        "results": {
            "eval_accuracy::top1": {"approx_p95_ns": 850_000_000},  # 0.85
            "real_e2e_latency_naive::small": {"approx_p95_ns": 200_000_000},
        },
    }
    candidate_data = {
        "version": "1.0",
        "results": {
            "eval_accuracy::top1": {"approx_p95_ns": 600_000_000},  # 0.60 — DROP
            "real_e2e_latency_naive::small": {"approx_p95_ns": 200_000_000},
        },
    }
    with tempfile.TemporaryDirectory() as tmp:
        bp = Path(tmp) / "baseline.json"
        cp = Path(tmp) / "candidate.json"
        bp.write_text(json.dumps(baseline_data))
        cp.write_text(json.dumps(candidate_data))

        # check_regression returns 1 when a regression is detected.
        result = subprocess.run(
            ["python3", str(_SCRIPTS / "check_regression.py"),
             "--baseline", str(bp), "--candidate", str(cp),
             "--threshold", "0.10"],
            capture_output=True, text=True,
        )
        # The 0.85 → 0.60 drop is a 29 % regression → must fail.
        assert result.returncode == 1, (
            f"check_regression failed to detect accuracy drop : "
            f"rc={result.returncode}, stdout={result.stdout!r}"
        )
        assert "eval_accuracy::top1" in result.stdout
        print("OK : check_regression detects accuracy drop")


def test_check_regression_ignores_accuracy_increase():
    """An accuracy *improvement* must not be flagged as regression."""
    baseline_data = {
        "version": "1.0",
        "results": {
            "eval_accuracy::top1": {"approx_p95_ns": 600_000_000},
        },
    }
    candidate_data = {
        "version": "1.0",
        "results": {
            "eval_accuracy::top1": {"approx_p95_ns": 850_000_000},
        },
    }
    with tempfile.TemporaryDirectory() as tmp:
        bp = Path(tmp) / "baseline.json"
        cp = Path(tmp) / "candidate.json"
        bp.write_text(json.dumps(baseline_data))
        cp.write_text(json.dumps(candidate_data))

        result = subprocess.run(
            ["python3", str(_SCRIPTS / "check_regression.py"),
             "--baseline", str(bp), "--candidate", str(cp),
             "--threshold", "0.10"],
            capture_output=True, text=True,
        )
        assert result.returncode == 0
        print("OK : check_regression ignores accuracy improvements")


def test_check_regression_detects_latency_increase():
    """A latency increase IS a regression (lte semantics, default)."""
    baseline_data = {
        "version": "1.0",
        "results": {
            "real_e2e_latency_naive::small": {"approx_p95_ns": 100_000_000},
        },
    }
    candidate_data = {
        "version": "1.0",
        "results": {
            "real_e2e_latency_naive::small": {"approx_p95_ns": 200_000_000},
        },
    }
    with tempfile.TemporaryDirectory() as tmp:
        bp = Path(tmp) / "baseline.json"
        cp = Path(tmp) / "candidate.json"
        bp.write_text(json.dumps(baseline_data))
        cp.write_text(json.dumps(candidate_data))

        result = subprocess.run(
            ["python3", str(_SCRIPTS / "check_regression.py"),
             "--baseline", str(bp), "--candidate", str(cp),
             "--threshold", "0.10"],
            capture_output=True, text=True,
        )
        assert result.returncode == 1
        print("OK : check_regression detects latency increase")


def main() -> int:
    tests = [
        test_evaluate_slos_propagates_display_as,
        test_evaluate_slos_no_ci_coefficient_on_ratio,
        test_evaluate_slos_ci_coefficient_on_latency,
        test_check_regression_detects_accuracy_drop,
        test_check_regression_ignores_accuracy_increase,
        test_check_regression_detects_latency_increase,
    ]
    failures = 0
    for t in tests:
        try:
            t()
        except AssertionError as e:
            print(f"FAIL : {t.__name__} — {e}")
            failures += 1
        except Exception as e:
            print(f"ERROR : {t.__name__} — {type(e).__name__}: {e}")
            failures += 1
    if failures:
        print(f"\n{failures}/{len(tests)} tests failed")
        return 1
    print(f"\nAll {len(tests)} tests passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
