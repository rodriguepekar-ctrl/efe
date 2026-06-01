# `deploy/` — Letter-E Operational Artefacts

Everything needed to run `efe-server` as a production-grade managed
service, as specified in `LETTER_E_ARCHITECTURE.md`.

## Directory layout

```
deploy/
├── helm/efe/              — Helm chart (production, Chart.yaml + templates/)
├── k8s/base/              — Kustomize base (namespace, deployment, service, ...)
├── k8s/overlays/
│   ├── staging/           — staging environment patches
│   └── prod/              — prod environment patches
├── docs/
│   ├── SLO.md             — Service Level Objectives (public contract)
│   ├── runbook.md         — on-call response playbook (13 alerts covered)
│   └── MIGRATION_D_TO_E.md — how to move a D consumer onto E
├── chaos/
│   └── game-day.md        — quarterly chaos drill script (5 scenarios)
├── observability/
│   ├── grafana/           — 3 dashboards (Overview, Per-Tenant, Fleet)
│   └── alerts/            — Alertmanager rules
├── load-tests/k6/         — classify.js, soak.js, admin.js
├── sdks/
│   ├── go/                — Go client README + example + Makefile
│   └── typescript/        — TS client README + example + package.json
├── proto-gen/scripts/     — gen-all / gen-go / gen-ts / gen-java
├── config/                — tenants example, SLO template
└── scripts/
    ├── chargeback.sh      — monthly tenant-level cost report
    ├── sign-image.sh      — cosign sign + syft SBOM
    └── smoke.sh           — post-deploy sanity check
```

## Fast path for a fresh deploy (staging)

```bash
# 1. Prerequisites in cluster (done once by ops):
#    - kube-prometheus-stack
#    - cert-manager
#    - Istio (or Linkerd)
#    - secrets-store.csi.k8s.io + Vault
#
# 2. Apply EFE manifests
kubectl create namespace efe-staging
kubectl label namespace efe-staging istio-injection=enabled \
  efe.project/environment=staging
kustomize build deploy/k8s/overlays/staging | kubectl apply -f -

# 3. Verify the pods
kubectl -n efe-staging get pods -w
kubectl -n efe-staging get servicemonitor

# 4. Register the first catalog via AdminService (from efe-admin ns)
kubectl -n efe-admin run admin-cli --rm -it --image=curlimages/curl -- \
  /v1/admin/catalogs -X POST \
  -H "Authorization: Bearer $ADMIN_JWT" \
  -d @catalog.yaml

# 5. Smoke test
export EFE_ENDPOINT=efe-staging.internal \
       EFE_JWT=$(vault read -field=token secret/efe/staging/qa-team) \
       EFE_TENANT=qa-team
bash deploy/scripts/smoke.sh --env=staging

# 6. Run a baseline load test
k6 run deploy/load-tests/k6/classify.js
```

## Fast path for a release

1. Tag: `git tag v0.2.0 && git push --tags`.
2. `release.yml` builds, signs, pushes the image + chart.
3. `canary.yml` rolls it out at 5 % to prod.
4. After 15 min of clean SLO burn, manually promote to 100 %.

See `.github/workflows/` for the full pipelines.

## Who owns what

| Artefact                     | Owner               |
|------------------------------|---------------------|
| Helm chart, Kustomize, YAML  | Platform team       |
| Grafana dashboards           | Platform + SRE      |
| Alertmanager rules           | Platform + SRE      |
| k6 load tests                | Platform + QA       |
| SDKs                         | Platform            |
| Runbook                      | Platform (on-call)  |
| SLO document                 | Platform + Product  |
| Chaos drills                 | SRE                 |
| Chargeback script            | Finance + Platform  |

## Related docs

- `LETTER_E_ARCHITECTURE.md` — architectural spec.
- `LETTER_D_ARCHITECTURE.md` — integration from a Python host (reference).
- `MODE1_ARCHITECTURE.md` — the core data/control-plane concepts.
- `M10_ARCHITECTURE.md` — gRPC service (server side).

## Feedback / change requests

Open a PR on this directory. Breaking changes to SLO, proto, or
public dashboards require sign-off from the platform lead + one
representative of a consumer team.
