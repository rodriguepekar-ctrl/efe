# ADR-M32-001 : HNSW plutôt que IVF / DiskANN / brute-force everywhere

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M32_VECTOR_SEARCH_ARCHITECTURE.md § 5.3

## Décision

HNSW comme algorithme principal d'approximate k-NN, avec fallback
brute-force (Flat) en dessous de 10 k vecteurs.

## Justification

1. **In-RAM** — HNSW est optimal quand l'index tient en RAM (notre
   cible : ≤ 1 M vecteurs dim 768 → ~ 3 GB). DiskANN serait surdimensionné.
2. **Recall vs latence** — à efSearch=64, recall@10 ≥ 0.95 vs flat
   sur 1 M docs en ~ 5-8 ms p99.
3. **Inserts incrémentaux** — HNSW supporte les inserts en O(log N)
   sans rebuild. IVF demande une re-clusterisation périodique.
4. **Ecosystem** — code de référence ouvert (hnsw_rs, hnswlib,
   FAISS) ; pas de risque algorithm.
5. **Pas de GPU requis** — DiskANN nécessite des SSD haute perf ou
   GPU pour la perf annoncée ; HNSW tourne sur CPU x86 standard.

vs **IVF (Inverted File)** : meilleur sur > 100 M vecteurs mais
demande clusterisation et re-train ; complexité opérationnelle.
vs **DiskANN** : excellent pour > 1 B vecteurs sur SSD mais
sur-dimensionné pour M32 cible.
vs **brute-force everywhere** : 10× plus lent à 1 M docs.

## Conséquences

### Positives
- Implementation Rust simple et auditable.
- Pas de dépendance Cargo nouvelle au-delà de `parking_lot`.

### Négatives
- HNSW memory overhead : ~ 4 × dim octets par vec en M=16.
- Délétions tombstoned, pas vraiment supprimées avant rebuild
  (snapshot replay).
