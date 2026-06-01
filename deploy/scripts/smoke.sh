#!/usr/bin/env bash
# Post-deploy smoke test — exercises health + classify + list.
# Exit code 0 = pass, 1 = fail.
#
# Invocation:
#   bash deploy/scripts/smoke.sh --env=staging

set -Eeuo pipefail

ENV="${1:-staging}"
case "$ENV" in
  --env=staging) ENDPOINT="efe-staging.internal";;
  --env=prod)    ENDPOINT="efe.internal";;
  *)             ENDPOINT="${EFE_ENDPOINT:-localhost:50051}";;
esac

TOKEN="${EFE_JWT:?export EFE_JWT first}"
TENANT="${EFE_TENANT:-qa-team}"
CATALOG="${EFE_CATALOG:-support_triage}"

log()  { printf "\033[1;34m[+]\033[0m %s\n" "$*"; }
fail() { printf "\033[1;31m[x]\033[0m %s\n" "$*"; exit 1; }

log "Target: $ENDPOINT"
log "Tenant: $TENANT, catalog: $CATALOG"

# 1. Health probe
log "checking health (/healthz)"
RES=$(curl -sS --max-time 5 -H "Authorization: Bearer $TOKEN" \
       "https://${ENDPOINT}/healthz" || echo '{}')
test "$(echo "$RES" | jq -r .status)" = "SERVING" || fail "health $RES"

# 2. Classify happy path
log "classify ...'test message'"
RES=$(curl -sS --max-time 10 \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d "{\"text\":\"test smoke\",\"catalog\":\"$CATALOG\",\"tenant_id\":\"$TENANT\",\"request_id\":\"smoke-$RANDOM\"}" \
  "https://${ENDPOINT}/v1/classify")
LABEL=$(echo "$RES" | jq -r .label // empty)
[[ -n "$LABEL" && "$LABEL" != null ]] || fail "classify $RES"
log "  → label=$LABEL confidence=$(echo "$RES" | jq -r .confidence)"

# 3. Deny test (unknown catalog)
log "classify with unknown catalog (should 400/403)"
CODE=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 5 \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d "{\"text\":\"x\",\"catalog\":\"_no_such_catalog\",\"tenant_id\":\"$TENANT\"}" \
  "https://${ENDPOINT}/v1/classify")
[[ "$CODE" =~ ^(4|5) ]] || fail "unknown catalog: expected 4xx/5xx, got $CODE"

log "All smoke checks passed ✓"
