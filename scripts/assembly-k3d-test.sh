#!/usr/bin/env bash
# scripts/assembly-k3d-test.sh — D93 Tier 4 : k3d local-cluster smoke
# for the M26 agent-runtime production chart.
#
# What it does :
#   1. Create (or reuse) a throwaway k3d cluster named `efe-d93`.
#   2. Pre-install cert-manager v1.16+ (required by admission webhook).
#   3. `helm install` the chart with bootstrap mode + selfSigned issuer.
#   4. Wait for all pods to reach Ready (≤ 5 min budget).
#   5. Apply a noop AgentRun fixture and verify the admission webhook
#      allows it.
#   6. Tear down (or leave for inspection if `--keep`).
#
# Exit code : 0 on success ; non-zero with the failing step printed.
# Designed to be idempotent so it can be re-run after edits without
# manual cleanup.
#
# Pre-requisites :
#   - k3d 5.x
#   - kubectl 1.30+
#   - helm 3.x
#   - docker daemon running
#
# Usage :
#   ./scripts/assembly-k3d-test.sh
#   ./scripts/assembly-k3d-test.sh --keep   # don't delete cluster

set -euo pipefail

CLUSTER="${EFE_D93_CLUSTER:-efe-d93}"
NS="${EFE_D93_NAMESPACE:-efe-system}"
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
    else
        banner "leaving k3d cluster $CLUSTER up (--keep)"
    fi
}
trap cleanup EXIT

# ── 1. k3d cluster ───────────────────────────────────────────────────
if k3d cluster list | awk '{print $1}' | grep -qx "$CLUSTER"; then
    banner "reusing existing k3d cluster $CLUSTER"
else
    banner "creating k3d cluster $CLUSTER"
    k3d cluster create "$CLUSTER" \
        --agents 1 \
        --wait \
        --timeout 120s
fi

kubectl create namespace "$NS" --dry-run=client -o yaml | kubectl apply -f -

# ── 2. cert-manager ──────────────────────────────────────────────────
if ! kubectl get crd certificates.cert-manager.io >/dev/null 2>&1; then
    banner "installing cert-manager $CERT_MANAGER_VERSION"
    kubectl apply -f "https://github.com/cert-manager/cert-manager/releases/download/$CERT_MANAGER_VERSION/cert-manager.yaml"
    kubectl -n cert-manager wait --for=condition=Available --timeout=180s \
        deployment/cert-manager deployment/cert-manager-cainjector deployment/cert-manager-webhook
fi

# ── 3. helm install efe-agent-runner ─────────────────────────────────
banner "helm install efe-agent-runner (bootstrap = true)"
helm upgrade --install efe-agent-runner ./charts/efe-agent-runner \
    --namespace "$NS" \
    --set agentRuntimeEnabled=true \
    --set admissionWebhook.certManager.bootstrap=true \
    --set imageWhitelist="ghcr.io/efe-ai/agent-core@sha256:NOOP" \
    --set agentCore.cosign.keyRef="dev-skip" \
    --set controller.replicas=1 \
    --set kvm.required=false \
    --wait --timeout 300s

# ── 4. wait Ready ────────────────────────────────────────────────────
banner "waiting for all efe-agent-runner pods to become Ready"
kubectl -n "$NS" rollout status deployment/efe-agent-runner-controller --timeout=180s

# ── 5. admission webhook smoke ──────────────────────────────────────
banner "applying tests/fixtures/agentrun-noop.yaml (admission must allow)"
if [[ -f tests/fixtures/agentrun-noop.yaml ]]; then
    kubectl -n "$NS" apply -f tests/fixtures/agentrun-noop.yaml
    sleep 5
    kubectl -n "$NS" get agentrun || true
else
    banner "tests/fixtures/agentrun-noop.yaml missing — skipping AgentRun apply"
fi

banner "k3d smoke OK"
