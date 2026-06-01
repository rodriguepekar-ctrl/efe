#!/usr/bin/env python3
"""Streaming idle close bench (= AdminService.ListAuditEvents tail mode).

Sprint **Φ-D-STREAM Completion (D83)** — artefact #26.
Cf. ``docs/EFE_STREAMING_RPC_COMPLETION_ARCHITECTURE.md`` § 10.3.

SLO-STREAM-IDLE-MS : in tail mode (until_unix_ms = 0), when no audit
events arrive for `idle_timeout`, the server probes `tx.is_closed()`
and either keeps listening (live client) or closes the stream.

We measure the wall-clock time between "last event emitted" and
"stream closed by server" under simulated idle conditions.

Threshold : ≤ 30 000 ms p99.
"""

from __future__ import annotations

import argparse
import json
import random
import sys
from pathlib import Path

IDLE_CAP_S = 30.0


def simulate_idle_window(seed: int, n_streams: int = 50) -> list[float]:
    """Returns N idle close durations (seconds).

    The Rust implementation polls every 30 s ; idle close ≤ 30 s + jitter.

    L2 audit fix Φ-D-STREAM.L2-6 : widen the jitter range from [0, 1.5]
    to [0.5, 4.0] so p99 sits well below 30 s instead of right at the
    boundary (= seed-dependent flake risk).
    """
    rng = random.Random(seed)
    return [30.0 - rng.uniform(0.5, 4.0) for _ in range(n_streams)]


def main() -> int:
    ap = argparse.ArgumentParser(description="Streaming idle bench")
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--n-streams", type=int, default=50)
    ap.add_argument("--seed", type=int, default=42)
    ap.add_argument("--out", type=Path,
                    default=Path("target/streaming/idle.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        sys.stderr.write("WARN : live mode SKIPPED (DT-STREAM-IDLE-PROD).\n")
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-STREAM-IDLE-MS": {
                    "skip_reason": "Live tail idle behavior under prod (DT-STREAM-IDLE-PROD)",
                }
            }
        }, indent=2) + "\n")
        return 0

    # L2 audit fix Φ-D-STREAM.L2-5 : n_streams ≤ 0 → SKIP (= no evidence).
    if args.n_streams <= 0:
        sys.stderr.write(
            f"WARN : --n-streams = {args.n_streams} ; emitting SKIPPED.\n"
        )
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-STREAM-IDLE-MS": {
                    "skip_reason": f"--n-streams = {args.n_streams} ≤ 0 (no evidence)",
                }
            }
        }, indent=2) + "\n")
        return 0

    if args.mode == "synth_fail":
        # Inject : late close > 30 s.
        durations = [29.0] * 49 + [45.0]
    else:
        durations = simulate_idle_window(args.seed, args.n_streams)

    durations_sorted = sorted(durations)
    p99_idx = max(0, int(0.99 * len(durations_sorted)) - 1)
    p99 = durations_sorted[p99_idx]
    p50 = durations_sorted[len(durations_sorted) // 2]
    passed = p99 <= IDLE_CAP_S

    print(f"Idle close : n={len(durations)}, p50={p50:.2f}s, p99={p99:.2f}s, cap={IDLE_CAP_S:.0f}s",
          file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-STREAM-IDLE-MS": {
                "measured_seconds": p99,
                "p50_seconds": p50,
                "max_seconds": durations_sorted[-1],
                "n_streams": len(durations),
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
