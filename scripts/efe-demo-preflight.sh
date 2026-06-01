#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  scripts/efe-demo-preflight.sh
#
#  Pre-flight check for the EFE demo (2026-05-17 production).
#
#  Runs ~ 14 verifications across :
#    - shell environment (cwd, env vars)
#    - hardware (disk, battery)
#    - software stack (Rust, Python, tools)
#    - cargo cache (binaries, test artifacts)
#    - critical files (lessons, fixtures, scripts)
#    - test sanity (the 7 PR #27 scope suites)
#    - script sanity (backfill dry-run, validator)
#    - macOS-specific (caffeinate, focus mode)
#
#  Returns 0 if all OK (PRE-FLIGHT PASS).
#  Returns ≥ 1 if any check FAILS (PRE-FLIGHT FAIL with N issues).
#
#  Usage :
#    cd /Users/rodrigue.pekar/Downloads/jjjj/efe
#    bash scripts/efe-demo-preflight.sh
#
#  Companion : docs/marketing/EFE-demo-environnement-2026-05-17.md § 17
# ─────────────────────────────────────────────────────────────────────

set -u

FAIL=0
WARN=0
PASS=0

green='\033[32m'
yellow='\033[33m'
red='\033[31m'
bold='\033[1m'
nc='\033[0m'

ok()   { printf "  ${green}✓${nc} %s\n" "$1" ; PASS=$((PASS+1)) ; }
warn() { printf "  ${yellow}!${nc} %s\n" "$1" ; WARN=$((WARN+1)) ; }
fail() { printf "  ${red}✗${nc} %s\n" "$1" ; FAIL=$((FAIL+1)) ; }
section() { printf "\n${bold}── %s${nc}\n" "$1" ; }

printf "\n${bold}═══════════════════════════════════════════════════════════════════${nc}\n"
printf "${bold}  EFE Demo Pre-flight — $(date -u +%Y-%m-%dT%H:%M:%SZ)${nc}\n"
printf "${bold}═══════════════════════════════════════════════════════════════════${nc}\n"

# ─────────────────────────────────────────────────────────────────────
#  1. Working directory
# ─────────────────────────────────────────────────────────────────────
section "1. Working directory"

EXPECTED_CWD="/Users/rodrigue.pekar/Downloads/jjjj/efe"
if [ "$(pwd)" = "$EXPECTED_CWD" ]; then
  ok "cwd : $(pwd)"
else
  fail "cwd : $(pwd) — expected $EXPECTED_CWD"
fi

# ─────────────────────────────────────────────────────────────────────
#  2. Environment variables
# ─────────────────────────────────────────────────────────────────────
section "2. Environment variables"

if [ "${PYO3_PYTHON:-}" = "python3.12" ]; then
  ok "PYO3_PYTHON=python3.12"
else
  fail "PYO3_PYTHON not set or wrong (current: '${PYO3_PYTHON:-unset}')"
fi

if [ "${CARGO_TERM_COLOR:-}" = "always" ]; then
  ok "CARGO_TERM_COLOR=always"
else
  warn "CARGO_TERM_COLOR not 'always' (current: '${CARGO_TERM_COLOR:-unset}')"
fi

# ─────────────────────────────────────────────────────────────────────
#  3. Disk space
# ─────────────────────────────────────────────────────────────────────
section "3. Disk space"

AVAIL_KB=$(df -k ~ | awk 'NR==2{print $4}')
AVAIL_GB=$((AVAIL_KB / 1024 / 1024))
if [ "$AVAIL_GB" -ge 10 ]; then
  ok "disk free : ${AVAIL_GB} Gi"
elif [ "$AVAIL_GB" -ge 5 ]; then
  warn "disk free : ${AVAIL_GB} Gi (≥ 5 Gi but tight)"
else
  fail "disk free : ${AVAIL_GB} Gi (< 5 Gi — clean target/release !)"
fi

if [ -d target ]; then
  TARGET_KB=$(du -sk target 2>/dev/null | awk '{print $1}')
  TARGET_GB=$((TARGET_KB / 1024 / 1024))
  ok "target/ size : ${TARGET_GB} Gi"
fi

# ─────────────────────────────────────────────────────────────────────
#  4. Software stack
# ─────────────────────────────────────────────────────────────────────
section "4. Software stack"

# Rust
if command -v rustc >/dev/null && rustc --version | grep -qE "1\.(8[0-9]|9[0-9])"; then
  ok "rustc : $(rustc --version | head -1)"
else
  fail "rustc : missing or version < 1.80"
fi

# Python 3.12
if /usr/local/bin/python3.12 --version >/dev/null 2>&1; then
  ok "python3.12 : $(/usr/local/bin/python3.12 --version)"
