#!/usr/bin/env python3
"""Verify all 4 SDKs declare the same semver version.

Sprint **Φ-C-SDK Completion (D29)** — artefact #09.
Cf. ``docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md`` § 7.3.

## Doctrine D-SDK-6 enforcement

The 4 SDKs (Python + Go + TypeScript + Java) MUST share the same
semver version at every release. Any drift is a SLO-SDK-VERSION-SYNC
violation that blocks the publish pipeline.

## Manifest sources

| SDK | Manifest | Field |
|-----|---------|-------|
| Python | `crates/efe-ffi/Cargo.toml` (workspace.package.version) | `version = "X.Y.Z"` |
| Go     | `clients/go/efe-sdk/version.go` | `const Version = "X.Y.Z"` |
| TS     | `clients/typescript/@efe/sdk/package.json` | `"version": "X.Y.Z"` |
| Java   | `clients/java/efe-sdk/pom.xml` (project root) | `<version>X.Y.Z</version>` |

## Output

Stderr : human-readable summary table.
``target/sdk/version_sync.json`` : SLO compliance JSON :

```json
{
  "slo_compliance": {
    "SLO-SDK-VERSION-SYNC": {
      "measured_pass_count": 1,
      "measured_expected_count": 1,
      "versions": {"python": "0.6.0", "go": "0.6.0", "ts": "0.6.0", "java": "0.6.0"}
    }
  }
}
```

## Exit codes

* 0 : all versions match.
* 1 : version drift detected.
* 2 : manifest missing or parse error.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]


def parse_workspace_cargo() -> str | None:
    """Read workspace.package.version from the top-level Cargo.toml."""
    cargo = REPO_ROOT / "Cargo.toml"
    if not cargo.exists():
        return None
    text = cargo.read_text()
    # Search for [workspace.package] block, then `version = "X.Y.Z"`.
    m = re.search(
        r"\[workspace\.package\][^\[]*?version\s*=\s*\"([^\"]+)\"",
        text,
        re.DOTALL,
    )
    return m.group(1) if m else None


def parse_go_version() -> str | None:
    """Read `const Version = "X.Y.Z"` from clients/go/efe-sdk/version.go."""
    path = REPO_ROOT / "clients" / "go" / "efe-sdk" / "version.go"
    if not path.exists():
        return None
    text = path.read_text()
    # Patterns supported :
    #   const Version = "0.6.0"
    #   var Version = "0.6.0"
    m = re.search(r"(?:const|var)\s+Version\s*=\s*\"([^\"]+)\"", text)
    return m.group(1) if m else None


def parse_ts_version() -> str | None:
    """Read package.json `version` field."""
    candidates = [
        REPO_ROOT / "clients" / "typescript" / "@efe" / "sdk" / "package.json",
        REPO_ROOT / "clients" / "typescript" / "package.json",
    ]
    for p in candidates:
        if p.exists():
            data = json.loads(p.read_text())
            return data.get("version")
    return None


def parse_java_version() -> str | None:
    """Read <version>X.Y.Z</version> from pom.xml (= project root, not parent ref)."""
    candidates = [
        REPO_ROOT / "clients" / "java" / "efe-sdk" / "pom.xml",
    ]
    for p in candidates:
        if not p.exists():
            continue
        text = p.read_text()
        # Strip XML comments.
        text = re.sub(r"<!--.*?-->", "", text, flags=re.DOTALL)
        # Take the FIRST <version> that is not nested in <parent>...</parent>
        # nor <dependency>...</dependency>.
        # Strategy : find all <version>X</version> outside <parent>/<dependency>
        # blocks. Easiest : strip parent + dependency blocks first.
        stripped = re.sub(r"<parent>.*?</parent>", "", text, flags=re.DOTALL)
        stripped = re.sub(r"<dependencies>.*?</dependencies>", "", stripped, flags=re.DOTALL)
        stripped = re.sub(r"<plugins>.*?</plugins>", "", stripped, flags=re.DOTALL)
        m = re.search(r"<version>\s*([^<]+?)\s*</version>", stripped)
        if m:
            return m.group(1).strip()
    return None


def parse_python_sdk_version() -> str | None:
    """Read efe-ffi version (= the canonical Python SDK manifest)."""
    cargo = REPO_ROOT / "crates" / "efe-ffi" / "Cargo.toml"
    if not cargo.exists():
        return None
    text = cargo.read_text()
    # efe-ffi typically uses `version.workspace = true` → fallback to
    # workspace.package.version.
    m = re.search(r"^version\s*=\s*\"([^\"]+)\"", text, re.MULTILINE)
    if m:
        return m.group(1)
    # Fallback : workspace.package.
    ws_ver = parse_workspace_cargo()
    return ws_ver


def main() -> int:
    ap = argparse.ArgumentParser(description="Verify SDK version sync")
    ap.add_argument(
        "--out",
        type=Path,
        default=REPO_ROOT / "target" / "sdk" / "version_sync.json",
    )
    args = ap.parse_args()

    versions = {
        "python": parse_python_sdk_version(),
        "go": parse_go_version(),
        "ts": parse_ts_version(),
        "java": parse_java_version(),
    }

    print("\nSDK version sync check :", file=sys.stderr)
    print("─" * 50, file=sys.stderr)
    for sdk, ver in versions.items():
        print(f"  {sdk:<8} : {ver or '<missing>'}", file=sys.stderr)
    print("─" * 50, file=sys.stderr)

    missing = [sdk for sdk, ver in versions.items() if ver is None]
    if missing:
        sys.stderr.write(
            f"FATAL : version missing for SDKs : {missing}\n"
            "Likely cause : manifest file deleted or schema changed.\n"
        )
        # Emit JSON for traceability before exiting.
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(
            json.dumps(
                {
                    "slo_compliance": {
                        "SLO-SDK-VERSION-SYNC": {
                            "measured_pass_count": 0,
                            "measured_expected_count": 1,
                            "versions": versions,
                            "error": f"missing manifests: {missing}",
                        }
                    }
                },
                indent=2,
            )
            + "\n"
        )
        return 2

    unique = set(versions.values())
    passed = len(unique) == 1
    if not passed:
        sys.stderr.write(
            f"\nFAIL : version drift detected. Unique values : {unique}\n"
        )

    # Emit SLO compliance JSON.
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(
        json.dumps(
            {
                "slo_compliance": {
                    "SLO-SDK-VERSION-SYNC": {
                        "measured_pass_count": 1 if passed else 0,
                        "measured_expected_count": 1,
                        "versions": versions,
                        "unique_count": len(unique),
                    }
                }
            },
            indent=2,
        )
        + "\n"
    )

    if passed:
        common = next(iter(unique))
        print(f"\n✓ ALL 4 SDKs aligned at version : {common}", file=sys.stderr)
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
