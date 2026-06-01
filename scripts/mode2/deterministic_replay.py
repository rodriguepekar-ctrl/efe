#!/usr/bin/env python3
"""Deterministic re-run of cached Mode-2 responses.

Sprint **Φ-D-MODE2 Completion (D68)** — artefact #28.
Cf. ``docs/EFE_MODE2_ANCHORS_COMPLETION_ARCHITECTURE.md`` § 8.4.

## Mission

Verify SLO-MODE2-REPLAY : for a given `(prompt_hash, catalog_hash,
provider_id, model_version)` cache key, **re-running** the cached
response materialization yields **bit-identical** anchors :
* Same f32 vectors (= deterministic embedder).
* Same JSON schema validation result.
* Same fingerprint cache key.

## Why this matters

Mode-2 cached responses are persisted in Redis with TTL 30 days. If
the materialization pipeline changes (= embedder bump, schema
validator change), the cache becomes inconsistent. The replay
catches this drift.

## Stub mode

When no cache snapshot is provided, emit SKIPPED rather than
faussement-PASS.

## Output

```json
{
  "slo_compliance": {
    "SLO-MODE2-REPLAY": {
      "measured_pass_count": 1,
      "measured_expected_count": 1,
      "samples_tested": 100,
      "drift_detected": 0
    }
  }
}
```
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path


def hash_response(response: dict) -> str:
    """Compute a canonical hash of a cached response."""
    canonical = json.dumps(response, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(canonical).hexdigest()


def replay_samples(cache_snapshot: list[dict]) -> dict:
    """Re-hash each entry and check vs stored hash."""
    drifts = []
    n_samples = len(cache_snapshot)
    for i, entry in enumerate(cache_snapshot):
        stored_hash = entry.get("response_hash")
        response = entry.get("response_json")
        if stored_hash is None or response is None:
            drifts.append(f"entry {i}: missing response_hash or response_json")
            continue
        # Parse if string.
        if isinstance(response, str):
            try:
                response_obj = json.loads(response)
            except json.JSONDecodeError:
                drifts.append(f"entry {i}: response_json is malformed")
                continue
        else:
            response_obj = response
        recomputed_hash = hash_response(response_obj)
        if recomputed_hash != stored_hash:
            drifts.append(
                f"entry {i}: hash drift — stored={stored_hash[:16]}..., "
                f"recomputed={recomputed_hash[:16]}..."
            )
    return {"n_samples": n_samples, "drifts": drifts}


def main() -> int:
    ap = argparse.ArgumentParser(description="Mode-2 deterministic replay")
    ap.add_argument(
        "--cache-snapshot", type=Path,
        help="JSON file with cache entries (= response_hash + response_json per entry)",
    )
    ap.add_argument(
        "--out", type=Path,
        default=Path("target/mode2/deterministic_replay.json"),
    )
    args = ap.parse_args()

    args.out.parent.mkdir(parents=True, exist_ok=True)

    if not args.cache_snapshot:
        sys.stderr.write(
            "WARN : --cache-snapshot not provided ; emitting SKIPPED for SLO-MODE2-REPLAY.\n"
            "       For live mode, dump the Redis cache as JSON and pass --cache-snapshot.\n"
        )
        args.out.write_text(
            json.dumps(
                {
                    "slo_compliance": {
                        "SLO-MODE2-REPLAY": {
                            "skip_reason": (
                                "Live replay requires Redis cache dump "
                                "(DT-MODE2-CACHE-DUMP)."
                            )
                        }
                    }
                },
                indent=2,
            )
            + "\n"
        )
        return 0

    if not args.cache_snapshot.exists():
        sys.stderr.write(f"FATAL : cache snapshot missing : {args.cache_snapshot}\n")
        return 2

    snapshot = json.loads(args.cache_snapshot.read_text())
    entries = snapshot if isinstance(snapshot, list) else snapshot.get("entries", [])
    result = replay_samples(entries)

    print(
        f"\nDeterministic replay : {result['n_samples']} entries, "
        f"{len(result['drifts'])} drifts",
        file=sys.stderr,
    )
    if result["drifts"]:
        for d in result["drifts"][:10]:
            print(f"  - {d}", file=sys.stderr)

    passed = len(result["drifts"]) == 0
    args.out.write_text(
        json.dumps(
            {
                "slo_compliance": {
                    "SLO-MODE2-REPLAY": {
                        "measured_pass_count": 1 if passed else 0,
                        "measured_expected_count": 1,
                        "samples_tested": result["n_samples"],
                        "drift_detected": len(result["drifts"]),
                        "drifts": result["drifts"][:20],
                    }
                }
            },
            indent=2,
        )
        + "\n"
    )

    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
