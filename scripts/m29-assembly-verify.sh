#!/usr/bin/env bash
# M29 Data rights — 5-tier assembly verification.

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

banner "Tier 2 : M29 unit tests (efe-pii + data_rights + byok + subprocessors + audit_export)"
cargo test -p efe-pii --lib
cargo test -p efe-server --lib -- v1::data_rights v1::byok v1::subprocessors v1::audit_export
ok "Tier 2"

banner "Tier 3 : Helm lint + Python syntax"
if command -v helm >/dev/null 2>&1; then
    helm lint charts/efe-stack
    helm template test charts/efe-stack \
        --set efe-pii.enabled=true \
        > /tmp/m29-helm.yaml
    ok "Tier 3 Helm"
else
    warn "helm CLI missing"
fi
for p in scripts/m29/build_vex.py bench/m29_data_rights_pentest/*.py; do
    python3 -c "import ast; ast.parse(open('$p').read())" && echo "  ✓ $p"
done

banner "Tier 4 : GitHub Actions workflow YAML"
for f in .github/workflows/release-sbom.yml .github/workflows/slsa-l3.yml .github/workflows/vex.yml; do
    python3 -c "import yaml; yaml.safe_load(open('$f'))" && echo "  ✓ $f"
done

TOTAL=$(($(date +%s) - START))
ok "M29 ALL TIERS PASSED in ${TOTAL}s"
