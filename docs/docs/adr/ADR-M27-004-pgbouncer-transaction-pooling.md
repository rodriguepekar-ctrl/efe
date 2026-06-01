# ADR-M27-004 : PgBouncer transaction pooling over per-pod sqlx pool only

- **Status** : Accepted
- **Date** : 2026-05-11
- **Deciders** : Eng lead, Backend lead, SRE
- **Related** : EFE_M27_COMMERCIAL_EDGE_ARCHITECTURE.md § 9

## Contexte

Aujourd'hui, chaque pod `efe-server` ouvre son propre `sqlx::PgPool` à
`max_connections=8`. À 10 pods → 80 connexions. Avec `efe-agent-runner`
controller + portal-admin + CI test jobs → on dépasse vite le
`max_connections=100` typique de RDS / managed PG. À échelle 50 pods
on est à 400+ connexions ; pas viable.

## Options considérées

### Option 1 : PgBouncer transaction pooling ✅

- **Pour** :
  - Multiplex 5000 client conn → 50 server conn.
  - Open-source, mature, déployé partout en prod.
  - Faible overhead (~50µs/req).
  - Compatible avec sqlx en désactivant statement cache.
- **Contre** :
  - Pas de prepared-statement cache cross-transaction.
  - LISTEN/NOTIFY non supportés (on ne les utilise pas).
  - Advisory locks cross-statement non supportés (idem).

### Option 2 : PgBouncer session pooling

- **Pour** : préserve prepared statements + LISTEN/NOTIFY.
- **Contre** : multiplexing dégradé (1 conn par client session) →
  perd l'intérêt principal.

### Option 3 : Pas de PgBouncer, augmenter sqlx pool

- **Pour** : zéro pod additionnel.
- **Contre** :
  - À 50 pods × 20 conn = 1000 conn → exhaust PG.
  - PG `max_connections` augmenté = mémoire PG quadruple.

### Option 4 : Supabase Supavisor / pgcat

- **Pour** : Rust-native, supportent prepared statements cross-tx.
- **Contre** :
  - Communauté plus petite, maturité plus jeune.
  - À évaluer post-M27 (réversible).

## Décision

**Option 1 retenue : PgBouncer transaction pooling**.

Justification :
1. Mature : déployé chez Heroku, Stripe, AWS RDS proxy uses it.
2. Compatible sqlx en désactivant `statement_cache_capacity` (déjà
   pattern dans db_context.rs).
3. Aucune feature PG utilisée bloque ce mode.
4. Operations team comfortable.

## Conséquences

### Positives
- Scale linéaire à 100+ efe-server pods sans toucher PG.
- 2 PgBouncer pods en HA suffisent (10k client conn cap each).
- Observabilité via pgbouncer_exporter standard.

### Négatives
- `statement_cache_capacity = 0` côté sqlx → coût de parsing
  query répétée. Mitigation : queries simples + index optimisés.
- Si on adopte LISTEN/NOTIFY future, devra contourner PgBouncer.

### Neutres
- Connection string change : `EFE_DATABASE_URL` pointe désormais sur
  PgBouncer service, pas PG direct.

## Pivot scenarios

Si statement cache absence pose problème de latence (P95 > 50ms sur
queries simples) :
- Migrer vers `pgcat` (Rust, supporte prepared statements cross-tx).
- Tweak final : utiliser sqlx avec `prepare_with_macro` pour les paths
  hot, et accepter session pooling pour ces routes.

## Révision

Revue fin Q3 — observabilité sqlx parse-time + PgBouncer pool
saturation.
