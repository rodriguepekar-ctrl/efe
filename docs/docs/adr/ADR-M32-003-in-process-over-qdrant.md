# ADR-M32-003 : Index in-process + PG mirror plutôt que Qdrant out-of-process

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M32_VECTOR_SEARCH_ARCHITECTURE.md § 4.1

## Décision

M32 déploie un index HNSW **in-process** au sein de `efe-server`,
avec mirror PG comme source de vérité. Qdrant reste disponible
comme option pour les tenants qui dépassent 10 M vecteurs.

## Justification

1. **Operational simplicity** — un service de moins à monitorer,
   déployer, sauvegarder. Le PG mirror + S3 snapshot suffit pour
   la durabilité.
2. **Coût** — Qdrant Cloud Enterprise = ~ $5k/mois minimum. M32
   inline coûte 0 supplémentaire.
3. **Latence** — pas de round-trip réseau ; recherche dans le même
   process que les autres requêtes.
4. **Mise à l'échelle progressive** — M32 cible 1 M vecteurs/tenant.
   Au-delà, on passe à Qdrant transparently (option dans la config
   de la collection).

vs **Qdrant out-of-process** : meilleur pour > 100 M vecteurs ou
multi-tenant à très haute densité, mais surdimensionné pour notre
cible M32.
vs **pgvector seul** : pgvector recall plafonne à ~ 0.85 sur HNSW
sans ré-ranking ; M32 ships HNSW Rust natif qui ≥ 0.95.

## Conséquences

### Positives
- Zero new daemon à opérer.
- PG mirror = portabilité GDPR Art. 20 gratuite.

### Négatives
- L'index tient en RAM ⇒ memory pressure visible sur les pods
  `efe-server` à fort volume. M32.1 introduit des segments
  multiples pour répartir.
- Restart pod = rebuild HNSW depuis snapshot S3 (30-60 s pour 1 M
  vecteurs).
