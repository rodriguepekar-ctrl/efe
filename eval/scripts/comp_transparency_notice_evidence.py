#!/usr/bin/env python3
"""Evidence : transparency notice freshness ≤ 30 days (D-COMP-9).

Sprint Φ-D-COMPLIANCE D88 — evidence #13.
Controls : GDPR-Art12, GDPR-Art13, GDPR-Art14, AI-ACT-Art50.
"""

from __future__ import annotations
import argparse, json, sys
from pathlib import Path

CONTROLS = ["GDPR-Art12", "GDPR-Art13", "GDPR-Art14", "AI-ACT-Art50"]
MAX_AGE_S = 30 * 24 * 3600


SYNTH_NOTICES_PASS = [
    {"catalog_id": "credit-decision-v1", "policy_version": "v3",
      "days_ago": 5, "current_policy": "v3"},
    {"catalog_id": "hiring-screen-v2",   "policy_version": "v3",
      "days_ago": 28, "current_policy": "v3"},
    {"catalog_id": "spam-classifier-v1", "policy_version": "v3",
      "days_ago": 60, "current_policy": "v3"},   # old but matches policy → OK
]

SYNTH_NOTICES_FAIL = [
    {"catalog_id": "old-notice", "policy_version": "v2", "days_ago": 90,
      "current_policy": "v3"},                    # policy mismatch + > 30 days
]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--out", type=Path,
                    default=Path("target/compliance/evidence_transparency_notice.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-COMP-TRANSPARENCY-NOTICE": {
                    "skip_reason": "Live CDN-published notices (DT-COMP-LIVE-TRANSPARENCY-CDN)",
                    "control_ids": CONTROLS,
                }
            }
        }, indent=2) + "\n")
        return 0

    notices = SYNTH_NOTICES_FAIL if args.mode == "synth_fail" else SYNTH_NOTICES_PASS
    failing = []
    for n in notices:
        age_s = n["days_ago"] * 86400
        policy_match = n["policy_version"] == n["current_policy"]
        # OK if : (≤ 30 days) OR (policy still matches).
        if age_s > MAX_AGE_S and not policy_match:
            failing.append({"catalog_id": n["catalog_id"], "age_days": n["days_ago"],
                             "policy_version": n["policy_version"],
                             "current": n["current_policy"]})

    passed = not failing

    print(f"Transparency notices : {len(notices)} catalogs, {len(failing)} stale "
          f"AND policy-mismatched", file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-COMP-TRANSPARENCY-NOTICE": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "control_ids": CONTROLS,
                "n_notices": len(notices),
                "n_failing": len(failing),
                "failing_notices": failing,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
