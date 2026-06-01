"""Embedder fine-tuning study — trains 5 methods, benchmarks each.

For each method we:

1. Take the degraded dataset (sibling-biased) — the "generic embedder" view.
2. Split its anchors 80/20 into train/val.
3. Fine-tune the head on the train split.
4. Re-embed the whole dataset (anchors AND queries) through the learned
   transform.
5. Run baseline + V1 swarm on the re-embedded dataset.
6. Compute metrics on easy/hard/all plus per-sibling-pair confusion.

The output blob goes to ``embedder_study_results.json`` and drives the PDF.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from dataclasses import asdict
from pathlib import Path
from typing import Any

import numpy as np

from benchmarks.baseline import NaiveCosineBaseline
from benchmarks.dataset import CLASS_NAMES
from benchmarks.dataset import _SIBLING
from benchmarks.embedders import (
    apply_transform_to_dataset,
    build_degraded_dataset,
    identity_transform,
)
from benchmarks.evaluate import evaluate
from benchmarks.finetune import (
    TrainSplit,
    TrainingReport,
    split_anchors,
    train_arcface,
    train_linear_probe,
    train_multitask,
    train_supcon_mlp,
    train_triplet_hnm,
)
from benchmarks.swarm_sim import SwarmClassifier


def _evaluate_pair(
    ds,
    name: str,
    train_report: TrainingReport | None,
) -> dict[str, Any]:
    baseline = NaiveCosineBaseline(ds)
    swarm = SwarmClassifier(ds)
    queries = ds.all_queries()

    # Warm up once.
    for q in queries[:3]:
        baseline.classify(q)
        swarm.classify(q)

    b_preds = [baseline.classify(q) for q in queries]
    s_preds = [swarm.classify(q) for q in queries]

    # Per-slice eval.
    slices: dict[str, Any] = {}
    qid_to_query = {q.qid: q for q in queries}
    easy_qids = {q.qid for q in ds.queries_easy}
    hard_qids = {q.qid for q in ds.queries_hard}
    for slice_name, pick in (
        ("easy", easy_qids),
        ("hard", hard_qids),
        ("all", {q.qid for q in queries}),
    ):
        b_slice = [p for p in b_preds if p.qid in pick]
        s_slice = [p for p in s_preds if p.qid in pick]
        q_slice = [qid_to_query[p.qid] for p in b_slice]
        slices[slice_name] = {
            "baseline": evaluate(
                name=f"{name}+baseline",
                slice_label=slice_name,
                queries=q_slice,
                predictions=b_slice,
            ).to_dict(),
            "swarm": evaluate(
                name=f"{name}+swarm",
                slice_label=slice_name,
                queries=q_slice,
                predictions=s_slice,
            ).to_dict(),
        }

    # Per-sibling-pair breakdown on HARD queries — the sibling pairs
    # are the ones we wanted fine-tuning to help on.
    sibling_pairs = set()
    for cls in CLASS_NAMES:
        sib = _SIBLING[cls]
        sibling_pairs.add(frozenset({cls, sib}))
    pair_stats: dict[str, dict[str, float]] = {}
    for pair in sibling_pairs:
        a, b = sorted(pair)
        pair_name = f"{a}↔{b}"
        picks = [
            (q, bp, sp)
            for q, bp, sp in zip(queries, b_preds, s_preds)
            if q.difficulty == "hard" and q.label in pair
        ]
        if not picks:
            continue
        n = len(picks)
        b_corr = sum(1 for _, bp, _ in picks if bp.predicted in pair)
        s_corr = sum(1 for _, _, sp in picks if sp.predicted in pair)
        # More discriminant: "correct within pair"
        b_right = sum(1 for q, bp, _ in picks if bp.predicted == q.label)
        s_right = sum(1 for q, _, sp in picks if sp.predicted == q.label)
        pair_stats[pair_name] = {
            "n": n,
            "baseline_acc": b_right / n,
            "swarm_acc": s_right / n,
            # How often we land in the right pair (even if on the wrong
            # side of it) — this filters outer errors.
            "baseline_in_pair": b_corr / n,
            "swarm_in_pair": s_corr / n,
        }

    # Inter-class silhouette on anchors — measures separation.
    silhouette = _silhouette_score(ds)

    result: dict[str, Any] = {
        "name": name,
        "slices": slices,
        "sibling_pair_stats": pair_stats,
        "silhouette": silhouette,
    }
    if train_report is not None:
        result["training"] = {
            **asdict(train_report),
            "history": [dict(h) for h in train_report.history],
        }
    return result


def _silhouette_score(ds) -> float:
    """Mean silhouette of anchor embeddings — cosine distance."""
    labels = []
    X = []
    for c in CLASS_NAMES:
        for v in ds.anchors[c]:
            labels.append(c)
            X.append(v)
    X_arr = np.stack(X)
    # L2-normalised → cosine distance = 0.5*(1 - dot).
    # Compute intra and nearest-other-cluster distances per point.
    n = X_arr.shape[0]
    sim = X_arr @ X_arr.T
    dist = 1.0 - sim
    labels_arr = np.array(labels)
    scores = []
    for i in range(n):
        own = labels_arr == labels_arr[i]
        own[i] = False
        if not own.any():
            continue
        a = dist[i, own].mean()
        bs = []
        for c in set(labels):
            if c == labels_arr[i]:
                continue
            mask = labels_arr == c
            bs.append(dist[i, mask].mean())
        b = min(bs) if bs else 0.0
        if max(a, b) > 0:
            scores.append((b - a) / max(a, b))
    return float(np.mean(scores)) if scores else 0.0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", default="benchmarks/embedder_study_results.json")
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--sibling-bias-tau", type=float, default=0.55)
    parser.add_argument("--n-easy", type=int, default=250)
    parser.add_argument("--n-hard", type=int, default=250)
    parser.add_argument("--epochs", type=int, default=200)
    parser.add_argument(
        "--include-easy-queries",
        action="store_true",
        help="Include the 250 easy queries in the training set — models "
        "the realistic 'we have more labelled data than anchors' scenario.",
    )
    args = parser.parse_args(argv)

    print("→ building degraded dataset …", flush=True)
    ds = build_degraded_dataset(
        seed=args.seed,
        n_easy=args.n_easy,
        n_hard=args.n_hard,
        sibling_bias_tau=args.sibling_bias_tau,
    )

    split = split_anchors(
        ds,
        val_frac=0.2,
        seed=args.seed,
        include_easy_queries=args.include_easy_queries,
    )
    print(
        f"  train: {len(split.X)} labeled samples "
        f"(easy_queries_included={args.include_easy_queries}), "
        f"val: {len(split.X_val)} anchors ({len(split.class_names)} classes)",
        flush=True,
    )

    # 0. Baseline: degraded embedder, identity transform (no fine-tuning).
    print("→ measuring baseline (no fine-tuning) …", flush=True)
    variants: dict[str, Any] = {}
    variants["identity"] = _evaluate_pair(ds, "identity", None)

    # 1..5. Train each fine-tuning method.
    methods = [
        ("linear_probe", lambda: train_linear_probe(split)),
        ("supcon_mlp", lambda: train_supcon_mlp(split, epochs=args.epochs, seed=args.seed)),
        ("triplet_hnm", lambda: train_triplet_hnm(split, epochs=args.epochs, seed=args.seed)),
        ("arcface", lambda: train_arcface(split, epochs=args.epochs, seed=args.seed)),
        ("multitask", lambda: train_multitask(split, epochs=args.epochs, seed=args.seed)),
    ]
    for name, trainer in methods:
        print(f"→ training {name} …", flush=True)
        transform, report = trainer()
        print(
            f"  {name}: {report.train_seconds:.2f}s, "
            f"{report.n_params:,} params, val_acc={report.final_val_acc:.3f}",
            flush=True,
        )
        ds_adapted = apply_transform_to_dataset(ds, transform)
        variants[name] = _evaluate_pair(ds_adapted, name, report)

    # Dump.
    blob = {
        "config": {
            "seed": args.seed,
            "sibling_bias_tau": args.sibling_bias_tau,
            "n_easy": args.n_easy,
            "n_hard": args.n_hard,
            "epochs": args.epochs,
            "n_train_samples": len(split.X),
            "n_val_samples": len(split.X_val),
            "include_easy_queries": args.include_easy_queries,
        },
        "variants": variants,
    }
    Path(args.out).write_text(
        json.dumps(blob, ensure_ascii=False, indent=2), encoding="utf-8"
    )
    print(f"→ wrote {args.out}")

    # On-screen summary.
    print()
    print("=" * 80)
    print("FINE-TUNING STUDY SUMMARY")
    print("=" * 80)
    print(
        f"{'variant':<18} {'base_acc':<10} {'swarm_acc':<10} "
        f"{'hard_base':<10} {'hard_swarm':<10} {'silhouette':<10}"
    )
    print("-" * 80)
    for name, res in variants.items():
        s_all = res["slices"]["all"]
        s_hard = res["slices"]["hard"]
        print(
            f"{name:<18} "
            f"{s_all['baseline']['accuracy']:<10.3f} "
            f"{s_all['swarm']['accuracy']:<10.3f} "
            f"{s_hard['baseline']['accuracy']:<10.3f} "
            f"{s_hard['swarm']['accuracy']:<10.3f} "
            f"{res['silhouette']:<10.3f}"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
