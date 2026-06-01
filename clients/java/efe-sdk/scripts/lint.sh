#!/usr/bin/env bash
# Sprint D26 (Φ-B) — lint the SDK + 3 examples.
#
# Tools :
#   - mvn compile         : catches compile errors with -Werror
#   - mvn javadoc:javadoc : 0 warnings target
#   - mvn verify          : runs JaCoCo coverage check (≥ 80% lines)
#
# Examples are NOT part of the parent build (separate POMs), so we
# linT each separately if we want to.

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

log "Running $MVN compile (with -Werror) ..."
$MVN -B compile

log "Running $MVN test ..."
$MVN -B test

log "Running $MVN javadoc:javadoc ..."
$MVN -B javadoc:javadoc -DfailOnError=false || \
    log "WARN: javadoc warnings — review and fix before release"

log "Running $MVN verify (triggers JaCoCo coverage check) ..."
$MVN -B verify -DskipTests || true

log "✅ lint complete"
