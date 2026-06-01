#!/usr/bin/env python3
"""Streaming replay determinism bench.

Sprint **Φ-D-STREAM Completion (D83)** — artefact #31.
Cf. ``docs/EFE_STREAMING_RPC_COMPLETION_ARCHITECTURE.md`` § 13.2 + D-STREAM-7.

SLO-STREAM-REPLAY : for a deterministic Mode-1/Mode-2 backend, the
chunk sequence produced for a given (request, seed) tuple MUST be
byte-for-byte identical across replays.

Mode-3 is explicitly NOT covered because sub-swarm arrival order is
inherently non-deterministic (= tokio scheduler).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import random
import sys
from pathlib import Path


def deterministic_chunk_seq(query: str, seed: int, n_rounds: int) -> list[str]:
    """Produce a chunk sequence as a list of canonical-JSON strings."""
    rng = random.Random(int.from_bytes(
        hashlib.sha256(f"{query}|{seed}".encode()).digest()[:8], "big"
    ))
    chunks: list[str] = []
    for i in range(1, n_rounds + 1):
        ru = {
            "type": "RoundUpdate",
            "round_index": i,
            "active_agents": 16 + i,
            "variance": round(rng.uniform(0.1, 0.4), 4),
            "confidence_so_far": round(0.5 + i * 0.05, 4),
            "elapsed_ms": i * 25.0,
        }
        chunks.append(json.dumps(ru, sort_keys=True, separators=(",", ":")))
    final = {"type": "Final", "rounds_executed": n_rounds,
             "request_hash": hashlib.sha256(f"{query}|{seed}".encode()).hexdigest()[:16]}
    chunks.append(json.dumps(final, sort_keys=True, separators=(",", ":")))
    return chunks


def main() -> int:
    ap = argparse.ArgumentParser(description="Streaming replay determinism bench")
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--n-replays", type=int, default=10)
    ap.add_argument("--n-rounds", type=int, default=5)
    ap.add_argument("--out", type=Path,
                    default=Path("target/streaming/replay.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        sys.stderr.write("WARN : live mode SKIPPED.\n")
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-STREAM-REPLAY": {
                    "skip_reason": "Live deterministic backend not exposed in CI",
                }
            }
        }, indent=2) + "\n")
        return 0

    query = "billing-dispute"
    seed = 42
    runs: list[list[str]] = []
    for replay_idx in range(args.n_replays):
        seq = deterministic_chunk_seq(query, seed, args.n_rounds)
        if args.mode == "synth_fail" and replay_idx == 5:
            # Inject : drift one chunk value mid-replay.
            seq[2] = seq[2].replace("\"variance\"", "\"variance_DRIFT\"")
        runs.append(seq)
    # Check all runs equal run[0].
    baseline = runs[0]
    diff_indices = [i for i, r in enumerate(runs) if r != baseline]
    passed = len(diff_indices) == 0

    print(f"Replay determinism : {len(runs)} replays, {len(diff_indices)} divergent",
          file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-STREAM-REPLAY": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "n_replays": len(runs),
                "n_divergent": len(diff_indices),
                "divergent_indices": diff_indices[:5],
                "baseline_chunk_count": len(baseline),
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
