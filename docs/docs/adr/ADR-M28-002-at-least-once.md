# ADR-M28-002 : At-least-once delivery + ULID event IDs

- **Status** : Accepted · **Date** : 2026-05-11

## Contexte

Le bus d'événements + le worker webhook + le CDC publisher doivent
arbitrer entre at-most-once, at-least-once, exactly-once.

## Décision

**At-least-once** avec **ULID** event IDs ; **subscribers de-dupent**.

Justification :
1. Exactly-once distribué = problème NP. NATS, Kafka, RabbitMQ
   l'approchent au prix d'overhead énorme (transactions 2PC, idempotency
   keys + acks de niveau brokers + state machines complexes).
2. At-most-once permet de perdre des events, ce qui est inacceptable
   pour l'audit chain et le billing.
3. At-least-once + idempotency-via-event-id est l'approche
   pragmatique : le subscriber maintient un cache `seen_ids` ou
   est conçu idempotent par construction.

## Conséquences

### Positives
- NATS JetStream RetentionPolicy::Limits + AckPolicy::Explicit suffit.
- ULID monotonic → cache `seen_ids` peut être un Bloom filter ou un
  set tronqué aux N dernières heures.
- CDC publisher : l'ID dérive de `(table, primary_key, xact_lsn)` ;
  un re-publish post-crash produit le même ID → dedup naturelle.

### Négatives
- Chaque subscriber doit implémenter sa propre dedup. Mitigation :
  doc + helper trait `IdempotentHandler<E>` dans `efe-events`.
- Les webhook customers doivent dedup côté reception (X-EFE-Webhook-ID
  unique). Documenté dans le webhook signing guide.
