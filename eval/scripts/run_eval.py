"""Golden-set evaluation of a Mode-1 catalog.

Usage (single catalog):
    python eval/scripts/run_eval.py \\
        --catalog catalogs/support_triage.yaml \\
        --golden  eval/golden/support_triage.jsonl \\
        --endpoint 127.0.0.1:50051

Usage (Sprint D7 multi-catalog mode):
    python eval/scripts/run_eval.py \\
        --multi-catalog \\
        --catalog catalogs/support_triage.yaml \\
        --golden  eval/golden/support_triage.jsonl \\
        --catalog catalogs/support_triage.yaml \\
        --golden  eval/golden/fraud_alert.jsonl \\
        --catalog catalogs/support_triage.yaml \\
        --golden  eval/golden/billing_escalation.jsonl \\
        --endpoint 127.0.0.1:50051

In multi-catalog mode the script runs the eval on each (catalog, golden)
pair in order, then prints :
    * per-catalog summaries
    * an aggregate {min, mean} of accuracy + macro_f1 across catalogs
    * a multi-catalog gate (`MIN_PER_CATALOG_*`) on top of the
      single-catalog `GATES` (D7 anti-overfit guard).

Emits to stdout:
    * per-class precision / recall / F1
    * macro F1
    * confusion matrix
    * calibration (mean confidence on correct vs incorrect)
    * reject statistics

Exits non-zero when any quality gate is violated (see `GATES` /
`MULTI_CATALOG_GATES` below).
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

# Allow `python eval/scripts/run_eval.py` to find the local efe_tool package.
_THIS = Path(__file__).resolve()
_ROOT = _THIS.parents[2]
_CLIENT = _ROOT / "clients" / "python"
if (_CLIENT / "efe_tool").is_dir():
    sys.path.insert(0, str(_CLIENT))

from efe_tool import AnchorCatalog, EfeClient, LocalSTEmbedder  # noqa: E402


GATES = {
    "accuracy_top1_min": 0.75,     # softened vs the archi doc (85 %) while MVP anchors are bootstrapped
    "macro_f1_min": 0.70,
    "false_reject_rate_max": 0.25,
    "mean_conf_correct_min": 0.65,
}

# Sprint D7 — multi-catalog gates. Tighter than the per-catalog gates
# precisely so the calibration sweep is forced to find a (β*, α*) that
# generalises rather than overfit one of the 3 golden sets.
MULTI_CATALOG_GATES = {
    # The aggregate macro_f1 (mean across catalogs) must clear 0.75.
    "aggregate_macro_f1_min": 0.75,
    # The *worst* per-catalog accuracy must clear 0.65 — anti-overfit.
    "min_per_catalog_accuracy_min": 0.65,
    # The *worst* per-catalog macro_f1 must clear 0.60 — same rationale.
    "min_per_catalog_macro_f1_min": 0.60,
}


def load_golden(path: str | Path) -> list[dict[str, Any]]:
    items: list[dict[str, Any]] = []
    with Path(path).open("r", encoding="utf-8") as f:
        for lineno, raw in enumerate(f, start=1):
            raw = raw.strip()
            if not raw:
                continue
            try:
                items.append(json.loads(raw))
            except json.JSONDecodeError as e:
                raise ValueError(f"{path}:{lineno}: invalid JSON ({e})") from e
    for it in items:
        for k in ("id", "text", "expected"):
            if k not in it:
                raise ValueError(f"golden entry missing '{k}': {it}")
    return items


def _bench_entry(value: float) -> dict[str, float]:
    """Sprint D6 — emit a parser-friendly per-metric record.

    The parser expects criterion-shaped entries with `mean_ns` and
    `approx_p95_ns` fields. We populate both with the same value
    since accuracy metrics have no distribution (single
    measurement). `std_dev_ns` and `mad_ns` are 0 by definition.
    """
    return {
        "mean_ns": float(value),
        "median_ns": float(value),
        "std_dev_ns": 0.0,
        "mad_ns": 0.0,
        "approx_p95_ns": float(value),
    }


def _evaluate_one(
    *,
    catalog: str,
    golden: str,
    endpoint: str,
    verbose: bool,
) -> tuple[dict[str, Any], list[str]]:
    """Run the eval pipeline for a single (catalog, golden) pair and
    return the (report, class_names) tuple.

    Sprint D7 — extracted from `run()` so the multi-catalog mode can
    iterate cheaply over many pairs without duplicating the embedder /
    client construction logic.
    """
    os.environ.setdefault("EFE_ENDPOINT", endpoint)
    emb = LocalSTEmbedder()
    cat = AnchorCatalog.load(catalog, embedder=emb)
    items = load_golden(golden)

    print(f"catalog  : {cat.name}@{cat.version} (N={len(cat.classes)})")
    print(f"golden   : {len(items)} examples ({golden})")
    print(f"endpoint : {endpoint}")
    print()

    client = EfeClient.from_env()
    try:
        preds: list[tuple[str, str, bool, float]] = []
        latencies: list[float] = []
        t0 = time.perf_counter()
        for it in items:
            expected = it["expected"]
            t = time.perf_counter()
            try:
                result = client.classify(it["text"], cat)
            except Exception as e:
                print(f"  !! {it['id']} error: {e}")
                preds.append((expected, "error", True, 0.0))
                latencies.append(0.0)
                continue
            latencies.append((time.perf_counter() - t) * 1000.0)
            predicted = result.label if not result.reject else "uncertain"
            preds.append((expected, predicted, result.reject, result.confidence))
            if verbose:
                print(
                    f"  {it['id']:>5}  exp={expected:<9}  got={predicted:<10}  "
                    f"conf={result.confidence:.2f}  rej={result.reject}"
                )
        total_s = time.perf_counter() - t0
    finally:
        client.close()

    report = compute_metrics(preds, labels=list(cat.class_names()))
    report["latency_ms"] = {
        "mean": round(sum(latencies) / max(1, len(latencies)), 2),
        "max": round(max(latencies or [0.0]), 2),
        "count": len(latencies),
    }
    report["total_seconds"] = round(total_s, 2)
    report["catalog"] = cat.name
    report["catalog_version"] = cat.version
    report["golden_path"] = str(golden)
    return report, list(cat.class_names())


def run(
    *,
    catalog: str,
    golden: str,
    endpoint: str,
    output: str | None = None,
    output_bench_format: str | None = None,
    verbose: bool = False,
) -> int:
    report, class_names = _evaluate_one(
        catalog=catalog,
        golden=golden,
        endpoint=endpoint,
        verbose=verbose,
    )

    _print_report(report, class_names)

    if output:
        Path(output).write_text(json.dumps(report, indent=2, ensure_ascii=False))
        print(f"\nwrote {output}")

    # --- Sprint D6 : bench-compatible output ---
    #
    # When --output-bench-format X.json is set, emit a parser-friendly
    # mapping that `parse_criterion_report.py` can consume in addition
    # to the criterion estimates tree. The mapping uses the same
    # `<group>::<id>` key convention as criterion : numeric metrics
    # are scaled by 1e9 so the ns-based parser sees them as
    # comparable durations (the new `gte` comparator in
    # `evaluate_slos()` handles the "higher is better" semantics).
    #
    # Sprint D7 : the keys here are intentionally DISJOINT from those
    # produced by `run_multi_catalog` (which uses `min_top1`,
    # `min_macro_f1`, `aggregate_top1`, `aggregate_macro_f1`). The
    # disjointness is pinned by `scripts/tests/test_d7_parser_eval_merge.py`.
    if output_bench_format:
        summary = report["summary"]
        latency = report["latency_ms"]
        bench_payload = {
            # Accuracy ratios scaled to ns-equivalent for the parser.
            "eval_accuracy::top1": _bench_entry(summary["accuracy_top1"] * 1e9),
            "eval_accuracy::macro_f1": _bench_entry(summary["macro_f1"] * 1e9),
            "eval_accuracy::mean_conf_correct": _bench_entry(summary["mean_conf_correct"] * 1e9),
            "eval_accuracy::false_reject_rate": _bench_entry(summary["false_reject_rate"] * 1e9),
            # Client-observed latency (ms → ns).
            "eval_latency::mean_ns": _bench_entry(latency["mean"] * 1e6),
            "eval_latency::max_ns": _bench_entry(latency["max"] * 1e6),
        }
        Path(output_bench_format).write_text(json.dumps(bench_payload, indent=2))
        print(f"wrote {output_bench_format} (bench-format)")

    # --- Gate check ---
    rc = 0
    for gate, threshold in GATES.items():
        if gate.endswith("_min") and report["summary"][gate[:-4]] < threshold:
            print(f"GATE FAIL : {gate[:-4]} = {report['summary'][gate[:-4]]} < {threshold}")
            rc = 1
        if gate.endswith("_max") and report["summary"][gate[:-4]] > threshold:
            print(f"GATE FAIL : {gate[:-4]} = {report['summary'][gate[:-4]]} > {threshold}")
            rc = 1
    if rc == 0:
        print("\nOK — all quality gates pass.")
    return rc


def run_multi_catalog(
    *,
    catalogs: list[str],
    goldens: list[str],
    endpoint: str,
    output: str | None = None,
    output_bench_format: str | None = None,
    verbose: bool = False,
) -> int:
    """Sprint D7 — run the eval pipeline on N (catalog, golden) pairs
    and gate on the **aggregate** + **min per-catalog** metrics.

    The contract :
        len(catalogs) == len(goldens), pairs are zipped in order.
        Each pair is run through `_evaluate_one`. Per-catalog gate
        violations are reported as warnings — only the aggregate +
        min gates are hard gates (anti-overfit).
    """
    if len(catalogs) != len(goldens):
        print(
            f"FATAL : --catalog count ({len(catalogs)}) does not match "
            f"--golden count ({len(goldens)}) ; pairs are zipped 1:1."
        )
        return 2
    if not catalogs:
        print("FATAL : no (catalog, golden) pair provided.")
        return 2

    per_catalog_reports: list[dict[str, Any]] = []
    for idx, (cat_path, gold_path) in enumerate(zip(catalogs, goldens), start=1):
        print(f"\n========== catalog {idx}/{len(catalogs)} ==========")
        report, class_names = _evaluate_one(
            catalog=cat_path,
            golden=gold_path,
            endpoint=endpoint,
            verbose=verbose,
        )
        _print_report(report, class_names)
        per_catalog_reports.append(report)

    # ----- Aggregate -----
    accs = [r["summary"]["accuracy_top1"] for r in per_catalog_reports]
    f1s = [r["summary"]["macro_f1"] for r in per_catalog_reports]
    aggregate = {
        "n_catalogs": len(per_catalog_reports),
        "aggregate_accuracy_top1": round(sum(accs) / len(accs), 4),
        "aggregate_macro_f1": round(sum(f1s) / len(f1s), 4),
        "min_per_catalog_accuracy": round(min(accs), 4),
        "min_per_catalog_macro_f1": round(min(f1s), 4),
        "max_per_catalog_accuracy": round(max(accs), 4),
        "max_per_catalog_macro_f1": round(max(f1s), 4),
        "per_catalog": [
            {
                "catalog": r["catalog"],
                "golden_path": r["golden_path"],
                "accuracy_top1": r["summary"]["accuracy_top1"],
                "macro_f1": r["summary"]["macro_f1"],
                "false_reject_rate": r["summary"]["false_reject_rate"],
                "n": r["summary"]["n"],
            }
            for r in per_catalog_reports
        ],
    }

    print("\n========== AGGREGATE (D7 multi-catalog) ==========")
    for k, v in aggregate.items():
        if k == "per_catalog":
            continue
        print(f"  {k:<32} {v}")
    print()
    for entry in aggregate["per_catalog"]:
        print(
            f"  {entry['catalog']:<22}  acc={entry['accuracy_top1']:.4f}  "
            f"f1={entry['macro_f1']:.4f}  rej={entry['false_reject_rate']:.4f}  "
            f"n={entry['n']}  ({entry['golden_path']})"
        )

    if output:
        full = {
            "aggregate": aggregate,
            "per_catalog_reports": per_catalog_reports,
        }
        Path(output).write_text(json.dumps(full, indent=2, ensure_ascii=False))
        print(f"\nwrote {output}")

    if output_bench_format:
        # Sprint D7 — emit multi-catalog metrics under DEDICATED keys so
        # they NEVER collide with the single-catalog `eval_accuracy::top1`
        # / `::macro_f1` keys produced by `run()`. The parser merges
        # both files into one results dict ; if we reused the same keys,
        # the multi-catalog (worst-case) value would overwrite the
        # single-catalog (support_triage-specific) value, silently
        # changing the meaning of `SLO-D6-ACC-*` (which conceptually
        # gate "the support_triage golden alone").
        #
        # By emitting `min_top1` / `min_macro_f1` instead, we guarantee:
        #   * `SLO-D6-ACC-1`     reads `eval_accuracy::top1`         (single)
        #   * `SLO-D6-ACC-2`     reads `eval_accuracy::macro_f1`     (single)
        #   * `SLO-D7-MC-1`      reads `eval_accuracy::min_top1`     (multi)
        #   * `SLO-D7-MC-2`      reads `eval_accuracy::min_macro_f1` (multi)
        #   * `SLO-D7-MC-3`      reads `eval_accuracy::aggregate_top1`
        #   * `SLO-D7-MC-4`      reads `eval_accuracy::aggregate_macro_f1`
        # → The two SLO families coexist without order dependency.
        bench_payload = {
            "eval_accuracy::min_top1": _bench_entry(
                aggregate["min_per_catalog_accuracy"] * 1e9
            ),
            "eval_accuracy::min_macro_f1": _bench_entry(
                aggregate["min_per_catalog_macro_f1"] * 1e9
            ),
            "eval_accuracy::aggregate_top1": _bench_entry(
                aggregate["aggregate_accuracy_top1"] * 1e9
            ),
            "eval_accuracy::aggregate_macro_f1": _bench_entry(
                aggregate["aggregate_macro_f1"] * 1e9
            ),
        }
        Path(output_bench_format).write_text(json.dumps(bench_payload, indent=2))
        print(f"wrote {output_bench_format} (bench-format, multi-catalog)")

    # ----- Gates -----
    rc = 0
    for k, threshold in MULTI_CATALOG_GATES.items():
        # Gate names end with `_min` ; strip the suffix to find the
        # metric key in `aggregate`.
        if not k.endswith("_min"):
            continue
        metric = k[:-4]  # e.g. "aggregate_macro_f1"
        if aggregate.get(metric, 0.0) < threshold:
            print(
                f"GATE FAIL : {metric} = {aggregate.get(metric)} < {threshold} "
                f"(D7 multi-catalog gate)"
            )
            rc = 1
    if rc == 0:
        print("\nOK — all D7 multi-catalog gates pass.")
    return rc


def compute_metrics(
    preds: list[tuple[str, str, bool, float]],
    *,
    labels: list[str],
) -> dict[str, Any]:
    """Compute per-class precision/recall/F1 + confusion + calibration."""
    n = len(preds)
    all_labels = sorted(set(labels) | {"uncertain"})
    conf_matrix: dict[str, Counter[str]] = defaultdict(Counter)
    correct = 0
    correct_confs: list[float] = []
    incorrect_confs: list[float] = []
    reject_correct_class = 0
    reject_unknown_class = 0
    for expected, predicted, reject, conf in preds:
        conf_matrix[expected][predicted] += 1
        if predicted == expected:
            correct += 1
            correct_confs.append(conf)
        else:
            incorrect_confs.append(conf)
        if reject:
            if expected in labels:
                reject_correct_class += 1  # false reject — expected was in catalog
            else:
                reject_unknown_class += 1  # useful reject

    tp: Counter[str] = Counter()
    fp: Counter[str] = Counter()
    fn: Counter[str] = Counter()
    for exp, row in conf_matrix.items():
        for pred, count in row.items():
            if exp == pred:
                tp[exp] += count
            else:
                fp[pred] += count
                fn[exp] += count

    per_class: dict[str, dict[str, float]] = {}
    for label in labels:
        prec = tp[label] / (tp[label] + fp[label]) if (tp[label] + fp[label]) else 0.0
        rec = tp[label] / (tp[label] + fn[label]) if (tp[label] + fn[label]) else 0.0
        f1 = 2 * prec * rec / (prec + rec) if (prec + rec) else 0.0
        per_class[label] = {
            "precision": round(prec, 4),
            "recall": round(rec, 4),
            "f1": round(f1, 4),
            "support": int(tp[label] + fn[label]),
        }
    macro_f1 = sum(v["f1"] for v in per_class.values()) / max(1, len(per_class))
    accuracy = correct / max(1, n)
    return {
        "summary": {
            "accuracy_top1": round(accuracy, 4),
            "macro_f1": round(macro_f1, 4),
            "false_reject_rate": round(reject_correct_class / max(1, n), 4),
            "useful_reject_count": reject_unknown_class,
            "mean_conf_correct": round(
                sum(correct_confs) / max(1, len(correct_confs)), 4
            ),
            "mean_conf_incorrect": round(
                sum(incorrect_confs) / max(1, len(incorrect_confs)), 4
            ),
            "n": n,
        },
        "per_class": per_class,
        "confusion": {exp: dict(row) for exp, row in conf_matrix.items()},
    }


def _print_report(report: dict[str, Any], labels: tuple[str, ...] | list[str]) -> None:
    s = report["summary"]
    print("--- SUMMARY ---")
    for k, v in s.items():
        print(f"  {k:<24} {v}")
    print("\n--- PER CLASS ---")
    print(f"  {'label':<10} {'prec':>6} {'rec':>6} {'f1':>6} {'n':>4}")
    for label in labels:
        pc = report["per_class"][label]
        print(
            f"  {label:<10} {pc['precision']:>6.3f} {pc['recall']:>6.3f} "
            f"{pc['f1']:>6.3f} {pc['support']:>4}"
        )
    print("\n--- CONFUSION (rows=expected, cols=predicted) ---")
    all_preds = sorted({p for row in report["confusion"].values() for p in row})
    print("  " + "  ".join(f"{c:<10}" for c in ["↓ / →", *all_preds]))
    for exp in sorted(report["confusion"]):
        cells = [f"{report['confusion'][exp].get(p, 0):<10d}" for p in all_preds]
        print(f"  {exp:<10}  " + "  ".join(cells))
    print(f"\n--- LATENCY ---\n  {report.get('latency_ms')}")


def _main() -> int:
    parser = argparse.ArgumentParser()
    # `action="append"` lets `--catalog A --catalog B` accumulate multiple
    # values when `--multi-catalog` is set. In single-catalog mode we
    # extract `[0]` and assert exactly one entry was passed.
    parser.add_argument("--catalog", action="append", required=True)
    parser.add_argument("--golden", action="append", required=True)
    parser.add_argument("--endpoint", default="127.0.0.1:50051")
    parser.add_argument("--output", default=None)
    # Sprint D6 — parser-compatible accuracy metrics output.
    parser.add_argument(
        "--output-bench-format",
        default=None,
        help="Emit accuracy/latency metrics in `parse_criterion_report.py` "
             "format so they merge into the unified bench SLO compliance.",
    )
    # Sprint D7 — multi-catalog anti-overfit mode.
    parser.add_argument(
        "--multi-catalog",
        action="store_true",
        help="Run on N (catalog, golden) pairs (zipped in order) and "
             "gate on aggregate + min-per-catalog metrics. Use "
             "--catalog / --golden multiple times.",
    )
    parser.add_argument("-v", "--verbose", action="store_true")
    args = parser.parse_args()
    if args.multi_catalog:
        return run_multi_catalog(
            catalogs=args.catalog,
            goldens=args.golden,
            endpoint=args.endpoint,
            output=args.output,
            output_bench_format=args.output_bench_format,
            verbose=args.verbose,
        )
    if len(args.catalog) != 1 or len(args.golden) != 1:
        print(
            "FATAL : single-catalog mode expects exactly one --catalog "
            "and one --golden ; pass --multi-catalog if you intended "
            "to evaluate multiple pairs."
        )
        return 2
    return run(
        catalog=args.catalog[0],
        golden=args.golden[0],
        endpoint=args.endpoint,
        output=args.output,
        output_bench_format=args.output_bench_format,
        verbose=args.verbose,
    )


if __name__ == "__main__":
    sys.exit(_main())
