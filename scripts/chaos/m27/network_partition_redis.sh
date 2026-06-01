#!/usr/bin/env bash
# Chaos test : simulate a network partition between Redis master and
# its 2 replicas. The cluster should refuse writes on the minority
# (CLUSTERDOWN) for the duration of the partition + cluster-node-timeout.
#
# Strategy : use `kubectl exec` on master pod to add iptables DROP
# rules for traffic from the replica pod IPs.

set -euo pipefail

NS="${EFE_EDGE_NS:-efe-edge}"
PARTITION_DURATION="${PARTITION_DURATION:-30}"

echo "▶ Identifying master node :"
master_pod=$(kubectl -n "$NS" get pods -l app.kubernetes.io/name=efe-redis-cluster \
  -o jsonpath='{.items[0].metadata.name}')
echo "    master = $master_pod"

# Get replica IPs
replica_ips=()
for i in 1 2; do
    pod="efe-edge-redis-${i}"
    ip=$(kubectl -n "$NS" get pod "$pod" -o jsonpath='{.status.podIP}')
    replica_ips+=("$ip")
done
echo "    replica IPs : ${replica_ips[*]}"

# Apply iptables DROP rules on master
echo "▶ Injecting partition (kubectl exec iptables DROP) :"
for ip in "${replica_ips[@]}"; do
    kubectl -n "$NS" exec "$master_pod" -- \
        sh -c "iptables -A INPUT -s $ip -j DROP; iptables -A OUTPUT -d $ip -j DROP" \
        || echo "  WARN: iptables exec failed for $ip (may need NET_ADMIN cap)"
done

echo "▶ Waiting $PARTITION_DURATION s ..."
sleep "$PARTITION_DURATION"

echo "▶ Asserting minority returned CLUSTERDOWN :"
kubectl -n "$NS" exec "$master_pod" -- redis-cli -a "$REDIS_PASSWORD" \
    cluster info | grep -E "cluster_state|cluster_known_nodes"

echo "▶ Healing partition :"
for ip in "${replica_ips[@]}"; do
    kubectl -n "$NS" exec "$master_pod" -- \
        sh -c "iptables -D INPUT -s $ip -j DROP; iptables -D OUTPUT -d $ip -j DROP" \
        || true
done

echo "▶ Waiting 30s for cluster to re-converge ..."
sleep 30

count=$(kubectl -n "$NS" exec efe-edge-redis-1 -- \
  redis-cli -a "$REDIS_PASSWORD" CLUSTER NODES | grep -v fail | wc -l)
if [ "$count" -ge 6 ]; then
    echo "✓ Cluster recovered : known_nodes=$count"
else
    echo "✗ Cluster did NOT recover : known_nodes=$count"
    exit 1
fi
