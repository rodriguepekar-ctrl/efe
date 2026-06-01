#!/usr/bin/env python3
"""Track Mode-3 human review SLA per stage.

Sprint **Φ-D-MODE3 Completion (D73)** — artefact #27.
Cf. ``docs/EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md`` § 9.3.

For each escalated session, measure :
  * `pending → in_review` latency (p99 ≤ 30 s)
  * `in_review → decided` latency (p99 ≤ 5 min)
  * Total escalation → decided (p99 ≤ 5 min 30 s)

Stub mode emits SKIPPED when no review queue dump is provided.
"""

from __future__ import annotations

import argparse
import json
import statistics
import sys
from pathlib import Path

P99_TOTAL_THRESHOLD_S = 330.0  # 5 min 30 s


def percentile(values: list[float], q: float) -> float:
    if not values:
        return 0.0
    s = sorted(values)
    idx = max(0, min(len(s) - 1, int(q * len(s)) - 1))
    return s[idx]


def main() -> int:
    ap = argparse.ArgumentParser(description="Mode-3 review SLA tracker")
    ap.add_argument("--review-queue-dump", type=Path,
                    help="JSON of mode3_review_queue rows")
    ap.add_argument("--out", type=Path,
                    default=Path("target/mode3/review_sla.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if not args.review_queue_dump:
        sys.stderr.write(
            "WARN : --review-queue-dump required for live mode ; emitting SKIPPED.\n"
        )
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-MODE3-HUMAN-REVIEW-LATENCY-P99": {
                    "skip_reason": "Live review queue dump required (DT-MODE3-QUEUE-DUMP)"
                }
            }
        }, indent=2) + "\n")
        return 0

    rows = json.loads(args.review_queue_dump.read_text())
    if isinstance(rows, dict):
        rows = rows.get("rows", [])

    total_latencies = []
    pending_latencies = []
    review_latencies = []
    for r in rows:
        esc_ts = r.get("escalation_timestamp", 0)
        rev_ts = r.get("review_timestamp", 0)
        if esc_ts and rev_ts:
            total_latencies.append((rev_ts - esc_ts) / 1e6)
            assignment = r.get("assignment_timestamp", esc_ts)
            pending_latencies.append((assignment - esc_ts) / 1e6)
            review_latencies.append((rev_ts - assignment) / 1e6)

    if not total_latencies:
        print("No review events found.", file=sys.stderr)
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-MODE3-HUMAN-REVIEW-LATENCY-P99": {
                    "skip_reason": "No review events in dump (= empty queue)"
                }
            }
        }, indent=2) + "\n")
        return 0

    p99_total = percentile(total_latencies, 0.99)
    passed = p99_total <= P99_TOTAL_THRESHOLD_S

    print(f"Review SLA (n={len(total_latencies)}) :", file=sys.stderr)
    print(f"  pending p99    : {percentile(pending_latencies, 0.99):.1f} s", file=sys.stderr)
    print(f"  review  p99    : {percentile(review_latencies, 0.99):.1f} s", file=sys.stderr)
    print(f"  total   p99    : {p99_total:.1f} s (threshold {P99_TOTAL_THRESHOLD_S})", file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-MODE3-HUMAN-REVIEW-LATENCY-P99": {
                "measured_seconds": p99_total,
                "n_reviews": len(total_latencies),
                "pending_p99_s": percentile(pending_latencies, 0.99),
                "review_p99_s": percentile(review_latencies, 0.99),
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
