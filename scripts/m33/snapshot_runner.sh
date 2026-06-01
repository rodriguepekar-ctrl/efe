#!/usr/bin/env bash
# M33 — Manual KG snapshot trigger.

set -euo pipefail
NS="${EFE_STACK_NS:-efe-system}"
ts=$(date +%s)
echo "▶ Triggering manual KG snapshot..."
kubectl -n "$NS" create job --from=cronjob/efe-stack-kg-snapshot \
    manual-kg-snapshot-$ts
kubectl -n "$NS" wait --for=condition=complete \
    job/manual-kg-snapshot-$ts --timeout=1800s
echo "✓ KG snapshot complete"
