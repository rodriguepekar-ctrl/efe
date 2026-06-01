#!/usr/bin/env python3
"""P1 burn-rate 4×3 matrix coverage audit.

Sprint **Φ-D-LATACT D91** — artefact #09.
Cf. ``docs/EFE_P1_LATENCE_P2_ACTIVATION_COMPLETION_ARCHITECTURE.md`` § 6.

Vérifie que le fichier `alerts/p1_burn_4x3.yaml` déclare bien les
12 cellules (= 4 fenêtres × 3 sévérités).

Emit SLO-LATACT-BURN-4x3.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
ALERTS_FILE = REPO / "alerts" / "p1_burn_4x3.yaml"

WINDOWS = ["1h", "6h", "24h", "72h"]
SEVERITIES = ["critical", "major", "watch"]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--out", type=Path,
                    default=Path("target/latact/burn_4x3_coverage.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-LATACT-BURN-4x3": {
                    "skip_reason": "Live Prometheus alerts file (= same as static)",
                }
            }
        }, indent=2) + "\n")
        return 0

    if not ALERTS_FILE.exists():
        sys.stderr.write(f"FATAL : {ALERTS_FILE} missing\n")
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-LATACT-BURN-4x3": {
                    "measured_pass_count": 0,
                    "measured_expected_count": 1,
                    "reason": f"alerts file missing : {ALERTS_FILE}",
                }
            }
        }, indent=2) + "\n")
        return 1

    text = ALERTS_FILE.read_text()
    # Expect each cell to declare a `window: "Xh"` + `severity: ...` label.
    cells_found: set[tuple[str, str]] = set()
    for window in WINDOWS:
        for severity in SEVERITIES:
            # Heuristic : look for both labels in the same context.
            pattern = re.compile(
                rf'window\s*:\s*"?{re.escape(window)}"?[^}}]*severity\s*:\s*"?{re.escape(severity)}"?'
                rf'|severity\s*:\s*"?{re.escape(severity)}"?[^}}]*window\s*:\s*"?{re.escape(window)}"?',
                re.MULTILINE,
            )
            if pattern.search(text):
                cells_found.add((window, severity))

    if args.mode == "synth_fail":
        # Pretend 1 cell missing.
        cells_found.discard(("72h", "watch"))

    n_expected = len(WINDOWS) * len(SEVERITIES)   # 12
    n_found = len(cells_found)
    missing = [
        (w, s) for w in WINDOWS for s in SEVERITIES
        if (w, s) not in cells_found
    ]
    passed = n_found == n_expected

    print(f"Burn 4×3 coverage : {n_found}/{n_expected} cells, "
          f"{len(missing)} missing", file=sys.stderr)
    for m in missing:
        print(f"  ✗ missing : window={m[0]} severity={m[1]}", file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-LATACT-BURN-4x3": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "n_cells_found": n_found,
                "n_cells_expected": n_expected,
                "missing_cells": [
                    {"window": w, "severity": s} for w, s in missing
                ],
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
