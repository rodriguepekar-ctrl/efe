#!/usr/bin/env bash
# SPDX-License-Identifier: UNLICENSED
#
# Démo single-binary du Preview Lab : build l'export statique du frontend puis
# lance le bridge `efe-preview-server` qui sert l'UI **et** l'API sur :8700.
#
#   ./preview-lab/run-preview.sh            # mode mock (gratuit)
#   EFE_PREVIEW_MOCK=0 ... real-llm         # vrais LLM (clés requises)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"   # preview-lab/
root="$(cd "$here/.." && pwd)"          # racine workspace

echo "▶ build export statique du frontend…"
( cd "$here" && (pnpm install --ignore-workspace || npm install) && pnpm build || npm run build )

echo "▶ lancement du bridge (sert out/ + API) sur http://127.0.0.1:8700"
EFE_PREVIEW_MOCK="${EFE_PREVIEW_MOCK:-1}" \
EFE_PREVIEW_STATIC_DIR="$here/out" \
  cargo run --manifest-path "$root/Cargo.toml" -p efe-preview-server --release
