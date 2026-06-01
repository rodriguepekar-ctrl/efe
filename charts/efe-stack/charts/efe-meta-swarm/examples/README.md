# efe-meta-swarm — example values overlays

Four production-ready values flavors curated for the most common
deployment scenarios.  Operators clone + override per cluster ;
the defaults inside the chart's own `values.yaml` remain conservative
(everything OFF) so this directory is the operator's starting point.

See `docs/EFE_A1_PHASE2_PR14_ARCHITECTURE.md` § 7 for the design.

## Flavors

| File | Use case | Notes |
|---|---|---|
| `values-dev.yaml` | Local laptop K8s (kind / minikube / Docker Desktop) | Small resources, no NetworkPolicy, reflection ON, hooks OFF |
| `values-staging.yaml` | Pre-prod cluster | Production-equivalent surface scaled down ; alerts + dashboard ON ; ignoreFailures default |
| `values-prod.yaml` | Production | Full hardening ; alerts critical ; hooks fail-fast ; PriorityClass ; node selectors + tolerations |
| `values-canary.yaml` | Canary slice alongside production | 1 controller + 1 participant ; alternate tenant slug ; isolated NetworkPolicy |

## Usage

### First-time install (production)

```bash
$ helm install efe-stack ./charts/efe-stack \
    --namespace efe-prod --create-namespace \
    -f charts/efe-stack/values-prod.yaml \
    -f charts/efe-stack/charts/efe-meta-swarm/examples/values-prod.yaml \
    --set global.tenant=kovant \
    --set global.metaSwarm.enabled=true \
    --set efe-meta-swarm.enabled=true
```

### Live manifest upgrade (production, with hooks)

```bash
$ vim values-prod.local.yaml      # edit manifest knobs

$ helm upgrade efe-stack ./charts/efe-stack \
    --namespace efe-prod \
    -f charts/efe-stack/values-prod.yaml \
    -f charts/efe-stack/charts/efe-meta-swarm/examples/values-prod.yaml \
    -f values-prod.local.yaml

# Hooks run automatically :
#   pre-upgrade : snapshot live ConfigMap → backup-N (rotating ring of 5)
#   apply       : ConfigMap mutates, Deployments unchanged
#   post-upgrade: SIGHUP controller + every participant
```

### Canary alongside production

```bash
# Main release.
$ helm install efe-stack ./charts/efe-stack -n efe-prod \
    -f examples/values-prod.yaml \
    --set global.tenant=kovant \
    --set efe-meta-swarm.enabled=true --set global.metaSwarm.enabled=true

# Canary release (different release name, different tenant slug).
$ helm install efe-stack-canary ./charts/efe-stack -n efe-prod \
    -f examples/values-canary.yaml \
    --set global.tenant=kovant-canary \
    --set efe-meta-swarm.enabled=true --set global.metaSwarm.enabled=true

# Operators target canary via label.
$ kubectl get pods -l efe.io/tenant=kovant-canary
```

### Air-gapped install

Add to your values overlay :

```yaml
image:
  repository: registry.internal/efe/efe-meta-swarm-server
  pullPolicy: IfNotPresent
hooks:
  image: registry.internal/efe/kubectl:1.28
bundle:
  sidecar:
    image: registry.internal/efe/pause:3.9
tests:
  grpcurlImage: registry.internal/efe/grpcurl:v1.9.1
```

## Cross-references

  * `docs/EFE_A1_PHASE2_PR13_ARCHITECTURE.md` (skeleton)
  * `docs/EFE_A1_PHASE2_PR14_ARCHITECTURE.md` (this PR — production-grade)
  * `docs/EFE_A1_PHASE2_PR12_ARCHITECTURE.md` (SIGHUP semantics)
  * Sub-chart README : `../README.md` (operator runbook)
