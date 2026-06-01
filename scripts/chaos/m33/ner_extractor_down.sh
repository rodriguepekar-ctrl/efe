#!/usr/bin/env bash
# M33 chaos test : kill the NER extractor pod and assert that
# `/v1/kg/extract` fails gracefully (5xx with retry headers) and
# that existing graph queries still work (NER is only on the
# extract path).

set -euo pipefail
NS="${EFE_STACK_NS:-efe-system}"

echo "▶ Listing NER extractor pods..."
pods=$(kubectl -n "$NS" get pod -l app.kubernetes.io/name=efe-kg,component=ner -o name)
if [ -z "$pods" ]; then
    echo "✗ no NER pods found"
    exit 1
fi
victim=$(echo "$pods" | head -1)
echo "▶ Killing $victim"
kubectl -n "$NS" delete "$victim" --grace-period=2

start=$(date +%s)
echo "▶ Verifying graph queries still work during outage..."
TOKEN="${EFE_TEST_BEARER:-test-bearer}"
status=$(kubectl -n "$NS" exec deploy/efe-stack-server -- \
    curl -s -o /dev/null -w "%{http_code}" \
    -H "Authorization: Bearer $TOKEN" \
    http://localhost:8080/v1/kg/graph/status)
if [ "$status" = "200" ]; then
    echo "✓ /v1/kg/graph/status = 200 during NER outage"
else
    echo "✗ /v1/kg/graph/status = $status"
    exit 1
fi

echo "▶ Waiting for NER recovery..."
kubectl -n "$NS" wait --for=condition=Available \
    deploy/efe-stack-kg-ner --timeout=120s
echo "✓ NER recovered in $(($(date +%s) - start))s"
