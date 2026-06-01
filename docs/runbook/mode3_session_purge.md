# Runbook — Mode-3 Session Expiry Purge Failure

> Sprint **Φ-D-MODE3 Completion (D73)** — artefact #35.
> Cf. `docs/EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md` § 6.1.

## Symptômes

| Indicateur | Implication |
|---|---|
| `SLO-MODE3-SESSION-EXPIRY` fail | Sessions > 24h not purged |
| `efe_mode3_session_expiry_purges_total` rate flat-zero | Purge worker stopped |
| Postgres `mode3_session_state` table size growing > expected | Backlog |
| Customer reports : "session not deleted as promised" | UX + GDPR risk |

## Diagnostic

```bash
# Count sessions older than 24h still in state store.
psql -c "SELECT count(*), tenant_id FROM mode3_session_state
         WHERE created_at_unix_micros < extract(epoch from now() - interval '24 hours') * 1e6
         GROUP BY tenant_id
         ORDER BY 1 DESC
         LIMIT 10"

# Check purge worker logs.
kubectl logs -f deployment/efe-mode3-purge-worker --tail=100
```

## 3 typical causes

### A : Purge worker crashed
Worker pod is in CrashLoopBackOff or stopped.

Mitigation :
```bash
kubectl get pods -l app=efe-mode3-purge-worker
kubectl logs --previous deployment/efe-mode3-purge-worker  # check crash cause
kubectl rollout restart deployment/efe-mode3-purge-worker
```

### B : Postgres lock contention
Worker can't acquire row locks fast enough.

Mitigation :
```sql
-- Identify long-running queries.
SELECT pid, query_start, query
FROM pg_stat_activity
WHERE query LIKE '%mode3_session_state%'
AND now() - query_start > interval '30 seconds';

-- Kill long-running session locks (= safe at off-hours).
SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE pid IN (...);
```

### C : Audit cascade dependency

Purge waits for downstream audit log flush (= D14-bis HMAC chain) to
preserve referential integrity. If downstream stuck, purge stuck.

Mitigation :
* Check `d14bis_audit_chain` worker.
* If stuck, run manual catchup : `efe-admin audit catchup --service mode3`.

## Manual purge runbook

If automation fails, manual purge is safe IF you preserve audit trail :

```sql
-- Step 1 : snapshot rows to purge.
COPY (
  SELECT * FROM mode3_session_state
  WHERE created_at_unix_micros < extract(epoch from now() - interval '24 hours') * 1e6
) TO '/tmp/purge_snapshot_$(date +%s).csv';

-- Step 2 : verify audit trail exists for each.
SELECT s.session_id FROM mode3_session_state s
LEFT JOIN session_consent_log c ON s.session_id = c.session_id
WHERE s.created_at_unix_micros < extract(epoch from now() - interval '24 hours') * 1e6
AND c.session_id IS NULL;
-- ↑ should return 0 rows (= every session had consent traced).

-- Step 3 : delete.
DELETE FROM mode3_session_state
WHERE created_at_unix_micros < extract(epoch from now() - interval '24 hours') * 1e6;
```

## Verification

```bash
# Verify purge completed.
psql -c "SELECT count(*) FROM mode3_session_state
         WHERE created_at_unix_micros < extract(epoch from now() - interval '24 hours') * 1e6"
# Expected : 0

# Verify SLO recovered.
python3 scripts/mode3/verify_session_expiry.py
```

## Escalation

| Sévérité | Qui | Délai |
|---|---|---|
| Backlog < 100 sessions | self-resolve | 24h |
| Backlog 100-1000 | `#efe-mode3` Slack | 4h |
| Backlog > 1000 OR > 48h sessions | PagerDuty SEV2 | 1h |
| GDPR-bound retention overrun (= sessions > 25h with consent | PagerDuty SEV1 + DPO | 30 min |

## Liens

* SLO : `SLO-MODE3-SESSION-EXPIRY`
* Archi § 6.1, § 8
* Tool : `scripts/mode3/verify_session_expiry.py`
