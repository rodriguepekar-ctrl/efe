#!/usr/bin/env bash
# Sprint D23 (Φ-B) — tag + push the SDK module.
#
# Two modes :
#   PUBLISH=private (default) — push to the private org repo. Used
#     during Φ-B (D23) shipped as `v0.3.0`.
#   PUBLISH=public            — push the public proxy entry. ONLY
#     for the GA release `v1.0.0`.
#
# Prerequisites :
#   - clean working tree (`git status` empty),
#   - `git tag -l v$VERSION` returns nothing yet,
#   - GPG key available for `git tag -s`.

set -euo pipefail
IFS=$'\n\t'

cd "$(dirname "${BASH_SOURCE[0]}")/.."

PUBLISH="${PUBLISH:-private}"
VERSION_FILE="version.go"

log() { echo "[$(date -u +%FT%TZ)] $*" >&2; }

# 1. Read version from version.go.
VERSION=$(grep -E '^const Version = ' "$VERSION_FILE" \
    | sed -E 's/^const Version = "([^"]+)"$/\1/')
if [[ -z "$VERSION" ]]; then
    log "FATAL: cannot extract Version from $VERSION_FILE"
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
    log "FATAL: tag v$VERSION already exists (locally) — bump version.go"
    exit 4
fi

# 3. Build + test before tagging.
log "Running tests..."
go test -race ./...
log "✅ tests OK"

# 4. Mode-specific publish.
case "$PUBLISH" in
private)
    log "Tagging v$VERSION (signed)..."
    git tag -s "v$VERSION" -m "EFE SDK Go v$VERSION (private)"
    log "Pushing to private origin..."
    git push origin "v$VERSION"
    log "✅ private publish complete"
    log "   Consumers : GOPRIVATE=github.com/efe-ai/* go get github.com/efe-ai/efe-sdk-go@v$VERSION"
    ;;
public)
    if [[ ! "$VERSION" =~ ^1\. ]]; then
        log "FATAL: PUBLISH=public requires v1.x.x ; got v$VERSION"
        log "       hint: bump version.go to 1.0.0 before public release"
        exit 5
    fi
    log "Public release flow :"
    log "  1. Tag is created in the org private repo as usual."
    log "  2. The Go module proxy (proxy.golang.org) auto-fetches"
    log "     once the repo is set to public."
    log "  3. Within ~30 min, `go get github.com/efe-ai/efe-sdk-go@v$VERSION`"
    log "     resolves through the proxy."
    git tag -s "v$VERSION" -m "EFE SDK Go v$VERSION (PUBLIC)"
    git push origin "v$VERSION"
    log "✅ public release tag pushed"
    log "   Now switch the GitHub repo visibility to PUBLIC manually."
    ;;
*)
    log "FATAL: unknown PUBLISH=$PUBLISH (expected private | public)"
    exit 1
    ;;
esac
