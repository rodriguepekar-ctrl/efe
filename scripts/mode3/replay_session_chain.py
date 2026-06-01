#!/usr/bin/env python3
"""Replay Mode-3 session HMAC chain to verify per-session integrity.

Sprint **Φ-D-MODE3 Completion (D71)** — artefact #15.
Cf. ``docs/EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md`` § 10.

## Mission

For each Mode-3 session, replay the HMAC chain of its attestations
(= Mode3Attestation records, one per turn) and verify :
  1. Each `attestation.prev_hash` matches the previous turn's `hmac_sha256`.
  2. Each `attestation.hmac_sha256` matches the canonical HMAC.
  3. The first turn's `prev_hash` is the genesis (= "00"*32) OR
     references a parent session chain.

## Output

```json
{
  "slo_compliance": {
    "SLO-MODE3-AUDIT-CHAIN-INTEGRITY": {
      "measured_pass_count": 1,
      "measured_expected_count": 1,
      "sessions_audited": 42,
      "broken_sessions": []
    }
  }
}
```
"""

from __future__ import annotations

import argparse
import hashlib
import hmac
import json
import os
import sys
from pathlib import Path

# Canonical fields for Mode3Attestation HMAC (must match Rust serialization).
CANONICAL_FIELDS = [
    "schema_version",
    "tenant_id",
    "request_id",
    "timestamp_unix_micros",
    "session_id",
    "turn_index",
    "consent_hash",
    "parent_anchor_hash",
    "refined_anchor_hash",
    "convergence_score",
    "convergence_status",
    "cost_usd_micros",
    "turn_latency_ms",
    "human_review_status",
    "fallback_reason",
    "context_tokens",
    "summarization_applied",
    "prev_hash",
]


def canonical_bytes(attestation: dict) -> bytes:
    canonical = {k: attestation.get(k) for k in CANONICAL_FIELDS}
    return json.dumps(canonical, sort_keys=True, separators=(",", ":")).encode("utf-8")


def compute_hmac(key: bytes, attestation: dict) -> str:
    h = hmac.new(key, canonical_bytes(attestation), hashlib.sha256)
    return h.hexdigest()


def replay_session(
    attestations: list[dict], hmac_key: bytes
) -> tuple[bool, list[str]]:
    """Replay one session's chain. Returns (passed, list of issues)."""
    if not attestations:
        return True, []
    issues: list[str] = []
    # Sort by turn_index (defensive).
    attestations = sorted(attestations, key=lambda a: a.get("turn_index", 0))
    previous_hmac = "00" * 32  # genesis
    for att in attestations:
        actual_prev = att.get("prev_hash", "")
        if actual_prev != previous_hmac:
            issues.append(
                f"turn={att.get('turn_index')} prev_hash mismatch : "
                f"got {actual_prev[:16]}..., expected {previous_hmac[:16]}..."
            )
        expected_hmac = compute_hmac(hmac_key, att)
        actual_hmac = att.get("hmac_sha256", "")
        if expected_hmac != actual_hmac:
            issues.append(
                f"turn={att.get('turn_index')} HMAC mismatch"
            )
        previous_hmac = actual_hmac
    return len(issues) == 0, issues


def main() -> int:
    ap = argparse.ArgumentParser(description="Mode-3 session chain replay")
    ap.add_argument("--chain-file", type=Path,
                    help="JSONL file of Mode3Attestations (= one per line)")
    ap.add_argument("--hmac-key-hex",
                    help="HMAC key as hex (DEV ONLY)")
    ap.add_argument("--hmac-key-env", default="EFE_MODE3_HMAC_KEY")
    ap.add_argument("--out", type=Path,
                    default=Path("target/mode3/audit_replay.json"))
    args = ap.parse_args()

    args.out.parent.mkdir(parents=True, exist_ok=True)

    if not args.chain_file:
        sys.stderr.write(
            "WARN : --chain-file required ; emitting SKIPPED.\n"
        )
        args.out.write_text(
            json.dumps(
                {
                    "slo_compliance": {
                        "SLO-MODE3-AUDIT-CHAIN-INTEGRITY": {
                            "skip_reason": (
                                "Live audit chain dump required "
                                "(DT-MODE3-AUDIT-DUMP)."
                            )
                        }
                    }
                },
                indent=2,
            )
            + "\n"
        )
        return 0

    if not args.chain_file.exists():
        sys.stderr.write(f"FATAL : chain file missing : {args.chain_file}\n")
        return 2

    # Load HMAC key.
    if args.hmac_key_hex:
        key = bytes.fromhex(args.hmac_key_hex)
    else:
        env_val = os.environ.get(args.hmac_key_env)
        if not env_val:
            sys.stderr.write(f"FATAL : HMAC key not provided\n")
            return 2
        key = bytes.fromhex(env_val)

    # Load + group attestations by session_id.
    attestations_per_session: dict[str, list[dict]] = {}
    for line in args.chain_file.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        att = json.loads(line)
        sid = att.get("session_id", "<no-session>")
        attestations_per_session.setdefault(sid, []).append(att)

    n_sessions = len(attestations_per_session)
    broken: list[str] = []
    for sid, atts in attestations_per_session.items():
        ok, issues = replay_session(atts, key)
        if not ok:
            broken.append(f"session={sid} : {len(issues)} issues")

    passed = len(broken) == 0
    print(
        f"\nSession chain replay : {n_sessions} sessions, {len(broken)} broken",
        file=sys.stderr,
    )
    if broken:
        for b in broken[:10]:
            print(f"  - {b}", file=sys.stderr)

    args.out.write_text(
        json.dumps(
            {
                "slo_compliance": {
                    "SLO-MODE3-AUDIT-CHAIN-INTEGRITY": {
                        "measured_pass_count": 1 if passed else 0,
                        "measured_expected_count": 1,
                        "sessions_audited": n_sessions,
                        "broken_sessions": broken[:20],
                    }
                }
            },
            indent=2,
        )
        + "\n"
    )

    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
