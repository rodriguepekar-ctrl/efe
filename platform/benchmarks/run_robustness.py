"""Multi-seed robustness sweep for the ablation report.

Re-runs baseline + V1 + V4 + V9 on 5 different seeds, produces
``robustness_results.json`` with per-seed and pooled statistics.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import numpy as np

from benchmarks.baseline import NaiveCosineBaseline
from benchmarks.dataset import build_dataset
from benchmarks.swarm_sim import SwarmClassifier
from benchmarks.swarm_v2 import SwarmV4Mahalanobis
from benchmarks.swarm_v8 import SwarmV9Ensemble


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", default="benchmarks/robustness_results.json")
    parser.add_argument("--seeds", nargs="*", type=int, default=[42, 123, 777, 2024, 31415])
    parser.add_argument("--n-easy", type=int, default=250)
    parser.add_argument("--n-hard", type=int, default=250)
    args = parser.parse_args(argv)

    variants = {
        "baseline": NaiveCosineBaseline,
        "V1": SwarmClassifier,
        "V4_mahalanobis": SwarmV4Mahalanobis,
        "V9_ensemble": SwarmV9Ensemble,
    }
    per_seed: dict[str, list[dict[str, float]]] = {name: [] for name in variants}

    for seed in args.seeds:
        print(f"→ seed={seed}", flush=True)
        ds = build_dataset(seed=seed, n_easy=args.n_easy, n_hard=args.n_hard)
        for name, cls in variants.items():
            clf = cls(ds)
            all_q = ds.all_queries()
            preds = [clf.classify(q) for q in all_q]
            # ALL accuracy
            acc_all = sum(
                1 for q, p in zip(all_q, preds) if p.predicted == q.label
            ) / len(all_q)
            # HARD accuracy
            hard_preds = [p for p in preds if p.qid.startswith("hard")]
            hard_q = [q for q in all_q if q.qid.startswith("hard")]
            acc_hard = sum(
                1 for q, p in zip(hard_q, hard_preds) if p.predicted == q.label
            ) / len(hard_q)
            # EASY
            easy_preds = [p for p in preds if p.qid.startswith("easy")]
            easy_q = [q for q in all_q if q.qid.startswith("easy")]
            acc_easy = sum(
                1 for q, p in zip(easy_q, easy_preds) if p.predicted == q.label
            ) / len(easy_q)
            per_seed[name].append(
                {
                    "seed": seed,
                    "accuracy_all": acc_all,
                    "accuracy_hard": acc_hard,
                    "accuracy_easy": acc_easy,
                }
            )

    # Stats.
    stats: dict[str, dict[str, dict[str, float]]] = {}
    for name, runs in per_seed.items():
        metric_stats = {}
        for metric in ("accuracy_all", "accuracy_hard", "accuracy_easy"):
            vals = np.array([r[metric] for r in runs])
            metric_stats[metric] = {
                "mean": float(vals.mean()),
                "std": float(vals.std()),
                "min": float(vals.min()),
                "max": float(vals.max()),
            }
        stats[name] = metric_stats

    # Paired Δ vs baseline with z-scores.
    base_runs = per_seed["baseline"]
    deltas: dict[str, dict[str, dict[str, float]]] = {}
    for name in variants:
        if name == "baseline":
            continue
        diffs = {"accuracy_all": [], "accuracy_hard": [], "accuracy_easy": []}
        for r_base, r in zip(base_runs, per_seed[name]):
            for metric in diffs:
                diffs[metric].append(r[metric] - r_base[metric])
        deltas[name] = {
            metric: {
                "mean_delta": float(np.mean(diffs[metric])),
                "std_delta": float(np.std(diffs[metric])),
                "z": float(
                    np.mean(diffs[metric]) / (np.std(diffs[metric]) + 1e-9)
                ),
            }
            for metric in diffs
        }

    blob = {
        "seeds": args.seeds,
        "per_seed": per_seed,
        "stats": stats,
        "deltas_vs_baseline": deltas,
    }
    Path(args.out).write_text(
        json.dumps(blob, ensure_ascii=False, indent=2), encoding="utf-8"
    )
    print(f"→ wrote {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
