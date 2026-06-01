#!/usr/bin/env python3
r"""Verify documentation coverage >= 90 % per SDK.

Sprint **Φ-C-SDK Completion (D29)** — artefact #10.
Cf. ``docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md`` § 11.3.

## Methodology

For each SDK we count the ratio of public symbols (= exported types,
functions, classes, methods) that have an attached doc comment.

| SDK | Public-symbol detection | Doc detection |
|-----|------------------------|----------------|
| Python | ast parse, public = no leading underscore | ast.get_docstring non-empty |
| Go     | regex on func/var/const/type exported | preceding // comment |
| TS     | export function/class/const declarations | preceding /\*\* \*/ JSDoc |
| Java   | public class/interface/method declarations | preceding /\*\* \*/ Javadoc |

## Output

```json
{
  "slo_compliance": {
    "SLO-SDK-DOC-GO":   {"measured_ratio": 0.94},
    "SLO-SDK-DOC-TS":   {"measured_ratio": 0.91},
    "SLO-SDK-DOC-JAVA": {"measured_ratio": 0.93}
  }
}
```

## Limitations

The Python SDK is largely auto-generated via PyO3 ; rust docstrings
become Python docstrings automatically. Coverage typically near 100 %.
Not gated here (= Φ-C-FFI handles Python doc).
"""

from __future__ import annotations

import argparse
import ast
import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

# Default threshold (= SLO-SDK-DOC-* ≥ 0.90).
DEFAULT_MIN_RATIO = 0.90

# Files excluded from coverage check (= auto-generated, vendored).
EXCLUDE_PATTERNS = [
    "**/generated/**",
    "**/target/**",
    "**/node_modules/**",
    "**/dist/**",
    "**/build/**",
    "**/_test.go",       # Go test files
    "**/test_*.py",      # Python tests
    "**/*_test.py",      # ditto
    "**/Test*.java",     # Java tests
    "**/*.test.ts",      # TS tests
    "**/*.spec.ts",      # ditto
]


def _excluded(path: Path) -> bool:
    s = str(path)
    return any(
        # Convert glob pattern to a contains-check
        chunk.strip("*").strip("/") in s
        for chunk in (
            "generated", "target", "node_modules", "dist", "build",
            "_test.go", "test_", "_test.py", "Test", ".test.ts", ".spec.ts",
        )
    )


# ────────────────────────────────────────────────────────────────────
# Go coverage : count exported names + preceding doc comments.
# ────────────────────────────────────────────────────────────────────


def count_go(src_dir: Path) -> tuple[int, int]:
    """Returns (total_exported, documented)."""
    total = 0
    docd = 0
    # Exported symbol patterns : `func Name`, `var Name`, `const Name`,
    # `type Name`. PascalCase.
    pat_sym = re.compile(
        r"^(?:func|var|const|type)\s+([A-Z]\w*)\b",
        re.MULTILINE,
    )
    for go_file in src_dir.rglob("*.go"):
        if _excluded(go_file):
            continue
        text = go_file.read_text(encoding="utf-8", errors="replace")
        lines = text.splitlines()
        for i, line in enumerate(lines):
            m = pat_sym.match(line)
            if not m:
                continue
            total += 1
            # Look upward for a contiguous block of `//` comments.
            j = i - 1
            has_doc = False
            while j >= 0 and lines[j].strip().startswith("//"):
                has_doc = True
                j -= 1
            if has_doc:
                docd += 1
    return total, docd


# ────────────────────────────────────────────────────────────────────
# TypeScript coverage : count `export` declarations + JSDoc above.
# ────────────────────────────────────────────────────────────────────


def count_ts(src_dir: Path) -> tuple[int, int]:
    total = 0
    docd = 0
    pat_sym = re.compile(
        r"^export\s+(?:async\s+)?(?:function|class|const|interface|type|enum)\s+(\w+)",
        re.MULTILINE,
    )
    for ts_file in src_dir.rglob("*.ts"):
        if _excluded(ts_file):
            continue
        text = ts_file.read_text(encoding="utf-8", errors="replace")
        lines = text.splitlines()
        for i, line in enumerate(lines):
            m = pat_sym.match(line)
            if not m:
                continue
            total += 1
            # Look upward for `*/` then `/**`.
            j = i - 1
            # Skip blank lines.
            while j >= 0 and not lines[j].strip():
                j -= 1
            if j >= 0 and lines[j].strip().endswith("*/"):
                # Walk up to find the matching `/**`.
                while j >= 0 and not lines[j].strip().startswith("/**"):
                    j -= 1
                if j >= 0:
                    docd += 1
    return total, docd


