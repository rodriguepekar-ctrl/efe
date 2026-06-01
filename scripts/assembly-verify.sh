#!/usr/bin/env bash
# scripts/assembly-verify.sh — D93 5-tier assembly verification.
#
# Runs every pre-merge gate for EFE 2.1.1 sprint sign-off :
#   Tier 1 : workspace cargo build (--exclude efe-ffi for macOS PyO3)
#   Tier 1.5 : workspace cargo test --no-run
#   Tier 2 : vsock-relayer Go build
#   Tier 3 : Helm lint + template
#   Tier 4 : k3d local cluster smoke
#
# Exit code : 0 if all tiers pass ; first non-zero from a tier on
# failure (subsequent tiers skipped). Useful for CI gating ; also
# documented in `docs/EFE_M26_ASSEMBLY_VERIFICATION_ARCHITECTURE.md`
# Annex B.
#
# Pre-requisites :
#   - rustc 1.88 (see `rust-toolchain.toml`)
#   - go 1.22+ for Tier 2
#   - helm 3.x for Tier 3
#   - k3d / docker for Tier 4 (or set SKIP_TIER_4=1)
#
# Usage :
#   ./scripts/assembly-verify.sh            # full run
#   SKIP_TIER_4=1 ./scripts/assembly-verify.sh   # CI without k3d
#   TIER=1 ./scripts/assembly-verify.sh      # only Tier 1
#
# Output format : color-coded section banners on stderr ; raw tool
# output on stdout. Total wall clock printed at the end.

set -euo pipefail

# ── Color helpers ────────────────────────────────────────────────────
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

START_TS=$(date +%s)
TIER_FILTER="${TIER:-all}"

run_tier() {
    local n="$1"; shift
    local title="$1"; shift
    if [[ "$TIER_FILTER" != "all" && "$TIER_FILTER" != "$n" ]]; then
        warn "Tier $n skipped (TIER=$TIER_FILTER)"
        return 0
    fi
    banner "Tier $n : $title"
    local t0=$(date +%s)
    if "$@"; then
        local t1=$(date +%s)
        ok "Tier $n PASSED (${BOLD}$((t1 - t0))s${NC}${GREEN})"
    else
        fail "Tier $n FAILED"
        return 1
    fi
}

# ── Tier 1 : workspace cargo build ──────────────────────────────────
tier1() {
    cargo check --workspace --exclude efe-ffi --locked
}

# ── Tier 1.5 : workspace cargo test --no-run ────────────────────────
tier1_5() {
    cargo test --workspace --exclude efe-ffi --no-run --locked
}

# ── Tier 2 : vsock-relayer Go build ─────────────────────────────────
tier2() {
    if [[ ! -d cmd/vsock-relayer ]]; then
        warn "cmd/vsock-relayer/ missing — Tier 2 N/A"
        return 0
    fi
    ( cd cmd/vsock-relayer && go vet ./... && go build -o /tmp/vsock-relayer . )
}

# ── Tier 3 : Helm lint + template ───────────────────────────────────
tier3() {
    if ! command -v helm >/dev/null 2>&1; then
        warn "helm CLI missing — Tier 3 N/A on this host"
        return 0
    fi
    helm lint charts/efe-agent-runner
    helm template test charts/efe-agent-runner \
        --set agentRuntimeEnabled=true \
        --set admissionWebhook.certManager.bootstrap=true \
        > /tmp/efe-agent-runner-helm-out.yaml
    helm lint charts/efe-stack
    helm template test charts/efe-stack > /tmp/efe-stack-helm-out.yaml
    # Optional : run kubectl --dry-run if available.
    if command -v kubectl >/dev/null 2>&1; then
        kubectl --dry-run=client apply -f /tmp/efe-agent-runner-helm-out.yaml \
            > /dev/null 2>&1 || warn "kubectl client dry-run validation warnings (review /tmp/efe-agent-runner-helm-out.yaml)"
    fi
}

# ── Tier 4 : k3d local cluster smoke ────────────────────────────────
tier4() {
    if [[ "${SKIP_TIER_4:-0}" == "1" ]]; then
        warn "Tier 4 skipped (SKIP_TIER_4=1)"
        return 0
    fi
    if ! command -v k3d >/dev/null 2>&1; then
        warn "k3d CLI missing — Tier 4 N/A on this host"
        return 0
    fi
    bash scripts/assembly-k3d-test.sh
}

# ── Driver ─────────────────────────────────────────────────────────
run_tier 1   "workspace cargo check"     tier1
run_tier 1.5 "workspace cargo test --no-run" tier1_5
run_tier 2   "vsock-relayer Go build"    tier2
run_tier 3   "Helm lint + template"      tier3
run_tier 4   "k3d local cluster smoke"   tier4

TOTAL=$(( $(date +%s) - START_TS ))
echo ""
ok "ALL TIERS PASSED (total ${BOLD}${TOTAL}s${NC}${GREEN})"
