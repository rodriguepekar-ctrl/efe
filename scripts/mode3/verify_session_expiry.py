#!/usr/bin/env python3
"""Verify Mode-3 session TTL ≤ 24h enforced.

Sprint **Φ-D-MODE3 Completion (D72)** — artefact #16.
Cf. ``docs/EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md`` § 6.1.

For every session, verify :
  expires_at_unix_micros - created_at_unix_micros ≤ 24 × 3600 × 1e6
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

TTL_MAX_MICROS = 24 * 3600 * 1_000_000  # 24 hours


def main() -> int:
    ap = argparse.ArgumentParser(description="Mode-3 session TTL audit")
    ap.add_argument("--sessions-list", type=Path)
    ap.add_argument("--out", type=Path,
                    default=Path("target/mode3/session_expiry.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if not args.sessions_list:
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-MODE3-SESSION-EXPIRY": {
                    "skip_reason": "sessions-list required (DT-MODE3-SESSIONS-DUMP)"
                }
            }
        }, indent=2) + "\n")
        return 0

    sessions = json.loads(args.sessions_list.read_text())
    if isinstance(sessions, dict):
        sessions = sessions.get("sessions", [])

    violations = []
    for s in sessions:
        created = s.get("created_at_unix_micros", 0)
        expires = s.get("expires_at_unix_micros", 0)
        delta = expires - created
        if delta > TTL_MAX_MICROS:
            violations.append(
                f"{s.get('session_id')} : TTL = {delta/1e6:.1f}s > {TTL_MAX_MICROS/1e6:.0f}s"
            )

    passed = len(violations) == 0
    print(f"Session TTL audit : {len(sessions)} sessions, {len(violations)} violations",
          file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-MODE3-SESSION-EXPIRY": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "sessions_audited": len(sessions),
                "violations": violations[:20],
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
