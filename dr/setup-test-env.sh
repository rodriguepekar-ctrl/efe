#!/usr/bin/env bash
# Sprint D17 — DR drill test-env setup.
#
# Spins a kind cluster with Postgres + MinIO + Vault for an
# end-to-end DR drill in CI.
#
# Idempotent : re-runs cleanly against an existing kind cluster.
#
# Usage : ./dr/setup-test-env.sh [--cluster-name <name>] [--keep]

set -euo pipefail

CLUSTER_NAME="efe-dr-drill"
KEEP=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --cluster-name) CLUSTER_NAME="$2"; shift 2 ;;
        --keep)         KEEP=1; shift ;;
        --help|-h)
            sed -n '2,15p' "$0"
            exit 0
            ;;
        *) echo "unknown arg: $1" >&2; exit 1 ;;
    esac
done

log() { echo "[$(date -u +%FT%TZ)] $*"; }

if ! command -v kind > /dev/null 2>&1; then
    log "ERROR: kind binary not found (install via https://kind.sigs.k8s.io/)"
    exit 1
fi
if ! command -v helm > /dev/null 2>&1; then
    log "ERROR: helm binary not found"
    exit 1
fi

# 1. Create / reset the kind cluster.
if kind get clusters 2>/dev/null | grep -qx "$CLUSTER_NAME"; then
    log "Reusing existing kind cluster $CLUSTER_NAME"
else
    log "Creating kind cluster $CLUSTER_NAME"
    kind create cluster --name "$CLUSTER_NAME"
fi

# 2. Install MinIO (S3-compatible) for backup storage.
log "Installing MinIO..."
helm repo add minio https://charts.min.io/ 2>/dev/null || true
helm upgrade --install minio minio/minio \
    --namespace minio --create-namespace \
    --set rootUser=adminadmin \
    --set rootPassword=adminadmin \
    --set persistence.size=2Gi \
    --set replicas=1 \
    --wait

# 3. Install Postgres via cloudnative-pg.
log "Installing Postgres..."
helm repo add cnpg https://cloudnative-pg.github.io/charts 2>/dev/null || true
helm upgrade --install cnpg cnpg/cloudnative-pg \
    --namespace cnpg --create-namespace \
    --wait

# 4. Apply EFE chart with backup.enabled=true.
log "Applying EFE chart with backup enabled..."
helm upgrade --install efe ./deploy/helm/efe \
    --namespace efe --create-namespace \
    --set backup.enabled=true \
    --set backup.devSecretValues=true \
    --set backup.storage.bucket=efe-backups-dev \
    --set backup.storage.region=us-east-1 \
    --set backup.bundles.sourceBucket=efe-bundles-dev \
    --set backup.bundles.destBucket=efe-bundle-backups-dev \
    --wait

log "Test env setup complete : cluster=$CLUSTER_NAME"

if [[ "$KEEP" -eq 0 ]]; then
    log "Use --keep to preserve the cluster after the script exits."
fi
