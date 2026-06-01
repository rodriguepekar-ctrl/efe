# EFE — Production deployment runbook

> **Audience** : SRE/platform engineer responsible for shepherding
> EFE from "first install" to "carry production traffic".
> **Sprint D9 deliverable**.

---

## 0. Pre-deployment checklist

- [ ] Cluster k8s version ≥ 1.27.
- [ ] Prometheus Operator installed (for ServiceMonitor + PrometheusRule).
- [ ] Network egress to `ghcr.io` (image pull) and your IDP (JWT issuer).
- [ ] DNS subdomain available for the gRPC endpoint
      (e.g. `efe.your-corp.io`).
- [ ] An external secret store (AWS Secrets Manager, GCP Secret Manager,
      HashiCorp Vault, External Secrets Operator) — DO NOT inline JWT
      keys in Helm values (cf. § 4 below).
- [ ] You have read [`d9_docker_helm_load_architecture.md`](../bench/d9_docker_helm_load_architecture.md)
      and the deep-analysis supplement.

---

## 1. Install the chart

```bash
helm install efe-prod oci://ghcr.io/efe-project/charts/efe \
  --version 0.1.9 \
  --namespace efe \
  --create-namespace \
  --values values.prod.yaml
```

Where `values.prod.yaml` looks like (copy from `deploy/helm/efe/values.prod.yaml`
in this repo):

```yaml
# Image — pin by digest for production.
image:
  repository: ghcr.io/efe-project/efe
  digest: "sha256:<paste-from-release-notes>"

# Scale.
replicaCount: 3
autoscaling:
  enabled: true
  minReplicas: 3       # stays > PDB minAvailable
  maxReplicas: 20
  targetCPUUtilizationPercentage: 70

pdb:
  enabled: true
  minAvailable: 2

# Resources (cf. supplement §3 — Little's law sizing).
resources:
  requests:
    cpu: 500m
    memory: 512Mi
  limits:
    cpu: "2"
    memory: 2Gi

# Service — headless ⇒ client-side gRPC LB across replicas.
service:
  type: ClusterIP
  headless: true
  grpcPort: 50051
  metricsPort: 9090

# Auth.
config:
  auth:
    enabled: true
    jwtAudience: "efe.v1"
    jwtIssuer: "https://idp.your-corp.io/"
    secretsFile: "/etc/efe/secrets/hs256"

# Use external-secrets ; never inline keys.
secrets:
  externalSecretName: efe-jwt-prod

# Observability.
serviceMonitor:
  enabled: true
prometheusRule:
  enabled: true
networkPolicy:
  enabled: true

# Tenant label (D9.A9 hook for D11).
tenant:
  id: "production-pilot"
```

---

## 2. Verify install

```bash
# Pods up
kubectl get pods -n efe -l app.kubernetes.io/name=efe

# Smoke test (grpc-health probe)
helm test efe-prod -n efe

# Metrics endpoint
kubectl port-forward -n efe svc/efe-prod 9090:9090 &
curl -s localhost:9090/metrics | head -20
```

The `up{job="efe-prod"}` metric should appear in Prometheus within
30 s of the chart applying (ServiceMonitor scrape interval = 15 s).

---

## 3. Image pinning policy

In **prod**, always pin by **digest**, never by tag:

```yaml
image:
  digest: "sha256:abc123..."   # immune to tag re-mapping
```

A tag (`v0.1.9`) can be re-pushed on top of a different image —
which is a supply-chain attack vector. The digest is content-addressed
and immutable.

**To update:** consult the GitHub Release notes ; copy the digest
from the cosign-attested entry; update `image.digest` ; `helm upgrade`.

---

## 4. JWT key handling

EFE accepts HS256 JWTs. The signing keys MUST come from an external
store — never the Helm values. Three patterns supported:

