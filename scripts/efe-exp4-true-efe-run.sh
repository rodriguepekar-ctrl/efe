#!/bin/bash
# ─────────────────────────────────────────────────────────────────────
#  scripts/efe-exp4-true-efe-run.sh
#
#  EXP 4-TRUE-EFE — EFE fait TOUT, je ne fais rien (3-5 min)
#
#  Companion : docs/marketing/EFE-demo-EXP4-TRUE-EFE-2026-05-17.md
#  All outputs verified live on 2026-05-17.
#
#  Steps :
#    4.0   Setup minimal (no fixture synthesis)
#    4.1   Show canonical fixture EFE will consume
#    4.2   Show Rust test source code
#    4.3   Run g11_round_trip (9 tests)
#    4.4   Run g11_tolerance_used_populated (7 tests)
#    4.5   Run lib::compare::effective_tolerance (45 tests)
#    4.6   Run stability_contract_v17 (51 freeze tests)
#    4.7   ★ EFE detects modification (corrupt golden → FAIL → restore → PASS)
#    4.8   Aggregate : 174 tests EFE-only
#    4.9   Récap "qui a fait quoi"
#    4.10  Cleanup (nothing to do)
#
#  Usage :
#    cd /Users/rodrigue.pekar/Downloads/jjjj/efe
#    bash scripts/efe-exp4-true-efe-run.sh
# ─────────────────────────────────────────────────────────────────────

set -u

if [ "$(pwd)" != "/Users/rodrigue.pekar/Downloads/jjjj/efe" ]; then
  echo "ERROR: must run from /Users/rodrigue.pekar/Downloads/jjjj/efe"
  exit 1
fi

export PYO3_PYTHON=python3.12 CARGO_TERM_COLOR=always

bold='\033[1m'
green='\033[32m'
yellow='\033[33m'
red='\033[31m'
cyan='\033[36m'
nc='\033[0m'

section() { printf "\n${bold}── %s ──${nc}\n" "$1" ; }
me()  { printf "  ${cyan}(moi)${nc} %s\n" "$1" ; }
efe() { printf "  ${green}(EFE)${nc} %s\n" "$1" ; }

printf "\n${bold}═══════════════════════════════════════════════════════════════${nc}\n"
printf "${bold}  EXP 4-TRUE-EFE — EFE fait TOUT, je ne fais rien${nc}\n"
printf "${bold}  $(date -u +%Y-%m-%dT%H:%M:%SZ)${nc}\n"
printf "${bold}═══════════════════════════════════════════════════════════════${nc}\n"

# ─────────────────────────────────────────────────────────────────────
section "Step 4.0 — Setup minimal"
# ─────────────────────────────────────────────────────────────────────
me "cd + export PYO3_PYTHON (déjà fait)"
efe "rien"

# ─────────────────────────────────────────────────────────────────────
section "Step 4.1 — Le canonical fixture qu'EFE va consommer"
# ─────────────────────────────────────────────────────────────────────
me "head -1 (juste pour te montrer)"
head -1 bench/fixtures/post_g11_ndjson/CB-01.ndjson
efe "a produit ce fichier antérieurement (visible dans git log)"

# ─────────────────────────────────────────────────────────────────────
section "Step 4.2 — Le code Rust EFE qui va analyser"
# ─────────────────────────────────────────────────────────────────────
me "grep -A 10 pour te montrer le source"
grep -A 10 "fn g11_round_trip_canonical_fixture_byte_identical" bench/harness/tests/g11_round_trip.rs | head -12

# ─────────────────────────────────────────────────────────────────────
section "Step 4.3 — Run g11_round_trip (9 tests)"
# ─────────────────────────────────────────────────────────────────────
me "cargo test (juste le command runner)"
efe "lit fixture → parse → valide → sérialise → compare → émet verdict"
cargo test -p efe-bench-harness --test g11_round_trip -- --test-threads=1 2>&1 | grep -E "^test |^test result:" | head -15

# ─────────────────────────────────────────────────────────────────────
section "Step 4.4 — Run g11_tolerance_used_populated (7 tests, G.11.a/b/c/d)"
# ─────────────────────────────────────────────────────────────────────
efe "épingle G.11.a/b/c/d + 3 enrichissements"
cargo test -p efe-bench-harness --test g11_tolerance_used_populated -- --test-threads=1 2>&1 | grep -E "^test |^test result:" | head -10

# ─────────────────────────────────────────────────────────────────────
section "Step 4.5 — Run lib::compare::effective_tolerance (45 unit tests)"
# ─────────────────────────────────────────────────────────────────────
efe "teste chaque fonction du resolver, taxonomies fermées, verdict matrix"
cargo test -p efe-bench-harness --lib compare::effective_tolerance --quiet 2>&1 | grep "^test result" | tail -1

# ─────────────────────────────────────────────────────────────────────
section "Step 4.6 — Run stability_contract_v17 (51 freeze tests)"
# ─────────────────────────────────────────────────────────────────────
efe "vérifie 51 surfaces critiques du contrat STABILITY"
cargo test -p efe-bench-harness --test stability_contract_v17 --quiet 2>&1 | grep "^test result" | tail -1

