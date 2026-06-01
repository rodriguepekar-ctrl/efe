#!/usr/bin/env bash
# PR #19 — golden Helm render regeneration.
#
# Run after intentional template / values changes :
#     bash charts/efe-stack/charts/efe-meta-swarm/tests/golden/regen.sh
#
# Commits the new golden alongside the template change so CI Job 4
# (helm-template-diff) stays green on the next PR.

set -euo pipefail

# Resolve the chart path relative to this script regardless of cwd.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHART_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
GOLDEN_FILE="${SCRIPT_DIR}/pr19_rendered.yaml"

if ! command -v helm >/dev/null 2>&1 ; then
  echo "ERROR: helm not installed.  brew install helm  (or see https://helm.sh)" >&2
  exit 1
fi

echo "→ Rendering chart with PR #19 surfaces enabled..."
helm template efe-meta-swarm "${CHART_DIR}" \
    --namespace efe-test \
    --set enabled=true \
    --set image.tag=ci-test \
    --set alerts.enabled=true \
    --set alerts.pr19.enabled=true \
    --set dashboard.enabled=true \
    --set dashboard.pr19.enabled=true \
    | sed 's|image: .*efe-meta-swarm-server.*|image: REDACTED|' \
    > "${GOLDEN_FILE}.tmp"

# Atomic replace to avoid leaving a half-rendered file on disk if
# something between `helm template` and `mv` panics.
mv "${GOLDEN_FILE}.tmp" "${GOLDEN_FILE}"

bytes=$(wc -c < "${GOLDEN_FILE}")
echo "→ Wrote $(basename "${GOLDEN_FILE}") (${bytes} bytes)"
echo "→ Review the diff and commit alongside the triggering change :"
echo "    git add ${GOLDEN_FILE}"
echo "    git diff --cached ${GOLDEN_FILE}"
