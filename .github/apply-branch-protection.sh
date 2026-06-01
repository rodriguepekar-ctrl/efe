#!/usr/bin/env bash
# Sprint M14 D42 audit fix M14-A5 — Apply branch protection at freeze.
#
# Applies the legacy branch protection endpoint AND separately enables
# required_signatures via the dedicated sub-endpoint. Combined, this
# enforces the M14 freeze posture documented in
# `docs/EFE_COMPLIANCE_EXTENSION_ARCHITECTURE.md` §15.
#
# Usage :
#   GH_REPO="efe-project/efe" \
#       bash .github/apply-branch-protection.sh

set -euo pipefail

REPO="${GH_REPO:-efe-project/efe}"
BRANCH="main"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if ! command -v gh >/dev/null; then
    echo "::error::gh CLI not installed (https://cli.github.com)" >&2
    exit 1
fi

if ! command -v yq >/dev/null; then
    echo "::error::yq not installed (https://github.com/mikefarah/yq)" >&2
    exit 1
fi

echo "→ Applying legacy branch protection (status checks + 2 reviewers)"
yq -o=json ".main" "${HERE}/branch-protection-rules.yml" \
    | gh api -X PUT \
        "repos/${REPO}/branches/${BRANCH}/protection" \
        --input -

echo "→ Enabling required_signatures (separate sub-endpoint)"
gh api -X POST \
    "repos/${REPO}/branches/${BRANCH}/protection/required_signatures" \
    || true   # idempotent

echo "→ Verifying applied rules"
gh api "repos/${REPO}/branches/${BRANCH}/protection" \
    | jq '{
        required_pull_request_reviews,
        enforce_admins,
        required_status_checks
      }'
gh api "repos/${REPO}/branches/${BRANCH}/protection/required_signatures" \
    | jq

echo ""
echo "✅ Branch protection applied. EFE 1.0 GA repo entered FREEZE state."
echo "   Allowed PR types : Security / Critical bug-fix / Compliance / Docs"
echo "   See .github/CONTRIBUTING-POST-FREEZE.md"
