#!/usr/bin/env bash
# Sprint D23 (Φ-B) — run the Go test suite with race detector
# + coverage + (optional) live smoke against staging.
#
# Modes :
#   $ ./scripts/test.sh              # unit + integration (race)
#   $ ./scripts/test.sh cover        # + coverage HTML
#   $ ./scripts/test.sh live         # + smoke vs staging.efe.ai
#
# CI invokes the canonical `make test` which calls this script
# without args.

set -euo pipefail
IFS=$'\n\t'

cd "$(dirname "${BASH_SOURCE[0]}")/.."

MODE="${1:-default}"

log() { echo "[$(date -u +%FT%TZ)] $*" >&2; }

if ! command -v go >/dev/null 2>&1; then
    log "FATAL: go not in PATH"
    exit 2
fi

# Detect if -race is supported on the current platform (it is on
# linux/amd64, linux/arm64, darwin/* — but not on plan9, windows/386).
RACE_FLAG="-race"
case "$(go env GOOS)/$(go env GOARCH)" in
    plan9/*|windows/386) RACE_FLAG="" ;;
esac

case "$MODE" in
default|cover)
    log "Running go test $RACE_FLAG -coverprofile=coverage.out ./..."
    go test $RACE_FLAG -coverprofile=coverage.out -covermode=atomic ./...
    log "✅ tests OK"
    if [[ "$MODE" == "cover" ]]; then
        go tool cover -func=coverage.out
        go tool cover -html=coverage.out -o coverage.html
        log "→ open coverage.html"
    fi
    ;;
live)
    log "Running live smoke against staging..."
    if [[ -z "${EFE_ENDPOINT:-}" || -z "${EFE_JWT_TOKEN:-}" ]]; then
        log "FATAL: live mode requires EFE_ENDPOINT + EFE_JWT_TOKEN env vars"
        exit 2
    fi
    go test -tags=live -timeout=60s ./tests/...
    log "✅ live smoke OK"
    ;;
*)
    log "FATAL: unknown mode $MODE (expected default | cover | live)"
    exit 1
    ;;
esac
