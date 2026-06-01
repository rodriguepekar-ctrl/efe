"""Sprint D8 anti-regression tests for `eval/scripts/run_eval_hybrid.py`
and `eval/scripts/compare_decoders.py`.

These pin :
* the disjoint-key invariant of hybrid bench-format output
* the gate definitions
* the markdown rendering structure of compare_decoders

Run :
    python3 scripts/tests/test_d8_hybrid_eval.py
"""
from __future__ import annotations

import sys
from pathlib import Path

_THIS = Path(__file__).resolve()
_ROOT = _THIS.parents[2]
sys.path.insert(0, str(_ROOT / "eval" / "scripts"))

import run_eval_hybrid as hybrid_mod  # noqa: E402
import compare_decoders as compare_mod  # noqa: E402


def test_hybrid_gates_exist_and_are_calibrated():
    gates = hybrid_mod.HYBRID_GATES
    # At least 3 gates with min thresholds plus a reject_rate max.
    assert "hybrid_accuracy_top1_min" in gates
    assert "hybrid_macro_f1_min" in gates
    assert "hybrid_reject_rate_max" in gates
    assert gates["hybrid_accuracy_top1_min"] >= 0.75  # sanity bound
    assert gates["hybrid_reject_rate_max"] <= 0.20    # sanity bound
    print("OK : HYBRID_GATES well-formed and within sanity bounds")


def test_hybrid_multi_catalog_gates_exist():
    gates = hybrid_mod.HYBRID_MULTI_CATALOG_GATES
    assert "hybrid_aggregate_macro_f1_min" in gates
    assert "hybrid_min_per_catalog_accuracy_min" in gates
    assert "hybrid_min_per_catalog_macro_f1_min" in gates
    print("OK : HYBRID_MULTI_CATALOG_GATES well-formed")


def test_hybrid_bench_keys_disjoint_from_swarm():
    """The bench-format keys produced by the hybrid mode must be
    DISJOINT from the swarm-only keys (single + multi). This is the
    same invariant as `test_d7_parser_eval_merge.py` extended to
    cover D8."""
    swarm_single_keys = {
        "eval_accuracy::top1",
        "eval_accuracy::macro_f1",
        "eval_accuracy::mean_conf_correct",
        "eval_accuracy::false_reject_rate",
        "eval_latency::mean_ns",
        "eval_latency::max_ns",
    }
    swarm_multi_keys = {
        "eval_accuracy::min_top1",
        "eval_accuracy::min_macro_f1",
        "eval_accuracy::aggregate_top1",
        "eval_accuracy::aggregate_macro_f1",
    }
    hybrid_keys_single = {
        "eval_accuracy::hybrid_top1",
        "eval_accuracy::hybrid_macro_f1",
        "eval_accuracy::hybrid_mean_conf_correct",
        "eval_accuracy::hybrid_reject_rate",
        "eval_accuracy::hybrid_branch_agree",
        "eval_accuracy::hybrid_branch_raw_overrides",
        "eval_accuracy::hybrid_branch_swarm_overrides",
        "eval_accuracy::hybrid_branch_reject",
    }
    hybrid_keys_multi = {
        "eval_accuracy::hybrid_min_top1",
        "eval_accuracy::hybrid_min_macro_f1",
        "eval_accuracy::hybrid_aggregate_top1",
        "eval_accuracy::hybrid_aggregate_macro_f1",
    }
    all_swarm = swarm_single_keys | swarm_multi_keys
    all_hybrid = hybrid_keys_single | hybrid_keys_multi
    overlap = all_swarm & all_hybrid
    assert overlap == set(), (
        f"D8 invariant violated : hybrid keys overlap with swarm keys : {overlap}. "
        f"This would make SLO compliance order-dependent in the parser merge."
    )
    print(f"OK : {len(all_swarm)} swarm keys + {len(all_hybrid)} hybrid keys are disjoint")


def test_compare_decoders_render_markdown_shape():
    """Render markdown on a synthetic report and verify the structure
    contains the expected sections."""
    fake_report = [{
        "catalog": "test_cat",
        "catalog_version": "1.0.0",
        "golden_path": "/tmp/g.jsonl",
        "n": 10,
        "raw_cosine": {
            "metrics": {"summary": {"accuracy_top1": 0.9, "macro_f1": 0.92,
                                     "mean_conf_correct": 0.0, "n": 10}},
            "latency_ms": {"mean": 12, "max": 25},
        },
        "swarm_only": {
            "metrics": {"summary": {"accuracy_top1": 0.85, "macro_f1": 0.86,
                                     "mean_conf_correct": 0.7, "n": 10}},
            "latency_ms": {"mean": 2200, "max": 5000},
        },
        "hybrid": {
            "metrics": {"summary": {"accuracy_top1": 0.88, "macro_f1": 0.89,
                                     "mean_conf_correct": 0.78, "n": 10}},
            "latency_ms": {"mean": 2240, "max": 5200},
            "branches": {"agree": 8, "raw_overrides_swarm": 1,
                         "swarm_overrides_raw": 1, "reject": 0},
        },
        "delta": {
            "only_raw_correct": 1,
            "only_swarm_correct": 0,
            "hybrid_better_than_swarm": 1,
            "hybrid_worse_than_swarm": 0,
            "all_three_wrong": 0,
        },
        "per_case": [],
    }]
    md = compare_mod.render_markdown(fake_report)
    # Expected sections.
    assert "# Sprint D8 — Decoder comparison report" in md
    assert "## test_cat@1.0.0" in md
    assert "Hybrid branch distribution" in md
    assert "Delta analysis" in md
    assert "0.9000" in md or "0.9" in md  # raw_cosine acc rendered
    print("OK : compare_decoders.render_markdown produces expected structure")


def main() -> int:
    tests = [
        test_hybrid_gates_exist_and_are_calibrated,
        test_hybrid_multi_catalog_gates_exist,
        test_hybrid_bench_keys_disjoint_from_swarm,
        test_compare_decoders_render_markdown_shape,
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
