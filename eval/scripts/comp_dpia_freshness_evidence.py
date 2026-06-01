#!/usr/bin/env python3
"""Evidence : DPIA freshness ≤ 6 months (D-COMP-10).

Sprint Φ-D-COMPLIANCE D88 — evidence #11.
Controls : GDPR-Art35, GDPR-Art36, AI-ACT-Art27, AI-ACT-Art9.
"""

from __future__ import annotations
import argparse, json, sys, time
from pathlib import Path

CONTROLS = ["GDPR-Art35", "GDPR-Art36", "AI-ACT-Art27", "AI-ACT-Art9"]
MAX_AGE_S = 6 * 30 * 24 * 3600   # 6 months


# Synthetic catalog → DPIA last-update timestamp (months ago).
SYNTH_CATALOGS_PASS = [
    {"catalog_id": "credit-decision-v1", "risk_level": "high",   "months_ago": 1},
    {"catalog_id": "hiring-screen-v2",   "risk_level": "high",   "months_ago": 3},
    {"catalog_id": "fraud-detect-v3",    "risk_level": "high",   "months_ago": 5},
    {"catalog_id": "spam-classifier-v1", "risk_level": "minimal","months_ago": 24},  # ignored (not high)
]

SYNTH_CATALOGS_FAIL = [
    {"catalog_id": "credit-decision-v1", "risk_level": "high",   "months_ago": 8},  # > 6 months
    {"catalog_id": "hiring-screen-v2",   "risk_level": "high",   "months_ago": 1},
]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--out", type=Path,
                    default=Path("target/compliance/evidence_dpia_freshness.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-COMP-DPIA-FRESHNESS": {
                    "skip_reason": "Live DPIA DPO sign-off (DT-COMP-LIVE-DPIA-DPO-SIGN)",
                    "control_ids": CONTROLS,
                }
            }
        }, indent=2) + "\n")
        return 0

    catalogs = SYNTH_CATALOGS_FAIL if args.mode == "synth_fail" else SYNTH_CATALOGS_PASS
    high_risk = [c for c in catalogs if c["risk_level"] == "high"]
    stale = []
    for c in high_risk:
        age_s = c["months_ago"] * 30 * 24 * 3600
        if age_s > MAX_AGE_S:
            stale.append({"catalog_id": c["catalog_id"], "age_months": c["months_ago"]})

    passed = not stale

    print(f"DPIA freshness : {len(high_risk)} high-risk catalogs, "
          f"{len(stale)} stale (> 6 months)", file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-COMP-DPIA-FRESHNESS": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "control_ids": CONTROLS,
                "n_high_risk": len(high_risk),
                "n_stale": len(stale),
                "stale_catalogs": stale,
                "max_age_seconds": MAX_AGE_S,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
