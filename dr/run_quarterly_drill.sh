#!/usr/bin/env bash
# =============================================================================
# Sprint D40 (Φ-F) — Quarterly DR drill orchestrator.
#
# Cf. `docs/EFE_BACKUP_EXTENSION_ARCHITECTURE.md` § 9.
# Cf. `docs/compliance/dr_drill_quarterly_evidence.md`.
#
# Post-audit pass 1 — defects M7-D40-A1..A24 corrigés (cf. CHANGELOG).
#
# This script EXTENDS the D17 weekly drill (`d17-dr-drill-cronjob.yaml`
# → `efe-backup drill --target-env=drill`) with the **business runbook**
# layer required by SOC 2 CC9.1 / ISO 27001 A.17.1.3 / NIS2 Art. 21 /
# RGPD Art. 32 :
#
#   Phase 1 — kind cluster bootstrap (reuses dr/setup-test-env.sh)
#   Phase 2 — Helm install verify (chart already applied by phase 1)
#   Phase 3 — restore from latest backup (calls dr/restore.sh)
#   Phase 4 — Hello-World workflow business end-to-end (NEW D40)
#   Phase 5 — verifier suite (`efe-backup drill --target-env=drill`)
#   Phase 6 — compliance evidence emit (`outcome.json` v2 + attestation)
#
# RTO target : 45 min end-to-end (D40 SLO D40-DR-1, vs 30 min weekly).
# Hello-World latency target : 60 s (D40 SLO D40-DR-2).
#
# Usage :
#   ./dr/run_quarterly_drill.sh \
#       --cluster-name efe-drill-q2-2026 \
#       --backup-id 20260601T000000Z-full \
#       --evidence-dir ./.ci-evidence/dr-drill-quarterly/2026-Q2 \
#       [--keep] [--target-env drill] [--namespace efe]
#
#   # Auto-resolve backup-id (CronJob context) :
#   EFE_BACKUP_AUTO_RESOLVE=true ./dr/run_quarterly_drill.sh \
#       --cluster-name efe-drill --evidence-dir /var/lib/efe/...
#
# Environment variables (rendered by Helm ConfigMap when run via CronJob) :
#   EFE_BACKUP_*                       — D17 baseline (cf. d17-backup-cronjobs)
#   EFE_BACKUP_RTO_TARGET_S=2700       — quarterly RTO target (45 min)
#   EFE_BACKUP_AUTO_RESOLVE=false      — opt into auto-resolution of --backup-id
#                                         from `efe-backup list --since=7d`
#   EFE_DRILL_NAMESPACE=efe            — k8s ns for restore + helloworld + verify
#   EFE_DRILL_HELM_TIMEOUT=5m          — `helm upgrade` timeout
#   EFE_DRILL_DEV_SECRET_VALUES=false  — pass `backup.devSecretValues=true` (kind)
#   EFE_HELLOWORLD_ENABLED=true        — gate Hello-World workflow on/off
#   EFE_HELLOWORLD_TENANT_ID           — synthetic tenant id (drill-only)
#   EFE_HELLOWORLD_CATALOG_NAME        — catalog name to register
#   EFE_HELLOWORLD_CLASS_NAME          — class name (e.g. greeting)
#   EFE_HELLOWORLD_SENTENCE            — canonical sentence
#   EFE_HELLOWORLD_MAX_LATENCY_MS=60000  — SLO D40-DR-2 ceiling
#   EFE_HELLOWORLD_FALLBACK_MOCK=false — opt-in to mock embedder fallback
#   EFE_GRPC_ENDPOINT=localhost:50051  — gRPC endpoint for Hello-World grpcurl
#   EFE_DRILL_TOKEN                    — JWT bearer for drill tenant calls
#   EFE_COMPLIANCE_EVIDENCE_BUCKET     — S3 bucket (Object Lock COMPLIANCE)
#   EFE_COMPLIANCE_EVIDENCE_RETENTION_YEARS=7
#   EFE_ATTESTATION_KEY_ID             — Ed25519 key id for outcome signing
#   EFE_ATTESTATION_PRIVKEY_PATH       — path to Ed25519 priv key (32-byte raw)
#
# Exit codes (mirrors dr/restore.sh ranges + D40-specific) :
#   0    — drill success (all phases + verifiers + Hello-World OK + within RTO)
#   1    — argument parse error
#   2    — pre-flight failed (kind/helm/kubectl missing or version mismatch)
#   3    — phase 1 (kind setup) failed
#   4    — phase 2 (helm verify) failed
#   5    — phase 3 (restore) failed
#   6    — phase 4 (Hello-World) failed (NEW D40)
#   7    — phase 5 (verifier suite) failed
#   8    — phase 6 (evidence emit) failed (NEW D40)
#   11   — RTO violation (SLO D40-DR-1 fail)
#   12   — Hello-World latency violation (SLO D40-DR-2 fail)
# =============================================================================

set -euo pipefail
IFS=$'\n\t'

# ─────────────────────────────────────────────────────────────────────────────
# § 0. Args + env validation
# ─────────────────────────────────────────────────────────────────────────────

CLUSTER_NAME="${CLUSTER_NAME:-efe-dr-drill-quarterly}"
BACKUP_ID=""
EVIDENCE_DIR=""
KEEP_CLUSTER=0
TARGET_ENV="drill"
NAMESPACE="${EFE_DRILL_NAMESPACE:-efe}"

