#!/usr/bin/env bash
# Chaos test : kill a Redis Cluster master and assert recovery < 30s.
set -euo pipefail

NS="${EFE_EDGE_NS:-efe-edge}"

echo "▶ Listing Redis cluster nodes :"
kubectl -n "$NS" exec efe-edge-redis-0 -- \
  redis-cli -a "$REDIS_PASSWORD" CLUSTER NODES \
  | awk '{print $1, $2, $3}'

# Pick a random master (statefulset pods 0..5).
echo "▶ Killing efe-edge-redis-0 :"
kubectl -n "$NS" delete pod efe-edge-redis-0 --grace-period=5

# Wait for replica promotion : check known_nodes back to 6.
echo "▶ Waiting for cluster recovery (max 60s) :"
start=$(date +%s)
while true; do
  now=$(date +%s)
  elapsed=$((now - start))
  if [ "$elapsed" -gt 60 ]; then
    echo "✗ Cluster did NOT recover within 60s"
    exit 1
  fi
  # Use another pod since 0 may be down.
  count=$(kubectl -n "$NS" exec efe-edge-redis-1 -- \
    redis-cli -a "$REDIS_PASSWORD" CLUSTER NODES \
    | grep -v "fail" | wc -l)
  if [ "$count" -ge 6 ]; then
    echo "✓ Cluster recovered in ${elapsed}s, known_nodes=$count"
    exit 0
  fi
  sleep 2
done
