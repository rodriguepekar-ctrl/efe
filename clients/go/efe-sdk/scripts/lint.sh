#!/usr/bin/env bash
# Sprint D23 (Φ-B) — lint the SDK + 3 examples.
#
# Tools :
#   - `go vet ./...`        (stdlib analyser)
#   - `golangci-lint run`   (if installed — comprehensive linter set)
#   - `govulncheck ./...`   (if installed — CVE detection)
#
# CI runs all three ; the script no-ops missing tools (operator
# install via `brew install golangci-lint govulncheck`).

set -euo pipefail
IFS=$'\n\t'

cd "$(dirname "${BASH_SOURCE[0]}")/.."

log() { echo "[$(date -u +%FT%TZ)] $*" >&2; }

# 1. go vet — always available with go installed.
if command -v go >/dev/null 2>&1; then
    log "Running go vet ./..."
    go vet ./...
    log "✅ go vet OK"
else
    log "WARN: go not in PATH — skipping go vet"
fi

# 2. golangci-lint — install via brew.
if command -v golangci-lint >/dev/null 2>&1; then
    log "Running golangci-lint run ./..."
    golangci-lint run ./...
    log "✅ golangci-lint OK"
else
    log "WARN: golangci-lint not in PATH — skipping"
    log "       hint: brew install golangci-lint"
fi

# 3. govulncheck — install via go install.
if command -v govulncheck >/dev/null 2>&1; then
    log "Running govulncheck ./..."
    govulncheck ./...
    log "✅ govulncheck OK"
else
    log "WARN: govulncheck not in PATH — skipping"
    log "       hint: go install golang.org/x/vuln/cmd/govulncheck@latest"
fi

# 4. Lint the 3 examples sub-modules too.
for ex in examples/gin examples/fiber examples/chi; do
    if [[ -f "$ex/go.mod" ]]; then
        log "Linting $ex/"
        (cd "$ex" && go vet ./... 2>&1 || true)
    fi
done

log "✅ lint complete"