else
  fail "python3.12 NOT found at /usr/local/bin/python3.12"
fi

# Critical tools
for cmd in git jq bash perl caffeinate xattr; do
  if command -v $cmd >/dev/null; then
    ok "$cmd : $(command -v $cmd)"
  else
    fail "$cmd : missing"
  fi
done

# ─────────────────────────────────────────────────────────────────────
#  5. Cargo cache & binaries
# ─────────────────────────────────────────────────────────────────────
section "5. Cargo cache & binaries"

for bin in efe-bench-harness efe-bench; do
  if [ -x "target/debug/$bin" ]; then
    ok "binary present : target/debug/$bin"
  else
    fail "binary MISSING : target/debug/$bin (run : cargo build -p efe-bench-harness --bin $bin)"
  fi
done

# Pre-compiled test artifacts (heuristic : check for any g11_round_trip test binary)
if ls target/debug/deps/g11_round_trip-* >/dev/null 2>&1; then
  ok "test artifacts pre-compiled (g11_round_trip-*)"
else
  warn "test artifacts NOT pre-compiled — first run will be slow"
fi

# ─────────────────────────────────────────────────────────────────────
#  6. Critical files
# ─────────────────────────────────────────────────────────────────────
section "6. Critical files"

CRITICAL_FILES=(
  "docs/lessons/PR-27-lessons.md"
  "docs/gaps/master_tracker.md"
  "docs/lessons/monitoring/PR-27-monitoring-week-9.md"
  "docs/lessons/monitoring/PR-27-monitoring-week-10.md"
  "docs/lessons/monitoring/PR-27-monitoring-week-11.md"
  "docs/lessons/monitoring/PR-27-monitoring-week-12.md"
  "docs/lessons/monitoring/PR-27-quarterly-dry-run-2026-07-31.md"
  "docs/lessons/monitoring/PR-27-closure-2026-08-07.md"
  "docs/lessons/checklists/PR-27-S3-P1.md"
  "docs/lessons/checklists/PR-27-S3-P2.md"
  "docs/lessons/checklists/PR-27-S3-P3.md"
  "docs/lessons/checklists/PR-27-S3-P4.md"
  "docs/lessons/checklists/PR-27-S3-P5.md"
  "docs/lessons/checklists/PR-27-S3-P6.md"
  "docs/lessons/checklists/S3-standup-notes.md"
  "docs/lessons/PR-27-chirurgical-audit-2026-06-05.md"
  "bench/fixtures/backfill_golden_diff.txt"
  "bench/fixtures/audit_reader_golden_explain.txt"
  "bench/fixtures/scenarios/G11_synthetic.yaml"
  "scripts/audit-prompt-validator.sh"
  "scripts/backfill-tolerance-used.sh"
)

MISSING_COUNT=0
for f in "${CRITICAL_FILES[@]}"; do
  if [ -f "$f" ]; then
    : # silent on pass to reduce noise
  else
    fail "MISSING : $f"
    MISSING_COUNT=$((MISSING_COUNT+1))
  fi
done
[ "$MISSING_COUNT" -eq 0 ] && ok "all ${#CRITICAL_FILES[@]} critical files present"

