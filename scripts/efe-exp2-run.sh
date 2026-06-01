#!/bin/bash
# ─────────────────────────────────────────────────────────────────────
#  scripts/efe-exp2-run.sh
#
#  EXP 2 — Backfill réel --write : EFE reconstruit les données (5-7 min)
#
#  Companion : docs/marketing/EFE-demo-EXP2-prepared-2026-05-17.md
#  All outputs verified live on 2026-05-17.
#
#  Steps :
#    2.0  Setup workdir frais
#    2.1  Inventaire AVANT
#    2.2  Inspect case_a AVANT
#    2.3  Run backfill --write
#    2.4  Inventaire APRÈS
#    2.5  Inspect case_a APRÈS (la transformation)
#    2.6  Aggregate 7 rows reconstruites
#    2.7  Distribution sources
#    2.8  Idempotence (run 2x = même sha256)
#    2.9  Byte-identical contre golden
#    2.10 Cleanup
#
#  Usage :
#    cd /Users/rodrigue.pekar/Downloads/jjjj/efe
#    bash scripts/efe-exp2-run.sh
# ─────────────────────────────────────────────────────────────────────

set -u

if [ "$(pwd)" != "/Users/rodrigue.pekar/Downloads/jjjj/efe" ]; then
  echo "ERROR: must run from /Users/rodrigue.pekar/Downloads/jjjj/efe"
  echo "  current: $(pwd)"
  exit 1
fi

if ! command -v jq >/dev/null; then
  echo "ERROR: jq required (brew install jq)"; exit 1
fi

if command -v shasum >/dev/null; then
  SHA="shasum -a 256"
elif command -v sha256sum >/dev/null; then
  SHA="sha256sum"
else
  echo "ERROR: need shasum or sha256sum"; exit 1
fi

bold='\033[1m'
green='\033[32m'
yellow='\033[33m'
red='\033[31m'
nc='\033[0m'

section() { printf "\n${bold}── %s ──${nc}\n" "$1" ; }
ok() { printf "  ${green}✓${nc} %s\n" "$1" ; }
fail() { printf "  ${red}✗${nc} %s\n" "$1" ; }

printf "\n${bold}═══════════════════════════════════════════════════════════════${nc}\n"
printf "${bold}  EXP 2 — Backfill réel --write${nc}\n"
printf "${bold}  $(date -u +%Y-%m-%dT%H:%M:%SZ)${nc}\n"
printf "${bold}═══════════════════════════════════════════════════════════════${nc}\n"

