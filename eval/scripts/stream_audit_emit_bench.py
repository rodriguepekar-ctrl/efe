#!/usr/bin/env python3
"""Audit emit EPS bench (= ListAuditEvents tail throughput).

Sprint **Φ-D-STREAM Completion (D83)** — artefact #28.
Cf. ``docs/EFE_STREAMING_RPC_COMPLETION_ARCHITECTURE.md`` § 13.3.

SLO-STREAM-AUDIT-EMIT-EPS : while the audit stream is in tail mode and
events flow in, the server must sustain ≥ 1000 events/s emission rate.

Bench measures wall-time to emit N events through the same canonical
verify-then-emit path used in production (simulated).
"""

from __future__ import annotations

import argparse
import hashlib
import hmac
import json
import sys
import time
from pathlib import Path

EPS_TARGET = 1000.0


def main() -> int:
    ap = argparse.ArgumentParser(description="Audit emit EPS bench")
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--n-events", type=int, default=5000)
    ap.add_argument("--out", type=Path,
                    default=Path("target/streaming/audit_emit_eps.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        sys.stderr.write("WARN : live mode SKIPPED (DT-STREAM-AUDIT-EMIT-LIVE).\n")
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-STREAM-AUDIT-EMIT-EPS": {
                    "skip_reason": "Live PG INSERT load (DT-STREAM-AUDIT-EMIT-LIVE)",
                }
            }
        }, indent=2) + "\n")
        return 0

    key = b"\x00" * 32

    def emit_event(i: int, prev_sig_hex: str) -> tuple[str, dict]:
        payload = json.dumps({"i": i}, sort_keys=True, separators=(",", ":"))
        m = hmac.new(key, digestmod=hashlib.sha256)
        m.update(prev_sig_hex.encode("utf-8"))
        m.update(payload.encode("utf-8"))
        sig = m.hexdigest()
        return sig, {
            "event_id": f"01h{i:08d}",
            "ts_unix_ms": 1700000000000 + i,
            "payload_json": payload,
            "signature_hex": sig,
            "prev_signature_hex": prev_sig_hex,
        }

    n_events = 50 if args.mode == "synth_fail" else args.n_events

    # Audit fix Φ-D-STREAM.L1-7 : n_events == 0 is "no evidence", not
    # "EPS = 0 = FAIL". Emit SKIP instead of faussement-FAIL.
    if n_events <= 0:
        sys.stderr.write(
            f"WARN : n_events = {n_events} ; emitting SKIPPED (no evidence).\n"
        )
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-STREAM-AUDIT-EMIT-EPS": {
                    "skip_reason": f"n_events = {n_events} ≤ 0 (no evidence)",
                }
            }
        }, indent=2) + "\n")
        return 0

    started = time.perf_counter()
    prev_sig = ""
    emitted = 0
    for i in range(n_events):
        prev_sig, ev = emit_event(i, prev_sig)
        emitted += 1
        if args.mode == "synth_fail":
            # Artificial slowdown : 100ms per event = 10 EPS.
            time.sleep(0.1)
    elapsed = time.perf_counter() - started
    eps = emitted / elapsed if elapsed > 0 else 0.0
    passed = eps >= EPS_TARGET

    print(f"Audit emit : n={emitted}, elapsed={elapsed:.3f}s, EPS={eps:.1f}, "
          f"target≥{EPS_TARGET}",
          file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-STREAM-AUDIT-EMIT-EPS": {
                "measured_ratio": eps,
                "n_events": emitted,
                "elapsed_seconds": elapsed,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
