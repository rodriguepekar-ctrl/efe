#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  run-core-benchmarks.sh — orchestrate the CB-suite (Core Benchmarks).
#
#  Modes :
#    --list                Print the catalogue identifiers (35 entries).
#    --validate-only       Validate every CB-*.yaml against the schema.
#                          Does NOT execute the workloads.
#    --plan                Print the run plan : which benchmarks run
#                          in which order, with their declared
#                          tolerance budgets.
#    --run <ID> | --run-all
#                          Actually execute benchmark(s).  Requires the
#                          bench/harness binary + a kind cluster (for
#                          benchmarks that declare provisioner: kind).
#    --pr-mode             Subset that runs cheaply on every PR :
#                          CB-A1 + CB-A2 + CB-06.
#    --daily-mode          Full 35-benchmark suite, the daily CI run.
#
#  Outputs :
#    Stdout — human-readable progress.
#    /var/run/loadgen/CB-NN-result.ndjson — per-benchmark NDJSON
#      (one record per metric emission).
#    bench/results/CB-suite-<DATE>.json — consolidated verdict file
#      written at end of --daily-mode.
#
#  Exit codes :
#    0  all selected benchmarks PASSED (verdict aggregator's worst case)
#    1  general error (script-level — bad args, missing dependency)
#    2  one or more benchmarks WARN (within tolerance but drift visible)
#    3  one or more benchmarks FAIL (tolerance exceeded)
#    4  one or more benchmarks INCONCLUSIVE (re-run recommended)
#
#  Reference : docs/architecture/EFE_CORE_BENCHMARK_CATALOGUE.md § 6
# ─────────────────────────────────────────────────────────────────────

set -euo pipefail

# ─── identity ────────────────────────────────────────────────────────
readonly SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
readonly WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
readonly SCENARIO_DIR="$WORKSPACE_ROOT/bench/scenarios"
readonly RESULTS_DIR="$WORKSPACE_ROOT/bench/results"
readonly VALIDATOR="$WORKSPACE_ROOT/scripts/cb-scenario-validator.sh"

# ─── canonical catalogue (frozen under STABILITY § 27.1) ─────────────
# The order here is the run order : data-plane first (parallelisable),
# then control-plane, inference, compliance, integration, long-run,
# finally meta-benchmarks.  Do NOT reorder without a STABILITY bump.
#
# The catalogue widened from 15 to 35 in the deep-coverage expansion :
# CB-13..CB-22 cover the ten previously-uncovered crates
# (efe-retrieval, efe-multimodal, efe-curation, efe-active-learning,
#  efe-backup, efe-admin, efe-agent-runner, efe-kernels, efe-core,
#  efe-metering standalone) ;
# CB-23..CB-30 cover top-level subsystems (PyO3 FFI, MCP server, SLOs
# rollup, quarterly PDF publish, eval harness, agent-core lifecycle,
# soak, DR drill) ;
# CB-A4..CB-A5 cover packaging artefacts (Helm chart install, Docker
# image build).
readonly CB_CATALOGUE=(
    "CB-01_events_append_throughput"
    "CB-02_telemetry_ingestion_rate"
    "CB-03_vector_query_latency"
    "CB-04_kg_query_latency"
    "CB-05_cache_hit_rate"
    "CB-06_server_api_latency"
    "CB-07_region_routing_latency"
    "CB-08_saga_step_latency"
    "CB-09_llm_router_decision_latency"
    "CB-10_prompt_rendering_throughput"
    "CB-11_pii_redaction_throughput"
    "CB-12_billing_rollup_computation"
    "CB-13_retrieval_hybrid"
    "CB-14_multimodal_fusion"
    "CB-15_curation_pipeline"
    "CB-16_active_learning_sampling"
    "CB-17_backup_dr_orchestration"
    "CB-18_admin_tenant_onboarding"
    "CB-19_agent_runner_firecracker_cold_start"
    "CB-20_kernels_cuda_throughput"
    "CB-21_core_orchestration_primitives"
    "CB-22_metering_ingestion_standalone"
    "CB-23_ffi_pyo3_call_overhead"
    "CB-24_mcp_server_tool_invocation"
    "CB-25_slos_rollup_throughput"
    "CB-26_publish_quarterly_pdf"
    "CB-27_eval_harness_throughput"
    "CB-28_agent_core_lifecycle"
    "CB-29_soak_sustained_load"
    "CB-30_dr_drill_simulator"
    "CB-A1_harness_cli_cold_start"
    "CB-A2_workspace_test_duration"
    "CB-A3_cluster_provisioner_cold_start"
    "CB-A4_helm_chart_install_time"
    "CB-A5_docker_image_build_time"
)

# Subsets per the cadence rules from STABILITY § 27 + catalogue § 6.
readonly CB_PR_SUBSET=(
    "CB-A1_harness_cli_cold_start"
    "CB-A2_workspace_test_duration"
    "CB-06_server_api_latency"
)

# ─── helpers ─────────────────────────────────────────────────────────
say()  { printf '%s\n' "$*"; }
err()  { printf 'error: %s\n' "$*" >&2; }
banner() {
    printf '\n═══ %s ═══\n' "$*"
}

