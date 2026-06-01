# Runbook — AL Gold Queue Overflow

> Sprint **Φ-D-AL Completion (D78)** — artefact #30.
> Cf. § 6 of completion archi.

## Symptômes

| Indicateur | Implication |
|---|---|
| `SLO-AL-GOLD-QUEUE-SIZE` ≥ 10 000 | Queue full, backpressure |
| `efe_al_queue_backpressure_total` rate spike | New samples rejected |
| `efe_al_llm_worker_lag_seconds` > 300 | Worker can't keep up |

## Diagnostic

```bash
# Queue size per tenant.
psql -c "SELECT tenant_id, count(*) FROM al_gold_queue
         WHERE status IN ('pending','llm_labeled','needs_review')
         GROUP BY tenant_id ORDER BY 2 DESC LIMIT 10"
```

## Mitigation

| Cause | Action |
|---|---|
| LLM oracle saturated | Scale `llm_worker` replicas |
| Human review backlog | Cf. `mode3_review_queue_full.md` runbook |
| Sudden traffic spike | Adjust uncertainty sampling budget |
| Worker stuck | Restart worker pod |

## Escalation

| Queue | Action |
|---|---|
| ≤ 5000 | monitor |
| 5000–9000 | `#efe-al` Slack |
| 9000–10000 | PagerDuty SEV3 |
| ≥ 10000 (overflow) | SEV2 + emergency capacity bump |
