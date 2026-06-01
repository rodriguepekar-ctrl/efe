#!/usr/bin/env bash
# append-vex-yank.sh — Append a yank entry to the VEX (OpenVEX) feed.
#
# Usage :
#   ./scripts/release/append-vex-yank.sh v1.0.1 P1 "fix breaking schema migration"

set -euo pipefail

VERSION="${1:?usage: $0 <version> <severity> <reason>}"
SEVERITY="${2:?usage: $0 <version> <severity> <reason>}"
REASON="${3:?usage: $0 <version> <severity> <reason>}"

VEX_FILE="docs/security/vex/feed.json"
mkdir -p "$(dirname "$VEX_FILE")"

if [[ ! -f "$VEX_FILE" ]]; then
  cat > "$VEX_FILE" <<EOF
{
  "@context": "https://openvex.dev/ns/v0.2.0",
  "@id": "https://efe.example.com/security/vex/feed.json",
  "author": "EFE Security Team",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "version": 1,
  "statements": []
}
EOF
fi

python3 - "$VEX_FILE" "$VERSION" "$SEVERITY" "$REASON" <<'PY'
import json
import sys
from datetime import datetime, timezone

path, version, severity, reason = sys.argv[1:5]
with open(path) as f:
    feed = json.load(f)

products = []
for img in ("efe-server", "mcp-server", "portal-admin", "efe-server-migrate"):
    products.append({"@id": f"pkg:oci/{img}@{version.lstrip('v')}"})
products.append({"@id": f"pkg:helm/efe-stack@{version.lstrip('v')}"})

statement = {
    "vulnerability": {
        "name": f"EFE-YANK-{version}",
        "description": f"Release {version} yanked. Severity {severity}. Reason: {reason}",
    },
    "products": products,
    "status": "fixed",
    "fix_version": "TBD (next patch)",
    "timestamp": datetime.now(timezone.utc).isoformat(),
    "impact_statement": reason,
    "action_statement": "Pin to next patch release ; do not deploy this version.",
}
feed["statements"].insert(0, statement)
feed["timestamp"] = datetime.now(timezone.utc).isoformat()
feed["version"] = feed.get("version", 0) + 1

with open(path, "w") as f:
    json.dump(feed, f, indent=2)

print(f"✓ Appended yank entry for {version} to {path}")
PY

echo "✅ VEX feed updated"
