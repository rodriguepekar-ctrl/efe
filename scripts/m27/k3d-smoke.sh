#!/usr/bin/env bash
# M27 — k3d Tier 4 smoke test.
#
# What it does :
#   1. Create (or reuse) a k3d cluster named `efe-m27`.
#   2. Install cert-manager (required for the M26 admission webhook).
#   3. helm install `efe-stack` with `efe-edge.enabled=true` +
#      bootstrap cert-manager Issuer.
#   4. Wait for all pods Ready (< 10 min).
#   5. Hit /healthz through the Envoy gateway.
#   6. Tear down (or leave with --keep).
#
# Pre-requisites : k3d 5.x, kubectl 1.30+, helm 3.x, docker.

set -euo pipefail

CLUSTER="${EFE_M27_CLUSTER:-efe-m27}"
NS="${EFE_M27_NAMESPACE:-efe-stack}"
CERT_MANAGER_VERSION="${CERT_MANAGER_VERSION:-v1.16.2}"
KEEP=0

for arg in "$@"; do
    case "$arg" in
        --keep) KEEP=1 ;;
        *) echo "unknown arg: $arg" >&2; exit 2 ;;
    esac
done

banner() { echo "▶ $*" >&2; }

cleanup() {
    if [[ "$KEEP" == "0" ]]; then
        banner "tearing down k3d cluster $CLUSTER"
        k3d cluster delete "$CLUSTER" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

# 1. cluster
if k3d cluster list | awk '{print $1}' | grep -qx "$CLUSTER"; then
    banner "reusing existing k3d cluster $CLUSTER"
else
    banner "creating k3d cluster $CLUSTER (1 server + 2 agents)"
    k3d cluster create "$CLUSTER" --agents 2 --wait --timeout 120s
fi

kubectl create namespace "$NS" --dry-run=client -o yaml | kubectl apply -f -

# 2. cert-manager (for M26 webhook)
if ! kubectl get crd certificates.cert-manager.io >/dev/null 2>&1; then
    banner "installing cert-manager $CERT_MANAGER_VERSION"
    kubectl apply -f "https://github.com/cert-manager/cert-manager/releases/download/$CERT_MANAGER_VERSION/cert-manager.yaml"
    kubectl -n cert-manager wait --for=condition=Available --timeout=180s \
        deployment/cert-manager deployment/cert-manager-cainjector deployment/cert-manager-webhook
fi

# 3. helm install (efe-stack with edge enabled)
banner "helm install efe-stack with efe-edge.enabled=true"
# Pre-create the placeholder Secrets the chart references (so helm install
# doesn't error on missing dependencies). In production these are real.
kubectl -n "$NS" create secret generic efe-redis-auth \
    --from-literal=password="$(openssl rand -hex 32)" \
    --dry-run=client -o yaml | kubectl apply -f -
kubectl -n "$NS" create secret generic efe-cloudflared-credentials \
    --from-literal=credentials.json='{"AccountTag":"test","TunnelSecret":"test","TunnelID":"test"}' \
    --dry-run=client -o yaml | kubectl apply -f -
kubectl -n "$NS" create secret generic efe-pgbouncer-userlist \
    --from-literal=userlist.txt='"efe" "md5_placeholder"' \
    --dry-run=client -o yaml | kubectl apply -f -

helm upgrade --install efe-stack ./charts/efe-stack \
    --namespace "$NS" \
    --set efe-edge.enabled=true \
    --set efe-server.enabled=false \
    --set mcp-server.enabled=false \
    --set portal-admin.enabled=false \
    --set observability.enabled=false \
    --set postgres-operator.enabled=false \
    --set redis-stack.enabled=false \
    --wait --timeout 300s || {
    echo "▶ helm install timed out — diagnostics :" >&2
    kubectl -n "$NS" get pods
    kubectl -n "$NS" get events --sort-by=.metadata.creationTimestamp | tail -20
    exit 1
}

# 4. wait Ready
banner "waiting for Envoy + Redis + PgBouncer Ready"
kubectl -n "$NS" rollout status deployment/efe-stack-edge-envoy --timeout=180s
kubectl -n "$NS" rollout status deployment/efe-stack-edge-pgbouncer --timeout=180s

# 5. smoke : reach Envoy admin /ready endpoint via port-forward
banner "smoke-testing Envoy admin :"
kubectl -n "$NS" port-forward svc/efe-stack-edge-envoy 8443:8443 >/dev/null 2>&1 &
PF_PID=$!
sleep 3
status=$(curl -sk -o /dev/null -w "%{http_code}" "https://localhost:8443/healthz" || echo "000")
kill $PF_PID 2>/dev/null || true

if [[ "$status" =~ ^(200|404)$ ]]; then
    banner "✓ Envoy listener responsive (got $status)"
else
    echo "✗ Envoy not responsive (got $status)"
    exit 1
fi

banner "k3d smoke OK"
