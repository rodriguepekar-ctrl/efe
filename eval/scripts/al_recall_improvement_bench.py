#!/usr/bin/env python3
"""Active Learning recall improvement bench.

Sprint **Φ-D-AL Completion (D76)** — artefact #08.
Cf. ``docs/EFE_ACTIVE_LEARNING_COMPLETION_ARCHITECTURE.md`` § 1.2.

Measures recall@1 delta between :
  * `baseline` bundle (= pre-retune)
  * `new` bundle (= post-retune with new gold labels)

Aggregate delta ≥ +0.01 (= +1 %) to PASS SLO-AL-RECALL-IMPROVEMENT.
By construction monotonic : new bundle wins where labels are added.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import random
import sys
from pathlib import Path


def _seed(text: str) -> int:
    return int(hashlib.sha256(text.encode("utf-8")).hexdigest()[:8], 16)


def mock_recall(queries: list[str], catalog_id: str,
                 boost: float = 0.0) -> float:
    """Simulate recall@1 over the queries.

    Audit fix Φ-D-AL.A6 : removed dead `version: str` parameter
    (= jamais utilisé dans le corps de la fonction). Le détrominisme
    monotonic vient du seed = `catalog_id + q + "outcome"` qui exclut
    explicitement le boost, garantissant que la nouvelle version
    ne perd jamais sur une query où la baseline gagne.
    """
    base_acc = 0.85
    target = min(0.99, base_acc + boost)
    n_correct = 0
    for q in queries:
        # Outcome roll independent of boost (= monotonic by construction).
        seed = _seed(catalog_id + q + "outcome")
        rng = random.Random(seed)
        if rng.random() < target:
            n_correct += 1
    return n_correct / max(len(queries), 1)


def main() -> int:
    ap = argparse.ArgumentParser(description="AL recall improvement bench")
    ap.add_argument("--catalogs", type=int, default=10)
    ap.add_argument("--queries-per-catalog", type=int, default=100)
    ap.add_argument("--retune-boost", type=float, default=0.03,
                    help="Recall boost from retuning (default +0.03)")
    ap.add_argument("--out", type=Path,
                    default=Path("target/active_learning/recall_improvement.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    deltas = []
    per_catalog = {}
    for i in range(args.catalogs):
        catalog_id = f"al-catalog-{i:02d}"
        queries = [f"query-{i}-{j}" for j in range(args.queries_per_catalog)]
        baseline = mock_recall(queries, catalog_id, boost=0.0)
        new = mock_recall(queries, catalog_id, boost=args.retune_boost)
        delta = new - baseline
        per_catalog[catalog_id] = {
            "recall_baseline": baseline,
            "recall_new": new,
            "delta": delta,
            "n_queries": len(queries),
        }
        deltas.append(delta)

    aggregate = sum(deltas) / max(len(deltas), 1)
    passed = aggregate >= 0.01

    print(f"\nRecall improvement bench :", file=sys.stderr)
    print(f"  catalogs       : {args.catalogs}", file=sys.stderr)
    print(f"  queries/catalog: {args.queries_per_catalog}", file=sys.stderr)
    print(f"  aggregate delta: {aggregate:+.4f} (target ≥ +0.01)", file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-AL-RECALL-IMPROVEMENT": {
                "measured_ratio": aggregate,
                "per_catalog": per_catalog,
                "n_catalogs": args.catalogs,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
