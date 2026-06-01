# 04 — Production deployment : from playground to K8s

> **Time** : 30 minutes (with cluster) or 5 minutes (read-through)
> **Pre-requisite** : tutorial 03 completed + a Kubernetes cluster

## Goal

Move from the playground to a real production EFE 1.0 GA deployment via the
Helm chart M12.

## Differences : playground vs production

| Concern | Playground | Production EFE 1.0 GA |
|---|---|---|
| Backend | Flask in-memory | gRPC service in Rust (`crates/efe-server/`) |
| Storage | dict | Postgres (D14) + connection pool |
| Auth | mock OIDC tokens | Real OIDC issuer (Auth0 / Keycloak / Azure AD) (D15) |
| Embedder | hash-fallback or sentence-transformers | Bundle-based ONNX runtime (D16/M13) |
| HA | single process | HPA 3-30 replicas + leader election (D9 / D18) |
| TLS | none | TLS 1.3 + mTLS optional (M12 D22) |
| NetworkPolicy | none | strict default-deny + whitelist (M12 D21) |
| Audit log | in-memory ring buffer | HMAC chain + Postgres + Object Lock 7-yr (D14-bis + D17) |
| Backup + DR | no persistence | D17 backup pipeline + D40 quarterly drill |
| Compliance | dev only | SOC 2 + ISO 27001 + GDPR + AI Act + NIST (M14) |

## Step 1 — Prerequisites

You'll need :
- A Kubernetes cluster (K8s 1.27+) — minikube / kind / EKS / GKE / AKS
- `helm` 3.13+
- An OIDC issuer (Auth0 / Keycloak / etc.)
- Postgres 14+ (managed RDS / Cloud SQL recommended)
- cert-manager 1.13+ installed (for mTLS)

## Step 2 — Install the Helm chart

```bash
# Pull from OCI registry (post-D40 GA, public registry)
helm pull oci://registry.efe.ai/charts/efe --version 1.0.0

# Or pre-D40 GA from private registry
helm pull oci://ghcr.io/efe-project/charts-private/efe --version 0.9.0
```

## Step 3 — Configure values

Create `values-production.yaml` :

```yaml
# Replicas
replicaCount: 3
autoscaling:
  enabled: true
  minReplicas: 3
  maxReplicas: 20

# Image (post-D40 GA)
image:
  repository: ghcr.io/efe-project/efe
  tag: "1.0.0"
  digest: "sha256:abc..."  # pin by digest for production

# OIDC
config:
  auth:
    enabled: true
    oidc:
      enabled: true
      existingSecretName: efe-oidc-issuers

# Postgres backend
config:
  catalog:
    backend: "postgres"
    postgres:
      host: "efe-pg.efe-catalog.svc.cluster.local"
      database: "efe_catalog_db"
      existingSecretName: "efe-pg-password"

# Bundles M13
config:
  embedder:
    backend: "bundle"
    bundle:
      existingSecretName: efe-bundle-paraphrase-multilingual
      trustStoreSecretName: efe-bundle-pubkeys

bundle:
  slug: paraphrase-multilingual-MiniLM-L12-v2
  version: "1.0.0"
  manifestSha256: "<filled at release>"

# M12 D21 NetworkPolicy strict
networkPolicyStrict:
  enabled: true
  ingress:
    tenantSelector:
      matchLabels:
        efe.io/tenant: "your-customer-tenant"

# M12 D22 mTLS
mtls:
  enabled: true
  issuerRef:
    name: efe-ca-issuer
    kind: ClusterIssuer

# M12 D27 bundle reload hook
bundleReload:
  enabled: true

# Backup + DR
backup:
  enabled: true
  drillQuarterly:
    enabled: true
```

## Step 4 — Provision secrets

```bash
# OIDC issuers
kubectl create secret generic efe-oidc-issuers \
    --from-file=oidc-issuers.json=./my-oidc-config.json \
    -n efe-production

# Postgres password
kubectl create secret generic efe-pg-password \
    --from-literal=password='<your-postgres-password>' \
    -n efe-production

# Bundle public keys (trust store)
kubectl create secret generic efe-bundle-pubkeys \
    --from-file=pubkeys.json=./bundles/pubkeys.json \
    -n efe-production
```

## Step 5 — Install

```bash
helm install efe efe-1.0.0.tgz \
    -f values-production.yaml \
    --namespace efe-production \
    --create-namespace \
    --atomic \
    --timeout 10m
```

## Step 6 — Verify

```bash
# Check pods
kubectl get pods -n efe-production -l app=efe-server

# Run helm test
helm test efe -n efe-production --timeout 5m

# Verify Polaris GREEN (M12 gate)
helm template efe efe-1.0.0.tgz -f values-production.yaml \
    | polaris audit --config deploy/helm/efe/.polaris.yaml \
                    --severity danger --output-format pretty
```

## Step 7 — Connect via SDK

Pick your favorite SDK :

```python
# Python (M8)
from efe_tool import Client

client = Client(
    endpoint="grpc.efe.example.com:443",
    oidc_token=os.environ["OIDC_TOKEN"],
    use_tls=True,
)
result = client.classify("support-routing", "I was charged twice")
print(result.top_class, result.confidence)
```

```go
// Go (M9)
import "github.com/efe-project/efe-sdk-go"

client, err := efe.NewClient(
    efe.WithEndpoint("grpc.efe.example.com:443"),
    efe.WithOIDCToken(os.Getenv("OIDC_TOKEN")),
    efe.WithTLS(),
)
result, err := client.Classify(ctx, "support-routing", "I was charged twice")
```

## Step 8 — Monitor

Production EFE ships :
- Prometheus metrics (M11) — see Grafana dashboards (D13)
- 10 D13 alerts + 22 D42 runbooks
- Quarterly DR drill (D40)
- Monthly KPI review per `docs/compliance/continuous_compliance.md`

## Step 9 — Compliance evidence

Per `docs/EFE_COMPLIANCE_EXTENSION_ARCHITECTURE.md` :
- SOC 2 evidence : `docs/compliance/soc2_evidence.md`
- ISO 27001 SoA : `docs/compliance/iso27001.md`
- GDPR Art. 32 : `docs/compliance/gdpr_art32.md`
- AI Act Annex IV : `docs/compliance/ai_act_annex_iv.md`
- NIST 800-53 : `docs/compliance/nist_800_53.md`
- Customer DDQ pre-fill : `docs/compliance/ddq_prefill.md` (243 questions)

## What you can do next

- **Customer onboarding** : per-tenant OIDC issuer config
- **Custom bundles** : build your own via `bundle-release.yml` workflow
- **Audit log analysis** : Loki + Grafana queries
- **Quarterly DR drill** : automated via M12 D40 CronJob
- **2.0 roadmap** : FedRAMP path, multi-bundle per-tenant routing

## Reference

- `docs/RELEASE_NOTES_1.0.md` — full GA release notes
- `docs/HANDOFF_OPS.md` — operations team handoff
- `docs/runbooks/general_incident_playbook.md` — incident response
- `docs/EFE_HELM_CHART_EXTENSION_ARCHITECTURE.md` — M12 chart deep dive

---

🎉 **You've completed the EFE Playground tutorials. Welcome to EFE 1.0 GA.**
