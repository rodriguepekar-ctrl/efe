#!/usr/bin/env python3
"""Replay AlLabelAttestation HMAC chain to verify integrity.

Sprint **Φ-D-AL Completion (D76)** — artefact #12.
Cf. ``docs/EFE_ACTIVE_LEARNING_COMPLETION_ARCHITECTURE.md`` § 12.

Mirror of Mode-3 `replay_session_chain.py` but for AlLabelAttestation
records (= one per gold label).
"""

from __future__ import annotations

import argparse
import hashlib
import hmac
import json
import os
import sys
from pathlib import Path

CANONICAL_FIELDS = [
    "schema_version",
    "tenant_id",
    "label_id",
    "query_hash",
    "catalog_id",
    "source",
    "label_text",
    "confidence",
    "consent_hash",
    "cost_usd_micros",
    "timestamp_unix_micros",
    "prev_hash",
]


def canonical_bytes(attestation: dict) -> bytes:
    canonical = {k: attestation.get(k) for k in CANONICAL_FIELDS}
    return json.dumps(canonical, sort_keys=True, separators=(",", ":")).encode("utf-8")


def compute_hmac(key: bytes, attestation: dict) -> str:
    h = hmac.new(key, canonical_bytes(attestation), hashlib.sha256)
    return h.hexdigest()


def replay(chain: list[dict], hmac_key: bytes) -> dict:
    broken = []
    previous = "00" * 32
    for i, a in enumerate(chain):
        prev = a.get("prev_hash", "")
        if prev != previous:
            broken.append(f"index={i} label_id={a.get('label_id')} prev_hash mismatch")
        expected = compute_hmac(hmac_key, a)
        actual = a.get("hmac_sha256", "")
        if expected != actual:
            broken.append(f"index={i} label_id={a.get('label_id')} HMAC mismatch")
        previous = actual
    return {"chain_length": len(chain), "broken_links": broken[:20], "total_broken": len(broken)}


def main() -> int:
    ap = argparse.ArgumentParser(description="AL label chain replay")
    ap.add_argument("--chain-file", type=Path)
    ap.add_argument("--hmac-key-hex")
    ap.add_argument("--hmac-key-env", default="EFE_AL_HMAC_KEY")
    ap.add_argument("--out", type=Path,
                    default=Path("target/active_learning/chain_replay.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if not args.chain_file:
        sys.stderr.write("WARN : --chain-file required ; emitting SKIPPED.\n")
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-AL-AUDIT-CHAIN-INTEGRITY": {
                    "skip_reason": "Live chain dump required (DT-AL-CHAIN-DUMP)"
                }
            }
        }, indent=2) + "\n")
        return 0

    if not args.chain_file.exists():
        sys.stderr.write(f"FATAL : chain file missing : {args.chain_file}\n")
        return 2

    if args.hmac_key_hex:
        key = bytes.fromhex(args.hmac_key_hex)
    else:
        env = os.environ.get(args.hmac_key_env)
        if not env:
            sys.stderr.write("FATAL : HMAC key not provided\n")
            return 2
        key = bytes.fromhex(env)

    chain = []
    for line in args.chain_file.read_text(encoding="utf-8").splitlines():
        if line.strip():
            chain.append(json.loads(line))

    result = replay(chain, key)
    passed = result["total_broken"] == 0

    print(f"\nLabel chain replay : {len(chain)} attestations, {result['total_broken']} broken",
          file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-AL-AUDIT-CHAIN-INTEGRITY": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "chain_length": result["chain_length"],
                "broken_links": result["broken_links"],
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
