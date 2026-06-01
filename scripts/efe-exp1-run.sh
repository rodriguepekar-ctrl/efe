#!/bin/bash
# ─────────────────────────────────────────────────────────────────────
#  scripts/efe-exp1-run.sh
#
#  EXP 1 — Pre vs Post G.11 NDJSON analysis (5 min)
#
#  Companion : docs/marketing/EFE-demo-EXP1-prepared-2026-05-17.md
#  All outputs verified live on 2026-05-17.
#
#  Usage :
#    cd /Users/rodrigue.pekar/Downloads/jjjj/efe
#    bash scripts/efe-exp1-run.sh
# ─────────────────────────────────────────────────────────────────────

set -u

# CWD safety
if [ "$(pwd)" != "/Users/rodrigue.pekar/Downloads/jjjj/efe" ]; then
  echo "ERROR: must run from /Users/rodrigue.pekar/Downloads/jjjj/efe"
  echo "  current: $(pwd)"
  exit 1
fi

# Tool safety
if ! command -v jq >/dev/null; then
  echo "ERROR: jq is required.  Install with: brew install jq"
  exit 1
fi

bold='\033[1m'
green='\033[32m'
yellow='\033[33m'
nc='\033[0m'

section() { printf "\n${bold}── %s ──${nc}\n" "$1" ; }

printf "\n${bold}═══════════════════════════════════════════════════════════════${nc}\n"
printf "${bold}  EXP 1 — Pre vs Post G.11 NDJSON analysis${nc}\n"
printf "${bold}  $(date -u +%Y-%m-%dT%H:%M:%SZ)${nc}\n"
printf "${bold}═══════════════════════════════════════════════════════════════${nc}\n"

# ─────────────────────────────────────────────────────────────────────
section "Step 1.1 — One pre-G.11 row (opaque)"
# ─────────────────────────────────────────────────────────────────────
head -1 bench/fixtures/pre_g11_ndjson/case_a_warn_null_tolerance.ndjson \
  | jq '{schema_version, scenario_id, verdict, tolerance_used, "tolerance.source": .tolerance.source}'

# ─────────────────────────────────────────────────────────────────────
section "Step 1.2 — One post-G.11 row (traceable)"
# ─────────────────────────────────────────────────────────────────────
head -1 bench/fixtures/post_g11_ndjson/CB-01.ndjson \
  | jq '{schema_version, scenario_id, verdict, tolerance_used}'

# ─────────────────────────────────────────────────────────────────────
section "Step 1.3 — Aggregate all pre-G.11 rows"
# ─────────────────────────────────────────────────────────────────────
for f in bench/fixtures/pre_g11_ndjson/*.ndjson; do
  jq -r '"\(.scenario_id) verdict=\(.verdict) tolerance_used=\(.tolerance_used)"' "$f"
done

# ─────────────────────────────────────────────────────────────────────
section "Step 1.4 — Aggregate all post-G.11 rows"
# ─────────────────────────────────────────────────────────────────────
for f in bench/fixtures/post_g11_ndjson/*.ndjson; do
  jq -r '"\(.scenario_id) verdict=\(.verdict) tolerance_used.value=\(.tolerance_used.value) source=\(.tolerance_used.source)"' "$f"
done

# ─────────────────────────────────────────────────────────────────────
section "Step 1.5 — Verdict distributions"
# ─────────────────────────────────────────────────────────────────────
printf "${yellow}▸ Verdicts pre-G.11 :${nc}\n"
for f in bench/fixtures/pre_g11_ndjson/*.ndjson; do
  jq -r '.verdict' "$f"
done | sort | uniq -c

printf "\n${yellow}▸ Verdicts post-G.11 :${nc}\n"
for f in bench/fixtures/post_g11_ndjson/*.ndjson; do
  jq -r '.verdict' "$f"
done | sort | uniq -c

# ─────────────────────────────────────────────────────────────────────
section "Step 1.6 — Source taxonomy distribution (post-G.11)"
# ─────────────────────────────────────────────────────────────────────
for f in bench/fixtures/post_g11_ndjson/*.ndjson; do
  jq -r '.tolerance_used.source // "UNSPECIFIED"' "$f"
done | sort | uniq -c

# ─────────────────────────────────────────────────────────────────────
section "Step 1.7 — Row counts"
# ─────────────────────────────────────────────────────────────────────
pre_count=0
for f in bench/fixtures/pre_g11_ndjson/*.ndjson; do
  pre_count=$((pre_count + $(wc -l < "$f")))
done
post_count=0
for f in bench/fixtures/post_g11_ndjson/*.ndjson; do
  post_count=$((post_count + $(wc -l < "$f")))
done
echo "  pre-G.11  : $pre_count rows / $(ls bench/fixtures/pre_g11_ndjson/*.ndjson | wc -l) files"
echo "  post-G.11 : $post_count rows / $(ls bench/fixtures/post_g11_ndjson/*.ndjson | wc -l) files"

printf "\n${bold}═══════════════════════════════════════════════════════════════${nc}\n"
printf "${bold}${green}  EXP 1 complete.${nc}\n"
printf "${bold}  See docs/marketing/EFE-demo-EXP1-prepared-2026-05-17.md for interpretation.${nc}\n"
printf "${bold}═══════════════════════════════════════════════════════════════${nc}\n"
