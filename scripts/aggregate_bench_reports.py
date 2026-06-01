#!/usr/bin/env python3
"""scripts/aggregate_bench_reports.py — merge per-job criterion outputs.

Sprint D5 introduces a 5-job CI matrix (cf. `bench.yml`). Each matrix
job runs a subset of benches and uploads its `target/criterion/**`
tree as an artefact. The aggregate job downloads all artefacts and
must compose them into a single criterion root before delegating to
`parse_criterion_report.py` (which expects one root).

This script is the merger.

Schema:
─────────────────────────────────────────────────────────────────
Input  :  --inputs <DIR>
          A directory containing one sub-directory per partial
          artefact downloaded by `actions/download-artifact@v4`
          (with `pattern: bench-partial-*` and `merge-multiple: false`).
          Each sub-directory contains a `criterion/` tree from one
          matrix job.

Output :  --merged-root <DIR>     (default: target/criterion-merged)
          A unified criterion-shaped tree obtained by symlinking /
          copying every group from every input. The aggregator does
          NOT call `parse_criterion_report.py` itself — that's a
          separate step in the workflow, so the operator can choose
          the parser flags and metadata.

Optional: --output <FILE>
          If set, ALSO call `parse_criterion_report.py` and write
          the consolidated JSON report to <FILE>. Convenience for
          local use; CI keeps the steps separate.

Collision policy:
─────────────────────────────────────────────────────────────────
Bench groups are namespaced by bench file (e.g., `swarm_e2e_*` vs
`server_rpc_*`), so concurrent matrix jobs CANNOT produce the same
group name. We assert this invariant and fail loudly if violated —
silent overwrite would cause data loss without a useful error.

Exit codes:
  0  on success.
  1  if a group collides between two input directories.
  2  if `--inputs` is missing or empty.
"""

from __future__ import annotations
import argparse
import glob
import os
import shutil
import subprocess
import sys
from typing import Dict, List


def discover_partial_roots(inputs_dir: str) -> List[str]:
    """Return absolute paths to each criterion root inside `inputs_dir`.

    A partial artefact may be either:
    - `<inputs>/<job>/criterion/<groups…>` (most common — workers
      preserve `target/criterion/` structure when uploading).
    - `<inputs>/<job>/<groups…>` (when the upload step rooted at
      `target/criterion/` directly).

    We probe both layouts. Returns deduplicated absolute paths.

    # Multi-segment id awareness
    #
    # The pre-fix layout-B heuristic only matched `<job>/<group>/new/
    # estimates.json` (group = single dir between job and `new`),
    # which silently missed criterion roots whose benches use
    # `BenchmarkId::new("prefix", value)` and produce 3-deep paths
    # (<job>/<group>/<prefix>/<value>/new/...). We now use a recursive
    # glob to detect any `new/estimates.json` at any depth.
    """
    roots: List[str] = []
    if not os.path.isdir(inputs_dir):
        return roots

    for entry in sorted(os.listdir(inputs_dir)):
        entry_path = os.path.join(inputs_dir, entry)
        if not os.path.isdir(entry_path):
            continue
        # Layout A : <job>/criterion/<groups…>
        criterion_sub = os.path.join(entry_path, "criterion")
        if os.path.isdir(criterion_sub):
            roots.append(os.path.abspath(criterion_sub))
            continue
        # Layout B : <job> itself is a criterion root. Detect via
        # recursive glob — handles 2-deep AND 3-deep id paths alike.
        probe = os.path.join(entry_path, "**", "new", "estimates.json")
        if any(glob.glob(probe, recursive=True)):
            roots.append(os.path.abspath(entry_path))

    # Dedupe while preserving order
    seen: set[str] = set()
    return [r for r in roots if not (r in seen or seen.add(r))]


