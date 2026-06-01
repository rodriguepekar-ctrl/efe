#!/usr/bin/env bash
# M31 — Manual rollup trigger (debugging tool).
#
# The hour + day rollups normally run via Helm CronJobs. This script
# lets an operator manually trigger one (useful when the CronJob
# was paused or skipped).

set -euo pipefail
PERIOD="${1:-hour}"
NS="${EFE_STACK_NS:-efe-system}"

case "$PERIOD" in
    hour|day) ;;
    *) echo "usage: $0 [hour|day]" ; exit 1 ;;
esac

echo "▶ Triggering manual rollup ($PERIOD)..."
kubectl -n "$NS" create job --from=cronjob/efe-stack-stripe-rollup-$PERIOD \
    manual-rollup-$PERIOD-$(date +%s)

echo "▶ Watching job status..."
kubectl -n "$NS" wait --for=condition=complete \
    job/manual-rollup-$PERIOD-$(date +%s) --timeout=600s
echo "✓ Rollup complete"
