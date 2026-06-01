#!/usr/bin/env python3
"""Evidence : EU AI Act risk classification per catalog (D-COMP-8).

Sprint Φ-D-COMPLIANCE D88 — evidence #14.
Controls : AI-ACT-Art6, AI-ACT-Art7, AI-ACT-AnnexIII, AI-ACT-Art9, AI-ACT-Art16.
"""

from __future__ import annotations
import argparse, json, sys
from pathlib import Path

CONTROLS = ["AI-ACT-Art6", "AI-ACT-Art7", "AI-ACT-AnnexIII",
             "AI-ACT-Art9", "AI-ACT-Art16"]
VALID_LEVELS = {"minimal", "limited", "high", "unacceptable"}


SYNTH_CATALOGS_PASS = [
    {"catalog_id": "credit-decision-v1", "ai_act_risk_level": "high",
      "ai_act_annex": "Annex_III_4_a"},
    {"catalog_id": "hiring-screen-v2",   "ai_act_risk_level": "high",
      "ai_act_annex": "Annex_III_5_a"},
    {"catalog_id": "spam-classifier-v1", "ai_act_risk_level": "minimal",
      "ai_act_annex": None},
    {"catalog_id": "chatbot-v2",         "ai_act_risk_level": "limited",
      "ai_act_annex": None},
]

SYNTH_CATALOGS_FAIL = [
    {"catalog_id": "missing-level", "ai_act_risk_level": None, "ai_act_annex": None},
    {"catalog_id": "invalid-level", "ai_act_risk_level": "MAGIC", "ai_act_annex": None},
    {"catalog_id": "unacceptable",  "ai_act_risk_level": "unacceptable", "ai_act_annex": None},
]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--out", type=Path,
                    default=Path("target/compliance/evidence_ai_act_risk.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-COMP-AI-ACT-CONTROLS": {
                    "skip_reason": "Live catalog scan (DT-COMP-LIVE-AI-ACT-AUDIT)",
                    "control_ids": CONTROLS,
                }
            }
        }, indent=2) + "\n")
        return 0

    catalogs = SYNTH_CATALOGS_FAIL if args.mode == "synth_fail" else SYNTH_CATALOGS_PASS

    issues = []
    n_high = 0
    for c in catalogs:
        level = c.get("ai_act_risk_level")
        if level is None:
            issues.append({"catalog_id": c["catalog_id"], "issue": "missing risk_level"})
        elif level not in VALID_LEVELS:
            issues.append({"catalog_id": c["catalog_id"],
                             "issue": f"invalid level : {level}"})
        elif level == "unacceptable":
            issues.append({"catalog_id": c["catalog_id"],
                             "issue": "unacceptable risk catalog MUST be removed (Art. 5)"})
        elif level == "high":
            n_high += 1
            if not c.get("ai_act_annex"):
                issues.append({"catalog_id": c["catalog_id"],
                                 "issue": "high-risk catalog without ai_act_annex"})

    passed = not issues

    print(f"AI Act classification : {len(catalogs)} catalogs ({n_high} high-risk), "
          f"{len(issues)} issues", file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-COMP-AI-ACT-CONTROLS": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "control_ids": CONTROLS,
                "n_catalogs": len(catalogs),
                "n_high_risk": n_high,
                "issues": issues,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
