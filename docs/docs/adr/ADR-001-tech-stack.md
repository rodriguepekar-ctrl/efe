# ADR-001: Choix de la stack technique

- **Status**: Accepted
- **Date**: 2026-06-29
- **Deciders**: Tech Lead, Rust Lead, CUDA Lead, ML Lead
- **Consulted**: Safety Lead, SRE, CTO

## Contexte

Nous démarrons l'implémentation EFE. Le choix de stack verrouille
l'architecture pour 2-4 ans minimum. Critères prioritaires :

1. **Performance** — hot path latence < 100 ms end-to-end.
2. **Sécurité mémoire** — le système orchestre des tenseurs GPU
   concurrents ; un bug mémoire = incident production.
3. **Productivité ML** — templates, distillation, entraînement
   doivent rester simples (écosystème PyTorch).
4. **Interopérabilité** — Python pour data scientists, Rust pour
   systèmes, CUDA pour kernels.
5. **Recrutement** — talents disponibles, courbe d'apprentissage
   acceptable.

## Options considérées

### Option 1 : Rust + CUDA + Python (via PyO3) ✅

- **Pour** : performance native, sécurité mémoire, async mature,
  écosystème ML côté Python.
- **Contre** : courbe Rust pour 30% de l'équipe, compilation lente.

### Option 2 : C++ + CUDA + Python

- **Pour** : familiarité C++/CUDA, pas de friction FFI.
- **Contre** : bugs mémoire fréquents, pas de garanties async,
  gestion manuelle des lifetimes, risque élevé sur 2-4 ans.

### Option 3 : Python pur + PyTorch

- **Pour** : vélocité maximale, écosystème ML natif.
- **Contre** : GIL, latence inacceptable (> 500 ms p50 même optimisé),
  pas de déploiement natif léger, pas de garanties sécurité mémoire
  sur le hot path.

### Option 4 : Go + CUDA + Python

- **Pour** : compilation rapide, goroutines, simplicité.
- **Contre** : moins de contrôle fin qu'un type system Rust,
  écosystème CUDA faible, pas de zero-cost abstractions.

## Décision

**Option 1 retenue : Rust + CUDA + Python avec PyO3**.

Raison principale : le hot path (fusion, consensus, arbiter) nécessite
des garanties de sécurité mémoire et de zero-cost abstractions. Le
risque d'erreurs mémoire avec C++ + CUDA sur 2-4 ans est trop élevé
pour un système traitant des requêtes avec budget HBM important.

Rust offre par ailleurs un excellent écosystème async (`tokio`, `tonic`)
critique pour le serveur gRPC.

## Conséquences

### Positives

- **Sécurité mémoire** dans le hot path (core, memory, alignment).
- **Performance prévisible** (pas de GC, zero-cost async).
- **Écosystème async robuste** (tokio, tonic pour gRPC).
- **Ergonomie types** via `thiserror`, `serde`, `pyo3`.
- **Séparation claire** compute (Rust + CUDA) / ML (Python).

### Négatives

- **Ramp-up Rust** pour ~30% de l'équipe initiale (formation 1-2
  semaines + mentoring continu).
- **Temps de compilation** Rust pénalise l'itération rapide
  (atténué par `sccache`, `nextest`, incremental builds).
- **Interop PyO3** nécessite discipline (ne pas serialiser sur le
  hot path, bien gérer les GIL).
- **Debugging** cross-stack (Rust + CUDA + Python) demande outillage.

### Neutres

- **Triton vs CUDA brut** : à évaluer pragmatiquement Sprint 3.
  Fallback CUDA natif prévu si Triton limite la perf.
- **PyO3** est mature mais évolue (v0.22 actuel).

## Alternatives envisagées en pivot

Si l'Option 1 échoue sur des aspects fondamentaux :

- **Pivot mineur** : remplacer Triton par CUDA C++ pour les kernels
  critiques (décidable Sprint 3).
- **Pivot majeur** : migration partielle vers C++ via `cxx` bridge,
  uniquement pour les composants où Rust borrow checker bloque.
- **Abandon** : si > 50% du code devient des `unsafe` (symptôme
  que le problème ne correspond pas au modèle Rust), réévaluer.

## Révision

Revue obligatoire à la **fin du Sprint 7 (MVP, 2026-10-11)**.
Critères de revue :
- Productivité ingénieurs Rust-nouveaux après 3 mois ?
- Temps de compilation sur PR typique ?
- Nombre d'incidents liés à FFI / memory / async ?
- Perf kernels : Triton atteint-il les objectifs, ou fallback CUDA ?

Si les coûts d'ingénierie dépassent significativement les gains perf,
envisager un rewrite partiel ciblé.
