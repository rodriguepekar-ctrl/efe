# `efe-bench-harness` Helm chart

PR #21 § 28 + AD44 — **optional** in-cluster orchestrator.

The default CI workflows (`bench-quick`, `bench-full`, `bench-release`)
run the harness from the **runner host**, port-forwarded to the
cluster. That path is faster and simpler.

This chart is the **escape hatch** for environments where `kubectl
port-forward` is blocked (managed K8s with private API endpoints,
restricted CI sandboxes). It packages the harness as a one-shot
`batch/v1` Job inside the cluster.

## Quick start

```bash
helm install bench-quick . \
    --namespace efe-bench --create-namespace \
    --set 'scenarios={S01_baseline_100qps_5min,S04_chaos_participant_loss}' \
    --set image.tag=0.1.0
```

Watch progress :

```bash
kubectl logs -n efe-bench -l app.kubernetes.io/instance=bench-quick -f
```

When the Job completes, copy results :

```bash
kubectl cp efe-bench/$(kubectl get pod -n efe-bench -l app.kubernetes.io/instance=bench-quick -o jsonpath='{.items[0].metadata.name}'):/var/run/bench/ ./results
```

## Security

| Knob | Default | Notes |
|---|---|---|
| `securityContext.runAsNonRoot` | `true` | T93 |
| `containerSecurityContext.readOnlyRootFilesystem` | `true` | T93 |
| `containerSecurityContext.allowPrivilegeEscalation` | `false` | T93 |
| `containerSecurityContext.capabilities.drop` | `[ALL]` | T93 |
| `rbac.namespaceScoped` | `true` (forced) | AD21 — `false` is rejected via `fail` |
| `mode=production` | rejected at template-time | AD11 / T86 |

## Limitations (Phase-2 stub)

The chart is intentionally minimal. Phase-3 follow-ups (Appendix G) :

- G.6 — push results to S3/GCS instead of PVC
- G.7 — multi-tenant : bench-as-a-service
- G.10 — Prometheus snapshot scraping inside the Job

## Values reference

See [`values.yaml`](values.yaml) — every knob is commented.

## Stability

This chart shares the harness STABILITY_CONTRACT v1.3 § 21 :
- `appVersion` follows the harness crate version (semver-major-stable for ≥ 6 chart-minor releases).
- Values keys are **stable** across patch releases ; renames require a major bump.
