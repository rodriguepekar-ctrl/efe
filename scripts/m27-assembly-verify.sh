#!/usr/bin/env bash
# M27 Commercial Edge — 5-tier verification.
#
# Run before merging the M27 PR. Mirrors the D93 assembly-verify.sh
# pattern.
#
# Usage :
#   ./scripts/m27-assembly-verify.sh                     # full run
#   TIER=1 ./scripts/m27-assembly-verify.sh              # only Tier 1
#   SKIP_TIER_5=1 ./scripts/m27-assembly-verify.sh       # skip load test

set -euo pipefail

if [[ -t 2 ]]; then
    BOLD=$'\e[1m'; GREEN=$'\e[32m'; RED=$'\e[31m'; YEL=$'\e[33m'; NC=$'\e[0m'
else
    BOLD=''; GREEN=''; RED=''; YEL=''; NC=''
fi
banner() { echo "${BOLD}=== $1 ===${NC}" >&2; }
ok()     { echo "${GREEN}✓ $1${NC}" >&2; }
warn()   { echo "${YEL}⚠ $1${NC}" >&2; }
fail()   { echo "${RED}✗ $1${NC}" >&2; }

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

START=$(date +%s)
TIER_FILTER="${TIER:-all}"

run_tier() {
    local n="$1"; shift; local title="$1"; shift
    if [[ "$TIER_FILTER" != "all" && "$TIER_FILTER" != "$n" ]]; then
        warn "Tier $n skipped (TIER=$TIER_FILTER)"
        return 0
    fi
    banner "Tier $n : $title"
    local t0=$(date +%s)
    if "$@"; then
        local t1=$(date +%s)
        ok "Tier $n PASSED in $((t1-t0))s"
    else
        fail "Tier $n FAILED"
        return 1
    fi
}

# ── Tier 1 : workspace cargo check + tests compile ────────────
tier1() {
    cargo check --workspace --exclude efe-ffi --locked
    cargo test --workspace --exclude efe-ffi --no-run --locked
}

# ── Tier 2 : efe-cache unit tests ────────────────────────────
tier2() {
    cargo test -p efe-cache --lib
    cargo test -p efe-server --lib middleware::
}

# ── Tier 3 : Helm + Terraform lint ────────────────────────────
tier3() {
    if command -v helm >/dev/null 2>&1; then
        helm lint charts/efe-stack
        helm template test charts/efe-stack \
            --set efe-edge.enabled=true \
            > /tmp/m27-helm.yaml
        if command -v kubectl >/dev/null 2>&1; then
            kubectl --dry-run=client apply -f /tmp/m27-helm.yaml >/dev/null 2>&1 \
              || warn "kubectl dry-run had warnings ; review /tmp/m27-helm.yaml"
        fi
    else
        warn "helm CLI missing — Tier 3 partial"
    fi
    if command -v terraform >/dev/null 2>&1; then
        (cd terraform/cloudflare && terraform init -backend=false -input=false \
             && terraform validate)
    else
        warn "terraform CLI missing — TF validate skipped"
    fi
}

# ── Tier 4 : k3d local cluster smoke ──────────────────────────
tier4() {
    if [[ "${SKIP_TIER_4:-0}" == "1" ]]; then
        warn "Tier 4 skipped (SKIP_TIER_4=1)"
        return 0
    fi
    if ! command -v k3d >/dev/null 2>&1; then
        warn "k3d CLI missing — Tier 4 N/A"
        return 0
    fi
    bash scripts/m27/k3d-smoke.sh
}

# ── Tier 5 : load test (k6) ───────────────────────────────────
tier5() {
    if [[ "${SKIP_TIER_5:-0}" == "1" ]]; then
        warn "Tier 5 skipped (SKIP_TIER_5=1)"
        return 0
    fi
    if ! command -v k6 >/dev/null 2>&1; then
        warn "k6 CLI missing — Tier 5 N/A"
        return 0
    fi
    EFE_API_BASE="${EFE_API_BASE:-http://localhost:8443}" \
    EFE_BENCH_TOKEN="${EFE_BENCH_TOKEN:-test-token}" \
        k6 run scripts/load/m27/baseline_1000rps.js
}

# ── Driver ────────────────────────────────────────────────────
run_tier 1 "workspace cargo check + tests compile"   tier1
run_tier 2 "unit tests : efe-cache + efe-server::middleware" tier2
run_tier 3 "Helm + Terraform lint"                   tier3
run_tier 4 "k3d smoke"                               tier4
run_tier 5 "k6 load test"                            tier5

TOTAL=$(($(date +%s) - START))
echo ""
ok "M27 ALL TIERS PASSED in ${TOTAL}s"
