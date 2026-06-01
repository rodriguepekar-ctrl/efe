#!/usr/bin/env python3
"""
Build an OpenVEX candidate from cargo-audit + pip-audit + govulncheck
JSON outputs.

Usage :
    python3 scripts/m29/build_vex.py \\
        --rust cargo-audit.json \\
        --python pip-audit.json \\
        --go go-audit.json \\
        --version <commit-sha-or-tag> > vex.json

Output : a single OpenVEX document. Every entry defaults to
`status = under_investigation` ; engineering reviews + sets the
correct status (not_affected | affected | fixed) before merging.
"""
from __future__ import annotations

import argparse
import datetime
import json
import sys
from pathlib import Path


def load_json(path: str | None) -> dict | list:
    if not path or not Path(path).exists():
        return {}
    with open(path) as f:
        return json.load(f)


def cves_from_cargo_audit(data: dict) -> list[str]:
    out: list[str] = []
    if not isinstance(data, dict):
        return out
    for v in data.get("vulnerabilities", {}).get("list", []):
        advisory = v.get("advisory") or {}
        cve = advisory.get("id") or advisory.get("cvss")
        if cve:
            out.append(cve)
    return out


def cves_from_pip_audit(data) -> list[str]:
    out: list[str] = []
    if not isinstance(data, list):
        return out
    for pkg in data:
        for vuln in pkg.get("vulns", []):
            cve = vuln.get("id")
            if cve:
                out.append(cve)
    return out


def cves_from_govulncheck(data: dict) -> list[str]:
    out: list[str] = []
    # govulncheck JSON is a stream of objects ; here we accept either
    # a single dict or a list.
    if isinstance(data, dict):
        for f in data.get("findings", []):
            cve = f.get("osv", {}).get("id")
            if cve:
                out.append(cve)
    return out


def build_openvex(
    cves: list[str], product: str, version: str
) -> dict:
    return {
        "@context": "https://openvex.dev/ns",
        "@id": f"https://efe.ai/vex/{version}",
        "author": "EFE Engineering",
        "timestamp": datetime.datetime.utcnow().strftime("%Y-%m-%dT%H:%M:%SZ"),
        "version": 1,
        "statements": [
            {
                "vulnerability": cve,
                "products": [
                    f"pkg:efe/{product}@{version}"
                ],
                "status": "under_investigation",
                # Engineering fills these in via PR review :
                "justification": None,
                "impact_statement": None,
            }
            for cve in sorted(set(cves))
        ],
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--rust", default=None)
    ap.add_argument("--python", default=None)
    ap.add_argument("--go", default=None)
    ap.add_argument("--version", required=True)
    ap.add_argument("--product", default="efe-server")
    args = ap.parse_args()

    cves: list[str] = []
    cves += cves_from_cargo_audit(load_json(args.rust))
    cves += cves_from_pip_audit(load_json(args.python))
    cves += cves_from_govulncheck(load_json(args.go))

    doc = build_openvex(cves, args.product, args.version)
    json.dump(doc, sys.stdout, indent=2)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
