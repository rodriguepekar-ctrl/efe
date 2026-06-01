#!/usr/bin/env python3
"""Compliance audit chain daily replay.

Sprint **Φ-D-COMPLIANCE D87** — aggregator #3.
Cf. ``docs/EFE_COMPLIANCE_COMPLETION_ARCHITECTURE.md`` § 6.3 + § 14.1.

Réutilise le pattern HMAC chain replay (= D14-bis baseline) appliqué à
la sous-table `audit_events WHERE action LIKE 'compliance.%'`.

Modes :
  * `synth_pass`   : 100 synthetic compliance events, chain valid → PASS.
  * `synth_fail`   : injects chain break at event 42 → must detect.
  * `live`         : real audit_events PG read (DT-COMP-LIVE-HMAC-ROTATION).

Emit SLO-COMP-AUDIT-CHAIN-DAILY.
"""

from __future__ import annotations

import argparse
import hashlib
import hmac
import json
import sys
from pathlib import Path

DEFAULT_KEY = b"\x00" * 32

COMPLIANCE_ACTIONS = [
    "compliance.consent_change_v2",
    "compliance.erasure_request",
    "compliance.erasure_complete",
    "compliance.breach_detected",
    "compliance.breach_dpo_notified",
    "compliance.dpia_generated",
    "compliance.dpia_signed_off",
    "compliance.ropa_rebuilt",
    "compliance.subprocessor_added",
    "compliance.ai_act_reclassified",
    "compliance.residency_violation",
]


def compute_sig(key: bytes, prev_sig_hex: str, payload_json: str) -> str:
    m = hmac.new(key, digestmod=hashlib.sha256)
    m.update(prev_sig_hex.encode("utf-8"))
    m.update(payload_json.encode("utf-8"))
    return m.hexdigest()


def gen_compliance_chain(n: int, key: bytes, break_at: int | None = None) -> list[dict]:
    out = []
    prev_sig = ""
    for i in range(n):
        action = COMPLIANCE_ACTIONS[i % len(COMPLIANCE_ACTIONS)]
        payload = json.dumps({
            "event_no": i,
            "tenant": f"tenant-{i % 5}",
            "action": action,
        }, sort_keys=True, separators=(",", ":"))
        sig = compute_sig(key, prev_sig, payload)
        out.append({
            "event_id": f"01h{i:08d}",
            "ts_unix_ms": 1700000000000 + i * 1000,
            "action": action,
            "tenant_id": f"tenant-{i % 5}",
            "principal": "system",
            "payload_json": payload,
            "signature_hex": sig,
            "prev_signature_hex": prev_sig,
        })
        if break_at is not None and i == break_at:
            prev_sig = "ff" * 32  # inject break for next event
        else:
            prev_sig = sig
    return out


def replay_chain(events: list[dict], key: bytes) -> dict:
    broken = []
    prev_sig = ""
    for i, ev in enumerate(events):
        prev = ev.get("prev_signature_hex", "")
        if prev != prev_sig:
            broken.append(f"index={i} event_id={ev.get('event_id')} prev mismatch")
        expected = compute_sig(key, prev, ev["payload_json"])
        if expected != ev["signature_hex"]:
            broken.append(f"index={i} event_id={ev.get('event_id')} sig mismatch")
        prev_sig = ev["signature_hex"]
    return {"chain_length": len(events), "broken_links": broken[:20],
            "total_broken": len(broken)}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--n-events", type=int, default=100)
    ap.add_argument("--out", type=Path,
                    default=Path("target/compliance/audit_chain_daily.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-COMP-AUDIT-CHAIN-DAILY": {
                    "skip_reason": "Live audit_events PG (DT-COMP-LIVE-HMAC-ROTATION)",
                }
            }
        }, indent=2) + "\n")
        return 0

    if args.n_events <= 0:
        sys.stderr.write(f"WARN : --n-events = {args.n_events} ; emitting SKIPPED.\n")
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-COMP-AUDIT-CHAIN-DAILY": {
                    "skip_reason": f"n_events = {args.n_events} ≤ 0",
                }
            }
        }, indent=2) + "\n")
        return 0

    key = DEFAULT_KEY
    break_at = 42 if args.mode == "synth_fail" else None
    chain = gen_compliance_chain(args.n_events, key, break_at=break_at)
    result = replay_chain(chain, key)
    if args.mode == "synth_fail":
        # We EXPECT broken chain → success is "did detect"
        passed = result["total_broken"] >= 1
        reason = (f"synth_fail : detected {result['total_broken']} broken links "
                  f"(negative test PASS)" if passed
                  else "synth_fail : chain break NOT detected (= regression)")
    else:
        passed = result["total_broken"] == 0
        reason = f"{result['chain_length']} compliance events, {result['total_broken']} broken"

    print(f"Audit chain daily : {reason}", file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-COMP-AUDIT-CHAIN-DAILY": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "chain_length": result["chain_length"],
                "total_broken": result["total_broken"],
                "broken_links": result["broken_links"],
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
