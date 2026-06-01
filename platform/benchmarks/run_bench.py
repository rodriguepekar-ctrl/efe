"""Entry point: run the whole benchmark and dump a JSON result blob.

Usage:

    PYTHONPATH=. python benchmarks/run_bench.py --out results.json

Produces a JSON document of the form::

    {
      "dataset": {
        "seed": 42,
        "n_classes": 8,
        "class_names": [...],
        "n_anchors_per_class": 32,
        "n_queries_easy": 250,
        "n_queries_hard": 250,
        "embedding_dim": 128
      },
      "swarm_config": {...},
      "environment": {"python": "3.12.0", "numpy": "...", "cpu_count": 10},
      "results": {
        "baseline": {"easy": {...}, "hard": {...}, "all": {...}},
        "swarm":    {"easy": {...}, "hard": {...}, "all": {...}}
      },
      "per_query": [
        {"qid": "...", "label": "...", "difficulty": "...",
         "baseline_pred": "...", "baseline_conf": 0.73,
         "swarm_pred": "...", "swarm_conf": 0.89, "swarm_verdict": "CONVERGED"},
        ...
      ]
    }
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import sys
import time
from dataclasses import asdict
from pathlib import Path
from typing import Any

import numpy as np

from benchmarks.baseline import NaiveCosineBaseline
from benchmarks.dataset import CLASS_NAMES, Dataset, build_dataset
from benchmarks.evaluate import evaluate
from benchmarks.swarm_sim import SwarmClassifier, SwarmConfig


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Run the EFE comparative benchmark (naïve baseline vs. swarm)."
    )
    parser.add_argument("--out", default="benchmark_results.json")
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--n-easy", type=int, default=250)
    parser.add_argument("--n-hard", type=int, default=250)
    parser.add_argument(
        "--hard-noise-std",
        type=float,
        default=0.25,
        help="σ of the Gaussian noise added to hard queries.",
    )
    parser.add_argument(
        "--n-agents",
        type=int,
        default=1024,
        help="Swarm size (must match sum of default mix unless --custom-mix).",
    )
    args = parser.parse_args(argv)

    # 1. Dataset.
    print("→ building dataset …", flush=True)
    t0 = time.perf_counter()
    ds = build_dataset(
        seed=args.seed,
        n_easy=args.n_easy,
        n_hard=args.n_hard,
        hard_noise_std=args.hard_noise_std,
    )
    dt_build = time.perf_counter() - t0
    print(
        f"  done in {dt_build * 1000:.1f} ms — "
        f"{len(ds.queries_easy)} easy + {len(ds.queries_hard)} hard "
        f"queries, {len(ds.anchors)} classes × {list(ds.anchors.values())[0].shape[0]} anchors",
        flush=True,
    )

    # 2. Classifiers.
    baseline = NaiveCosineBaseline(ds)
    swarm_cfg = SwarmConfig(
        n_agents=args.n_agents,
        mix={
            "na-sem": args.n_agents * 512 // 1024,
            "na-log": args.n_agents * 384 // 1024,
            "na-crit": args.n_agents - (args.n_agents * 512 // 1024) - (args.n_agents * 384 // 1024),
        },
    )
    swarm = SwarmClassifier(ds, swarm_cfg)

    # 3. Warm-up (avoid JIT/page-fault noise in p99).
    print("→ warm-up …", flush=True)
    for q in ds.queries_easy[:5]:
        baseline.classify(q)
        swarm.classify(q)

    # 4. Inference — sequential, measured per-query latency.
    all_queries = ds.all_queries()

    print(f"→ running baseline on {len(all_queries)} queries …", flush=True)
    t0 = time.perf_counter()
    baseline_preds = [baseline.classify(q) for q in all_queries]
    dt_baseline = time.perf_counter() - t0
    print(f"  {len(all_queries) / dt_baseline:.0f} q/s wall-clock", flush=True)

    print(f"→ running swarm on {len(all_queries)} queries …", flush=True)
    t0 = time.perf_counter()
    swarm_preds = [swarm.classify(q) for q in all_queries]
    dt_swarm = time.perf_counter() - t0
    print(f"  {len(all_queries) / dt_swarm:.0f} q/s wall-clock", flush=True)

    # 5. Evaluate on three slices.
    qid_to_query = {q.qid: q for q in all_queries}
    easy_qids = {q.qid for q in ds.queries_easy}
    hard_qids = {q.qid for q in ds.queries_hard}

    def _slice(preds, picks):  # type: ignore[no-untyped-def]
        return [p for p in preds if p.qid in picks]

    results: dict[str, dict[str, Any]] = {"baseline": {}, "swarm": {}}
    for slice_name, pick in [
        ("easy", easy_qids),
        ("hard", hard_qids),
        ("all", {q.qid for q in all_queries}),
    ]:
        b_slice = _slice(baseline_preds, pick)
        s_slice = _slice(swarm_preds, pick)
        q_slice = [qid_to_query[p.qid] for p in b_slice]

        # Extras for the swarm: verdict distribution + dissent.
        verdicts: dict[str, int] = {}
        dissent_vals = []
        rounds_vals = []
        for s in s_slice:
            verdicts[s.verdict] = verdicts.get(s.verdict, 0) + 1
            dissent_vals.append(s.critique_dissent)
            rounds_vals.append(s.rounds_executed)
        swarm_extras = {
            "mean_critique_dissent": float(np.mean(dissent_vals)) if dissent_vals else 0.0,
            "mean_rounds_executed": float(np.mean(rounds_vals)) if rounds_vals else 0.0,
            **{f"verdict_{k.lower()}_count": float(v) for k, v in verdicts.items()},
        }

        results["baseline"][slice_name] = evaluate(
            name="baseline",
            slice_label=slice_name,
            queries=q_slice,
            predictions=b_slice,
        ).to_dict()
        results["swarm"][slice_name] = evaluate(
            name="swarm",
            slice_label=slice_name,
            queries=q_slice,
            predictions=s_slice,
            extras=swarm_extras,
        ).to_dict()

    # 6. Per-query dump (compact, for charts).
    per_query = []
    b_by_qid = {p.qid: p for p in baseline_preds}
    s_by_qid = {p.qid: p for p in swarm_preds}
    for q in all_queries:
        b = b_by_qid[q.qid]
        s = s_by_qid[q.qid]
        per_query.append(
            {
                "qid": q.qid,
                "label": q.label,
                "difficulty": q.difficulty,
                "confusion_sibling": q.confusion_sibling,
                "baseline": {
                    "pred": b.predicted,
                    "conf": round(b.confidence, 4),
                    "latency_us": round(b.latency_us, 2),
                    "correct": b.predicted == q.label,
                },
                "swarm": {
                    "pred": s.predicted,
                    "conf": round(s.confidence, 4),
                    "verdict": s.verdict,
                    "rounds": s.rounds_executed,
                    "critique_dissent": round(s.critique_dissent, 4),
                    "latency_us": round(s.latency_us, 2),
                    "correct": s.predicted == q.label,
                },
            }
        )

    # 7. Environment capture — reproducibility evidence.
    env = {
        "python": sys.version.split()[0],
        "implementation": platform.python_implementation(),
        "system": platform.system(),
        "machine": platform.machine(),
        "cpu_count": os.cpu_count(),
        "numpy": np.__version__,
    }

    blob = {
        "dataset": {
            "seed": ds.seed,
            "n_classes": len(ds.anchors),
            "class_names": list(CLASS_NAMES),
            "n_anchors_per_class": list(ds.anchors.values())[0].shape[0],
            "n_queries_easy": len(ds.queries_easy),
            "n_queries_hard": len(ds.queries_hard),
            "embedding_dim": list(ds.anchors.values())[0].shape[1],
            "hard_noise_std": args.hard_noise_std,
        },
        "swarm_config": _swarm_config_to_dict(swarm_cfg),
        "environment": env,
        "wall_clock_seconds": {
            "dataset_build": round(dt_build, 4),
            "baseline_total": round(dt_baseline, 4),
            "swarm_total": round(dt_swarm, 4),
        },
        "results": results,
        "per_query": per_query,
    }

    out_path = Path(args.out)
    out_path.write_text(json.dumps(blob, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"→ wrote {out_path} ({out_path.stat().st_size / 1024:.1f} KiB)", flush=True)

    # 8. Small on-screen summary so the human can sanity-check.
    _print_summary(blob)
    return 0


def _swarm_config_to_dict(cfg: SwarmConfig) -> dict[str, Any]:
    return {
        "n_agents": cfg.n_agents,
        "mix": dict(cfg.mix),
        "max_rounds": cfg.max_rounds,
        "epsilon_variance": cfg.epsilon_variance,
        "epsilon_margin": cfg.epsilon_margin,
        "stability_window": cfg.stability_window,
        "dropout": dict(cfg.dropout),
        "weight": dict(cfg.weight),
    }


def _print_summary(blob: dict[str, Any]) -> None:
    print()
    print("=" * 72)
    print("SUMMARY")
    print("=" * 72)
    for name in ("baseline", "swarm"):
        r = blob["results"][name]
        print(
            f"  {name:<9} easy: acc={r['easy']['accuracy']:.3f} "
            f"f1={r['easy']['macro_f1']:.3f} p50={r['easy']['latency_us']['p50']:.1f}µs"
        )
        print(
            f"  {name:<9} hard: acc={r['hard']['accuracy']:.3f} "
            f"f1={r['hard']['macro_f1']:.3f} p50={r['hard']['latency_us']['p50']:.1f}µs"
        )
        print(
            f"  {name:<9} all : acc={r['all']['accuracy']:.3f} "
            f"f1={r['all']['macro_f1']:.3f} ece={r['all']['ece']:.3f}"
        )
        print()


if __name__ == "__main__":
    sys.exit(main())
