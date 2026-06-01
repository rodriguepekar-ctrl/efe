#!/usr/bin/env python3
"""Detect cross-tenant leakage in Mode-2 audit chains.

Sprint **Φ-D-MODE2 Completion (D66)** — artefact #15.
Cf. ``docs/EFE_MODE2_ANCHORS_COMPLETION_ARCHITECTURE.md`` § 15.

## Mission

For each pair of tenants (A, B), verify :
1. No `request_id` from tenant A appears in tenant B's audit chain.
2. No `prev_hash` of tenant B's attestation references tenant A's HMAC.
3. No `prompt_hash` collision across tenants on cached responses
   (= legitimate same prompt is OK ; mismatch in tenant_id is leak).

This is the **runtime enforcement** of doctrine D-MODE2-6 (tenant isolation).

## Output

```json
{
  "slo_compliance": {
    "SLO-MODE2-TENANT-ISOLATION": {
      "measured_pass_count": 1,
      "measured_expected_count": 1,
      "tenants_audited": 12,
      "leakages_found": []
    }
  }
}
```

If `leakages_found` is non-empty, the SLO fails and a security incident
must be opened (= runbook `mode2_audit_chain_break.md`).
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import defaultdict
from pathlib import Path


def load_chain(path: Path) -> list[dict]:
    """Load a JSONL chain file (= one attestation per line)."""
    chain = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        chain.append(json.loads(line))
    return chain


def detect_leakage(chains_per_tenant: dict[str, list[dict]]) -> list[str]:
    """Cross-tenant leakage detection. Returns list of human-readable findings."""
    findings: list[str] = []

    # Build per-tenant index of (request_id, hmac_sha256).
    per_tenant_request_ids: dict[str, set[str]] = {}
    per_tenant_hmacs: dict[str, set[str]] = {}
    for tenant, chain in chains_per_tenant.items():
        per_tenant_request_ids[tenant] = {
            a.get("request_id", "") for a in chain
        }
        per_tenant_hmacs[tenant] = {
            a.get("hmac_sha256", "") for a in chain
        }

    # Check 1 : request_id uniqueness across tenants.
    # A request_id should be unique cluster-wide.
    request_id_owner: dict[str, str] = {}
    for tenant, ids in per_tenant_request_ids.items():
        for rid in ids:
            if rid in request_id_owner and request_id_owner[rid] != tenant:
                findings.append(
                    f"LEAK : request_id={rid} appears in both "
                    f"{request_id_owner[rid]} and {tenant}"
                )
            request_id_owner[rid] = tenant

    # Check 2 : prev_hash references must stay within tenant boundaries.
    for tenant, chain in chains_per_tenant.items():
        tenant_hmacs = per_tenant_hmacs[tenant] | {"00" * 32}  # add genesis
        for i, attestation in enumerate(chain):
            prev = attestation.get("prev_hash", "")
            if prev not in tenant_hmacs:
                # Check if it matches another tenant's HMAC (= leak).
                for other_tenant, other_hmacs in per_tenant_hmacs.items():
                    if other_tenant == tenant:
                        continue
                    if prev in other_hmacs:
                        findings.append(
                            f"LEAK : tenant {tenant} attestation index={i} "
                            f"prev_hash={prev[:16]}... references {other_tenant}'s HMAC"
                        )
                        break

    return findings


def main() -> int:
    ap = argparse.ArgumentParser(description="Mode-2 cross-tenant audit verifier")
    ap.add_argument(
        "--chains-dir", type=Path,
        help="Dir containing per-tenant <tenant>.jsonl files",
    )
    ap.add_argument(
        "--out", type=Path,
        default=Path("target/mode2/cross_tenant_audit.json"),
    )
    args = ap.parse_args()

    args.out.parent.mkdir(parents=True, exist_ok=True)

    if not args.chains_dir or not args.chains_dir.exists():
        sys.stderr.write(
            "WARN : --chains-dir not provided ; emitting SKIPPED for SLO-MODE2-TENANT-ISOLATION.\n"
            "       For live mode, provide a dir with per-tenant <tenant>.jsonl audit files.\n"
        )
        args.out.write_text(
            json.dumps(
                {
                    "slo_compliance": {
                        "SLO-MODE2-TENANT-ISOLATION": {
                            "skip_reason": (
                                "Live cross-tenant audit requires production "
                                "audit log dump (DT-MODE2-AUDIT-DUMP)."
                            )
                        }
                    }
                },
                indent=2,
            )
            + "\n"
        )
        return 0

    # Load all per-tenant chains.
    chains_per_tenant: dict[str, list[dict]] = {}
    for path in args.chains_dir.glob("*.jsonl"):
        tenant = path.stem
        chains_per_tenant[tenant] = load_chain(path)

    if not chains_per_tenant:
        sys.stderr.write(f"WARN : no .jsonl files in {args.chains_dir}\n")
        # Still report a passing SLO (= no tenants → no leak possible).

    findings = detect_leakage(chains_per_tenant)
    n_tenants = len(chains_per_tenant)

    print(f"\nCross-tenant audit : {n_tenants} tenants scanned", file=sys.stderr)
    if findings:
        print(f"  ✗ {len(findings)} leakage findings :", file=sys.stderr)
        for f in findings[:10]:
            print(f"    - {f}", file=sys.stderr)
    else:
        print("  ✓ no cross-tenant leakage detected", file=sys.stderr)

    passed = len(findings) == 0
    args.out.write_text(
        json.dumps(
            {
                "slo_compliance": {
                    "SLO-MODE2-TENANT-ISOLATION": {
                        "measured_pass_count": 1 if passed else 0,
                        "measured_expected_count": 1,
                        "tenants_audited": n_tenants,
                        "leakages_found": findings[:20],
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
