#!/usr/bin/env bash
# EFE Playground — one-command quickstart.
#
# Usage :
#   bash playground/scripts/quickstart.sh

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PLAYGROUND="$(dirname "${HERE}")"
REPO="$(dirname "${PLAYGROUND}")"

echo "╔═══════════════════════════════════════════════════════════════╗"
echo "║  EFE Playground — quickstart                                  ║"
echo "║                                                               ║"
echo "║  Try EFE 1.0 GA locally in your browser. No K8s required.     ║"
echo "║  Falls back to deterministic hash embedder if you don't have  ║"
echo "║  sentence-transformers installed (offline-friendly).          ║"
echo "╚═══════════════════════════════════════════════════════════════╝"
echo ""

# 1. Check Python
if ! command -v python3 >/dev/null; then
    echo "::error::python3 not installed (>= 3.10 required)"
    exit 1
fi

PYTHON_VER=$(python3 -c "import sys; print('.'.join(map(str, sys.version_info[:2])))")
echo "✓ Python ${PYTHON_VER}"

# 2. Set up venv (idempotent)
VENV="${PLAYGROUND}/.venv"
if [ ! -d "${VENV}" ]; then
    echo "→ Creating virtualenv at ${VENV}"
    python3 -m venv "${VENV}"
fi
# shellcheck disable=SC1091
source "${VENV}/bin/activate"

# 3. Install required deps
echo "→ Installing playground dependencies (Flask + Flask-CORS + numpy)"
pip install -q --upgrade pip
pip install -q -r "${PLAYGROUND}/backend/requirements.txt"

# 4. Optional : install sentence-transformers if requested
if [ "${EFE_PLAYGROUND_REAL_EMBEDDER:-no}" = "yes" ]; then
    echo "→ Installing sentence-transformers (real semantic embeddings ~500 MiB)"
    pip install -q "sentence-transformers>=2.2,<3.0" "torch>=2.0,<3.0"
fi

# 5. Quick port availability check
PORT="${PORT:-8765}"
if lsof -i :${PORT} >/dev/null 2>&1; then
    echo "::warn::Port ${PORT} is already in use. Set PORT=<other> to override."
    exit 1
fi

echo ""
echo "→ Starting backend on http://127.0.0.1:${PORT}/"
echo "→ Open http://127.0.0.1:${PORT}/ in your browser"
echo "→ Ctrl+C to stop"
echo ""

# 6. Run backend
exec python3 "${PLAYGROUND}/backend/app.py" --host 127.0.0.1 --port "${PORT}"