# Pre-G.11 NDJSON fixtures
NDJSON_COUNT=$(ls bench/fixtures/pre_g11_ndjson/*.ndjson 2>/dev/null | wc -l | tr -d ' ')
if [ "$NDJSON_COUNT" -ge 5 ]; then
  ok "pre-G.11 NDJSON fixtures : $NDJSON_COUNT files"
else
  fail "pre-G.11 NDJSON fixtures : $NDJSON_COUNT (expected ≥ 5)"
fi

# ─────────────────────────────────────────────────────────────────────
#  7. Test sanity — the 7 PR #27 scope suites
# ─────────────────────────────────────────────────────────────────────
section "7. Test sanity — 7 PR #27 scope suites"

TESTS=(
  "g11_round_trip:9"
  "g11_tolerance_used_populated:7"
  "stability_contract_v17:51"
  "backfill_edge_cases:14"
  "pr27_workflow_yaml_invariants:18"
  "s3_audit_chain_integrity:14"
  "s9_s12_monitoring_chain_integrity:16"
)

for entry in "${TESTS[@]}"; do
  test_name="${entry%:*}"
  expected_count="${entry#*:}"
  out=$(cargo test -p efe-bench-harness --test "$test_name" --quiet 2>&1 | grep "^test result:" | tail -1)
  actual_count=$(echo "$out" | grep -oE '[0-9]+ passed' | head -1 | awk '{print $1}')
  if [ "$out" != "${out#*ok}" ] && [ "$actual_count" = "$expected_count" ]; then
    ok "test $test_name : $actual_count passed"
  else
    fail "test $test_name : got '$out' (expected $expected_count passed)"
  fi
done

# ─────────────────────────────────────────────────────────────────────
#  8. Script sanity
# ─────────────────────────────────────────────────────────────────────
section "8. Script sanity"

if bash scripts/backfill-tolerance-used.sh --dry-run bench/fixtures/pre_g11_ndjson/ 2>&1 | grep -q "End of golden diff"; then
  ok "backfill dry-run reaches end-of-golden"
else
  fail "backfill dry-run does NOT complete"
fi

if bash scripts/audit-prompt-validator.sh docs/lessons/PR-27-lessons.md 2>&1 | grep -q "passes schema validation"; then
  ok "audit-prompt validator : lessons file PASS"
else
  fail "audit-prompt validator : lessons file FAIL"
fi

# ─────────────────────────────────────────────────────────────────────
#  9. Macros — wow-moment file presence
# ─────────────────────────────────────────────────────────────────────
section "9. Wow-moment file presence"

if [ -f "docs/lessons/monitoring/PR-27-monitoring-week-11.md" ]; then
  ok "wow-moment target file present (will be temporarily mv'd during demo)"
else
  fail "wow-moment target file MISSING — demo will fail"
fi

if [ -f "/tmp/.efe-wm-week11.md" ]; then
  warn "stale wow-moment temp file found in /tmp — possible previous demo crash ; consider : mv /tmp/.efe-wm-week11.md docs/lessons/monitoring/PR-27-monitoring-week-11.md"
fi

# ─────────────────────────────────────────────────────────────────────
#  10. macOS specifics (caffeinate, battery)
# ─────────────────────────────────────────────────────────────────────
section "10. macOS specifics"

if pgrep -f "caffeinate -dimsu" >/dev/null; then
  ok "caffeinate -dimsu : running"
else
  warn "caffeinate NOT running — launch with : caffeinate -dimsu &"
fi

# Battery
if command -v pmset >/dev/null; then
  if pmset -g batt 2>/dev/null | grep -q "AC Power"; then
    ok "AC power : connected"
  else
    BATT_PCT=$(pmset -g batt 2>/dev/null | grep -o '[0-9]*%' | head -1 | tr -d '%')
    if [ -n "$BATT_PCT" ]; then
      if [ "$BATT_PCT" -ge 60 ]; then
        warn "battery : ${BATT_PCT}% (on battery — connect AC before demo)"
      else
        fail "battery : ${BATT_PCT}% (< 60% and not on AC)"
      fi
    fi
  fi
fi

# Focus mode (cosmetic — can only warn)
warn "Focus 'Demo' mode : verify manually in Control Center"

# ─────────────────────────────────────────────────────────────────────
#  11. Git working tree
# ─────────────────────────────────────────────────────────────────────
section "11. Git working tree"

if command -v git >/dev/null && [ -d .git ]; then
  DIRTY=$(git status --porcelain | wc -l | tr -d ' ')
  if [ "$DIRTY" -eq 0 ]; then
    ok "working tree clean"
  else
    warn "working tree has $DIRTY uncommitted change(s) — run 'git status' to inspect"
  fi
  BRANCH=$(git branch --show-current)
  COMMIT=$(git rev-parse HEAD | head -c 12)
  ok "branch: $BRANCH @ $COMMIT"
fi

# ─────────────────────────────────────────────────────────────────────
#  Final tally
# ─────────────────────────────────────────────────────────────────────
echo ""
printf "${bold}═══════════════════════════════════════════════════════════════════${nc}\n"
if [ "$FAIL" -eq 0 ] && [ "$WARN" -eq 0 ]; then
  printf "${bold}${green}  PRE-FLIGHT PASS — ${PASS} checks OK, 0 warning, 0 fail${nc}\n"
  printf "${bold}  Ready to demo.${nc}\n"
elif [ "$FAIL" -eq 0 ]; then
  printf "${bold}${yellow}  PRE-FLIGHT PASS WITH WARNINGS — ${PASS} OK, ${WARN} warn, 0 fail${nc}\n"
  printf "  Demo OK but verify the warnings above.\n"
else
  printf "${bold}${red}  PRE-FLIGHT FAIL — ${PASS} OK, ${WARN} warn, ${FAIL} FAIL${nc}\n"
  printf "  AVORTER LA DÉMO et corriger les ${FAIL} échecs ci-dessus.\n"
fi
printf "${bold}═══════════════════════════════════════════════════════════════════${nc}\n"

exit "$FAIL"
