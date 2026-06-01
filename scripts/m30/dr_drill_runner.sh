#!/usr/bin/env bash
# M30 — Run the quarterly DR drill and upload the report to the
# SOC 2 evidence vault. Operator-only — call from a runbook.

set -euo pipefail
API_URL="${EFE_API_URL:?must set EFE_API_URL}"
TOKEN="${EFE_OPERATOR_TOKEN:?must set EFE_OPERATOR_TOKEN}"
S3_BUCKET="${EVIDENCE_S3_BUCKET:?must set EVIDENCE_S3_BUCKET}"
INITIAL="${EFE_INITIAL_ACTIVE:-eu-west-1}"
TARGET="${EFE_FAILOVER_TARGET:-eu-west-3}"

ts=$(date -u +"%Y%m%dT%H%M%SZ")
echo "▶ Starting DR drill at $ts ($INITIAL → $TARGET)"
resp=$(curl -fsS -X POST "$API_URL/v1/region/drill" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d "$(cat <<EOF
{"initial_active":"$INITIAL","failover_target":"$TARGET","force":false,"total_timeout_secs":300}
EOF
)")
echo "$resp" | jq .

verdict=$(echo "$resp" | jq -r '.verdict')
drill_id=$(echo "$resp" | jq -r '.drill_id')

local_path="/tmp/dr_drill_${drill_id}.json"
echo "$resp" > "$local_path"
echo "▶ Uploading report to s3://${S3_BUCKET}/dr_drill/${ts}_${verdict}.json"
aws s3 cp "$local_path" "s3://${S3_BUCKET}/dr_drill/${ts}_${verdict}.json" \
    --acl bucket-owner-full-control \
    --metadata "drill_id=$drill_id,verdict=$verdict,operator=$USER"

if [ "$verdict" != "pass" ]; then
    echo "✗ Drill verdict = $verdict — open P2 ticket and review report"
    exit 1
fi
echo "✓ Drill verdict = pass — evidence archived"
