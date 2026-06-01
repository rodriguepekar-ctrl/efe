#!/usr/bin/env python3
"""Token-level reconciliation for Mode-2.

Sprint **Φ-D-MODE2 Completion (D66)** — artefact #13.
Cf. ``docs/EFE_MODE2_ANCHORS_COMPLETION_ARCHITECTURE.md`` § 13.3.

## Mission

Sister tool to ``reconcile_costs.py`` but at the **token** level.
For each (tenant, provider, period), compare :
* ``efe-llm-router`` accumulated token counts
* Provider-reported token usage (= via provider's usage API)

Drift > 5 % triggers alert. Drift > 10 % triggers incident.

## Why a separate tool

Cost can match while tokens drift (= per-token pricing varies). The
token-level reconciliation catches model-version surprises (= a
provider silently bumped its tokenizer).

## Stub mode

When `--provider-billing` is not provided, emit a SKIPPED SLO marker
so check_slo.py reports it as SKIPPED rather than faussement-PASS.

## Output

```json
{
  "slo_compliance": {
    "SLO-MODE2-TOKEN-TRACK": {
      "skip_reason": "Provider usage API integration deferred (DT-MODE2-TOKEN-API)"
    }
  }
}
```

Or, when live data is provided, the SLO is gated normally :

```json
{
  "slo_compliance": {
    "SLO-MODE2-TOKEN-TRACK": {
      "measured_pass_count": 1,
      "measured_expected_count": 1,
      "per_provider": {
        "openai":    {"governor_tokens": 1234567, "billing_tokens": 1230000, "drift_pct": 0.37}
      }
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

DEFAULT_TOLERANCE_PCT = 5.0


def reconcile_tokens(
    governor: dict, billing: dict, tolerance_pct: float
) -> dict:
    """Per-provider token-level drift computation."""
    result = {}
    max_drift = 0.0
    gov_providers = governor.get("providers", {})
    bill_providers = billing.get("providers", {})
    all_providers = set(gov_providers) | set(bill_providers)
    for prov in sorted(all_providers):
        gov_tokens = int(gov_providers.get(prov, {}).get("tokens_total", 0))
        bill_tokens = int(bill_providers.get(prov, {}).get("tokens_total", 0))
        if bill_tokens == 0 and gov_tokens == 0:
            drift_pct = 0.0
        elif bill_tokens == 0:
            drift_pct = 100.0
        else:
            drift_pct = abs(gov_tokens - bill_tokens) / bill_tokens * 100.0
        max_drift = max(max_drift, drift_pct)
        result[prov] = {
            "governor_tokens": gov_tokens,
            "billing_tokens": bill_tokens,
            "drift_pct": drift_pct,
            "within_tolerance": drift_pct <= tolerance_pct,
        }
    return {"per_provider": result, "max_drift_pct": max_drift}


def main() -> int:
    ap = argparse.ArgumentParser(description="Mode-2 token reconciliation")
    ap.add_argument("--governor-state", type=Path,
                    help="JSON file with cost_governor token state (optional)")
    ap.add_argument("--provider-billing", type=Path,
                    help="JSON file with per-provider token usage (optional)")
    ap.add_argument("--tolerance-pct", type=float, default=DEFAULT_TOLERANCE_PCT,
                    help="Max acceptable drift per provider (default 5 percent)")
    ap.add_argument("--out", type=Path,
                    default=Path("target/mode2/token_reconciliation.json"))
    args = ap.parse_args()

    args.out.parent.mkdir(parents=True, exist_ok=True)

    if not args.governor_state or not args.provider_billing:
        sys.stderr.write(
            "WARN : token reconciliation requires both --governor-state and "
            "--provider-billing for live mode ; emitting SKIPPED.\n"
        )
        args.out.write_text(
            json.dumps(
                {
                    "slo_compliance": {
                        "SLO-MODE2-TOKEN-TRACK": {
                            "skip_reason": (
                                "Provider usage API integration deferred "
                                "(DT-MODE2-TOKEN-API). Provide --governor-state "
                                "and --provider-billing JSONs for live gating."
                            )
                        }
                    }
                },
                indent=2,
            )
            + "\n"
        )
        return 0

    if not args.governor_state.exists():
        sys.stderr.write(f"FATAL : governor state missing : {args.governor_state}\n")
        return 2
    if not args.provider_billing.exists():
        sys.stderr.write(f"FATAL : provider billing missing : {args.provider_billing}\n")
        return 2

    governor = json.loads(args.governor_state.read_text())
    billing = json.loads(args.provider_billing.read_text())
    result = reconcile_tokens(governor, billing, args.tolerance_pct)

    print("\nToken reconciliation report :", file=sys.stderr)
    print("─" * 70, file=sys.stderr)
    print(f"{'Provider':<15}{'Governor':>15}{'Billing':>15}{'Drift':>12}", file=sys.stderr)
    print("─" * 70, file=sys.stderr)
    for prov, data in result["per_provider"].items():
        sym = "✓" if data["within_tolerance"] else "✗"
        print(
            f"{prov:<15}{data['governor_tokens']:>15,}{data['billing_tokens']:>15,}"
            f"{data['drift_pct']:>11.2f}% {sym}",
            file=sys.stderr,
        )
    print("─" * 70, file=sys.stderr)
    print(f"max drift : {result['max_drift_pct']:.2f}% (tolerance {args.tolerance_pct}%)",
          file=sys.stderr)

    passed = result["max_drift_pct"] <= args.tolerance_pct
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
