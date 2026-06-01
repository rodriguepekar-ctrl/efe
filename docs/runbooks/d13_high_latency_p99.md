# d13_high_latency_p99 — Runbook : p99 latency > 1s per tenant

> **Severity** : critical.
> **Alert** : `D13ObsHighLatencyP99`.
> **Owner** : SRE on-call.
> **Related dashboards** : `01_operator_overview`,
> `02_tenant_drill_down`.

## 1. Alert summary

p99 gRPC latency for tenant `$tenant` exceeded 1 s for 10 min.

## 2. First 5 minutes — triage

1. In dashboard 01, click the slow bucket on the
   "Latency heatmap" panel.
2. Grafana shows 1-3 trace exemplars (red dots) — click any.
3. In the Tempo waterfall, identify the dominant span :
   `efe.embedder.encode` ? `efe.cone.fit` ?
   `efe.fusion.round` ? `efe.catalog.get` ?
4. Cross-check with dashboard 02 latency p50/p95/p99 by RPC —
   is one specific RPC slow, or all of them ?
5. Check if the slow path correlates with a recent deploy :
   ```bash
   kubectl rollout history deploy/efe-server -n tenant-acme
   ```

## 3. Common root causes

| Cause | Symptom | Fix |
|-------|---------|-----|
| Embedder cold cache | `efe.embedder.encode` ≈ 500 ms | Pre-warm via classify ping at boot |
| Cone reload contention | Latency spike right after register/evolve | Cf. d12 catalog architecture § 7 |
| BLAS thread oversubscription | All ops slower across the board | Set `OMP_NUM_THREADS` per pod |
| Embedder GC pause | Periodic spikes | Enable jemalloc allocator |
| Disk pressure | Slow audit log flush | Check `kubectl describe pvc` |

## 4. Mitigation (immediate)

- Scale up replicas : `kubectl scale deploy/efe-server --replicas=+50%`.
- If a specific RPC is slow, increase its timeout / budget
  in the tenant's Helm values.
- If correlated with deploy : `kubectl rollout undo deploy/efe-server`.

## 5. Resolution (durable)

- Add a perf regression test covering the slow path.
- File a profiling ticket if root cause is unclear.

## 6. Escalation

T+30 min : page tier-2.

## 7. Postmortem

Use template.
