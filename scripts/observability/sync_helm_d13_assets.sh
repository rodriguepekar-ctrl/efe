#!/usr/bin/env bash
# Sprint D13 audit C-2 fix — sync canonical observability assets into
# the Helm chart's in-tree mirror.
#
# Helm's ``.Files.Glob`` / ``.Files.Get`` cannot escape the chart
# sandbox (``deploy/helm/efe/``). To ship the dashboards / rules /
# promtail config via the chart we mirror them under
# ``deploy/helm/efe/d13_files/``. This script keeps the mirror in
# sync with the canonical location.
#
# Pinned by ``scripts/tests/test_d13_helm_files_in_sync.py`` — CI
# fails if the mirror drifts.
#
# Usage::
#
#     bash scripts/observability/sync_helm_d13_assets.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

SRC_DASH="${REPO_ROOT}/deploy/observability/grafana/dashboards/d13"
SRC_RULES="${REPO_ROOT}/deploy/observability/prometheus/rules.d13"
SRC_LOKI="${REPO_ROOT}/deploy/observability/loki"

# **Audit fix T-A2** — M5 extension dashboards (D29 GPU canary, D34
# templates + Mode-2, D37 Mode-3) live in their own per-sprint
# subdirs canonically. Helm provisioning expects them flat under
# `d13_files/dashboards/` ; we copy with a `dXX_NN_<slug>` prefix
# to avoid collisions with the existing 6 d13 dashboards
# (which are numbered 01-06).
SRC_DASH_D29="${REPO_ROOT}/deploy/observability/grafana/dashboards/d29"
SRC_DASH_D34="${REPO_ROOT}/deploy/observability/grafana/dashboards/d34"
SRC_DASH_D37="${REPO_ROOT}/deploy/observability/grafana/dashboards/d37"

DST_DASH="${REPO_ROOT}/deploy/helm/efe/d13_files/dashboards"
DST_RULES="${REPO_ROOT}/deploy/helm/efe/d13_files/prometheus"
DST_LOKI="${REPO_ROOT}/deploy/helm/efe/d13_files/loki"

mkdir -p "${DST_DASH}" "${DST_RULES}" "${DST_LOKI}"

# D13 originals (Sprint D13).
cp -f "${SRC_DASH}"/*.json                       "${DST_DASH}/"
cp -f "${SRC_RULES}/recording.yaml"              "${DST_RULES}/"
cp -f "${SRC_RULES}/alerts.yaml"                 "${DST_RULES}/"
cp -f "${SRC_LOKI}/promtail-config.example.yaml" "${DST_LOKI}/"

# **M5 extension** (Sprint D29 / D34 / D37). Each canonical file is
# copied with a sprint prefix so the flat helm dir avoids name
# collisions (d13's `01_*.json` vs d29's `01_*.json`).
if [[ -d "${SRC_DASH_D29}" ]]; then
    for f in "${SRC_DASH_D29}"/*.json; do
        bn="$(basename "${f}")"
        cp -f "${f}" "${DST_DASH}/d29_${bn}"
    done
fi
if [[ -d "${SRC_DASH_D34}" ]]; then
    for f in "${SRC_DASH_D34}"/*.json; do
        bn="$(basename "${f}")"
        cp -f "${f}" "${DST_DASH}/d34_${bn}"
    done
fi
if [[ -d "${SRC_DASH_D37}" ]]; then
    for f in "${SRC_DASH_D37}"/*.json; do
        bn="$(basename "${f}")"
        cp -f "${f}" "${DST_DASH}/d37_${bn}"
    done
fi

echo "✓ synced :"
ls -1 "${DST_DASH}"  | sed 's/^/    dashboards\//'
ls -1 "${DST_RULES}" | sed 's/^/    prometheus\//'
ls -1 "${DST_LOKI}"  | sed 's/^/    loki\//'
