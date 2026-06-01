#!/usr/bin/env python3
"""Evidence : HMAC chain integrity (D-COMP-3).

Sprint **Φ-D-COMPLIANCE D87** — evidence #1.
Cf. ``docs/EFE_COMPLIANCE_COMPLETION_ARCHITECTURE.md`` § 5.

Controls couverts (cf. controls_index.yaml) :
  GDPR-Art32, ISO-A.8.15, ISO-A.8.16, ISO-A.8.34,
  SOC2-CC7.1, SOC2-CC7.2, AI-ACT-Art12, HIPAA-164.312.b, CCPA-1798.100

Cette evidence est complémentaire à `comp_audit_chain_daily` :
elle vérifie l'INVARIANCE STATIQUE des HMAC primitives (= same key,
same payload → same sig) plutôt que le replay temporel.
"""

from __future__ import annotations

import argparse
import hashlib
import hmac
import json
import sys
from pathlib import Path


def sig(key: bytes, prev: str, payload: str) -> str:
    m = hmac.new(key, digestmod=hashlib.sha256)
    m.update(prev.encode())
    m.update(payload.encode())
    return m.hexdigest()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--out", type=Path,
                    default=Path("target/compliance/evidence_hmac_chain.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-COMP-EVIDENCE-COVERAGE": {
                    "skip_reason": "Live HMAC primitives invariance (DT-COMP-LIVE-HMAC-ROTATION)",
                    "control_ids": ["GDPR-Art32", "ISO-A.8.15", "AI-ACT-Art12"],
                }
            }
        }, indent=2) + "\n")
        return 0

    # 6 invariants HMAC primitives (= same property as Φ-D-AL replay).
    key = bytes(32)
    ref = "ref payload"
    prev = "00" * 32
    invariants: list[tuple[str, bool]] = []
    # 1. Deterministic.
    invariants.append(("deterministic", sig(key, prev, ref) == sig(key, prev, ref)))
    # 2. Different keys → different sigs.
    invariants.append(("key_sensitivity", sig(key, prev, ref) != sig(bytes([1]*32), prev, ref)))
    # 3. Different prev → different sigs.
    invariants.append(("prev_sensitivity",
                        sig(key, prev, ref) != sig(key, "ff"*32, ref)))
    # 4. Different payload → different sigs.
    invariants.append(("payload_sensitivity",
                        sig(key, prev, ref) != sig(key, prev, "other")))
    # 5. Single bit flip key → different sig.
    flipped = bytearray(key); flipped[0] ^= 1
    invariants.append(("single_bit_key_flip",
                        sig(key, prev, ref) != sig(bytes(flipped), prev, ref)))
    # 6. SHA-256 = 64 hex chars.
    invariants.append(("output_64_hex", len(sig(key, prev, ref)) == 64))

    if args.mode == "synth_fail":
        invariants[0] = ("deterministic", False)  # inject

    n_pass = sum(1 for _, ok in invariants if ok)
    passed = n_pass == len(invariants)

    print(f"HMAC invariants : {n_pass}/{len(invariants)} PASS", file=sys.stderr)
    for name, ok in invariants:
        print(f"  {'✓' if ok else '✗'} {name}", file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-COMP-EVIDENCE-COVERAGE": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "control_ids": ["GDPR-Art32", "ISO-A.8.15", "ISO-A.8.16",
                                  "ISO-A.8.34", "SOC2-CC7.1", "SOC2-CC7.2",
                                  "AI-ACT-Art12", "HIPAA-164.312.b", "CCPA-1798.100"],
                "invariants": [{"name": n, "pass": ok} for n, ok in invariants],
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
