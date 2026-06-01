#!/usr/bin/env bash
# M32 — Manual snapshot trigger (debugging tool).

set -euo pipefail
NS="${EFE_STACK_NS:-efe-system}"
ts=$(date +%s)
echo "▶ Triggering manual snapshot..."
kubectl -n "$NS" create job --from=cronjob/efe-stack-vector-snapshot \
    manual-snapshot-$ts
kubectl -n "$NS" wait --for=condition=complete \
    job/manual-snapshot-$ts --timeout=1800s
echo "✓ Snapshot complete"
