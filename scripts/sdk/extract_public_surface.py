#!/usr/bin/env python3
"""Extract the public API surface of each SDK for isomorphism checks.

Sprint **Φ-C-SDK Completion (D34)** — artefact #39.
Cf. ``docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md`` § 15.2.

## Mission

For each of the 4 SDKs, extract the list of public methods + classes
exposed by the canonical client. Verify that the **minimal-divergence
policy** (= D-SDK-3) holds : 5 required methods present in each SDK
under idiomatic names.

## Required methods (= canonical surface)

| Canonical | Python | Go | TS | Java |
|-----------|--------|----|----|------|
| `classify(text)` → Verdict | `classify` | `Classify` | `classify` | `classify` |
| `classify_async(text)` → Future<Verdict> | `aclassify` | (= via context) | `classify` (= already Promise) | `classifyAsync` |
| `load_bundle(path)` → Bundle | `load_bundle` | n/a (= server-side) | n/a | n/a |
| `healthcheck()` → bool | `healthcheck` | `Healthcheck` | `healthcheck` | `healthcheck` |
| `close()` | `close` | `Close` | `close` | `close` |

## Output

```json
{
  "slo_compliance": {
    "SLO-SDK-API-SURFACE": {
      "measured_pass_count": 1,
      "measured_expected_count": 1,
      "per_sdk_methods": {
        "python": ["classify", "aclassify", "load_bundle", "healthcheck", "close"],
        "go":     ["Classify", "Healthcheck", "Close"],
        "ts":     ["classify", "healthcheck", "close"],
        "java":   ["classify", "classifyAsync", "healthcheck", "close"]
      },
      "missing": []
    }
  }
}
```
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]


# Map canonical method → expected idiomatic name per SDK.
# `None` means "not required in this SDK" (e.g. load_bundle is
# Python-only because Go/TS/Java go through gRPC).
CANONICAL_METHODS = {
    "classify":       {"python": "classify",      "go": "Classify",      "ts": "classify",      "java": "classify"},
    "classify_async": {"python": "aclassify",     "go": None,            "ts": None,            "java": "classifyAsync"},
    "load_bundle":    {"python": "load_bundle",   "go": None,            "ts": None,            "java": None},
    "healthcheck":    {"python": "healthcheck",   "go": "Healthcheck",   "ts": "healthcheck",   "java": "healthcheck"},
    "close":          {"python": "close",         "go": "Close",         "ts": "close",         "java": "close"},
}


def extract_python_methods() -> list[str]:
    """Extract method names from the Python SDK `EfeClient` class."""
    # Look for EfeClient in efe_tool or efe._native stubs.
    candidates = [
        REPO_ROOT / "clients" / "python" / "efe_tool" / "__init__.py",
        REPO_ROOT / "clients" / "python" / "efe_tool" / "client.py",
        REPO_ROOT / "crates" / "efe-ffi" / "python" / "efe" / "_native.pyi",
    ]
    methods: set[str] = set()
    for p in candidates:
        if not p.exists():
            continue
        text = p.read_text(encoding="utf-8")
        # Match `def method(self, ...)` inside class EfeClient or EfeNativeClient.
        in_class = False
        for line in text.splitlines():
            if re.match(r"class\s+(EfeClient|EfeNativeClient)\b", line):
                in_class = True
            elif re.match(r"class\s+\w", line) and not "EfeClient" in line and not "EfeNativeClient" in line:
                in_class = False
            elif in_class:
                m = re.match(r"\s+(?:async\s+)?def\s+([a-z_]\w*)\s*\(", line)
                if m and not m.group(1).startswith("_"):
                    methods.add(m.group(1))
    return sorted(methods)


def extract_go_methods() -> list[str]:
    """Extract exported method names from clients/go/efe-sdk/client.go."""
    p = REPO_ROOT / "clients" / "go" / "efe-sdk" / "client.go"
    if not p.exists():
        return []
    text = p.read_text(encoding="utf-8")
    # Match `func (c *Client) Method(...) ...` patterns.
    methods: set[str] = set()
    for m in re.finditer(
        r"^func\s+\(.+?\)\s+([A-Z]\w*)\s*\(", text, re.MULTILINE
    ):
        methods.add(m.group(1))
    return sorted(methods)


def extract_ts_methods() -> list[str]:
    """Extract method names from the TS EfeClient class."""
    candidates = list(
        (REPO_ROOT / "clients" / "typescript" / "@efe" / "sdk" / "src").rglob("*.ts")
    ) if (REPO_ROOT / "clients" / "typescript" / "@efe" / "sdk" / "src").exists() else []
    methods: set[str] = set()
    in_class = False
    for p in candidates:
        if any(x in str(p) for x in ("generated", "test", "spec")):
            continue
        text = p.read_text(encoding="utf-8")
        for line in text.splitlines():
            if re.match(r"export\s+class\s+EfeClient\b", line):
                in_class = True
            elif re.match(r"export\s+class\s+\w", line) and "EfeClient" not in line:
                in_class = False
            elif in_class:
                # Match `methodName(...): ReturnType {`
                m = re.match(
                    r"\s+(?:async\s+)?(?:public\s+)?([a-z][\w$]*)\s*\(", line
                )
                if m:
                    methods.add(m.group(1))
    return sorted(methods)


def extract_java_methods() -> list[str]:
    """Extract method names from the Java EfeClient class."""
    candidates = list(
        (REPO_ROOT / "clients" / "java").rglob("EfeClient.java")
    )
    methods: set[str] = set()
    for p in candidates:
        if any(x in str(p) for x in ("generated", "test", "Test")):
            continue
        text = p.read_text(encoding="utf-8")
        # Match `public <ReturnType> methodName(...)` patterns.
        for m in re.finditer(
            r"public\s+(?:static\s+)?(?:final\s+)?[\w<>,\s?]+?\s+([a-z]\w*)\s*\(",
            text,
        ):
            methods.add(m.group(1))
    return sorted(methods)


def main() -> int:
    ap = argparse.ArgumentParser(description="Extract polyglot SDK public surface")
    ap.add_argument(
        "--out", type=Path,
        default=REPO_ROOT / "target" / "sdk" / "api_surface.json",
    )
    args = ap.parse_args()

    surface = {
        "python": extract_python_methods(),
        "go":     extract_go_methods(),
        "ts":     extract_ts_methods(),
        "java":   extract_java_methods(),
    }

    print("\nPolyglot SDK API surface :", file=sys.stderr)
    print("─" * 60, file=sys.stderr)
    for sdk in ("python", "go", "ts", "java"):
        print(f"  {sdk:<8} : {surface[sdk]}", file=sys.stderr)
    print("─" * 60, file=sys.stderr)

    # Check isomorphism : each required canonical method present.
    missing = []
    for canon, mapping in CANONICAL_METHODS.items():
        for sdk, expected_name in mapping.items():
            if expected_name is None:
                continue  # not required in this SDK
            if expected_name not in surface[sdk]:
                missing.append(f"{sdk}: {canon} (= expected method `{expected_name}`)")

    passed = len(missing) == 0

    # Emit SLO compliance.
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(
        json.dumps(
            {
                "slo_compliance": {
                    "SLO-SDK-API-SURFACE": {
                        "measured_pass_count": 1 if passed else 0,
                        "measured_expected_count": 1,
                        "per_sdk_methods": surface,
                        "missing": missing,
                    }
                }
            },
            indent=2,
        )
        + "\n"
    )

    if not passed:
        print(f"\nFAIL : {len(missing)} canonical methods missing :", file=sys.stderr)
        for m in missing:
            print(f"  - {m}", file=sys.stderr)
        return 1

    print("\n✓ API surface isomorphic across all 4 SDKs", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
