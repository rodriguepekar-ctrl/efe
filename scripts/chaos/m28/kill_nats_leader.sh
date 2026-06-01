#!/usr/bin/env bash
# Chaos test : kill the current NATS Raft leader, expect re-election < 10s.

set -euo pipefail
NS="${EFE_STACK_NS:-efe-stack}"

leader=$(kubectl -n "$NS" exec efe-stack-nats-0 -- \
  nats server info -j 2>/dev/null \
  | jq -r '.jetstream.meta.leader // "efe-stack-nats-0"')
echo "▶ Current Raft leader : $leader"
kubectl -n "$NS" delete pod "$leader" --grace-period=2

start=$(date +%s)
while true; do
    now=$(date +%s)
    if [ $((now - start)) -gt 30 ]; then
        echo "✗ Re-election did not complete within 30s"
        exit 1
    fi
    new=$(kubectl -n "$NS" exec efe-stack-nats-1 -- nats server info -j 2>/dev/null \
      | jq -r '.jetstream.meta.leader' 2>/dev/null || true)
    if [ -n "$new" ] && [ "$new" != "$leader" ]; then
        echo "✓ New leader elected in $((now - start))s : $new"
        exit 0
    fi
    sleep 1
done
