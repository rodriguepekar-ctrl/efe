#!/bin/bash
# ─────────────────────────────────────────────────────────────────────
#  scripts/efe-exp6-run.sh
#
#  EXP 6 — Agent IA branché à EFE (orchestrator bash)
#
#  Wraps : python3.12 scripts/efe-exp6-agent.py
#
#  Companion :
#    docs/marketing/EFE-demo-EXP6-agent-2026-05-18.md
#    scripts/efe-exp6-agent.py
#
#  Contrat d'additivité STRICT :
#    - L'agent ne modifie aucun fichier EFE source
#    - Tous les outputs vont dans /tmp/efe-agent-output/
#    - Le repo git status est identique avant/après le run
#
#  Usage :
#    cd /Users/rodrigue.pekar/Downloads/jjjj/efe
#    bash scripts/efe-exp6-run.sh
# ─────────────────────────────────────────────────────────────────────

set -u

if [ "$(pwd)" != "/Users/rodrigue.pekar/Downloads/jjjj/efe" ]; then
  echo "ERROR: must run from /Users/rodrigue.pekar/Downloads/jjjj/efe"
  exit 1
fi

bold='\033[1m'
green='\033[32m'
yellow='\033[33m'
red='\033[31m'
cyan='\033[36m'
nc='\033[0m'

# Pre-flight : tools required
for cmd in python3.12 cargo bash git; do
  if ! command -v $cmd >/dev/null; then
    printf "  ${red}✗${nc} missing : %s\n" "$cmd"
    exit 1
  fi
done

# Pre-flight : binaries pre-built (agent ne build pas, c'est explicite)
for bin in efe-bench-harness efe-bench efe-smoke; do
  if [ ! -x "./target/debug/$bin" ]; then
    printf "  ${red}✗${nc} binary missing : target/debug/%s\n" "$bin"
    printf "  build with : cargo build -p efe-bench-harness --bin %s\n" "$bin"
    exit 1
  fi
done

printf "\n${bold}═══════════════════════════════════════════════════════════════${nc}\n"
printf "${bold}  EXP 6 — Agent branché à EFE${nc}\n"
printf "${bold}  $(date -u +%Y-%m-%dT%H:%M:%SZ)${nc}\n"
printf "${bold}═══════════════════════════════════════════════════════════════${nc}\n"

# Step 0 — snapshot du git status AVANT le run (le contract d'additivité)
SNAPSHOT_BEFORE=$(mktemp /tmp/efe-exp6-status-before.XXXXXX)
git status --short > "$SNAPSHOT_BEFORE"
LINES_BEFORE=$(wc -l < "$SNAPSHOT_BEFORE" | tr -d ' ')
printf "${cyan}  Snapshot git status AVANT (= %s lignes)${nc}\n\n" "$LINES_BEFORE"

# Step 1 — Run l'agent Python
python3.12 scripts/efe-exp6-agent.py

# Step 2 — Vérification finale d'additivité (outside-of-agent perspective)
printf "\n${bold}━━ Vérification additivité (perspective bash externe) ━━${nc}\n"
SNAPSHOT_AFTER=$(mktemp /tmp/efe-exp6-status-after.XXXXXX)
git status --short > "$SNAPSHOT_AFTER"
LINES_AFTER=$(wc -l < "$SNAPSHOT_AFTER" | tr -d ' ')

if diff -q "$SNAPSHOT_BEFORE" "$SNAPSHOT_AFTER" >/dev/null; then
  printf "  ${green}✓${nc} git status identique avant (%s lignes) / après (%s lignes)\n" "$LINES_BEFORE" "$LINES_AFTER"
  printf "  ${green}✓${nc} L'agent n'a modifié AUCUN fichier du repo EFE.\n"
else
  printf "  ${red}✗${nc} git status a changé pendant le run !\n"
  printf "  Diff :\n"
  diff "$SNAPSHOT_BEFORE" "$SNAPSHOT_AFTER" | head -20 | sed 's/^/    /'
fi

# Step 3 — Inventaire des artefacts agent (DOIVENT être uniquement dans /tmp/efe-agent-output/)
printf "\n${bold}━━ Artefacts produits par l'agent (uniquement /tmp/efe-agent-output/) ━━${nc}\n"
if [ -d /tmp/efe-agent-output ]; then
  ls -la /tmp/efe-agent-output/ | tail -n +2 | awk '{printf "  %s   %s bytes\n", $NF, $5}'
fi

# Cleanup
rm -f "$SNAPSHOT_BEFORE" "$SNAPSHOT_AFTER"

printf "\n${bold}═══════════════════════════════════════════════════════════════${nc}\n"
printf "${bold}${green}  EXP 6 complete.${nc}\n"
printf "${bold}  See docs/marketing/EFE-demo-EXP6-agent-2026-05-18.md${nc}\n"
printf "${bold}═══════════════════════════════════════════════════════════════${nc}\n"
