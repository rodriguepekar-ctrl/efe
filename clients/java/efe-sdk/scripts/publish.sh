#!/usr/bin/env bash
# Sprint D26 (Φ-B) — publish efe-sdk-java via Maven.
#
# Two modes :
#   PUBLISH=private (default) — deploy to GitHub Packages (Φ-B private).
#   PUBLISH=public            — deploy to Maven Central (GA, requires v1.x.x).
#
# Pre-requisites :
#   - clean git tree.
#   - GPG key available (for `release` profile signing) ; expected via
#     env vars GPG_PRIVATE_KEY + GPG_PASSPHRASE in CI.
#   - For private mode : GITHUB_TOKEN env var with packages:write scope.
#   - For public mode  : Sonatype Central Portal token in ~/.m2/settings.xml
#                         (server id "central").

set -euo pipefail
IFS=$'\n\t'

cd "$(dirname "${BASH_SOURCE[0]}")/.."

PUBLISH="${PUBLISH:-private}"

log() { echo "[$(date -u +%FT%TZ)] $*" >&2; }

MVN="./mvnw"
if [[ ! -x "$MVN" ]]; then
    if command -v mvn >/dev/null 2>&1; then
        MVN="mvn"
    else
        log "FATAL: neither mvnw nor mvn in PATH"
        exit 2
    fi
fi

# 1. Read version from parent POM.
VERSION=$($MVN -q -Dexec.executable=echo -Dexec.args='${project.version}' \
    --non-recursive exec:exec 2>/dev/null || \
    grep -m1 '<version>' pom.xml | sed -E 's/.*<version>([^<]+)<\/version>.*/\1/')
log "SDK version : v$VERSION"
log "Publish mode : $PUBLISH"

# 2. Pre-flight checks.
if [[ -n "$(git status --porcelain)" ]]; then
    log "FATAL: working tree dirty — commit or stash first"
    exit 3
fi
if git rev-parse "v$VERSION" >/dev/null 2>&1; then
    log "FATAL: tag v$VERSION already exists — bump pom.xml first"
    exit 4
fi

# 3. Build + test.
log "Running $MVN clean install ..."
$MVN clean install

# 4. Mode-specific publish.
case "$PUBLISH" in
private)
    log "Publishing v$VERSION to GitHub Packages (private)..."
    $MVN -DskipTests deploy \
        -DaltDeploymentRepository=github-private::default::https://maven.pkg.github.com/efe-ai/efe
    log "Tagging v$VERSION (signed) ..."
    git tag -s "v$VERSION" -m "efe-sdk-java v$VERSION (private)"
    git push origin "v$VERSION"
    log "✅ private publish complete"
    log "   Consumers : <dependency> ai.efe / efe-sdk-java / $VERSION (with @efe scoped repo)"
    ;;
public)
    if [[ ! "$VERSION" =~ ^1\. ]]; then
        log "FATAL: PUBLISH=public requires v1.x.x ; got v$VERSION"
        log "       hint: bump pom.xml to 1.0.0 before public release"
        exit 5
    fi
    log "Public release flow (Maven Central via Sonatype Central Portal) :"
    log "  1. mvn deploy -P release  →  signs JARs (GPG) + stages on portal"
    log "  2. central-publishing-maven-plugin auto-publishes within ~30 min"
    $MVN -DskipTests -P release deploy
    log "Tagging v$VERSION (signed) ..."
    git tag -s "v$VERSION" -m "efe-sdk-java v$VERSION (PUBLIC)"
    git push origin "v$VERSION"
    log "✅ public release complete — wait ~30 min for Maven Central sync"
    ;;
*)
    log "FATAL: unknown PUBLISH=$PUBLISH (expected private | public)"
    exit 1
    ;;
esac
