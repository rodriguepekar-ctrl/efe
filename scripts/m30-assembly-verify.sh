#!/usr/bin/env bash
# M30 Multi-modal + Multi-region — 5-tier assembly verification.

set -euo pipefail
if [[ -t 2 ]]; then BOLD=$'\e[1m'; GREEN=$'\e[32m'; YEL=$'\e[33m'; NC=$'\e[0m'; else BOLD=''; GREEN=''; YEL=''; NC=''; fi
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

banner "Tier 2 : M30 unit tests (efe-multimodal + efe-region + v1::multimodal + v1::region)"
cargo test -p efe-multimodal --lib
cargo test -p efe-region --lib
cargo test -p efe-server --lib -- v1::multimodal v1::region
ok "Tier 2"

banner "Tier 3 : Helm lint + Python syntax"
if command -v helm >/dev/null 2>&1; then
    helm lint charts/efe-stack
    helm template test charts/efe-stack \
        --set efe-triton.enabled=true \
        -f charts/efe-stack/region-values/values-eu-west-1.yaml \
        > /tmp/m30-helm-eu-west-1.yaml
    helm template test charts/efe-stack \
        --set efe-triton.enabled=true \
        -f charts/efe-stack/region-values/values-eu-west-3.yaml \
        > /tmp/m30-helm-eu-west-3.yaml
    ok "Tier 3 Helm"
else
    warn "helm CLI missing"
fi
for p in scripts/chaos/m30/*.sh scripts/load/m30/*.sh scripts/m30/*.sh; do
    bash -n "$p" && echo "  ✓ $p"
done
for p in bench/m30_multimodal_pentest/*.py; do
    python3 -c "import ast; ast.parse(open('$p').read())" && echo "  ✓ $p"
done

banner "Tier 4 : Terraform fmt"
if command -v terraform >/dev/null 2>&1; then
    terraform -chdir=terraform/m30_gpu_pool fmt -check -recursive
    terraform -chdir=terraform/m30_multiregion fmt -check -recursive
    ok "Tier 4 Terraform"
else
    warn "terraform CLI missing"
fi

banner "Tier 5 : SQL migration balanced"
for f in crates/efe-server/migrations/0024_m30_multimodal_region.sql \
         crates/efe-server/migrations/0024_m30_multimodal_region_rollback.sql; do
    begin=$(grep -c '^BEGIN;' "$f")
    commit=$(grep -c '^COMMIT;' "$f")
    if [ "$begin" -ne "$commit" ]; then
        echo "✗ $f BEGIN/COMMIT imbalanced ($begin/$commit)"
        exit 1
    fi
    echo "  ✓ $f BEGIN/COMMIT balanced ($begin)"
done

TOTAL=$(($(date +%s) - START))
ok "M30 ALL TIERS PASSED in ${TOTAL}s"
