#!/usr/bin/env python3
"""Verify canary duration ≥ 24h before promotion.

Sprint **Φ-D-AL Completion (D77)** — artefact #17.

Audit Level 1 fixes (Φ-D-AL.A1 + Φ-D-AL.A2) :
* **A1** : pre-fix le code exposait `measured_seconds = p99` (= max-leaning) ;
  pour un SLO `gte 86400s`, le MAX est l'angle facile à passer alors qu'une
  promotion individuelle peut être < 24h. Faussement-PASS résolu en exposant
  `measured_seconds = MIN(durations)` (= pire cas, doit ≥ threshold).
* **A2** : pre-fix quand `len(promoted) == 0`, le script retournait
  `measured_seconds = 0` avec exit 0 (= `violations == []`) → check_slo.py
  lisait 0 < 86400 → SLO **FAIL** ambigu (sans evidence). Post-fix : émettre
  `skip_reason` explicite quand aucune evidence (anti-Φ-D.A1).
"""

from __future__ import annotations
import argparse, json, sys
from pathlib import Path

MIN_CANARY_SECONDS = 86400  # 24h


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--retune-attestations", type=Path)
    ap.add_argument("--out", type=Path,
                    default=Path("target/active_learning/canary_duration.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if not args.retune_attestations:
        sys.stderr.write("WARN : --retune-attestations required ; emitting SKIPPED.\n")
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-AL-CANARY-DURATION": {
                    "skip_reason": "Live retune attestations required (DT-AL-RETUNE-DUMP)"
                }
            }
        }, indent=2) + "\n")
        return 0

    atts = json.loads(args.retune_attestations.read_text())
    if isinstance(atts, dict):
        atts = atts.get("attestations", [])

    promoted_durations = []
    violations = []
    for a in atts:
        if a.get("promotion_decision") != "promoted":
            continue
        start = a.get("canary_started_at", 0)
        end = a.get("canary_ended_at", 0)
        duration = (end - start) / 1e6  # micros → seconds
        promoted_durations.append(duration)
        if duration < MIN_CANARY_SECONDS:
            violations.append(
                f"{a.get('catalog_id', '?')}: canary {duration:.0f}s < {MIN_CANARY_SECONDS}s"
            )

    # **Audit fix Φ-D-AL.A2** — anti-Φ-D.A1 : si aucune evidence
    # (= aucune promotion dans le dump), on doit SKIP, pas FAIL/PASS.
    if not promoted_durations:
        sys.stderr.write(
            f"WARN : no promoted retune in {len(atts)} attestations ; "
            "emitting SKIPPED (no evidence).\n"
        )
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-AL-CANARY-DURATION": {
                    "skip_reason": (
                        f"no promoted retune found in {len(atts)} attestations "
                        "(= no canary duration evidence ; anti-Φ-D.A1)"
                    ),
                    "n_attestations_seen": len(atts),
                }
            }
        }, indent=2) + "\n")
        return 0

    # **Audit fix Φ-D-AL.A1** — emit MIN duration as `measured_seconds`
    # because the SLO is `gte 86400s`. The minimum is the worst case
    # (= smallest canary window) and must still ≥ threshold.
    min_duration = min(promoted_durations)
    max_duration = max(promoted_durations)
    passed = len(violations) == 0

    print(f"Canary duration : {len(promoted_durations)} promotions, "
          f"min={min_duration:.0f}s max={max_duration:.0f}s, "
          f"{len(violations)} violations", file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-AL-CANARY-DURATION": {
                "measured_seconds": min_duration,
                "max_seconds": max_duration,
                "n_promotions": len(promoted_durations),
                "n_violations": len(violations),
                "violations": violations[:20],
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
