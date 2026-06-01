#!/usr/bin/env python3
"""Verify Active Learning holdout split has 0 % training overlap.

Sprint **Φ-D-AL Completion (D76)** — artefact #11.
Cf. ``docs/EFE_ACTIVE_LEARNING_COMPLETION_ARCHITECTURE.md`` § 10.2.
"""

from __future__ import annotations
import argparse, hashlib, json, sys
from pathlib import Path


def fingerprint_label(label: dict) -> str:
    """Deterministic fingerprint of a label.

    Audit fix Φ-D-AL.A7 : pre-fix used concatenation `query_text + "|"
    + label_text + "|" + catalog_id` which was vulnerable to delimiter
    injection — a malicious `query_text = "a|b"` with `label_text = ""`
    collided with `query_text = "a"` + `label_text = "b"`. Post-fix :
    canonical JSON with sort_keys + no whitespace + key-included keys.
    The fingerprint is now uniquely tied to the structured fields,
    immune to injection.
    """
    import json
    canonical_obj = {
        "query_text": label.get("query_text", ""),
        "label_text": label.get("label_text", ""),
        "catalog_id": label.get("catalog_id", ""),
    }
    canonical_bytes = json.dumps(
        canonical_obj, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")
    return hashlib.sha256(canonical_bytes).hexdigest()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--training-set", type=Path,
                    help="JSON of training labels")
    ap.add_argument("--holdout-set", type=Path,
                    help="JSON of holdout labels")
    ap.add_argument("--out", type=Path,
                    default=Path("target/active_learning/holdout_integrity.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if not args.training_set or not args.holdout_set:
        sys.stderr.write("WARN : --training-set + --holdout-set required ; emitting SKIPPED.\n")
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-AL-HOLDOUT-SPLIT-INTEGRITY": {
                    "skip_reason": "Live training + holdout dumps required (DT-AL-HOLDOUT-DUMP)"
                }
            }
        }, indent=2) + "\n")
        return 0

    training = json.loads(args.training_set.read_text())
    holdout = json.loads(args.holdout_set.read_text())
    if isinstance(training, dict):
        training = training.get("labels", [])
    if isinstance(holdout, dict):
        holdout = holdout.get("labels", [])

    training_fps = {fingerprint_label(l) for l in training}
    holdout_fps = {fingerprint_label(l) for l in holdout}
    overlap = training_fps & holdout_fps
    passed = len(overlap) == 0

    print(f"Holdout integrity : training={len(training_fps)} holdout={len(holdout_fps)} "
          f"overlap={len(overlap)}", file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-AL-HOLDOUT-SPLIT-INTEGRITY": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "n_training": len(training_fps),
                "n_holdout": len(holdout_fps),
                "n_overlap": len(overlap),
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
