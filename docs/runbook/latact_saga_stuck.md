# Runbook — Saga stuck (= rollback failure ou execute timeout)

> **Doctrine** : D-LATACT-7 (= rollback atomique).
> **Symptôme** : `SLO-P2-ROLLBACK` FAIL OU outcome `CompensationFailure`.

## Détection

* Alert : `efe_saga_rollback_duration_seconds{p99} > 600`.
* Audit : event `compliance.saga_compensation_failed` apparait.
* User-facing : tenant POC reste en état `provisioning` > 30 min.

## Causes probables

1. **Compensation step hung** : ex. `drop database` bloqué par connexions actives.
2. **Resource lock conflict** : 2 sagas concurrentes sur même tenant.
3. **Sub-processor 5xx** : provider extérieur (Stripe, SendGrid) down.
4. **Idempotency key collision** : retry détecte état inattendu.
5. **GDPR S3 purge lent** : Glacier restore > 4h.

## Triage rapide

```bash
# 1. Saga en cours.
psql efe -c "SELECT saga_id, name, started_at, status FROM efe_saga_runs WHERE status = 'in_progress' ORDER BY started_at;"

# 2. Compensation pending.
psql efe -c "SELECT saga_id, step_idx, step_name, compensation_status FROM efe_saga_steps WHERE compensation_status = 'pending';"

# 3. Audit chain compensation events.
psql efe -c "SELECT * FROM audit_events WHERE action LIKE 'compliance.saga_%' ORDER BY ts_unix_ms DESC LIMIT 20;"
```

## Mitigation immédiate

1. **Step hung** : force-kill saga via `efe-admin saga kill --saga-id <ID>`.
2. **Lock conflict** : `pg_terminate_backend(pid)` sur la connexion bloquante.
3. **Sub-processor down** : retry policy avec backoff exponentiel.
4. **Idempotency collision** : audit + reset state machine via admin RPC.
5. **GDPR S3 lent** : accepter le delay + log audit (= cascade Glacier hors timer SLO).

## Mitigation durable

* Pin test `test_latact_efe_saga_unit_tests_present` empêche refactor cassant compensation order.
* Bench `p2_saga_rollback_bench.py --n-injections 100` nightly → tail p99 cap 10 min.
* Distributed lock Redis Redlock pour empêcher sagas concurrentes (= DT-LATACT-SAGA-DISTRIBUTED-LOCK).

## Cross-references

* SLOs : SLO-P2-ROLLBACK, SLO-P2-DELETE-GDPR
* Crate : `crates/efe-saga/src/lib.rs`
* Bench : `eval/scripts/p2_saga_rollback_bench.py`
* DT : DT-LATACT-SAGA-DISTRIBUTED-LOCK, DT-LATACT-SAGA-INJECT-PROD
