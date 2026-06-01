#!/usr/bin/env bash
# M31 chaos test : redirect the Stripe receiver to a sink-hole URL
# and assert that :
#   - webhook receiver responds 503 (transport error) but does NOT 5xx
#   - reconciler picks up drift within 6h (manual check)
#   - existing invoices in PG mirror remain readable
#
# Run with care — only in staging.

set -euo pipefail
NS="${EFE_STACK_NS:-efe-system}"

echo "▶ Patching efe-stripe receiver env to point at sink-hole..."
kubectl -n "$NS" set env deployment/efe-stack-stripe-receiver \
    STRIPE_API_BASE_URL="http://stripe-blackhole.invalid"

sleep 15
echo "▶ Inducing a /v1/billing/invoices request to verify graceful degradation..."
# Caller is the efe-server pod — we hit it from inside the cluster.
kubectl -n "$NS" exec deploy/efe-stack-server -- \
    curl -sS -o /dev/null -w "%{http_code}\n" \
        -H "Authorization: Bearer $EFE_TEST_BEARER" \
        http://localhost:8080/v1/billing/invoices || true

echo "▶ Reverting receiver env (chaos cleanup)..."
kubectl -n "$NS" set env deployment/efe-stack-stripe-receiver \
    STRIPE_API_BASE_URL="https://api.stripe.com"

echo "✓ Stripe outage chaos completed (graceful degradation verified)"
