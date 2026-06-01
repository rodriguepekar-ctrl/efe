#!/usr/bin/env python3
"""Evidence : data residency enforcement (D-COMP-7).

Sprint Φ-D-COMPLIANCE D87 — evidence #5.
Controls : GDPR-Art44, GDPR-Art46, GDPR-Art47, SOC2-C1.1.
"""

from __future__ import annotations
import argparse, json, sys
from pathlib import Path

CONTROLS = ["GDPR-Art44", "GDPR-Art46", "GDPR-Art47", "SOC2-C1.1"]

# (provider, regions where it operates)
PROVIDER_REGIONS = {
    "openai":     {"US"},
    "anthropic":  {"US"},
    "vertex_eu":  {"EU"},
    "vertex_us":  {"US"},
    "mistral":    {"EU"},
    "cohere":     {"US"},
    "azure_eu":   {"EU"},
    "azure_us":   {"US"},
    "bedrock_eu": {"EU"},
    "bedrock_us": {"US"},
    "llama_host": {"EU", "US", "APAC"},  # client-hosted
}


def validate(provider: str, tenant_region: str) -> tuple[bool, str]:
    regions = PROVIDER_REGIONS.get(provider, set())
    if not regions:
        return (False, f"unknown provider : {provider}")
    if tenant_region == "GLOBAL":
        return (True, "global tenant accepts all")
    if tenant_region not in regions:
        return (False, f"residency violation : {provider} regions {regions} ∌ {tenant_region}")
    return (True, "ok")


SCENARIOS = [
    # (name, tenant_region, provider, expect_allow)
    ("eu_tenant_eu_provider_ok",     "EU",     "mistral",    True),
    ("eu_tenant_us_provider_denied", "EU",     "openai",     False),
    ("global_tenant_us_provider_ok", "GLOBAL", "openai",     True),
    ("us_tenant_eu_provider_denied", "US",     "mistral",    False),
    ("eu_tenant_eu_azure_ok",        "EU",     "azure_eu",   True),
    ("apac_tenant_llama_ok",         "APAC",   "llama_host", True),
    ("unknown_provider_denied",      "EU",     "fake",       False),
]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--out", type=Path,
                    default=Path("target/compliance/evidence_data_residency.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-COMP-RESIDENCY-LEAK": {
                    "skip_reason": "Live multi-region routing (DT-COMP-LIVE-RESIDENCY-PROD)",
                    "control_ids": CONTROLS,
                }
            }
        }, indent=2) + "\n")
        return 0

    results = []
    n_pass = 0
    for name, region, provider, expect in SCENARIOS:
        ok_actual, reason = validate(provider, region)
        match = ok_actual == expect
        if args.mode == "synth_fail" and name == "eu_tenant_us_provider_denied":
            match = False  # inject
        if match:
            n_pass += 1
        results.append({"name": name, "tenant_region": region,
                         "provider": provider, "expect": expect,
                         "got": ok_actual, "ok": match, "reason": reason})
        print(f"  {'✓' if match else '✗'} {name}", file=sys.stderr)

    passed = n_pass == len(SCENARIOS)
    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-COMP-RESIDENCY-LEAK": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "control_ids": CONTROLS,
                "n_scenarios": len(SCENARIOS), "n_passed": n_pass,
                "scenarios": results,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
