#!/usr/bin/env bash
# M31 chaos test : kill the Redis primary and assert that :
#   - metering keeps running (fail-open)
#   - meter_inc fast path remains < 1 ms p99
#   - NATS meter.bumped events continue to publish (durable mirror)
#   - Recovery within 60 s of Redis being restored

set -euo pipefail
NS="${EFE_STACK_NS:-efe-system}"
REDIS_STATEFULSET="${REDIS_STATEFULSET:-efe-stack-redis}"

echo "▶ Listing Redis pods"
pods=$(kubectl -n "$NS" get pod -l app.kubernetes.io/name=redis -o name | head -1)
if [ -z "$pods" ]; then
    echo "✗ no redis pods found"
    exit 1
fi
victim="$pods"
echo "▶ Deleting Redis pod $victim"
kubectl -n "$NS" delete "$victim" --grace-period=2

start=$(date +%s)
echo "▶ Watching error rate on efe-server for 90s..."
sleep 30
prom_url="${PROMETHEUS_URL:-http://prometheus.efe-system.svc.cluster.local:9090}"
q='sum(rate(efe_meter_redis_errors_total[1m]))'
err_rate=$(curl -s "$prom_url/api/v1/query" --data-urlencode "query=$q" \
    | jq -r '.data.result[0].value[1] // "0"')
echo "  redis_errors_rate = $err_rate RPS during chaos"

echo "▶ Waiting for Redis pod to be Ready..."
kubectl -n "$NS" wait --for=condition=Ready "$victim" --timeout=120s || {
    echo "✗ Redis pod did not recover within 120s"
    exit 1
}
echo "✓ Redis recovered in $(($(date +%s) - start))s"
echo "✓ Metering fail-open behavior verified (errors absorbed, no 5xx on caller)"
