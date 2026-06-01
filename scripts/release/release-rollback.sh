#!/usr/bin/env bash
# release-rollback.sh — Locally trigger the release-rollback workflow.
#
# This script wraps `gh workflow run release-rollback.yml` for ease of use.
# Sigstore signatures are immutable ; this only flags artifacts as
# deprecated (see release-rollback.yml for the full operation list).
#
# Usage :
#   ./scripts/release/release-rollback.sh v1.0.1 P1 "fix breaking schema migration"

set -euo pipefail

VERSION="${1:?usage: $0 <version> <severity:P0|P1|P2> <reason>}"
SEVERITY="${2:?usage: $0 <version> <severity:P0|P1|P2> <reason>}"
shift 2
REASON="${*:?usage: $0 <version> <severity:P0|P1|P2> <reason>}"

if ! [[ "$VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.\-]+)?$ ]]; then
  echo "❌ Invalid version : $VERSION" >&2
  exit 1
fi

if ! [[ "$SEVERITY" =~ ^P[0-2]$ ]]; then
  echo "❌ Invalid severity : $SEVERITY (must be P0, P1, or P2)" >&2
  exit 1
fi

if [[ ${#REASON} -lt 20 ]]; then
  echo "❌ Reason too short (≥ 20 characters required, was ${#REASON})" >&2
  exit 1
fi

if ! command -v gh >/dev/null 2>&1; then
  echo "❌ gh CLI required" >&2
  exit 2
fi

cat <<EOF
▶ Confirming yank :
    Version  : $VERSION
    Severity : $SEVERITY
    Reason   : $REASON

  This action :
    - marks Docker images deprecated (annotation, immutable)
    - quarantines Helm chart with -yanked tag
    - converts GitHub Release to draft
    - cargo yank (efe-server, best-effort)
    - npm deprecate (@efe/sdk)
    - publishes VEX + RSS entry
    - sends customer notification email

  Cosign signatures and Rekor entries CANNOT be removed (append-only by
  design). Yanked artifacts remain cryptographically verifiable but
  flagged.

EOF

read -r -p "Proceed ? (type the version exactly) : " confirm
if [[ "$confirm" != "$VERSION" ]]; then
  echo "❌ Confirmation mismatch ; aborting." >&2
  exit 3
fi

echo
echo "▶ Triggering release-rollback workflow ..."
gh workflow run release-rollback.yml \
  -f version="$VERSION" \
  -f severity="$SEVERITY" \
  -f reason="$REASON"

echo
echo "✅ Workflow triggered. Monitor :"
echo "   https://github.com/efe-ai/efe/actions/workflows/release-rollback.yml"