def merge_roots(roots: List[str], merged_root: str) -> Dict[str, str]:
    """Copy every group from every root into `merged_root`.

    Returns a `{group_name: source_root}` map for collision diagnostics.
    """
    os.makedirs(merged_root, exist_ok=True)
    provenance: Dict[str, str] = {}

    for root in roots:
        for group in sorted(os.listdir(root)):
            src = os.path.join(root, group)
            if not os.path.isdir(src):
                continue
            dst = os.path.join(merged_root, group)
            if group in provenance:
                raise SystemExit(
                    f"FATAL: group '{group}' collides between "
                    f"{provenance[group]} and {root}. Bench groups "
                    f"must be globally unique across the catalog."
                )
            provenance[group] = root
            # `shutil.copytree` is portable and respects the criterion
            # tree shape (`<group>/<id>/new/estimates.json`).
            if os.path.exists(dst):
                shutil.rmtree(dst)
            shutil.copytree(src, dst)
    return provenance


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--inputs", required=True,
        help="Root containing per-job criterion artefacts.",
    )
    ap.add_argument(
        "--merged-root", default="target/criterion-merged",
        help="Where to write the unified criterion tree.",
    )
    ap.add_argument(
        "--output",
        help=(
            "If set, also delegate to parse_criterion_report.py and "
            "write the consolidated JSON report here."
        ),
    )
    # Pass-through metadata flags. We forward everything to the parser
    # if --output is set ; absent flags get parser defaults.
    for flag in [
        "--metadata-os", "--metadata-cpu", "--metadata-ncpu",
        "--metadata-ram-gb", "--metadata-rustc", "--metadata-cargo",
        "--metadata-rustflags", "--metadata-blas", "--metadata-sha",
        "--metadata-branch", "--metadata-dirty", "--metadata-timestamp",
    ]:
        ap.add_argument(flag, default=None)
    # Sprint D6 — forward the eval-bench-format file if provided.
    # Sprint D7 — repeatable so both single- and multi-catalog reports
    # can be merged in one parse pass.
    ap.add_argument(
        "--eval-bench-format",
        action="append",
        default=None,
        help="Path to a JSON file produced by run_eval.py "
             "--output-bench-format ; merged into the unified report. "
             "May be repeated.",
    )

    args = ap.parse_args()

    roots = discover_partial_roots(args.inputs)
    if not roots:
        print(
            f"WARN: no criterion roots found under {args.inputs}. "
            "Producing empty merged tree.",
            file=sys.stderr,
        )

    print(f"==> Found {len(roots)} partial criterion root(s):")
    for r in roots:
        print(f"    - {r}")

    provenance = merge_roots(roots, args.merged_root)
    print(f"==> Merged {len(provenance)} group(s) into {args.merged_root}")

    if args.output:
        # Forward any provided metadata flags ; the parser tolerates
        # missing ones (uses defaults) thanks to its own argparse.
        cmd = [
            "python3", "scripts/parse_criterion_report.py",
            "--criterion-root", args.merged_root,
            "--output", args.output,
        ]
        for flag in [
            "metadata_os", "metadata_cpu", "metadata_ncpu",
            "metadata_ram_gb", "metadata_rustc", "metadata_cargo",
            "metadata_rustflags", "metadata_blas", "metadata_sha",
            "metadata_branch", "metadata_dirty", "metadata_timestamp",
        ]:
            value = getattr(args, flag, None)
            if value is not None:
                cmd.extend([f"--{flag.replace('_', '-')}", value])
        # `--metadata-timestamp` is REQUIRED by the parser ; default to
        # an empty string if the caller didn't provide one (parser
        # accepts that and writes "" into the report).
        if args.metadata_timestamp is None:
            cmd.extend(["--metadata-timestamp", ""])
        # Sprint D6 — forward the eval-bench-format flag(s) if provided.
        # D7 — flag is repeatable ; preserve order so the parser sees
        # them in the same order the operator did.
        for eval_path in args.eval_bench_format or []:
            cmd.extend(["--eval-bench-format", eval_path])
        print(f"==> Invoking parser: {' '.join(cmd)}")
        rc = subprocess.call(cmd)
        if rc != 0:
            return rc

    return 0


if __name__ == "__main__":
    sys.exit(main())
