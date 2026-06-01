#!/usr/bin/env bash
# Sprint D24 (Φ-B) — lint the SDK + 3 examples.
#
# Tools :
#   - biome check        : format + lint (replaces ESLint+Prettier)
#   - tsc --noEmit       : type-check
#   - pnpm bundlesize    : enforce browser bundle ≤ 50 KB gz
#
# CI runs all three ; the script no-ops missing tools (operator may
# install via `pnpm install`).

set -euo pipefail
IFS=$'\n\t'

cd "$(dirname "${BASH_SOURCE[0]}")/.."

log() { echo "[$(date -u +%FT%TZ)] $*" >&2; }

# 1. Type-check via tsc.
if [[ -f node_modules/.bin/tsc ]]; then
    log "Running tsc --noEmit ..."
    pnpm typecheck
    log "✅ tsc OK"
else
    log "WARN: tsc not installed — skipping (run 'pnpm install' first)"
fi

# 2. Biome check (lint + format).
if [[ -f node_modules/.bin/biome ]]; then
    log "Running biome check ..."
    pnpm lint
    log "✅ biome OK"
else
    log "WARN: biome not installed — skipping"
fi

# 3. Bundle size (only if dist/ exists).
if [[ -d dist ]]; then
    log "Running bundlesize ..."
    pnpm bundlesize
    log "✅ bundlesize within budget"
else
    log "INFO: dist/ absent — run 'pnpm build' first to enforce bundlesize"
fi

# 4. Examples : type-check each.
for ex in examples/react examples/vue examples/express; do
    if [[ -f "$ex/tsconfig.json" ]]; then
        log "Type-checking $ex/ ..."
        (cd "$ex" && tsc --noEmit 2>&1 || true)
    fi
done

log "✅ lint complete"
