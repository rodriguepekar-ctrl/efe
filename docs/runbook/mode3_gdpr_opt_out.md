# Runbook — Mode-3 GDPR Opt-Out Cascade

> Sprint **Φ-D-MODE3 Completion (D73)** — artefact #33.
> Cf. `docs/EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md` § 8.

## Symptômes

| Indicateur | Implication |
|---|---|
| `efe_mode3_consent_events_total{event_type="opt_out"}` rate spike | Mass opt-out event |
| `efe_mode3_session_expiry_purges_total` rate elevated | Purge backlog |
| Customer reports : "data still visible after opt-out" | Purge SLA breached |
| `SLO-MODE3-GDPR-OPT-IN-COVERAGE` fail | Orphan sessions detected |

## Opt-out cascade requirements

Per Art. 22 + EFE policy :
1. User-initiated opt-out triggers session termination.
2. Session state purge ≤ 30 s.
3. Audit log marks `opt_out` event with timestamp + reason.
4. Downstream systems (= active learning, anomaly detection) notified.
5. NO data retention beyond purge SLA.

## Diagnostic

### Step 1 : identify the affected sessions

```bash
# Recent opt-out events.
psql -c "SELECT session_id, tenant_id, timestamp_unix_micros, notes
         FROM session_consent_log
         WHERE event_type = 'opt_out'
         AND timestamp_unix_micros > extract(epoch from now() - interval '1 hour') * 1e6
         ORDER BY timestamp_unix_micros DESC
         LIMIT 50"
```

### Step 2 : verify purge completed

```bash
# For each opt_out session_id, verify session_state row is gone.
for sid in $(echo "$AFFECTED_SESSIONS"); do
    count=$(psql -t -c "SELECT count(*) FROM mode3_session_state WHERE session_id = '$sid'")
    if [ "$count" -ne 0 ]; then
        echo "PURGE FAILURE : session $sid still has state"
    fi
done
```

### Step 3 : audit chain replay for opt-out sessions

```bash
python3 scripts/mode3/replay_consent_log.py \
    --consent-log /tmp/consent_log.jsonl \
    --sessions-list /tmp/sessions.json \
    --out target/mode3/consent_audit.json
```

## Mitigation immediate

### Cas A : Purge SLA breach (= state retained > 30 s after opt-out)

This is a **CRIT** GDPR compliance violation.

1. Manually purge each affected session :
   ```sql
   DELETE FROM mode3_session_state WHERE session_id IN (...);
   DELETE FROM mode3_review_queue WHERE session_id IN (...);
   -- Audit log entries are RETAINED (= legal record of consent decision).
   ```

2. Log incident with timestamp gap (= purge_completed_at - opt_out_at).

3. Open compliance incident with DPO.

### Cas B : Mass opt-out (= reputation crisis)

If ≥ 1 % of active users opt-out in < 1h :
1. Immediate notification to executive team.
2. Pause new Mode-3 session creation tenant-wide (= ops kill switch).
3. Engage with affected users via Customer Success.

### Cas C : Orphan sessions (= sessions without opt-in)

This is a CRIT GDPR violation : sessions created without consent traced.

1. **STOP** all session creation immediately :
   ```bash
   kubectl set env deployment/efe-server EFE_MODE3_ALLOW_NEW_SESSIONS=false
   ```
2. Audit all orphan sessions :
   ```bash
   python3 scripts/mode3/replay_consent_log.py --consent-log ... --sessions-list ...
   ```
3. Force-purge orphans + open security incident.
4. Investigate root cause (= bypass in consent middleware ?).

## Escalation

| Sévérité | Qui | Délai |
|---|---|---|
| Single purge SLA breach | self-resolve | 24h |
| Multiple purge SLA breaches | `#efe-incidents` + DPO | 1h |
| Orphan session detected | PagerDuty SEV1 + DPO + Legal | 5 min |
| Mass opt-out event | PagerDuty SEV1 + executive | 5 min |

## Liens

* SLO : `SLO-MODE3-GDPR-OPT-IN-COVERAGE`
* Archi § 8 : `docs/EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md`
* Tool : `scripts/mode3/replay_consent_log.py`
* GDPR Art. 22 : <https://eur-lex.europa.eu/eli/reg/2016/679/oj#art_22>
