"""Sprint D7 anti-regression tests for `eval/scripts/calibrate_mode1.py`.

We pin the **pure functions** of the calibration sweep (grid construction,
ranking, output formatting) — these run without spawning a real
efe-server, so they are fast and hermetic. The integration test that
actually spins a server with EFE_FUSION_* env vars is documented but
gated behind `D7_CALIBRATION_INTEGRATION=1` so CI doesn't pay the cost
of building the binary on every PR.

Run :
    python3 -m pytest scripts/tests/test_d7_calibrate_grid.py -v
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

_THIS = Path(__file__).resolve()
_ROOT = _THIS.parents[2]
_EVAL = _ROOT / "eval" / "scripts"
sys.path.insert(0, str(_EVAL))

import calibrate_mode1 as cal  # noqa: E402


def test_subgrid_is_27_points():
    grid = cal.build_grid(full=False)
    assert len(grid) == 27, f"sub-grid should be 27 points, got {len(grid)}"
    print("OK : sub-grid has 27 points")


def test_full_grid_is_80_points():
    grid = cal.build_grid(full=True)
    assert len(grid) == 80, f"full grid should be 80 points, got {len(grid)}"
    print("OK : full grid has 80 points")


def test_grid_includes_analytical_d7_starting_point():
    grid = cal.build_grid(full=False)
    found = any(
        abs(p.beta_init - 0.45) < 1e-6
        and abs(p.beta_decay - 0.05) < 1e-6
        and abs(p.alpha_init - 0.30) < 1e-6
        for p in grid
    )
    assert found, "sub-grid must contain the analytical D7 starting point"
    print("OK : analytical D7 point ∈ sub-grid")


def test_grid_point_label_uniqueness():
    grid = cal.build_grid(full=True)
    labels = {p.label() for p in grid}
    assert len(labels) == len(grid), "labels must be unique across grid"
    print("OK : grid labels are unique")


def test_grid_point_env_format():
    p = cal.GridPoint(0.45, 0.05, 0.30)
    env = p.as_env()
    assert env["EFE_FUSION_BETA_INIT"] == "0.45"
    assert env["EFE_FUSION_BETA_DECAY"] == "0.05"
    assert env["EFE_FUSION_ALPHA_INIT"] == "0.3"
    print("OK : GridPoint.as_env produces parseable values")


def test_rank_results_orders_by_macro_f1_desc():
    pts = [
        cal.GridResult(
            point=cal.GridPoint(0.40, 0.05, 0.30),
            aggregate_macro_f1=0.70,
            aggregate_accuracy_top1=0.72,
            min_per_catalog_macro_f1=0.65,
            min_per_catalog_accuracy=0.68,
        ),
        cal.GridResult(
            point=cal.GridPoint(0.45, 0.05, 0.30),
            aggregate_macro_f1=0.85,
            aggregate_accuracy_top1=0.86,
            min_per_catalog_macro_f1=0.80,
            min_per_catalog_accuracy=0.82,
        ),
        cal.GridResult(
            point=cal.GridPoint(0.50, 0.05, 0.30),
            aggregate_macro_f1=0.85,
            aggregate_accuracy_top1=0.88,
            min_per_catalog_macro_f1=0.79,
            min_per_catalog_accuracy=0.81,
        ),
        cal.GridResult(
            point=cal.GridPoint(0.30, 0.12, 0.40),
            aggregate_macro_f1=0.0,
            aggregate_accuracy_top1=0.0,
            min_per_catalog_macro_f1=0.0,
            min_per_catalog_accuracy=0.0,
            error="server timeout",
        ),
    ]
    ranked = cal.rank_results(pts)
    # Top tied f1 at 0.85 ; tie-break by min_per_catalog_macro_f1 (0.80 > 0.79).
    assert ranked[0].point.beta_init == 0.45
    assert ranked[1].point.beta_init == 0.50
    # Failed tuple sinks to the bottom.
    assert ranked[-1].error is not None
    print("OK : ranking obeys the documented order (f1, min_f1, accuracy)")


def test_write_outputs_emits_three_files(tmp_path: Path):
    results = [
        cal.GridResult(
            point=cal.GridPoint(0.45, 0.05, 0.30),
            aggregate_macro_f1=0.85,
            aggregate_accuracy_top1=0.86,
            min_per_catalog_macro_f1=0.80,
            min_per_catalog_accuracy=0.82,
            per_catalog=[
                {"catalog": "support_triage", "accuracy_top1": 0.875},
                {"catalog": "fraud_alert", "accuracy_top1": 0.83},
                {"catalog": "billing_escalation", "accuracy_top1": 0.82},
            ],
        ),
    ]
    cal.write_outputs(
        tmp_path,
        results,
        catalogs=["catalogs/support_triage.yaml"] * 3,
        goldens=[
            "eval/golden/support_triage.jsonl",
            "eval/golden/fraud_alert.jsonl",
            "eval/golden/billing_escalation.jsonl",
        ],
    )
    for fname in ("winner.json", "full.json", "grid.csv"):
        assert (tmp_path / fname).exists(), f"missing {fname}"
    winner = json.loads((tmp_path / "winner.json").read_text())
    assert winner["sprint"] == "D7"
    assert winner["winner"]["problem_anchor_init"] == 0.45
    assert winner["winner"]["alignment_strength_init"] == 0.30
    assert winner["winner"]["problem_anchor_decay"] == 0.05
    print("OK : write_outputs emits {winner.json, full.json, grid.csv}")


def main() -> int:
    tests = [
        test_subgrid_is_27_points,
        test_full_grid_is_80_points,
        test_grid_includes_analytical_d7_starting_point,
        test_grid_point_label_uniqueness,
        test_grid_point_env_format,
        test_rank_results_orders_by_macro_f1_desc,
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
    # The tmp_path-based test is left to pytest only.
    if failures:
        print(f"\n{failures}/{len(tests)} tests failed")
        return 1
    print(f"\nAll {len(tests)} tests passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
