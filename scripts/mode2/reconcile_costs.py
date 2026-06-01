#!/usr/bin/env python3
"""Reconcile Mode-2 cost tracking vs provider billing.

Sprint **Φ-D-MODE2 Completion (D66)** — artefact #12.
Cf. ``docs/EFE_MODE2_ANCHORS_COMPLETION_ARCHITECTURE.md`` § 7.4 + § 13.3.

## Mission

For each (tenant, provider, period), compare the cost accumulated by
`efe-llm-router::cost_governor` to the actual billing reported by the
provider's API. Drift > 5 % triggers alert ; drift > 10 % triggers
incident.

## Inputs

* `--governor-state` : JSON dump of cost_governor state (= live API
  or local snapshot).
* `--provider-billing` : JSON file per provider with billing data.

## Output

```json
{
  "slo_compliance": {
    "SLO-MODE2-TOKEN-TRACK": {
      "measured_pass_count": 1,
      "measured_expected_count": 1,
      "per_provider": {
        "openai":    {"governor_usd": 123.45, "billing_usd": 121.00, "drift_pct": 2.0},
        "anthropic": {"governor_usd": 56.78,  "billing_usd": 57.10,  "drift_pct": 0.56}
      },
      "max_drift_pct": 2.0
    }
  }
}
```
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

# Tolerance : 5 % drift max per provider.
DEFAULT_TOLERANCE_PCT = 5.0


def reconcile_per_provider(
    governor: dict, billing: dict, tolerance_pct: float
) -> dict:
    """Per-provider drift computation.

    `governor` schema : {"providers": {"openai": {"usd_total": X, "tokens_input": ..., "tokens_output": ...}, ...}}
    `billing`  schema : {"providers": {"openai": {"usd_total": X}, ...}}
    """
    result = {}
    max_drift = 0.0
    gov_providers = governor.get("providers", {})
    bill_providers = billing.get("providers", {})
    all_providers = set(gov_providers) | set(bill_providers)
    for prov in sorted(all_providers):
        gov_usd = float(gov_providers.get(prov, {}).get("usd_total", 0.0))
        bill_usd = float(bill_providers.get(prov, {}).get("usd_total", 0.0))
        if bill_usd == 0.0 and gov_usd == 0.0:
            drift_pct = 0.0
        elif bill_usd == 0.0:
            # Governor reports cost but provider billing is zero → drift is 100 %.
            drift_pct = 100.0
        else:
            drift_pct = abs(gov_usd - bill_usd) / bill_usd * 100.0
        max_drift = max(max_drift, drift_pct)
        result[prov] = {
            "governor_usd": gov_usd,
            "billing_usd": bill_usd,
            "drift_pct": drift_pct,
            "within_tolerance": drift_pct <= tolerance_pct,
        }
    return {"per_provider": result, "max_drift_pct": max_drift}


def main() -> int:
    ap = argparse.ArgumentParser(description="Mode-2 cost reconciliation")
    ap.add_argument("--governor-state", type=Path, required=True,
                    help="JSON file with cost_governor's accumulated state")
    ap.add_argument("--provider-billing", type=Path, required=True,
                    help="JSON file with provider-side billing data")
    ap.add_argument("--tolerance-pct", type=float, default=DEFAULT_TOLERANCE_PCT,
                    help="Max acceptable drift per provider (default 5 percent)")
    ap.add_argument("--out", type=Path,
                    default=Path("target/mode2/cost_reconciliation.json"))
    args = ap.parse_args()

    if not args.governor_state.exists():
        sys.stderr.write(f"FATAL : governor state missing : {args.governor_state}\n")
        return 2
    if not args.provider_billing.exists():
        sys.stderr.write(f"FATAL : provider billing missing : {args.provider_billing}\n")
        return 2

    governor = json.loads(args.governor_state.read_text())
    billing = json.loads(args.provider_billing.read_text())
    result = reconcile_per_provider(governor, billing, args.tolerance_pct)

    print("\nCost reconciliation report :", file=sys.stderr)
    print("─" * 60, file=sys.stderr)
    print(f"{'Provider':<15}{'Governor':>15}{'Billing':>15}{'Drift':>10}", file=sys.stderr)
    print("─" * 60, file=sys.stderr)
    for prov, data in result["per_provider"].items():
        sym = "✓" if data["within_tolerance"] else "✗"
        print(
            f"{prov:<15}{data['governor_usd']:>14.4f}${data['billing_usd']:>14.4f}$"
            f"{data['drift_pct']:>9.2f}% {sym}",
            file=sys.stderr,
        )
    print("─" * 60, file=sys.stderr)
    print(f"max drift : {result['max_drift_pct']:.2f}% (tolerance {args.tolerance_pct}%)",
          file=sys.stderr)

    passed = result["max_drift_pct"] <= args.tolerance_pct
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(
        json.dumps(
            {
                "slo_compliance": {
                    "SLO-MODE2-TOKEN-TRACK": {
                        "measured_pass_count": 1 if passed else 0,
                        "measured_expected_count": 1,
                        "max_drift_pct": result["max_drift_pct"],
                        "per_provider": result["per_provider"],
                    }
                }
            },
            indent=2,
        )
        + "\n"
    )

    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
