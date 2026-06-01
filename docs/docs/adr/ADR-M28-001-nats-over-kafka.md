# ADR-M28-001 : NATS JetStream over Kafka

- **Status** : Accepted
- **Date** : 2026-05-11
- **Related** : EFE_M28_STREAMING_EVENTS_ARCHITECTURE.md § 9

## Contexte

M28 introduit un event bus interne + une plateforme de pub/sub
customer-facing (webhooks). Deux candidats sérieux : NATS JetStream
et Apache Kafka. Aussi évalué : Redis Streams (rejeté car notre
Redis Cluster M27 est dédié au cache, pas au persistent durable).

## Options

| Critère | NATS JetStream | Kafka | Redis Streams |
|---|:--:|:--:|:--:|
| Mémoire / nœud | ~50 MB | 4 GB JVM | ~30 MB |
| Persistance Raft | ✅ native | ✅ KRaft (récent) | ⚠️ AOF only |
| Subject hierarchy / wildcards | ✅ `efe.tenant.*.>` | ❌ flat topics | ⚠️ key patterns |
| Replay par timestamp | ✅ start_time | ✅ offset | ⚠️ XREAD complex |
| Async-nats Rust crate maturité | ✅ official | ⚠️ rdkafka lourd | ✅ |
| ACL JWT decentralisée | ✅ nsc + JWT | ❌ Kerberos / TLS only | ❌ ACL basique |
| Ops complexity | bas | medium-haut | bas |
| Migration vers managed | NATS.io managed / Synadia | Confluent / MSK | Upstash |

## Décision

**NATS JetStream**.

Justification :
1. **Resource footprint** : 3 × 50 MB vs 3 × 4 GB. À la scale M28 (10M
   events/jour) Kafka serait surdimensionné.
2. **Subject hierarchy** : la convention `efe.tenant.{slug}.{topic}`
   se cartographie naturellement sur les NATS subjects + wildcards.
   Kafka aurait nécessité une topic-per-tenant ou une filter logic
   côté consumer.
3. **JWT-based ACL** (`nsc`) : compatible avec notre OIDC + KMS
   workflow. Kafka aurait demandé Kerberos ou SASL/SCRAM (heavier).
4. **Rust client** : `async-nats` est officiel + activement maintenu.
   `rdkafka` est mature mais lourd (libc binding).

## Conséquences

### Positives
- Pods NATS très légers ; déploiement k3d trivial.
- Replay par `start_time` simplifie le `POST /webhooks/:id/replay`.
- Décodage `pgoutput` côté CDC publisher + republication NATS naturelle.

### Négatives
- Écosystème Kafka plus large (Schema Registry, ksqlDB, Connect). On y
  renonce. Mitigation : Schema Registry remplacé par JSON Schema en
  `efe-events/schemas/` ; Connect remplacé par notre CDC publisher.
- Si on adopte Flink ou Spark Streaming plus tard, Kafka-bridge sera
  nécessaire. À ce moment-là, NATS → Kafka bridge via Synadia tooling.

### Pivot
- Switch vers Kafka via une nouvelle implémentation de
  `EventPublisher` / `EventSubscriber` traits. Aucun changement aux
  callers (l'abstraction le permet).