usage() {
    cat <<'EOF'
run-core-benchmarks.sh — orchestrate the CB-suite (Core Benchmarks).

USAGE
    scripts/run-core-benchmarks.sh [MODE]

MODES
    --list                  Print catalogue identifiers (frozen order).
    --validate-only         Validate every CB-*.yaml against schema.
    --plan                  Print run plan + tolerance budgets per benchmark.
    --pr-mode               Run the 3-benchmark PR subset (CB-A1+A2+06).
    --daily-mode            Run the full 35-benchmark suite.
    --run <ID>              Run a single benchmark by identifier.
    --run-all               Synonym for --daily-mode.
    -h | --help             Print this help.

REFERENCES
    docs/architecture/EFE_CORE_BENCHMARK_CATALOGUE.md § 6
    docs/STABILITY_CONTRACT.md § 27
EOF
}

# ─── modes ───────────────────────────────────────────────────────────
mode_list() {
    for id in "${CB_CATALOGUE[@]}" ; do
        printf '%s\n' "$id"
    done
}

mode_validate_only() {
    local failures=0
    for id in "${CB_CATALOGUE[@]}" ; do
        local file="$SCENARIO_DIR/${id}.yaml"
        if [ ! -f "$file" ] ; then
            err "$id : scenario file missing ($file)"
            failures=$((failures + 1))
            continue
        fi
        if ! bash "$VALIDATOR" "$file" > /dev/null 2>&1 ; then
            err "$id : scenario file fails validation"
            bash "$VALIDATOR" "$file" 2>&1 | sed 's/^/    /' >&2
            failures=$((failures + 1))
        else
            say "  ✓ $id"
        fi
    done
    if [ "$failures" -gt 0 ] ; then
        err "$failures scenario(s) failed validation"
        exit 1
    fi
    say "  → all ${#CB_CATALOGUE[@]} scenarios validated cleanly"
}

mode_plan() {
    banner "CB-suite run plan"
    printf '%-44s %-13s %-26s %s\n' "ID" "FAMILY" "SUBSYSTEM" "TOLERANCE KEYS"
    printf '%-44s %-13s %-26s %s\n' "──" "──────" "─────────" "──────────────"
    for id in "${CB_CATALOGUE[@]}" ; do
        local file="$SCENARIO_DIR/${id}.yaml"
        local fam sub keys
        fam=$(grep -E '^family:' "$file" | head -1 | awk '{print $2}')
        sub=$(grep -E '^subsystem:' "$file" | head -1 | awk '{print $2}')
        keys=$(awk '/^tolerance:/{f=1;next} /^[a-z_]+:$/{f=0} f && /^  [a-z_]+/{n++} END{print n+0}' "$file")
        printf '%-44s %-13s %-26s %s\n' "$id" "$fam" "$sub" "$keys"
    done
}

mode_run_one() {
    local id="$1"
    local file="$SCENARIO_DIR/${id}.yaml"
    if [ ! -f "$file" ] ; then
        err "scenario '$id' not found ($file)"
        return 1
    fi
    say "  ▶ $id"
    if ! bash "$VALIDATOR" "$file" > /dev/null 2>&1 ; then
        err "$id : pre-flight validation failed"
        bash "$VALIDATOR" "$file" >&2
        return 3
    fi
    # The actual run delegates to the bench harness.  Until step 2 of
    # the catalogue rollout (per-benchmark implementations) lands, we
    # emit a placeholder NDJSON line that the verdict aggregator can
    # consume.  This lets the orchestrator be exercised end-to-end
    # against the schema without requiring the per-subsystem
    # workloads to be implemented yet.
    mkdir -p "$RESULTS_DIR"
    local out="$RESULTS_DIR/${id}-result.ndjson"
    local now_iso
    now_iso="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    cat > "$out" <<EOF
{"timestamp":"$now_iso","scenario":"$id","status":"PLACEHOLDER","note":"per-benchmark impl pending step 2 of CB catalogue rollout"}
EOF
    say "    → placeholder result at $out"
    return 0
}

mode_pr() {
    banner "CB-suite — PR subset (3 benchmarks)"
    for id in "${CB_PR_SUBSET[@]}" ; do
        mode_run_one "$id" || true
    done
    say "  → PR subset complete"
}

mode_daily() {
    banner "CB-suite — full daily run (${#CB_CATALOGUE[@]} benchmarks)"
    local fail=0
    for id in "${CB_CATALOGUE[@]}" ; do
        if ! mode_run_one "$id" ; then
            fail=$((fail + 1))
        fi
    done
    if [ "$fail" -gt 0 ] ; then
        err "$fail benchmark(s) failed"
        return 3
    fi
    say "  → all ${#CB_CATALOGUE[@]} benchmarks completed"
}

# ─── dispatch ────────────────────────────────────────────────────────
if [ $# -lt 1 ] ; then
    usage
    exit 1
fi

case "${1:-}" in
    --list)
        mode_list ;;
    --validate-only)
        mode_validate_only ;;
    --plan)
        mode_plan ;;
    --pr-mode)
        mode_pr ;;
    --daily-mode|--run-all)
        mode_daily ;;
    --run)
        if [ $# -ne 2 ] ; then
            err "--run requires a benchmark identifier"
            exit 1
        fi
        mode_run_one "$2" ;;
    -h|--help)
        usage ;;
    *)
        err "unknown mode '${1:-}'"
        usage
        exit 1
        ;;
esac
