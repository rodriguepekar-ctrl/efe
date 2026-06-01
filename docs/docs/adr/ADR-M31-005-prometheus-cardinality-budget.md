# ADR-M31-005 : Budget cardinalité Prometheus pour M31

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M31_FINOPS_BILLING_ARCHITECTURE.md § 9

## Décision

Aucune métrique M31 ne porte `tenant_id` en label. Les labels sont :

- `meter` : 8 valeurs (Meter::all).
- `outcome` : 2 (ok, err).
- `kind` (Redis errors) : 3 (transport, encode, capacity).
- `endpoint` (Stripe API) : ~ 8 valeurs.
- `event_type` (webhook) : ~ 12 valeurs.
- `severity` (budget) : 2 (soft, hard).
- `status` (invoice) : 6 (draft, open, paid, void, …).
- `period` (rollup lag) : 2 (hour, day).

Budget : ≤ 32 séries par histogramme. ~ 15 metrics × 32 = 480
séries totales.

## Justification

1. **Tenant cardinality** — interdit (D13 incident où ajouter
   `tenant_id` a fait passer Prometheus de 800k à 12M séries).
2. **Drill-down per-tenant** — passe par Loki (logs structurés) ou
   par PG (`v_tenant_month_to_date`).
3. **Ergonomie PromQL** — les dashboards FinOps montrent des
   tendances globales, pas du per-tenant.

## Conséquences

### Positives
- Marge confortable Prometheus single-shard.
- Cardinalité prévisible (~ 480 séries).

### Négatives
- Drill-down par tenant nécessite Loki ou SQL — flow plus lent
  pour l'on-call.
- Reports per-tenant générés via PG ad-hoc, pas Grafana.
