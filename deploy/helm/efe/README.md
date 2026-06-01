# efe — Helm chart

Deploys the EFE gRPC server on Kubernetes.
Specification: [`M12_ARCHITECTURE.md`](../../../M12_ARCHITECTURE.md).

## Install

```sh
# Register the OCI chart registry (once per workstation).
helm registry login ghcr.io -u $GITHUB_USER -p $GITHUB_TOKEN

# Install the latest release.
helm install efe oci://ghcr.io/efe-project/charts/efe \
    --version 0.1.0 \
    --namespace efe --create-namespace \
    --values values.prod.yaml \
    --set image.digest=sha256:<digest from release notes>
```

Or install straight from the repo:

```sh
helm install efe ./deploy/helm/efe \
    --namespace efe --create-namespace \
    --values ./deploy/helm/efe/values.prod.yaml
```

## What gets deployed

| Kind                     | Purpose                                      |
|--------------------------|----------------------------------------------|
| `Deployment`             | `efe-server` pods (distroless, non-root UID 10001) |
| `Service`                | ClusterIP `grpc:50051` + `metrics:9090`      |
| `ServiceAccount`         | No API-server token mounted by default       |
| `ConfigMap`              | `EFE_*` environment variables                |
| `Secret` (inline dev) OR | Only if `secrets.inline.enabled=true`        |
| `ExternalSecret` ref     | When `secrets.externalSecretName` is set     |
| `HorizontalPodAutoscaler`| 3 → 30 replicas by default                   |
| `PodDisruptionBudget`    | `minAvailable: 2`                            |
| `NetworkPolicy`          | ingress-nginx + prometheus + DNS egress only |
| `ServiceMonitor`         | Prometheus Operator discovery                |
| `PrometheusRule`         | 7 alert rules mapped to runbook anchors      |
| `PersistentVolumeClaim`  | 5 Gi audit log volume (prod)                 |

## Values reference

See [`values.yaml`](./values.yaml) for exhaustively commented defaults
and the environment-specific overlays:

- [`values.staging.yaml`](./values.staging.yaml)
- [`values.prod.yaml`](./values.prod.yaml)

## Secrets

Three secret strategies are supported, **strongly prefer option 1 in prod**:

1. **External** — `secrets.externalSecretName: efe-hs256` (managed by
   [`external-secrets`](https://external-secrets.io) from Vault / SSM / GSM).
2. **Existing self-managed Secret** — `secrets.existingSecretName: my-secret`.
3. **Inline (dev only)** — `secrets.inline.enabled=true, keys=[...]`
   puts the secret material into Helm's release history in plain base64.
   Do not use in production.

The Secret must expose a `hs256` key whose value is the multi-line
file consumed by `ServerConfig::from_env` (see M12 §8.2 / §15.2): one
base64-url-encoded HS256 key per line, first line = active signing key.

## Rollout and rollback

```sh
# Standard upgrade.
helm upgrade --install efe ./deploy/helm/efe \
    --namespace efe --values values.prod.yaml \
    --set image.digest=sha256:<digest>

# Rollback to the previous revision.
helm rollback efe --namespace efe

# Inspect history.
helm history efe --namespace efe
```

A configuration change alone triggers a rolling restart because the
Deployment template pins a `checksum/config` annotation — no manual
`kubectl rollout restart` is needed.

## OIDC tenant onboarding (Sprint D15)

When `config.auth.oidc.enabled=true`, the runtime validates JWTs
through a multi-issuer OIDC pipeline (Auth0, Keycloak, Okta, Azure
AD…) instead of the legacy HS256 path. See
[`docs/bench/d15_onboarding_idp_architecture.md`](../../../docs/bench/d15_onboarding_idp_architecture.md)
for the design.

### Operator checklist

1. Author your `oidc-issuers.json` (one entry per IdP). Template :
   [`deploy/config/oidc-issuers.example.json`](../../config/oidc-issuers.example.json).
2. Push it as a single-key K8s Secret :

   ```sh
   kubectl -n efe create secret generic efe-oidc-issuers \
       --from-file=oidc-issuers.json=./oidc-issuers.json
   ```

3. Set in your `values.yaml` :

   ```yaml
   config:
     auth:
       oidc:
         enabled: true
         existingSecretName: efe-oidc-issuers
   ```

4. Run the preflight check before flipping the release :

   ```sh
   ./efe-onboard preflight \
       --issuers-file ./oidc-issuers.json \
       --pg-url "$EFE_PG_URL"
   ```

5. Onboard tenants (one row in `tenants` per onboarded org) :

   ```sh
   ./efe-onboard register \
       --tenant-id acme \
       --display-name "Acme Corp" \
       --idp-issuer "https://acme.auth0.com/" \
       --idp-strategy claim
   ./efe-onboard activate acme
   ```

The registry is now the single source of truth ; the runtime
publishes `efe_tenants_total{status}` and emits an audit row on each
state transition (D14-bis chain).

## Tested on

- Kubernetes 1.27, 1.28, 1.29, 1.30
- Helm 3.14+
- Prometheus Operator (kube-prometheus-stack 58+)
- external-secrets 0.9+
- cert-manager 1.14+ (for the optional TLS ingress; ingress itself is
  not part of this chart)

## Uninstall

```sh
helm uninstall efe --namespace efe
kubectl delete namespace efe   # only if you wish to reclaim the PVC
```

The audit PVC is retained by default to preserve the on-disk JSONL
log; delete it explicitly if you are sure the data is not needed.
