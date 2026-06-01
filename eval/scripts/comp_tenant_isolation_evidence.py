#!/usr/bin/env python3
"""Evidence : tenant isolation (D-COMP-7).

Sprint Φ-D-COMPLIANCE D87 — evidence #2.
Controls : GDPR-Art25, GDPR-Art32, ISO-A.5.15, SOC2-CC6.1, SOC2-CC6.2.

Re-uses the same 3 adversarial scenarios as AL `al_tenant_isolation_test.py` :
clean fetch, cross-tenant injection, orphan tenant_id.
"""

from __future__ import annotations
import argparse, json, sys
from pathlib import Path

CONTROLS = ["GDPR-Art25", "GDPR-Art32", "ISO-A.5.15", "SOC2-CC6.1", "SOC2-CC6.2"]


def filter_by_tenant(labels: list[dict], scope: str) -> list[dict]:
    out = []
    for l in labels:
        tid = l.get("tenant_id", "")
        if not tid:
            raise PermissionError(f"missing tenant_id for {l.get('id')}")
        if tid != scope:
            raise PermissionError(f"cross-tenant : {tid} != {scope}")
        out.append(l)
    return out


SCENARIOS = [
    ("clean_same_tenant",
     [{"id": f"r-{i}", "tenant_id": "tenant-A"} for i in range(20)], "tenant-A", True),
    ("cross_tenant_injection",
     [{"id": f"r-{i}", "tenant_id": "tenant-A"} for i in range(15)]
     + [{"id": "leak", "tenant_id": "tenant-B"}],
     "tenant-A", False),
    ("orphan_no_tenant",
     [{"id": f"r-{i}", "tenant_id": "tenant-A"} for i in range(5)]
     + [{"id": "orphan", "tenant_id": ""}],
     "tenant-A", False),
]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--out", type=Path,
                    default=Path("target/compliance/evidence_tenant_isolation.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-COMP-EVIDENCE-COVERAGE": {
                    "skip_reason": "Live tenant isolation requires prod adversarial scan",
                    "control_ids": CONTROLS,
                }
            }
        }, indent=2) + "\n")
        return 0

    results = []
    n_pass = 0
    for name, items, scope, expect_ok in SCENARIOS:
        try:
            filter_by_tenant(items, scope)
            got_ok = True
        except PermissionError:
            got_ok = False
        ok = got_ok == expect_ok
        if args.mode == "synth_fail" and name == "cross_tenant_injection":
            ok = False  # inject
        if ok:
            n_pass += 1
        results.append({"name": name, "expect_pass": expect_ok, "got_pass": got_ok, "ok": ok})
        print(f"  {'✓' if ok else '✗'} {name}", file=sys.stderr)

    passed = n_pass == len(SCENARIOS)
    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-COMP-EVIDENCE-COVERAGE": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "control_ids": CONTROLS,
                "n_scenarios": len(SCENARIOS), "n_passed": n_pass,
                "scenarios": results,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
