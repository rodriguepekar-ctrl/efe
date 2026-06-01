# RB_M28_NATS_FULL — NATS JetStream disk pressure

**Severity** : P1 (> 80% disk) | P0 (cluster rejects writes).

## 1. Confirm
```bash
kubectl -n efe-stack exec -ti efe-stack-nats-0 -- \
  nats stream report --all
```

Look for `Storage` column near limits.

## 2. Identify the largest stream
```bash
kubectl -n efe-stack exec -ti efe-stack-nats-0 -- \
  nats stream ls --json | jq '.streams | sort_by(-.state.bytes)[:5]'
```

Per-tenant streams `EVENTS_TENANT_<slug>` are usually the culprit.

## 3. Mitigate

### 3.1 Manual purge (oldest 50% of a stream)
```bash
kubectl exec -ti efe-stack-nats-0 -- \
  nats stream purge EVENTS_TENANT_acme --seq <half>
```

### 3.2 Tighten retention
```bash
kubectl exec -ti efe-stack-nats-0 -- \
  nats stream edit EVENTS_TENANT_acme --max-age=72h
```

### 3.3 Scale PVC up
```bash
kubectl -n efe-stack patch pvc data-efe-stack-nats-0 \
  -p '{"spec":{"resources":{"requests":{"storage":"30Gi"}}}}'
# Repeat for nats-1, nats-2 ; PVC must support expansion.
```

## 4. Long-term
- Add CDC publish filtering (drop noisy topics from publication).
- Move per-tenant streams to per-tenant NATS account (Synadia
  Cloud) for multi-region cost-sharing.

## 5. Post-incident
- Run `M28NatsDiskHigh` alert dry-run to confirm budget.
- Update SLO `M28-5` if retention reduced.
