#!/usr/bin/env bash
# release-publish.sh — Push tags + sub-tags atomically and trigger workflow.
#
# Use this script ONLY after release-prepare.sh has passed.
#
# Usage :
#   ./scripts/release/release-publish.sh v1.0.0

set -euo pipefail

VERSION="${1:?usage: $0 <version>}"

if ! [[ "$VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.\-]+)?$ ]]; then
  echo "❌ Invalid version : $VERSION" >&2
  exit 1
fi

# Pre-flight one more time.
./scripts/release/release-prepare.sh "$VERSION"

echo
echo "▶ Creating signed tags for $VERSION"

git tag -s "$VERSION" -m "EFE $VERSION GA"
echo "  ✓ tagged $VERSION"

for sub in tf-aws tf-gcp tf-azure efe-admin portal-admin charts/efe-stack; do
  git tag -s "${sub}/${VERSION}" -m "EFE ${sub} ${VERSION} GA"
  echo "  ✓ tagged ${sub}/${VERSION}"
done

echo
echo "▶ Pushing all tags to origin"
git push origin --tags
echo "  ✓ pushed"

echo
echo "✅ Tags pushed. The release-v1-ga.yml workflow will now run."
echo "   Monitor at : https://github.com/efe-ai/efe/actions/workflows/release-v1-ga.yml"
echo
echo "After the workflow succeeds, run :"
echo "   ./scripts/release/verify-release.sh $VERSION"
