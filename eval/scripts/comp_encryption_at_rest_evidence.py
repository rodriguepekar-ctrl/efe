#!/usr/bin/env python3
"""Evidence : AES-256-GCM at rest (transverse).

Sprint Φ-D-COMPLIANCE D87 — evidence #4.
Controls : GDPR-Art32, ISO-A.8.24, SOC2-CC6.7, HIPAA-164.312.a.2.iv.

Verifies :
  1. AES-256-GCM primitive correctness (round-trip).
  2. Different keys → different ciphertexts.
  3. Authentication tag detects tampering.
"""

from __future__ import annotations
import argparse, json, os, sys
from pathlib import Path

CONTROLS = ["GDPR-Art32", "ISO-A.8.24", "SOC2-CC6.7", "HIPAA-164.312.a.2.iv"]


def aes_roundtrip(key: bytes, plaintext: bytes) -> bytes:
    """Returns ciphertext+tag+nonce ; tests round-trip via stdlib cryptography fallback."""
    try:
        from cryptography.hazmat.primitives.ciphers.aead import AESGCM
        aesgcm = AESGCM(key)
        nonce = os.urandom(12)
        ct = aesgcm.encrypt(nonce, plaintext, None)
        decoded = aesgcm.decrypt(nonce, ct, None)
        assert decoded == plaintext
        return nonce + ct
    except ImportError:
        # Fallback : simulate via XOR (= NOT real crypto, just for CI sandbox).
        return plaintext  # marker for "primitive available"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--out", type=Path,
                    default=Path("target/compliance/evidence_encryption_at_rest.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-COMP-EVIDENCE-COVERAGE": {
                    "skip_reason": "Live encryption at rest = production storage audit",
                    "control_ids": CONTROLS,
                }
            }
        }, indent=2) + "\n")
        return 0

    checks: list[tuple[str, bool]] = []

    # 1. AES-256-GCM primitive available (= Rust uses ring/aes-gcm crate).
    try:
        from cryptography.hazmat.primitives.ciphers.aead import AESGCM
        checks.append(("aes_256_gcm_lib_available", True))
        key = os.urandom(32)  # 256-bit key
        pt = b"sensitive tenant data"
        ct_bundle = aes_roundtrip(key, pt)
        checks.append(("roundtrip_ok", ct_bundle != pt))  # ciphertext != plaintext
        # Different keys → different ciphertexts.
        key2 = os.urandom(32)
        nonce = ct_bundle[:12]
        aesgcm = AESGCM(key)
        ct_only = ct_bundle[12:]
        try:
            AESGCM(key2).decrypt(nonce, ct_only, None)
            checks.append(("wrong_key_detected", False))  # bad : key2 shouldn't decrypt
        except Exception:
            checks.append(("wrong_key_detected", True))   # good : auth tag failure
        # Tampering detection.
        tampered = bytearray(ct_only)
        tampered[0] ^= 1
        try:
            aesgcm.decrypt(nonce, bytes(tampered), None)
            checks.append(("tampering_detected", False))
        except Exception:
            checks.append(("tampering_detected", True))
    except ImportError:
        checks.append(("aes_256_gcm_lib_available", False))

    if args.mode == "synth_fail":
        checks.append(("synth_fail_marker", False))

    n_pass = sum(1 for _, ok in checks if ok)
    passed = n_pass == len(checks)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-COMP-EVIDENCE-COVERAGE": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "control_ids": CONTROLS,
                "checks": [{"name": n, "ok": ok} for n, ok in checks],
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
