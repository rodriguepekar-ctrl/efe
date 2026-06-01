# How-to — Multi-tenant deployment

> Sprint D11 deliverable. Audience: a partner who needs EFE to
> serve multiple end-customer tenants from the same install.

---

## Three tiers — pick one

| Tier | Mechanism | Cost (relative) | Use when |
|-----:|-----------|---------------:|----------|
| **1 — Soft** | Shared deployment, JWT scoping | × | small SaaS, low-trust mutualisation |
| **2 — Namespace** | One Helm release per tenant in its own namespace, with ResourceQuota + LimitRange + NetworkPolicy | 2× | mid-market SaaS, k8s-level fairness needed |
| **3 — Cluster** | One k8s cluster per tenant | 5-10× | regulated tenants, residency contracts (out of scope D11) |

For the architectural rationale, see
[explanation/security-model](../explanation/security-model.md) and
[`docs/bench/d11_per_tenant_isolation_architecture.md`](https://github.com/efe-project/efe/blob/main/docs/bench/d11_per_tenant_isolation_architecture.md)
(internal).

---

## Tier 1 (default) — same deployment, multiple tenants

Tier 1 is the chart's default. Add new tenants WITHOUT redeploying:

1. **Mint a JWT signing key** at your IDP carrying
   `tenant_id: "<your-tenant>"` in the claims.
2. **Distribute the JWT** to your tenant's application code.
3. **Verify the flow:**
   ```bash
   EFE_JWT=$(your-idp-cli token --tenant acme) \
     python examples/01_quickstart_classify.py
   ```

The server enforces scoping automatically:

* The rate limiter has a **separate token bucket per tenant** —
  one tenant's throttling doesn't impact another.
* The idempotency cache is **keyed by `(tenant_id, request_id)`**
  — same `request_id` from two tenants are independent entries.
* Prometheus metrics carry a `tenant` label everywhere.
* Audit logs include `tenant_id` on every line.
* Cone registry is **per-(tenant, task)** — RegisterCone/EvolveCone
  with a foreign tenant fails with `PERMISSION_DENIED`.

### Sanity check after adding a tenant

```python
from efe_tool import EfeAdminClient

with EfeAdminClient(endpoint="efe.your-corp:50051", jwt=your_jwt) as admin:
    refs = admin.list_catalogs()
    print(f"Tenant has {len(refs)} catalogs visible.")
```

The server filters by your JWT's tenant. You will NEVER see catalogs
belonging to another tenant via this method.

---

## Tier 2 — one Helm release per tenant

When tenant fairness must be enforced at the k8s layer (not just
in-process), promote to Tier 2.

### Onboard a new tenant

```bash
# Step 1 — create + label the namespace
kubectl create namespace tenant-acme
kubectl label namespace tenant-acme tenant.efe.ai/owner=acme

# Step 2 — install the chart with the Tier 2 preset
helm install efe-acme oci://ghcr.io/efe-project/charts/efe \
  --version 0.1.11 \
  --namespace tenant-acme \
  --values https://raw.githubusercontent.com/efe-project/efe/main/deploy/helm/efe/values.tier2.yaml \
  --set tenant.id=acme \
  --set tenant.namespace.networkPolicy.tenantIngress.allowFromNamespaceLabels."tenant\.efe\.ai/owner"=acme

# Step 3 — verify
helm test efe-acme -n tenant-acme
```

### What this gives you

* **`ResourceQuota`** caps the namespace's aggregate CPU / memory
  / pods. A tenant that runs away can't starve the cluster.
* **`LimitRange`** caps each pod's max size. No cookie-monster
  pods that hide quota.
* **`NetworkPolicy`** blocks ingress from any namespace not
  carrying the tenant-owner label. Cross-tenant network flows are
  denied at the L3/L4 layer, BEFORE the JWT auth layer.
* **`PrometheusRule`** ships per-tenant alerts
  (`EfeTenantHighErrorRate`, `EfeTenantRateLimitSaturated`,
  `EfeTenantConeRegisterStorm`, `EfeTenantCardinalityExplosion`).
* **`ServiceMonitor`** stamps the `tenant` label from the namespace.

### Decommission a tenant

```bash
helm uninstall efe-acme -n tenant-acme
kubectl delete namespace tenant-acme
```

The chart has no PVCs and no external state — uninstall is total.

---

## SDK patterns

### Per-tenant clients

```python
from efe_tool import EfeClient, LocalSTEmbedder

# One EfeClient per tenant. Never share a client between tenants —
# each carries the tenant's JWT.
clients: dict[str, EfeClient] = {}

def get_client(tenant: str) -> EfeClient:
    if tenant not in clients:
        clients[tenant] = EfeClient(
            endpoint=ENDPOINT,
            embedder=LocalSTEmbedder.from_env(),
            jwt=token_for(tenant),
            insecure=False,
        )
    return clients[tenant]
```

### Listing my tenant's catalogs

```python
from efe_tool import EfeClient, EfeAdminClient

with EfeAdminClient(endpoint=ENDPOINT, jwt=my_jwt) as admin:
    for ref in admin.list_catalogs():
        print(f"{ref.name}@{ref.version}: "
              f"{ref.num_classes} classes, {ref.num_anchors} anchors")
```

The server filters server-side based on your JWT's `tenant_id`. The
returned `CatalogRef.tenant` is always your tenant.

---

## Cost attribution

Every call increments two Prometheus counters labelled `tenant` :

* `efe_classify_compute_seconds_total{tenant}` — sum of fusion
  wallclock per tenant.
* `efe_egress_bytes_total{tenant}` — gRPC response bytes shipped
  to that tenant.

For monthly billing, run the aggregator:

```bash
python scripts/billing/aggregate_usage.py \
  --prom-url http://prometheus.monitoring:9090 \
  --start 2026-04-01T00:00:00Z \
  --end   2026-05-01T00:00:00Z \
  --output target/billing/2026-04.csv
```

The CSV has one row per tenant with `requests`, `compute_seconds`,
`egress_bytes`. See [`operations/billing`](../../operations/billing.md).

---

## Troubleshooting

### "PERMISSION_DENIED: tenant_id in request does not match authenticated principal"

The request body carries a `tenant` field (e.g. `ConeRef.tenant`)
that doesn't match the JWT's `tenant_id` claim. Likely causes:

* You hand-constructed a `ConeRef` with the wrong tenant.
* Your IDP issues JWTs with a different tenant than your app
  thinks.

Fix: confirm the JWT's `tenant_id` claim matches what you put in
the request.

### "UNAUTHENTICATED" on a fresh JWT

The JWT signing key isn't loaded server-side. Either:

* The JWT was signed with an old key removed during rotation.
  Check [`operations/key_rotation`](../../operations/key_rotation.md).
* Auth was disabled (`config.auth.enabled=false`) — that's a dev
  convenience, never use it in production. Tier 2 chart enforces
  `auth.enabled=true`.

---

## See also

* [Security model explanation](../explanation/security-model.md)
* [JWT claims reference](../reference/tenant-claims.md)
* [Operations / Multi-tenant observability](../../operations/multi-tenant-observability.md)
* [Operations / Billing](../../operations/billing.md)
