#!/bin/bash
# ─────────────────────────────────────────────────────────────────────
#  scripts/efe-exp3-run.sh
#
#  EXP 3 — efe-bench-harness compare : EFE détecte les régressions (5-7 min)
#
#  Companion : docs/marketing/EFE-demo-EXP3-prepared-2026-05-17.md
#  All outputs verified live on 2026-05-17.
#
#  Steps :
#    3.0   Setup workdir
#    3.1   Créer baseline.json (run de référence)
#    3.2   Créer 3 results : regression, healthy, warn
#    3.3   Compare REGRESSION en markdown (table PR-grade)
#    3.4   Compare REGRESSION en text (CI logs + exit 1)
#    3.5   Compare REGRESSION en json (dashboard machine-readable)
#    3.6   Compare HEALTHY (delta < 5 % → verdict pass)
#    3.7   Compare WARN-level (delta 5-15 % → verdict warn, exit 0)
#    3.8   Self-compare (baseline vs baseline → 0.0 % partout)
#    3.9   Récap exit codes pour les 4 cas
#    3.10  Inspect tolerance_used par métrique (les seuils)
#    3.11  Cleanup
#
#  Usage :
#    cd /Users/rodrigue.pekar/Downloads/jjjj/efe
#    bash scripts/efe-exp3-run.sh
# ─────────────────────────────────────────────────────────────────────

set -u

if [ "$(pwd)" != "/Users/rodrigue.pekar/Downloads/jjjj/efe" ]; then
  echo "ERROR: must run from /Users/rodrigue.pekar/Downloads/jjjj/efe"
  echo "  current: $(pwd)"
  exit 1
fi

if ! [ -x "./target/debug/efe-bench-harness" ]; then
  echo "ERROR: ./target/debug/efe-bench-harness not found"
  echo "  build with : cargo build -p efe-bench-harness --bin efe-bench-harness"
  exit 1
fi

if ! command -v jq >/dev/null; then
  echo "ERROR: jq required (brew install jq)"
  exit 1
fi

bold='\033[1m'
green='\033[32m'
yellow='\033[33m'
red='\033[31m'
cyan='\033[36m'
nc='\033[0m'

section() { printf "\n${bold}── %s ──${nc}\n" "$1" ; }
ok() { printf "  ${green}✓${nc} %s\n" "$1" ; }

printf "\n${bold}═══════════════════════════════════════════════════════════════${nc}\n"
printf "${bold}  EXP 3 — efe-bench-harness compare détecte les régressions${nc}\n"
printf "${bold}  $(date -u +%Y-%m-%dT%H:%M:%SZ)${nc}\n"
printf "${bold}═══════════════════════════════════════════════════════════════${nc}\n"

# ─────────────────────────────────────────────────────────────────────
section "Step 3.0 — Setup workdir"
# ─────────────────────────────────────────────────────────────────────
rm -rf /tmp/efe-exp3 2>/dev/null
mkdir -p /tmp/efe-exp3
echo "  workdir : /tmp/efe-exp3/"

# ─────────────────────────────────────────────────────────────────────
section "Step 3.1 — Créer baseline.json (run stable de référence)"
# ─────────────────────────────────────────────────────────────────────
cat > /tmp/efe-exp3/baseline.json <<'EOF'
{
  "schema_version": 1, "scenario_id": "S01_smoke_test",
  "harness_version": "0.2.0", "loadgen_version": "0.1.0",
  "chart_version": "0.6.3", "image_tag": "v0.2.0",
  "git_commit": "abc1234", "run_id": "baseline-run-001",
  "started_at": "2026-05-17T10:00:00Z",
  "duration_s": 300, "total_requests": 50000,
  "metrics": {
    "p50_ms": 12.5, "p95_ms": 45.0, "p99_ms": 95.0,
    "error_rate": 0.001, "throughput_qps": 167.0,
    "recovery_time_s": null
  },
  "chaos_events": [], "verdict": "pass"
}
EOF
ok "baseline.json créé (p50=12.5, p95=45, p99=95 ms, error 0.1%, 167 qps)"

