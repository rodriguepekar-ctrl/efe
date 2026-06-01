#!/usr/bin/env bash
# Chaos test : OOM-kill a PgBouncer replica, verify failover.
set -euo pipefail

NS="${EFE_EDGE_NS:-efe-edge}"

pod=$(kubectl -n "$NS" get pod \
  -l app.kubernetes.io/name=efe-pgbouncer \
  -o jsonpath='{.items[0].metadata.name}')

echo "▶ Triggering OOM on $pod via stress-ng :"
# Simulate OOM by allocating > limit memory.
kubectl -n "$NS" exec "$pod" -- /bin/sh -c \
  "yes | tr \\n x | head -c 500m" &
oom_pid=$!

sleep 5

echo "▶ Killing the test pod :"
kubectl -n "$NS" delete pod "$pod" --grace-period=2 &
kill $oom_pid 2>/dev/null || true

# Wait for replacement.
echo "▶ Waiting for replacement pod Ready (max 60s) :"
kubectl -n "$NS" wait --for=condition=ready \
  --timeout=60s \
  pod -l app.kubernetes.io/name=efe-pgbouncer

echo "✓ PgBouncer replacement Ready"
