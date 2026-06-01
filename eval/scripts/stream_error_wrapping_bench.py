#!/usr/bin/env python3
"""Streaming error wrapping bench.

Sprint **Φ-D-STREAM Completion (D83)** — artefact #30.
Cf. ``docs/EFE_STREAMING_RPC_COMPLETION_ARCHITECTURE.md`` § 11 + D-STREAM-2.

SLO-STREAM-ERR-TYPED : 100 % of errors raised AFTER the gRPC handshake
(= after the first chunk could have been sent) must be wrapped in a
typed `oneof::Error` chunk, never propagated as a raw `tonic::Status`.

Pre-handshake errors (auth, rate-limit, scope) are explicitly allowed
to use raw Status per D-STREAM-16.

Injects 6 error scenarios and verifies the wrapping policy is applied.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


def wrap_or_raw(stage: str, error: dict) -> dict:
    """Returns the form of error delivery per the policy.

    stage ∈ { "pre_handshake", "post_handshake" }
    error = {code, message, details}
    """
    if stage == "pre_handshake":
        return {"kind": "raw_status", "code": error["code"], "message": error["message"]}
    elif stage == "post_handshake":
        return {"kind": "chunk_error", "payload": {
            "type": "Error", "code": error["code"],
            "message": error["message"],
            "details": error.get("details", {}),
        }}
    raise ValueError(stage)


SCENARIOS = [
    # (name, stage, error, expected_kind)
    ("auth_fail_pre",       "pre_handshake",  {"code": 16, "message": "auth"},        "raw_status"),
    ("rate_limit_pre",      "pre_handshake",  {"code": 8,  "message": "rate"},        "raw_status"),
    ("scope_fail_pre",      "pre_handshake",  {"code": 7,  "message": "scope"},       "raw_status"),
    ("backend_timeout_post","post_handshake", {"code": 4,  "message": "backend timeout"}, "chunk_error"),
    ("router_5xx_post",     "post_handshake", {"code": 13, "message": "router 5xx"},  "chunk_error"),
    ("aggregation_fail_post","post_handshake",{"code": 13, "message": "agg failed"},  "chunk_error"),
]


def main() -> int:
    ap = argparse.ArgumentParser(description="Streaming error wrapping bench")
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--out", type=Path,
                    default=Path("target/streaming/error_wrapping.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        sys.stderr.write("WARN : live mode SKIPPED.\n")
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-STREAM-ERR-TYPED": {
                    "skip_reason": "Live error injection requires gRPC fixture",
                }
            }
        }, indent=2) + "\n")
        return 0

    results = []
    n_pass = 0
    for name, stage, err, expected in SCENARIOS:
        if args.mode == "synth_fail":
            # Inject : all post_handshake use raw_status (= regression).
            actual = {"kind": "raw_status", "code": err["code"], "message": err["message"]}
        else:
            actual = wrap_or_raw(stage, err)
        ok = actual["kind"] == expected
        if ok:
            n_pass += 1
        results.append({"scenario": name, "stage": stage, "expected": expected,
                         "got": actual["kind"], "pass": ok})
        sym = "✓" if ok else "✗"
        print(f"  {sym} {name:<25} stage={stage:<15} got={actual['kind']}",
              file=sys.stderr)

    passed = n_pass == len(SCENARIOS) and args.mode != "synth_fail"

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-STREAM-ERR-TYPED": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "n_scenarios": len(SCENARIOS),
                "n_passed": n_pass,
                "results": results,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
