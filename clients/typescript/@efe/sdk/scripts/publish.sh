#!/usr/bin/env bash
# Sprint D24 (Φ-B) — tag + publish @efe/sdk via npm.
#
# Two modes :
#   PUBLISH=private (default) — npm publish --access=restricted on the
#     org-private registry. Used during Φ-B (D24) shipped as v0.3.0.
#   PUBLISH=public            — npm publish --access=public on
#     npmjs.com. ONLY for the GA release v1.0.0.
#
# Prerequisites :
#   - clean working tree (git status empty),
#   - `git tag -l v$VERSION` returns nothing yet,
#   - GPG key available for `git tag -s`,
#   - NPM_TOKEN env var set (or .npmrc with auth).

set -euo pipefail
IFS=$'\n\t'

cd "$(dirname "${BASH_SOURCE[0]}")/.."

PUBLISH="${PUBLISH:-private}"

log() { echo "[$(date -u +%FT%TZ)] $*" >&2; }

# 1. Read version from package.json.
VERSION=$(node -p "require('./package.json').version")
if [[ -z "$VERSION" ]]; then
    log "FATAL: cannot extract version from package.json"
    exit 2
fi
log "SDK version : v$VERSION"
log "Publish mode : $PUBLISH"

# 2. Pre-flight checks.
if [[ -n "$(git status --porcelain)" ]]; then
    log "FATAL: working tree dirty — commit or stash first"
    exit 3
fi
if git rev-parse "v$VERSION" >/dev/null 2>&1; then
    log "FATAL: tag v$VERSION already exists (locally) — bump package.json"
    exit 4
fi

# 3. Build + test before publishing.
log "Installing deps ..."
pnpm install --frozen-lockfile

log "Generating proto stubs (idempotent) ..."
pnpm gen:proto

log "Type-checking ..."
pnpm typecheck

log "Running tests + coverage ..."
pnpm test

log "Building dist/ ..."
pnpm build

log "Checking bundle size ..."
pnpm bundlesize

log "✅ pre-publish checks all GREEN"

# 4. Mode-specific publish.
case "$PUBLISH" in
private)
    log "Publishing v$VERSION to private scoped registry (--access=restricted)..."
    npm publish --access=restricted
    log "Tagging v$VERSION (signed) ..."
    git tag -s "v$VERSION" -m "@efe/sdk v$VERSION (private)"
    log "Pushing tag to origin ..."
    git push origin "v$VERSION"
    log "✅ private publish complete"
    log "   Consumers : pnpm add @efe/sdk@$VERSION  (with @efe:registry .npmrc)"
    ;;
public)
    if [[ ! "$VERSION" =~ ^1\. ]]; then
        log "FATAL: PUBLISH=public requires v1.x.x ; got v$VERSION"
        log "       hint: bump package.json to 1.0.0 before public release"
        exit 5
    fi
    log "Public release flow :"
    log "  1. npm publish --access=public to npmjs.com."
    log "  2. Within seconds, the package resolves at"
    log "     https://www.npmjs.com/package/@efe/sdk"
    npm publish --access=public
    log "Tagging v$VERSION (signed) ..."
    git tag -s "v$VERSION" -m "@efe/sdk v$VERSION (PUBLIC)"
    git push origin "v$VERSION"
    log "✅ public release complete"
    ;;
*)
    log "FATAL: unknown PUBLISH=$PUBLISH (expected private | public)"
    exit 1
    ;;
esac
