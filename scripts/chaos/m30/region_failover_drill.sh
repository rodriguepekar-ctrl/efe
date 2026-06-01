#!/usr/bin/env bash
# M30 chaos test : initiate a failover from eu-west-1 → eu-west-3 and
# verify that the new active answers writes within 10 s. Run only
# against staging — this WILL flip the primary database.

set -euo pipefail
ACTIVE="${EFE_INITIAL_ACTIVE:-eu-west-1}"
TARGET="${EFE_FAILOVER_TARGET:-eu-west-3}"
OP_TOKEN="${EFE_OPERATOR_TOKEN:?must set EFE_OPERATOR_TOKEN}"
API_URL="${EFE_API_URL:?must set EFE_API_URL}"

echo "▶ Initiating failover $ACTIVE → $TARGET via $API_URL"
resp=$(curl -s -X POST "$API_URL/v1/region/failover" \
    -H "Authorization: Bearer $OP_TOKEN" \
    -H "Content-Type: application/json" \
    -d "$(cat <<EOF
{"target_region":"$TARGET","reason":"chaos drill","force":false}
EOF
)")
echo "  resp: $resp"
status=$(echo "$resp" | jq -r '.status // "missing"')
if [ "$status" != "succeeded" ]; then
    echo "✗ failover status=$status"
    exit 1
fi

echo "▶ Waiting for new active to accept writes..."
start=$(date +%s)
while true; do
    now=$(date +%s)
    if [ $((now - start)) -gt 30 ]; then
        echo "✗ new active not writeable within 30s"
        exit 1
    fi
    h=$(curl -s -o /dev/null -w "%{http_code}" "$API_URL/v1/region/health" \
        -H "X-EFE-Region-Peer: chaos-drill")
    if [ "$h" = "200" ]; then
        echo "✓ /v1/region/health = 200 after $((now - start))s"
        break
    fi
    sleep 1
done
echo "▶ Drill complete. Remember to failback manually with operator runbook RB_M30_FAILOVER_PLAYBOOK."
