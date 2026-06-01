#!/usr/bin/env python3
"""Replay Mode-2 audit chain to verify HMAC integrity.

Sprint **Φ-D-MODE2 Completion (D66)** — artefact #14.
Cf. ``docs/EFE_MODE2_ANCHORS_COMPLETION_ARCHITECTURE.md`` § 12.

## Mission

Read all `Mode2Attestation` records from a tenant's chain, recompute
the HMAC for each link, and verify the chain is intact :
* Every `attestation_N.prev_hash == attestation_N-1.hmac_sha256`.
* Every `attestation_N.hmac_sha256 == HMAC(key, canonical_fields)`.

Outputs SLO compliance JSON for `SLO-MODE2-AUDIT-CHAIN-INTEGRITY`.

## Usage

```bash
# Mock backend (= local file) for dev
python3 scripts/mode2/replay_audit_chain.py \\
    --tenant tenant-acme \\
    --chain-file /tmp/mode2_chain.jsonl \\
    --hmac-key-hex 00112233...

# Real backend (= efe-server audit chain API) — placeholder
python3 scripts/mode2/replay_audit_chain.py \\
    --tenant tenant-acme \\
    --since 2026-05-01T00:00:00Z \\
    --hmac-key-env EFE_MODE2_HMAC_KEY
```

## Output

```json
{
  "slo_compliance": {
    "SLO-MODE2-AUDIT-CHAIN-INTEGRITY": {
      "measured_pass_count": 1,
      "measured_expected_count": 1,
      "chain_length": 1234,
      "broken_links": []
    },
    "SLO-MODE2-AUDIT-COVERAGE": {
      "measured_pass_count": 1,
      "measured_expected_count": 1,
      "expected_attestations": 1234,
      "observed_attestations": 1234
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

# Canonical field order for HMAC computation. MUST match the Rust-side
# Mode2Attestation serialization (= ``crates/efe-server/src/audit/mode2.rs``).
CANONICAL_FIELDS = [
    "schema_version",
    "tenant_id",
    "request_id",
    "prompt_id",
    "prompt_version",
    "prompt_hash",
    "catalog_id",
    "catalog_hash",
    "provider_id",
    "model_version",
    "response_hash",
    "validation_status",
    "cost_usd_micros",
    "tokens_input",
    "tokens_output",
    "latency_ms",
    "fallback_reason",
    "timestamp_unix_micros",
    "prev_hash",
]


def canonical_bytes(attestation: dict) -> bytes:
    """Serialize an attestation to canonical bytes for HMAC computation.

    The format is a strict JSON object with sorted keys + no whitespace,
    excluding the `hmac_sha256` field itself.
    """
    canonical = {k: attestation.get(k) for k in CANONICAL_FIELDS}
    return json.dumps(canonical, sort_keys=True, separators=(",", ":")).encode("utf-8")


def compute_hmac(key: bytes, attestation: dict) -> str:
    """Compute HMAC-SHA256 of an attestation's canonical bytes (hex)."""
    h = hmac.new(key, canonical_bytes(attestation), hashlib.sha256)
    return h.hexdigest()


def load_chain(path: Path) -> list[dict]:
    """Load a chain from a JSONL file (= one attestation per line)."""
    chain = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        chain.append(json.loads(line))
    return chain


def replay(chain: list[dict], hmac_key: bytes) -> dict:
    """Replay the chain and verify each link. Returns a diagnostic dict."""
    broken: list[str] = []
    previous_hmac = "00" * 32  # genesis : 32 zero bytes
    for i, attestation in enumerate(chain):
        # Check prev_hash link.
        actual_prev = attestation.get("prev_hash", "")
        if actual_prev != previous_hmac:
            broken.append(
                f"index={i} req={attestation.get('request_id')} "
                f"prev_hash mismatch : got {actual_prev[:16]}..., "
                f"expected {previous_hmac[:16]}..."
            )
        # Check HMAC.
        expected_hmac = compute_hmac(hmac_key, attestation)
        actual_hmac = attestation.get("hmac_sha256", "")
        if expected_hmac != actual_hmac:
            broken.append(
                f"index={i} req={attestation.get('request_id')} "
                f"HMAC mismatch : got {actual_hmac[:16]}..., "
                f"expected {expected_hmac[:16]}..."
            )
        previous_hmac = actual_hmac

    return {
        "chain_length": len(chain),
        "broken_links": broken[:20],  # cap output
        "total_broken": len(broken),
    }


def main() -> int:
    ap = argparse.ArgumentParser(description="Mode-2 audit chain replay verifier")
    ap.add_argument("--tenant", required=True, help="Tenant ID to replay")
    ap.add_argument("--chain-file", type=Path,
                    help="Local JSONL file of attestations (= mock mode)")
    ap.add_argument("--since", help="ISO8601 start timestamp (= live mode, NOT YET WIRED)")
    ap.add_argument("--hmac-key-hex",
                    help="HMAC key as hex string (DEV ONLY ; use --hmac-key-env in prod)")
    ap.add_argument("--hmac-key-env", default="EFE_MODE2_HMAC_KEY",
                    help="Environment variable containing the hex HMAC key")
    ap.add_argument("--out", type=Path,
                    default=Path("target/mode2/audit_replay.json"))
    args = ap.parse_args()

    # Load HMAC key.
    if args.hmac_key_hex:
        hmac_key = bytes.fromhex(args.hmac_key_hex)
    else:
        env_val = os.environ.get(args.hmac_key_env)
        if not env_val:
            sys.stderr.write(
                f"FATAL : HMAC key not found in env {args.hmac_key_env} "
                f"and --hmac-key-hex not given.\n"
            )
            return 2
        hmac_key = bytes.fromhex(env_val)

    # Load chain.
    if args.chain_file:
        if not args.chain_file.exists():
            sys.stderr.write(f"FATAL : chain file missing : {args.chain_file}\n")
            return 2
        chain = load_chain(args.chain_file)
    else:
        # Live mode placeholder.
        sys.stderr.write(
            "WARN : live mode (= fetch from efe-server) not yet wired. "
            "Use --chain-file for now.\n"
        )
        chain = []

    # Filter by tenant.
    chain = [a for a in chain if a.get("tenant_id") == args.tenant]
    print(f"Replaying {len(chain)} attestations for tenant {args.tenant}",
          file=sys.stderr)

    result = replay(chain, hmac_key)
    passed = result["total_broken"] == 0
    coverage_pass = len(chain) > 0  # SLO-MODE2-AUDIT-COVERAGE : at least 1 attestation

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(
        json.dumps(
            {
                "slo_compliance": {
                    "SLO-MODE2-AUDIT-CHAIN-INTEGRITY": {
                        "measured_pass_count": 1 if passed else 0,
                        "measured_expected_count": 1,
                        "chain_length": result["chain_length"],
                        "total_broken": result["total_broken"],
                        "broken_links": result["broken_links"],
                    },
                    "SLO-MODE2-AUDIT-COVERAGE": {
                        "measured_pass_count": 1 if coverage_pass else 0,
                        "measured_expected_count": 1,
                        "observed_attestations": len(chain),
                    },
                }
            },
            indent=2,
        )
        + "\n"
    )

    if not passed:
        print(
            f"\nFAIL : {result['total_broken']} broken links detected :",
            file=sys.stderr,
        )
        for b in result["broken_links"]:
            print(f"  - {b}", file=sys.stderr)
        return 1

    print(
        f"\n✓ Chain integrity verified : {len(chain)} attestations, 0 broken links",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
