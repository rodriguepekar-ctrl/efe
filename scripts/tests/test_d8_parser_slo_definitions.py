"""Sprint D8 anti-regression tests for the D8-* SLO definitions in
`scripts/parse_criterion_report.py`.

These tests pin :
* the new SLO IDs and bench_keys
* the disjoint-key invariant across all 4 SLO families
  (D6-ACC, D7-MC, D8-HYB, D8-RD/LAT)
* end-to-end parser merge with hybrid + real-data eval files

Run :
    python3 scripts/tests/test_d8_parser_slo_definitions.py
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


def _make_dummy_criterion_root(root: Path) -> None:
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
        "--metadata-timestamp", "20260427T000000Z",
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


def test_d8_slos_present_in_report():
    """All 7 D8-* SLO IDs must appear in the parser's compliance dict."""
    with tempfile.TemporaryDirectory() as tmp:
        tmpd = Path(tmp)
        _make_dummy_criterion_root(tmpd / "criterion")
        out = tmpd / "r.json"
        report = _run_parser(tmpd / "criterion", [], out)
        for slo_id in ["SLO-D8-HYB-1", "SLO-D8-HYB-2", "SLO-D8-HYB-3",
                       "SLO-D8-HYB-4", "SLO-D8-RD-1", "SLO-D8-RD-2",
                       "SLO-D8-LAT-2"]:
            assert slo_id in report["slo_compliance"], f"missing {slo_id}"
        print("OK : 7 D8 SLOs present")


def test_d8_hyb_slos_read_hybrid_keys():
    """SLO-D8-HYB-* must read the hybrid keys from a hybrid eval file
    and report passed/failed correctly without affecting D6/D7 SLOs.
    """
    with tempfile.TemporaryDirectory() as tmp:
        tmpd = Path(tmp)
        _make_dummy_criterion_root(tmpd / "criterion")
        hybrid = tmpd / "hybrid.json"
        hybrid.write_text(json.dumps({
            "eval_accuracy::hybrid_top1": _bench_entry(0.88 * 1e9),
            "eval_accuracy::hybrid_macro_f1": _bench_entry(0.84 * 1e9),
            "eval_accuracy::hybrid_reject_rate": _bench_entry(0.05 * 1e9),
            "eval_accuracy::hybrid_min_macro_f1": _bench_entry(0.62 * 1e9),
        }))
        out = tmpd / "r.json"
        report = _run_parser(tmpd / "criterion", [hybrid], out)
        assert _slo(report, "SLO-D8-HYB-1")["passed"] is True   # 0.88 ≥ 0.80
        assert _slo(report, "SLO-D8-HYB-2")["passed"] is True   # 0.84 ≥ 0.75
        assert _slo(report, "SLO-D8-HYB-3")["passed"] is True   # 0.05 ≤ 0.15
        assert _slo(report, "SLO-D8-HYB-4")["passed"] is True   # 0.62 ≥ 0.60
        # D6/D7 SLOs untouched (no swarm keys provided).
        assert _slo(report, "SLO-D6-ACC-1")["skipped"] is True
        assert _slo(report, "SLO-D7-MC-1")["skipped"] is True
        print("OK : D8-HYB-* read hybrid_* keys, isolated from D6/D7")


def test_d8_hyb_3_fails_on_high_reject_rate():
    """Reject rate above 0.15 → SLO-D8-HYB-3 must FAIL."""
    with tempfile.TemporaryDirectory() as tmp:
        tmpd = Path(tmp)
        _make_dummy_criterion_root(tmpd / "criterion")
        hybrid = tmpd / "hybrid.json"
        hybrid.write_text(json.dumps({
            "eval_accuracy::hybrid_top1": _bench_entry(0.88 * 1e9),
            "eval_accuracy::hybrid_macro_f1": _bench_entry(0.84 * 1e9),
            "eval_accuracy::hybrid_reject_rate": _bench_entry(0.25 * 1e9),  # too high
            "eval_accuracy::hybrid_min_macro_f1": _bench_entry(0.65 * 1e9),
        }))
        out = tmpd / "r.json"
        report = _run_parser(tmpd / "criterion", [hybrid], out)
        assert _slo(report, "SLO-D8-HYB-3")["passed"] is False, \
            "reject rate 0.25 > 0.15 must fail SLO-D8-HYB-3"
        print("OK : D8-HYB-3 correctly fires on high reject rate")


