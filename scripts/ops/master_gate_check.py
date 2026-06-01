#!/usr/bin/env python3
"""
Sprint Φ-D-OPS (D95) — Master gate check.

Consumes `target/ops/master_report.json` produced by
`consolidate_all_slos.py` and decides whether the master gate PASSES.

Rules :
  - **Any FAIL    → exit 1.** Hard gate, cannot be overridden in CI.
  - **Any SILENT  → exit 1.** Doctrine anti-Φ-D.A1 : a SLO defined and
    not measured is a build-breaker regardless of severity. Use
    `--tolerate-skip-with-dt` to allow waived skips.
  - **PASS+SKIP-DT → exit 0** when `--tolerate-skip-with-dt` is set.
  - **All PASS    → exit 0.**

The script is deterministic, has no network dependencies, and is
reproducible offline.

Cf. `docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md` § 8.2.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any, Dict, List


_FAIL = "FAIL"
_PASS = "PASS"
_SKIP_DT = "SKIP-DT"
_SILENT = "SKIP-SILENT"


def _load(path: Path) -> Dict[str, Any]:
    if not path.exists():
        sys.stderr.write(f"missing report: {path}\n")
        sys.exit(2)
    return json.loads(path.read_text(encoding="utf-8"))


def _print_offenders(master: List[Dict[str, Any]],
                     status: str,
                     header: str) -> None:
    rows = [m for m in master if m.get("rolled_status") == status]
    if not rows:
        return
    print(f"\n=== {header} ({len(rows)}) ===")
    for m in rows:
        ex = ", ".join(m.get("fail_examples", [])) or "—"
        print(f"  - {m['id']:<35} severity={m.get('severity','?'):<8} "
              f"children={m.get('children_count',0):>2}  fail_examples={ex}")


def main(argv: List[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Master gate hard check.")
    parser.add_argument("--report",
                        default="target/ops/master_report.json",
                        help="JSON report produced by consolidate_all_slos.py")
    parser.add_argument("--tolerate-skip-with-dt", action="store_true",
                        help="Allow SLO rows skipped under a deferred ticket.")
    parser.add_argument("--no-silent-skip", action="store_true", default=True,
                        help="Reject silent skips (default ON).")
    parser.add_argument("--max-fail", type=int, default=0,
                        help="Max FAIL count tolerated (default 0).")
    parser.add_argument("--fail-on-fail", action="store_true", default=True,
                        help="Exit non-zero on FAIL (default ON).")
    args = parser.parse_args(argv)

    payload = _load(Path(args.report))
    master = payload.get("master", [])
    counts = payload.get("counts", {})

    fail = int(counts.get("fail", 0))
    silent = int(counts.get("silent_skip", 0))
    skip_dt = int(counts.get("skip_dt", 0))
    ok = int(counts.get("pass", 0))
    total = len(master)

    print("=" * 66)
    print(f" Φ-D-OPS master gate — {total} master SLOs")
    print("=" * 66)
    print(f"  PASS         : {ok}")
    print(f"  FAIL         : {fail}")
    print(f"  SKIP-DT      : {skip_dt}")
    print(f"  SKIP-SILENT  : {silent}     (anti-Φ-D.A1 must be 0)")

    failed = False

    if args.no_silent_skip and silent > 0:
        print(f"\n[GATE-FAIL] anti-Φ-D.A1 doctrine violated : "
              f"{silent} silent-skip(s) detected.")
        _print_offenders(master, _SILENT, "SILENT-SKIP offenders")
        failed = True

    if args.fail_on_fail and fail > args.max_fail:
        print(f"\n[GATE-FAIL] {fail} FAIL row(s) > tolerated {args.max_fail}")
        _print_offenders(master, _FAIL, "FAIL offenders")
        failed = True

    if skip_dt > 0 and not args.tolerate_skip_with_dt:
        print(f"\n[GATE-FAIL] {skip_dt} SKIP-DT row(s) present but "
              f"--tolerate-skip-with-dt is OFF.")
        _print_offenders(master, _SKIP_DT, "SKIP-DT offenders")
        failed = True

    if failed:
        print("\nMaster gate : ❌ BLOCK")
        return 1

    print("\nMaster gate : ✅ PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
