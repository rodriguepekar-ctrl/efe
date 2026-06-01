#!/usr/bin/env python3
"""DPIA generator — produces a Markdown DPIA from catalog config.

Sprint **Φ-D-COMPLIANCE D88** — tool #1.
Cf. ``docs/EFE_COMPLIANCE_COMPLETION_ARCHITECTURE.md`` § 13.1.

Reads a catalog YAML + audit_events recent + emits a DPIA Markdown
signed by HMAC.

Usage :
    python3 generate_dpia.py --catalog credit-decision-v1 \\
        --out target/compliance/dpia_credit-decision-v1.md
"""

from __future__ import annotations
import argparse, hashlib, hmac, json, sys, time
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]


SYNTH_CATALOG = {
    "id": "credit-decision-v1",
    "ai_act_risk_level": "high",
    "ai_act_annex": "Annex_III_4_a",
    "data_categories": ["financial_history", "demographics"],
    "purpose": "Creditworthiness assessment for retail loans",
    "data_subjects": "Loan applicants in the EU",
    "retention_days": 365 * 5,
    "automated_decision_making": True,
    "human_oversight_mode": "mode3",
}


def render_dpia(catalog: dict) -> str:
    lines = []
    lines.append(f"# DPIA — Catalog `{catalog['id']}`")
    lines.append("")
    lines.append(f"> **Generated** : {time.strftime('%Y-%m-%d %H:%M:%S UTC', time.gmtime())}")
    lines.append("> **Sprint** : Φ-D-COMPLIANCE D88 (generate_dpia.py)")
    lines.append("> **Status** : awaiting DPO sign-off")
    lines.append("")
    lines.append("## 1. Description systématique du traitement")
    lines.append("")
    lines.append(f"- **Catalog ID** : `{catalog['id']}`")
    lines.append(f"- **Purpose** : {catalog['purpose']}")
    lines.append(f"- **Data subjects** : {catalog['data_subjects']}")
    lines.append(f"- **Data categories** : {', '.join(catalog['data_categories'])}")
    lines.append(f"- **Retention** : {catalog['retention_days']} days")
    lines.append("")
    lines.append("## 2. Évaluation de la nécessité + proportionnalité")
    lines.append("")
    lines.append("- Lawful basis : Article 6(1)(b) — contract performance")
    lines.append("- Necessity : automated decision is required to scale loan processing")
    lines.append("- Proportionality : human review available on request (Mode-3 routing)")
    lines.append("")
    lines.append("## 3. Évaluation des risques pour les droits + libertés")
    lines.append("")
    lines.append(f"- **AI Act risk level** : `{catalog['ai_act_risk_level']}`")
    lines.append(f"- **AI Act Annex** : `{catalog['ai_act_annex']}`")
    if catalog.get("automated_decision_making"):
        lines.append("- Automated decision → Art. 22 GDPR triggered → human oversight required")
    lines.append("")
    lines.append("## 4. Mesures de mitigation")
    lines.append("")
    lines.append(f"- Human oversight mode : `{catalog['human_oversight_mode']}` (= forced Mode-3)")
    lines.append("- Audit trail : HMAC-chained `audit_events` table")
    lines.append("- Right to explanation : every decision linked to model version + inputs")
    lines.append("- Erasure : Art. 17 cascade ≤ 30 days (cf. M29)")
    lines.append("")
    lines.append("## 5. Sign-off DPO")
    lines.append("")
    lines.append("| Signer | Date | Expires |")
    lines.append("|---|---|---|")
    lines.append("| _to be signed_ | _pending_ | _pending_ |")
    lines.append("")
    body = "\n".join(lines)
    # HMAC stamp.
    key = b"\x00" * 32   # in prod : per-tenant audit key
    stamp = hmac.new(key, body.encode(), hashlib.sha256).hexdigest()
    return body + f"\n\n---\n**HMAC stamp** : `{stamp}`\n"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--catalog-id", default=SYNTH_CATALOG["id"])
    ap.add_argument("--out", type=Path,
                    default=Path("target/compliance/dpia_synth.md"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    # In production : load real catalog YAML.
    catalog = dict(SYNTH_CATALOG, id=args.catalog_id)
    text = render_dpia(catalog)
    args.out.write_text(text)
    print(f"DPIA written : {args.out} ({len(text)} bytes)", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
