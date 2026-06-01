"""Sprint D7 — anti-regression tests for the parser's merge of
single-catalog + multi-catalog `--eval-bench-format` files.

Pins the **disjoint-key invariant** : the bench-format JSON produced
by `run_eval.py` (single-catalog) and `run_eval.py --multi-catalog`
must use NON-OVERLAPPING `eval_accuracy::*` keys. If the two ever
share a key, the parser's last-write-wins merge would silently
change the meaning of one SLO family (D6-ACC-* or D7-MC-*) depending
on argv order.

These tests run the actual `parse_criterion_report.py` script as a
subprocess against synthetic eval-bench files, then assert the SLO
compliance for each family hits its expected values.

Run :
    python3 -m pytest scripts/tests/test_d7_parser_eval_merge.py -v

Or standalone :
    python3 scripts/tests/test_d7_parser_eval_merge.py
"""
from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path

_HERE = Path(__file__).resolve().parent
_ROOT = _HERE.parents[1]
_PARSER = _ROOT / "scripts" / "parse_criterion_report.py"


def _bench_entry(value_ns: float) -> dict[str, float]:
    return {
        "mean_ns": float(value_ns),
        "median_ns": float(value_ns),
        "std_dev_ns": 0.0,
        "mad_ns": 0.0,
        "approx_p95_ns": float(value_ns),
    }


def _write_single_eval(path: Path, *, top1: float, macro_f1: float,
                      mean_conf: float, false_rej: float) -> None:
    payload = {
        "eval_accuracy::top1": _bench_entry(top1 * 1e9),
        "eval_accuracy::macro_f1": _bench_entry(macro_f1 * 1e9),
        "eval_accuracy::mean_conf_correct": _bench_entry(mean_conf * 1e9),
        "eval_accuracy::false_reject_rate": _bench_entry(false_rej * 1e9),
        "eval_latency::mean_ns": _bench_entry(2_500_000_000),
        "eval_latency::max_ns": _bench_entry(3_500_000_000),
    }
    path.write_text(json.dumps(payload))


def _write_multi_eval(path: Path, *, min_top1: float, min_macro_f1: float,
                     agg_top1: float, agg_macro_f1: float) -> None:
    payload = {
        "eval_accuracy::min_top1": _bench_entry(min_top1 * 1e9),
        "eval_accuracy::min_macro_f1": _bench_entry(min_macro_f1 * 1e9),
        "eval_accuracy::aggregate_top1": _bench_entry(agg_top1 * 1e9),
        "eval_accuracy::aggregate_macro_f1": _bench_entry(agg_macro_f1 * 1e9),
    }
    path.write_text(json.dumps(payload))


def _make_dummy_criterion_root(root: Path) -> None:
    """Produce a minimal `target/criterion`-shaped tree the parser
    accepts. Without this the parser warns "no criterion estimates"
    but still emits the SLO compliance over the eval files alone."""
    grp = root / "dummy" / "id" / "new"
    grp.mkdir(parents=True, exist_ok=True)
    (grp / "estimates.json").write_text(json.dumps({
        "mean": {"point_estimate": 1_000_000.0},
        "median": {"point_estimate": 1_000_000.0},
        "std_dev": {"point_estimate": 0.0},
        "median_abs_dev": {"point_estimate": 0.0},
    }))


def _run_parser(criterion_root: Path, eval_files: list[Path],
                output: Path) -> dict:
    cmd: list[str] = [
        sys.executable, str(_PARSER),
        "--criterion-root", str(criterion_root),
        "--metadata-timestamp", "20260426T000000Z",
        "--output", str(output),
    ]
    for ev in eval_files:
        cmd.extend(["--eval-bench-format", str(ev)])
    res = subprocess.run(cmd, capture_output=True, text=True)
    assert res.returncode == 0, (
        f"parser failed (rc={res.returncode}) "
        f"stdout={res.stdout!r} stderr={res.stderr!r}"
    )
    return json.loads(output.read_text())


def _slo(report: dict, slo_id: str) -> dict:
    return report["slo_compliance"][slo_id]


def test_keys_are_disjoint_between_single_and_multi():
    """The parser must merge the two files without any key collision
    on the `eval_accuracy::*` namespace.
    """
    with tempfile.TemporaryDirectory() as tmp:
        tmpd = Path(tmp)
        _make_dummy_criterion_root(tmpd / "criterion")
        single = tmpd / "single.json"
        multi = tmpd / "multi.json"
        _write_single_eval(single, top1=0.85, macro_f1=0.80,
                           mean_conf=0.78, false_rej=0.05)
        _write_multi_eval(multi, min_top1=0.70, min_macro_f1=0.65,
                          agg_top1=0.78, agg_macro_f1=0.74)

        single_keys = set(json.loads(single.read_text()).keys())
        multi_keys = set(json.loads(multi.read_text()).keys())
        # Filter out latency keys — only `eval_accuracy::*` matters
        # for the disjointness invariant.
        single_acc = {k for k in single_keys if k.startswith("eval_accuracy::")}
        multi_acc = {k for k in multi_keys if k.startswith("eval_accuracy::")}
        overlap = single_acc & multi_acc
        assert overlap == set(), (
            f"D7 invariant violated : single-catalog and multi-catalog "
            f"share eval_accuracy keys : {overlap}. The merge in the "
            f"parser would become order-dependent."
        )
        print("OK : single & multi eval_accuracy::* keys are disjoint")