# ─────────────────────────────────────────────────────────────────────
section "Step 2.0 — Setup workdir frais"
# ─────────────────────────────────────────────────────────────────────
rm -rf /tmp/efe-exp2 2>/dev/null
mkdir -p /tmp/efe-exp2/ndjson
cp bench/fixtures/pre_g11_ndjson/*.ndjson /tmp/efe-exp2/ndjson/
echo "  workdir : /tmp/efe-exp2/ndjson/"
echo "  fichiers : $(ls /tmp/efe-exp2/ndjson/ | wc -l | tr -d ' ')"

# ─────────────────────────────────────────────────────────────────────
section "Step 2.1 — Inventaire AVANT"
# ─────────────────────────────────────────────────────────────────────
for f in /tmp/efe-exp2/ndjson/*.ndjson; do
  printf "  %-50s : %d row(s)\n" "$(basename $f)" "$(wc -l < $f)"
done
total_before=0
for f in /tmp/efe-exp2/ndjson/*.ndjson; do
  total_before=$((total_before + $(wc -l < "$f")))
done
echo "  → Total : $total_before rows à reconstruire"

# ─────────────────────────────────────────────────────────────────────
section "Step 2.2 — Inspect case_a AVANT (opaque)"
# ─────────────────────────────────────────────────────────────────────
head -1 /tmp/efe-exp2/ndjson/case_a_warn_null_tolerance.ndjson | jq '{schema_version, scenario_id, verdict, tolerance_used}'

# ─────────────────────────────────────────────────────────────────────
section "Step 2.3 — Run backfill --write"
# ─────────────────────────────────────────────────────────────────────
bash scripts/backfill-tolerance-used.sh --write /tmp/efe-exp2/ndjson/

# ─────────────────────────────────────────────────────────────────────
section "Step 2.4 — Inventaire APRÈS (10 fichiers : 5 orig + 5 backfilled)"
# ─────────────────────────────────────────────────────────────────────
ls -la /tmp/efe-exp2/ndjson/ | grep ndjson | awk '{printf "  %s   (%s bytes)\n", $NF, $5}'

# ─────────────────────────────────────────────────────────────────────
section "Step 2.5 — Inspect case_a APRÈS (traçable, avec provenance)"
# ─────────────────────────────────────────────────────────────────────
head -1 /tmp/efe-exp2/ndjson/case_a_warn_null_tolerance-backfilled.ndjson | jq '{schema_version, scenario_id, verdict, tolerance_used}'

# ─────────────────────────────────────────────────────────────────────
section "Step 2.6 — Aggregate 7 rows reconstruites"
# ─────────────────────────────────────────────────────────────────────
for f in /tmp/efe-exp2/ndjson/*-backfilled.ndjson; do
  printf "${yellow}▸ $(basename $f) :${nc}\n"
  while IFS= read -r line; do
    echo "$line" | jq -c '{scenario_id, schema_version, verdict, source: .tolerance_used.source, value: .tolerance_used.value}'
  done < "$f"
done

# ─────────────────────────────────────────────────────────────────────
section "Step 2.7 — Distribution des source reconstruites"
# ─────────────────────────────────────────────────────────────────────
for f in /tmp/efe-exp2/ndjson/*-backfilled.ndjson; do
  while IFS= read -r line; do
    echo "$line" | jq -r '.tolerance_used.source'
  done < "$f"
done | sort | uniq -c

# ─────────────────────────────────────────────────────────────────────
section "Step 2.8 — IDEMPOTENCE (run 2x = mêmes hashes)"
# ─────────────────────────────────────────────────────────────────────
$SHA /tmp/efe-exp2/ndjson/*-backfilled.ndjson > /tmp/efe-exp2/hashes-1.txt 2>&1

echo "  ▸ Hashes APRÈS 1er run :"
cat /tmp/efe-exp2/hashes-1.txt | awk '{print "    " $1, "  " $2}' | head -5

echo ""
echo "  ▸ Run --write 2e fois ..."
bash scripts/backfill-tolerance-used.sh --write /tmp/efe-exp2/ndjson/ 2>&1 | tail -5

$SHA /tmp/efe-exp2/ndjson/*-backfilled.ndjson > /tmp/efe-exp2/hashes-2.txt 2>&1

echo ""
echo "  ▸ Hashes APRÈS 2e run :"
cat /tmp/efe-exp2/hashes-2.txt | awk '{print "    " $1, "  " $2}' | head -5

echo ""
if diff -q /tmp/efe-exp2/hashes-1.txt /tmp/efe-exp2/hashes-2.txt >/dev/null; then
  ok "Idempotent : hashes identiques avant/après 2e run"
else
  fail "RÉGRESSION : hashes différents → backfill non-idempotent"
fi

# ─────────────────────────────────────────────────────────────────────
section "Step 2.9 — BYTE-IDENTICAL avec golden file"
# ─────────────────────────────────────────────────────────────────────
bash scripts/backfill-tolerance-used.sh --dry-run bench/fixtures/pre_g11_ndjson/ > /tmp/efe-exp2/dry-run-actual.txt 2>&1
diff_lines=$(diff bench/fixtures/backfill_golden_diff.txt /tmp/efe-exp2/dry-run-actual.txt | wc -l | tr -d ' ')
if [ "$diff_lines" -eq 0 ]; then
  ok "byte-identical avec golden file (0 lignes de diff)"
else
  fail "diff = $diff_lines lignes — RÉGRESSION"
  diff bench/fixtures/backfill_golden_diff.txt /tmp/efe-exp2/dry-run-actual.txt | head -10
fi

# ─────────────────────────────────────────────────────────────────────
section "Step 2.10 — Cleanup"
# ─────────────────────────────────────────────────────────────────────
rm -rf /tmp/efe-exp2
echo "  cleanup done"

printf "\n${bold}═══════════════════════════════════════════════════════════════${nc}\n"
printf "${bold}${green}  EXP 2 complete.${nc}\n"
printf "${bold}  See docs/marketing/EFE-demo-EXP2-prepared-2026-05-17.md for interpretation.${nc}\n"
printf "${bold}═══════════════════════════════════════════════════════════════${nc}\n"
