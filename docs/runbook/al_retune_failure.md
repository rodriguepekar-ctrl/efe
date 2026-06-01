# Runbook — Active Learning Retune Failure

> Sprint **Φ-D-AL Completion (D78)** — artefact #28.
> Cf. `docs/EFE_ACTIVE_LEARNING_COMPLETION_ARCHITECTURE.md` § 10-11.

## Symptômes

| Indicateur | Implication |
|---|---|
| `SLO-AL-RETUNE-LATENCY` fail (> 1h) | Retune cycle stuck |
| `SLO-AL-RECALL-IMPROVEMENT` fail | New bundle ≤ baseline |
| `efe_al_retune_failures_total` rate spike | Retune worker errors |
| Daily retune cron audit missing | Cycle didn't run |

## Diagnostic

```bash
# Check retune worker logs.
kubectl logs -f deployment/efe-al-retune-worker --tail=200

# Check last successful retune.
psql -c "SELECT catalog_id, MAX(timestamp_unix_micros)/1e6 FROM al_retune_attestations
         GROUP BY catalog_id"
```

## 3 typical causes

### A : Holdout split changes mid-cycle
Holdout fingerprint mismatch → re-tune aborts.

Fix : verify `scripts/active_learning/verify_holdout_integrity.py` ; if drift detected, regenerate holdout from snapshot.

### B : LLM oracle saturated
Cost cap hit → no new labels → recall improvement = 0.

Fix : check `efe_al_label_cost_usd_total{tenant}`. If at cap, either bump tenant budget OR defer retune.

### C : Concept drift > threshold
Wasserstein > 0.15 → retune triggered but new labels insufficient.

Fix : run `eval/scripts/al_drift_detector.py --mode live` to see drift magnitude. Increase sampling budget if drift severe.

## Mitigation immediate

1. Manual retune trigger : `efe-admin al retune --catalog X --force`.
2. If retune still fails : rollback to baseline bundle.
3. Open incident ticket.

## Escalation

| Sévérité | Qui | Délai |
|---|---|---|
| Single-catalog retune fail | `#efe-al` Slack | 4h |
| Multi-catalog fail | PagerDuty | 1h |
| All-catalog fail OR SLO breach | SEV1 + Eng lead | 15 min |
