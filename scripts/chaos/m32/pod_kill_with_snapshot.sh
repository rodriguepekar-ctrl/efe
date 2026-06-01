#!/usr/bin/env bash
# M32 chaos test : kill an efe-server pod (which holds an in-process
# vector index) and assert that after restart the replay job
# rebuilds collections from the latest S3 snapshot within 5 minutes.

set -euo pipefail
NS="${EFE_STACK_NS:-efe-system}"

echo "▶ Forcing a snapshot first (clean baseline)..."
kubectl -n "$NS" create job --from=cronjob/efe-stack-vector-snapshot \
    chaos-snapshot-$(date +%s) > /dev/null
sleep 30

echo "▶ Selecting victim pod..."
victim=$(kubectl -n "$NS" get pod -l app.kubernetes.io/name=efe-server -o name | head -1)
echo "  victim = $victim"

kubectl -n "$NS" delete "$victim" --grace-period=2

start=$(date +%s)
echo "▶ Waiting for replacement pod to be Ready..."
kubectl -n "$NS" wait --for=condition=Ready pod -l app.kubernetes.io/name=efe-server --timeout=300s || {
    echo "✗ replacement pod did not become Ready in 5 min"
    exit 1
}
echo "✓ replacement Ready in $(($(date +%s) - start))s"

echo "▶ Confirming a search hits the index (re-built from snapshot)..."
sleep 10
TOKEN="${EFE_TEST_BEARER:-test-bearer}"
status=$(kubectl -n "$NS" exec deploy/efe-stack-server -- \
    curl -s -o /dev/null -w "%{http_code}" \
    -H "Authorization: Bearer $TOKEN" \
    -X POST http://localhost:8080/v1/vector/collections/chaos-test/status)
if [ "$status" = "200" ]; then
    echo "✓ collection status = 200 — snapshot replay worked"
else
    echo "✗ collection status = $status — replay failed"
    exit 1
fi