def test_d6_slos_read_single_catalog_top1():
    """`SLO-D6-ACC-1` and `SLO-D6-ACC-2` must read the SINGLE-catalog
    `top1` and `macro_f1` regardless of merge order.
    """
    with tempfile.TemporaryDirectory() as tmp:
        tmpd = Path(tmp)
        _make_dummy_criterion_root(tmpd / "criterion")
        single = tmpd / "single.json"
        multi = tmpd / "multi.json"
        # Single-catalog: 0.85 / 0.80 (support_triage)
        _write_single_eval(single, top1=0.85, macro_f1=0.80,
                           mean_conf=0.78, false_rej=0.05)
        # Multi-catalog: min=0.70 / 0.65 (worst-case across 3 catalogs)
        _write_multi_eval(multi, min_top1=0.70, min_macro_f1=0.65,
                          agg_top1=0.78, agg_macro_f1=0.74)

        # Pass them in BOTH orders. The result must be identical.
        for order_label, files in [
            ("single,multi", [single, multi]),
            ("multi,single", [multi, single]),
        ]:
            out = tmpd / f"report-{order_label}.json"
            report = _run_parser(tmpd / "criterion", files, out)
            # D6-ACC-1 / -2 must reflect the SINGLE-catalog values.
            assert _slo(report, "SLO-D6-ACC-1")["measured_ns"] == 850_000_000, (
                f"order={order_label} : SLO-D6-ACC-1 measured_ns="
                f"{_slo(report, 'SLO-D6-ACC-1')['measured_ns']} "
                f"(expected 850M for single-catalog top1=0.85)"
            )
            assert _slo(report, "SLO-D6-ACC-1")["passed"] is True
            assert _slo(report, "SLO-D6-ACC-2")["measured_ns"] == 800_000_000
            assert _slo(report, "SLO-D6-ACC-2")["passed"] is True
        print("OK : D6-ACC-* SLOs reflect single-catalog values, order-independent")


def test_d7_mc_slos_read_multi_catalog_min_and_aggregate():
    """`SLO-D7-MC-1..4` must read the MULTI-catalog dedicated keys
    (`min_*`, `aggregate_*`), regardless of merge order.
    """
    with tempfile.TemporaryDirectory() as tmp:
        tmpd = Path(tmp)
        _make_dummy_criterion_root(tmpd / "criterion")
        single = tmpd / "single.json"
        multi = tmpd / "multi.json"
        _write_single_eval(single, top1=0.85, macro_f1=0.80,
                           mean_conf=0.78, false_rej=0.05)
        _write_multi_eval(multi, min_top1=0.70, min_macro_f1=0.65,
                          agg_top1=0.78, agg_macro_f1=0.74)

        for order_label, files in [
            ("single,multi", [single, multi]),
            ("multi,single", [multi, single]),
        ]:
            out = tmpd / f"report-{order_label}.json"
            report = _run_parser(tmpd / "criterion", files, out)

            assert _slo(report, "SLO-D7-MC-1")["measured_ns"] == 700_000_000, (
                f"order={order_label} : SLO-D7-MC-1 must read min_top1=0.70 "
                f"(700M), got {_slo(report, 'SLO-D7-MC-1')['measured_ns']}"
            )
            assert _slo(report, "SLO-D7-MC-1")["passed"] is True  # 0.70 ≥ 0.65

            assert _slo(report, "SLO-D7-MC-2")["measured_ns"] == 650_000_000
            assert _slo(report, "SLO-D7-MC-2")["passed"] is True  # 0.65 ≥ 0.60

            assert _slo(report, "SLO-D7-MC-3")["measured_ns"] == 780_000_000
            assert _slo(report, "SLO-D7-MC-3")["passed"] is True  # 0.78 ≥ 0.75

            assert _slo(report, "SLO-D7-MC-4")["measured_ns"] == 740_000_000
            assert _slo(report, "SLO-D7-MC-4")["passed"] is False  # 0.74 < 0.75
        print("OK : D7-MC-* SLOs reflect multi-catalog values, order-independent")


def test_d7_mc_min_top1_below_gate_fails_independently_of_d6():
    """If multi-catalog min_top1 < 0.65 the D7-MC-1 gate must FAIL
    even when the single-catalog top1 is high (0.85). The single
    catalog value must NEVER mask the worst-case anti-overfit gate."""
    with tempfile.TemporaryDirectory() as tmp:
        tmpd = Path(tmp)
        _make_dummy_criterion_root(tmpd / "criterion")
        single = tmpd / "single.json"
        multi = tmpd / "multi.json"
        _write_single_eval(single, top1=0.85, macro_f1=0.80,
                           mean_conf=0.78, false_rej=0.05)
        _write_multi_eval(multi, min_top1=0.50, min_macro_f1=0.45,
                          agg_top1=0.62, agg_macro_f1=0.60)

        out = tmpd / "report.json"
        report = _run_parser(tmpd / "criterion", [single, multi], out)
        # D6-ACC-1 PASS on the high single-catalog top1.
        assert _slo(report, "SLO-D6-ACC-1")["passed"] is True
        # D7-MC-1 FAIL on the low multi-catalog min_top1.
        assert _slo(report, "SLO-D7-MC-1")["passed"] is False, (
            "D7-MC-1 must FAIL when worst-case is below gate, even if "
            "support_triage alone passes — that's the whole point of "
            "the anti-overfit gate."
        )
        # And the aggregate gates also FAIL.
        assert _slo(report, "SLO-D7-MC-3")["passed"] is False
        assert _slo(report, "SLO-D7-MC-4")["passed"] is False
        print("OK : D7-MC anti-overfit gates fire independently of D6-ACC")


def main() -> int:
    tests = [
        test_keys_are_disjoint_between_single_and_multi,
        test_d6_slos_read_single_catalog_top1,
        test_d7_mc_slos_read_multi_catalog_min_and_aggregate,
        test_d7_mc_min_top1_below_gate_fails_independently_of_d6,
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
