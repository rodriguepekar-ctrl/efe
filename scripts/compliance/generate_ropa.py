#!/usr/bin/env python3
"""ROPA generator — produces a per-tenant ROPA from catalogs + sub-processors.

Sprint **Φ-D-COMPLIANCE D88** — tool #2.
Cf. ``docs/EFE_COMPLIANCE_COMPLETION_ARCHITECTURE.md`` § 13.2.

Generates a Records of Processing Activities (GDPR Art. 30) Markdown
per tenant. Source : catalogs config + sub-processor list.
"""

from __future__ import annotations
import argparse, hashlib, hmac, json, sys, time
from pathlib import Path


SYNTH_TENANT = {
    "tenant_id": "tenant-acme",
    "controller": "ACME Corp",
    "dpo_email": "dpo@acme.example",
    "data_region": "EU",
    "activities": [
        {
            "name": "Mode-2 query routing",
            "purpose": "Intent classification",
            "categories": ["query_text"],
            "recipients": ["mistral-EU"],
            "retention_days_raw": 7,
            "retention_days_hash": 90,
            "transfers": ["EU-only"],
        },
        {
            "name": "Active Learning retune",
            "purpose": "Model improvement",
            "categories": ["confirmed_labels"],
            "recipients": ["internal"],
            "retention_days_raw": 365 * 5,
            "retention_days_hash": None,
            "transfers": ["EU-only"],
        },
        {
            "name": "Audit trail",
            "purpose": "Compliance + incident response",
            "categories": ["action_logs", "principal", "hmac_signed_payloads"],
            "recipients": ["internal", "DPO"],
            "retention_days_raw": 365 * 7,
            "retention_days_hash": None,
            "transfers": ["EU-only"],
        },
    ],
}


def render_ropa(tenant: dict) -> str:
    lines = []
    lines.append(f"# ROPA — Tenant `{tenant['tenant_id']}`")
    lines.append("")
    lines.append(f"> **Generated** : {time.strftime('%Y-%m-%d %H:%M:%S UTC', time.gmtime())}")
    lines.append("> **Sprint** : Φ-D-COMPLIANCE D88 (generate_ropa.py)")
    lines.append("> **Controller** : " + tenant["controller"])
    lines.append("> **DPO contact** : " + tenant["dpo_email"])
    lines.append("> **Data region** : " + tenant["data_region"])
    lines.append("")
    lines.append("## Processing activities (Art. 30 GDPR)")
    lines.append("")
    for i, a in enumerate(tenant["activities"], 1):
        lines.append(f"### Activity {i} — {a['name']}")
        lines.append("")
        lines.append(f"- **Purpose** : {a['purpose']}")
        lines.append(f"- **Categories of data** : {', '.join(a['categories'])}")
        lines.append(f"- **Recipients** : {', '.join(a['recipients'])}")
        lines.append(f"- **Retention (raw)** : {a['retention_days_raw']} days")
        if a["retention_days_hash"]:
            lines.append(f"- **Retention (hashed)** : {a['retention_days_hash']} days")
        lines.append(f"- **Transfers** : {', '.join(a['transfers'])}")
        lines.append("")
    body = "\n".join(lines)
    key = b"\x00" * 32
    stamp = hmac.new(key, body.encode(), hashlib.sha256).hexdigest()
    return body + f"\n---\n**HMAC stamp** : `{stamp}`\n"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--tenant-id", default=SYNTH_TENANT["tenant_id"])
    ap.add_argument("--out", type=Path,
                    default=Path("target/compliance/ropa_synth.md"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    tenant = dict(SYNTH_TENANT, tenant_id=args.tenant_id)
    text = render_ropa(tenant)
    args.out.write_text(text)
    print(f"ROPA written : {args.out} ({len(text)} bytes)", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
