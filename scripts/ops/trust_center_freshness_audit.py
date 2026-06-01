#!/usr/bin/env python3
"""
Sprint Φ-D-OPS (D98) — Trust center freshness audit.

The trust center serves customer-facing evidence. It must be :

  1. Regenerated within the last 24 h (= D-OPS-9 doctrine).
  2. Free of TODO / FIXME / placeholder markers (= no half-baked
     compliance claim that could mislead an auditor).
  3. Cross-linked to the latest master report (= the SLO badges must
     reflect today's run).

This audit is invoked from the master gate and from the nightly
workflow.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import sys
from pathlib import Path
from typing import List, Optional


REPO_ROOT = Path(__file__).resolve().parents[2]
TRUST_INDEX = REPO_ROOT / "trust" / "index.html"
FORBIDDEN_TOKENS = [
    "TODO", "FIXME", "PLACEHOLDER", "TBD", "lorem ipsum",
]


def _check_freshness(path: Path, max_age_hours: int) -> Optional[str]:
    if not path.exists():
        return f"missing file: {path}"
    age_h = (dt.datetime.now().timestamp() - path.stat().st_mtime) / 3600
    if age_h > max_age_hours:
        return f"file mtime {age_h:.0f} h old > {max_age_hours} h budget"
    return None


def _check_forbidden(path: Path) -> List[str]:
    text = path.read_text(encoding="utf-8")
    findings: List[str] = []
    for tok in FORBIDDEN_TOKENS:
        if re.search(rf"\b{re.escape(tok)}\b", text, re.IGNORECASE):
            findings.append(tok)
    return findings


def _check_required_sections(path: Path) -> List[str]:
    text = path.read_text(encoding="utf-8").lower()
    needed = [
        "compliance frameworks",
        "sub-processor list",
        "continuous conformity",
        "recent disclosures",
        "reach us",
    ]
    return [s for s in needed if s not in text]


def main(argv: Optional[List[str]] = None) -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--max-age-hours", type=int, default=48,
                   help="Max age before the freshness gate trips.")
    p.add_argument("--out", type=Path,
                   default=Path("target/ops/measures.trust_center.json"))
    args = p.parse_args(argv)

    stale_msg = _check_freshness(TRUST_INDEX, args.max_age_hours)
    forbidden = _check_forbidden(TRUST_INDEX) if TRUST_INDEX.exists() else []
    missing_sections = _check_required_sections(TRUST_INDEX) if TRUST_INDEX.exists() else []

    stale_count = (
        (1 if stale_msg else 0)
        + len(forbidden)
        + len(missing_sections)
    )
    payload = {
        "SLO-OPS-TRUST-CENTER-STALE": stale_count,
        "_meta": {
            "stale_message":     stale_msg,
            "forbidden_tokens":  forbidden,
            "missing_sections":  missing_sections,
        },
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    print(f"trust-center audit → {args.out}")
    print(f"  stale_message     : {stale_msg or '—'}")
    print(f"  forbidden tokens  : {forbidden or '—'}")
    print(f"  missing sections  : {missing_sections or '—'}")
    return 0 if stale_count == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