# ─────────────────────────────────────────────────────────────────────
section "Step 3.2 — Créer 3 results candidates"
# ─────────────────────────────────────────────────────────────────────
cat > /tmp/efe-exp3/result-regression.json <<'EOF'
{
  "schema_version": 1, "scenario_id": "S01_smoke_test",
  "harness_version": "0.2.0", "loadgen_version": "0.1.0",
  "chart_version": "0.6.3", "image_tag": "v0.2.1",
  "git_commit": "def5678", "run_id": "test-run-001",
  "started_at": "2026-05-17T11:00:00Z",
  "duration_s": 300, "total_requests": 50500,
  "metrics": {
    "p50_ms": 18.0, "p95_ms": 65.5, "p99_ms": 145.0,
    "error_rate": 0.008, "throughput_qps": 168.0,
    "recovery_time_s": null
  },
  "chaos_events": [], "verdict": null
}
EOF
cat > /tmp/efe-exp3/result-healthy.json <<'EOF'
{
  "schema_version": 1, "scenario_id": "S01_smoke_test",
  "harness_version": "0.2.0", "loadgen_version": "0.1.0",
  "chart_version": "0.6.3", "image_tag": "v0.2.0-rc.2",
  "git_commit": "ghi9012", "run_id": "test-run-002",
  "started_at": "2026-05-17T11:30:00Z",
  "duration_s": 300, "total_requests": 50100,
  "metrics": {
    "p50_ms": 12.8, "p95_ms": 46.0, "p99_ms": 97.0,
    "error_rate": 0.0012, "throughput_qps": 167.5,
    "recovery_time_s": null
  },
  "chaos_events": [], "verdict": null
}
EOF
cat > /tmp/efe-exp3/result-warn.json <<'EOF'
{
  "schema_version": 1, "scenario_id": "S01_smoke_test",
  "harness_version": "0.2.0", "loadgen_version": "0.1.0",
  "chart_version": "0.6.3", "image_tag": "v0.2.0-rc.3",
  "git_commit": "jkl3456", "run_id": "test-run-003",
  "started_at": "2026-05-17T12:00:00Z",
  "duration_s": 300, "total_requests": 50050,
  "metrics": {
    "p50_ms": 14.2, "p95_ms": 52.0, "p99_ms": 112.0,
    "error_rate": 0.003, "throughput_qps": 167.2,
    "recovery_time_s": null
  },
  "chaos_events": [], "verdict": null
}
EOF
ok "result-regression.json (p50 18, p95 65.5, p99 145, error 0.8 % → fail)"
ok "result-healthy.json   (p50 12.8, p95 46, p99 97, error 0.12 % → pass)"
ok "result-warn.json      (p50 14.2, p95 52, p99 112, error 0.3 % → warn)"

# ─────────────────────────────────────────────────────────────────────
section "Step 3.3 — Compare REGRESSION (markdown — PR comment format)"
# ─────────────────────────────────────────────────────────────────────
./target/debug/efe-bench-harness compare \
  --result /tmp/efe-exp3/result-regression.json \
  --baseline /tmp/efe-exp3/baseline.json \
  --format markdown 2>&1 | grep -v "ERROR\|aborted"

# ─────────────────────────────────────────────────────────────────────
section "Step 3.4 — Compare REGRESSION (text — CI logs + exit code)"
# ─────────────────────────────────────────────────────────────────────
./target/debug/efe-bench-harness compare \
  --result /tmp/efe-exp3/result-regression.json \
  --baseline /tmp/efe-exp3/baseline.json \
  --format text 2>&1 | grep -v "aborted"
rc=$(./target/debug/efe-bench-harness compare \
  --result /tmp/efe-exp3/result-regression.json \
  --baseline /tmp/efe-exp3/baseline.json \
  --format text > /dev/null 2>&1 ; echo $?)
printf "  ${red}exit code : %d${nc}  ← gate CI bloque le merge\n" "$rc"

# ─────────────────────────────────────────────────────────────────────
section "Step 3.5 — Compare REGRESSION (json — dashboard machine-readable)"
# ─────────────────────────────────────────────────────────────────────
./target/debug/efe-bench-harness compare \
  --result /tmp/efe-exp3/result-regression.json \
  --baseline /tmp/efe-exp3/baseline.json \
  --format json 2>/dev/null | head -30
