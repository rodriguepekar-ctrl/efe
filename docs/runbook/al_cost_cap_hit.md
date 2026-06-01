# Runbook — AL Cost Cap Hit

> Sprint **Φ-D-AL Completion (D78)** — artefact #32.

## Symptômes

| Indicateur | Implication |
|---|---|
| `SLO-AL-COST-PER-LABEL` fail | Per-label cost > $0.01 p99 |
| `efe_al_cost_cap_hits_total` rate ≥ 1 | Tenant budget exhausted |
| `efe_al_llm_worker_paused_total` rate ≥ 1 | Worker auto-paused |

## Diagnostic

```bash
# Recent cost per tenant.
curl -s "http://prometheus/api/v1/query?query=sum%20by%20(tenant)(increase(efe_al_label_cost_usd_total%5B24h%5D))" | jq
```

## Common causes

* Unusual traffic spike → more samples → more LLM calls
* Verbose LLM prompt → token explosion
* Provider price change
* Tenant misconfiguration (= cap too low)

## Mitigation

1. Verify tenant budget : raise temporarily if traffic spike legitimate.
2. Check prompt template for verbosity (= compare token usage history).
3. Switch to cheaper provider for non-premium tenants.
4. Document incident + adjust budget defaults.

## Escalation

| Cost overrun | Action |
|---|---|
| 1 tenant @ cap | self-resolve | 24h |
| Multi-tenant @ cap | `#efe-al` Slack | 4h |
| Reconciliation drift > 5 % | PagerDuty + billing | 1h |
