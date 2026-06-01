#!/usr/bin/env bash
# generate-changelog.sh — Auto-generate release notes from git history.
#
# Reads :
#   1. docs/release/RELEASE_NOTES_<version>.md if present (preferred)
#   2. else generates from commit log between previous tag and current
#
# Outputs to stdout. Used by release-v1-ga.yml Phase 4.
#
# Usage :
#   ./scripts/release/generate-changelog.sh v1.0.0

set -euo pipefail

VERSION="${1:?usage: $0 <version>}"
VERSION_NO_V="${VERSION#v}"

# Canonical filename uses the v-prefix (RELEASE_NOTES_v1.0.0.md) ;
# accept the no-prefix variant for backward compatibility.
NOTES_FILE="docs/release/RELEASE_NOTES_${VERSION}.md"
NOTES_FILE_LEGACY="docs/release/RELEASE_NOTES_${VERSION_NO_V}.md"

if [[ -f "$NOTES_FILE" ]]; then
  cat "$NOTES_FILE"
  exit 0
elif [[ -f "$NOTES_FILE_LEGACY" ]]; then
  cat "$NOTES_FILE_LEGACY"
  exit 0
fi

# Fallback : auto-generated.
PREV_TAG=$(git tag --sort=-v:refname | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | grep -v "^${VERSION}$" | head -1 || echo "")

cat <<EOF
# EFE ${VERSION}

Released: $(date -u +%Y-%m-%d)

## Auto-generated changelog

EOF

if [[ -n "$PREV_TAG" ]]; then
  echo "Changes since $PREV_TAG :"
  echo
  git log --no-merges --pretty=format:'- %s (%h)' "${PREV_TAG}..HEAD" \
    | head -100
else
  echo "No previous tag found ; this is the initial release."
fi

cat <<EOF


## Verifying this release

\`\`\`bash
curl -fsSL https://raw.githubusercontent.com/efe-ai/efe/${VERSION}/scripts/release/verify-release.sh \\
  | bash -s -- ${VERSION}
\`\`\`

## Distribution

- Helm : \`helm install efe-stack oci://ghcr.io/efe-ai/charts/efe-stack --version ${VERSION_NO_V}\`
- Docker : \`docker pull ghcr.io/efe-ai/efe-server:${VERSION_NO_V}\`
- CLI (Mac/Linux) : \`brew install efe/tap/efe-admin\`
- CLI (Windows) : direct download from this release page
- npm SDK : \`npm install @efe/sdk@${VERSION_NO_V}\`
- Terraform AWS : \`source = "efe-ai/efe-stack/aws", version = "${VERSION_NO_V}"\`

— The EFE Team
EOF
