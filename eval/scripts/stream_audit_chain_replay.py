#!/usr/bin/env python3
"""ListAuditEvents HMAC chain replay tool.

Sprint **Φ-D-STREAM Completion (D83)** — artefact #24.
Cf. ``docs/EFE_STREAMING_RPC_COMPLETION_ARCHITECTURE.md`` § 8.

Standalone replay of an audit-event chain :
  * `--chain-file <jsonl>` : replay live dump
  * default               : self-test with 100 synthetic events

For each event, verifies :
  1. `prev_signature_hex == previous.signature_hex`
  2. `signature_hex == HMAC-SHA256(key, prev_sig || payload_json)`

SLO-STREAM-AUDIT-CHAIN : all integrity invariants hold.
"""

from __future__ import annotations

import argparse
import hashlib
import hmac
import json
import os
import sys
from pathlib import Path

DEFAULT_KEY = b"\x00" * 32


def compute_sig(key: bytes, prev_sig_hex: str, payload_json: str) -> str:
    m = hmac.new(key, digestmod=hashlib.sha256)
    m.update(prev_sig_hex.encode("utf-8"))
    m.update(payload_json.encode("utf-8"))
    return m.hexdigest()


def replay_chain(events: list[dict], key: bytes) -> dict:
    broken: list[str] = []
    prev_signature = ""
    for i, ev in enumerate(events):
        prev = ev.get("prev_signature_hex", "")
        if prev != prev_signature:
            broken.append(f"index={i} event_id={ev.get('event_id')} prev_signature_hex mismatch")
        payload = ev.get("payload_json", "")
        sig = ev.get("signature_hex", "")
        expected = compute_sig(key, prev, payload)
        if expected != sig:
            broken.append(f"index={i} event_id={ev.get('event_id')} signature_hex mismatch")
        prev_signature = sig
    return {
        "chain_length": len(events),
        "broken_links": broken[:20],
        "total_broken": len(broken),
    }


def gen_self_test_chain(n: int, key: bytes) -> list[dict]:
    """Synthetic chain. Same algorithm as stream_admin_audit_bench (DRY)."""
    out = []
    prev_sig = ""
    for i in range(n):
        payload = json.dumps({"event_no": i, "tenant": "tenant-acme", "action": "test"},
                              sort_keys=True, separators=(",", ":"))
        sig = compute_sig(key, prev_sig, payload)
        out.append({
            "event_id": f"01h{i:08d}",
            "ts_unix_ms": 1700000000000 + i * 1000,
            "tenant_id": "tenant-acme",
            "principal": "tester",
            "payload_json": payload,
            "signature_hex": sig,
            "prev_signature_hex": prev_sig,
        })
        prev_sig = sig
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description="Audit chain HMAC replay")
    ap.add_argument("--chain-file", type=Path,
                    help="JSONL dump of audit events to replay")
    ap.add_argument("--hmac-key-hex",
                    help="HMAC key in hex ; defaults to all-zero for self-test")
    ap.add_argument("--hmac-key-env", default="EFE_AUDIT_HMAC_KEY")
    ap.add_argument("--n-self-test", type=int, default=100,
                    help="Number of synthetic events to generate when no chain provided")
    ap.add_argument("--out", type=Path,
                    default=Path("target/streaming/audit_chain_replay.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    # Resolve key.
    if args.hmac_key_hex:
        key = bytes.fromhex(args.hmac_key_hex)
    else:
        env = os.environ.get(args.hmac_key_env)
        key = bytes.fromhex(env) if env else DEFAULT_KEY

    # L2 audit fix Φ-D-STREAM.L2-4 : n_self_test ≤ 0 → SKIP (no evidence).
    # Pre-fix : empty chain replayed → 0 broken_links → PASS vacuously.
    if not args.chain_file and args.n_self_test <= 0:
        sys.stderr.write(
            f"WARN : --n-self-test = {args.n_self_test} ; emitting SKIPPED.\n"
        )
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-STREAM-AUDIT-CHAIN": {
                    "skip_reason": (
                        f"--n-self-test = {args.n_self_test} ≤ 0 (no chain to replay)"
                    ),
                }
            }
        }, indent=2) + "\n")
        return 0

    # Resolve chain (= live or self-test).
    if args.chain_file:
        if not args.chain_file.exists():
            # Audit fix Φ-D-STREAM.L1-8 : missing file must STILL write the
            # output JSON with a structured skip_reason. Pre-fix the script
            # returned 2 without writing → consolidated.json had the SLO
            # missing entirely (= faussement-PASS by omission).
            sys.stderr.write(f"FATAL : chain file missing : {args.chain_file}\n")
            args.out.write_text(json.dumps({
                "slo_compliance": {
                    "SLO-STREAM-AUDIT-CHAIN": {
                        "skip_reason": (
                            f"--chain-file does not exist : {args.chain_file}"
                        ),
                    }
                }
            }, indent=2) + "\n")
            return 2
        chain = [json.loads(l) for l in args.chain_file.read_text().splitlines() if l.strip()]
        mode = "live"
    else:
        chain = gen_self_test_chain(args.n_self_test, key)
        mode = "self_test"

    result = replay_chain(chain, key)
    passed = result["total_broken"] == 0

    print(f"\nAudit chain replay : {result['chain_length']} events, "
          f"{result['total_broken']} broken (mode={mode})",
          file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-STREAM-AUDIT-CHAIN": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "chain_length": result["chain_length"],
                "broken_links": result["broken_links"],
                "mode": mode,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
