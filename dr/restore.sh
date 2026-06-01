#!/usr/bin/env bash
# Sprint D17 — DR restore runbook (canonical script).
#
# Cf. `docs/bench/d17_backup_dr_architecture.md` § 10.2.
#
# Restores in the architecture-mandated order :
#   1. Vault   (HMAC keys, OIDC issuers, trust store)
#   2. Postgres (catalog tables + audit_log)
#   3. Bundles  (rclone S3 sync from backup bucket)
#   4. Helm release (re-applies the runtime config)
#   5. Verify  (HMAC chain, bundle signatures, OIDC reachability)
#   6. Smoke   (efe-server catalog list + Execute RPC)
#
# RTO target : 30 minutes end-to-end (D17 SLO N-DR-1).
#
# Usage :
#   ./dr/restore.sh <backup-id> --target <env> [--target-time <iso8601>] [--force-cross-env]
#
# Environment variables :
#   EFE_BACKUP_*               (rendered by Helm ConfigMap)
#   EFE_DR_HELM_VALUES_PATH    (default: deploy/helm/efe/values-${TARGET_ENV}.yaml)
#   EFE_DR_NAMESPACE           (default: efe)
#
# Exit codes :
#   0   — restore + smoke succeeded
#   1   — argument parse error
#   2   — pre-flight failed
#   3   — Vault restore failed
#   4   — Postgres restore failed
#   5   — Bundles restore failed
#   6   — Helm upgrade failed
#   7   — Post-restore verify failed
#   8   — Smoke test failed

set -euo pipefail

BACKUP_ID="${1:-}"
shift || true
TARGET_ENV=""
TARGET_TIME=""
FORCE_CROSS_ENV=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --target)            TARGET_ENV="$2"; shift 2 ;;
        --target-time)       TARGET_TIME="$2"; shift 2 ;;
        --force-cross-env)   FORCE_CROSS_ENV="--force-cross-env"; shift ;;
        --help|-h)
            sed -n '2,40p' "$0"
            exit 0
            ;;
        *) echo "unknown arg: $1" >&2; exit 1 ;;
    esac
done

if [[ -z "$BACKUP_ID" || -z "$TARGET_ENV" ]]; then
    echo "Usage: $0 <backup-id> --target <env> [--target-time <iso8601>] [--force-cross-env]" >&2
    exit 1
fi

NAMESPACE="${EFE_DR_NAMESPACE:-efe}"
HELM_VALUES_PATH="${EFE_DR_HELM_VALUES_PATH:-deploy/helm/efe/values-${TARGET_ENV}.yaml}"

T0=$(date -u +%s)

log() {
    echo "[$(date -u +%FT%TZ)] $*"
}

log "DR restore starting"
log "  backup_id=$BACKUP_ID"
log "  target_env=$TARGET_ENV"
log "  target_time=${TARGET_TIME:-latest}"
log "  namespace=$NAMESPACE"

# 0. Pre-flight : verify the backup is intact at rest before any
# side-effect.
log "Step 0/6 — pre-flight verify..."
efe-backup verify --backup-id="$BACKUP_ID" || {
    log "FATAL: backup verification failed"
    exit 2
}

# Audit fix § D17-AC5 : each step restores ONE source (--source).
# Pre-audit, the CLI looped over all 3 sources internally so each
# of these 3 shell calls restored 3× (each source restored 9×
# total). Post-audit, the CLI honors --source so each step
# restores exactly its one source.

# 1. Vault restore — MUST come first because Postgres restore
#    re-verifies the audit chain HMAC against the keys in Vault.
log "Step 1/6 — restoring Vault..."
efe-backup restore --backup-id="$BACKUP_ID" \
    --target-env="$TARGET_ENV" --source=vault $FORCE_CROSS_ENV \
    || { log "FATAL: vault restore failed"; exit 3; }

# 2. Postgres restore. Optional --target-time enables PITR.
log "Step 2/6 — restoring Postgres..."
PG_ARGS=(--backup-id="$BACKUP_ID" --target-env="$TARGET_ENV" --source=postgres $FORCE_CROSS_ENV)
if [[ -n "$TARGET_TIME" ]]; then
    PG_ARGS+=(--target-time "$TARGET_TIME")
fi
efe-backup restore "${PG_ARGS[@]}" || {
    log "FATAL: postgres restore failed"
    exit 4
}

# 3. Bundles restore (rclone reverse sync).
log "Step 3/6 — restoring bundles..."
efe-backup restore --backup-id="$BACKUP_ID" \
    --target-env="$TARGET_ENV" --source=bundles $FORCE_CROSS_ENV \
    || { log "FATAL: bundles restore failed"; exit 5; }

# 4. Helm upgrade (re-applies the runtime config so the new pods
#    pick up freshly-restored secrets/configmaps).
log "Step 4/6 — Helm upgrade..."
if [[ -f "$HELM_VALUES_PATH" ]]; then
    helm upgrade --install efe ./deploy/helm/efe \
        --namespace "$NAMESPACE" \
        -f "$HELM_VALUES_PATH" || {
        log "FATAL: helm upgrade failed"
        exit 6
    }
else
    log "WARN: $HELM_VALUES_PATH not found, skipping helm upgrade"
fi

# 5. Post-restore verify : HMAC chain, bundle signatures, OIDC,
#    catalog smoke, Execute smoke.
log "Step 5/6 — post-restore verify..."
efe-backup verify --backup-id="$BACKUP_ID" || {
    log "FATAL: post-restore verify failed"
    exit 7
}

# 6. Smoke test (separate script).
log "Step 6/6 — smoke test..."
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
"${SCRIPT_DIR}/smoke_test.sh" "$TARGET_ENV" "$NAMESPACE" || {
    log "FATAL: smoke test failed"
    exit 8
}

T1=$(date -u +%s)
DURATION=$((T1 - T0))

log "DR restore complete in ${DURATION}s"

# Emit timing.json for the CI drill harness.
cat > "${SCRIPT_DIR}/timing.json" <<EOF
{
  "backup_id": "$BACKUP_ID",
  "target_env": "$TARGET_ENV",
  "duration_seconds": $DURATION,
  "rto_target_seconds": ${EFE_BACKUP_RTO_TARGET_S:-1800}
}
EOF

# Architecture § 22 acceptance criterion : RTO ≤ 30 min.
RTO_TARGET="${EFE_BACKUP_RTO_TARGET_S:-1800}"
if [[ "$DURATION" -gt "$RTO_TARGET" ]]; then
    log "WARN: RTO violation ($DURATION s > $RTO_TARGET s target)"
    exit 11
fi

exit 0
