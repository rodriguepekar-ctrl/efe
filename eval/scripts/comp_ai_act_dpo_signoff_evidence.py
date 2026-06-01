#!/usr/bin/env python3
"""Evidence : AI Act DPO sign-off for high-risk catalogs (D-COMP-8).

Sprint Φ-D-COMPLIANCE D88 — evidence #16.
Controls : GDPR-Art37, GDPR-Art39, AI-ACT-Art16.c, AI-ACT-Art43.
"""

from __future__ import annotations
import argparse, json, sys, time
from pathlib import Path

CONTROLS = ["GDPR-Art37", "GDPR-Art39", "AI-ACT-Art16.c", "AI-ACT-Art43"]
MAX_AGE_S = 365 * 24 * 3600   # 1 year


SYNTH_CATALOGS_PASS = [
    {"catalog_id": "credit-decision-v1", "risk": "high",
      "dpo_signoff": {"signer": "alice@efe.ai", "days_ago": 60}},
    {"catalog_id": "hiring-screen-v2", "risk": "high",
      "dpo_signoff": {"signer": "bob@efe.ai", "days_ago": 200}},
    {"catalog_id": "spam-classifier-v1", "risk": "minimal",
      "dpo_signoff": None},   # not required for minimal
]

SYNTH_CATALOGS_FAIL = [
    {"catalog_id": "expired", "risk": "high",
      "dpo_signoff": {"signer": "carol@efe.ai", "days_ago": 400}},   # > 1 year
    {"catalog_id": "missing", "risk": "high", "dpo_signoff": None},  # no sign-off
]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--out", type=Path,
                    default=Path("target/compliance/evidence_ai_act_dpo_signoff.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-COMP-AI-ACT-DPO-SIGNOFF": {
                    "skip_reason": "Live DPO sign-off registry",
                    "control_ids": CONTROLS,
                }
            }
        }, indent=2) + "\n")
        return 0

    catalogs = SYNTH_CATALOGS_FAIL if args.mode == "synth_fail" else SYNTH_CATALOGS_PASS

    issues = []
    n_high = 0
    for c in catalogs:
        if c["risk"] != "high":
            continue
        n_high += 1
        signoff = c.get("dpo_signoff")
        if signoff is None:
            issues.append({"catalog_id": c["catalog_id"], "issue": "no sign-off"})
            continue
        age_s = signoff["days_ago"] * 86400
        if age_s > MAX_AGE_S:
            issues.append({"catalog_id": c["catalog_id"],
                             "issue": f"sign-off expired ({signoff['days_ago']} days ago)"})

    passed = not issues

    print(f"DPO sign-off : {n_high} high-risk catalogs, {len(issues)} issues",
          file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-COMP-AI-ACT-DPO-SIGNOFF": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "control_ids": CONTROLS,
                "n_high_risk": n_high,
                "issues": issues,
                "max_age_seconds": MAX_AGE_S,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
