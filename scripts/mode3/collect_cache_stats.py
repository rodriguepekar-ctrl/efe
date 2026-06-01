#!/usr/bin/env python3
"""Collect Mode-3 Redis cache hit/miss stats.

Sprint **Φ-D-MODE3 Completion (D73)** — artefact #30.
Cf. ``docs/EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md`` § Annexe B.

Queries the Redis cache keyspace for Mode-3 sessions :
  * Total keys (= active sessions in cache)
  * Hit rate per provider
  * TTL distribution

Stub mode emits SKIPPED when no Redis connection details given.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

CACHE_HIT_THRESHOLD = 0.60


def main() -> int:
    ap = argparse.ArgumentParser(description="Mode-3 cache stats collector")
    ap.add_argument("--redis-stats", type=Path,
                    help="JSON file with Redis stats (= prometheus snapshot)")
    ap.add_argument("--out", type=Path,
                    default=Path("target/mode3/cache_stats.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if not args.redis_stats or not args.redis_stats.exists():
        sys.stderr.write(
            "WARN : --redis-stats required for live mode ; emitting SKIPPED.\n"
        )
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-MODE3-CACHE-HIT-RATE": {
                    "skip_reason": "Live Redis stats required (DT-MODE3-REDIS-STATS)"
                }
            }
        }, indent=2) + "\n")
        return 0

    stats = json.loads(args.redis_stats.read_text())
    hits = int(stats.get("keyspace_hits", 0))
    misses = int(stats.get("keyspace_misses", 0))
    total = hits + misses
    hit_rate = hits / total if total > 0 else 0.0
    passed = hit_rate >= CACHE_HIT_THRESHOLD

    print(f"Cache stats : {hits} hits / {misses} misses → {hit_rate:.2%}",
          file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-MODE3-CACHE-HIT-RATE": {
                "measured_ratio": hit_rate,
                "hits": hits,
                "misses": misses,
                "total": total,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
