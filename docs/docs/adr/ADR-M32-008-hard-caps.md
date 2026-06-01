# ADR-M32-008 : Hard caps `max_dim = 4096`, `max_docs_per_collection = 10 M`

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M32_VECTOR_SEARCH_ARCHITECTURE.md § 2 + § 8

## Décision

- `MAX_DIM = 4096` — toute vecteur > 4096 dim est rejeté.
- `MAX_DOCS_PER_COLLECTION = 10_000_000` — overrides per-collection.
- `MAX_K = 10_000` — top-k cap.
- `MAX_COLLECTIONS_PER_TENANT = 10` — soft cap (élevable via quota).

## Justification

1. **MAX_DIM** : 4096 couvre tous les embedders publics (OpenAI
   text-embedding-3-large = 3072 ; Cohere = 1536). Refuser au-delà
   évite les attaques DoS via vecteurs adversariaux.
2. **MAX_DOCS** : au-delà de 10 M, HNSW in-RAM devient infaisable
   sur les nœuds standard (RAM 32 GB). Tenant à plus haute densité
   ⇒ M33 Qdrant out-of-process.
3. **MAX_K** : retourner > 10k résultats n'a aucun usage business.
4. **MAX_COLLECTIONS_PER_TENANT** : prévient la fragmentation.

## Conséquences

### Positives
- Surface d'attaque DoS bornée.
- Capacité planning prévisible.

### Négatives
- Tenants demandant > 10 M docs doivent contacter le SE. (Pas de
  self-service uplift.)
- `MAX_K = 10k` peut frustrer le très rare use case "give me all
  candidates" — workaround : pagination.
