"""Sprint D8 — Hybrid-decoder eval (Plan B activation).

Mirrors `run_eval.py` but routes every classification through the
`HybridClassifier` (raw cosine + swarm consensus) instead of the
default `EfeClient.classify` swarm-only path.

Usage (single catalog) :
    python eval/scripts/run_eval_hybrid.py \
        --catalog catalogs/support_triage.yaml \
        --golden  eval/golden/support_triage.jsonl \
        --endpoint 127.0.0.1:50051 \
        --output target/eval-hybrid.json \
        --output-bench-format target/eval-hybrid-bench.json

Usage (multi-catalog) :
    python eval/scripts/run_eval_hybrid.py --multi-catalog \
        --catalog catalogs/support_triage.yaml --golden eval/golden/support_triage.jsonl \
        --catalog catalogs/support_triage.yaml --golden eval/golden/fraud_alert.jsonl \
        --catalog catalogs/support_triage.yaml --golden eval/golden/billing_escalation.jsonl \
        --endpoint 127.0.0.1:50051

The bench-format output uses **dedicated keys** (`eval_accuracy::hybrid_*`)
that are DISJOINT from the swarm-only keys produced by `run_eval.py`.
The full disjointness invariant is pinned by
`scripts/tests/test_d8_hybrid_eval.py` (single + multi hybrid keys
vs swarm-only single + multi keys, total 22 unique keys) and
`scripts/tests/test_d8_parser_slo_definitions.py` (6-permutation
order-independence test through the parser merge). The contract :

    swarm-only (`run_eval.py`):
        eval_accuracy::top1
        eval_accuracy::macro_f1
        eval_accuracy::mean_conf_correct
        eval_accuracy::false_reject_rate
        eval_latency::mean_ns
        eval_latency::max_ns

    swarm-only multi (`run_eval.py --multi-catalog`):
        eval_accuracy::min_top1
        eval_accuracy::min_macro_f1
        eval_accuracy::aggregate_top1
        eval_accuracy::aggregate_macro_f1

    hybrid (this script):
        eval_accuracy::hybrid_top1
        eval_accuracy::hybrid_macro_f1
        eval_accuracy::hybrid_branch_agree
        eval_accuracy::hybrid_branch_raw_overrides
        eval_accuracy::hybrid_branch_swarm_overrides
        eval_accuracy::hybrid_branch_reject

    hybrid multi (this script --multi-catalog):
        eval_accuracy::hybrid_min_top1
        eval_accuracy::hybrid_min_macro_f1
        eval_accuracy::hybrid_aggregate_top1
        eval_accuracy::hybrid_aggregate_macro_f1
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import time
from collections import Counter
from pathlib import Path
from typing import Any

# Allow `python eval/scripts/run_eval_hybrid.py` to find the local efe_tool package.
_THIS = Path(__file__).resolve()
_ROOT = _THIS.parents[2]
_CLIENT = _ROOT / "clients" / "python"
if (_CLIENT / "efe_tool").is_dir():
    sys.path.insert(0, str(_CLIENT))

# Import compute_metrics from the sibling run_eval.py to avoid duplicating
# the metrics computation logic (single source of truth).
sys.path.insert(0, str(_THIS.parent))
from run_eval import compute_metrics, _print_report, _bench_entry, load_golden  # noqa: E402

from efe_tool import AnchorCatalog, EfeClient, LocalSTEmbedder  # noqa: E402
from efe_tool.integrations.hybrid_decoder import (  # noqa: E402
    HybridClassifier,
    HybridResult,
)


# Sprint D8 — Hybrid-mode-specific gates. Slightly more lenient than
# the swarm-only gates because the hybrid intentionally rejects
# (returns "uncertain") on borderline cases ; the reject_rate gate
# bounds this so the hybrid doesn't degenerate into a "reject
# everything" classifier.
HYBRID_GATES = {
    "hybrid_accuracy_top1_min": 0.80,
    "hybrid_macro_f1_min": 0.75,
    "hybrid_reject_rate_max": 0.15,  # ≤ 15 % uncertain labels
    "hybrid_mean_conf_correct_min": 0.65,
}

HYBRID_MULTI_CATALOG_GATES = {
    "hybrid_aggregate_macro_f1_min": 0.75,
    "hybrid_min_per_catalog_accuracy_min": 0.65,
    "hybrid_min_per_catalog_macro_f1_min": 0.60,
}


# ---------------------------------------------------------------------------
# One-catalog runner
# ---------------------------------------------------------------------------


def _evaluate_one_hybrid(
    *,
    catalog: str,
    golden: str,
    endpoint: str,
    verbose: bool,
) -> tuple[dict[str, Any], list[str]]:
    """Run the hybrid eval for a single (catalog, golden) pair.

    Returns
    -------
    (report, class_names)
        `report` augmented with a `hybrid_branches` Counter mapping
        each path (`agree` / `raw_overrides_swarm` /
        `swarm_overrides_raw` / `reject`) to its incidence count.
    """
    os.environ.setdefault("EFE_ENDPOINT", endpoint)
    emb = LocalSTEmbedder()
    cat = AnchorCatalog.load(catalog, embedder=emb)
    items = load_golden(golden)

    print(f"catalog  : {cat.name}@{cat.version} (N={len(cat.classes)})")
    print(f"golden   : {len(items)} examples ({golden})")
    print(f"endpoint : {endpoint}  [HYBRID DECODER ACTIVATED — Plan B]")
    print()

    client = EfeClient.from_env()
    try:
        hybrid = HybridClassifier(client=client, catalog=cat)
        preds: list[tuple[str, str, bool, float]] = []
        latencies: list[float] = []
        branch_counter: Counter[str] = Counter()
        t0 = time.perf_counter()
        for it in items:
            expected = it["expected"]
            t = time.perf_counter()
            try:
                result: HybridResult = hybrid.classify(it["text"])
            except Exception as e:
                print(f"  !! {it['id']} error: {e}")
                preds.append((expected, "error", True, 0.0))
                latencies.append(0.0)
                branch_counter["error"] += 1
                continue
            latencies.append((time.perf_counter() - t) * 1000.0)
            # Map "uncertain" to reject for the metrics path.
            predicted = result.label if result.label != "uncertain" else "uncertain"
            is_reject = predicted == "uncertain"
            preds.append((expected, predicted, is_reject, result.confidence))
            branch_counter[result.path] += 1
            if verbose:
                print(
                    f"  {it['id']:>5}  exp={expected:<9}  got={predicted:<10}  "
                    f"conf={result.confidence:.2f}  path={result.path:<22}  "
                    f"raw={result.raw_label:<10}  swarm={result.swarm_label}"
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
    report["hybrid_branches"] = dict(branch_counter)
    report["hybrid_reject_rate"] = round(
        branch_counter.get("reject", 0) / max(1, len(items)), 4
    )
    return report, list(cat.class_names())


def _print_hybrid_branches(branches: dict[str, int], n: int) -> None:
    print("\n--- HYBRID BRANCH DISTRIBUTION ---")
    for path in ("agree", "raw_overrides_swarm", "swarm_overrides_raw", "reject"):
        count = branches.get(path, 0)
        pct = 100.0 * count / max(1, n)
        print(f"  {path:<22} {count:>3} / {n}  ({pct:>5.1f} %)")


def run(
    *,
    catalog: str,
    golden: str,
    endpoint: str,
    output: str | None = None,
    output_bench_format: str | None = None,
    verbose: bool = False,
) -> int:
    report, class_names = _evaluate_one_hybrid(
        catalog=catalog, golden=golden, endpoint=endpoint, verbose=verbose
    )
    _print_report(report, class_names)
    _print_hybrid_branches(report["hybrid_branches"], report["summary"]["n"])

    if output:
        Path(output).write_text(json.dumps(report, indent=2, ensure_ascii=False))
        print(f"\nwrote {output}")

    if output_bench_format:
        summary = report["summary"]
        branches = report["hybrid_branches"]
        n = summary["n"]
        bench_payload = {
            # Dedicated `hybrid_*` keys, disjoint from swarm-only.
            "eval_accuracy::hybrid_top1": _bench_entry(summary["accuracy_top1"] * 1e9),
            "eval_accuracy::hybrid_macro_f1": _bench_entry(summary["macro_f1"] * 1e9),
            "eval_accuracy::hybrid_mean_conf_correct": _bench_entry(
                summary["mean_conf_correct"] * 1e9
            ),
            "eval_accuracy::hybrid_reject_rate": _bench_entry(
                report["hybrid_reject_rate"] * 1e9
            ),
            "eval_accuracy::hybrid_branch_agree": _bench_entry(
                branches.get("agree", 0) / max(1, n) * 1e9
            ),
            "eval_accuracy::hybrid_branch_raw_overrides": _bench_entry(
                branches.get("raw_overrides_swarm", 0) / max(1, n) * 1e9
            ),
            "eval_accuracy::hybrid_branch_swarm_overrides": _bench_entry(
                branches.get("swarm_overrides_raw", 0) / max(1, n) * 1e9
            ),
            "eval_accuracy::hybrid_branch_reject": _bench_entry(
                branches.get("reject", 0) / max(1, n) * 1e9
            ),
        }
        Path(output_bench_format).write_text(json.dumps(bench_payload, indent=2))
        print(f"wrote {output_bench_format} (bench-format, hybrid)")

    rc = 0
    s = report["summary"]
    if s["accuracy_top1"] < HYBRID_GATES["hybrid_accuracy_top1_min"]:
        print(f"GATE FAIL : hybrid_accuracy_top1 = {s['accuracy_top1']} < "
              f"{HYBRID_GATES['hybrid_accuracy_top1_min']}")
        rc = 1
    if s["macro_f1"] < HYBRID_GATES["hybrid_macro_f1_min"]:
        print(f"GATE FAIL : hybrid_macro_f1 = {s['macro_f1']} < "
              f"{HYBRID_GATES['hybrid_macro_f1_min']}")
        rc = 1
    if report["hybrid_reject_rate"] > HYBRID_GATES["hybrid_reject_rate_max"]:
        print(f"GATE FAIL : hybrid_reject_rate = {report['hybrid_reject_rate']} > "
              f"{HYBRID_GATES['hybrid_reject_rate_max']}")
        rc = 1
    if rc == 0:
        print("\nOK — all D8 hybrid gates pass.")
    return rc


# ---------------------------------------------------------------------------
# Multi-catalog runner
# ---------------------------------------------------------------------------


def run_multi_catalog(
    *,
    catalogs: list[str],
    goldens: list[str],
    endpoint: str,
    output: str | None = None,
    output_bench_format: str | None = None,
    verbose: bool = False,
) -> int:
    """Sprint D8 — multi-catalog hybrid eval. Mirror of
    `run_eval.py::run_multi_catalog` but using the hybrid path.
    """
    if len(catalogs) != len(goldens):
        print(
            f"FATAL : --catalog count ({len(catalogs)}) does not match "
            f"--golden count ({len(goldens)})."
        )
        return 2
    if not catalogs:
        print("FATAL : no (catalog, golden) pair provided.")
        return 2

    per_catalog_reports: list[dict[str, Any]] = []
    for idx, (cat_path, gold_path) in enumerate(zip(catalogs, goldens), start=1):
        print(f"\n========== catalog {idx}/{len(catalogs)} ==========")
        report, class_names = _evaluate_one_hybrid(
            catalog=cat_path, golden=gold_path, endpoint=endpoint, verbose=verbose
        )
        _print_report(report, class_names)
        _print_hybrid_branches(report["hybrid_branches"], report["summary"]["n"])
        per_catalog_reports.append(report)

    accs = [r["summary"]["accuracy_top1"] for r in per_catalog_reports]
    f1s = [r["summary"]["macro_f1"] for r in per_catalog_reports]
    rejects = [r["hybrid_reject_rate"] for r in per_catalog_reports]

    aggregate = {
        "n_catalogs": len(per_catalog_reports),
        "hybrid_aggregate_accuracy_top1": round(sum(accs) / len(accs), 4),
        "hybrid_aggregate_macro_f1": round(sum(f1s) / len(f1s), 4),
        "hybrid_min_per_catalog_accuracy": round(min(accs), 4),
        "hybrid_min_per_catalog_macro_f1": round(min(f1s), 4),
        "hybrid_max_reject_rate": round(max(rejects), 4),
        "per_catalog": [
            {
                "catalog": r["catalog"],
                "golden_path": r["golden_path"],
                "accuracy_top1": r["summary"]["accuracy_top1"],
                "macro_f1": r["summary"]["macro_f1"],
                "reject_rate": r["hybrid_reject_rate"],
                "branches": r["hybrid_branches"],
                "n": r["summary"]["n"],
            }
            for r in per_catalog_reports
        ],
    }

    print("\n========== AGGREGATE (D8 hybrid multi-catalog) ==========")
    for k, v in aggregate.items():
        if k == "per_catalog":
            continue
        print(f"  {k:<40} {v}")

    if output:
        full = {"aggregate": aggregate, "per_catalog_reports": per_catalog_reports}
        Path(output).write_text(json.dumps(full, indent=2, ensure_ascii=False))
        print(f"\nwrote {output}")

    if output_bench_format:
        bench_payload = {
            "eval_accuracy::hybrid_min_top1": _bench_entry(
                aggregate["hybrid_min_per_catalog_accuracy"] * 1e9
            ),
            "eval_accuracy::hybrid_min_macro_f1": _bench_entry(
                aggregate["hybrid_min_per_catalog_macro_f1"] * 1e9
            ),
            "eval_accuracy::hybrid_aggregate_top1": _bench_entry(
                aggregate["hybrid_aggregate_accuracy_top1"] * 1e9
            ),
            "eval_accuracy::hybrid_aggregate_macro_f1": _bench_entry(
                aggregate["hybrid_aggregate_macro_f1"] * 1e9
            ),
        }
        Path(output_bench_format).write_text(json.dumps(bench_payload, indent=2))
        print(f"wrote {output_bench_format} (bench-format, hybrid multi-catalog)")

    rc = 0
    for k, threshold in HYBRID_MULTI_CATALOG_GATES.items():
        if not k.endswith("_min"):
            continue
        metric = k[:-4]
        if aggregate.get(metric, 0.0) < threshold:
            print(
                f"GATE FAIL : {metric} = {aggregate.get(metric)} < {threshold} "
                f"(D8 hybrid multi-catalog gate)"
            )
            rc = 1
    if rc == 0:
        print("\nOK — all D8 hybrid multi-catalog gates pass.")
    return rc


# ---------------------------------------------------------------------------
# CLI / main
# ---------------------------------------------------------------------------


def _main() -> int:
    parser = argparse.ArgumentParser(
        description="Sprint D8 — Hybrid-decoder eval (Plan B)."
    )
    parser.add_argument("--catalog", action="append", required=True)
    parser.add_argument("--golden", action="append", required=True)
    parser.add_argument("--endpoint", default="127.0.0.1:50051")
    parser.add_argument("--output", default=None)
    parser.add_argument("--output-bench-format", default=None)
    parser.add_argument("--multi-catalog", action="store_true")
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
        print("FATAL : single-catalog mode expects exactly one --catalog "
              "and one --golden.")
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
