#!/usr/bin/env bash
# EFE — Génère les 4 grammars GBNF depuis les JSON Schemas via le script
# Python officiel de llama.cpp (json_schema_to_grammar.py).
#
# Usage :
#   ./scripts/generate-grammars.sh                    # auto-detect llama.cpp source
#   LLAMA_CPP_DIR=~/Tools/llama.cpp ./scripts/generate-grammars.sh
#
# Note : llama.cpp doit être cloné en SOURCE (pas juste le binaire) pour
# avoir accès à examples/json_schema_to_grammar.py.

set -euo pipefail

CRATE_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SCHEMAS_DIR="$CRATE_DIR/assets/schemas"
GRAMMARS_DIR="$CRATE_DIR/assets/grammars"

green=$(tput setaf 2 2>/dev/null || echo "")
red=$(tput setaf 1 2>/dev/null || echo "")
yellow=$(tput setaf 3 2>/dev/null || echo "")
reset=$(tput sgr0 2>/dev/null || echo "")

# ─── Localise json_schema_to_grammar.py ───────────────────────────────
SCRIPT=""
for candidate in \
    "${LLAMA_CPP_DIR:-}/examples/json_schema_to_grammar.py" \
    "$HOME/Tools/llama.cpp/examples/json_schema_to_grammar.py" \
    "$HOME/Tools/llama.cpp-b9297/examples/json_schema_to_grammar.py" \
    "/opt/llama.cpp/examples/json_schema_to_grammar.py"; do
    if [ -f "$candidate" ]; then
        SCRIPT="$candidate"
        break
    fi
done

if [ -z "$SCRIPT" ]; then
    echo "${red}✗${reset} json_schema_to_grammar.py introuvable."
    echo ""
    echo "Clone llama.cpp en SOURCE :"
    echo "  git clone https://github.com/ggml-org/llama.cpp.git ~/Tools/llama.cpp"
    echo ""
    echo "Ou indique le chemin via env :"
    echo "  LLAMA_CPP_DIR=/path/to/llama.cpp ./scripts/generate-grammars.sh"
    exit 1
fi
echo "${green}✓${reset} script trouvé : $SCRIPT"

# ─── Génération pour chaque schéma ────────────────────────────────────
mkdir -p "$GRAMMARS_DIR"

for schema in "$SCHEMAS_DIR"/*.json; do
    base=$(basename "$schema" .json)
    grammar="$GRAMMARS_DIR/${base}.gbnf"
    backup="${grammar}.bak"

    # Backup l'existant (cas où on a raffiné à la main).
    if [ -f "$grammar" ]; then
        cp "$grammar" "$backup"
        echo "${yellow}⚠${reset} ${base}: backup → $(basename "$backup")"
    fi

    # Génère.
    if python3 "$SCRIPT" "$schema" > "${grammar}.new" 2>/tmp/genfailure; then
        mv "${grammar}.new" "$grammar"
        size=$(wc -l < "$grammar")
        echo "${green}✓${reset} ${base}: $size lignes générées"
    else
        rm -f "${grammar}.new"
        echo "${red}✗${reset} ${base}: échec génération"
        cat /tmp/genfailure | head -5
        exit 1
    fi
done

echo ""
echo "${green}Done.${reset} Les fichiers .bak sont vos versions raffinées — diff manuellement :"
echo "  for f in $GRAMMARS_DIR/*.bak; do diff -u \"\$f\" \"\${f%.bak}\"; done"
