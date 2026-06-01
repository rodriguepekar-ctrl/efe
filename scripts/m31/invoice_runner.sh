#!/usr/bin/env bash
# M31 — Manual invoice generation trigger.
#
# Normally invoked by the monthly CronJob. Use this for re-runs
# (e.g. after a Stripe outage on the 1st of the month).

set -euo pipefail
NS="${EFE_STACK_NS:-efe-system}"

echo "▶ Triggering manual invoicer run..."
kubectl -n "$NS" create job --from=cronjob/efe-stack-stripe-invoicer \
    manual-invoicer-$(date +%s)

echo "▶ Watching job status..."
kubectl -n "$NS" wait --for=condition=complete \
    job/manual-invoicer-$(date +%s) --timeout=1800s
echo "✓ Invoicer complete"
