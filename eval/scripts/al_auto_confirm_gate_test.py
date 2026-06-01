#!/usr/bin/env python3
"""Auto-confirm gate policy test for AL labels.

Sprint **Φ-D-AL Completion (D78)** — artefact #15.
Cf. ``docs/EFE_ACTIVE_LEARNING_COMPLETION_ARCHITECTURE.md`` § 9 (D-AL-4).

The auto-confirm gate decides whether an LLM-labeled record may be
promoted to GOLD without human review. Doctrine D-AL-4 requires :

  1. LLM confidence ≥ 0.95
  2. Catalog NOT flagged `gdpr_sensitive`
  3. Tenant policy authorises auto-confirm
  4. Consent traced (= consent_hash present + non-zero)

If ALL 4 conditions hold → AUTO_CONFIRM.
If ANY 1 fails → HUMAN_REVIEW.

This script unit-tests the 4 individual conditions + the AND-of-all
gate, plus the doctrine-D-AL-4 negative edges.

SLO contribution : provides evidence for the doctrine D-AL-4 compliance
(complementary to the SLO-AL-AUTO-CONFIRM-COVERAGE metric which tracks
production rate ; this test pins the POLICY itself).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

CONFIDENCE_THRESHOLD = 0.95


def auto_confirm_gate(
    confidence: float,
    catalog_gdpr_sensitive: bool,
    tenant_allows_auto_confirm: bool,
    consent_hash: str,
) -> tuple[bool, str]:
    """Returns (allow, reason). `allow = True` means auto-confirm."""
    if confidence < CONFIDENCE_THRESHOLD:
        return (False, f"confidence {confidence:.3f} < {CONFIDENCE_THRESHOLD}")
    if catalog_gdpr_sensitive:
        return (False, "catalog flagged gdpr_sensitive")
    if not tenant_allows_auto_confirm:
        return (False, "tenant policy denies auto-confirm")
    if not isinstance(consent_hash, str) or len(consent_hash) != 64:
        return (False, "consent_hash invalid (length / type)")
    if consent_hash == "0" * 64:
        return (False, "consent_hash is zero placeholder")
    return (True, "all gates pass")


SCENARIOS = [
    # name, kwargs, expected_allow
    ("happy_path",
     {"confidence": 0.97, "catalog_gdpr_sensitive": False,
      "tenant_allows_auto_confirm": True,
      "consent_hash": "a" * 64},
     True),
    ("low_confidence",
     {"confidence": 0.90, "catalog_gdpr_sensitive": False,
      "tenant_allows_auto_confirm": True,
      "consent_hash": "a" * 64},
     False),
    ("exact_threshold",
     {"confidence": 0.95, "catalog_gdpr_sensitive": False,
      "tenant_allows_auto_confirm": True,
      "consent_hash": "a" * 64},
     True),
    ("just_below_threshold",
     {"confidence": 0.9499, "catalog_gdpr_sensitive": False,
      "tenant_allows_auto_confirm": True,
      "consent_hash": "a" * 64},
     False),
    ("gdpr_sensitive_catalog",
     {"confidence": 0.99, "catalog_gdpr_sensitive": True,
      "tenant_allows_auto_confirm": True,
      "consent_hash": "a" * 64},
     False),
    ("tenant_policy_denies",
     {"confidence": 0.99, "catalog_gdpr_sensitive": False,
      "tenant_allows_auto_confirm": False,
      "consent_hash": "a" * 64},
     False),
    ("consent_missing",
     {"confidence": 0.99, "catalog_gdpr_sensitive": False,
      "tenant_allows_auto_confirm": True,
      "consent_hash": ""},
     False),
    ("consent_zero_placeholder",
     {"confidence": 0.99, "catalog_gdpr_sensitive": False,
      "tenant_allows_auto_confirm": True,
      "consent_hash": "0" * 64},
     False),
    ("consent_wrong_length",
     {"confidence": 0.99, "catalog_gdpr_sensitive": False,
      "tenant_allows_auto_confirm": True,
      "consent_hash": "a" * 63},
     False),
    # Defense-in-depth : every individual layer must block independently.
    ("all_fail_together",
     {"confidence": 0.50, "catalog_gdpr_sensitive": True,
      "tenant_allows_auto_confirm": False,
      "consent_hash": ""},
     False),
]


def main() -> int:
    ap = argparse.ArgumentParser(description="AL auto-confirm gate test")
    ap.add_argument("--out", type=Path,
                    default=Path("target/active_learning/auto_confirm_gate.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    results = []
    n_pass = 0
    for name, kw, expected in SCENARIOS:
        got, reason = auto_confirm_gate(**kw)
        ok = got == expected
        if ok:
            n_pass += 1
        results.append({
            "scenario": name, "expected": expected, "got": got,
            "reason": reason, "pass": ok,
        })
        sym = "✓" if ok else "✗"
        print(f"  {sym} {name:<26} : got={got} expected={expected} ({reason})",
              file=sys.stderr)

    passed = n_pass == len(SCENARIOS)
    print(f"\nAuto-confirm gate : {n_pass}/{len(SCENARIOS)} PASS",
          file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            # Note : this populates a doctrine-D-AL-4-pin SLO key. The
            # SLO YAML's `SLO-AL-AUTO-CONFIRM-COVERAGE` measures the
            # production *rate*, which is separate (= live-only, DT-AL-CONFIRM-STATS).
            "POLICY-D-AL-4-AUTO-CONFIRM-GATE": {
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