usage() {
    sed -n '2,75p' "$0"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --cluster-name)   CLUSTER_NAME="$2"; shift 2 ;;
        --backup-id)      BACKUP_ID="$2"; shift 2 ;;
        --evidence-dir)   EVIDENCE_DIR="$2"; shift 2 ;;
        --target-env)     TARGET_ENV="$2"; shift 2 ;;
        --namespace)      NAMESPACE="$2"; shift 2 ;;
        --keep)           KEEP_CLUSTER=1; shift ;;
        --help|-h)        usage; exit 0 ;;
        *) echo "unknown arg: $1" >&2; usage; exit 1 ;;
    esac
done

# **Audit fix M7-D40-A17 (CRIT)** — backup-id is required by default
# but can be auto-resolved by the helm CronJob via EFE_BACKUP_AUTO_RESOLVE=true.
# Pre-audit the missing-arg path exit'd before the trap was armed, so the
# CronJob would crashloop without producing any evidence.
if [[ -z "$BACKUP_ID" && "${EFE_BACKUP_AUTO_RESOLVE:-false}" != "true" ]]; then
    echo "FATAL: --backup-id is required (or set EFE_BACKUP_AUTO_RESOLVE=true to" \
         "auto-resolve via \`efe-backup list --since=7d\`)" >&2
    usage >&2
    exit 1
fi

# **B-8 cross-env safety** — refuse anything that smells like prod.
case "$TARGET_ENV" in
    prod|production|PROD|PRODUCTION)
        echo "FATAL: target-env=$TARGET_ENV is forbidden for the drill (B-8)" >&2
        exit 1
        ;;
esac

