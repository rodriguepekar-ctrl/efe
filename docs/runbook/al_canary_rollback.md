# Runbook — AL Canary Auto-Rollback

> Sprint **Φ-D-AL Completion (D78)** — artefact #29.
> Cf. § 11 of completion archi.

## Symptômes

| Indicateur | Implication |
|---|---|
| `efe_al_canary_rollback_total` rate ≥ 1 | Auto-rollback fired |
| Customer reports : "results changed yesterday, now worse" | Promoted bundle had regression |
| `SLO-AL-PROMOTION-AUTO-ROLLBACK` injection test fails | Rollback mechanism broken |

## Diagnostic

```bash
# Recent rollback events.
psql -c "SELECT catalog_id, baseline_hash, new_hash, rollback_reason, timestamp
         FROM al_retune_attestations
         WHERE promotion_decision = 'rollback'
         ORDER BY timestamp DESC LIMIT 10"
```

## Common rollback triggers

* recall@1 delta < -0.01 (= regression)
* latency p99 increase > 20 %
* cost per call increase > 20 %
* Human review queue spike (= more "uncertain" flags than expected)

## Recovery

Once auto-rollback fires :
1. Bundle pointer flipped to baseline (= immediate user-visible).
2. Investigate root cause via audit attestations.
3. Identify offending label batches.
4. Optionally exclude those batches + manually retrigger retune.

## Escalation

| Sévérité | Qui | Délai |
|---|---|---|
| Single rollback in 24h | `#efe-al` | 4h |
| ≥ 3 rollbacks in 24h | PagerDuty | 1h |
| Rollback mechanism broken (= injection test fail) | SEV1 | 15 min |
