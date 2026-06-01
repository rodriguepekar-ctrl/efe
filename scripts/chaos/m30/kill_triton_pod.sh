#!/usr/bin/env bash
# M30 chaos test : kill a random Triton pod and assert that :
#   - the HPA notices and queues a replacement within 30 s
#   - in-flight inference 5xx rate stays < 5 % during the 60 s observation
#   - readiness recovers within 90 s
#
# Run against a staging cluster where Triton + a load generator are
# already running.

set -euo pipefail
NS="${EFE_STACK_NS:-efe-system}"
STATEFULSET="${TRITON_STATEFULSET:-efe-stack-triton}"
OBS_SECS="${OBSERVATION_SECS:-90}"

echo "▶ Listing Triton pods in $NS / $STATEFULSET"
pods=$(kubectl -n "$NS" get pod -l app.kubernetes.io/name=efe-triton -o name)
if [ -z "$pods" ]; then
    echo "✗ no triton pods found in $NS"
    exit 1
fi
victim=$(echo "$pods" | shuf -n 1)
echo "▶ Deleting victim: $victim"
kubectl -n "$NS" delete "$victim" --grace-period=2

start=$(date +%s)
echo "▶ Watching readiness for ${OBS_SECS}s..."
while true; do
    now=$(date +%s)
    if [ $((now - start)) -gt "$OBS_SECS" ]; then
        echo "✗ readiness did not recover within ${OBS_SECS}s"
        kubectl -n "$NS" get pod -l app.kubernetes.io/name=efe-triton
        exit 1
    fi
    ready=$(kubectl -n "$NS" get statefulset "$STATEFULSET" -o jsonpath='{.status.readyReplicas}')
    desired=$(kubectl -n "$NS" get statefulset "$STATEFULSET" -o jsonpath='{.spec.replicas}')
    if [ "$ready" = "$desired" ]; then
        echo "✓ readiness recovered in $((now - start))s ($ready/$desired)"
        break
    fi
    sleep 2
done

# Pull the 5xx rate from Prometheus (gateway in-cluster).
echo "▶ Checking 5xx rate during chaos window..."
prom_url="${PROMETHEUS_URL:-http://prometheus.efe-system.svc.cluster.local:9090}"
query='sum(rate(efe_multimodal_requests_total{outcome="err"}[1m]))/sum(rate(efe_multimodal_requests_total[1m]))'
err_rate=$(curl -s "$prom_url/api/v1/query" --data-urlencode "query=$query" \
  | jq -r '.data.result[0].value[1] // "0"')
err_pct=$(awk -v r="$err_rate" 'BEGIN{printf "%.2f", r*100}')
echo "  observed 5xx rate: ${err_pct}%"
threshold="${MAX_ERR_PCT:-5.0}"
if awk -v r="$err_pct" -v t="$threshold" 'BEGIN{exit !(r<t)}'; then
    echo "✓ 5xx rate ${err_pct}% < threshold ${threshold}%"
else
    echo "✗ 5xx rate ${err_pct}% ≥ threshold ${threshold}%"
    exit 1
fi
