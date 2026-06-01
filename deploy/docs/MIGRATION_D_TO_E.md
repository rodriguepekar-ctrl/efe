# Migration Guide — From Letter D (embedded) to Letter E (service)

**Audience:** Python teams currently using `efe_tool.integrations.EfeFacade`
inside their application process, who want to switch to the centrally
managed service deployed by the Platform team.

**Estimated time:** 1–2 days, including staging validation.

---

## Why migrate?

| D (embedded)                              | E (service)                                   |
|-------------------------------------------|-----------------------------------------------|
| Each app process loads its own embedder   | One embedder per pod, shared across consumers |
| ~500 MB RAM per app replica               | RAM cost amortised over fleet                 |
| Upgrades coupled to app release           | Upgrades done by Platform team                |
| No multi-tenant isolation                 | Quotas + metrics per tenant                   |
| Python-only                               | Any gRPC language                             |

If you are the **only** Python consumer and your apps are not memory
constrained, **D is fine**. Migrate to E when a second team wants the
service, or when you deploy ≥ 5 replicas (RAM math wins).

---

## Architecture diff

```
    Before (D):                          After (E):
  ┌──────────────────┐                ┌──────────────────┐
  │  your-app pod    │                │  your-app pod    │
  │  ┌────────────┐  │                │  ┌────────────┐  │
  │  │ app code   │  │                │  │ app code   │  │
  │  │            │  │                │  │            │  │
  │  │ EfeFacade  │──│                │  │ EfeFacade  │──│──┐ gRPC
  │  │ embedder   │  │                │  │ (remote)   │  │  │
  │  │ 500 MB     │  │                │  │ 5 MB       │  │  │
  │  └────────────┘  │                │  └────────────┘  │  │
  └──────────────────┘                └──────────────────┘  │
                                                            │
                                      ┌──────────────────┐  │
                                      │  efe-server pods │◀─┘
                                      │  (managed by     │
                                      │   Platform team) │
                                      └──────────────────┘
```

---

## Step 1 — Request access

1. File a ticket in the platform team queue (`#efe-onboarding`).
2. Provide:
   - Team name.
   - Expected QPS (burst + sustained).
   - Catalogs to access (existing or new).
   - Contact for quota negotiation.
3. Platform team returns:
   - `tenant_id` (e.g. `team-billing`).
   - JWT client credentials (mounted as Kubernetes Secret).
   - mTLS CA to trust.
   - Staging + prod endpoints.

**Time:** 1–2 business days.

---

## Step 2 — Update your Python dependencies

Add the lightweight (no sentence-transformers!) client extra:

```toml
# pyproject.toml
[project]
dependencies = [
    "efe-tool[remote]==1.2.*",   # NEW: remote-only extra
    # REMOVE: "efe-tool[compile]"   # no need to embed the model
]
```

The `[remote]` extra pins `grpcio`, `protobuf`, but not `sentence-transformers`,
`torch` or `numpy` — your image shrinks by ~2 GB.

---

## Step 3 — Code change

The public API is **unchanged**. Only the config source differs.

```python
# Before
facade = EfeFacade(EfeConfig(
    endpoint="127.0.0.1:50051",
    catalogs=("support_triage",),
    catalog_search_paths=("catalogs",),   # local FS
    embedder_model="paraphrase-multilingual-MiniLM-L12-v2",  # loaded locally
))

# After
facade = EfeFacade(EfeConfig(
    endpoint="efe.platform.svc.cluster.local:50051",
    catalogs=("support_triage",),
    tls=True,                              # mTLS obligatoire
    tls_ca="/etc/efe/ca.crt",
    jwt=open("/etc/efe/jwt").read().strip(),
    tenant="team-billing",
    # NO more catalog_search_paths: the service loads them
    # NO more embedder_model: the service owns the model
))
```

The `EfeFacade.preload()` now just validates auth + does a healthcheck;
no more model warm-up. Cold start drops from ~1.5 s to ~50 ms.

