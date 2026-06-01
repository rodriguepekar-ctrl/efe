# RB_M27_REDIS_FAIL — Redis Cluster failure

**Severity** : P1 (single node) — P0 (cluster down).
**Owner** : SRE on-call.
**Triggers** :
- `M27RedisClusterUnhealthy` (known_nodes != 6).
- `M27RedisClusterDown` (cluster_state != ok).

## 1. Confirm

```bash
kubectl -n efe-edge get pods -l app.kubernetes.io/name=efe-redis-cluster
kubectl -n efe-edge exec -ti efe-edge-redis-0 -- \
  redis-cli -a "$REDIS_PASSWORD" CLUSTER NODES
```

Look for : `disconnected`, `fail?`, `fail` flags.

## 2. Single node fail (replica)

Usually self-heals : pod restart picks up the cluster state from
`/data/nodes.conf`. Wait 1 min, re-check.

```bash
kubectl -n efe-edge delete pod efe-edge-redis-X --grace-period=10
```

## 3. Single node fail (master)

Replica should be auto-promoted within `cluster-node-timeout = 15s`.
Verify via `CLUSTER NODES` : look for new `master` role assignment.

If promotion didn't happen :

```bash
kubectl -n efe-edge exec -ti <surviving-replica-pod> -- \
  redis-cli -a "$REDIS_PASSWORD" CLUSTER FAILOVER FORCE
```

## 4. Multi-node failure

If 2+ master nodes are down, the cluster is in CLUSTERDOWN until quorum
is re-established. The cache becomes unavailable but the app keeps
working (fail-open in `efe-cache`).

Recovery :
```bash
# Re-attach the surviving + new pods to the cluster.
kubectl -n efe-edge rollout restart statefulset efe-edge-redis
# Wait for all 6 pods Ready :
kubectl -n efe-edge wait --for=condition=ready --timeout=180s \
  pod -l app.kubernetes.io/name=efe-redis-cluster
# Reconcile cluster state (if /data is intact, cluster heals
# automatically) :
kubectl -n efe-edge exec efe-edge-redis-0 -- \
  redis-cli -a "$REDIS_PASSWORD" CLUSTER NODES
```

## 5. Total cluster loss (PV destroyed)

Last resort. Re-init from scratch :

```bash
# 1. Disable cache from efe-server pods (no-cache mode).
kubectl -n efe-system set env deployment/efe-server EFE_CACHE_DISABLED=true

# 2. Delete StatefulSet + PVCs.
kubectl -n efe-edge delete statefulset efe-edge-redis
kubectl -n efe-edge delete pvc -l app.kubernetes.io/name=efe-redis-cluster

# 3. Re-install :
helm upgrade efe-stack ./charts/efe-stack \
  --set efe-edge.enabled=true --reuse-values

# 4. Wait for cluster init job (out-of-scope template — manual `CLUSTER MEET` if needed).

# 5. Re-enable cache :
kubectl -n efe-system set env deployment/efe-server EFE_CACHE_DISABLED-
```

**Data loss** : embedding cache + LLM cache repopulate from misses
(expected ~30 min to warm up). Quota counters reset (acceptable for the
current minute / day, no billing impact since aggregator reads from
already-persisted `tenant_daily_usage`).

## 6. Emergency disable

```bash
# Force efe-server to skip cache entirely.
kubectl -n efe-system set env deployment/efe-server EFE_CACHE_DISABLED=true
```

`efe-cache` is fail-open : this flag forces every GET to return None
immediately without contacting Redis.

## 7. Post-incident

- Verify all 6 pods healthy : `CLUSTER NODES`.
- Run cache warmup load test (see `scripts/load/m27/cache_warmup_hit_ratio.js`).
- File incident report.
- If recurring : review `redis.conf` parameters (`cluster-node-timeout`,
  `maxmemory`, persistence settings).
