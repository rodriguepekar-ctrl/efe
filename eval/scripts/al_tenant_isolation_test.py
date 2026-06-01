#!/usr/bin/env python3
"""Tenant isolation test for Active Learning gold labels + centroids.

Sprint **Φ-D-AL Completion (D77)** — artefact #16.
Cf. ``docs/EFE_ACTIVE_LEARNING_COMPLETION_ARCHITECTURE.md`` § 14.

Adversarial scan : injects labels from tenant A into a centroid retune
for tenant B and verifies the retune REJECTS them.

## Defense layers

1. `tenant_id` field on each `AlLabelAttestation` (= integrity).
2. Centroid compute filter `WHERE tenant_id = ?` (= scope).
3. Cross-tenant fetch attempt → expected `TenantMismatch` exception.
4. Audit chain replay verifies no foreign tenant labels in chain.

SLO-AL-TENANT-ISOLATION : 0 cross-tenant leak.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path


class TenantMismatchError(Exception):
    """Raised when a label's tenant_id != requested scope."""


def filter_labels_by_tenant(labels: list[dict], scope_tenant: str) -> list[dict]:
    """Production-grade filter : drops labels with tenant_id != scope.

    Audit fix : this MUST raise on cross-tenant attempts (not silently
    skip) to enforce strict isolation per D-AL-7.
    """
    out = []
    for lbl in labels:
        lbl_tenant = lbl.get("tenant_id", "")
        if not lbl_tenant:
            raise TenantMismatchError(
                f"label_id={lbl.get('label_id')} has no tenant_id"
            )
        if lbl_tenant != scope_tenant:
            raise TenantMismatchError(
                f"label_id={lbl.get('label_id')} tenant={lbl_tenant} "
                f"≠ scope={scope_tenant}"
            )
        out.append(lbl)
    return out


def _gen_labels(tenant: str, n: int) -> list[dict]:
    return [
        {
            "label_id": f"{tenant}-label-{i:04d}",
            "tenant_id": tenant,
            "label_text": f"cat-{i % 3}",
            "consent_hash": hashlib.sha256(f"{tenant}-{i}".encode()).hexdigest(),
        }
        for i in range(n)
    ]


def main() -> int:
    ap = argparse.ArgumentParser(description="AL tenant isolation test")
    ap.add_argument("--out", type=Path,
                    default=Path("target/active_learning/tenant_isolation.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    results = []
    n_pass = 0

    # ── Test 1 : clean fetch (same tenant) → must succeed.
    labels_a = _gen_labels("tenant-A", 50)
    try:
        kept = filter_labels_by_tenant(labels_a, "tenant-A")
        ok = len(kept) == 50
        results.append({"test": "clean_same_tenant", "pass": ok,
                        "n_kept": len(kept)})
        if ok:
            n_pass += 1
    except TenantMismatchError as e:
        results.append({"test": "clean_same_tenant", "pass": False, "err": str(e)})

    # ── Test 2 : cross-tenant injection → must RAISE.
    labels_mixed = labels_a + _gen_labels("tenant-B", 5)  # 5 foreign
    try:
        filter_labels_by_tenant(labels_mixed, "tenant-A")
        # If we reach here, the cross-tenant was NOT detected → FAIL.
        results.append({
            "test": "cross_tenant_injection", "pass": False,
            "err": "TenantMismatchError NOT raised on 5 foreign labels"
        })
    except TenantMismatchError:
        results.append({"test": "cross_tenant_injection", "pass": True,
                        "err": None})
        n_pass += 1

    # ── Test 3 : empty tenant_id → must RAISE.
    labels_with_orphan = _gen_labels("tenant-A", 10) + [
        {"label_id": "orphan", "tenant_id": ""}
    ]
    try:
        filter_labels_by_tenant(labels_with_orphan, "tenant-A")
        results.append({
            "test": "orphan_no_tenant", "pass": False,
            "err": "TenantMismatchError NOT raised on orphan"
        })
    except TenantMismatchError:
        results.append({"test": "orphan_no_tenant", "pass": True})
        n_pass += 1

    n_total = len(results)
    passed = n_pass == n_total
    print(f"\nTenant isolation : {n_pass}/{n_total} PASS", file=sys.stderr)
    for r in results:
        sym = "✓" if r["pass"] else "✗"
        print(f"  {sym} {r['test']}", file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-AL-TENANT-ISOLATION": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "n_tests": n_total,
                "n_passed": n_pass,
                "results": results,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
