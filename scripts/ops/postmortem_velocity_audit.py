#!/usr/bin/env python3
"""
Sprint Φ-D-OPS (D97) — Postmortem velocity audit.

Verifies that every Sev1 / Sev2 incident in the last 30 days has a
postmortem committed under `docs/postmortems/` within 48 h of
resolution.

Inputs :
   1. An incident log (= JSONL ; each line one incident) provided via
      `--incident-log` (or `--demo` to use synthetic data).
   2. The git tree under `docs/postmortems/`.

Emits the SLO-OPS-POSTMORTEM-VELOCITY measurement.

Cf. `docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md` § 10.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, List, Optional


REPO_ROOT = Path(__file__).resolve().parents[2]


@dataclass
class Incident:
    incident_id: str
    severity: str
    resolved_at: dt.datetime


def _parse_iso(s: str) -> dt.datetime:
    s = s.replace("Z", "+00:00")
    return dt.datetime.fromisoformat(s).astimezone(dt.timezone.utc)


def _load_incidents(path: Optional[Path]) -> List[Incident]:
    rows: List[Incident] = []
    if path is None or not path.exists():
        return rows
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        d = json.loads(line)
        rows.append(Incident(
            incident_id=d["incident_id"],
            severity=d["severity"].lower(),
            resolved_at=_parse_iso(d["resolved_at"]),
        ))
    return rows


def _list_postmortems() -> Dict[str, dt.datetime]:
    dir = REPO_ROOT / "docs" / "postmortems"
    pm_index: Dict[str, dt.datetime] = {}
    if not dir.exists():
        return pm_index
    for f in dir.glob("*.md"):
        if f.name.startswith("_"):
            continue
        # Filename convention : YYYY-MM-DD_<slug>.md
        m = re.match(r"^(\d{4})-(\d{2})-(\d{2})", f.name)
        if not m:
            continue
        y, mo, d = (int(x) for x in m.groups())
        # Use file mtime as fallback ; filename date is canonical.
        published = dt.datetime(y, mo, d, tzinfo=dt.timezone.utc)
        slug = f.stem.split("_", 1)[1] if "_" in f.stem else f.stem
        pm_index[slug] = published
    return pm_index


def _audit(incidents: List[Incident], pm_index: Dict[str, dt.datetime],
           window_days: int) -> Dict[str, object]:
    now = dt.datetime.now(dt.timezone.utc)
    cutoff = now - dt.timedelta(days=window_days)
    overdue = []
    pending_in_window = []
    for inc in incidents:
        if inc.resolved_at < cutoff:
            continue
        if inc.severity not in ("sev1", "sev2"):
            continue
        match = next((slug for slug in pm_index if inc.incident_id in slug), None)
        if match is None:
            age_h = (now - inc.resolved_at).total_seconds() / 3600
            if age_h > 48:
                overdue.append({
                    "incident_id": inc.incident_id,
                    "severity": inc.severity,
                    "resolved_at": inc.resolved_at.isoformat(),
                    "hours_overdue": round(age_h - 48, 1),
                })
            else:
                pending_in_window.append({
                    "incident_id": inc.incident_id,
                    "hours_since_resolution": round(age_h, 1),
                })
            continue
        pm_date = pm_index[match]
        delta_h = (pm_date - inc.resolved_at).total_seconds() / 3600
        if delta_h > 48:
            overdue.append({
                "incident_id": inc.incident_id,
                "severity": inc.severity,
                "resolved_at": inc.resolved_at.isoformat(),
                "postmortem_at": pm_date.isoformat(),
                "hours_overdue": round(delta_h - 48, 1),
            })
    return {
        "incidents_in_window": sum(
            1 for i in incidents
            if i.resolved_at >= cutoff and i.severity in ("sev1", "sev2")
        ),
        "overdue":               overdue,
        "pending_in_window":     pending_in_window,
        "postmortems_present":   len(pm_index),
    }


def main(argv: Optional[List[str]] = None) -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--incident-log", type=Path)
    p.add_argument("--window-days", type=int, default=30)
    p.add_argument("--out", type=Path,
                   default=Path("target/ops/measures.postmortem.json"))
    p.add_argument("--demo", action="store_true",
                   help="Synthetic 3-incident dataset.")
    args = p.parse_args(argv)

    if args.demo:
        now = dt.datetime.now(dt.timezone.utc)
        incidents = [
            Incident("inc-2026-05-01-pg-disk", "sev1", now - dt.timedelta(hours=72)),
            Incident("inc-2026-05-08-llm-rate", "sev2", now - dt.timedelta(hours=12)),
            Incident("inc-2026-05-10-canary-stuck", "sev2", now - dt.timedelta(days=2)),
        ]
    else:
        incidents = _load_incidents(args.incident_log)

    pm_index = _list_postmortems()
    report = _audit(incidents, pm_index, args.window_days)

    payload = {
        "SLO-OPS-POSTMORTEM-VELOCITY": len(report["overdue"]),
        "_meta": report,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(payload, indent=2, default=str),
                        encoding="utf-8")
    print(f"postmortem velocity → {args.out}")
    print(f"  incidents in {args.window_days}d window : {report['incidents_in_window']}")
    print(f"  postmortems published                  : {report['postmortems_present']}")
    print(f"  overdue                                : {len(report['overdue'])}")
    return 0 if not report["overdue"] else 1


if __name__ == "__main__":
    sys.exit(main())
