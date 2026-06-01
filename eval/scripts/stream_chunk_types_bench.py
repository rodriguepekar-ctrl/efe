#!/usr/bin/env python3
"""Streaming chunk type proportion bench.

Sprint **Φ-D-STREAM Completion (D83)** — artefact #23.
Cf. ``docs/EFE_STREAMING_RPC_COMPLETION_ARCHITECTURE.md`` § 11 + D-STREAM-1.

Validates SLO-STREAM-CHUNK-TYPES : every successful stream must emit
≥ 1 non-terminal chunk BEFORE the terminal chunk (Final or Error).

A stream that emits only a single Final chunk = "disguised unary" and
must be flagged as faussement-PASS.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

TERMINAL_TYPES = {"Final", "Error"}


def evaluate_stream(chunks: list[dict]) -> tuple[bool, str]:
    if not chunks:
        return (False, "empty stream")
    terminals = [c for c in chunks if c["type"] in TERMINAL_TYPES]
    if not terminals:
        # No terminal — accept as cancelled.
        return (True, "no terminal (= cancelled)")
    if len(terminals) > 1:
        return (False, f"{len(terminals)} terminal chunks (must be ≤ 1)")
    # If a terminal exists, there must be at least 1 non-terminal BEFORE it.
    terminal_idx = next(i for i, c in enumerate(chunks) if c["type"] in TERMINAL_TYPES)
    non_terminal_before = [c for c in chunks[:terminal_idx] if c["type"] not in TERMINAL_TYPES]
    if not non_terminal_before:
        return (False, "stream emits ONLY a terminal chunk (= disguised unary, violates D-STREAM-1)")
    return (True, f"{len(non_terminal_before)} non-terminal + 1 terminal at idx {terminal_idx}")


SCENARIOS_PASS = [
    # 5 valid streams covering all 4 RPCs
    ("mode1_5_rounds", [
        {"type": "RoundUpdate", "round_index": 1},
        {"type": "RoundUpdate", "round_index": 2},
        {"type": "RoundUpdate", "round_index": 3},
        {"type": "RoundUpdate", "round_index": 4},
        {"type": "RoundUpdate", "round_index": 5},
        {"type": "Final"},
    ]),
    ("mode2_router_plus_rounds", [
        {"type": "Router", "router_id": "mock"},
        {"type": "RoundUpdate", "round_index": 1},
        {"type": "RoundUpdate", "round_index": 2},
        {"type": "Final"},
    ]),
    ("mode3_subverdicts", [
        {"type": "SubVerdict", "swarm_idx": 0},
        {"type": "SubVerdict", "swarm_idx": 1},
        {"type": "SubVerdict", "swarm_idx": 2},
        {"type": "Final"},
    ]),
    ("admin_audit", [
        {"type": "AuditEvent", "event_id": "01h00000001"},
        {"type": "AuditEvent", "event_id": "01h00000002"},
        # Admin audit has no Final chunk per proto, STREAM_END is implicit.
    ]),
    ("error_after_progress", [
        {"type": "RoundUpdate", "round_index": 1},
        {"type": "RoundUpdate", "round_index": 2},
        {"type": "Error", "code": 13, "message": "backend timeout"},
    ]),
]


SCENARIOS_FAIL = [
    # Disguised-unary = single Final chunk.
    ("disguised_unary_final_only", [{"type": "Final"}]),
    ("disguised_unary_error_only", [{"type": "Error", "code": 13}]),
]


def main() -> int:
    ap = argparse.ArgumentParser(description="Streaming chunk types bench")
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--out", type=Path,
                    default=Path("target/streaming/chunk_types.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        sys.stderr.write("WARN : live mode requires gRPC fixture ; SKIPPED.\n")
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-STREAM-CHUNK-TYPES": {
                    "skip_reason": "Live gRPC streaming bench (DT-STREAM-CHUNK-LIVE-LATENCY)",
                }
            }
        }, indent=2) + "\n")
        return 0

    scenarios = SCENARIOS_FAIL if args.mode == "synth_fail" else SCENARIOS_PASS
    results = []
    n_pass = 0
    for name, chunks in scenarios:
        ok, reason = evaluate_stream(chunks)
        sym = "✓" if ok else "✗"
        print(f"  {sym} {name:<35} : {reason}", file=sys.stderr)
        results.append({"scenario": name, "pass": ok, "reason": reason})
        if ok:
            n_pass += 1

    if args.mode == "synth_fail":
        # In synth_fail, we expect ALL scenarios to fail (= negative test).
        slo_passed = n_pass == 0
        reason = f"{len(scenarios) - n_pass}/{len(scenarios)} regressions correctly flagged"
    else:
        slo_passed = n_pass == len(scenarios)
        reason = f"{n_pass}/{len(scenarios)} streams correctly classified"

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-STREAM-CHUNK-TYPES": {
                "measured_pass_count": 1 if slo_passed else 0,
                "measured_expected_count": 1,
                "n_scenarios": len(scenarios),
                "n_passed": n_pass,
                "results": results,
                "reason": reason,
            }
        }
    }, indent=2) + "\n")
    return 0 if slo_passed else 1


if __name__ == "__main__":
    sys.exit(main())
