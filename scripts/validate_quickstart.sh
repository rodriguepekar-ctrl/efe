#!/usr/bin/env bash
#
# scripts/validate_quickstart.sh — Sprint D10.4 DX validation.
#
# Runs the 6 quickstart steps end-to-end against a fresh helm
# install on a minikube/kind cluster, and ASSERTS that wallclock
# stays under the SLO budget D10-DX-1 (600 s = 10 min).
#
# Used by:
#   * 3 external reviewers during the D10 audit (manual run on
#     their dev machine, report ELAPSED).
#   * The bench-dx CI job (post-merge nightly on a kind cluster).
#
# Usage:
#   scripts/validate_quickstart.sh [--budget-seconds 600] [--no-cleanup]
#
# Pre-requisites:
#   * helm ≥ 3.13 on PATH
#   * kubectl ≥ 1.27 with a cluster context selected
#   * python3 ≥ 3.10
#   * Internet access to ghcr.io (image + chart pull) AND PyPI
#     (efe-tool wheel)
#
# Exit codes:
#   0 — quickstart succeeded within budget
#   1 — quickstart failed (any step)
#   2 — exceeded budget (steps OK but > N seconds)
#   3 — pre-flight failed (tool missing, no cluster context)

set -Eeuo pipefail

# ---------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------
BUDGET_S=600
CLEANUP=1
NAMESPACE="${EFE_QS_NAMESPACE:-efe-quickstart}"
CHART_VERSION="${EFE_QS_CHART_VERSION:-0.1.10}"
SDK_VERSION="${EFE_QS_SDK_VERSION:-0.1.10}"
PORT_FORWARD_PID=""

cleanup() {
  if [[ -n "$PORT_FORWARD_PID" ]]; then
    kill "$PORT_FORWARD_PID" 2>/dev/null || true
  fi
  if [[ "$CLEANUP" -eq 1 ]]; then
    helm uninstall efe -n "$NAMESPACE" 2>/dev/null || true
    kubectl delete namespace "$NAMESPACE" --ignore-not-found 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

# ---------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
  case "$1" in
    --budget-seconds)  BUDGET_S="$2"; shift 2 ;;
    --no-cleanup)      CLEANUP=0; shift ;;
    --chart-version)   CHART_VERSION="$2"; shift 2 ;;
    --sdk-version)     SDK_VERSION="$2"; shift 2 ;;
    -h|--help)         sed -n '3,30p' "$0"; exit 0 ;;
    *) echo "::error::unknown flag: $1" >&2; exit 3 ;;
  esac
done

# ---------------------------------------------------------------------
# Step 0 — preflight
# ---------------------------------------------------------------------
echo "[0/6] preflight"
T_START=$(date +%s)

for tool in helm kubectl python3; do
  if ! command -v "$tool" >/dev/null; then
    echo "::error::$tool not on PATH" >&2
    exit 3
  fi
done

if ! kubectl get nodes >/dev/null 2>&1; then
  echo "::error::no kubectl cluster context" >&2
  exit 3
fi

python3 -c "import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)" \
  || { echo "::error::need Python ≥ 3.10" >&2; exit 3; }

# ---------------------------------------------------------------------
# Step 1 — helm install
# ---------------------------------------------------------------------
echo "[1/6] helm install"
helm install efe oci://ghcr.io/efe-project/charts/efe \
  --version "$CHART_VERSION" \
  --namespace "$NAMESPACE" --create-namespace \
  --set replicaCount=2 \
  --set service.headless=true \
  --set autoscaling.minReplicas=2 \
  --wait --timeout=180s

# ---------------------------------------------------------------------
# Step 2 — wait deployment ready
# ---------------------------------------------------------------------
echo "[2/6] wait deployment available"
kubectl wait deploy/efe -n "$NAMESPACE" \
  --for=condition=available --timeout=120s

# ---------------------------------------------------------------------
# Step 3 — pip install SDK
# ---------------------------------------------------------------------
echo "[3/6] pip install efe-tool==$SDK_VERSION"
python3 -m pip install --quiet "efe-tool==$SDK_VERSION"

# ---------------------------------------------------------------------
# Step 4 — port-forward
# ---------------------------------------------------------------------
echo "[4/6] port-forward"
kubectl port-forward -n "$NAMESPACE" svc/efe 50051:50051 \
  > /tmp/efe-quickstart-pf.log 2>&1 &
PORT_FORWARD_PID=$!

# Wait for the port-forward to be reachable.
for _ in $(seq 1 30); do
  python3 -c "
import socket
s = socket.socket()
s.settimeout(1.0)
try:
    s.connect(('127.0.0.1', 50051))
    s.close()
    exit(0)
except OSError:
    exit(1)
" && break
  sleep 1
done

# ---------------------------------------------------------------------
# Step 5 — run example 01
# ---------------------------------------------------------------------
echo "[5/6] python examples/01_quickstart_classify.py"
QS_OUT="/tmp/efe-quickstart-out.txt"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXAMPLE="$SCRIPT_DIR/../examples/01_quickstart_classify.py"
if [[ -f "$EXAMPLE" ]]; then
  python3 "$EXAMPLE" | tee "$QS_OUT"
else
  echo "::error::example file not found: $EXAMPLE" >&2
  exit 1
fi

# ---------------------------------------------------------------------
# Step 6 — assert expected labels in output
# ---------------------------------------------------------------------
echo "[6/6] validate output"
errors=0
for label in fraud question bug billing; do
  if ! grep -q "$label" "$QS_OUT"; then
    echo "::error::expected label '$label' missing from output" >&2
    errors=$(( errors + 1 ))
  fi
done
if [[ "$errors" -gt 0 ]]; then
  echo "::error::quickstart output validation failed ($errors missing labels)" >&2
  exit 1
fi

# ---------------------------------------------------------------------
# Final
# ---------------------------------------------------------------------
ELAPSED=$(($(date +%s) - T_START))
echo ""
echo "====================================================="
echo "  ✅ quickstart validated in ${ELAPSED}s (budget: ${BUDGET_S}s)"
echo "====================================================="

# Emit a small JSON for CI ingestion (D10-DX-1).
mkdir -p target/dx
cat > target/dx/quickstart_result.json <<EOF
{
  "wallclock_s": $ELAPSED,
  "budget_s": $BUDGET_S,
  "passed": $([ "$ELAPSED" -le "$BUDGET_S" ] && echo true || echo false),
  "chart_version": "$CHART_VERSION",
  "sdk_version": "$SDK_VERSION",
  "namespace": "$NAMESPACE"
}
EOF

if [[ "$ELAPSED" -gt "$BUDGET_S" ]]; then
  echo "::warning::quickstart took ${ELAPSED}s, exceeds budget ${BUDGET_S}s"
  exit 2
fi
exit 0
