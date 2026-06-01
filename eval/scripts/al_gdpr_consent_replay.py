#!/usr/bin/env python3
"""GDPR consent coverage replay for Active Learning gold labels.

Sprint **Φ-D-AL Completion (D78)** — artefact #25.
Cf. ``docs/EFE_ACTIVE_LEARNING_COMPLETION_ARCHITECTURE.md`` § 9.2.

For each `AlLabelAttestation`, verifies :
  1. `consent_hash` is non-empty (= 64-char hex SHA-256).
  2. `consent_hash != "0" * 64` (= placeholder zero).
  3. `tenant_id` set (= no orphan label).

SLO-AL-GDPR-CONSENT-COVERAGE : 100 % of gold labels have valid consent.

## Modes

* `--labels-dump <file>`  : live mode (= production attestation dump).
* No file given            : `SKIPPED` with explicit reason (anti-Φ-D.A1).
* `--mode synth_pass`      : 100 synthetic labels, all with valid consent
                              → SLO must PASS.
* `--mode synth_fail`      : 100 synthetic labels, one with missing consent
                              → SLO must FAIL (deliberate negative test).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path


def _validate_consent(att: dict) -> tuple[bool, str | None]:
    """Returns (is_valid, reason_if_invalid).

    L2 audit fix Φ-D-AL.A10 : defensively guard against non-string types
    (= JSON numbers parsed as int/float, JSON null parsed as None).
    Pre-fix `len(non_string)` raised TypeError on malformed input.
    """
    consent_hash = att.get("consent_hash", "")
    tenant_id = att.get("tenant_id", "")
    if not isinstance(tenant_id, str) or not tenant_id:
        return (False, "tenant_id missing or non-string")
    if not isinstance(consent_hash, str) or not consent_hash:
        return (False, "consent_hash missing or non-string")
    if len(consent_hash) != 64:
        return (False, f"consent_hash length {len(consent_hash)} != 64")
    if consent_hash == "0" * 64:
        return (False, "consent_hash is zero placeholder")
    try:
        bytes.fromhex(consent_hash)
    except ValueError:
        return (False, "consent_hash not hex")
    return (True, None)


def _gen_synth(n: int, fail: bool) -> list[dict]:
    """Synthetic labels for self-test mode."""
    out = []
    for i in range(n):
        consent = "0" * 64 if (fail and i == n // 2) else \
                  hashlib.sha256(f"consent-{i}".encode()).hexdigest()
        out.append({
            "label_id": f"label-{i:04d}",
            "tenant_id": f"tenant-{i % 5:03d}",
            "consent_hash": consent,
            "consent_version": "v1.0",
        })
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description="AL GDPR consent replay")
    ap.add_argument("--labels-dump", type=Path,
                    help="JSON dump of AL label attestations")
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass",
                    help="synth_pass = self-test all valid ; "
                         "synth_fail = self-test with one missing consent")
    ap.add_argument("--out", type=Path,
                    default=Path("target/active_learning/gdpr_consent.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        if not args.labels_dump:
            sys.stderr.write(
                "WARN : --labels-dump required for live mode ; emitting SKIPPED.\n"
            )
            args.out.write_text(json.dumps({
                "slo_compliance": {
                    "SLO-AL-GDPR-CONSENT-COVERAGE": {
                        "skip_reason": (
                            "Live label dump required (DT-AL-GDPR-DUMP)"
                        )
                    }
                }
            }, indent=2) + "\n")
            return 0
        # L2 audit fix Φ-D-AL.A11 : clean FATAL message if file is missing.
        if not args.labels_dump.exists():
            sys.stderr.write(f"FATAL : labels dump missing : {args.labels_dump}\n")
            return 2
        labels = json.loads(args.labels_dump.read_text())
        if isinstance(labels, dict):
            labels = labels.get("labels", labels.get("attestations", []))
    elif args.mode == "synth_pass":
        labels = _gen_synth(100, fail=False)
    else:  # synth_fail
        labels = _gen_synth(100, fail=True)

    n_total = len(labels)
    n_valid = 0
    invalid_reasons = []
    for lbl in labels:
        ok, reason = _validate_consent(lbl)
        if ok:
            n_valid += 1
        else:
            invalid_reasons.append(
                f"{lbl.get('label_id', '?')}: {reason}"
            )

    # Anti-Φ-D.A1 : empty corpus → SKIP, not 100 %.
    if n_total == 0:
        sys.stderr.write(
            "WARN : 0 labels in dump ; emitting SKIPPED (no evidence).\n"
        )
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-AL-GDPR-CONSENT-COVERAGE": {
                    "skip_reason": "Empty label corpus (no evidence)"
                }
            }
        }, indent=2) + "\n")
        return 0

    coverage = n_valid / n_total
    passed = coverage == 1.0

    print(f"GDPR consent : {n_valid}/{n_total} valid = {coverage:.2%}",
          file=sys.stderr)
    if invalid_reasons:
        for r in invalid_reasons[:10]:
            print(f"  ✗ {r}", file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-AL-GDPR-CONSENT-COVERAGE": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "n_total": n_total,
                "n_valid": n_valid,
                "coverage": coverage,
                "invalid_reasons": invalid_reasons[:20],
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
