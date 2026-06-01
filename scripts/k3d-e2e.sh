#!/usr/bin/env bash
# scripts/k3d-e2e.sh — full local stack smoke test on k3d.
#
# Prerequisites :
#   - docker
#   - k3d 5.x
#   - helm 3.15+
#   - kubectl 1.30+
#
# Usage :
#   ./scripts/k3d-e2e.sh [cluster-name]

set -euo pipefail

CLUSTER="${1:-efe-e2e-$(date +%s)}"
NAMESPACE="efe"

cleanup() {
  echo "🧹 Cleaning up cluster $CLUSTER..."
  k3d cluster delete "$CLUSTER" >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "🚀 Creating k3d cluster $CLUSTER..."
k3d cluster create "$CLUSTER" \
  --servers 1 --agents 2 \
  --no-lb \
  --wait --timeout 5m

echo "📦 Installing efe-stack..."
helm install efe-stack ./charts/efe-stack \
  --values ./charts/efe-stack/values-local.yaml \
  --set global.tenant=test \
  --set postgres-operator.enabled=false \
  --create-namespace --namespace "$NAMESPACE" \
  --wait --timeout 10m

echo "🔍 Pods status :"
kubectl get pods -n "$NAMESPACE"

echo "⏳ Waiting for pods Ready..."
kubectl wait -n "$NAMESPACE" \
  --for=condition=Ready pod \
  -l 'app.kubernetes.io/name in (efe-server,mcp-server,redis-stack)' \
  --timeout 180s

echo "✅ E2E PASS — efe-stack deployed locally on k3d cluster $CLUSTER"
echo
echo "Useful commands :"
echo "  kubectl logs -n $NAMESPACE deploy/test-efe-server"
echo "  kubectl port-forward -n $NAMESPACE svc/test-efe-server 50051"
echo "  kubectl port-forward -n $NAMESPACE svc/test-mcp-server 8080"