echo "  (... 24 lignes additionnelles tronquées)"

# ─────────────────────────────────────────────────────────────────────
section "Step 3.6 — Compare HEALTHY (delta < 5 %, verdict pass)"
# ─────────────────────────────────────────────────────────────────────
./target/debug/efe-bench-harness compare \
  --result /tmp/efe-exp3/result-healthy.json \
  --baseline /tmp/efe-exp3/baseline.json \
  --format markdown 2>&1 | grep -v "ERROR\|aborted"
rc=$(./target/debug/efe-bench-harness compare \
  --result /tmp/efe-exp3/result-healthy.json \
  --baseline /tmp/efe-exp3/baseline.json \
  --format text > /dev/null 2>&1 ; echo $?)
printf "  ${green}exit code : %d${nc}  ← merge OK\n" "$rc"

# ─────────────────────────────────────────────────────────────────────
section "Step 3.7 — Compare WARN (delta 5-15 %, verdict warn)"
# ─────────────────────────────────────────────────────────────────────
./target/debug/efe-bench-harness compare \
  --result /tmp/efe-exp3/result-warn.json \
  --baseline /tmp/efe-exp3/baseline.json \
  --format markdown 2>&1 | grep -v "ERROR\|aborted"
rc=$(./target/debug/efe-bench-harness compare \
  --result /tmp/efe-exp3/result-warn.json \
  --baseline /tmp/efe-exp3/baseline.json \
  --format text > /dev/null 2>&1 ; echo $?)
printf "  ${yellow}exit code : %d${nc}  ← merge OK mais revue humaine\n" "$rc"

# ─────────────────────────────────────────────────────────────────────
section "Step 3.8 — Self-compare (sanity à 0.0 %)"
# ─────────────────────────────────────────────────────────────────────
./target/debug/efe-bench-harness compare \
  --result /tmp/efe-exp3/baseline.json \
  --baseline /tmp/efe-exp3/baseline.json \
  --format markdown 2>&1 | grep -v "ERROR\|aborted"

# ─────────────────────────────────────────────────────────────────────
section "Step 3.9 — Récap exit codes (le critère gate CI)"
# ─────────────────────────────────────────────────────────────────────
for entry in "Self (baseline vs baseline):baseline.json" "HEALTHY:result-healthy.json" "WARN:result-warn.json" "REGRESSION:result-regression.json"; do
  name="${entry%:*}"
  file="${entry#*:}"
  ./target/debug/efe-bench-harness compare \
    --result /tmp/efe-exp3/$file \
    --baseline /tmp/efe-exp3/baseline.json \
    --format text > /dev/null 2>&1
  rc=$?
  if [ "$rc" -eq 0 ]; then
    printf "  ${green}✓${nc} %-32s : exit %d   ← merge OK\n" "$name" "$rc"
  else
    printf "  ${red}✗${nc} %-32s : exit %d   ← merge BLOQUÉ\n" "$name" "$rc"
  fi
done

# ─────────────────────────────────────────────────────────────────────
section "Step 3.10 — Tolerance per metric (seuils hardcoded)"
# ─────────────────────────────────────────────────────────────────────
./target/debug/efe-bench-harness compare \
  --result /tmp/efe-exp3/result-regression.json \
  --baseline /tmp/efe-exp3/baseline.json \
  --format json 2>/dev/null | \
  jq -c '.rows | to_entries[] | {metric: .key, tolerance: .value.tolerance_used, delta_pct: .value.delta_pct, status: .value.status}'

# ─────────────────────────────────────────────────────────────────────
section "Step 3.11 — Cleanup"
# ─────────────────────────────────────────────────────────────────────
rm -rf /tmp/efe-exp3
echo "  cleanup done"

printf "\n${bold}═══════════════════════════════════════════════════════════════${nc}\n"
printf "${bold}${green}  EXP 3 complete.${nc}\n"
printf "${bold}  See docs/marketing/EFE-demo-EXP3-prepared-2026-05-17.md for interpretation.${nc}\n"
printf "${bold}═══════════════════════════════════════════════════════════════${nc}\n"
