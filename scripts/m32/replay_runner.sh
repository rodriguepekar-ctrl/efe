#!/usr/bin/env bash
# M32 — Trigger an index replay from the latest snapshot.

set -euo pipefail
NS="${EFE_STACK_NS:-efe-system}"
COLLECTION_ID="${COLLECTION_ID:?must set COLLECTION_ID}"
ts=$(date +%s)
echo "▶ Triggering manual replay for collection $COLLECTION_ID..."
kubectl -n "$NS" create job --from=job/efe-stack-vector-replay-template \
    manual-replay-$ts -- --collection-id "$COLLECTION_ID"
kubectl -n "$NS" wait --for=condition=complete \
    job/manual-replay-$ts --timeout=600s
echo "✓ Replay complete"
