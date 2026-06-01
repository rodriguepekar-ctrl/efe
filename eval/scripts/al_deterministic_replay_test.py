#!/usr/bin/env python3
"""Deterministic replay test for Active Learning chains.

Sprint **Φ-D-AL Completion (D77)** — artefact #18.
Cf. ``docs/EFE_ACTIVE_LEARNING_COMPLETION_ARCHITECTURE.md`` § 12.

Verifies that :
  1. The same `AlLabelAttestation` chain is byte-for-byte identical
     when re-canonicalized.
  2. HMAC values are stable across two independent computations.
  3. `canonical_bytes()` ignores extra (non-canonical) fields.

SLO-AL-REPLAY : deterministic replay PASS.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "scripts" / "active_learning"))
from replay_label_chain import canonical_bytes, compute_hmac, CANONICAL_FIELDS  # noqa: E402


def main() -> int:
    ap = argparse.ArgumentParser(description="AL deterministic replay test")
    ap.add_argument("--out", type=Path,
                    default=Path("target/active_learning/replay_determinism.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    results = []
    n_pass = 0

    # Build a representative attestation.
    ref = {
        "schema_version": 1,
        "tenant_id": "tenant-acme",
        "label_id": "label-001",
        "query_hash": "a" * 64,
        "catalog_id": "router-v1",
        "source": "llm_oracle",
        "label_text": "billing-dispute",
        "confidence": 0.97,
        "consent_hash": "b" * 64,
        "cost_usd_micros": 5000,
        "timestamp_unix_micros": 1700000000000000,
        "prev_hash": "00" * 32,
    }
    key = bytes(32)

    # ── Test 1 : canonical_bytes byte-identical across 2 runs.
    cb1 = canonical_bytes(ref)
    cb2 = canonical_bytes(ref)
    ok = cb1 == cb2
    results.append({"test": "canonical_bytes_idempotent", "pass": ok})
    if ok:
        n_pass += 1

    # ── Test 2 : HMAC deterministic.
    h1 = compute_hmac(key, ref)
    h2 = compute_hmac(key, ref)
    ok = h1 == h2
    results.append({"test": "hmac_deterministic", "pass": ok, "hmac": h1})
    if ok:
        n_pass += 1

    # ── Test 3 : extra fields ignored.
    augmented = dict(ref)
    augmented["extra_field"] = "should_be_ignored"
    augmented["another"] = {"nested": "junk"}
    h_aug = compute_hmac(key, augmented)
    ok = h_aug == h1  # must match because extras are not in CANONICAL_FIELDS
    results.append({"test": "extra_fields_ignored", "pass": ok})
    if ok:
        n_pass += 1

    # ── Test 4 : missing canonical field → distinct HMAC (= integrity).
    truncated = {k: ref[k] for k in CANONICAL_FIELDS if k != "label_text"}
    h_trunc = compute_hmac(key, truncated)
    ok = h_trunc != h1   # missing field must change HMAC
    results.append({"test": "missing_field_changes_hmac", "pass": ok})
    if ok:
        n_pass += 1

    # ── Test 5 : key rotation → different HMAC.
    key2 = bytes([1] * 32)
    h_alt = compute_hmac(key2, ref)
    ok = h_alt != h1
    results.append({"test": "different_key_different_hmac", "pass": ok})
    if ok:
        n_pass += 1

    # ── Test 6 : key sensitivity (single bit flip).
    key3 = bytearray(key)
    key3[0] ^= 0x01
    h_bit = compute_hmac(bytes(key3), ref)
    ok = h_bit != h1
    results.append({"test": "single_bit_key_flip", "pass": ok})
    if ok:
        n_pass += 1

    # ── Test 7 : dict-input order independence (L2 audit fix Φ-D-AL.A9).
    # If `canonical_bytes` ever drops `sort_keys=True`, the HMAC would
    # depend on input dict insertion order. Pin the invariant.
    reverse_order = {k: ref[k] for k in reversed(list(ref.keys()))}
    h_rev = compute_hmac(key, reverse_order)
    ok = h_rev == h1
    results.append({"test": "input_dict_order_independence", "pass": ok})
    if ok:
        n_pass += 1

    # ── Test 8 : tampering a canonical field changes HMAC (= sensitivity).
    # Detects regression where a field accidentally gets excluded from canon.
    tampered = dict(ref)
    tampered["confidence"] = 0.50  # was 0.97
    h_tampered = compute_hmac(key, tampered)
    ok = h_tampered != h1
    results.append({"test": "confidence_change_detected", "pass": ok})
    if ok:
        n_pass += 1

    # ── Test 9 : prev_hash change cascades (= chain integrity property).
    prev_changed = dict(ref)
    prev_changed["prev_hash"] = "ff" * 32  # different prev
    h_prev = compute_hmac(key, prev_changed)
    ok = h_prev != h1
    results.append({"test": "prev_hash_change_detected", "pass": ok})
    if ok:
        n_pass += 1

    n_total = len(results)
    passed = n_pass == n_total
    print(f"\nDeterministic replay : {n_pass}/{n_total} PASS", file=sys.stderr)
    for r in results:
        sym = "✓" if r["pass"] else "✗"
        print(f"  {sym} {r['test']}", file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-AL-REPLAY": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "n_tests": n_total,
                "n_passed": n_pass,
                "results": results,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