# Constants.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# **Audit fix M7-D40-A7 (HIGH)** — portable date arithmetic.
# Pre-audit `date -u +%-m` was a GNU extension that fails on busybox /
# Alpine / some BSD distros (returns the literal `%-m`). Post-audit we
# strip the leading zero in shell, which is portable across any
# POSIX-conformant date impl.
_drill_quarter() {
    local month_str month_num quarter
    month_str=$(date -u +%m)
    month_num=$((10#$month_str))   # force base-10 (avoid octal parse on "08", "09")
    quarter=$(( (month_num - 1) / 3 + 1 ))
    printf "%s-Q%d" "$(date -u +%Y)" "$quarter"
}

if [[ -z "$EVIDENCE_DIR" ]]; then
    EVIDENCE_DIR="./.ci-evidence/dr-drill-quarterly/$(_drill_quarter)"
fi

# **Audit fix M7-D40-A16 (MED)** — convert EVIDENCE_DIR to absolute path
# so subsequent cd / log writes are unambiguous regardless of CWD changes.
mkdir -p "$EVIDENCE_DIR"
EVIDENCE_DIR="$(cd "$EVIDENCE_DIR" && pwd)"

DRILL_LABEL="drill-$(_drill_quarter)-$(date -u +%Y%m%dT%H%M%SZ)"
RTO_TARGET_S="${EFE_BACKUP_RTO_TARGET_S:-2700}"
HELM_TIMEOUT="${EFE_DRILL_HELM_TIMEOUT:-5m}"
DEV_SECRET_VALUES="${EFE_DRILL_DEV_SECRET_VALUES:-false}"
HELLOWORLD_MAX_LATENCY_MS="${EFE_HELLOWORLD_MAX_LATENCY_MS:-60000}"
HELLOWORLD_TENANT_ID="${EFE_HELLOWORLD_TENANT_ID:-drill-helloworld-tenant}"
HELLOWORLD_CATALOG_NAME="${EFE_HELLOWORLD_CATALOG_NAME:-helloworld-catalog}"
HELLOWORLD_CLASS_NAME="${EFE_HELLOWORLD_CLASS_NAME:-greeting}"
HELLOWORLD_SENTENCE="${EFE_HELLOWORLD_SENTENCE:-Hello, world!}"
HELLOWORLD_ENABLED="${EFE_HELLOWORLD_ENABLED:-true}"
HELLOWORLD_FALLBACK_MOCK="${EFE_HELLOWORLD_FALLBACK_MOCK:-false}"
GRPC_ENDPOINT="${EFE_GRPC_ENDPOINT:-localhost:50051}"
DRILL_TOKEN="${EFE_DRILL_TOKEN:-}"
ATTESTATION_KEY_ID="${EFE_ATTESTATION_KEY_ID:-drill-attestation-$(date -u +%Y)}"

KIND_LOG="$EVIDENCE_DIR/kind-cluster.log"
HELM_LOG="$EVIDENCE_DIR/helm-install.log"
RESTORE_LOG="$EVIDENCE_DIR/restore.log"
HELLOWORLD_LOG="$EVIDENCE_DIR/helloworld.log"
VERIFIER_LOG="$EVIDENCE_DIR/verifier-suite.log"
TIMING_JSON="$EVIDENCE_DIR/timing.json"
OUTCOME_JSON="$EVIDENCE_DIR/outcome.json"
ATTESTATION_TXT="$EVIDENCE_DIR/attestation.txt"

# Per-phase wall-clock counters.
T_START=$(date -u +%s)
T_KIND_END=0
T_HELM_END=0
T_RESTORE_END=0
T_HELLOWORLD_END=0
T_VERIFIER_END=0
T_END=0

# Per-phase outcome flags (used by emit_evidence at exit).
KIND_OK="false"
HELM_OK="false"
RESTORE_OK="false"
HELLOWORLD_OK="false"
HELLOWORLD_ELAPSED_S=0
HELLOWORLD_FAIL_STEP=""    # Audit fix M7-D40-A8 — capture which step failed.
VERIFIER_OK="false"
VERIFIER_DETAIL_JSON='[]'
HELLOWORLD_DETAIL_JSON='null'

log() {
    echo "[$(date -u +%FT%TZ)] $*" | tee -a "$EVIDENCE_DIR/run_quarterly_drill.log" >&2
}

# ─────────────────────────────────────────────────────────────────────────────
# § 1. Cleanup trap (B-6 — cleanup obligatoire)
# ─────────────────────────────────────────────────────────────────────────────

cleanup() {
    local rc=$?
    log "🧹 Cleanup phase (rc=$rc)"
    if [[ "$KEEP_CLUSTER" -eq 0 ]]; then
        log "  Destroying kind cluster: $CLUSTER_NAME"
        kind delete cluster --name "$CLUSTER_NAME" 2>>"$KIND_LOG" || \
            log "  WARN: kind delete cluster failed (already gone?)"
    else
        log "  --keep set: leaving cluster $CLUSTER_NAME alive"
    fi
    # **Audit fix M7-D40-A1 (CRIT)** — preserve evidence-emit's return
    # code so a successful drill that violates RTO (rc=11 from
    # emit_evidence) is surfaced as the script exit code. Pre-audit
    # the `|| log` swallowed the rc, hiding SLO D40-DR-1 violations.
    local emit_rc=0
    emit_evidence "$rc" || emit_rc=$?
    if [[ "$emit_rc" -ne 0 ]]; then
        log "  ⚠ emit_evidence returned rc=$emit_rc (overrides phase rc=$rc)"
        rc=$emit_rc
    fi
    log "🧹 Cleanup done (final rc=$rc)"
    return "$rc"
}

trap cleanup EXIT

# ─────────────────────────────────────────────────────────────────────────────
# § 2. Pre-flight (incl. Audit fix M7-D40-A17 backup-id auto-resolve)
# ─────────────────────────────────────────────────────────────────────────────

# Auto-resolve backup_id if the operator opted in via env var.
# Runs in the host cluster context (where efe-backup + backups live),
# BEFORE the kind cluster is provisioned in phase 1.
auto_resolve_backup_id() {
    if [[ -n "$BACKUP_ID" ]]; then
        log "  backup_id=$BACKUP_ID (provided via flag/env)"
        return 0
    fi
    if [[ "${EFE_BACKUP_AUTO_RESOLVE:-false}" != "true" ]]; then
        return 0   # arg-parse already enforced -- defensive return
    fi
    log "🔍 Auto-resolving latest successful backup_id..."
    BACKUP_ID=$(kubectl --namespace "$NAMESPACE" exec deploy/efe-server -- \
        efe-backup list --since=7d --json 2>/dev/null \
        | jq -r '.[0].label // empty' 2>/dev/null || echo "")
    if [[ -z "$BACKUP_ID" ]]; then
        log "FATAL: no backup found in last 7 days (run \`efe-backup list --since=7d\` to debug)"
        exit 1
    fi
    log "  ✅ resolved backup_id=$BACKUP_ID"
}

preflight() {
    log "🔍 Pre-flight checks"
    for bin in kind helm kubectl jq; do
        if ! command -v "$bin" >/dev/null 2>&1; then
            log "FATAL: $bin not in PATH"
            exit 2
        fi
    done
    log "  ✅ kind, helm, kubectl, jq present"
    auto_resolve_backup_id
    log "  drill_label=$DRILL_LABEL"
    log "  cluster_name=$CLUSTER_NAME"
    log "  evidence_dir=$EVIDENCE_DIR"
    log "  namespace=$NAMESPACE"
    log "  rto_target=${RTO_TARGET_S}s"
    log "  helloworld_enabled=$HELLOWORLD_ENABLED"
    log "  dev_secret_values=$DEV_SECRET_VALUES"
}

# ─────────────────────────────────────────────────────────────────────────────
# § 3. Phase 1 — kind setup (reuses dr/setup-test-env.sh)
#
# `setup-test-env.sh` already does kind cluster + minio + cnpg + the
# base helm install of the EFE chart with backup.enabled=true. We do
# NOT re-helm-install in phase 2 ; we only verify + apply drill-specific
# overrides via `--reuse-values`.
# ─────────────────────────────────────────────────────────────────────────────

phase_kind_setup() {
    log "▶️  Phase 1/6 — kind cluster setup"
    if "$SCRIPT_DIR/setup-test-env.sh" --cluster-name "$CLUSTER_NAME" --keep \
            >>"$KIND_LOG" 2>&1; then
        KIND_OK="true"
        log "  ✅ kind cluster + minio + cnpg + base helm install ready"
    else
        log "  ❌ kind setup failed — see $KIND_LOG"
        exit 3
    fi
    T_KIND_END=$(date -u +%s)
}

# ─────────────────────────────────────────────────────────────────────────────
# § 4. Phase 2 — Helm verify + apply drillQuarterly overrides
#
# **Audit fix M7-D40-A24 (CRIT)** — pre-audit this phase did a fresh
# `helm upgrade --install` without storage.bucket / region / bundles.*
# overrides, which RESET those required fields and crashed the chart.
# Post-audit we use `--reuse-values` so the `setup-test-env.sh` base
# install survives ; only `drillQuarterly.enabled=true` (the D40
# override) is applied here.
# ─────────────────────────────────────────────────────────────────────────────

phase_helm_install() {
    log "▶️  Phase 2/6 — Helm verify + apply drillQuarterly overrides"
    # 1. Verify the base release exists (planted by setup-test-env.sh).
    if ! helm status efe -n "$NAMESPACE" >>"$HELM_LOG" 2>&1; then
        log "  ❌ helm release efe missing in namespace $NAMESPACE"
        exit 4
    fi
    # 2. Apply drillQuarterly-specific overrides via --reuse-values.
    local extra_args=()
    if [[ "$DEV_SECRET_VALUES" == "true" ]]; then
        extra_args+=(--set "backup.devSecretValues=true")
    fi
    if helm upgrade efe "$REPO_ROOT/deploy/helm/efe" \
            --namespace "$NAMESPACE" \
            --reuse-values \
            --set "backup.drillQuarterly.enabled=true" \
            "${extra_args[@]}" \
            --wait --timeout "$HELM_TIMEOUT" \
            >>"$HELM_LOG" 2>&1; then
        HELM_OK="true"
        log "  ✅ helm upgrade complete (drillQuarterly enabled)"
    else
        log "  ❌ helm upgrade failed — see $HELM_LOG"
        exit 4
    fi
    T_HELM_END=$(date -u +%s)
}

# ─────────────────────────────────────────────────────────────────────────────
# § 5. Phase 3 — restore from latest backup (calls dr/restore.sh)
# ─────────────────────────────────────────────────────────────────────────────

phase_restore() {
    log "▶️  Phase 3/6 — restore backup_id=$BACKUP_ID into target_env=$TARGET_ENV"
    # **B-8 cross-env safety** — the drill SHOULD use --force-cross-env
    # because we deliberately restore a prod backup into the drill ns.
    # The target_env is hardcoded to "drill" by parse_args ; restore.sh
    # itself refuses prod via T-DR-12 mitigation.
    if EFE_DR_NAMESPACE="$NAMESPACE" \
            "$SCRIPT_DIR/restore.sh" "$BACKUP_ID" \
            --target "$TARGET_ENV" --force-cross-env \
            >>"$RESTORE_LOG" 2>&1; then
        RESTORE_OK="true"
        log "  ✅ restore complete"
    else
        log "  ❌ restore failed — see $RESTORE_LOG"
        exit 5
    fi
    T_RESTORE_END=$(date -u +%s)
}

# ─────────────────────────────────────────────────────────────────────────────
# § 6. Phase 4 — Hello-World workflow (NEW D40)
#
# **Audit fix M7-D40-A14 (CRIT)** — pre-audit this phase invoked
# `efe-server admin {register-catalog,evolve-catalog,canary-execute,
# verify-audit-chain}` subcommands that DO NOT EXIST in the v0.1.18
# binary (the admin namespace lives under `efe-onboard` + `efe-bundle`,
# and gRPC RPCs are not surfaced via a CLI subcommand). Post-audit
# we use `grpcurl` against the gRPC services directly (CatalogService
# RegisterCatalog / EvolveCatalog / GetCatalog + SwarmService Execute)
# which is the canonical wire contract. The HMAC chain integrity check
# falls back to D17-AS-IS verifier path.
# ─────────────────────────────────────────────────────────────────────────────

phase_helloworld() {
    log "▶️  Phase 4/6 — Hello-World workflow business end-to-end"

    if [[ "$HELLOWORLD_ENABLED" != "true" ]]; then
        log "  ⚠ HELLOWORLD_ENABLED=false → skipping (NOT recommended in prod)"
        HELLOWORLD_OK="true"
        HELLOWORLD_ELAPSED_S=0
        T_HELLOWORLD_END=$(date -u +%s)
        return 0
    fi

    # **B-2 directing principle** — refuse fallback by default.
    if [[ "$HELLOWORLD_FALLBACK_MOCK" == "true" ]]; then
        log "  ⚠ HELLOWORLD_FALLBACK_MOCK=true (B-2 violation in prod ; OK in CI/kind)"
    fi

    local hw_t0
    hw_t0=$(date -u +%s)

    if run_helloworld_steps >>"$HELLOWORLD_LOG" 2>&1; then
        HELLOWORLD_OK="true"
        log "  ✅ Hello-World workflow OK"
    else
        log "  ❌ Hello-World workflow failed at step '$HELLOWORLD_FAIL_STEP' — see $HELLOWORLD_LOG"
        HELLOWORLD_ELAPSED_S=$(( $(date -u +%s) - hw_t0 ))
        T_HELLOWORLD_END=$(date -u +%s)   # Audit fix M7-D40-A8 — set on fail too
        exit 6
    fi

    HELLOWORLD_ELAPSED_S=$(( $(date -u +%s) - hw_t0 ))

    # **SLO D40-DR-2** — Hello-World latency cap.
    local latency_ms=$(( HELLOWORLD_ELAPSED_S * 1000 ))
    if [[ "$latency_ms" -gt "$HELLOWORLD_MAX_LATENCY_MS" ]]; then
        log "  ❌ Hello-World latency=${latency_ms}ms > ${HELLOWORLD_MAX_LATENCY_MS}ms cap (SLO D40-DR-2)"
        T_HELLOWORLD_END=$(date -u +%s)
        exit 12
    fi
    log "  ✅ latency=${latency_ms}ms ≤ ${HELLOWORLD_MAX_LATENCY_MS}ms cap"

    T_HELLOWORLD_END=$(date -u +%s)
}

# Build the grpcurl auth header from EFE_DRILL_TOKEN (if set).
_grpc_auth_header() {
    if [[ -n "$DRILL_TOKEN" ]]; then
        printf -- "-H authorization: Bearer %s" "$DRILL_TOKEN"
    fi
}

# Run the 6-step Hello-World workflow via grpcurl against the live
# gRPC endpoint (efe-server in the drill namespace).
run_helloworld_steps() {
    local pod
    pod=$(kubectl --namespace "$NAMESPACE" get pods -l app.kubernetes.io/name=efe-server \
            -o jsonpath='{.items[0].metadata.name}' 2>/dev/null || echo "")
    if [[ -z "$pod" ]]; then
        HELLOWORLD_FAIL_STEP="0_pod_lookup"
        echo "FATAL: no efe-server pod ready in namespace $NAMESPACE"
        return 1
    fi
    echo "Hello-World pod: $pod (namespace=$NAMESPACE, grpc=$GRPC_ENDPOINT)"

    local auth_arg=""
    if [[ -n "$DRILL_TOKEN" ]]; then
        auth_arg="-H authorization: Bearer $DRILL_TOKEN"
    fi

    # Step 1 — register_catalog (gRPC efe.v1.catalog.CatalogService/RegisterCatalog)
    HELLOWORLD_FAIL_STEP="1_register_catalog"
    echo "Step 1/6 — register_catalog"
    local req1
    req1=$(jq -n \
        --arg tenant "$HELLOWORLD_TENANT_ID" \
        --arg name "$HELLOWORLD_CATALOG_NAME" \
        '{
          tenant: $tenant,
          name: $name,
          version: "v1",
          content: {
            embedder: {
              family: "sentence-transformers",
              model: "paraphrase-multilingual-MiniLM-L12-v2",
              dim: 384,
              normalize: "l2"
            },
            classes: [],
            cone_policy: { max_projection_distance: 0.7 },
            thresholds: { confidence_high: 0.8, confidence_medium: 0.5 }
          },
          created_by: "drill-helloworld",
          description: "DR drill Hello-World catalog (D40)"
        }')
    kubectl --namespace "$NAMESPACE" exec "$pod" -- \
        grpcurl -plaintext -max-time 10 \
            ${auth_arg:+$auth_arg} \
            -d "$req1" \
            "$GRPC_ENDPOINT" \
            efe.v1.catalog.CatalogService/RegisterCatalog \
        > "$EVIDENCE_DIR/_helloworld_step1.json" || return 1

    # Step 2 — register_class (EvolveCatalog with append_class op)
    HELLOWORLD_FAIL_STEP="2_register_class"
    echo "Step 2/6 — register_class"
    local from_key_v1="${HELLOWORLD_TENANT_ID}/${HELLOWORLD_CATALOG_NAME}@v1"
    local req2
    req2=$(jq -n \
        --arg from_key "$from_key_v1" \
        --arg class_name "$HELLOWORLD_CLASS_NAME" \
        '{
          from_key: $from_key,
          op: {
            append_class: {
              cls: {
                name: $class_name,
                description: "DR drill greeting class",
                canonical_sentences: [],
                color: "#00aa00",
                deprecated: false
              }
            }
          }
        }')
    kubectl --namespace "$NAMESPACE" exec "$pod" -- \
        grpcurl -plaintext -max-time 10 \
            ${auth_arg:+$auth_arg} \
            -d "$req2" \
            "$GRPC_ENDPOINT" \
            efe.v1.catalog.CatalogService/EvolveCatalog \
        > "$EVIDENCE_DIR/_helloworld_step2.json" || return 1

    # Step 3 — register_sentence (EvolveCatalog with replace_class_anchors op)
    HELLOWORLD_FAIL_STEP="3_register_sentence"
    echo "Step 3/6 — register_sentence"
    # Read the v2 key from step 2's response (more robust than hardcoding @v2).
    local from_key_v2
    from_key_v2=$(jq -r '.key // ""' "$EVIDENCE_DIR/_helloworld_step2.json")
    if [[ -z "$from_key_v2" ]]; then
        echo "FATAL: step 2 response missing .key field"
        return 1
    fi
    local req3
    req3=$(jq -n \
        --arg from_key "$from_key_v2" \
        --arg class_name "$HELLOWORLD_CLASS_NAME" \
        --arg sentence "$HELLOWORLD_SENTENCE" \
        '{
          from_key: $from_key,
          op: {
            replace_class_anchors: {
              class_name: $class_name,
              canonical_sentences: [$sentence]
            }
          }
        }')
    kubectl --namespace "$NAMESPACE" exec "$pod" -- \
        grpcurl -plaintext -max-time 10 \
            ${auth_arg:+$auth_arg} \
            -d "$req3" \
            "$GRPC_ENDPOINT" \
            efe.v1.catalog.CatalogService/EvolveCatalog \
        > "$EVIDENCE_DIR/_helloworld_step3.json" || return 1

    # Step 4 — resolve_catalog (GetCatalog with include_compiled=true)
    HELLOWORLD_FAIL_STEP="4_resolve_catalog"
    echo "Step 4/6 — resolve_catalog"
    local from_key_v3
    from_key_v3=$(jq -r '.key // ""' "$EVIDENCE_DIR/_helloworld_step3.json")
    if [[ -z "$from_key_v3" ]]; then
        echo "FATAL: step 3 response missing .key field"
        return 1
    fi
    local req4
    req4=$(jq -n --arg key "$from_key_v3" \
        '{ key: $key, include_compiled: true }')
    kubectl --namespace "$NAMESPACE" exec "$pod" -- \
        grpcurl -plaintext -max-time 10 \
            ${auth_arg:+$auth_arg} \
            -d "$req4" \
            "$GRPC_ENDPOINT" \
            efe.v1.catalog.CatalogService/GetCatalog \
        > "$EVIDENCE_DIR/_helloworld_step4.json" || return 1

    # Step 5 — execute_workflow (SwarmService.Execute with the catalog as cone_ref)
    HELLOWORLD_FAIL_STEP="5_execute_workflow"
    echo "Step 5/6 — execute_workflow"
    local req5
    req5=$(jq -n \
        --arg request_id "drill-helloworld-$(date -u +%s)" \
        --arg raw_prompt "$HELLOWORLD_SENTENCE" \
        --arg tenant "$HELLOWORLD_TENANT_ID" \
        --arg catalog_name "$HELLOWORLD_CATALOG_NAME" \
        '{
          request_id: $request_id,
          raw_prompt: $raw_prompt,
          spec: {
            max_rounds: 3,
            alignment_cone_ref: {
              tenant: $tenant,
              task: $catalog_name,
              version: 3
            }
          },
          decode_mode: { text: { max_tokens: 16 } }
        }')
    kubectl --namespace "$NAMESPACE" exec "$pod" -- \
        grpcurl -plaintext -max-time 60 \
            ${auth_arg:+$auth_arg} \
            -d "$req5" \
            "$GRPC_ENDPOINT" \
            efe.v1.SwarmService/Execute \
        > "$EVIDENCE_DIR/_helloworld_step5.json" || return 1

    # Step 6 — assert_response (HMAC chain integrity post-Execute via efe-verify-audit)
    # The D17 baseline ships `efe-verify-audit` (+ M5 monitoring).
    # We tolerate the binary being absent (older images) — the drill
    # still emits a clear signal in `hmac_chain_post_execute_ok`.
    HELLOWORLD_FAIL_STEP="6_assert_hmac_chain"
    echo "Step 6/6 — assert_response (HMAC chain integrity)"
    local hmac_ok="false"
    if kubectl --namespace "$NAMESPACE" exec "$pod" -- \
            sh -c 'command -v efe-verify-audit >/dev/null 2>&1' \
            >/dev/null 2>&1; then
        if kubectl --namespace "$NAMESPACE" exec "$pod" -- \
                efe-verify-audit --since 1m --full >/dev/null 2>&1; then
            hmac_ok="true"
            echo "  ✅ HMAC chain intact post-Execute"
        else
            echo "  ❌ HMAC chain broken post-Execute"
            return 1
        fi
    else
        echo "  ⚠ efe-verify-audit not in image — emitting hmac_ok=false (DT-DR.40-3)"
        # Don't fail the drill ; mark the field so the auditor sees it.
    fi

    # Reset fail step on success (for emit_evidence narrative).
    HELLOWORLD_FAIL_STEP=""

    # Build the helloworld JSON (consumed by emit_evidence).
    local exec_response="$EVIDENCE_DIR/_helloworld_step5.json"
    HELLOWORLD_DETAIL_JSON=$(jq -n \
        --arg schema_version "1.0.0" \
        --arg tenant_id "$HELLOWORLD_TENANT_ID" \
        --arg catalog_name "$HELLOWORLD_CATALOG_NAME" \
        --arg class_name "$HELLOWORLD_CLASS_NAME" \
        --arg sentence "$HELLOWORLD_SENTENCE" \
        --argjson final_version 3 \
        --argjson anchors "$(jq -r '(.compiled[0].num_anchors // 0)' "$EVIDENCE_DIR/_helloworld_step4.json" 2>/dev/null || echo 0)" \
        --argjson execute_response "$(cat "$exec_response" 2>/dev/null || echo '{}')" \
        --argjson hmac_ok "$hmac_ok" \
        --argjson all_ok true \
        --argjson elapsed_secs "$HELLOWORLD_ELAPSED_S" \
        --argjson fallback_mock "$([[ "$HELLOWORLD_FALLBACK_MOCK" == "true" ]] && echo true || echo false)" \
        '{
          schema_version: $schema_version,
          tenant_id: $tenant_id,
          catalog_name: $catalog_name,
          final_version: $final_version,
          class_name: $class_name,
          sentence: $sentence,
          anchors_compiled: $anchors,
          execute: {
            request_id: ($execute_response.request_id // ""),
            verdict: ($execute_response.verdict // "VERDICT_UNSPECIFIED"),
            confidence: ($execute_response.confidence // 0),
            total_ms: ($execute_response.total_ms // 0),
            rounds_executed: ($execute_response.rounds_executed // 0),
          },
          hmac_chain_post_execute_ok: $hmac_ok,
          all_steps_ok: $all_ok,
          elapsed_secs: $elapsed_secs,
          fallback_mock_enabled: $fallback_mock,
        }')
    return 0
}

# ─────────────────────────────────────────────────────────────────────────────
# § 7. Phase 5 — verifier suite (`efe-backup drill --target-env=drill`)
# ─────────────────────────────────────────────────────────────────────────────

phase_verifier_suite() {
    log "▶️  Phase 5/6 — verifier suite"
    local pod
    pod=$(kubectl --namespace "$NAMESPACE" get pods -l app.kubernetes.io/name=efe-server \
            -o jsonpath='{.items[0].metadata.name}' 2>/dev/null || echo "")
    if [[ -z "$pod" ]]; then
        log "  ❌ no efe-server pod ready in namespace $NAMESPACE (verifier suite skipped)"
        exit 7
    fi
    if kubectl --namespace "$NAMESPACE" exec "$pod" -- \
            efe-backup drill --target-env="$TARGET_ENV" --json \
            > "$EVIDENCE_DIR/_verifier_raw.json" 2>>"$VERIFIER_LOG"; then
        VERIFIER_OK="true"
        VERIFIER_DETAIL_JSON=$(jq '.verifiers // []' "$EVIDENCE_DIR/_verifier_raw.json")
        log "  ✅ 5/5 verifiers OK"
    else
        log "  ❌ verifier suite failed — see $VERIFIER_LOG"
        VERIFIER_DETAIL_JSON=$(jq '.verifiers // []' "$EVIDENCE_DIR/_verifier_raw.json" 2>/dev/null || echo '[]')
        exit 7
    fi
    T_VERIFIER_END=$(date -u +%s)
}

# ─────────────────────────────────────────────────────────────────────────────
# § 8. Phase 6 — compliance evidence emit
# ─────────────────────────────────────────────────────────────────────────────

emit_evidence() {
    local rc="${1:-0}"
    log "▶️  Phase 6/6 — compliance evidence emit"
    T_END=$(date -u +%s)

    # Per-phase secs (if a phase didn't complete, its END is 0 → secs = 0).
    local kind_secs=$(( T_KIND_END > 0 ? T_KIND_END - T_START : 0 ))
    local helm_secs=$(( T_HELM_END > T_KIND_END ? T_HELM_END - T_KIND_END : 0 ))
    local restore_secs=$(( T_RESTORE_END > T_HELM_END ? T_RESTORE_END - T_HELM_END : 0 ))
    local helloworld_secs=$(( T_HELLOWORLD_END > T_RESTORE_END ? T_HELLOWORLD_END - T_RESTORE_END : 0 ))
    local verifier_secs=$(( T_VERIFIER_END > T_HELLOWORLD_END ? T_VERIFIER_END - T_HELLOWORLD_END : 0 ))
    local cleanup_secs=$(( T_END > T_VERIFIER_END ? T_END - T_VERIFIER_END : 0 ))
    local duration_secs=$(( T_END - T_START ))
    local within_rto="false"
    [[ "$duration_secs" -le "$RTO_TARGET_S" ]] && within_rto="true"

    # Compute success_overall (fail-closed).
    local success_overall="false"
    if [[ "$KIND_OK" == "true" && "$HELM_OK" == "true" && "$RESTORE_OK" == "true" \
            && "$HELLOWORLD_OK" == "true" && "$VERIFIER_OK" == "true" \
            && "$within_rto" == "true" ]]; then
        success_overall="true"
    fi

    # Emit timing.json.
    jq -n \
        --argjson kind_secs "$kind_secs" \
        --argjson helm_secs "$helm_secs" \
        --argjson restore_secs "$restore_secs" \
        --argjson helloworld_secs "$helloworld_secs" \
        --argjson verifier_secs "$verifier_secs" \
        --argjson cleanup_secs "$cleanup_secs" \
        --argjson duration_secs "$duration_secs" \
        --argjson rto_target_secs "$RTO_TARGET_S" \
        --argjson within_rto "$within_rto" \
        '{
          phases: {
            kind_setup_secs: $kind_secs,
            helm_install_secs: $helm_secs,
            restore_secs: $restore_secs,
            verifier_suite_secs: $verifier_secs,
            helloworld_secs: $helloworld_secs,
            cleanup_secs: $cleanup_secs,
          },
          duration_secs: $duration_secs,
          rto_target_secs: $rto_target_secs,
          within_rto: $within_rto,
        }' > "$TIMING_JSON"

    # Emit outcome.json (schema v2). Portable date formatting :
    # macOS BSD `date -u -r EPOCH` ; GNU coreutils `date -u -d @EPOCH`.
    local started_at finished_at
    started_at="$(date -u -r "$T_START" +%FT%TZ 2>/dev/null || date -u -d "@$T_START" +%FT%TZ)"
    finished_at="$(date -u -r "$T_END" +%FT%TZ 2>/dev/null || date -u -d "@$T_END" +%FT%TZ)"

    jq -n \
        --arg schema_version "2.0.0" \
        --arg drill_kind "quarterly" \
        --arg drill_label "$DRILL_LABEL" \
        --arg started_at "$started_at" \
        --arg finished_at "$finished_at" \
        --argjson duration_secs "$duration_secs" \
        --argjson rto_target_secs "$RTO_TARGET_S" \
        --argjson within_rto "$within_rto" \
        --argjson kind_secs "$kind_secs" \
        --argjson helm_secs "$helm_secs" \
        --argjson restore_secs "$restore_secs" \
        --argjson helloworld_secs "$helloworld_secs" \
        --argjson verifier_secs "$verifier_secs" \
        --argjson cleanup_secs "$cleanup_secs" \
        --argjson verifiers "$VERIFIER_DETAIL_JSON" \
        --argjson helloworld "$HELLOWORLD_DETAIL_JSON" \
        --arg attestation_pubkey_id "$ATTESTATION_KEY_ID" \
        --argjson success_overall "$success_overall" \
        '{
          schema_version: $schema_version,
          drill_kind: $drill_kind,
          drill_label: $drill_label,
          started_at: $started_at,
          finished_at: $finished_at,
          duration_secs: $duration_secs,
          rto_target_secs: $rto_target_secs,
          within_rto: $within_rto,
          phases: {
            kind_setup_secs: $kind_secs,
            helm_install_secs: $helm_secs,
            restore_secs: $restore_secs,
            verifier_suite_secs: $verifier_secs,
            helloworld_secs: $helloworld_secs,
            cleanup_secs: $cleanup_secs,
          },
          verifiers: $verifiers,
          helloworld: $helloworld,
          compliance_frameworks: ["SOC2-CC9.1", "ISO27001-A.17.1.3", "NIS2-Art21", "RGPD-Art32"],
          attestation_pubkey_id: $attestation_pubkey_id,
          success_overall: $success_overall,
        }' > "$OUTCOME_JSON.unsigned"

    # Sign + finalise.
    sign_attestation "$OUTCOME_JSON.unsigned" "$OUTCOME_JSON" "$ATTESTATION_TXT"

    # Cleanup intermediate files.
    rm -f "$OUTCOME_JSON.unsigned" \
          "$EVIDENCE_DIR/_verifier_raw.json" \
          "$EVIDENCE_DIR/_helloworld_step"*.json

    log "  ✅ evidence pack at $EVIDENCE_DIR/"

    # **Audit fix M7-D40-A1 (CRIT)** — SLO D40-DR-1 RTO violation
    # surfacing : if the script otherwise passed (rc=0) BUT the RTO
    # was violated, override with rc=11 so CI gates fire. Pre-audit
    # the cleanup trap swallowed this rc via `|| log`.
    if [[ "$within_rto" != "true" && "$rc" -eq 0 ]]; then
        log "  ❌ RTO violation : ${duration_secs}s > ${RTO_TARGET_S}s (SLO D40-DR-1)"
        return 11
    fi
    return "$rc"
}

# Sign the unsigned outcome with Ed25519, emit the final outcome.json,
# and write a human-readable attestation.txt.
#
# **Audit fix M7-D40 § 3.1** — the signature pipeline is best-effort :
# if the privkey path is unset or the file is missing, we degrade
# gracefully (outcome still emitted, signature field absent, auditor
# sees a clear signal). This mirrors the D17 § AC3 + AH10 pattern.
sign_attestation() {
    local unsigned="$1"
    local signed="$2"
    local attest_file="$3"
    local sig=""
    local privkey_path="${EFE_ATTESTATION_PRIVKEY_PATH:-}"

    if [[ -n "$privkey_path" && -f "$privkey_path" ]]; then
        # Canonicalise (jq -cS = compact + sorted).
        local canonical
        canonical=$(jq -cS . "$unsigned")
        # Sha256 the canonicalised payload.
        local digest
        # macOS: `shasum -a 256` ; Linux: `sha256sum`. Try both.
        if command -v sha256sum >/dev/null 2>&1; then
            digest=$(printf '%s' "$canonical" | sha256sum | awk '{print $1}')
        elif command -v shasum >/dev/null 2>&1; then
            digest=$(printf '%s' "$canonical" | shasum -a 256 | awk '{print $1}')
        else
            digest=""
        fi
        # Sign with openssl + raw Ed25519 key.
        if [[ -n "$digest" ]] && command -v openssl >/dev/null 2>&1; then
            sig=$(printf '%s' "$digest" \
                | openssl pkeyutl -sign -inkey "$privkey_path" -rawin 2>/dev/null \
                | base64 2>/dev/null \
                | tr -d '\n' || echo "")
        fi
    fi

    if [[ -n "$sig" ]]; then
        jq --arg sig "$sig" '. + {attestation_signature: $sig}' "$unsigned" > "$signed"
    else
        # No sig available — emit unsigned + leave the field absent.
        jq '.' "$unsigned" > "$signed"
    fi

    # Human-readable attestation.
    {
        echo "EFE Quarterly DR Drill — Attestation"
        echo "===================================="
        echo "Drill label    : $DRILL_LABEL"
        echo "Schema version : 2.0.0"
        echo "Attestation key: $ATTESTATION_KEY_ID"
        echo "Started at     : $(jq -r .started_at "$signed" 2>/dev/null || echo unknown)"
        echo "Finished at    : $(jq -r .finished_at "$signed" 2>/dev/null || echo unknown)"
        echo "Duration       : $(jq -r .duration_secs "$signed" 2>/dev/null || echo 0)s (RTO target: ${RTO_TARGET_S}s)"
        echo "Within RTO     : $(jq -r .within_rto "$signed" 2>/dev/null || echo unknown)"
        echo "Success overall: $(jq -r .success_overall "$signed" 2>/dev/null || echo unknown)"
        echo "Frameworks     : SOC2-CC9.1, ISO27001-A.17.1.3, NIS2-Art21, RGPD-Art32"
        if [[ -n "$sig" ]]; then
            echo "Signature      : (Ed25519 base64) ${sig:0:64}..."
        else
            echo "Signature      : <unsigned — EFE_ATTESTATION_PRIVKEY_PATH unset or sign failed>"
        fi
        echo
        echo "This file is part of the compliance evidence pack."
        echo "See docs/compliance/dr_drill_quarterly_evidence.md for the auditor playbook."
    } > "$attest_file"
}

# ─────────────────────────────────────────────────────────────────────────────
# § 9. Main
# ─────────────────────────────────────────────────────────────────────────────

main() {
    log "🚀 EFE Quarterly DR drill starting"
    log "   sprint=D40 (Φ-F)"
    log "   drill_label=$DRILL_LABEL"
    log "   target_env=$TARGET_ENV (B-8 cross-env safety enforced)"

    preflight
    phase_kind_setup
    phase_helm_install
    phase_restore
    phase_helloworld
    phase_verifier_suite

    # emit_evidence is run by the cleanup trap on EXIT, so we don't call
    # it explicitly here. The trap ensures evidence is written even if
    # a phase fails mid-way (B-3 directing principle).

    log "✅ EFE Quarterly DR drill MAIN COMPLETE"
}

main "$@"
