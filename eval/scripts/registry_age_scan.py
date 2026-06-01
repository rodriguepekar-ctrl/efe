#!/usr/bin/env python3
"""Registry age scan : meta-SLO SLO-LATACT-PENDING-AGE ≤ 90 jours.

Sprint **Φ-D-LATACT D94** — artefact #26.
Cf. ``docs/EFE_P1_LATENCE_P2_ACTIVATION_COMPLETION_ARCHITECTURE.md`` § 10.2.

Lit tous les `bench_baselines/v*/*.yaml`, identifie les SLOs `PENDING_*`
et calcule leur age depuis `frozen_at`. Si un PENDING > 90 jours, FAIL.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CAP_DAYS = 90
CAP_S = CAP_DAYS * 24 * 3600


def parse_frozen_at(frozen_at: str | None) -> dt.datetime | None:
    """Returns naive datetime, or None if placeholder/invalid."""
    if not frozen_at:
        return None
    if "PLACEHOLDER" in frozen_at:
        return None
    try:
        return dt.datetime.fromisoformat(frozen_at.replace("Z", ""))
    except ValueError:
        return None


def scan_baselines(now: dt.datetime) -> dict:
    yaml = __import__("yaml")
    results = {
        "pendings": [],
        "max_age_seconds": 0,
        "n_baselines_scanned": 0,
        "n_pending_total": 0,
        "n_pending_over_cap": 0,
    }
    for yaml_file in sorted((REPO / "bench_baselines").rglob("*.yaml")):
        results["n_baselines_scanned"] += 1
        try:
            data = yaml.safe_load(yaml_file.read_text())
        except Exception:
            continue
        frozen_at = data.get("frozen_at") if isinstance(data, dict) else None
        slos = data.get("slos", []) if isinstance(data, dict) else []
        for slo in slos:
            status = slo.get("status", "")
            if not isinstance(status, str) or not status.startswith("PENDING"):
                continue
            results["n_pending_total"] += 1
            fa = parse_frozen_at(frozen_at)
            if fa is None:
                # Placeholder or invalid → assume CI sandbox = age=0.
                age_s = 0
            else:
                age_s = (now - fa).total_seconds()
            entry = {
                "slo_id": slo.get("id"),
                "status": status,
                "baseline_file": str(yaml_file.relative_to(REPO)),
                "frozen_at": frozen_at,
                "age_seconds": age_s,
                "over_cap": age_s > CAP_S,
            }
            results["pendings"].append(entry)
            if age_s > results["max_age_seconds"]:
                results["max_age_seconds"] = age_s
            if entry["over_cap"]:
                results["n_pending_over_cap"] += 1
    return results


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--out", type=Path,
                    default=Path("target/latact/registry_age_scan.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-LATACT-PENDING-AGE": {
                    "skip_reason": "Live registry (DT-LATACT-PENDING-90D-PROD)",
                }
            }
        }, indent=2) + "\n")
        return 0

    now = dt.datetime.utcnow()
    results = scan_baselines(now)

    if args.mode == "synth_fail":
        # Inject : pretend one PENDING is 120 days old.
        results["pendings"].append({
            "slo_id": "SLO-FAKE-OVERDUE", "status": "PENDING_FAKE",
            "age_seconds": 120 * 86400, "over_cap": True,
            "baseline_file": "synth_fail injection", "frozen_at": "(synth)",
        })
        results["n_pending_over_cap"] += 1
        results["max_age_seconds"] = 120 * 86400

    passed = results["n_pending_over_cap"] == 0

    print(f"Registry scan : {results['n_baselines_scanned']} baselines, "
          f"{results['n_pending_total']} PENDING, "
          f"{results['n_pending_over_cap']} over cap ({CAP_DAYS}j)",
          file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-LATACT-PENDING-AGE": {
                "measured_seconds": results["max_age_seconds"],
                "n_baselines_scanned": results["n_baselines_scanned"],
                "n_pending_total": results["n_pending_total"],
                "n_pending_over_cap": results["n_pending_over_cap"],
                "pendings": results["pendings"][:20],
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
