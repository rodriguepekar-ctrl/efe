# EFE P6 Lot 1 — consensus latent vs centroïde-kNN · st:paraphrase-multilingual-MiniLM-L12-v2

> Go/no-go du moat, **30 splits stratifiés répétés** (moyenne ± σ — le verdict ne dépend plus d'un seul tirage). kNN = consensus@1iter ; consensus = 6 iters. Même embedding & ancres → tout écart vient de l'itération. 108 clairs, 24 ambigus. Généré 2026-06-02T19:10:04.069230+00:00.

## Verdict

**INCONCLUS — le gain ambigu chevauche 0 (sous-puissance) : agrandir le seed / passer a bge-m3**

- **Δaccuracy AMBIGU strict (consensus − kNN) : -0.4% ± 3.1%** ← décide.
- Δaccuracy AMBIGU **lenient** (pred ∈ primaire ∪ `also`) : -0.3% ± 1.8%.
- ECE : kNN 0.282 ± 0.033 · consensus 0.183 ± 0.032.
- Désaccords nets en faveur du consensus : -0.1 ± 1.0 prompts.

## Accuracy par groupe (moyenne ± σ)

| groupe | kNN (1 iter) | consensus (K iter) |
|---|---|---|
| facile | 85% ± 5% | 85% ± 5% |
| ambigu (strict) | 52% ± 4% | 52% ± 4% |
| ambigu (lenient) | 93% ± 3% | 93% ± 3% |
| macro-F1 (global) | 0.73 ± 0.04 | 0.72 ± 0.04 |

## Comment lire

- **σ** dit si l'écart est robuste ou du bruit. Si la moyenne ± σ du Δambigu **chevauche 0**, le gain n'est pas prouvé (sous-puissance) → agrandir le seed ou passer à **bge-m3**.
- **lenient** : un ambigu est juste si la prédiction tombe sur la classe primaire **ou** une classe `also` légitime (frontière) — métrique plus équitable pour les ambigus.
- ⚠️ Embedder **hashing** = indicatif. **Verdict réel : `--embedder st`** (idéal `BAAI/bge-m3`).
- Sweep reproductible : `--sweep` (grille iters×tau×beta).
