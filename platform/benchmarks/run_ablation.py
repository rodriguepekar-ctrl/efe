"""Ablation runner — benchmark baseline + V1…V6 on the same dataset.

Outputs ``ablation_results.json`` with the standard metrics per variant.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

import numpy as np

from benchmarks.baseline import NaiveCosineBaseline
from benchmarks.dataset import CLASS_NAMES, build_dataset
from benchmarks.evaluate import evaluate
from benchmarks.swarm_sim import SwarmClassifier
from benchmarks.swarm_v2 import (
    SwarmV2OrthogonalProjections,
    SwarmV3TopKPooling,
    SwarmV4Mahalanobis,
    SwarmV5TemperatureCalibrated,
    SwarmV6Hybrid,
)
from benchmarks.swarm_v8 import SwarmV8CalibratedMahalanobis, SwarmV9Ensemble


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", default="benchmarks/ablation_results.json")
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--n-easy", type=int, default=250)
    parser.add_argument("--n-hard", type=int, default=250)
    args = parser.parse_args(argv)

    print("→ building dataset …", flush=True)
    ds = build_dataset(seed=args.seed, n_easy=args.n_easy, n_hard=args.n_hard)

    # V5 needs a calibration slice — use 30% of easy queries (held-out).
    n_cal = len(ds.queries_easy) // 3
    calibration_queries = ds.queries_easy[:n_cal]
    test_easy = ds.queries_easy[n_cal:]
    # Hard queries stay as-is for test.

    print(
        f"  dataset: {len(test_easy)} easy (test) + {len(ds.queries_hard)} hard "
        f"+ {n_cal} easy (cal for V5)",
        flush=True,
    )

    classifiers = {}

    print("→ building classifiers …", flush=True)
    classifiers["baseline"] = NaiveCosineBaseline(ds)
    classifiers["V1"] = SwarmClassifier(ds)
    classifiers["V2_orthogonal"] = SwarmV2OrthogonalProjections(ds)
    classifiers["V3_topk"] = SwarmV3TopKPooling(ds)
    classifiers["V4_mahalanobis"] = SwarmV4Mahalanobis(ds)
    # V5 wraps V1 with temperature fitted on calibration slice.
    print("  fitting V5 temperature on calibration slice …", flush=True)
    v1_for_v5 = SwarmClassifier(ds)
    classifiers["V5_temperature"] = SwarmV5TemperatureCalibrated(
        v1_for_v5, calibration_queries
    )
    classifiers["V6_hybrid"] = SwarmV6Hybrid(ds)

    # V7 = V6 + temperature calibration (composing the two best ideas).
    print("  fitting V7 temperature on calibration slice …", flush=True)
    v6_for_v7 = SwarmV6Hybrid(ds)
    classifiers["V7_hybrid_calibrated"] = SwarmV5TemperatureCalibrated(
        v6_for_v7, calibration_queries
    )
    # V8 = calibrated Mahalanobis (the one that wins raw accuracy).
    print("  fitting V8 calibrated Mahalanobis on calibration slice …", flush=True)
    classifiers["V8_calibrated_mahalanobis"] = SwarmV8CalibratedMahalanobis(
        ds, calibration_queries
    )
    # V9 = ensemble V1 ⊕ V4 by agreement.
    classifiers["V9_ensemble"] = SwarmV9Ensemble(ds)

    print("→ warming up …", flush=True)
    for clf in classifiers.values():
        for q in test_easy[:3]:
            clf.classify(q)

    all_queries = test_easy + ds.queries_hard
    results: dict[str, dict] = {}
    wall_clock: dict[str, float] = {}

    for name, clf in classifiers.items():
        print(f"→ running {name} on {len(all_queries)} queries …", flush=True)
        t0 = time.perf_counter()
        preds = [clf.classify(q) for q in all_queries]
        dt = time.perf_counter() - t0
        wall_clock[name] = dt
        print(f"  {len(all_queries) / dt:.0f} q/s", flush=True)

        # Split by slice.
        qid_to_query = {q.qid: q for q in all_queries}
        easy_qids = {q.qid for q in test_easy}
        hard_qids = {q.qid for q in ds.queries_hard}

        slice_metrics: dict[str, dict] = {}
        for slice_name, pick in [
            ("easy", easy_qids),
            ("hard", hard_qids),
            ("all", {q.qid for q in all_queries}),
        ]:
            sub_preds = [p for p in preds if p.qid in pick]
            sub_queries = [qid_to_query[p.qid] for p in sub_preds]
            m = evaluate(
                name=name,
                slice_label=slice_name,
                queries=sub_queries,
                predictions=sub_preds,
            )
            slice_metrics[slice_name] = m.to_dict()

        # Verdict discipline for swarm variants (baseline has none).
        verdict_stats: dict[str, dict[str, int]] = {}
        if hasattr(preds[0], "verdict"):
            for p in preds:
                q = qid_to_query[p.qid]
                bucket = verdict_stats.setdefault(
                    p.verdict, {"n": 0, "correct": 0}
                )
                bucket["n"] += 1
                if p.predicted == q.label:
                    bucket["correct"] += 1
        results[name] = {
            "slices": slice_metrics,
            "verdict_stats": verdict_stats,
            "wall_clock_seconds": round(dt, 4),
        }

    # Dump.
    blob = {
        "dataset": {
            "seed": args.seed,
            "n_classes": len(CLASS_NAMES),
            "class_names": list(CLASS_NAMES),
            "n_easy_test": len(test_easy),
            "n_easy_calibration": n_cal,
            "n_hard": len(ds.queries_hard),
        },
        "variants": results,
        "wall_clock_seconds": wall_clock,
    }
    Path(args.out).write_text(
        json.dumps(blob, ensure_ascii=False, indent=2), encoding="utf-8"
    )
    print(f"→ wrote {args.out}", flush=True)

    # Pretty-print the ablation table.
    print()
    print("=" * 90)
    print("ABLATION SUMMARY (ALL slice)")
    print("=" * 90)
    print(
        f"{'variant':<22} {'acc':>7} {'F1':>7} {'ECE':>7} "
        f"{'p50 µs':>10} {'p95 µs':>10} {'hard_acc':>9}"
    )
    print("-" * 90)
    for name in results:
        r_all = results[name]["slices"]["all"]
        r_hard = results[name]["slices"]["hard"]
        print(
            f"{name:<22} "
            f"{r_all['accuracy']:>7.3f} "
            f"{r_all['macro_f1']:>7.3f} "
            f"{r_all['ece']:>7.3f} "
            f"{r_all['latency_us']['p50']:>10.1f} "
            f"{r_all['latency_us']['p95']:>10.1f} "
            f"{r_hard['accuracy']:>9.3f}"
        )
    print()
    # Verdict discipline for swarm variants.
    print("VERDICT DISCIPLINE (CONVERGED-gated)")
    print("-" * 90)
    print(
        f"{'variant':<22} {'CONVERGED n':>14} {'coverage':>10} "
        f"{'gated acc':>12}"
    )
    for name, res in results.items():
        stats = res["verdict_stats"]
        if not stats:
            continue
        conv = stats.get("CONVERGED", {"n": 0, "correct": 0})
        total = sum(s["n"] for s in stats.values())
        cov = conv["n"] / total if total else 0.0
        acc = conv["correct"] / conv["n"] if conv["n"] else 0.0
        print(f"{name:<22} {conv['n']:>14} {cov:>10.1%} {acc:>12.3f}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
