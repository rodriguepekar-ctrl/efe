#!/usr/bin/env bash
# Sprint D24 (Φ-B) — bootstrap the e2e environment.
#
# Spawns the 3 processes needed by `pnpm test:e2e` :
#   1. In-memory gRPC mock on :50051 (started by Playwright fixture).
#   2. Envoy gRPC-Web translator on :50052 (Docker).
#   3. Vite dev server for examples/react on :5173.
#
# Each backgrounded ; PIDs written to /tmp/efe-e2e-pids.txt.
# Cleanup : `./scripts/e2e-cleanup.sh` (or simply kill the PIDs).

set -euo pipefail
IFS=$'\n\t'

cd "$(dirname "${BASH_SOURCE[0]}")/.."

log() { echo "[$(date -u +%FT%TZ)] $*" >&2; }

PID_FILE=/tmp/efe-e2e-pids.txt
: > "$PID_FILE"

# 1. Envoy translator.
if command -v docker >/dev/null 2>&1; then
    log "Starting Envoy gRPC-Web translator on :50052..."
    docker run --rm --network host \
        -v "$PWD/tests/e2e/fixtures":/cfg \
        --name efe-e2e-envoy \
        envoyproxy/envoy:v1.29-latest \
        -c /cfg/envoy.yaml --log-level warn &
    echo $! >> "$PID_FILE"
    sleep 3  # give Envoy time to bind.
else
    log "WARN: docker not in PATH — Envoy step skipped (browser e2e will skip)"
fi

# 2. Vite dev server (React example).
if [[ -d examples/react ]]; then
    log "Starting Vite dev server (React example) on :5173..."
    (cd examples/react && pnpm dev > /tmp/efe-vite.log 2>&1) &
    echo $! >> "$PID_FILE"
    sleep 5
else
    log "WARN: examples/react missing — skipping Vite"
fi

log "✅ e2e bootstrap complete"
log "PIDs : $(cat "$PID_FILE" | tr '\n' ' ')"
log
log "Now run :  pnpm test:e2e"
log "Cleanup :  kill \$(cat $PID_FILE)"
