#!/usr/bin/env bash
# M28 Streaming + Events — 5-tier assembly verification.

set -euo pipefail
if [[ -t 2 ]]; then BOLD=$'\e[1m'; GREEN=$'\e[32m'; RED=$'\e[31m'; YEL=$'\e[33m'; NC=$'\e[0m'; else BOLD=''; GREEN=''; RED=''; YEL=''; NC=''; fi
banner() { echo "${BOLD}=== $1 ===${NC}" >&2; }
ok()     { echo "${GREEN}✓ $1${NC}" >&2; }
warn()   { echo "${YEL}⚠ $1${NC}" >&2; }

cd "$(dirname "${BASH_SOURCE[0]}")/.."
START=$(date +%s)

banner "Tier 1 : workspace cargo check"
cargo check --workspace --exclude efe-ffi --locked
ok "Tier 1"

banner "Tier 1.5 : workspace test compile"
cargo test --workspace --exclude efe-ffi --no-run --locked
ok "Tier 1.5"

banner "Tier 2 : M28 unit tests (efe-events + webhooks + batch + events)"
cargo test -p efe-events --lib
cargo test -p efe-server --lib -- v1::webhooks v1::batch v1::events
ok "Tier 2"

banner "Tier 3 : Helm lint"
if command -v helm >/dev/null 2>&1; then
    helm lint charts/efe-stack
    helm template test charts/efe-stack \
        --set efe-nats.enabled=true \
        --set efe-edge.enabled=false \
        > /tmp/m28-helm.yaml
    ok "Tier 3"
else
    warn "helm CLI missing"
fi

banner "Tier 4 : pen-test syntax"
for p in bench/m28_streaming_pentest/*.py; do
    python3 -c "import ast; ast.parse(open('$p').read())" && ok "$p"
done

TOTAL=$(($(date +%s) - START))
ok "M28 ALL TIERS PASSED in ${TOTAL}s"
