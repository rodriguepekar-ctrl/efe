#!/usr/bin/env bash
# Sprint D18 — kind cluster setup for multi-replica chaos + bench tests.
#
# Spins a kind cluster with :
#   * 3 worker nodes (forces real spread of replicas)
#   * Postgres via cloudnative-pg
#   * EFE chart with replicas.min=3
#   * Optional : pre-populate efe_idempotency + efe_ratelimit_buckets

set -euo pipefail

CLUSTER_NAME="efe-d18-multi-replica"
KEEP_AFTER=0
SKIP_HELM=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --cluster-name) CLUSTER_NAME="$2"; shift 2 ;;
        --keep)         KEEP_AFTER=1; shift ;;
        --skip-helm)    SKIP_HELM=1; shift ;;
        --help|-h)
            sed -n '2,15p' "$0"
            exit 0
            ;;
        *) echo "unknown arg: $1" >&2; exit 1 ;;
    esac
done

log() { echo "[$(date -u +%FT%TZ)] $*"; }

if ! command -v kind > /dev/null 2>&1; then
    log "ERROR: kind binary not found"
    exit 1
fi

# 1. Create / reset the kind cluster with 3 worker nodes.
if kind get clusters 2>/dev/null | grep -qx "$CLUSTER_NAME"; then
    log "Reusing existing kind cluster $CLUSTER_NAME"
else
    log "Creating kind cluster $CLUSTER_NAME (3 workers)..."
    cat > /tmp/kind-multi-replica.yaml <<'EOF'
kind: Cluster
apiVersion: kind.x-k8s.io/v1alpha4
nodes:
  - role: control-plane
  - role: worker
    labels:
      topology.kubernetes.io/zone: zone-a
  - role: worker
    labels:
      topology.kubernetes.io/zone: zone-b
  - role: worker
    labels:
      topology.kubernetes.io/zone: zone-c
EOF
    kind create cluster --name "$CLUSTER_NAME" --config /tmp/kind-multi-replica.yaml
    rm -f /tmp/kind-multi-replica.yaml
fi

# 2. Install cloudnative-pg.
if [[ "$SKIP_HELM" -eq 0 ]]; then
    log "Installing cloudnative-pg operator..."
    helm repo add cnpg https://cloudnative-pg.github.io/charts 2>/dev/null || true
    helm upgrade --install cnpg cnpg/cloudnative-pg \
        --namespace cnpg-system --create-namespace --wait

    # 3. Create a Postgres cluster.
    log "Creating efe-postgres cluster..."
    kubectl apply -f - <<'EOF'
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata:
  name: efe-postgres
  namespace: efe
spec:
  instances: 1
  bootstrap:
    initdb:
      database: efe
      owner: efe
  storage:
    size: 1Gi
EOF

    # 4. Apply EFE chart with replicas.min=3.
    log "Applying EFE chart with replicas.min=3..."
    helm upgrade --install efe ./deploy/helm/efe \
        --namespace efe --create-namespace \
        --set replicas.min=3 \
        --set replicas.max=5 \
        --set leaderElection.enabled=true \
        --set service.headless=true \
        --wait
fi

log "Multi-replica kind cluster ready : cluster=$CLUSTER_NAME"
log "Verify replicas : kubectl --namespace efe get pods -l app.kubernetes.io/name=efe-server -o wide"

if [[ "$KEEP_AFTER" -eq 0 ]]; then
    log "Pass --keep to preserve the cluster after this script."
fi
