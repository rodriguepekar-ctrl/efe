#!/usr/bin/env python3
"""Evidence : ROPA freshness ≤ 1 month (D-COMP-10).

Sprint Φ-D-COMPLIANCE D88 — evidence #12.
Controls : GDPR-Art30, SOC2-CC2.1, ISO-A.5.34.
"""

from __future__ import annotations
import argparse, json, sys
from pathlib import Path

CONTROLS = ["GDPR-Art30", "SOC2-CC2.1", "ISO-A.5.34"]
MAX_AGE_S = 30 * 24 * 3600   # 1 month


SYNTH_TENANTS_PASS = [
    {"tenant_id": "tenant-acme", "days_ago": 2},
    {"tenant_id": "tenant-bigcorp", "days_ago": 5},
    {"tenant_id": "tenant-startup", "days_ago": 20},
]

SYNTH_TENANTS_FAIL = [
    {"tenant_id": "tenant-stale-1", "days_ago": 45},
    {"tenant_id": "tenant-fresh", "days_ago": 5},
]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--out", type=Path,
                    default=Path("target/compliance/evidence_ropa_freshness.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-COMP-ROPA-FRESHNESS": {
                    "skip_reason": "Live ROPA cross-tenant (DT-COMP-LIVE-ROPA-CROSSPROD)",
                    "control_ids": CONTROLS,
                }
            }
        }, indent=2) + "\n")
        return 0

    tenants = SYNTH_TENANTS_FAIL if args.mode == "synth_fail" else SYNTH_TENANTS_PASS
    stale = []
    for t in tenants:
        age_s = t["days_ago"] * 86400
        if age_s > MAX_AGE_S:
            stale.append({"tenant_id": t["tenant_id"], "age_days": t["days_ago"]})

    passed = not stale

    print(f"ROPA freshness : {len(tenants)} tenants, {len(stale)} stale", file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-COMP-ROPA-FRESHNESS": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "control_ids": CONTROLS,
                "n_tenants": len(tenants),
                "n_stale": len(stale),
                "stale_tenants": stale,
                "max_age_seconds": MAX_AGE_S,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