def test_d8_keys_disjoint_from_d6_d7_keys():
    """Cross-family disjointness — single + multi + hybrid keys form
    3 mutually-disjoint sets ; the parser merge produces a result
    independent of file ordering."""
    with tempfile.TemporaryDirectory() as tmp:
        tmpd = Path(tmp)
        _make_dummy_criterion_root(tmpd / "criterion")
        single = tmpd / "single.json"
        multi = tmpd / "multi.json"
        hybrid = tmpd / "hybrid.json"
        single.write_text(json.dumps({
            "eval_accuracy::top1": _bench_entry(0.90 * 1e9),
            "eval_accuracy::macro_f1": _bench_entry(0.88 * 1e9),
        }))
        multi.write_text(json.dumps({
            "eval_accuracy::min_top1": _bench_entry(0.75 * 1e9),
            "eval_accuracy::min_macro_f1": _bench_entry(0.70 * 1e9),
            "eval_accuracy::aggregate_top1": _bench_entry(0.85 * 1e9),
            "eval_accuracy::aggregate_macro_f1": _bench_entry(0.80 * 1e9),
        }))
        hybrid.write_text(json.dumps({
            "eval_accuracy::hybrid_top1": _bench_entry(0.92 * 1e9),
            "eval_accuracy::hybrid_macro_f1": _bench_entry(0.91 * 1e9),
            "eval_accuracy::hybrid_reject_rate": _bench_entry(0.03 * 1e9),
            "eval_accuracy::hybrid_min_macro_f1": _bench_entry(0.78 * 1e9),
        }))

        # Try ALL 6 permutations of file order → all should yield identical results.
        from itertools import permutations
        results = []
        for files in permutations([single, multi, hybrid]):
            out = tmpd / f"r-{'_'.join(f.stem for f in files)}.json"
            r = _run_parser(tmpd / "criterion", list(files), out)
            results.append({
                "single_top1": _slo(r, "SLO-D6-ACC-1")["measured_ns"],
                "multi_min": _slo(r, "SLO-D7-MC-1")["measured_ns"],
                "hybrid_top1": _slo(r, "SLO-D8-HYB-1")["measured_ns"],
            })
        # All 6 results must be identical.
        for r in results[1:]:
            assert r == results[0], (
                f"file ordering changed parser output : {r} != {results[0]}"
            )
        print("OK : 3-file merge order-independent (6 permutations identical)")


def test_d8_real_data_lat2_separate_target():
    """SLO-D8-LAT-2 has a 5 s target (vs 3 s for SLO-D6-LAT-1)."""
    sys.path.insert(0, str(_ROOT / "scripts"))
    import importlib
    parse_mod = importlib.import_module("parse_criterion_report")
    slos = {s["id"]: s for s in parse_mod.SLO_DEFINITIONS}
    assert slos["SLO-D8-LAT-2"]["target_ns"] == 5_000_000_000
    assert slos["SLO-D8-LAT-2"]["comparator"] == "lte"
    assert slos["SLO-D6-LAT-1"]["target_ns"] == 3_000_000_000
    print("OK : SLO-D8-LAT-2 = 5s, SLO-D6-LAT-1 = 3s, distinct keys")


def main() -> int:
    tests = [
        test_d8_slos_present_in_report,
        test_d8_hyb_slos_read_hybrid_keys,
        test_d8_hyb_3_fails_on_high_reject_rate,
        test_d8_keys_disjoint_from_d6_d7_keys,
        test_d8_real_data_lat2_separate_target,
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