# ─────────────────────────────────────────────────────────────────────
section "Step 4.7 — ★ EFE détecte une modification (preuve définitive)"
# ─────────────────────────────────────────────────────────────────────
me "cp pour sauvegarder le golden"
cp bench/fixtures/backfill_golden_diff.txt /tmp/.efe-golden-backup

me "sed pour modifier 'PR #27' → 'PR #99' dans le golden"
sed -i.bak 's/PR #27/PR #99/' bench/fixtures/backfill_golden_diff.txt
rm -f bench/fixtures/backfill_golden_diff.txt.bak
echo "  ▸ Header modifié :"
head -2 bench/fixtures/backfill_golden_diff.txt | sed 's/^/    /'

echo ""
efe "lance le test EFE qui régénère le dry-run dynamiquement et compare byte-par-byte..."
cargo test -p efe-bench-harness --test backfill_edge_cases dry_run_byte_identical_to_golden_file 2>&1 | grep -E "test result|panicked at|does NOT match" | head -3

echo ""
me "restaure le golden (avec trap implicite via cp)"
cp /tmp/.efe-golden-backup bench/fixtures/backfill_golden_diff.txt
rm -f /tmp/.efe-golden-backup

echo ""
efe "re-lance le test → devrait passer maintenant"
cargo test -p efe-bench-harness --test backfill_edge_cases dry_run_byte_identical_to_golden_file --quiet 2>&1 | tail -1

# ─────────────────────────────────────────────────────────────────────
section "Step 4.8 — Aggregate : les 174 tests EFE-only"
# ─────────────────────────────────────────────────────────────────────
me "boucle bash pour parser les outputs (pas l'analyse !)"
total_passed=0
for t in g11_round_trip g11_tolerance_used_populated stability_contract_v17 \
         backfill_edge_cases pr27_workflow_yaml_invariants \
         s3_audit_chain_integrity s9_s12_monitoring_chain_integrity ; do
  out=$(cargo test -p efe-bench-harness --test "$t" --quiet 2>&1 | grep "^test result:" | tail -1)
  passed=$(echo "$out" | grep -oE '[0-9]+ passed' | head -1 | awk '{print $1}')
  failed=$(echo "$out" | grep -oE '[0-9]+ failed' | head -1 | awk '{print $1}')
  if [ "${failed:-0}" -eq 0 ]; then
    printf "  ${green}✓${nc} %-40s : %s passed\n" "$t" "${passed:-?}"
  else
    printf "  ${red}✗${nc} %-40s : %s passed / %s failed\n" "$t" "${passed:-?}" "${failed:-?}"
  fi
  total_passed=$((total_passed + ${passed:-0}))
done

out=$(cargo test -p efe-bench-harness --lib compare::effective_tolerance --quiet 2>&1 | grep "^test result:" | tail -1)
unit_passed=$(echo "$out" | grep -oE '[0-9]+ passed' | head -1 | awk '{print $1}')
printf "  ${green}✓${nc} %-40s : %s passed\n" "+ lib::compare::effective_tolerance" "${unit_passed:-?}"
total_passed=$((total_passed + ${unit_passed:-0}))
echo ""
printf "  ${bold}Total = %d tests EFE${nc}\n" "$total_passed"

# ─────────────────────────────────────────────────────────────────────
section "Step 4.9 — Récap honnête : qui a fait quoi"
# ─────────────────────────────────────────────────────────────────────
cat <<'EOF'
  Step | Moi (humain)              | EFE (Rust + lib + tests)
  -----+----------------------------+-----------------------------------
  4.0  | cd + export               | rien
  4.1  | head -1 (affichage)       | a produit le fixture antérieurement
  4.2  | grep -A 10 (affichage)    | est ce code
  4.3  | cargo test                | a fait 9 analyses + verdicts
  4.4  | cargo test                | a fait 7 analyses G.11.a-d
  4.5  | cargo test                | a testé 45 fonctions de sa lib
  4.6  | cargo test                | a vérifié 51 surfaces du contrat
  4.7  | cp/sed/cp (file system)   | a DÉTECTÉ ma modification 'PR #27' → 'PR #99'
  4.8  | boucle bash (parsing)     | a fait 174 analyses + 174 verdicts
  4.9  | rédige ce tableau         | rien
  4.10 | rien                      | rien
EOF

# ─────────────────────────────────────────────────────────────────────
section "Step 4.10 — Cleanup"
# ─────────────────────────────────────────────────────────────────────
if [ -f /tmp/.efe-golden-backup ]; then
  echo "  WARNING: /tmp/.efe-golden-backup encore présent"
  rm /tmp/.efe-golden-backup
  echo "  removed"
else
  echo "  Rien à cleanup (file system non-mutated, fixtures restaurées)"
fi

printf "\n${bold}═══════════════════════════════════════════════════════════════${nc}\n"
printf "${bold}${green}  EXP 4-TRUE-EFE complete.${nc}\n"
printf "${bold}  See docs/marketing/EFE-demo-EXP4-TRUE-EFE-2026-05-17.md for interpretation.${nc}\n"
printf "${bold}═══════════════════════════════════════════════════════════════${nc}\n"
