#!/usr/bin/env python3
"""
Sprint Φ-D-OPS (D97) — Runbook auto-link audit.

Walks `bench_baselines/v0.9.0/ops_master_slos.yaml` and every alert in
`deploy/observability/prometheus/rules.ops/*.yaml`, and asserts that
the path under `runbook:` resolves to an existing file under
`docs/runbooks/` (or `docs/postmortems/`).

This is the doctrine D-OPS-7 enforcement bench :
"every SLO has a linked runbook ; every alert has a linked runbook".

Emits a JSON measurement payload and exits non-zero on missing files.

Cf. `docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md` § 10.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Dict, List, Optional, Set

try:
    import yaml  # type: ignore
except ImportError:
    yaml = None  # type: ignore


REPO_ROOT = Path(__file__).resolve().parents[2]


def _collect_master_runbooks(path: Path) -> Set[str]:
    bundle = yaml.safe_load(path.read_text(encoding="utf-8"))
    refs = set()
    for slo in bundle["spec"]["slos"]:
        rb = slo.get("runbook")
        if rb:
            refs.add(rb)
    return refs


def _collect_alert_runbooks(rules_dir: Path) -> Set[str]:
    refs = set()
    for f in rules_dir.glob("*.yaml"):
        bundle = yaml.safe_load(f.read_text(encoding="utf-8"))
        for group in bundle.get("groups", []):
            for rule in group.get("rules", []):
                if "alert" in rule:
                    ann = rule.get("annotations", {})
                    rb = ann.get("runbook")
                    if rb:
                        refs.add(rb)
                    rb2 = rule.get("labels", {}).get("runbook")
                    if rb2:
                        refs.add(rb2)
    return refs


def main(argv: Optional[List[str]] = None) -> int:
    p = argparse.ArgumentParser(description="Runbook autolink bench")
    p.add_argument("--master",
                   default="bench_baselines/v0.9.0/ops_master_slos.yaml")
    p.add_argument("--rules-dir",
                   default="deploy/observability/prometheus/rules.ops")
    p.add_argument("--out", type=Path,
                   default=Path("target/ops/measures.runbook_links.json"))
    args = p.parse_args(argv)

    if yaml is None:
        sys.exit("missing pyyaml")

    master_runbooks = _collect_master_runbooks(REPO_ROOT / args.master)
    alert_runbooks = _collect_alert_runbooks(REPO_ROOT / args.rules_dir)
    all_runbooks = master_runbooks | alert_runbooks

    missing: List[str] = []
    found:   List[str] = []
    for rb in sorted(all_runbooks):
        if (REPO_ROOT / rb).exists():
            found.append(rb)
        else:
            missing.append(rb)

    payload = {
        "SLO-OPS-RUNBOOK-FRESHNESS": len(missing),
        "_meta": {
            "master_refs": len(master_runbooks),
            "alert_refs":  len(alert_runbooks),
            "total_refs":  len(all_runbooks),
            "missing":     missing,
        },
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    print(f"runbook autolink → {args.out}")
    print(f"  refs total : {len(all_runbooks)}")
    print(f"  found      : {len(found)}")
    print(f"  missing    : {len(missing)}")
    for m in missing:
        print(f"    ✗ {m}")
    return 0 if not missing else 1


if __name__ == "__main__":
    sys.exit(main())
