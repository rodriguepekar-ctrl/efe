# ADR-M32-004 : Quantization binaire (1-bit) pour le MVP M32

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M32_VECTOR_SEARCH_ARCHITECTURE.md § 5.4

## Décision

Quantization binaire (1 bit / dim) comme option de compression.
Product Quantization (PQ) reportée à M32.1.

## Justification

1. **Simplicité algorithmique** — 1 ligne de code (sign bit), 1
   ligne de score (Hamming distance).
2. **32× compression** — dim 768 → 96 octets vs 3 KB. Permet
   ~ 10 M docs/GB de RAM.
3. **Recall préservé** — ≥ 0.85 vs fp32 sur SigLIP, validé par
   `bench/m32_recall.py`. Avec re-ranking step (load top-k×4,
   rerank fp32), recall remonte à ≥ 0.93.
4. **CPU-friendly** — Hamming distance = `popcount(a XOR b)` = 1
   cycle sur x86_64 + ARMv8.

vs **PQ (Product Quantization)** : meilleur recall à
compression équivalente mais nécessite k-means training par
sous-vecteur ⇒ pipeline plus complexe. M32.1 ajoutera.
vs **scalar quantization (int8)** : seulement 4× compression,
moins de bénéfice pour la complexité ajoutée.

## Conséquences

### Positives
- Implémentation minimale + auditable (~ 100 LOC).
- Pas de période d'entraînement.

### Négatives
- Recall ≈ 0.85 sans re-ranking. Pour les tenants qui exigent
  ≥ 0.95, M32 garde le mode non-quantizé par défaut.
- Pas adapté à toutes les distributions de vecteurs (queue
  d'angles très étroite ⇒ binarisation perd le signal).