---

## Step 4 — Secret mount (Kubernetes)

```yaml
apiVersion: v1
kind: Pod
metadata: { name: your-app }
spec:
  containers:
  - name: app
    image: your-app:1.0
    volumeMounts:
    - { name: efe-certs, mountPath: /etc/efe, readOnly: true }
  volumes:
  - name: efe-certs
    csi:
      driver: secrets-store.csi.k8s.io
      readOnly: true
      volumeAttributes: { secretProviderClass: efe-client-certs }
```

The `SecretProviderClass` is provided by the Platform team during
onboarding.

---

## Step 5 — Dual-run (recommended, 1 week)

Run both D and E behind a feature flag:

```python
if feature_flag("use_efe_remote"):
    facade = build_e_facade()
else:
    facade = build_d_facade()
```

Start at 10 % traffic, monitor disagreements:

```python
d_result = d_facade.classify(text, catalog="support_triage")
e_result = e_facade.classify(text, catalog="support_triage")
if d_result.label != e_result.label:
    log.warn("efe_disagreement", d=d_result, e=e_result)
```

Disagreement rate should be < 0.5 % (same catalog, same embedder).
Larger discrepancy → Platform team is running a different catalog version.

---

## Step 6 — Cut over

Once dual-run shows < 0.5 % disagreement for 72 h:

1. Flip feature flag to 100 % E.
2. Remove `[compile]` extra from `pyproject.toml`.
3. Strip model pre-download from your Dockerfile.
4. Roll out.

**Expected wins:**

- Image size: −1.5 to −2.5 GB.
- Cold-start RAM: −500 MB per replica.
- Rebuild time (CI): −2 to −4 min (no torch cache).

---

## Step 7 — Observability

After cutover, you lose the *local* metrics you had from the D
integration, but gain access to:

- `https://grafana.internal/d/efe-per-tenant?var-tenant=team-billing`
- SLO error budget for your tenant.
- Cost attribution line in the monthly `#efe-chargeback` report.

Add to your app's own dashboards:

```promql
# Classify calls originating from your app
sum(rate(efe_classify_requests_total{tenant="team-billing"}[5m]))

# Your app's experienced latency (from the client SDK)
histogram_quantile(0.95, rate(your_app_efe_rpc_duration_seconds_bucket[5m]))
```

---

## Step 8 — Rollback plan

If E causes incidents, rollback is a flag flip:

```python
feature_flag("use_efe_remote", enabled=False)
```

Bridge the gap by keeping `[compile]` extra installed for 30 days
post-migration. After 30 days stable on E, finalise the removal.

---

## Common pitfalls

1. **Forgetting mTLS in dev**: your local dev cluster uses self-signed
   certs. Use `tls=False` only in dev.
2. **Asking for too many catalogs**: JWT `catalogs:` claim limits access.
   Ask Platform for additions.
3. **Hard-coded `127.0.0.1:50051`**: use DNS
   (`efe.platform.svc.cluster.local`).
4. **Ignoring `EfeFacadeTransientError`**: the remote service returns
   503/504 sometimes. Retry with exponential backoff (or use a circuit
   breaker like `tenacity`).

---

## Checklist

- [ ] Tenant created, JWT received.
- [ ] Dependencies updated (`[remote]` extra).
- [ ] Code updated with `tls`, `jwt`, `tenant`.
- [ ] SecretProviderClass deployed.
- [ ] Staging deployed + smoke tested.
- [ ] Dual-run in prod 7+ days.
- [ ] Disagreement rate < 0.5 %.
- [ ] Cut over to 100 % E.
- [ ] `[compile]` removed from dependencies.
- [ ] Dashboards updated.
- [ ] Postmortem of migration filed (successes + gotchas).

**Questions?** `#efe-oncall` Slack or platform team office hours Tuesdays 14:00 UTC.
