# Runbook — Tenant chargeback / budget overrun

> **Severity** : major (Sev2)
> **Owner** : FinOps on-call
> **Linked SLOs** : `SLO-OPS-BUDGET-OVERRUN`, `SLO-OPS-COST-PER-TENANT`,
> `SLO-OPS-AL-COST-PER-LABEL`
> **Alerts firing** : `OpsBudgetOverrun`, `OpsAlLoopCostExceeded`

---

## 0. TL;DR

A tenant's runtime spend exceeded **110 %** of their committed monthly
budget. This is a customer-trust issue : we must (1) confirm the
overrun is real, (2) notify the tenant, (3) decide between throttling
and absorbing the cost.

---

## 1. Confirm the overrun (≤ 10 min)

```bash
# Regenerate the chargeback rollup against live usage.
python3 scripts/ops/chargeback_compute.py \
    --usage target/ops/usage_$(date -u +%Y-%m).csv \
    --out  target/ops/chargeback_live.json

# Re-run the budget bench.
python3 scripts/ops/budget_alert_check.py \
    --chargeback target/ops/chargeback_live.json \
    --budgets    bench_baselines/v0.9.0/budgets.yaml
```

Inspect the per-tenant line :

```bash
jq '.rows[] | select(.tenant=="<slug>")' target/ops/chargeback_live.json
```

The breakdown shows the dominant cost driver
(`compute_usd` / `storage_usd` / `egress_usd` / `al_loop_usd` /
`overhead_usd`). Use it to pick the right mitigation in § 3.

---

## 2. Verify the metering is correct

False positives often come from a meter bug (= 2× count after a redis
restart, or a bad price book bump). Quick sanity :

```sql
-- Compare metered tokens vs LLM-router-emitted tokens for the tenant.
SELECT
    date_trunc('day', emitted_at) AS day,
    SUM(meter_value)              AS metered_tokens,
    (SELECT SUM(tokens_used) FROM llm_router_logs
     WHERE tenant_id = $1 AND emitted_at::date = day::date) AS router_tokens
FROM tenant_meters
WHERE tenant_id = $1
  AND meter_key = 'llm_tokens_routed'
  AND emitted_at > now() - INTERVAL '7 days'
GROUP BY 1 ORDER BY 1 DESC;
```

If the discrepancy > 5 %, treat this as a metering incident, **not** a
chargeback issue ; route to `efe-platform` on-call.

---

## 3. Mitigation paths

### 3.1 If `compute_usd` dominates

Likely cause : Mode-3 traffic surge or premium LLM token bloat.

```bash
efe-admin quotas set <tenant> --max-rps <half-current> --yes
efe-admin tenant update <tenant> --mode3-enabled false --yes
```

### 3.2 If `al_loop_usd` dominates

The active-learning loop is annotating too aggressively. Disable
opportunistic retunes for this tenant :

```bash
efe-admin learning retune disable <tenant>
```

### 3.3 If `egress_usd` dominates

Likely cause : tenant misconfigured a backup or replication. Reach out
to validate ; if accepted, enable cross-region replication caps in the
tenant K8s `Catalog` CRD.

### 3.4 If `overhead_usd` dominates (= small tenant + flat overhead)

Suggest a smaller plan ; no immediate action required.

---

## 4. Customer notification template

```
Subject : EFE billing notice — <tenant> 2026-MM

Hi <ops>,

Our automated chargeback rollup for <tenant> is flagging this month's
spend at USD <amount>, which is <ratio>× your committed budget of
USD <budget>.

The dominant driver is <category>. Per our SLA we are notifying you
≥ 7 days before invoice issuance so you can either :

  1. Re-confirm the increase and accept the higher invoice, or
  2. Throttle to <max_rps> RPS to stay within budget.

A summary breakdown is attached.

Best,
EFE FinOps
```

Always loop in the tenant's contact email recorded under
`tenants.contact_email`.

---

## 5. Closing

Once the situation is resolved :

- [ ] Add a row to `docs/postmortems/` only if the overrun was caused
      by an internal bug.
- [ ] Update `bench_baselines/v0.9.0/budgets.yaml` if the budget was
      renegotiated.
- [ ] Re-run `scripts/ops/budget_alert_check.py` ; confirm
      `over_count = 0`.

---

## 6. Cross-references

- [`scripts/ops/chargeback_compute.py`](../../scripts/ops/chargeback_compute.py)
- [`scripts/ops/budget_alert_check.py`](../../scripts/ops/budget_alert_check.py)
- [`docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md`](../EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md) § 12
