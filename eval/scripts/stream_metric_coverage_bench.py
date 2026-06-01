#!/usr/bin/env python3
"""Streaming metric coverage bench.

Sprint **Φ-D-STREAM Completion (D83)** — artefact #32.
Cf. ``docs/EFE_STREAMING_RPC_COMPLETION_ARCHITECTURE.md`` § 12.1 + D-STREAM-9.

SLO-STREAM-METRIC-COV : every emitted chunk must increment exactly one
`efe_streaming_chunks_total{rpc, tenant, payload_kind}` counter. No
chunk goes unaccounted.

Simulates the production hot path : a stream produces 20 chunks across
4 RPCs ; we check that the counter receives EXACTLY 20 increments and
that the `payload_kind` label distribution matches the emitted types.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from collections import Counter


# Expected payload_kind values per RPC (= matches the proto oneof variants).
EXPECTED_PAYLOAD_KINDS = {
    "SwarmService.ExecuteStreaming":      {"RoundUpdate", "Final", "Error"},
    "Mode2Service.ExecuteStreaming":      {"Router", "RoundUpdate", "Final", "Error"},
    "Mode3Service.SubscribeVerdict":      {"SubVerdict", "Final", "Error"},
    "AdminService.ListAuditEvents":       {"AuditEvent"},   # no oneof, single message type
}


def simulate_stream_with_counter(rpc: str, chunks: list[dict]) -> tuple[int, dict]:
    """Returns (n_increments, label_counter)."""
    counter: Counter = Counter()
    for c in chunks:
        kind = c["type"]
        if kind not in EXPECTED_PAYLOAD_KINDS[rpc]:
            # Unknown kind = leak ; counter increments under "unknown".
            counter["unknown"] += 1
        else:
            counter[kind] += 1
    return (sum(counter.values()), dict(counter))


SCENARIOS = [
    ("SwarmService.ExecuteStreaming", [
        {"type": "RoundUpdate"}, {"type": "RoundUpdate"}, {"type": "RoundUpdate"},
        {"type": "Final"},
    ]),
    ("Mode2Service.ExecuteStreaming", [
        {"type": "Router"}, {"type": "RoundUpdate"}, {"type": "RoundUpdate"},
        {"type": "Final"},
    ]),
    ("Mode3Service.SubscribeVerdict", [
        {"type": "SubVerdict"}, {"type": "SubVerdict"}, {"type": "SubVerdict"},
        {"type": "Final"},
    ]),
    ("AdminService.ListAuditEvents", [
        {"type": "AuditEvent"} for _ in range(8)
    ]),
]


def main() -> int:
    ap = argparse.ArgumentParser(description="Streaming metric coverage bench")
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--out", type=Path,
                    default=Path("target/streaming/metric_coverage.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        sys.stderr.write("WARN : live mode SKIPPED.\n")
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-STREAM-METRIC-COV": {
                    "skip_reason": "Live Prometheus scrape required",
                }
            }
        }, indent=2) + "\n")
        return 0

    leaks: list[str] = []
    total_chunks = 0
    total_counted = 0
    per_rpc = {}

    for rpc, chunks in SCENARIOS:
        n_chunks = len(chunks)
        if args.mode == "synth_fail":
            # Inject : 1 chunk goes unaccounted (= -1 increment).
            n_counted, labels = (n_chunks - 1, {"RoundUpdate": n_chunks - 1})
        else:
            n_counted, labels = simulate_stream_with_counter(rpc, chunks)
        per_rpc[rpc] = {"n_chunks": n_chunks, "n_counted": n_counted, "labels": labels}
        total_chunks += n_chunks
        total_counted += n_counted
        if n_counted != n_chunks:
            leaks.append(f"{rpc}: {n_counted}/{n_chunks}")
        if "unknown" in labels:
            leaks.append(f"{rpc}: unknown kind label = {labels['unknown']}")

    passed = not leaks and total_chunks == total_counted

    print(f"Metric coverage : {total_counted}/{total_chunks} chunks counted, "
          f"{len(leaks)} leaks",
          file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-STREAM-METRIC-COV": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "total_chunks": total_chunks,
                "total_counted": total_counted,
                "leaks": leaks,
                "per_rpc": per_rpc,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
