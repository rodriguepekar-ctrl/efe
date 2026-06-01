# Runbook — Streaming audit HMAC chain break

> **Symptôme** : `AdminService.ListAuditEvents` ferme le stream avec
> `Status::Internal("chain break at event_id …")` ou le bench
> `stream_audit_chain_replay.py` rapporte `total_broken > 0`.
>
> **Doctrine violée** : D-STREAM-6 (= audit chain integrity inter-batch).

## Détection

* Alert Prometheus : `increase(efe_audit_chain_breaks_total[1h]) > 0`.
* Workflow `stream-audit-replay.yml` retourne exit 1 sur le self-test.
* Client gRPC reçoit `Status::Internal` mid-stream avec un message
  `chain break at event_id 01h…`.

## Causes probables

1. **Out-of-band UPDATE sur la table `audit_events`** — quelqu'un a modifié
   `signature_hex` ou `prev_signature_hex` via `psql` direct (= cassure
   manuelle).
2. **HMAC key rotation incomplète** — la rotation 90 jours a renouvelé
   la clé mais des events anciens ont conservé leur ancienne signature.
3. **Migration PG mal appliquée** — un ALTER TABLE a perdu une colonne
   ou réordonné les rows.
4. **Concurrent INSERT race** — deux writers ont calculé le même
   `prev_signature_hex` simultanément avant que l'index unique ne fire.
5. **Backup / restore partiel** — une row a été perdue dans un PITR
   restore.

## Triage rapide

```bash
# 1. Identifier l'event_id du break.
echo "$STATUS_MSG" | grep -oE 'event_id [^ ]+'

# 2. Inspecter la chaîne avant/après l'event.
psql efe -c "
SELECT event_id, prev_signature_hex, signature_hex
FROM audit_events
WHERE tenant_id = '<TENANT>'
  AND event_id BETWEEN '<EVENT_ID-2>' AND '<EVENT_ID+2>'
ORDER BY event_id;"

# 3. Recomputer le HMAC attendu localement.
python3 eval/scripts/stream_audit_chain_replay.py \
    --chain-file /tmp/audit_dump.jsonl \
    --hmac-key-env EFE_AUDIT_HMAC_KEY
```

## Mitigation immédiate

1. **Quarantaine de la chaîne** : flag `audit_events.deleted=true` pour
   la fenêtre suspecte ; le stream filtre automatiquement.
2. **Rebuild de la chaîne** : un job admin `rebuild_audit_chain` (=
   sprint M29) recalcule les `signature_hex` à partir des `payload_json`
   préservés.
3. **Notification DPO** : un chain break sur la table audit a des
   implications GDPR Art. 30 ; déclencher un incident formel.

## Mitigation durable

* Hardening : `audit_events` table avec `REVOKE UPDATE` sauf pour le
  user `efe_app` ; les rotations de clé passent par une procédure
  signée.
* Pin test : `test_stream_audit_chain_replay_self_test` doit rester vert
  sur chaque PR.
* Réplique PG read-only utilisée pour le tail mode = pas de WAL replay
  drift.

## Cross-references

* Architecture : § 8.3 (verify_chain)
* SLO : `SLO-STREAM-AUDIT-CHAIN`
* Bench : `eval/scripts/stream_audit_chain_replay.py`
* Workflow : `.github/workflows/stream-audit-replay.yml`
