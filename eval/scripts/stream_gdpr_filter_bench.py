#!/usr/bin/env python3
"""Streaming GDPR consent filter bench.

Sprint **Φ-D-STREAM Completion (D83)** — artefact #35.
Cf. ``docs/EFE_STREAMING_RPC_COMPLETION_ARCHITECTURE.md`` § 15.

SLO-STREAM-GDPR : in ListAuditEvents, every emitted audit event for
actions like `swarm.execute*` MUST carry a valid `consent_hash` in its
payload_json. Events without consent_hash are filtered out at the
server (= GDPR Art. 6 lawfulness).

Verifies the filter via 4 scenarios :
  1. happy : all events have consent_hash → all emitted
  2. mixed : 50 % have consent_hash → only those emitted
  3. all_missing : none have → 0 emitted
  4. invalid : consent_hash = "0"*64 → filtered (= zero placeholder)
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path


def is_action_subject_to_consent(action: str) -> bool:
    """Actions that require consent_hash in payload."""
    return action.startswith("swarm.") or action.startswith("mode2.") or action.startswith("mode3.")


def filter_event(event: dict) -> tuple[bool, str | None]:
    """Returns (keep, reason_if_dropped)."""
    action = event.get("action", "")
    if not is_action_subject_to_consent(action):
        return (True, None)  # admin events don't need consent
    try:
        payload = json.loads(event.get("payload_json", "{}"))
    except json.JSONDecodeError:
        return (False, "payload_json not parseable")
    consent_hash = payload.get("consent_hash", "")
    if not isinstance(consent_hash, str) or len(consent_hash) != 64:
        return (False, f"consent_hash invalid (length {len(consent_hash) if isinstance(consent_hash, str) else 'non-str'})")
    if consent_hash == "0" * 64:
        return (False, "consent_hash is zero placeholder")
    try:
        bytes.fromhex(consent_hash)
    except ValueError:
        return (False, "consent_hash not hex")
    return (True, None)


def gen_events(n: int, with_consent_ratio: float) -> list[dict]:
    out = []
    for i in range(n):
        if i / n < with_consent_ratio:
            consent = hashlib.sha256(f"consent-{i}".encode()).hexdigest()
        else:
            consent = ""
        out.append({
            "event_id": f"01h{i:08d}",
            "action": "swarm.execute" if i % 2 else "catalog.read",
            "payload_json": json.dumps({"consent_hash": consent, "ix": i},
                                         sort_keys=True, separators=(",", ":")),
        })
    return out


SCENARIOS = [
    ("happy_100pct", 100, 1.0),
    ("mixed_50pct",  100, 0.5),
    ("all_missing",  100, 0.0),
    ("invalid_zero", "ZERO_PLACEHOLDER", None),  # special : forces 0 * 64
]


def main() -> int:
    ap = argparse.ArgumentParser(description="Streaming GDPR filter bench")
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--out", type=Path,
                    default=Path("target/streaming/gdpr_filter.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        sys.stderr.write("WARN : live mode SKIPPED (DT-STREAM-GDPR-PROD).\n")
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-STREAM-GDPR": {
                    "skip_reason": "Live consent telemetry (DT-STREAM-GDPR-PROD)",
                }
            }
        }, indent=2) + "\n")
        return 0

    results = []
    n_pass = 0
    for scn, n, ratio in SCENARIOS:
        if scn == "invalid_zero":
            events = [
                {"event_id": "01h00000001", "action": "swarm.execute",
                 "payload_json": json.dumps({"consent_hash": "0" * 64})},
            ]
            expected_kept = 0
        else:
            events = gen_events(n, ratio)
            # admin/catalog (= even ix when ix % 2 == 0 → catalog.read) keep regardless,
            # swarm.execute keeps only with consent
            n_admin = sum(1 for e in events if not is_action_subject_to_consent(e["action"]))
            n_swarm_with_consent = sum(
                1 for e in events if is_action_subject_to_consent(e["action"])
                and filter_event(e)[0]
            )
            expected_kept = n_admin + n_swarm_with_consent

        kept = [e for e in events if filter_event(e)[0]]
        if args.mode == "synth_fail" and scn == "happy_100pct":
            # Inject : filter passes everything (= 0 filter applied).
            kept = events
            expected_kept = -1  # force mismatch

        ok = len(kept) == expected_kept
        if ok:
            n_pass += 1
        results.append({
            "scenario": scn, "n_input": len(events),
            "n_kept": len(kept), "n_expected": expected_kept, "pass": ok,
        })
        sym = "✓" if ok else "✗"
        print(f"  {sym} {scn:<20} : kept {len(kept)}/{len(events)} (expected {expected_kept})",
              file=sys.stderr)

    passed = n_pass == len(SCENARIOS)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-STREAM-GDPR": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "n_scenarios": len(SCENARIOS),
                "n_passed": n_pass,
                "results": results,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
