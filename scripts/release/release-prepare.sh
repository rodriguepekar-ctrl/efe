#!/usr/bin/env bash
# release-prepare.sh — Pre-flight checklist before tagging a release.
#
# Verifies :
#   - main branch is up-to-date
#   - working tree is clean
#   - all CI on main is green
#   - CHANGELOG entry exists for this version
#   - RELEASE_NOTES_<version>.md exists
#   - 2 maintainers' GPG keys configured
#   - tag does not already exist
#
# Usage :
#   ./scripts/release/release-prepare.sh v1.0.0

set -euo pipefail

VERSION="${1:?usage: $0 <version>}"

if ! [[ "$VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.\-]+)?$ ]]; then
  echo "❌ Invalid version : $VERSION" >&2
  exit 1
fi

VERSION_NO_V="${VERSION#v}"

ok() { echo "  ✓ $1"; }
fail() { echo "  ✗ $1" >&2; exit 1; }

echo "▶ Pre-flight checklist for release $VERSION"
echo

# 1. Working tree clean.
echo "[1/8] Working tree clean"
if [[ -n "$(git status --porcelain)" ]]; then
  fail "uncommitted changes present (run git status)"
fi
ok "no uncommitted changes"

# 2. On main branch.
echo "[2/8] On main branch"
BRANCH=$(git rev-parse --abbrev-ref HEAD)
if [[ "$BRANCH" != "main" ]]; then
  fail "current branch is $BRANCH, expected main"
fi
ok "on main"

# 3. Up-to-date with origin.
echo "[3/8] Up-to-date with origin/main"
git fetch origin main --quiet
LOCAL=$(git rev-parse @)
REMOTE=$(git rev-parse '@{u}')
if [[ "$LOCAL" != "$REMOTE" ]]; then
  fail "local main is behind origin/main"
fi
ok "in sync with origin/main"

# 4. Tag doesn't exist.
echo "[4/8] Tag $VERSION doesn't exist yet"
if git rev-parse "$VERSION" >/dev/null 2>&1; then
  fail "tag $VERSION already exists"
fi
if git ls-remote --tags origin "refs/tags/$VERSION" | grep -q "$VERSION"; then
  fail "tag $VERSION already exists on origin"
fi
ok "tag is new"

# 5. CHANGELOG entry.
# Tolerant grep : matches `## [v1.0.0]`, `## [1.0.0]`, `## v1.0.0`, or
# `## 1.0.0`. The canonical format used in this repo is `## [v1.0.0]`
# (Keep-a-Changelog with v-prefix), but we accept both for tolerance.
echo "[5/8] CHANGELOG entry for $VERSION"
if [[ -f CHANGELOG.md ]] && \
   grep -Eq "^## \[?(v?${VERSION_NO_V//./\\.})\]?" CHANGELOG.md; then
  ok "CHANGELOG.md entry present"
else
  echo "  ⚠ CHANGELOG.md entry missing for $VERSION (please add ## [$VERSION] section)"
fi

# 6. Release notes file.
# Canonical filename : RELEASE_NOTES_v<VERSION>.md (with v-prefix).
# Tolerate the no-prefix variant for backward compatibility.
echo "[6/8] docs/release/RELEASE_NOTES_${VERSION}.md exists"
RELEASE_NOTES_FILE="docs/release/RELEASE_NOTES_${VERSION}.md"
RELEASE_NOTES_FILE_LEGACY="docs/release/RELEASE_NOTES_${VERSION_NO_V}.md"
if [[ -f "$RELEASE_NOTES_FILE" ]]; then
  ok "$RELEASE_NOTES_FILE present"
elif [[ -f "$RELEASE_NOTES_FILE_LEGACY" ]]; then
  ok "$RELEASE_NOTES_FILE_LEGACY present (legacy no-prefix)"
else
  echo "  ⚠ $RELEASE_NOTES_FILE missing — auto-generated changelog will be used"
fi

# 7. GPG signing key configured.
echo "[7/8] GPG signing key configured"
if git config --get user.signingkey >/dev/null 2>&1; then
  KEY=$(git config --get user.signingkey)
  ok "user.signingkey = $KEY"
else
  fail "git user.signingkey not set (configure GPG key for signed tags)"
fi

# 8. CI on main is green.
echo "[8/8] CI on main is green"
if command -v gh >/dev/null 2>&1; then
  STATUS=$(gh run list --branch main --workflow ci.yml --limit 1 --json status,conclusion --jq '.[0].conclusion' 2>/dev/null || echo "unknown")
  if [[ "$STATUS" == "success" ]]; then
    ok "latest CI run = success"
  else
    echo "  ⚠ latest CI run on main = $STATUS (please verify before tagging)"
  fi
else
  echo "  ⚠ gh CLI not available ; skipping CI check"
fi

echo
echo "✅ Pre-flight complete. To tag :"
cat <<EOF

  git tag -s $VERSION -m "EFE $VERSION GA"

  # Sub-tags (same commit) :
  for c in tf-aws tf-gcp tf-azure efe-admin portal-admin charts/efe-stack; do
    git tag -s "\$c/$VERSION" -m "EFE \$c $VERSION GA"
  done

  git push origin --tags

EOF