# ────────────────────────────────────────────────────────────────────
# Java coverage : count public declarations + Javadoc above.
# ────────────────────────────────────────────────────────────────────


def count_java(src_dir: Path) -> tuple[int, int]:
    total = 0
    docd = 0
    pat_sym = re.compile(
        r"^\s*public\s+(?:static\s+)?(?:final\s+)?(?:abstract\s+)?"
        r"(?:class|interface|enum|record|"
        r"(?:[A-Za-z_<>?,\s]+\s+\w+\s*\())",
        re.MULTILINE,
    )
    for java_file in src_dir.rglob("*.java"):
        if _excluded(java_file):
            continue
        text = java_file.read_text(encoding="utf-8", errors="replace")
        lines = text.splitlines()
        for i, line in enumerate(lines):
            stripped = line.strip()
            # Heuristic : line starts with "public " and contains either
            # "class", "interface", "enum", "record", or "(" (= method).
            if not stripped.startswith("public "):
                continue
            if not any(
                kw in stripped
                for kw in (" class ", " interface ", " enum ", " record ", "(")
            ):
                continue
            total += 1
            # Look upward for Javadoc (= /** ... */ block).
            j = i - 1
            # Skip blank + annotation lines.
            while j >= 0 and (
                not lines[j].strip() or lines[j].strip().startswith("@")
            ):
                j -= 1
            if j >= 0 and lines[j].strip().endswith("*/"):
                # Walk up to find `/**`.
                while j >= 0 and not lines[j].strip().startswith("/**"):
                    j -= 1
                if j >= 0:
                    docd += 1
    return total, docd


# ────────────────────────────────────────────────────────────────────
# Driver.
# ────────────────────────────────────────────────────────────────────


def main() -> int:
    ap = argparse.ArgumentParser(description="Verify SDK doc coverage")
    ap.add_argument(
        "--min", type=float, default=DEFAULT_MIN_RATIO,
        help="Minimum doc coverage ratio (default 0.90)",
    )
    ap.add_argument(
        "--out", type=Path,
        default=REPO_ROOT / "target" / "sdk" / "doc_coverage.json",
    )
    args = ap.parse_args()

    sdks = {
        "go":   (REPO_ROOT / "clients" / "go", count_go),
        "ts":   (REPO_ROOT / "clients" / "typescript", count_ts),
        "java": (REPO_ROOT / "clients" / "java", count_java),
    }

    results: dict[str, dict] = {}
    overall_pass = True

    print("\nSDK doc coverage check (≥ {:.0%}) :".format(args.min), file=sys.stderr)
    print("─" * 60, file=sys.stderr)
    for sdk, (root, counter) in sdks.items():
        if not root.exists():
            print(f"  {sdk:<6} : <missing> {root}", file=sys.stderr)
            results[sdk] = {"ratio": None, "passed": False, "total": 0, "documented": 0}
            overall_pass = False
            continue
        total, docd = counter(root)
        ratio = docd / total if total > 0 else 1.0
        passed = ratio >= args.min
        print(
            f"  {sdk:<6} : {docd}/{total} = {ratio:.2%} "
            f"{'✓' if passed else '✗ (< ' + format(args.min, '.0%') + ')'}",
            file=sys.stderr,
        )
        results[sdk] = {
            "ratio": ratio,
            "passed": passed,
            "total": total,
            "documented": docd,
        }
        if not passed:
            overall_pass = False
    print("─" * 60, file=sys.stderr)

    # Emit per-SDK SLO compliance.
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(
        json.dumps(
            {
                "slo_compliance": {
                    "SLO-SDK-DOC-GO": {
                        "measured_ratio": results["go"]["ratio"] or 0.0,
                        "total_symbols": results["go"]["total"],
                        "documented_symbols": results["go"]["documented"],
                    },
                    "SLO-SDK-DOC-TS": {
                        "measured_ratio": results["ts"]["ratio"] or 0.0,
                        "total_symbols": results["ts"]["total"],
                        "documented_symbols": results["ts"]["documented"],
                    },
                    "SLO-SDK-DOC-JAVA": {
                        "measured_ratio": results["java"]["ratio"] or 0.0,
                        "total_symbols": results["java"]["total"],
                        "documented_symbols": results["java"]["documented"],
                    },
                }
            },
            indent=2,
        )
        + "\n"
    )

    return 0 if overall_pass else 1


if __name__ == "__main__":
    sys.exit(main())
