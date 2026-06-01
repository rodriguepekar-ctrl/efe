# ADR-M31-003 : Rollups horaires (pas minutes)

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M31_FINOPS_BILLING_ARCHITECTURE.md § 5

## Décision

Le rollup primaire est **horaire**, pas par minute. `usage_now`
expose les compteurs Redis en temps réel pour les 60 dernières
minutes ; au-delà on lit `usage_rollup_hour`.

## Justification

1. **Volume PG** — 24 rows/jour/tenant/meter × 1k tenants × 8 meters
   = 192 k rows/jour. Soutenable. À la minute : 11.5 M rows/jour ;
   pgbouncer + table partitioning obligatoire.
2. **Granularité facturation** — la facturation est mensuelle. Une
   granularité horaire est largement suffisante pour les rapports
   "where did my money go".
3. **Coût de la précision** — `usage/projection` est lissé sur le
   mois entier ; pas besoin de précision sub-horaire.
4. **Buget retention** — `usage_rollup_hour` conservé 13 mois pour
   les comparaisons YoY ; à la minute on saturait le storage SLO.

vs **minute-level** : 60× plus de stockage + complexité partitioning,
sans bénéfice business.

vs **day-only** : insuffisant — on perd les motifs "burst en début
de mois" qui aident à conseiller les clients.

## Conséquences

### Positives
- Schema PG simple, pas de partitioning requis avant ~ 100k tenants.
- Backfill possible depuis NATS replay (compteurs cumulables).

### Négatives
- `usage/now` ne couvre que les 60 dernières minutes. Pour les
  tenants qui veulent voir "au cumul de la journée", on lit
  `usage_rollup_hour` ce qui ajoute ~ 30 ms à la query.
- Latence visible dans le portail : ~ 5 minutes (la cron tourne à
  HH:05). Acceptable car le portail n'est pas un dashboard live.
