#!/usr/bin/env python3
"""
Sprint Φ-D-OPS (D95) — Consolidate 105 surface SLOs into the 18
master SLOs declared in `bench_baselines/v0.9.0/ops_master_slos.yaml`.

The script ingests the per-sprint baseline files, computes per-master
rollups (worst, mean, parity-ratio), and emits a markdown report under
`target/ops/master_report.md`. The hard gate is delegated to
`master_gate_check.py`.

Cf. `docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md` § 8.
Doctrine anti-Φ-D.A1 : a SLO declared in a baseline but with no
measurement in the consolidated payload is treated as **silent skip**
and the report flags it explicitly. The downstream gate rejects.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import statistics
import sys
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional

try:
    import yaml  # type: ignore[import-untyped]
except ImportError:  # pragma: no cover
    sys.stderr.write("missing dep: pip install pyyaml\n")
    sys.exit(2)


REPO_ROOT = Path(__file__).resolve().parents[2]


# ---------------------------------------------------------------------
# Domain types
# ---------------------------------------------------------------------


class SLOStatus:
    PASS = "PASS"
    FAIL = "FAIL"
    SKIP_WITH_DT = "SKIP-DT"
    SILENT_SKIP = "SKIP-SILENT"   # anti-doctrine — never tolerated
    NO_MEASURE = "NO-MEASURE"     # measurement absent but waived OK


_OK_STATUSES = {SLOStatus.PASS, SLOStatus.SKIP_WITH_DT, SLOStatus.NO_MEASURE}


def _classify(target: float, measure: Optional[float],
              operator: str, dt_ticket: Optional[str]) -> str:
    """Decide pass/fail/skip for a single SLO row."""
    if measure is None:
        return SLOStatus.SKIP_WITH_DT if dt_ticket else SLOStatus.SILENT_SKIP
    op = operator
    if op in (">=", "ge"):
        return SLOStatus.PASS if measure >= target else SLOStatus.FAIL
    if op in ("<=", "le"):
        return SLOStatus.PASS if measure <= target else SLOStatus.FAIL
    if op in ("==", "eq"):
        return SLOStatus.PASS if abs(measure - target) < 1e-9 else SLOStatus.FAIL
    raise ValueError(f"unknown operator {operator!r}")


# ---------------------------------------------------------------------
# IO
# ---------------------------------------------------------------------


def _load_yaml(path: Path) -> Dict[str, Any]:
    with path.open("r", encoding="utf-8") as fh:
        data = yaml.safe_load(fh)
    if not isinstance(data, dict):
        raise ValueError(f"{path}: expected top-level mapping")
    return data


def _load_measurements(path: Path) -> Dict[str, float]:
    """Optional JSON file produced by the benches : `{slo_id: value}`.

    Tolerates :
      * Missing file       → empty mapping.
      * Empty / blank file → empty mapping.
      * `/dev/null`        → empty mapping (CI smoke convenience).
    """
    if not path.exists():
        return {}
    try:
        raw = path.read_text(encoding="utf-8").strip()
    except OSError:
        return {}
    if not raw:
        return {}
    return json.loads(raw)


def _iter_slos(bundle: Dict[str, Any]) -> Iterable[Dict[str, Any]]:
    """Yield SLO records from a YAML file regardless of nesting style."""
    spec = bundle.get("spec", bundle)
    if isinstance(spec, list):
        yield from spec
        return
    items = spec.get("slos") if isinstance(spec, dict) else None
    if items is None and isinstance(spec, dict):
        # Master bundle pattern : spec is a list dict-of-lists
        for k, v in spec.items():
            if isinstance(v, list) and all(isinstance(x, dict) for x in v):
                yield from v
        return
    if isinstance(items, list):
        yield from items


# ---------------------------------------------------------------------
# Consolidation
# ---------------------------------------------------------------------


def _flatten_surface(paths: List[Path],
                     measures: Dict[str, float]) -> List[Dict[str, Any]]:
    rows = []
    for path in paths:
        bundle = _load_yaml(path)
        sprint_tag = (
            bundle.get("metadata", {}).get("sprint")
            or path.stem
        )
        for slo in _iter_slos(bundle):
            slo_id = slo.get("id")
            if not slo_id:
                continue
            target = slo.get("target")
            operator = slo.get("operator", ">=")
            dt_ticket = slo.get("deferred_ticket") or slo.get("dt_ticket")
            measure = measures.get(slo_id)
            verdict = _classify(target, measure, operator, dt_ticket)
            rows.append({
                "sprint": sprint_tag,
                "id": slo_id,
                "name": slo.get("name", slo_id),
                "target": target,
                "operator": operator,
                "measure": measure,
                "status": verdict,
                "owner": slo.get("owner", "unknown"),
                "severity": slo.get("severity", "watch"),
                "source": str(path.relative_to(REPO_ROOT)) if path.is_absolute() else str(path),
            })
    return rows


def _master_rollup(master: Dict[str, Any],
                   surface_rows: List[Dict[str, Any]]) -> Dict[str, Any]:
    """Compute the consolidated status for the 18 master SLOs.

    The rollup rules :
      - PASS    iff all inherited surface SLOs PASS
      - FAIL    if any inherited surface SLO FAIL
      - SKIP-DT if at least one is SKIP-DT and no FAIL
      - SKIP-SILENT propagates upward and is the build-breaker
    """
    rollups = []
    for master_slo in _iter_slos(master):
        inherits = master_slo.get("inherits") or []
        # Surface rows whose id is in `inherits`.
        children = [r for r in surface_rows if r["id"] in inherits]
        if not children:
            # Master without inheritance is its own measure.
            rollups.append({
                **master_slo,
                "children_count": 0,
                "rolled_status": master_slo.get("status", "ACTIVE"),
                "fail_examples": [],
            })
            continue
        statuses = {c["status"] for c in children}
        if SLOStatus.SILENT_SKIP in statuses:
            rolled = SLOStatus.SILENT_SKIP
        elif SLOStatus.FAIL in statuses:
            rolled = SLOStatus.FAIL
        elif SLOStatus.SKIP_WITH_DT in statuses:
            rolled = SLOStatus.SKIP_WITH_DT
        else:
            rolled = SLOStatus.PASS
        fail_examples = [c["id"] for c in children if c["status"] == SLOStatus.FAIL]
        rollups.append({
            **master_slo,
            "children_count": len(children),
            "rolled_status": rolled,
            "fail_examples": fail_examples[:3],
        })
    return rollups


# ---------------------------------------------------------------------
# Reporting
# ---------------------------------------------------------------------


def _render_markdown(master_rollups: List[Dict[str, Any]],
                     surface_rows: List[Dict[str, Any]],
                     out_path: Path,
                     baselines: List[Path]) -> None:
    out_path.parent.mkdir(parents=True, exist_ok=True)
    lines = []
    now = dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%M:%SZ")
    lines.append("# Φ-D-OPS master SLO consolidation\n")
    lines.append(f"_Generated {now}_\n")
    lines.append("## Inputs\n")
    for b in baselines:
        try:
            relative = b.relative_to(REPO_ROOT)
        except ValueError:
            relative = b
        lines.append(f"- `{relative}`")
    lines.append("")
    lines.append("## Master roll-up (18 SLOs)\n")
    lines.append("| Master SLO | Severity | Status | Children | Failing examples |")
    lines.append("|---|---|:---:|---:|---|")
    for m in master_rollups:
        examples = ", ".join(m["fail_examples"]) or "—"
        lines.append(
            f"| `{m['id']}` | {m.get('severity','watch')} | "
            f"**{m['rolled_status']}** | {m['children_count']} | {examples} |"
        )

    pass_count = sum(1 for m in master_rollups
                     if m["rolled_status"] == SLOStatus.PASS)
    fail_count = sum(1 for m in master_rollups
                     if m["rolled_status"] == SLOStatus.FAIL)
    skip_dt = sum(1 for m in master_rollups
                  if m["rolled_status"] == SLOStatus.SKIP_WITH_DT)
    silent = sum(1 for m in master_rollups
                 if m["rolled_status"] == SLOStatus.SILENT_SKIP)
    lines.append("")
    lines.append("## Summary\n")
    lines.append(f"- PASS         : **{pass_count}** / {len(master_rollups)}")
    lines.append(f"- FAIL         : **{fail_count}**")
    lines.append(f"- SKIP-DT      : {skip_dt}")
    lines.append(f"- SKIP-SILENT  : **{silent}**  ← must be 0 per Φ-D.A1 doctrine")
    lines.append("")

    # Surface details (collapsible) appended verbatim for traceability.
    lines.append("## Surface SLO details (105 rows)\n")
    lines.append("<details><summary>Click to expand</summary>\n")
    lines.append("| Sprint | SLO | Target | Op | Measure | Status | Severity | Owner |")
    lines.append("|---|---|---:|:---:|---:|:---:|:---:|---|")
    for r in sorted(surface_rows, key=lambda x: (x["sprint"], x["id"])):
        m = "—" if r["measure"] is None else f"{r['measure']:.4f}"
        lines.append(
            f"| {r['sprint']} | `{r['id']}` | {r['target']} | {r['operator']} | "
            f"{m} | **{r['status']}** | {r['severity']} | {r['owner']} |"
        )
    lines.append("\n</details>\n")
    out_path.write_text("\n".join(lines), encoding="utf-8")


def _render_json(master_rollups: List[Dict[str, Any]],
                 surface_rows: List[Dict[str, Any]],
                 out_json: Path) -> None:
    out_json.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "apiVersion": "efe.io/v1",
        "kind": "MasterSLOConsolidation",
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "master": master_rollups,
        "surface": surface_rows,
        "counts": {
            "pass": sum(1 for m in master_rollups
                        if m["rolled_status"] == SLOStatus.PASS),
            "fail": sum(1 for m in master_rollups
                        if m["rolled_status"] == SLOStatus.FAIL),
            "skip_dt": sum(1 for m in master_rollups
                           if m["rolled_status"] == SLOStatus.SKIP_WITH_DT),
            "silent_skip": sum(1 for m in master_rollups
                               if m["rolled_status"] == SLOStatus.SILENT_SKIP),
        },
    }
    out_json.write_text(json.dumps(payload, indent=2, default=str),
                        encoding="utf-8")


# ---------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(
        description="Consolidate 105 surface SLOs into the 18 master OPS SLOs.",
    )
    parser.add_argument("--baselines", action="append", required=True,
                        help="Path to a surface SLO YAML (repeatable).")
    parser.add_argument("--master",
                        default="bench_baselines/v0.9.0/ops_master_slos.yaml",
                        help="Master SLO YAML to roll up against.")
    parser.add_argument("--measures",
                        default="target/ops/measures.json",
                        help="Optional JSON {slo_id: value} produced by the "
                             "per-sprint benches. Missing measurements are "
                             "tolerated only if a deferred_ticket is set on "
                             "the surface SLO.")
    parser.add_argument("--out",
                        default="target/ops/master_report.md",
                        help="Markdown report path.")
    parser.add_argument("--out-json",
                        default="target/ops/master_report.json")
    args = parser.parse_args(argv)

    baselines = [Path(p) for p in args.baselines]
    for p in baselines:
        if not p.exists():
            sys.stderr.write(f"missing baseline file: {p}\n")
            return 2

    master_path = Path(args.master)
    if not master_path.exists():
        sys.stderr.write(f"missing master bundle: {master_path}\n")
        return 2
    master = _load_yaml(master_path)

    measures = _load_measurements(Path(args.measures))
    surface_rows = _flatten_surface(baselines, measures)
    master_rollups = _master_rollup(master, surface_rows)

    out_md = Path(args.out)
    out_json = Path(args.out_json)
    _render_markdown(master_rollups, surface_rows, out_md, baselines)
    _render_json(master_rollups, surface_rows, out_json)

    print(f"master report      : {out_md}")
    print(f"machine-readable   : {out_json}")
    print(f"surface SLOs found : {len(surface_rows)}")
    print(f"master rollups     : {len(master_rollups)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
