# ADR-M31-001 : Compteurs Redis plutôt qu'insertions SQL per-événement

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M31_FINOPS_BILLING_ARCHITECTURE.md § 4.1

## Décision

Le hot-path `meter_inc` écrit dans un compteur Redis bucketé par
heure (key `m31:counter:{tenant}:{yyyymmddhh}` ; hash field = meter).
Un cron horaire snapshote dans PG `usage_rollup_hour` puis efface la
clé Redis.

## Justification

1. **Latence** — un `HINCRBY` Redis prend ~ 100 µs ; un `INSERT INTO`
   PG via sqlx + pool prend ~ 2-5 ms. À 10 RPS classify par tenant ×
   1 000 tenants × 8 meters par requête = ~ 80 k events/sec.
   PG saturerait. Redis tient.
2. **Cardinalité bornée** — un seul hash par (tenant, heure) ≪ une
   ligne par événement. TTL 48 h évite la dérive.
3. **Coût** — Redis Cluster M27 (existant) absorbe l'augmentation
   sans hardware additionnel ; PG insert-rate aurait nécessité un
   pgbouncer + sharding (~ 6 mois ingénierie).
4. **Acceptable lossiness** — perte = window entre dernier flush
   buffer et crash. NATS mirror (`meter.bumped`) fournit la replay
   durabilité.

vs **per-event SQL** : trop cher en write-amplification.
vs **OpenTelemetry exporters** : peu adapté aux compteurs business.

## Conséquences

### Positives
- `meter_inc` invisible côté caller.
- Rollup cron consomme ~ 30 s/heure.

### Négatives
- Redis devient SPOF pour la précision de facturation.
- Reconciliation contre NATS replay nécessaire en cas de Redis loss.
- Caution requise sur les rotations Redis Cluster (perdre une replica
  pendant le flush ⇒ events perdus si pas re-publiés sur NATS).
