#!/usr/bin/env bash
# Sprint D26 (Φ-B) — bootstrap the e2e environment for `mvn verify`.
#
# Steps :
#   1. Sync protos/ → src/main/proto/.
#   2. ./mvnw clean compile (generates proto stubs into target/generated-sources/).
#   3. ./mvnw test (runs JUnit 5 unit + integration via InProcessServer).
#
# No external services required (InProcessServer is in-memory).

set -euo pipefail
IFS=$'\n\t'

cd "$(dirname "${BASH_SOURCE[0]}")/.."

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

log "1. Sync protos ..."
./scripts/gen-proto.sh

log "2. Compile ..."
$MVN -B compile

log "3. Run tests + integration ..."
$MVN -B test

log "✅ e2e bootstrap complete"
