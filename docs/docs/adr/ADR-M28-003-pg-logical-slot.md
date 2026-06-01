# ADR-M28-003 : PG logical replication slot per CDC publisher

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M28_STREAMING_EVENTS_ARCHITECTURE.md § 10

## Contexte

Nous voulons publier sur NATS chaque écriture sur `agent_audit`,
`classify_call_log`, etc. — y compris celles faites par jobs background
ou SQL manuel. Options : trigger-based publish (app code), polling
queries, ou CDC via logical replication slot.

## Décision

**PG logical replication slot** (`pgoutput` plugin) + binary
`efe-cdc-publisher`.

Justification :
1. **Exhaustivité** : capture toute écriture, même non-app.
2. **Exactly-once à l'amont** : slot avance `confirmed_flush_lsn` après
   NATS ack. Crash post-NATS ack mais pré-flush = re-publish (couvert
   par ADR-M28-002).
3. **Pas de coût pour les writers** : zero-cost à l'INSERT/UPDATE/DELETE.
4. **Évolutif** : ajouter une table à la publication = ALTER PUBLICATION.

## Conséquences

### Positives
- Audit chain complète et fiable.
- M24 active learning trigger sur `drift_event` simple : NATS sub.
- P3 billing aggregator a une source unique de classify_call_log.

### Négatives
- Slot abandonné → WAL exhaustion. Mitigation : alert + auto-recreate
  runbook RB_M28_CDC_LAG.
- Single-point-of-failure du CDC publisher pod. Mitigation : leader
  election via PG advisory lock (only one publisher claims the slot
  at a time ; failover dans 60s).
- DDL changes (column added) : pgoutput inclut le nouveau champ
  automatiquement ; subscribers tolèrent unknown fields.
