"""Sweep the sibling-bias strength τ to find the fine-tuning crossover.

The hypothesis: for small τ the embedder is already near-optimal and
fine-tuning overfits; for large τ the embedder confuses siblings and
fine-tuning recovers accuracy.

We run identity + linear_probe + arcface + supcon_mlp on
``τ ∈ {0.0, 0.2, 0.4, 0.6, 0.8, 1.0, 1.2, 1.5}`` with fixed seed.
Training data: anchors + 250 easy queries (realistic client setup).

Output: ``embedder_sweep_results.json``.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

from benchmarks.baseline import NaiveCosineBaseline
from benchmarks.embedders import (
    apply_transform_to_dataset,
    build_degraded_dataset,
)
from benchmarks.finetune import (
    split_anchors,
    train_arcface,
    train_linear_probe,
    train_supcon_mlp,
)
from benchmarks.swarm_sim import SwarmClassifier


def _accuracy(ds, *, use_swarm: bool) -> dict[str, float]:
    clf = SwarmClassifier(ds) if use_swarm else NaiveCosineBaseline(ds)
    qs = ds.all_queries()
    preds = [clf.classify(q) for q in qs]
    hard = [(q, p) for q, p in zip(qs, preds) if q.difficulty == "hard"]
    easy = [(q, p) for q, p in zip(qs, preds) if q.difficulty == "easy"]
    return {
        "all": sum(1 for q, p in zip(qs, preds) if p.predicted == q.label)
        / len(qs),
        "hard": sum(1 for q, p in hard if p.predicted == q.label) / max(1, len(hard)),
        "easy": sum(1 for q, p in easy if p.predicted == q.label) / max(1, len(easy)),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", default="benchmarks/embedder_sweep_results.json")
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument(
        "--taus", nargs="*", type=float,
        default=[0.0, 0.2, 0.4, 0.6, 0.8, 1.0, 1.2, 1.5],
    )
    parser.add_argument("--epochs", type=int, default=200)
    args = parser.parse_args(argv)

    all_results: dict[str, Any] = {"taus": args.taus, "rows": []}

    methods = {
        "linear_probe": lambda split: train_linear_probe(split)[0],
        "arcface": lambda split: train_arcface(
            split, epochs=args.epochs, seed=args.seed
        )[0],
        "supcon_mlp": lambda split: train_supcon_mlp(
            split, epochs=args.epochs, seed=args.seed
        )[0],
    }

    for tau in args.taus:
        print(f"\n── τ = {tau:.2f} ──", flush=True)
        ds = build_degraded_dataset(
            seed=args.seed,
            sibling_bias_tau=tau,
        )
        split = split_anchors(
            ds, val_frac=0.2, seed=args.seed, include_easy_queries=True
        )
        row: dict[str, Any] = {"tau": tau, "variants": {}}

        # Identity.
        acc_b = _accuracy(ds, use_swarm=False)
        acc_s = _accuracy(ds, use_swarm=True)
        row["variants"]["identity"] = {
            "baseline": acc_b,
            "swarm": acc_s,
        }
        print(
            f"  identity       baseline acc={acc_b['all']:.3f} hard={acc_b['hard']:.3f} | "
            f"swarm acc={acc_s['all']:.3f} hard={acc_s['hard']:.3f}",
            flush=True,
        )

        # Each fine-tuning method.
        for name, trainer in methods.items():
            transform = trainer(split)
            ds_adapted = apply_transform_to_dataset(ds, transform)
            acc_b = _accuracy(ds_adapted, use_swarm=False)
            acc_s = _accuracy(ds_adapted, use_swarm=True)
            row["variants"][name] = {
                "baseline": acc_b,
                "swarm": acc_s,
            }
            print(
                f"  {name:<14} baseline acc={acc_b['all']:.3f} hard={acc_b['hard']:.3f} | "
                f"swarm acc={acc_s['all']:.3f} hard={acc_s['hard']:.3f}",
                flush=True,
            )
        all_results["rows"].append(row)

    Path(args.out).write_text(
        json.dumps(all_results, ensure_ascii=False, indent=2), encoding="utf-8"
    )
    print(f"\n→ wrote {args.out}")

    # Summary table — hard accuracy across τ × method.
    print()
    print("=" * 95)
    print("HARD accuracy sweep (swarm)")
    print("=" * 95)
    header = f"{'τ':<8} " + " ".join(
        f"{v:<14}" for v in ("identity", "linear_probe", "arcface", "supcon_mlp")
    )
    print(header)
    print("-" * 95)
    for row in all_results["rows"]:
        t = row["tau"]
        line = f"{t:<8.2f} "
        for name in ("identity", "linear_probe", "arcface", "supcon_mlp"):
            a = row["variants"][name]["swarm"]["hard"]
            line += f"{a:<14.3f}"
        print(line)
    return 0


if __name__ == "__main__":
    sys.exit(main())
