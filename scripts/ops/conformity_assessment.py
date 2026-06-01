#!/usr/bin/env python3
"""
Sprint Φ-D-OPS (D98) — Automated conformity assessment.

Walks the canonical control list (GDPR Art. + EU AI Act Art. + SOC 2
+ ISO 27001) and verifies that each control has an `evidence/`
artefact updated in the last 30 days.

Output : `target/ops/conformity_report.json` + measurement file for the
consolidator.

Cf. `docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md` § 13.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import sys
from dataclasses import dataclass, asdict
from pathlib import Path
from typing import List, Optional


REPO_ROOT = Path(__file__).resolve().parents[2]


# Authoritative control list. The path is **conventional** —
# `docs/compliance/<framework>/<control_id>.md`. The audit checks
# existence + mtime ; the contents are the regulator-facing evidence
# pack and live elsewhere (in the trust center + Stripe-customer-vault).
CONTROLS = [
    # GDPR
    ("gdpr", "art_5",  "Principles relating to processing"),
    ("gdpr", "art_13", "Information when data collected from data subject"),
    ("gdpr", "art_17", "Right to erasure"),
    ("gdpr", "art_22", "No automated individual decision-making"),
    ("gdpr", "art_25", "Data protection by design and by default"),
    ("gdpr", "art_28", "Processor"),
    ("gdpr", "art_30", "Records of processing activities"),
    ("gdpr", "art_32", "Security of processing"),
    ("gdpr", "art_33", "Breach notification"),
    ("gdpr", "art_35", "Data protection impact assessment"),
    # EU AI Act
    ("eu_ai_act", "art_9",  "Risk management system"),
    ("eu_ai_act", "art_13", "Transparency"),
    ("eu_ai_act", "art_14", "Human oversight"),
    ("eu_ai_act", "art_50", "Provider transparency to deployers"),
    # SOC 2
    ("soc2", "cc1", "Control environment"),
    ("soc2", "cc2", "Communication and information"),
    ("soc2", "cc3", "Risk assessment"),
    ("soc2", "cc4", "Monitoring"),
    ("soc2", "cc5", "Control activities"),
    ("soc2", "cc6", "Logical access"),
    ("soc2", "cc7", "System operations"),
    ("soc2", "cc8", "Change management"),
    ("soc2", "cc9", "Risk mitigation"),
    # ISO 27001 (Annex A summary cluster)
    ("iso27001", "annex_a_5", "Information security policies"),
    ("iso27001", "annex_a_6", "Organisation of information security"),
    ("iso27001", "annex_a_7", "Human resource security"),
    ("iso27001", "annex_a_8", "Asset management"),
    ("iso27001", "annex_a_9", "Access control"),
]


@dataclass
class ControlStatus:
    framework: str
    control: str
    title: str
    evidence_path: str
    present: bool
    age_days: Optional[float]
    fresh: bool


def main(argv: Optional[List[str]] = None) -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--evidence-root",
                   default="docs/compliance/evidence")
    p.add_argument("--max-age-days", type=int, default=30)
    p.add_argument("--out", type=Path,
                   default=Path("target/ops/conformity_report.json"))
    p.add_argument("--measures-out", type=Path,
                   default=Path("target/ops/measures.conformity.json"))
    args = p.parse_args(argv)

    root = REPO_ROOT / args.evidence_root
    now = dt.datetime.now().timestamp()

    statuses: List[ControlStatus] = []
    stale = 0
    missing = 0
    for fw, ctrl, title in CONTROLS:
        rel = f"{args.evidence_root}/{fw}/{ctrl}.md"
        path = REPO_ROOT / rel
        if path.exists():
            age_days = (now - path.stat().st_mtime) / 86400
            fresh = age_days <= args.max_age_days
            if not fresh:
                stale += 1
        else:
            age_days = None
            fresh = False
            missing += 1
        statuses.append(ControlStatus(
            framework=fw, control=ctrl, title=title,
            evidence_path=rel, present=path.exists(),
            age_days=None if age_days is None else round(age_days, 2),
            fresh=fresh,
        ))

    report = {
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "controls": [asdict(s) for s in statuses],
        "summary": {
            "total":   len(statuses),
            "present": sum(1 for s in statuses if s.present),
            "fresh":   sum(1 for s in statuses if s.fresh),
            "stale":   stale,
            "missing": missing,
        },
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2), encoding="utf-8")

    stale_total = stale + missing
    payload = {
        "SLO-OPS-COMPLIANCE-POSTURE": stale_total,
        "_meta": report["summary"],
    }
    args.measures_out.write_text(json.dumps(payload, indent=2),
                                 encoding="utf-8")

    print(f"conformity report  → {args.out}")
    print(f"measures           → {args.measures_out}")
    print(f"  total            : {report['summary']['total']}")
    print(f"  present          : {report['summary']['present']}")
    print(f"  fresh            : {report['summary']['fresh']}")
    print(f"  stale            : {report['summary']['stale']}")
    print(f"  missing          : {report['summary']['missing']}")
    return 0 if stale_total == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
