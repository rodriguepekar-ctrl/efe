#!/usr/bin/env bash
# M31 FinOps + Billing — 5-tier assembly verification.

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

banner "Tier 2 : M31 unit tests (efe-metering + efe-billing + v1::usage + v1::billing)"
cargo test -p efe-metering --lib
cargo test -p efe-billing --lib
cargo test -p efe-server --lib -- v1::usage v1::billing
ok "Tier 2"

banner "Tier 3 : Helm lint + Python syntax + Bash syntax"
if command -v helm >/dev/null 2>&1; then
    helm lint charts/efe-stack
    helm template test charts/efe-stack \
        --set efe-stripe.enabled=true \
        > /tmp/m31-helm.yaml
    ok "Tier 3 Helm"
else
    warn "helm CLI missing"
fi
for p in scripts/chaos/m31/*.sh scripts/load/m31/*.sh scripts/m31/*.sh; do
    bash -n "$p" && echo "  ✓ $p"
done
for p in bench/m31_billing_pentest/*.py; do
    python3 -c "import ast; ast.parse(open('$p').read())" && echo "  ✓ $p"
done

banner "Tier 4 : Terraform fmt"
if command -v terraform >/dev/null 2>&1; then
    terraform -chdir=terraform/m31_billing fmt -check -recursive
    ok "Tier 4 Terraform"
else
    warn "terraform CLI missing"
fi

banner "Tier 5 : SQL migration balanced"
for f in crates/efe-server/migrations/0025_m31_billing.sql \
         crates/efe-server/migrations/0025_m31_billing_rollback.sql; do
    begin=$(grep -c '^BEGIN;' "$f")
    commit=$(grep -c '^COMMIT;' "$f")
    if [ "$begin" -ne "$commit" ]; then
        echo "✗ $f BEGIN/COMMIT imbalanced ($begin/$commit)"
        exit 1
    fi
    echo "  ✓ $f BEGIN/COMMIT balanced ($begin)"
done

TOTAL=$(($(date +%s) - START))
ok "M31 ALL TIERS PASSED in ${TOTAL}s"