| Pattern | When | Mechanism |
|---------|------|-----------|
| `existingSecretName` | You manage Secrets via GitOps (Sealed-Secrets, SOPS) | Pre-create the Secret ; reference its name |
| `externalSecretName` | You use [External Secrets Operator](https://external-secrets.io) | ESO syncs ; chart references the resulting Secret |
| `inline.enabled` (DEV ONLY) | Local minikube ; **never prod** | Helm-managed Secret |

The Secret object MUST contain a key file at `/etc/efe/secrets/hs256`
with one HS256 secret per line. Multi-key support means you can
**rotate without downtime** — see [`key_rotation.md`](key_rotation.md).

---

## 5. Observability

### 5.1 Metrics

Helm chart includes a `ServiceMonitor` that prometheus-operator
discovers automatically. Sample query expressions:

```promql
# Request rate per replica
sum by (pod) (rate(grpc_server_handled_total{job="efe-prod"}[5m]))

# p99 latency
histogram_quantile(0.99,
  sum by (le) (rate(grpc_server_handling_seconds_bucket{job="efe-prod"}[5m])))

# Memory
process_resident_memory_bytes{job="efe-prod"}
```

### 5.2 Logs

EFE logs JSON to stdout. Aggregate via your usual sink (Loki,
Cloud Logging, Datadog) by labelling on `app.kubernetes.io/name=efe`.

Sensitive fields (raw_prompt) are NEVER logged; the audit logger
redacts them per the D8 PII pipeline (`pii_check.yml` enforces).

### 5.3 Alerts

The `PrometheusRule` ships these alerts:

| Alert | Threshold | Action |
|-------|-----------|--------|
| `EfeHighErrorRate` | error rate > 1 % for 5 min | page |
| `EfeHighP99Latency` | p99 > 5 s for 5 min | page |
| `EfeRSSGrowth` | RSS slope > 870 B/s sustained | ticket |
| `EfeReplicasBelowMinAvailable` | available < PDB.minAvailable | page |
| `EfeAuthFailureSpike` | 401s × 10 over baseline | investigate |

Tune thresholds in `values.prod.yaml` if your traffic profile differs.

---

## 6. Capacity planning

Reference: deep-analysis supplement §3.

```
λ_max_per_replica ≈ 1.6 RPS    (sustained, 4 concurrent clients)
                   6.4 RPS    (stress, 16 concurrent clients)

K_min = ceil(target_rps / 1.6)
```

| Target RPS | minReplicas | Notes |
|----------:|-----------:|-------|
| 1-2 | 2 | floor for PDB minAvailable |
| 5 | 4 | pilot 0.1 |
| 10 | 7 | typical SaaS partner |
| 50 | 32 | Phase 2 multi-tenant |

If you see CPU plateau but latency rises, consider switching from
CPU-based HPA to **custom metrics** (`grpc_server_active_requests`).
This requires Prometheus Adapter — see annexes of supplement §3.5.

---

## 7. Upgrade

```bash
# 1. Test the new chart version on a separate namespace first.
helm install efe-canary oci://ghcr.io/efe-project/charts/efe \
  --version 0.1.10 \
  --namespace efe-canary \
  --create-namespace \
  --values values.prod.yaml

# 2. Run helm test + send N canary requests.
helm test efe-canary -n efe-canary

# 3. If green: roll the prod release.
helm upgrade efe-prod oci://ghcr.io/efe-project/charts/efe \
  --version 0.1.10 \
  --namespace efe \
  --values values.prod.yaml

# 4. Watch the rollout.
kubectl rollout status -n efe deploy/efe-prod
```

The Deployment uses RollingUpdate with `maxUnavailable: 0` and
PDB.minAvailable=2 — every new pod has to pass startup + readiness
before an old pod is removed. **No request is dropped.**

---

## 8. Rollback

```bash
helm rollback efe-prod 1 -n efe
```

(`1` = previous revision ; `helm history efe-prod -n efe` lists them.)

If the rollback Pod fails to start, the previous revision's Pods
remain serving — Kubernetes only removes them when the new
generation is healthy.

---

## 9. Decommission

```bash
helm uninstall efe-prod -n efe
kubectl delete namespace efe   # if EFE is the only tenant
```

The chart has no PersistentVolumeClaims, no hanging finalizers, no
external resources to clean up. Uninstall removes everything.

---

## 10. References

* Architecture spec: [`docs/bench/d9_docker_helm_load_architecture.md`](../bench/d9_docker_helm_load_architecture.md)
* Deep analysis (math + decisions): [`docs/bench/d9_deep_analysis_supplement.md`](../bench/d9_deep_analysis_supplement.md)
* SLO list (canonical): [`BENCHMARKS.md` § 11](../../BENCHMARKS.md)
* Helm chart README: [`deploy/helm/efe/README.md`](../../deploy/helm/efe/README.md)
* Source: [`crates/efe-server/`](../../crates/efe-server/)
