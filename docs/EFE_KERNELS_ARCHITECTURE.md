# `efe-kernels` — Architecture détaillée du module GPU/CUDA (M2)

> **Statut** : Draft v1.0.0 — Document de référence pour l'achèvement
> du module **M2 = `efe-kernels`** entre l'AS-IS (stub 29 lignes au
> 2026-04-28, post-D18) et la cible **GA 1.0** (CUDA fusion path
> production-ready, multi-GPU sharding, SLO C-1 ≤ 200 ms p99).
> **Auteur** : rodrigue.pekar@icloud.com
> **Date** : 2026-04-28.
> **Dépendances doc** : `MODE1_ARCHITECTURE.md`, `M12_ARCHITECTURE.md`,
> `docs/PRODUCT_COMPLETION_ARCHITECTURE.md` (§4 phase Φ-D),
> `crates/efe-core/src/compute/ROADMAP.md` (Phase 2-3 préparée),
> `docs/compute/tolerances.md` (cross-backend numérique).
> **Portée** : la totalité du portage GPU — sources CUDA, bindings
> Rust via `cudarc`, build pipeline `nvcc`, multi-GPU sharding via
> NCCL, observabilité GPU, fallback automatique CPU, et migration
> du stub `CudaBackend` depuis `efe-core` vers ce crate. Le document
> couvre les 5 sprints **D28 → D32** (10 semaines wallclock).

---

## Table des matières

§0. Méta, glossaire, conventions
§1. Mission de `efe-kernels`
§2. Position dans la topologie EFE — direction de dépendance
§3. État AS-IS au 2026-04-28
§4. Définition opérationnelle de `efe-kernels` GA
§5. Principes directeurs
§6. Invariants structurels
§7. Vue d'ensemble architecturale
§8. Le contrat — `ComputeBackend` 23 opérations × 4 niveaux de priorité
§9. Choix de la chaîne d'outils CUDA
§10. Système de build (build.rs + nvcc + features Cargo)
§11. Sous-système 1 — Couche d'intégration `cudarc`
§12. Sous-système 2 — Organisation des sources kernels `.cu`
§13. Sous-système 3 — Implémentation `CudaBackend` (migration depuis `efe-core`)
§14. Sous-système 4 — Gestion streams + mémoire device
§15. Sous-système 5 — Multi-GPU sharding (NCCL)
§16. Sous-système 6 — Harnais d'équivalence numérique CPU vs GPU
§17. Sous-système 7 — Budget perf + tile sizes par type GPU
§18. Sous-système 8 — Observabilité (6 instruments Prometheus)
§19. Sous-système 9 — Fallback + auto-détection au boot
§20. Déterminisme + reproductibilité sur GPU
§21. Sûreté numérique (NaN/Inf, OOM, overflow, deadlock)
§22. Taxonomie des erreurs
§23. Stratégie de tests (unit + golden + property + soak + cuda-memcheck)
§24. Matrice CI (runners GPU + cross-platform)
§25. Migration path — où `CudaBackend` réside
§26. Plan d'implémentation détaillé sprint-par-sprint (D28 → D32)
§27. Risques + atténuations
§28. Annexes — spec opération-par-opération (23 ops)
§29. Annexes — Gantt + RACI + budget cloud GPU
§30. Mises à jour du document

---

## §0. Méta, glossaire, conventions

### §0.1 Conventions

* **AS-IS** = état du code à fin D18 (post-audit), v0.1.19.
* **TO-BE** = état cible défini par chaque section.
* Les **opérations du trait `ComputeBackend`** sont nommées avec leur
  signature exacte ; elles sont citées en `code` style avec lien vers
  le fichier source.
* Les **kernels CUDA `.cu`** sont nommés `<op>.cu` avec une fonction
  `extern "C"` exportée préfixée `efe_<op>_launch`.
* Les sprints sont **D28 → D32** (continuité de la numérotation D18).
* Les défauts résiduels sont **DT-K.x** (Defect Trace, Kernels).
* Toute mention de "le moteur" sans qualification = `efe-core::SwarmController`.
* `[ref:fichier:ligne]` = pointeur dans le code source ; à grep pour
  vérifier.

### §0.2 Glossaire

| Terme | Définition |
|-------|------------|
| **`efe-kernels`** | Crate Rust dédié au substrat GPU. Contient les sources `.cu`, les bindings `cudarc`, et l'impl `CudaBackend` du trait `ComputeBackend`. |
| **`ComputeBackend`** | Trait Rust à 23 méthodes défini dans `efe-core/src/compute/backend.rs` ligne 38. Abstrait CPU vs GPU. |
| **`CpuBackend`** | Impl du trait dans `efe-core/src/compute/cpu/`. **100 % livrée** (Sprints 1-5 compute). |
| **`CudaBackend`** | Impl du trait pour CUDA. AS-IS : stub dans `efe-core/src/compute/cuda/mod.rs` (200 lignes, 23 méthodes `Err(NotImplemented)`). TO-BE : migré dans `efe-kernels` avec impl réelle. |
| **`cudarc`** | Crate Rust officiel non-`unsafe` de bindings CUDA (`cudarc 0.10+`). Ship driver + runtime + cuBLAS + cuBLASLt + cuRAND + NCCL. |
| **`nvcc`** | Compilateur CUDA C++ NVIDIA. Invoqué par `build.rs` via le crate `cc`. |
| **`cuBLAS`** | Lib NVIDIA pour BLAS sur GPU (matmul, dot, …). |
| **`cuDNN`** | Lib NVIDIA pour primitives ML (softmax, attention, …). Optionnel — on évite si possible (lourd, dépend versions précises). |
| **`NCCL`** | NVIDIA Collective Communications Library. Multi-GPU all-reduce. |
| **PTX** | Parallel Thread eXecution — IR intermédiaire CUDA, généré par `nvcc`. |
| **Kernel** | Fonction exécutée en parallèle par N threads sur GPU. Préfixe `__global__` en CUDA. |
| **Stream** | File d'attente CUDA d'opérations. Permet le parallélisme intra-device. |
| **Tile** | Bloc de mémoire shared utilisé par un warp/block pour la localité. |
| **Warp** | Groupe de 32 threads exécutés en SIMT sur NVIDIA. |
| **SM** | Streaming Multiprocessor — unité d'exécution physique GPU. |
| **VRAM** | Video RAM — mémoire HBM2/HBM3 du GPU. |

### §0.3 Cible matérielle

| Tier | GPU | VRAM | TFLOPS f32 | Use case |
|------|-----|:----:|:----------:|----------|
| **Tier-1 dev** | NVIDIA T4 | 16 GiB | 8.1 | smoke tests, kind cluster |
| **Tier-2 staging** | NVIDIA L4 | 24 GiB | 30.3 | bench staging |
| **Tier-3 prod** | NVIDIA A10 | 24 GiB | 31.2 | prod baseline |
| **Tier-4 prod** | NVIDIA A100 40 GB | 40 GiB | 19.5 (FP32 dense) / 312 (TF32) | prod high-load |
| **Tier-5 prod** | NVIDIA H100 SXM | 80 GiB | 51 (FP32) / 989 (TF32) | future prod multi-tenant |

Cible v1.0 GA = **Tier-1 + Tier-3** (T4 + A10) pour SLO C-1
(p99 ≤ 200 ms à 200 RPS). Tier-4/5 = post-GA scale-up.

---

## §1. Mission de `efe-kernels`

### §1.1 Énoncé opérationnel

> *Substituer aux kernels CPU livrés par `efe-core::compute::cpu` un
> ensemble de kernels CUDA fonctionnellement équivalents, dont
> l'exécution sur un GPU NVIDIA T4 ou supérieur permette à
> `efe-server` de tenir la SLO **C-1 : Mode-1 GPU p99 ≤ 200 ms à
> 200 RPS** (`docs/PRODUCT_COMPLETION_ARCHITECTURE.md` §3.1) —
> avec speedup ≥ 5× sur p99 par rapport au `MvpCpuFusion` actuel.*

### §1.2 Périmètre stricte

`efe-kernels` est responsable de **3 livrables atomiques** :

1. **Les sources CUDA C++** (`crates/efe-kernels/cuda/*.cu` + headers)
   — un kernel par opération du trait `ComputeBackend` qui n'a pas
   de mapping cuBLAS direct.
2. **Les bindings Rust → CUDA** via `cudarc` — chaque kernel
   `__global__` est exposé sous forme d'une `extern "C"` C function
   et appelé depuis Rust en `cudarc::driver::LaunchAsync`.
3. **L'impl `CudaBackend`** du trait `efe_core::compute::ComputeBackend`
   — 23 méthodes, une par opération, qui fait le pont entre la
   représentation `Tensor<f32>` et les buffers device.

### §1.3 Hors-périmètre

`efe-kernels` n'est **PAS** responsable de :

* La logique de fusion / consensus / dissolution — vit dans `efe-core`.
* L'orchestration multi-réplica — vit dans `efe-server::multi_replica`.
* Les LLM routers (Mode-2) — vit dans `efe-core::swarm::mode2`.
* Les SDKs clients Python/Go/TS/Java — vivent dans `clients/`.
* AMD ROCm / Intel OneAPI — déférés post-GA (v1.2).

### §1.4 Mission complémentaire

`efe-kernels` est aussi le **canari numérique** : tout kernel doit
être validé contre son équivalent CPU à `rtol=1e-4, atol=1e-5`
(tolérance définie dans `docs/compute/tolerances.md`). Toute
divergence > 1e-3 cosine déclenche un `tracing::error!` + alert
Prometheus.

---

## §2. Position dans la topologie EFE — direction de dépendance

### §2.1 Graph de dépendances post-D32

```
                    ┌─────────────────────────┐
                    │     efe-server          │
                    │   (gRPC frontends,       │
                    │    multi_replica, modes) │
                    └─────────┬───────────────┘
                              │
                    ┌─────────▼─────────┐
                    │     efe-core      │
                    │   (logic +        │
                    │    ComputeBackend │   ← TRAIT (23 méthodes)
                    │    trait,         │
                    │    CpuBackend)    │
                    └─────────▲─────────┘
                              │ depends on
                              │
                    ┌─────────┴───────────────┐
                    │     efe-kernels         │   ← CE DOC
                    │   (CudaBackend impl,     │
                    │    .cu sources,          │
                    │    cudarc bindings)      │
                    └─────────▲────────────────┘
                              │
                    ┌─────────┴────────┐
                    │   cudarc 0.10+   │
                    │   nccl-rs        │
                    │   (build deps)   │
                    └──────────────────┘
```

**Lecture critique** : la flèche `efe-kernels → efe-core` est
**unidirectionnelle**. `efe-kernels` IMPORTE le trait
`ComputeBackend` depuis `efe-core` et fournit une impl. `efe-core` ne
sait pas (et ne doit pas savoir) que `efe-kernels` existe.

### §2.2 Justification de cette direction

Cette direction garantit :

1. **`efe-core` reste compilable seul** sans dépendance CUDA. Crucial
   pour les tests CI macOS / Windows / sans GPU.
2. **Le client `efe-server` choisit dynamiquement** au boot : si
   feature `cuda` activée + GPU détecté → instancier `CudaBackend` ;
   sinon → instancier `CpuBackend`. Le `Box<dyn ComputeBackend>` est
   le polymorphisme de runtime.
3. **`efe-kernels` peut être absent** du build — `cargo build -p efe-server`
   sans `--features cuda` ne tire pas `efe-kernels` du tout.

### §2.3 Conséquence : où `CudaBackend` doit vivre

L'AS-IS place `CudaBackend` dans `efe-core/src/compute/cuda/mod.rs`.
**Cette position est temporaire**. Le TO-BE migre `CudaBackend`
vers `efe-kernels/src/backend.rs` :

```
AS-IS                                    TO-BE post-D29
─────                                    ──────────────
efe-core/src/compute/cuda/mod.rs   →    efe-kernels/src/backend.rs
  (200 lignes stub Err(NotImplemented))   (impl complète, ~800 lignes)

efe-core/src/compute/cuda/         →    [supprimé]
                                          (sauf re-export via cfg(cuda))
```

Cette migration est faite en **D29 — Sprint 2** de Φ-D (cf. §26.2).

---

## §3. État AS-IS au 2026-04-28

### §3.1 Inventaire physique

```
crates/efe-kernels/
├── Cargo.toml                          (683 octets)
├── src/
│   └── lib.rs                          (29 lignes, version stub)
└── cuda/
    ├── stub.cu                         (78 lignes, noop kernel)
    └── include/
        └── efe.cuh                     (15 lignes, constantes globales)
```

**Total** : 5 fichiers, ~120 lignes, **zéro fonction utile**.

### §3.2 Détail `Cargo.toml`

```toml
[package]
name = "efe-kernels"
description = "CUDA kernels and Rust bindings for EFE"

[dependencies]
efe-core = { path = "../efe-core" }     # ✅ direction OK
thiserror.workspace = true
tracing.workspace = true
tokio = { workspace = true, features = ["sync", "time", "rt"] }
half = { version = "2.4" }              # f16/bf16 — anticipation
rand = "0.8"
rand_pcg = "0.3"
rand_distr = "0.4"

[dev-dependencies]
tokio = { workspace = true, features = ["test-util", "macros", "rt-multi-thread"] }

[build-dependencies]
# Placeholder. Sprint 1+ will add cc / bindgen for CUDA integration.
```

**Constats** :

* ✅ `efe-core` en dépendance directe.
* ✅ `half` déjà inclus (anticipation FP16/BF16).
* ✅ `rand` + `rand_pcg` (déterminisme via PCG).
* ❌ **`cudarc` absent** — à ajouter en D28.
* ❌ **`build-dependencies` vide** — `cc` à ajouter en D28.
* ❌ **`nccl-rs` absent** — à ajouter en D30 (multi-GPU).

### §3.3 Détail `src/lib.rs` (29 lignes)

```rust
//! CUDA kernel bindings for EFE.
//!
//! MVP ships CPU-reference kernels inside `efe-core` to keep the
//! dependency direction linear (`efe-kernels → efe-core`, not the
//! reverse). Sprint 3+ will introduce real CUDA kernels here behind
//! trait-based launchers.

#![warn(clippy::all)]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub const fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
```

**Le seul export public** est `version()` — la crate est en pratique
un shell vide.

### §3.4 Détail `cuda/stub.cu` (78 lignes)

```cpp
// EFE CUDA kernels — Sprint 0 stub.
//
// Real kernels arrive in Sprint 1+:
//   - spawn_batch.cu (Sprint 2)        ← nommage roadmap original
//   - swarm_attention.cu (Sprint 3, may be Triton only)
//   - langevin_sde.cu (Sprint 3)
//   - frechet_mean.cu (Sprint 5)
//   - projection.cu (Sprint 5)

#include "efe.cuh"

extern "C" int efe_kernels_version() { ... }

__global__ void efe_noop_kernel() {}

extern "C" cudaError_t efe_noop_launch(cudaStream_t stream) {
    efe_noop_kernel<<<1, 1, 0, stream>>>();
    return cudaGetLastError();
}
```

**Lecture stricte** : seul le **build pipeline** est validé (un noop
kernel garantit que `nvcc` compile correctement quand activé). **Aucun
kernel réel** n'existe.

### §3.5 Détail `cuda/include/efe.cuh` (15 lignes)

```cpp
#pragma once
#include <cuda_runtime.h>
#include <cuda_bf16.h>

constexpr size_t EFE_TENSOR_ALIGNMENT = 256;
constexpr float EFE_EPS = 1e-6f;
constexpr float EFE_INIT_SIGMA = 0.02f;

#ifndef EFE_VERSION_MAJOR
#error "EFE_VERSION_MAJOR must be defined by the build system"
#endif
```

**Constats** :

* ✅ Alignement 256 octets — anticipation H100 cache lines.
* ✅ `EFE_EPS = 1e-6` — convention numérique partagée.
* ✅ Header inclut `cuda_bf16.h` — anticipation FP16/BF16.
* ❌ **`EFE_VERSION_*` macros non définies** par le `build.rs`
  actuel — le compile cassera dès qu'on tentera de l'inclure.

### §3.6 État `efe-core/src/compute/cuda/mod.rs` (CudaBackend stub)

[ref:`crates/efe-core/src/compute/cuda/mod.rs`] — **200 lignes** qui
déclarent `CudaBackend { ordinal: u32 }` et implémentent les
**23 méthodes** du trait `ComputeBackend`, **toutes retournant**
`Err(ComputeError::NotImplemented { op, backend: "cuda" })`.

Snippet :

```rust
impl ComputeBackend for CudaBackend {
    fn synchronize(&self) -> Result<()> {
        Err(ComputeError::NotImplemented { op: "synchronize", backend: "cuda" })
    }
    fn matmul(&self, _a: &Tensor<f32>, _b: &Tensor<f32>) -> Result<Tensor<f32>> {
        Err(not_impl("matmul"))
    }
    // … 21 autres méthodes identiques
}
```

Ce stub est **fonctionnellement testable** : on peut écrire
`Box::new(CudaBackend::new(0)?)` mais toute opération renvoie une
erreur claire.

### §3.7 État `efe-core/src/compute/` (CpuBackend) — 100 % LIVRÉ

D'après `crates/efe-core/src/compute/ROADMAP.md` :

* **Sprints 1, 2, 3, 4 Light, 5** ✅ LIVRÉS + audits.
* **23/23 ops du trait** implémentées dans `CpuBackend`.
* **1 260+ tests** GREEN avec `--all-features`.
* **22 golden vectors** chargés en tests.
* **33 property tests** (`proptest`, ~3 300 cas).
* **18 benchmarks criterion** (Sprint 2 + Sprint 3 + Sprint 4).
* **0 warning clippy** en `-D warnings` (pure + accelerate + cuda
  feature combinés).
* **BLAS Accelerate** branché (Mac) — speedup ×40 sur matmul 256².
* **Fused epilog** matmul+bias+GELU — speedup ×1.5-2 mémoire.
* **22 → 23 ops** : Sprint 5 a ajouté `l2_distance`,
  `cosine_similarity_pair`, `cosine_anchor_blend`, `shannon_entropy`,
  `sigmoid`.

**Conséquence pour `efe-kernels`** : la **référence numérique** est
solide. Tout kernel CUDA doit produire des résultats matchant le
`CpuBackend` à la tolérance documentée (`rtol=1e-4, atol=1e-5`).

### §3.8 Synthèse AS-IS

* `efe-kernels` (le crate) : **shell vide** (~120 lignes au total,
  une fonction `version()`, un kernel noop).
* `CudaBackend` (l'impl) : **stub dans `efe-core`**, 23 méthodes
  `Err(NotImplemented)`.
* **Aucune dépendance CUDA** ne tire encore (toolkit absent du build).
* **Trait + référence CPU livrés** ✅ — la cible numérique est claire.

**Verdict** : on a 80 % du DESIGN livré (le trait, la ref CPU, les
tests d'équivalence, la roadmap pré-faite) ; il reste 100 % du CODE
à écrire. Effort ≈ **40 jours-homme** sur 5 sprints.

---

## §4. Définition opérationnelle de `efe-kernels` GA

### §4.1 Le contrat GA

`efe-kernels` v1.0 satisfait **15 propriétés contractuelles** :

| # | Propriété | Métrique opposable |
|--:|-----------|--------------------|
| K-1 | Trait coverage | 23/23 ops `ComputeBackend` impl GREEN |
| K-2 | Numerical equivalence | Cross-backend rtol ≤ 1e-4, atol ≤ 1e-5 sur les 22 goldens |
| K-3 | Single-GPU latency | p99 fusion round ≤ 10 ms (T4, N=256, D=512) |
| K-4 | Single-GPU speedup | ≥ 5× vs `CpuBackend` à charge égale |
| K-5 | Multi-GPU scaling | ≥ 3× sur 4 shards (N=1024) |
| K-6 | Memory leak free | RSS stable sur 1000 swarms (cuda-memcheck CI nightly) |
| K-7 | Soak 24h | Aucune divergence > 1e-3 cosine sur 24h continu |
| K-8 | Determinism (single-GPU, fixed seed) | bit-identical sur 100 runs |
| K-9 | Streaming-compatible | `ExecuteStreaming` GPU GREEN avec back-pressure |
| K-10 | Hybrid-compatible | Run CPU shadow + GPU primary sans drop request |
| K-11 | Auto-fallback CPU | Si GPU OOM/error → fallback CPU sans drop request |
| K-12 | Multi-stream | ≥ 4 streams concurrents sans corruption |
| K-13 | NCCL all-reduce | Latence p99 ≤ 5 ms sur 4 GPUs PCIe |
| K-14 | OOM-safe | Refus propre `ResourceExhausted` si VRAM > 80 % |
| K-15 | Build cross-platform | `cargo build -p efe-server` sans `--features cuda` GREEN sur Mac/Linux/Windows |

### §4.2 Versioning produit aligné

| Version | Sprint | Livrable |
|---------|:------:|----------|
| **v0.5.0** | D30 | CUDA kernels v1 (single-GPU complet, 23/23 ops) |
| **v0.6.0** | D32 | Multi-GPU + streaming + hybrid + perf budget GREEN |
| v1.0.0 GA | D42 | Toutes K-1..K-15 vérifiées en CI |

### §4.3 Gating entre versions

Aucune version n'est releasée tant que :

* CI matrix **GPU runner** GREEN (T4 minimum).
* **22 golden vectors** matched à la tolérance définie.
* **`efe-bench --runtime gpu`** passe les 7 SLO D18-bench-1..7
  + le SLO C-1 (≤ 200 ms p99).
* **24h soak** sans memleak (`cuda-memcheck` rapport vide).
* **Determinism test** (100 runs même seed) bit-identical.

---

## §5. Principes directeurs

### §5.1 Principes opposables en revue PR

1. **Le trait est sacré**. Aucune extension du trait `ComputeBackend`
   en cours de Φ-D. Si une op CUDA nécessite un nouveau paramètre
   (ex: `cuda_stream`), elle est ajoutée **dans `efe-kernels` privément**
   et le trait reste figé. Toute modification du trait remonte à
   l'équipe Core (M1) avec ADR.
2. **Pas d'unsafe sans justification**. `cudarc` 0.10+ expose une
   API safe pour 95 % des cas. Tout `unsafe` dans `efe-kernels`
   doit avoir un commentaire `// SAFETY:` explicite + un test de
   non-régression.
3. **Pas de cuDNN obligatoire**. `cuDNN` est lourde (~500 MiB), versionnée
   strictement, et change de licence selon le contexte. On l'évite
   ; les kernels custom sont préférés. Exception unique : si un
   bench démontre ≥ 2× speedup d'un kernel custom vers cuDNN, on
   ouvre le débat.
4. **Toutes les ops cross-backend sont testées**. Pour chaque op
   du trait, il existe un test dans `tests/cross_backend_<op>.rs`
   qui appelle CPU + CUDA et vérifie tolérance.
5. **NaN/Inf est une erreur, pas un avertissement**. Toute
   opération qui produit NaN/Inf retourne
   `ComputeError::NumericalIssue { op, kind }`. Pas de propagation
   silencieuse.
6. **Build sans CUDA reste GREEN**. Toute modification de
   `efe-kernels` doit être validée par `cargo check --workspace`
   sur Mac (sans CUDA toolkit). Si CUDA toolkit absent, le crate
   compile avec une impl `Err(NotAvailable)`.
7. **Determinism d'abord, perf ensuite**. Si une optimisation casse
   le bit-identical sur même seed, on la rejette. La perf n'est jamais
   un argument suffisant pour casser le déterminisme.
8. **Streams par défaut, pas globaux**. Aucun kernel ne peut être
   appelé sur le default stream (stream 0). Tous les launches
   passent par un stream allocué explicitement.
9. **Une op = une fonction `extern "C"`**. Pas de fonctions
   "multi-purpose". Cela facilite le binding `cudarc` + le profiling.
10. **Doc pour chaque kernel**. Chaque `__global__` a un docstring
    `///` qui précise : signature, hypothèses, complexité, tile size
    optimal, GPU minimum.

### §5.2 Ce que `efe-kernels` refuse d'implémenter

* **Backends non-NVIDIA** (AMD ROCm, Intel OneAPI, Apple Metal)
  → déférés post-GA (cf. `PRODUCT_COMPLETION_ARCHITECTURE.md` §24.1).
* **Quantization runtime** (INT8, INT4) → out-of-scope kernels ;
  géré au niveau bundle/embedder en D26.
* **Custom training loops** — `efe-kernels` est inférence-only.
  Le forward pass uniquement.
* **Auto-tuning runtime** (cuDNN-style heuristic) → out-of-scope GA ;
  les tile sizes sont sélectionnées au build time selon
  `EFE_GPU_TIER` env var. Auto-tuning post-GA.
* **CUDA Graph capture** (cudaGraph) → out-of-scope GA ; potentielle
  optimisation post-GA pour l'amorti de launch overhead.

---

## §6. Invariants structurels

### §6.1 Invariants de structure de fichiers

* `efe-kernels/src/lib.rs` est le **point d'entrée unique** du crate.
  Toute API publique est ré-exportée depuis ici.
* `efe-kernels/src/backend.rs` contient l'unique impl
  `CudaBackend` du trait `ComputeBackend`. Pas de variantes
  (CudaBackendF16, etc.) — le typage générique gère ça.
* `efe-kernels/cuda/` contient **uniquement** des sources CUDA
  (`.cu`, `.cuh`). Aucun `.rs` ici.
* Toute `extern "C"` exposée par un `.cu` est nommée
  `efe_<op>_launch` ou `efe_<op>_<variant>_launch`.
* `efe-kernels/src/streams.rs` est le seul module qui possède un
  `cudarc::driver::CudaStream`. Aucune autre place ne crée des
  streams.
* `efe-kernels/src/memory.rs` est le seul module qui possède un
  `cudarc::driver::CudaSlice`. Toute alloc passe par lui.

### §6.2 Invariants de runtime

* Aucun kernel ne tourne sur le **default stream** (stream 0).
* Toute alloc device est tracée via `tracing::debug!(target: "efe.cuda.alloc", ...)`
  pour faciliter le memleak hunting.
* Toute libération de slice device synchronise le stream associé
  avant le free (évite use-after-free).
* `CudaBackend::synchronize()` n'est appelé que dans :
  * Les tests (pour assertions).
  * Le shutdown (pour drain final).
  * Le multi-GPU all-reduce (avant et après NCCL ops).
  Jamais dans le hot path.
* Aucun kernel ne fait de `cudaMalloc`/`cudaFree` en interne.
  Tout est pré-alloué dans le `Tensor` qui en sortie.
* Le `Tensor::device()` est figé à la construction. Pas de
  promotion CPU → CUDA en runtime ; cela passe par un appel
  explicite `tensor.to_device(Device::Cuda(0))`.

### §6.3 Invariants de versioning

* La version du crate `efe-kernels` est **identique** à celle de
  `efe-core` (workspace SemVer).
* `cudarc` est pinné à une version mineure (ex `cudarc = "0.10"`).
* `cuda_runtime.h` minimum **CUDA 12.4**. Documenté dans `efe.cuh`.
* `nvcc` minimum **12.4**. Vérifié par `build.rs`.

---

## §7. Vue d'ensemble architecturale

### §7.1 Arborescence cible post-D32

```
crates/efe-kernels/
├── Cargo.toml                         (étendu : cudarc, cc, nccl-rs)
├── build.rs                           (NOUVEAU — D28)
├── src/
│   ├── lib.rs                         (étendu : ré-exports CudaBackend, etc.)
│   ├── backend.rs                     (NOUVEAU — D29 ; migré depuis efe-core)
│   ├── streams.rs                     (NOUVEAU — D29 ; CudaStream pool)
│   ├── memory.rs                      (NOUVEAU — D28 ; CudaSlice mgmt)
│   ├── kernel_loader.rs               (NOUVEAU — D28 ; PTX/cubin loader)
│   ├── kernels.rs                     (NOUVEAU — D28 ; bindings extern "C")
│   ├── multi_gpu.rs                   (NOUVEAU — D30 ; NCCL + sharding)
│   ├── fallback.rs                    (NOUVEAU — D29 ; auto-detect + fallback)
│   ├── observability.rs               (NOUVEAU — D29 ; 6 Prom instruments)
│   ├── error.rs                       (NOUVEAU — D28 ; CudaError taxonomie)
│   └── tests/
│       ├── integration_smoke.rs       (NOUVEAU — D28)
│       ├── cross_backend_<op>.rs      (NOUVEAU — D28-D30, 23 fichiers)
│       ├── multi_gpu.rs               (NOUVEAU — D30)
│       ├── memcheck.rs                (NOUVEAU — D31)
│       └── soak_24h.rs                (NOUVEAU — D31)
├── cuda/
│   ├── include/
│   │   ├── efe.cuh                    (étendu : tile sizes, alignement)
│   │   └── efe_warp.cuh               (NOUVEAU — D28 ; warp primitives)
│   ├── linalg/
│   │   ├── matmul.cu                  (NOUVEAU — D28)
│   │   ├── matmul_fused_gelu.cu       (NOUVEAU — D28)
│   │   ├── dot.cu                     (NOUVEAU — D28)
│   │   ├── l2_norm.cu                 (NOUVEAU — D28)
│   │   ├── l2_normalize.cu            (NOUVEAU — D28)
│   │   ├── l2_distance.cu             (NOUVEAU — D28)
│   │   └── cosine.cu                  (NOUVEAU — D28 ; sim + blend)
│   ├── activations/
│   │   ├── softmax.cu                 (NOUVEAU — D28)
│   │   ├── log_softmax.cu             (NOUVEAU — D28)
│   │   ├── gelu.cu                    (NOUVEAU — D28)
│   │   └── sigmoid.cu                 (NOUVEAU — D28)
│   ├── attention/
│   │   ├── scaled_dot_product_attention.cu (NOUVEAU — D28)
│   │   └── flash_attention.cu         (NOUVEAU — D31, opt)
│   ├── reductions/
│   │   ├── topk.cu                    (NOUVEAU — D28)
│   │   ├── argmax.cu                  (NOUVEAU — D28)
│   │   ├── mean.cu                    (NOUVEAU — D28)
│   │   ├── std_dev.cu                 (NOUVEAU — D28)
│   │   ├── shannon_entropy.cu         (NOUVEAU — D28)
│   │   └── kl_divergence.cu           (NOUVEAU — D28)
│   ├── efe_core/
│   │   ├── cone_project.cu            (NOUVEAU — D28 ; cœur EFE)
│   │   ├── weighted_mean.cu           (NOUVEAU — D28 ; cœur EFE)
│   │   └── moe_reconcile.cu           (NOUVEAU — D29 ; Mode-2 prep)
│   ├── sampling/
│   │   └── multinomial_sample.cu      (NOUVEAU — D29)
│   └── stub.cu                        (à supprimer en D32)
└── benches/
    ├── kernel_microbench.rs           (NOUVEAU — D31 ; criterion)
    └── e2e_swarm_round.rs             (NOUVEAU — D31)
```

**Total cible** : ~25 fichiers `.cu` + ~12 fichiers `.rs` = ~5 000 LOC
CUDA + ~3 500 LOC Rust + ~1 500 LOC tests = **~10 000 lignes**.

### §7.2 Lifecycle d'un appel `CudaBackend::scaled_dot_product_attention`

```
Application (efe-server) ──┐
                            │
  let backend: Box<dyn ComputeBackend> = build_backend(&cfg)?;
                            │
                            ▼
  ┌─ CudaBackend::scaled_dot_product_attention(q, k, v, mask) ─┐
  │                                                            │
  │ 1. Validate shapes → ComputeError::ShapeMismatch if KO     │
  │ 2. Get current stream from streams::current_stream()       │
  │ 3. Allocate output tensor on device                        │
  │ 4. Compute launch params (blocks, threads, smem)           │
  │ 5. Launch kernel via cudarc::driver::launch_kernel:        │
  │       efe_sdpa_launch(stream, q.dev, k.dev, v.dev,         │
  │                       mask.dev, output.dev, b, h, t, d)    │
  │ 6. Record event for telemetry                              │
  │ 7. Return Tensor wrapping output device buffer             │
  │ (No sync — caller responsible)                             │
  │                                                            │
  └────────────────────────────────────────────────────────────┘
                            │
                            ▼
                   Result<Tensor<f32>>
```

### §7.3 Flux dependency au build

```
cargo build -p efe-server --features cuda
                    │
                    ▼
       Cargo: dep graph efe-server → efe-kernels → efe-core
                    │
                    ▼
   ┌────────────────┴───────────────┐
   │      efe-kernels build.rs      │
   │                                 │
   │ 1. Detect nvcc in PATH          │
   │    → if absent: warn + skip CUDA│
   │      (compile error stubs only) │
   │                                 │
   │ 2. Detect CUDA arch from env    │
   │    EFE_CUDA_ARCH (default 80    │
   │    for Ampere = T4/A10/A100)    │
   │                                 │
   │ 3. For each .cu in cuda/:       │
   │    nvcc -arch=sm_${ARCH}        │
   │         -O3 -Xcompiler=-fPIC    │
   │         -c source.cu -o source.o │
   │                                 │
   │ 4. Link static lib              │
   │    libefe_kernels.a             │
   │                                 │
   │ 5. cargo:rustc-link-lib=...     │
   │    cargo:rustc-link-search=...  │
   └─────────────────────────────────┘
                    │
                    ▼
          target/release/efe-server (with CUDA symbols linked)
```

---

## §8. Le contrat — `ComputeBackend` 23 opérations × 4 niveaux de priorité

### §8.1 Inventaire complet

[ref:`crates/efe-core/src/compute/backend.rs`] définit le trait
`ComputeBackend` avec 23 méthodes. Catégorisation par hot-path
fréquence + impact perf :

#### Priorité **P0 — Infra** (3 ops)

| Op | Signature | Description | Sprint |
|----|-----------|-------------|:------:|
| `device` | `&self -> Device` | Device handle | D29 |
| `name` | `&self -> &'static str` | "cuda" | D29 |
| `synchronize` | `&self -> Result<()>` | `cudaStreamSynchronize` | D29 |

#### Priorité **P1 — Hot path fusion** (8 ops, 80 % temps round)

| Op | Cuda mapping | Sprint |
|----|--------------|:------:|
| `scaled_dot_product_attention` | custom kernel `sdpa.cu` (D31 flash-attention opt) | D28 |
| `softmax` | custom kernel warp-reduce | D28 |
| `weighted_mean` | custom kernel | D28 |
| `cone_project` | custom kernel | D28 |
| `mean` | custom kernel reduce | D28 |
| `std_dev` | custom kernel reduce | D28 |
| `shannon_entropy` | custom warp-reduce | D28 |
| `l2_normalize` | custom kernel | D28 |

#### Priorité **P2 — Hot path support** (6 ops)

| Op | Cuda mapping | Sprint |
|----|--------------|:------:|
| `matmul` | `cublasSgemm` | D28 |
| `matmul_fused_gelu` | custom kernel (fused) | D28 |
| `gelu` | custom kernel (libm::erff equivalent) | D28 |
| `sigmoid` | custom kernel | D28 |
| `dot` | `cublasSdot` | D28 |
| `l2_norm` | `cublasSnrm2` | D28 |

#### Priorité **P3 — Cold path / Mode-2/3** (6 ops)

| Op | Cuda mapping | Sprint |
|----|--------------|:------:|
| `topk` | custom kernel (warp top-k) | D28 |
| `argmax` | custom kernel reduce | D28 |
| `kl_divergence` | custom kernel | D29 |
| `multinomial_sample` | custom kernel + cuRAND | D29 |
| `moe_reconcile` | custom kernel (top-k softmax) | D29 |
| `cosine_similarity_pair` + `cosine_anchor_blend` + `l2_distance` | custom kernels | D29 |
| `log_softmax` | custom kernel | D29 |

**Total** : 23/23 ops impl en D28-D29.

### §8.2 Ordre d'implémentation

Critères :
1. **P1 d'abord** — sans P1 le swarm ne tourne pas du tout.
2. **P2 ensuite** — débloquer SLO C-1.
3. **P3 enfin** — utiles pour Mode-2/3 mais non bloquant Mode-1.

Ordre concret D28 (sprint 1) :
* J1-J2 : `synchronize` + `mean` + `std_dev` + `softmax` (4 ops simples reduce)
* J3-J4 : `matmul` (cuBLAS) + `dot` + `l2_norm` (3 ops cuBLAS)
* J5-J6 : `gelu` + `sigmoid` + `argmax` + `topk` (4 ops simples elementwise)
* J7-J8 : `weighted_mean` + `cone_project` + `l2_normalize` + `shannon_entropy` (4 EFE-core)
* J9 : `scaled_dot_product_attention` (composé de matmul + softmax)
* J10 : Tests cross-backend, audit numérique.

### §8.3 Ce qui est PAS dans le trait (pour info)

Le trait ne contient **PAS** :

* L'allocation de `Tensor` (gérée par `Tensor::zeros`/`Tensor::from_slice`).
* Le transfert host ↔ device (gérée par `Tensor::to_device`).
* La sérialisation (gérée par `serde` côté `Tensor`).
* Les ops in-place (le trait est purement fonctionnel — chaque op
  crée un nouveau `Tensor` ; les optimisations in-place sont
  détectées par le compilateur Rust via le borrow checker).

---

## §9. Choix de la chaîne d'outils CUDA

### §9.1 Décision : `cudarc` vs alternatives

| Option | Pour | Contre | Verdict |
|--------|------|--------|:-------:|
| **`cudarc 0.10+`** | Pure Rust, safe API, modulaire (driver/runtime/cuBLAS/cuRAND/NCCL), maintenu (NVIDIA-employee), no `unsafe` dans 95 % des cas | Couvre pas 100 % de cuDNN | ✅ **RETENU** |
| `rustacuda` | Maintenance limitée (dernière release 2021) | Stagnant | ❌ |
| `cust` | Compile Rust → PTX | Cool mais expérimental | ❌ |
| Bindgen direct sur `cuda_runtime.h` | Maximum flexibilité | Maintenance manuelle, `unsafe` partout | ❌ (rejeté en revue) |
| FFI manuel via `cc` + `extern "C"` | Simple | Verbeux, pas de cuBLAS wrappé | ⚠️ Complément (pour kernels custom) |

**Décision** : `cudarc` pour 95 % du code + FFI manuel via `cc` pour
les kernels custom où on appelle directement `extern "C" cudaError_t
efe_<op>_launch(...)`.

### §9.2 Modules `cudarc` retenus

```toml
# Cargo.toml additions D28
cudarc = { version = "0.10", default-features = false, features = [
    "driver",      # CudaDevice, CudaStream, CudaSlice
    "cublas",      # cuBLAS GEMM, dot, nrm2
    "cublaslt",    # cuBLASLt (matmul_fused_gelu)
    "curand",      # multinomial_sample
    "nccl",        # multi-GPU all-reduce (D30)
    "f16",         # half precision (D31 future)
] }
```

### §9.3 Toolchain CUDA

| Component | Min version | Notes |
|-----------|:-----------:|-------|
| **CUDA Toolkit** | 12.4 | Pinné via `EFE_CUDA_VERSION` env var en CI |
| **`nvcc`** | 12.4 | Auto-détecté par `build.rs` |
| **`cuBLAS`** | 12.4 (bundled) | Via `cudarc` |
| **`cuBLASLt`** | 12.4 (bundled) | Via `cudarc` (matmul_fused) |
| **`cuRAND`** | 10.3 (bundled) | Via `cudarc` |
| **`NCCL`** | 2.18 | Optionnel ; multi-GPU only ; via `cudarc` |
| **`cuDNN`** | n/a | **Évité** (cf. principe §5.1.3) |
| **`nvprof`** | bundled | Profiling local |
| **`nsight-compute`** | bundled | Profiling avancé |
| **`cuda-memcheck`** | bundled | CI nightly memleak |

### §9.4 Compute capability cible

| Compute Capability | GPU correspondants | Action |
|:-------------------|--------------------|--------|
| `sm_75` (Turing) | T4 | ✅ obligatoire (Tier-1 dev) |
| `sm_80` (Ampere) | A10, A100 40GB | ✅ obligatoire (Tier-3 prod) |
| `sm_86` (Ampere) | RTX 3000 | ✅ obligatoire (DEV alternative) |
| `sm_89` (Ada Lovelace) | L4, RTX 4000 | ✅ obligatoire (Tier-2 staging) |
| `sm_90` (Hopper) | H100 | ✅ obligatoire (Tier-5 future) |
| `sm_70` (Volta) | V100 | ⚠️ optional (deprecated NVIDIA roadmap) |

**Build par défaut** : `nvcc -arch=sm_75 -code=sm_75,sm_80,sm_86,sm_89,sm_90`.
Cela produit un fat binary couvrant tous les targets prod sans
recompilation.

---

## §10. Système de build (`build.rs` + nvcc + features Cargo)

### §10.1 Cargo features

```toml
[features]
default = []
# Compile-time switch ; absent en build CI Mac (no CUDA toolkit).
cuda = ["cudarc/driver", "cudarc/cublas", "cudarc/cublaslt", "cudarc/curand"]
# Multi-GPU support (deps NCCL).
nccl = ["cuda", "cudarc/nccl"]
# Half-precision support (deps cudarc f16).
fp16 = ["cuda", "cudarc/f16"]
# Auto-tuning (post-GA).
autotune = ["cuda"]
```

### §10.2 `build.rs` (D28 livrable)

Pseudocode du `build.rs` :

```rust
fn main() {
    println!("cargo:rerun-if-env-changed=EFE_CUDA_ARCH");

    let cuda_feature_active = env::var("CARGO_FEATURE_CUDA").is_ok();
    if !cuda_feature_active {
        // Pas de feature cuda → noop. Rust crate compile mais ne
        // contient que les stubs Err(NotAvailable).
        println!("cargo:warning=efe-kernels: CUDA feature disabled.");
        return;
    }

    // 1. Détecter nvcc.
    let nvcc = which("nvcc").unwrap_or_else(|_| {
        panic!("CUDA feature enabled but nvcc not in PATH. \
                Install CUDA Toolkit ≥ 12.4 or unset --features cuda.")
    });

    // 2. Détecter version.
    let version_output = Command::new(&nvcc).arg("--version").output().unwrap();
    let version_str = std::str::from_utf8(&version_output.stdout).unwrap();
    let (major, minor) = parse_nvcc_version(version_str);
    if major < 12 || (major == 12 && minor < 4) {
        panic!("efe-kernels requires CUDA ≥ 12.4 ; found {major}.{minor}");
    }

    // 3. Compute archs.
    let archs = env::var("EFE_CUDA_ARCH")
        .unwrap_or_else(|_| "75,80,86,89,90".to_string());

    // 4. Compile each .cu file via cc + nvcc.
    let cu_files = glob("cuda/**/*.cu").unwrap();
    let mut cc_build = cc::Build::new();
    cc_build
        .compiler(&nvcc)
        .cuda(true)
        .flag("-O3")
        .flag("-Xcompiler=-fPIC")
        .flag(&format!("-DEFE_VERSION_MAJOR={}", env!("CARGO_PKG_VERSION_MAJOR")))
        .flag(&format!("-DEFE_VERSION_MINOR={}", env!("CARGO_PKG_VERSION_MINOR")))
        .flag(&format!("-DEFE_VERSION_PATCH={}", env!("CARGO_PKG_VERSION_PATCH")))
        .include("cuda/include");
    for arch in archs.split(',') {
        cc_build.flag(&format!("-gencode=arch=compute_{arch},code=sm_{arch}"));
    }
    for f in cu_files { cc_build.file(f); }
    cc_build.compile("efe_kernels_cuda");

    // 5. Linker hints.
    println!("cargo:rustc-link-lib=cudart");
    println!("cargo:rustc-link-lib=cublas");
    println!("cargo:rustc-link-lib=cublasLt");
    println!("cargo:rustc-link-lib=curand");
}
```

### §10.3 Comportement build sans CUDA

```bash
$ cargo build -p efe-kernels                 # default features = []
   Compiling efe-kernels v0.1.x
   Compiling efe-server v0.1.x  (links efe-kernels but no CUDA symbols)
   Finished [no GPU code]
```

```bash
$ cargo build -p efe-kernels --features cuda   # full
   Compiling efe-kernels v0.1.x
   Compiling efe-kernels (build script)
     nvcc: detected CUDA 12.4
     compiling 25 .cu files for sm_{75,80,86,89,90} ...
   Finished
```

### §10.4 Cross-compilation

* **Build Linux x86_64 → x86_64** : ✅ standard.
* **Build Linux x86_64 → arm64** (Jetson) : ✅ via cross-toolchain
  cuda-tegra. Spécifier `EFE_CUDA_ARCH=87` (Orin).
* **Build macOS** : ❌ pas de CUDA toolkit. `--features cuda` panic au
  build time avec message clair.
* **Build Windows** : ⚠️ supporté via WSL2 ou native MSVC ; CI
  Windows non prévue avant v1.1.

---

## §11. Sous-système 1 — Couche d'intégration `cudarc`

### §11.1 Module `kernel_loader.rs` (D28)

Charge les kernels custom compilés en PTX/cubin :

```rust
use cudarc::driver::{CudaDevice, CudaFunction, LaunchAsync};

pub struct KernelLoader {
    device: Arc<CudaDevice>,
    cache: HashMap<&'static str, CudaFunction>,
}

impl KernelLoader {
    pub fn new(device: Arc<CudaDevice>) -> Result<Self> {
        // Charger le module compilé statiquement (libefe_kernels.a)
        // expose des fonctions extern "C" qu'on appelle directement.
        Ok(Self { device, cache: HashMap::new() })
    }

    pub fn launch_kernel(
        &self,
        name: &'static str,
        grid: (u32, u32, u32),
        block: (u32, u32, u32),
        shared_mem_bytes: u32,
        stream: &CudaStream,
        args: &[*const std::ffi::c_void],
    ) -> Result<()> {
        // Appel via cudaLaunchKernel ou cudarc::launch_kernel.
        // Pour les kernels custom, on les expose en extern "C" dans
        // .cu et on les appelle via un wrapper Rust simple.
        unsafe {
            cudarc::driver::result::launch_kernel(
                self.cache[name].handle(),
                grid, block, shared_mem_bytes,
                stream.handle(),
                args,
            )?;
        }
        Ok(())
    }
}
```

### §11.2 Module `kernels.rs` (D28)

Re-exports des `extern "C"` C functions :

```rust
// Bindings vers les .cu files compilés statiquement.
extern "C" {
    pub fn efe_softmax_launch(
        stream: cudaStream_t,
        input_dev: *const f32, output_dev: *mut f32,
        n: u32, axis_size: u32, axes: u32,
    ) -> cudaError_t;

    pub fn efe_cone_project_launch(
        stream: cudaStream_t,
        latents_dev: *const f32, anchors_dev: *const f32, output_dev: *mut f32,
        b: u32, n: u32, d: u32, max_dist: f32,
    ) -> cudaError_t;

    // ... 23 ops × N kernels = ~30 extern "C"
}
```

### §11.3 Helper `cuda_check_status!` macro

```rust
#[macro_export]
macro_rules! cuda_check {
    ($expr:expr) => {{
        let status = $expr;
        if status != 0 {
            return Err(CudaError::FromCuda(status));
        }
    }};
}
```

Utilisée systématiquement après chaque `efe_*_launch`.

---

## §12. Sous-système 2 — Organisation des sources kernels `.cu`

### §12.1 Layout `cuda/`

Cf. §7.1 pour l'arborescence complète. Principes :

* **Une op = un fichier `.cu`**. Pas de bundling.
* **Une `__global__` kernel par op simple** ; multiples kernels pour
  ops complexes (ex `scaled_dot_product_attention.cu` a 3 kernels :
  attention scores, softmax, output).
* **Un `extern "C"` launcher par fichier** (au minimum). Le launcher
  s'appelle `efe_<op>_launch`.
* **`include/efe.cuh`** = constantes globales partagées
  (`EFE_TENSOR_ALIGNMENT`, `EFE_EPS`).
* **`include/efe_warp.cuh`** = primitives warp partagées (warpReduce
  sum/max, etc.).

### §12.2 Template d'un fichier `.cu`

```cpp
// cuda/activations/softmax.cu
//
// Numerically stable softmax (LogSumExp trick).
//
// Maps to ComputeBackend::softmax(x, axis).
// Match to CpuBackend output: rtol=1e-4, atol=1e-5.

#include "efe.cuh"
#include "efe_warp.cuh"

namespace {

// Compute max(x[i, :]) for each row i.
__device__ float row_max(const float* row, int len) {
    float m = -INFINITY;
    for (int i = 0; i < len; ++i) {
        m = fmaxf(m, row[i]);
    }
    return m;
}

}  // anon

// Each block processes one row of length `axis_size`.
// Threads in block cooperate via shared memory for the reduction.
__global__ void efe_softmax_rowwise_kernel(
    const float* __restrict__ input,
    float* __restrict__ output,
    int axis_size
) {
    extern __shared__ float smem[];

    int row = blockIdx.x;
    int tid = threadIdx.x;
    const float* row_in = input + row * axis_size;
    float* row_out = output + row * axis_size;

    // Phase 1: max reduction
    float local_max = -INFINITY;
    for (int i = tid; i < axis_size; i += blockDim.x) {
        local_max = fmaxf(local_max, row_in[i]);
    }
    smem[tid] = local_max;
    __syncthreads();
    // tree reduction
    for (int s = blockDim.x / 2; s > 0; s >>= 1) {
        if (tid < s) smem[tid] = fmaxf(smem[tid], smem[tid + s]);
        __syncthreads();
    }
    float max_val = smem[0];

    // Phase 2: sum of exp(x - max)
    float local_sum = 0.0f;
    for (int i = tid; i < axis_size; i += blockDim.x) {
        local_sum += expf(row_in[i] - max_val);
    }
    smem[tid] = local_sum;
    __syncthreads();
    for (int s = blockDim.x / 2; s > 0; s >>= 1) {
        if (tid < s) smem[tid] += smem[tid + s];
        __syncthreads();
    }
    float sum_val = smem[0];

    // Phase 3: write output
    for (int i = tid; i < axis_size; i += blockDim.x) {
        row_out[i] = expf(row_in[i] - max_val) / sum_val;
    }
}

extern "C" cudaError_t efe_softmax_launch(
    cudaStream_t stream,
    const float* input_dev, float* output_dev,
    int n_rows, int axis_size
) {
    int threads = std::min(axis_size, 256);
    // power-of-2 round-up for tree reduction
    threads = next_pow2(threads);
    dim3 block(threads);
    dim3 grid(n_rows);
    size_t smem = threads * sizeof(float);
    efe_softmax_rowwise_kernel<<<grid, block, smem, stream>>>(
        input_dev, output_dev, axis_size);
    return cudaGetLastError();
}
```

### §12.3 Décisions de design pour chaque kernel

Cf. **§28 (Annexes)** pour le détail kernel-by-kernel. Principes
généraux :

* **Block size** : multiple de 32 (warp) ; default 256 (un block par row pour ops rowwise).
* **Grid size** : pour les ops élémentwise, `(N + block - 1) / block`.
  Pour les ops rowwise, `n_rows`.
* **Shared memory** : utilisée pour les réductions tree (sum, max) ;
  taille = `threads * sizeof(T)`.
* **Coalesced memory access** : tous les accès `input[tid + i * stride]`
  sont alignés sur 128 octets (warp size × 4 bytes).
* **Avoid branch divergence** : pas de `if (tid % 2 == 0)` ; préférer
  `mask + select` pattern.
* **Use FMA** : `fmaf(a, b, c)` au lieu de `a * b + c` (1 instruction
  vs 2).

---

## §13. Sous-système 3 — Implémentation `CudaBackend`

### §13.1 Migration depuis `efe-core` (D29)

**Étape 1** (D29 J1) — copier le contenu de
`efe-core/src/compute/cuda/mod.rs` vers
`efe-kernels/src/backend.rs` (200 lignes).

**Étape 2** (D29 J1) — supprimer
`efe-core/src/compute/cuda/mod.rs` ; retirer le module
`pub mod cuda;` du `compute/mod.rs` ; supprimer les usages
`#[cfg(feature = "cuda")]` qui référencent `compute::cuda::CudaBackend`.

**Étape 3** (D29 J1) — re-router via `lib.rs` du crate :

```rust
// crates/efe-kernels/src/lib.rs
#[cfg(feature = "cuda")]
pub use crate::backend::CudaBackend;

// Ré-export du trait pour les utilisateurs (commodité).
pub use efe_core::compute::ComputeBackend;
```

**Étape 4** (D29 J1) — `efe-server` change son import :

```rust
// AVANT (D18)
use efe_core::compute::cuda::CudaBackend;

// APRÈS (D29)
#[cfg(feature = "cuda")]
use efe_kernels::CudaBackend;
```

**Étape 5** (D29 J2) — pin test `test_cuda_backend_lives_in_efe_kernels.rs`
qui vérifie via grep que `efe-core` ne contient plus aucune référence
à `CudaBackend`.

### §13.2 Squelette `backend.rs` (D29)

```rust
// crates/efe-kernels/src/backend.rs
use std::sync::Arc;
use cudarc::driver::{CudaDevice, CudaSlice};
use cudarc::cublas::CudaBlas;
use efe_core::compute::{ComputeBackend, Device, Tensor, ComputeError, Result};

use crate::streams::StreamPool;
use crate::memory::DeviceAllocator;
use crate::observability::record_kernel_call;

#[derive(Debug)]
pub struct CudaBackend {
    ordinal: u32,
    device: Arc<CudaDevice>,
    blas: Arc<CudaBlas>,
    streams: StreamPool,
    allocator: DeviceAllocator,
}

impl CudaBackend {
    pub fn new(ordinal: u32) -> Result<Self> {
        let device = CudaDevice::new(ordinal as usize)
            .map_err(|e| ComputeError::Backend(format!("CUDA init: {e}")))?;
        let blas = CudaBlas::new(device.clone())?;
        let streams = StreamPool::new(device.clone(), 4)?; // 4 streams
        let allocator = DeviceAllocator::new(device.clone());
        Ok(Self { ordinal, device, blas: Arc::new(blas), streams, allocator })
    }
}

impl ComputeBackend for CudaBackend {
    fn device(&self) -> Device { Device::Cuda(self.ordinal) }
    fn name(&self) -> &'static str { "cuda" }

    fn synchronize(&self) -> Result<()> {
        self.device.synchronize()
            .map_err(|e| ComputeError::Backend(format!("CUDA sync: {e}")))
    }

    fn matmul(&self, a: &Tensor<f32>, b: &Tensor<f32>) -> Result<Tensor<f32>> {
        // 1. Validate shapes
        // 2. Allocate output tensor (device)
        // 3. cuBLAS sgemm via self.blas
        // 4. Return tensor
    }

    fn softmax(&self, x: &Tensor<f32>, axis: i32) -> Result<Tensor<f32>> {
        let _t = record_kernel_call("softmax");
        // 1. Validate shapes + axis
        // 2. Get current stream
        // 3. Allocate output tensor
        // 4. Call kernel via crate::kernels::efe_softmax_launch
        // 5. Return
    }

    // ... 21 autres méthodes
}
```

### §13.3 Erreur taxonomie

```rust
// crates/efe-kernels/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum CudaError {
    #[error("CUDA driver error: code {0}")]
    FromCuda(i32),
    #[error("VRAM exhausted on device {0}: requested {1} bytes, available {2}")]
    OutOfMemory(u32, usize, usize),
    #[error("kernel launch failed: {0}")]
    LaunchFailed(String),
    #[error("NCCL error: {0}")]
    Nccl(String),
    #[error("device {0} not available")]
    DeviceUnavailable(u32),
    #[error("compute capability sm_{0} not supported (min sm_75)")]
    UnsupportedCapability(u32),
}

impl From<CudaError> for ComputeError {
    fn from(e: CudaError) -> Self {
        match e {
            CudaError::OutOfMemory(..) => ComputeError::ResourceExhausted(e.to_string()),
            other => ComputeError::Backend(other.to_string()),
        }
    }
}
```

---

## §14. Sous-système 4 — Gestion streams + mémoire device

### §14.1 `StreamPool` (D29)

```rust
// crates/efe-kernels/src/streams.rs
use cudarc::driver::CudaStream;

pub struct StreamPool {
    streams: Vec<CudaStream>,
    next_idx: AtomicUsize,
}

impl StreamPool {
    pub fn new(device: Arc<CudaDevice>, count: usize) -> Result<Self> {
        let streams = (0..count).map(|_| device.fork_default_stream()).collect::<Result<_>>()?;
        Ok(Self { streams, next_idx: AtomicUsize::new(0) })
    }

    /// Round-robin stream allocation. Used by hot path.
    pub fn next(&self) -> &CudaStream {
        let idx = self.next_idx.fetch_add(1, Ordering::Relaxed) % self.streams.len();
        &self.streams[idx]
    }

    pub fn synchronize_all(&self) -> Result<()> {
        for s in &self.streams { s.synchronize()?; }
        Ok(())
    }
}
```

* **4 streams par défaut** (cf. principe §5.1.7).
* Round-robin assignment ; l'app peut surcharger via env.
* Drop = synchronize (sécurité).

### §14.2 `DeviceAllocator` (D28)

```rust
// crates/efe-kernels/src/memory.rs
use cudarc::driver::{CudaDevice, CudaSlice};

pub struct DeviceAllocator {
    device: Arc<CudaDevice>,
    high_water_mark: AtomicUsize,
    threshold_bytes: usize,
}

impl DeviceAllocator {
    pub fn alloc<T: cudarc::types::DeviceRepr>(&self, len: usize) -> Result<CudaSlice<T>> {
        let bytes = len * std::mem::size_of::<T>();
        let used = self.high_water_mark.fetch_add(bytes, Ordering::Relaxed);
        if used + bytes > self.threshold_bytes {
            return Err(CudaError::OutOfMemory(self.device.ordinal() as u32, bytes, self.threshold_bytes - used).into());
        }
        let slice: CudaSlice<T> = self.device.alloc_zeros(len)?;
        tracing::debug!(target: "efe.cuda.alloc", bytes, "allocated");
        Ok(slice)
    }
}
```

* **High-water mark tracking** — refus si > 80 % VRAM.
* **`alloc_zeros`** — toujours initialisé à 0 pour déterminisme.
* **Tracing tags** — facilite le memleak hunting via grep.

### §14.3 Tensor extension : `to_device` / `to_host`

Le `Tensor<T>` actuel dans `efe-core` a un champ `device: Device`.
La conversion CPU ↔ CUDA est ajoutée :

```rust
// efe-kernels/src/tensor_ext.rs
use efe_core::compute::Tensor;

pub trait CudaTensorExt<T> {
    fn to_device(self, device: Device, allocator: &DeviceAllocator) -> Result<Tensor<T>>;
    fn to_host(self) -> Result<Tensor<T>>;
}

impl<T: cudarc::types::DeviceRepr> CudaTensorExt<T> for Tensor<T> {
    fn to_device(self, device: Device, alloc: &DeviceAllocator) -> Result<Tensor<T>> {
        match (self.device(), device) {
            (Device::Cpu, Device::Cuda(_)) => {
                let host_slice = self.as_slice()?;
                let mut dev_slice = alloc.alloc::<T>(host_slice.len())?;
                alloc.device.htod_copy_into(host_slice.to_vec(), &mut dev_slice)?;
                Ok(Tensor::from_device_slice(dev_slice, self.shape().clone()))
            }
            (Device::Cuda(_), Device::Cpu) => {
                // ... reverse
            }
            _ => Ok(self), // already on target
        }
    }
}
```

---

## §15. Sous-système 5 — Multi-GPU sharding (NCCL)

### §15.1 Architecture (D30)

Pour les essaims `N > 256` qui dépassent le budget mémoire d'un GPU
seul, l'ELS est partitionné en **K shards** (K = nombre de GPUs) :

```
ELS [N=1024 × D=512]
  │
  ├─ Shard 0: [256 × D] sur GPU 0
  ├─ Shard 1: [256 × D] sur GPU 1
  ├─ Shard 2: [256 × D] sur GPU 2
  └─ Shard 3: [256 × D] sur GPU 3

  Round k :
   1. Each GPU runs LOCAL fusion (attention + softmax + aggregate)
      on its shard → partial centroid contribution
   2. NCCL all-reduce: sum partial centroids across GPUs
   3. Each GPU broadcasts the global centroid
   4. Continue with mix + project
```

### §15.2 Module `multi_gpu.rs` (D30)

```rust
// crates/efe-kernels/src/multi_gpu.rs
use cudarc::nccl::{Comm, NcclType, ReduceOp};

pub struct ShardedBackend {
    backends: Vec<CudaBackend>,         // one per GPU
    nccl_comms: Vec<Comm>,              // NCCL communicators
}

impl ShardedBackend {
    pub fn new(ordinals: &[u32]) -> Result<Self> {
        let backends = ordinals.iter().map(|&o| CudaBackend::new(o)).collect::<Result<_>>()?;
        let nccl_comms = Comm::from_devices(...)?;
        Ok(Self { backends, nccl_comms })
    }

    pub fn all_reduce_centroid(
        &self,
        partial_centroids: &[CudaSlice<f32>],
    ) -> Result<Vec<CudaSlice<f32>>> {
        for (i, comm) in self.nccl_comms.iter().enumerate() {
            comm.all_reduce(&partial_centroids[i], &mut output[i], ReduceOp::Sum)?;
        }
        Ok(output)
    }
}
```

### §15.3 Fallback `tcp` si NCCL absent

NCCL est absent en dev (Mac, single-GPU CI). Fallback :

* `EFE_RUNTIME_GPU_COMMS=tcp` → simple TCP socket all-reduce.
* Latence ~10× pire que NCCL mais fonctionne en CI.
* Activé automatiquement si NCCL non détecté au boot.

### §15.4 Topology auto-detection

Au boot, `efe-server` interroge :

```rust
fn detect_gpu_topology() -> GpuTopology {
    let n = cudarc::driver::result::device::get_count().unwrap_or(0);
    let mut gpus = Vec::new();
    for i in 0..n {
        if let Ok(dev) = CudaDevice::new(i as usize) {
            let cc = dev.compute_capability();
            let vram = dev.total_memory();
            gpus.push(GpuInfo { ordinal: i as u32, cc, vram });
        }
    }
    GpuTopology { gpus, nccl_available: probe_nccl() }
}
```

* Log au boot : `efe.cuda.topology` avec liste GPUs + capacités.
* Décide single-GPU vs multi-GPU sharding selon nombre détecté.

### §15.5 Sharding policies

| Policy | Description | Trigger |
|--------|-------------|---------|
| `disabled` | Single-GPU only | `EFE_RUNTIME_GPU_SHARDING=off` |
| `auto` | Multi-GPU si N > N_threshold (default 256) | default |
| `forced` | Multi-GPU toujours, même pour smoke | `EFE_RUNTIME_GPU_SHARDING=forced` |

---

## §16. Sous-système 6 — Harnais d'équivalence numérique

### §16.1 Principe

Pour chaque op du trait, un test `tests/cross_backend_<op>.rs` :

```rust
// crates/efe-kernels/src/tests/cross_backend_softmax.rs
use efe_core::compute::*;
use efe_kernels::CudaBackend;

#[test]
fn cross_backend_softmax_basic() {
    let cpu = CpuBackend::new();
    let gpu = CudaBackend::new(0).expect("GPU 0 available");

    let input = Tensor::from_slice(&[1.0, 2.0, 3.0, 4.0, 5.0], shape![5]).unwrap();

    let cpu_out = cpu.softmax(&input, -1).unwrap();
    let gpu_out_dev = gpu.softmax(&input.to_device(gpu.device(), &gpu.alloc)?, -1)?;
    let gpu_out_host = gpu_out_dev.to_host()?;

    assert_close(cpu_out.as_slice()?, gpu_out_host.as_slice()?, 1e-4, 1e-5);
}

#[test]
fn cross_backend_softmax_stable_large_values() {
    // Test numerical stability with x = [1000, 1001, 1002]
    // ...
}

#[test]
fn cross_backend_softmax_property_sum_to_one() {
    use proptest::prelude::*;
    proptest!(|(input in any::<Vec<f32>>().prop_filter(...))| {
        let cpu = CpuBackend::new();
        let gpu = CudaBackend::new(0).unwrap();
        let t = Tensor::from_slice(&input, shape![input.len()]).unwrap();
        let cpu_out = cpu.softmax(&t, -1).unwrap();
        let gpu_out = gpu.softmax(&t.to_device(...), -1).unwrap().to_host().unwrap();
        // Both should sum to 1
        let cpu_sum: f32 = cpu_out.as_slice().unwrap().iter().sum();
        let gpu_sum: f32 = gpu_out.as_slice().unwrap().iter().sum();
        prop_assert!((cpu_sum - 1.0).abs() < 1e-5);
        prop_assert!((gpu_sum - 1.0).abs() < 1e-5);
    });
}
```

### §16.2 Tolérance définie

* **Default** : `rtol=1e-4, atol=1e-5` (cf. `docs/compute/tolerances.md`).
* **Per-op overrides** :
  * `matmul` : `rtol=1e-3` pour grandes matrices (accumulation FP32).
  * `softmax` : `rtol=1e-5` (numérically stable, doit être strict).
  * `multinomial_sample` : pas de tolérance numérique — comparer
    distribution histogramme via Kolmogorov-Smirnov.

### §16.3 Golden vectors

Les **22 golden vectors** chargés en tests CPU sont également
chargés en tests GPU. Test pin :

```rust
#[test]
fn gpu_load_all_22_goldens() {
    let goldens = load_golden_vectors();
    assert_eq!(goldens.len(), 22);
    for g in goldens {
        let cpu_out = run_op_cpu(&g);
        let gpu_out = run_op_gpu(&g);
        assert_close(&cpu_out, &gpu_out, g.rtol, g.atol);
    }
}
```

### §16.4 Property tests cross-backend

Pour chaque op simple (softmax, l2_normalize, etc.) :

```rust
proptest! {
    #[test]
    fn cross_backend_propty_softmax(input in arb_tensor_1d(64)) {
        let cpu = CpuBackend::new();
        let gpu = CudaBackend::new(0).unwrap();
        let cpu_out = cpu.softmax(&input, -1).unwrap();
        let gpu_out = gpu.softmax(&input.to_device(...), -1).unwrap().to_host().unwrap();
        prop_assert!(close(cpu_out, gpu_out, 1e-4, 1e-5));
    }
}
```

* **23 ops × ~5 propriétés × 100 cas = 11 500 tests cross-backend**.

---

## §17. Sous-système 7 — Budget perf + tile sizes par GPU type

### §17.1 Budget end-to-end pour SLO C-1 (≤ 200 ms p99)

| Étape | Budget | GPU T4 | GPU A10 |
|-------|:------:|:------:|:-------:|
| Embedder ONNX CUDA | 30 ms | OK | OK |
| Genesis (alloc + spawn) | 5 ms | OK | OK |
| Fusion 10 rounds | 100 ms (10 ms/round) | T4: 10ms | A10: 4ms |
| Consensus + Dissolution | 2 ms | OK | OK |
| Alignment | 5 ms | OK | OK |
| gRPC + serialization | 50 ms | OK | OK |
| **Total p99 cible** | **192 ms** | ≤ 200 ms | ≤ 100 ms |

### §17.2 Tile sizes optimaux par GPU

```rust
// crates/efe-kernels/src/tuning.rs
pub fn tile_size_for_gpu(cc: u32) -> TileSizes {
    match cc {
        75 => TileSizes { matmul: 64, softmax: 256, attention: 128 },  // T4
        80 => TileSizes { matmul: 128, softmax: 256, attention: 256 }, // A100
        86 => TileSizes { matmul: 128, softmax: 256, attention: 256 }, // RTX
        89 => TileSizes { matmul: 256, softmax: 512, attention: 512 }, // L4
        90 => TileSizes { matmul: 256, softmax: 1024, attention: 1024 }, // H100
        _ => TileSizes::default(),
    }
}
```

* **Auto-tuning runtime** : différé post-GA (cf. §5.2).
* **Override** via env `EFE_GPU_TILE_<OP>=<size>` (debug uniquement).

### §17.3 Profiling driven optimizations (D31)

Phase D31 utilise `nvprof` / `nsight-compute` :

* **Identifier kernels bottleneck** — typiquement `softmax` + `entropy`
  car réductions tree.
* **Fuse opérations** : `softmax + entropy` → 1 kernel custom partagé.
* **Optimize tile sizes** par GPU type.
* **Reduce launch overhead** : batch les launches via CUDA Graph (post-GA).

### §17.4 Bench microsecond per op

| Op | T4 (µs) | A10 (µs) | A100 (µs) |
|----|:------:|:-------:|:--------:|
| `matmul (256x256)` | ~50 | ~20 | ~15 |
| `softmax (256x256)` | ~30 | ~15 | ~10 |
| `cone_project (256x512)` | ~80 | ~30 | ~20 |
| `weighted_mean (256x512)` | ~25 | ~12 | ~8 |
| `scaled_dot_product_attention (B=1, H=4, T=64, d=128)` | ~120 | ~50 | ~30 |

**Cibles approximatives** ; ajustées en D31 après profiling réel.

---

## §18. Sous-système 8 — Observabilité (6 instruments Prometheus)

### §18.1 Instruments à ajouter (D29)

Cf. `crates/efe-telemetry/src/metrics/gpu_instruments.rs`
(NOUVEAU — D29, ~150 LOC sur le pattern `d18_instruments.rs`) :

| # | Métrique | Type | Labels | Buckets |
|--:|----------|------|--------|---------|
| 1 | `efe_gpu_kernel_duration_seconds` | Histogram | `kernel, device` | exponentiel 1µs → 100ms |
| 2 | `efe_gpu_memory_used_bytes` | Gauge | `device` | n/a |
| 3 | `efe_gpu_utilization_ratio` | Gauge | `device` | n/a |
| 4 | `efe_gpu_kernel_calls_total` | Counter | `kernel, outcome` | n/a |
| 5 | `efe_gpu_oom_total` | Counter | `device` | n/a |
| 6 | `efe_gpu_nccl_allreduce_seconds` | Histogram | (no labels) | exponentiel 100µs → 100ms |

### §18.2 Cardinalité

* `kernel` ∈ {23 ops + helper kernels} ≤ 30.
* `device` ∈ {0..7} ≤ 8.
* `outcome` ∈ {ok, fail, oom} = 3.

→ Cardinalité totale ≤ 30 × 8 × 3 + 8 + 8 + 30 × 8 = ~ 1 000 séries.
Sous le budget D13 50 000 ✅.

### §18.3 Dashboard Grafana D29

Dashboard dédié `EFE GPU Runtime` :

* Panel 1 : GPU utilization par device (gauge).
* Panel 2 : Memory used / total par device (stack chart).
* Panel 3 : Kernel duration histogram p50/p95/p99 par kernel.
* Panel 4 : Calls / sec par kernel + outcome.
* Panel 5 : OOM events alarm.
* Panel 6 : NCCL all-reduce latency p99.

### §18.4 Alerts (D29)

| Alert | Threshold | Sévérité |
|-------|-----------|:--------:|
| `D18GpuOom` | `rate(efe_gpu_oom_total) > 0` for 1 min | Critical |
| `D18GpuKernelLatencyHigh` | `histogram_quantile(0.99, ...) > 50ms` for 5 min | Warning |
| `D18GpuMemoryHigh` | `efe_gpu_memory_used_bytes / total > 0.9` for 10 min | Warning |
| `D18NcclLatencyHigh` | `histogram_quantile(0.99, ...nccl...) > 10ms` for 5 min | Warning |

---

## §19. Sous-système 9 — Fallback + auto-détection

### §19.1 Détection au boot (D29)

```rust
// efe-server boot path
async fn build_compute_backend(cfg: &ServerConfig) -> Box<dyn ComputeBackend> {
    let want_gpu = cfg.runtime.gpu_enabled;

    if !want_gpu {
        tracing::info!("GPU disabled by config — using CPU");
        return Box::new(CpuBackend::new());
    }

    #[cfg(feature = "cuda")]
    {
        match CudaBackend::new(0) {
            Ok(gpu) => {
                tracing::info!(ordinal = 0, "GPU initialized");
                return Box::new(gpu);
            }
            Err(e) => {
                tracing::warn!(error = %e, "GPU init failed — falling back to CPU");
                return Box::new(CpuBackend::new());
            }
        }
    }
    #[cfg(not(feature = "cuda"))]
    {
        tracing::info!("CUDA feature not compiled — using CPU");
        Box::new(CpuBackend::new())
    }
}
```

### §19.2 Fallback en cours de runtime

Si une op CUDA échoue (OOM transitoire, kernel error) :

* **Option 1 — strict** (default) : retourne `Err(ResourceExhausted)`,
  caller (RPC handler) répond 503.
* **Option 2 — soft fallback** (`EFE_RUNTIME_GPU_FALLBACK=cpu_on_error`) :
  rejoue l'op sur `CpuBackend` ; log warn.

### §19.3 Hybrid fusion (D32 cross-cutting)

Cf. `PRODUCT_COMPLETION_ARCHITECTURE.md` §13.7. Le `HybridFusion`
trait wrappe deux `ComputeBackend` :

```rust
pub struct HybridBackend {
    primary: Box<dyn ComputeBackend>,   // CudaBackend
    shadow: Option<Box<dyn ComputeBackend>>,  // CpuBackend (validation)
    strategy: HybridStrategy,           // RunPrimary | RunBoth | EnsembleMean
}
```

Use cases :

* **Canary GPU kernels** : `RunBoth`, alert si divergence > 1e-3.
* **Embedded fallback** : `RunPrimary` ; si échec → ré-exec `shadow`.
* **Ensemble** : `RunBoth` + moyenne pondérée (réduit variance).

---

## §20. Déterminisme + reproductibilité sur GPU

### §20.1 Déterminisme single-GPU avec seed fixé

Le déterminisme est **plus difficile sur GPU** que CPU à cause de :

* **Réductions atomiques** non-déterministes (`atomicAdd` sur float).
* **Ordre des warps** non garanti.
* **Choix de tensor core path** (cuBLAS) variable.

### §20.2 Stratégies pour atteindre le determinism

1. **Pas d'atomic ops sur f32** dans nos kernels custom. Toutes les
   réductions passent par tree-reduce en shared memory (déterministe).
2. **`cublasSgemm` en mode déterministe** : `cublasSetMathMode(handle,
   CUBLAS_PEDANTIC_MATH)` — évite les Tensor Cores qui peuvent réordonner.
3. **`cuBLAS workspace fixe**` — `cublasSetWorkspace` avec un buffer
   pré-alloué de taille fixe.
4. **Seed du `cuRAND`** explicite.
5. **Single-stream** pour les ops déterministes (pas de race entre streams).

### §20.3 Pin test

```rust
#[test]
fn gpu_softmax_deterministic_seed_42() {
    let gpu = CudaBackend::new(0).unwrap();
    let input = make_tensor_seeded(42, &[256, 512]);
    let mut outputs = Vec::new();
    for _ in 0..100 {
        outputs.push(gpu.softmax(&input, -1).unwrap().to_host().unwrap());
    }
    // All 100 outputs MUST be bit-identical.
    let first = outputs[0].as_slice().unwrap();
    for out in &outputs[1..] {
        let other = out.as_slice().unwrap();
        assert_eq!(first, other, "GPU softmax not deterministic!");
    }
}
```

### §20.4 Multi-GPU determinism

Multi-GPU + NCCL est **plus difficile encore** :

* Ordre d'arrivée des messages NCCL non garanti.
* Mitigation : `NCCL_DETERMINISTIC=1` env var.
* Vérification : pin test multi-GPU avec seed fixé sur 4 shards.

### §20.5 Tolérance hybride CPU vs GPU

Même avec déterminisme strict, CPU vs GPU peut différer à cause :

* Ordre de sommation float (associativity loss).
* `expf` CPU vs GPU (libm vs CUDA math) à 1-2 ULP.

Tolérance documentée : `rtol=1e-4, atol=1e-5`. Au-delà → divergence
réelle = bug à corriger.

---

## §21. Sûreté numérique (NaN/Inf, OOM, overflow, deadlock)

### §21.1 NaN/Inf detection

Chaque kernel doit détecter NaN/Inf en sortie :

```cpp
__global__ void check_finite_kernel(const float* x, int n, int* found) {
    int tid = blockIdx.x * blockDim.x + threadIdx.x;
    if (tid < n && !isfinite(x[tid])) {
        atomicAdd(found, 1);
    }
}
```

Appelé en **debug mode** (`EFE_RUNTIME_GPU_CHECK_FINITE=true`)
après chaque kernel hot. En production : désactivé pour perf.

Si `found > 0` → `ComputeError::NumericalIssue { op, kind: NaN/Inf }`.

### §21.2 OOM gracieux

Cf. `DeviceAllocator::alloc` §14.2. Refus propre via
`CudaError::OutOfMemory` → `ComputeError::ResourceExhausted` →
RPC handler retourne `Status::ResourceExhausted` (gRPC code 8).

### §21.3 Overflow detection

* Réductions sum + : Kahan compensation déjà en CPU ; on porte en GPU
  via `kahan_sum_warp` helper.
* `expf` overflow : softmax max-shift suffit.
* `matmul` overflow : pour matrices grandes, on accumule en `double`
  intermédiaire (`cublasSgemmEx` mode) — option déterministe.

### §21.4 Deadlock prevention

Multi-GPU NCCL peut bloquer si un GPU plante en cours d'all-reduce :

* **Timeout** sur chaque NCCL op (`tokio::time::timeout` en Rust côté
  appelant).
* **Watchdog thread** qui kill le process si un kernel dure > 10× p99
  expected.
* **Health probe** GPU : `cudaDeviceSynchronize` au boot + nominal RPS.

### §21.5 GPU hang recovery

Si un kernel hang (deadlock memory ou infinite loop) :

* `cudaDeviceReset()` sur le device → tous les contextes libérés.
* Marquer le pod K8s comme unhealthy → restart automatique via probe.

---

## §22. Taxonomie des erreurs

### §22.1 `CudaError` (interne au crate)

```rust
pub enum CudaError {
    FromCuda(i32),                    // direct CUDA error code
    OutOfMemory(u32, usize, usize),   // device, requested, available
    LaunchFailed(String),             // kernel launch failed
    Nccl(String),                     // NCCL error
    DeviceUnavailable(u32),           // ordinal not present
    UnsupportedCapability(u32),       // sm_XX < 75
    DeterminismViolation(String),     // multi-GPU NCCL non-deterministic mode
    StreamError(String),              // stream creation/destroy
    KernelTimeout(String),            // watchdog killed kernel
}
```

### §22.2 Mapping vers `ComputeError`

```rust
impl From<CudaError> for ComputeError {
    fn from(e: CudaError) -> Self {
        match e {
            CudaError::OutOfMemory(_, _, _) =>
                ComputeError::ResourceExhausted(e.to_string()),
            CudaError::DeviceUnavailable(_) | CudaError::UnsupportedCapability(_) =>
                ComputeError::BackendUnavailable(e.to_string()),
            other =>
                ComputeError::Backend(other.to_string()),
        }
    }
}
```

### §22.3 Mapping gRPC

Cf. `efe-server::services::swarm` :

| Erreur Rust | gRPC Code |
|-------------|-----------|
| `ComputeError::ResourceExhausted` | `RESOURCE_EXHAUSTED` (8) |
| `ComputeError::BackendUnavailable` | `UNAVAILABLE` (14) |
| `ComputeError::ShapeMismatch` | `INVALID_ARGUMENT` (3) |
| `ComputeError::NumericalIssue` | `INTERNAL` (13) |
| `ComputeError::NotImplemented` | `UNIMPLEMENTED` (12) |
| `ComputeError::Backend` (autre) | `INTERNAL` (13) |

---

## §23. Stratégie de tests

### §23.1 Pyramide de tests

```
                       ┌──────────────────┐
                       │  Soak 24h GPU    │   ← Nightly, post-D31
                       │  (1 test)        │
                       └──────────────────┘
                    ┌────────────────────────┐
                    │   E2E swarm round      │   ← bench harness, D31
                    │   (5 tests)            │
                    └────────────────────────┘
                ┌────────────────────────────────┐
                │   Cross-backend per op         │   ← D28, 23 fichiers
                │   (23 × ~10 = 230 tests)       │
                └────────────────────────────────┘
            ┌────────────────────────────────────────┐
            │   Property tests (proptest)            │   ← D28
            │   (~20 propriétés × 100 cas = 2000)    │
            └────────────────────────────────────────┘
        ┌────────────────────────────────────────────────┐
        │   Unit tests par kernel                        │   ← D28
        │   (~3 tests × 23 ops = 69 tests)               │
        └────────────────────────────────────────────────┘
    ┌────────────────────────────────────────────────────────┐
    │   Smoke build                                          │   ← D28
    │   (cargo check sans + avec --features cuda)            │
    └────────────────────────────────────────────────────────┘
```

**Total cible** : ~2 500 tests GPU + 1 260 CPU déjà livrés ≈ **3 800
tests** au total post-D32.

### §23.2 `cuda-memcheck` integration

Workflow CI nightly :

```yaml
# .github/workflows/gpu-memcheck.yml
- run: |
    cuda-memcheck --tool memcheck \
      cargo test -p efe-kernels --features cuda --release \
        cross_backend_smoke
```

Tout leak / out-of-bounds → CI RED.

### §23.3 Soak 24h

Test marqué `#[ignore]` lancé via cron nightly :

```rust
#[test]
#[ignore]
fn soak_24h_gpu_no_memleak() {
    let gpu = CudaBackend::new(0).unwrap();
    let start = Instant::now();
    let mut iter = 0u64;
    while start.elapsed() < Duration::from_secs(24 * 3600) {
        // Run 1 swarm round
        run_one_round(&gpu).unwrap();
        iter += 1;

        if iter % 10000 == 0 {
            // Probe RSS via /proc/self/statm
            let rss = read_self_rss();
            tracing::info!(iter, rss_mb = rss / 1024 / 1024, "soak progress");
        }
    }
}
```

### §23.4 GPU runner CI matrix

| Workflow | Trigger | Runner | Tests |
|----------|---------|--------|-------|
| `test-cpu` | Every PR | `ubuntu-22.04` | 1260+ CPU tests |
| `test-gpu-smoke` | Every PR with `cuda` label | `[self-hosted, gpu, t4]` | 100 cross-backend tests (smoke) |
| `test-gpu-full` | Daily | `[self-hosted, gpu, t4]` | All 2 500 GPU tests |
| `test-multi-gpu` | Weekly | `[self-hosted, gpu, multi-a10]` | NCCL tests |
| `cuda-memcheck` | Nightly | `[self-hosted, gpu, t4]` | Memcheck on smoke subset |
| `soak-24h` | Cron weekly | `[self-hosted, gpu, t4]` | 24h non-stop |
| `bench-gpu` | On release tag | `[self-hosted, gpu, a10]` | Criterion bench publish |

---

## §24. Matrice CI (runners GPU + cross-platform)

### §24.1 Self-hosted GPU runners requis

| Runner label | Specs | Coût mensuel | Use case |
|--------------|-------|:------------:|----------|
| `gpu, t4` | 4 vCPU + 16 GB RAM + T4 16 GB | ~120 €/mois (cloud) | smoke + memcheck |
| `gpu, a10` | 8 vCPU + 32 GB RAM + A10 24 GB | ~250 €/mois | bench + soak |
| `gpu, multi-a10` | 16 vCPU + 64 GB RAM + 4× A10 | ~900 €/mois | NCCL tests |

**Total** : ~1 270 €/mois pour la matrix complète. Réduisable à 350 €
en supprimant multi-GPU (test moins fréquent — passer à 1×/semaine
sur cloud spot).

### §24.2 Alternative low-cost : Lambda Labs

* T4 spot : ~0.50 €/h × 50 h/mois CI = 25 €/mois
* A10 spot : ~0.80 €/h × 30 h/mois = 24 €/mois
* 4×A100 (one-shot soak/bench) : ~5 €/h × 4 h/mois = 20 €/mois

**Total** : ~70 €/mois si CI strict (test only on tags + nightly).
Cf. `compute/ROADMAP.md` Phase 2-3 budget = ~335 € one-shot pour
le portage.

### §24.3 GitHub Actions self-hosted setup

* Runner agent installé sur VM cloud avec NVIDIA driver + CUDA toolkit.
* Auto-scaling : un runner ne tourne que pendant le test, puis se shutdown.
* Secrets : Github token + cloud creds.
* Monitoring : GHA queue length alert si > 10 jobs en attente.

---

## §25. Migration path

### §25.1 Phase 1 — D29 setup migration

Ordre des commits :

1. **C1** : ajout `efe-kernels/Cargo.toml` deps `cudarc`, `cc`, `nccl-rs`.
2. **C2** : ajout `build.rs` minimaliste (détection nvcc only).
3. **C3** : copie `efe-core/src/compute/cuda/mod.rs` →
   `efe-kernels/src/backend.rs`.
4. **C4** : suppression du module `cuda` de `efe-core/src/compute/`.
5. **C5** : update `efe-server` import : `use efe_kernels::CudaBackend`.
6. **C6** : pin test `test_cuda_backend_lives_in_efe_kernels.rs`.
7. **C7** : `cargo build --workspace --features cuda` GREEN.

### §25.2 Phase 2 — D29-D30 implémentation

* C8-C30 : impls op-par-op (~ 23 commits).
* Chaque commit ajoute :
  * Le `.cu` source.
  * Le binding Rust (`extern "C"`).
  * L'impl dans `CudaBackend::<op>`.
  * Le test cross-backend.
* Critère commit : test cross-backend GREEN sur GPU runner.

### §25.3 Phase 3 — D31-D32 hardening

* Optimisations profiling-driven.
* Streaming activation.
* Hybrid fusion.
* Memory leak hunt + soak.

### §25.4 Phase 4 — D32 freeze

* Pin tests régression pour les 23 ops.
* Documentation finale (rustdoc + `docs/efe-kernels.md`).
* Audit chirurgical (pattern projet).

---

## §26. Plan d'implémentation détaillé sprint-par-sprint

### §26.1 Sprint **D28** — CUDA kernels MVP

**Durée** : 10 jours ouvrés.
**Objectif** : 18 ops du trait impl en CUDA + tests cross-backend GREEN.

| Jour | Livrable | Owner |
|:----:|----------|-------|
| J1 | `Cargo.toml` deps, `build.rs` minimal, `efe.cuh` étendu, `CudaBackend::new` + `synchronize` | Core |
| J2 | `mean`, `std_dev`, `softmax`, `log_softmax` (4 ops simples reduce) | Core |
| J3 | `matmul` (cuBLAS), `dot`, `l2_norm` (3 ops cuBLAS) | Core |
| J4 | `gelu`, `sigmoid`, `argmax`, `topk` (4 ops simples elementwise) | Core |
| J5 | `weighted_mean`, `cone_project` (2 EFE-core) | Core |
| J6 | `l2_normalize`, `l2_distance`, `shannon_entropy` (3 norm/info) | Core |
| J7 | `scaled_dot_product_attention` (composé) + `cosine_similarity_pair` + `cosine_anchor_blend` (3) | Core |
| J8 | Tests cross-backend pour les 18 ops (~ 200 tests) | Core |
| J9 | Property tests `proptest` cross-backend (15 props × 100 = 1500 tests) | Core |
| J10 | Cross-platform build sans CUDA, CI workflow `test-gpu-smoke` | Core |

**Critères Go/No-Go D28** :
* GG-D28.1 : 18 ops impl GREEN sur T4 runner.
* GG-D28.2 : Cross-backend tolerance respectée.
* GG-D28.3 : `cargo build --workspace` (sans CUDA) GREEN sur Mac.

### §26.2 Sprint **D29** — Migration + 5 ops restantes + CI

**Durée** : 10 jours.
**Objectif** : `CudaBackend` migré dans `efe-kernels` ; 5 ops restantes ; CI complet.

| Jour | Livrable |
|:----:|----------|
| J1 | Migration `CudaBackend` `efe-core` → `efe-kernels` (cf. §25.1) |
| J2 | Pin test migration + builder selection dans `efe-server::mode1::build_backend` |
| J3 | `kl_divergence`, `multinomial_sample` (2 ops Mode-2/sampling) |
| J4 | `moe_reconcile`, `cosine_anchor_blend` final + `argmax` polish |
| J5 | Module `streams.rs` (StreamPool 4 streams) + `memory.rs` (DeviceAllocator) |
| J6 | Module `fallback.rs` (auto-detect + boot fallback CPU) |
| J7 | `gpu_instruments.rs` (6 Prom metrics) + dashboard Grafana D29 |
| J8 | Tests integration dans `efe-server` : `tests/d29_cuda_fusion_integration.rs` |
| J9 | Bench `efe-bench --runtime gpu --target-env staging` GREEN |
| J10 | CI matrix complète (`test-gpu-smoke` + `test-gpu-full` + `cuda-memcheck`) |

**Critères Go/No-Go D29** :
* GG-D29.1 : `CudaBackend` 23/23 ops GREEN.
* GG-D29.2 : Speedup CPU → GPU ≥ 5× sur p99 mesuré.
* GG-D29.3 : Auto-fallback CPU si GPU absent fonctionne.
* GG-D29.4 : 6 Prom GPU metrics exposées + dashboard live.

### §26.3 Sprint **D30** — Multi-GPU sharding (NCCL)

**Durée** : 10 jours.
**Objectif** : Sharding 2-4 GPUs avec speedup ≥ 3× pour N=1024.

| Jour | Livrable |
|:----:|----------|
| J1-J2 | `multi_gpu.rs` (`ShardedBackend`, NCCL comm setup) |
| J3 | Implémentation all-reduce des centroïdes partiels |
| J4 | TCP fallback si NCCL absent |
| J5 | Topology auto-detection au boot |
| J6-J7 | Tests integration multi-GPU (force sharding sur 1 GPU pour test logical) |
| J8 | Test e2e 4 GPUs A10 (cloud Lambda Labs) |
| J9 | Bench `efe-bench --gpu-shards 4 --workload soak` |
| J10 | Audit NCCL deadlock prevention + watchdog |

**Critères Go/No-Go D30** :
* GG-D30.1 : Multi-GPU 4× shard speedup ≥ 3× sur N=1024.
* GG-D30.2 : NCCL all-reduce p99 ≤ 5 ms.
* GG-D30.3 : Auto-fallback single-GPU si NCCL fail.

### §26.4 Sprint **D31** — Optimisations + perf budget

**Durée** : 10 jours.
**Objectif** : Profiling-driven optimisations + atteindre SLO C-1.

| Jour | Livrable |
|:----:|----------|
| J1-J2 | Profiling avec `nsight-compute` ; identifier top-3 bottleneck kernels |
| J3-J4 | Fuse `softmax + entropy` en 1 kernel custom |
| J5 | Optimiser tile sizes par GPU type (T4 vs A10) |
| J6 | Optimiser `cone_project` (shared memory + warp primitives) |
| J7 | Bench micro per kernel (criterion) — publication HTML |
| J8 | `cuda-memcheck` rapport vide ; soak 24h GREEN |
| J9 | Subcommand `efe-server tune` (auto-suggest env vars) |
| J10 | Audit chirurgical D31 + pin tests régression |

**Critères Go/No-Go D31** :
* GG-D31.1 : Mode-1 GPU p99 ≤ 200 ms (SLO C-1) sur charge 200 RPS, T4.
* GG-D31.2 : 24h soak GPU sans memleak/divergence.
* GG-D31.3 : Speedup ≥ 5× confirmé sur bench public.

### §26.5 Sprint **D32** — Streaming + Hybrid fusion

**Durée** : 10 jours.
**Objectif** : Activer streaming RPC + Hybrid CPU/GPU.

| Jour | Livrable |
|:----:|----------|
| J1-J2 | Activation `Mode1Service.ExecuteStreaming` côté server (déjà stubbé) |
| J3-J4 | Tests streaming GPU (chunks par round) |
| J5-J6 | `HybridFusion` trait + impl `RunBoth` |
| J7 | Tests hybrid CPU shadow + GPU primary |
| J8 | Tests hybrid embedded fallback (GPU OOM → re-exec CPU) |
| J9 | Documentation `docs/efe-kernels.md` complète + rustdoc 100 % |
| J10 | Audit chirurgical D32 + freeze v0.6.0 |

**Critères Go/No-Go D32** :
* GG-D32.1 : Streaming RPC e2e GREEN (chunks live arrivent client).
* GG-D32.2 : Hybrid fusion shadow detection des divergences > 1e-3.
* GG-D32.3 : Hybrid fallback OOM re-exec CPU sans drop request.
* GG-D32.4 : v0.6.0 tag + CHANGELOG.

### §26.6 Vue agrégée

| Sprint | Effort jours-h | Livrable | Dépendance amont |
|:------:|:--:|:--------:|------------------|
| D28 | 10 | 18 ops + build pipeline | aucune |
| D29 | 10 | 5 ops restantes + migration + CI | D28 |
| D30 | 10 | Multi-GPU NCCL | D29 |
| D31 | 10 | Optim + perf budget | D29-D30 |
| D32 | 10 | Streaming + Hybrid | D29 |

**Total** : 50 jours-h (5 semaines × 1-2 dev) ≈ Φ-D entière.

---

## §27. Risques + atténuations

### §27.1 Top-10 risques

| # | Risque | Probabilité | Impact | Mitigation |
|--:|--------|:----------:|:------:|------------|
| K-R-1 | Speedup < 5× → SLO C-1 raté | Médium | Critique | Buffer 2 sprints D31 + fallback ONNX RT CUDA EP en option d'urgence |
| K-R-2 | Cross-backend tolerance échoue (NaN/divergence) | Médium | Critique | Tolérance documentée stricte ; pin tests sur 22 goldens ; debug mode `EFE_RUNTIME_GPU_CHECK_FINITE` |
| K-R-3 | NCCL deadlock multi-GPU prod | Faible | Critique | `NCCL_DETERMINISTIC=1` + watchdog timeout + DR drill multi-GPU |
| K-R-4 | Memleak GPU RSS croissant en soak | Médium | Hard | `cuda-memcheck` CI nightly + soak 24h hebdo + DeviceAllocator high-water-mark |
| K-R-5 | Build CUDA cassé sur ARM64 (Jetson) | Faible | Médium | CI matrix arm64 (post-GA optional) ; doc claire requirements |
| K-R-6 | `cudarc` 0.10 → 0.11 break ABI | Médium | Médium | Pin minor version ; renouvellement contrôlé via ADR |
| K-R-7 | `nvcc` toolchain absent en runner CI | Faible | Hard | Image Docker pré-baked avec CUDA 12.4 ; fallback Lambda Labs runner |
| K-R-8 | GPU runner cloud spot evicted | Médium | Médium | Spot fallback to on-demand ; tests retriable |
| K-R-9 | Hybrid CPU shadow trop coûteux | Médium | Médium | Sample 1 % seulement ; opt-in via env |
| K-R-10 | Determinism perdu sur multi-GPU | Médium | Critique | `NCCL_DETERMINISTIC=1` + pin test seed-fixé sur 4 shards |

### §27.2 Risque budgétaire cloud GPU

* **CI strict** = ~70 €/mois (Lambda Labs spot).
* **CI tolérant** = ~1 270 €/mois (cloud always-on).
* **DR drill quarterly multi-GPU** = ~25 € one-shot.
* **Soak 24h hebdo** = ~5 € × 4 = 20 €/mois.

Total budget cloud GPU CI **estimation** : 100-1 300 €/mois.
Décision : démarrer avec strict (70 €/mois), monter si besoin.

---

## §28. Annexes — spec opération-par-opération

### §28.1 Format d'une fiche kernel

Pour chaque op du trait, fiche structurée :

* **Op** : nom Rust dans le trait
* **Fichier** : path absolu du `.cu`
* **Signature CUDA** : prototype C
* **Algorithme** : description haute niveau
* **Tile size** : par GPU type
* **Complexité** : `O(...)`
* **Dépendances cuBLAS / cuRAND** : oui/non
* **Test** : path du test cross-backend
* **Tolérance** : rtol / atol
* **Sprint** : D28 ou D29

### §28.2 Fiches détaillées (P1 — hot path)

#### `softmax`

* **Op** : `softmax(x, axis)`
* **Fichier** : `cuda/activations/softmax.cu`
* **Signature CUDA** : `efe_softmax_launch(stream, in, out, n_rows, axis_size)`
* **Algorithme** : LogSumExp 3-phases (max-shift + exp + sum + divide).
  Tree reduction in shared memory.
* **Tile size** : 256 (T4), 256 (A10), 512 (L4), 1024 (H100)
* **Complexité** : `O(N)` — 3 passes mémoire.
* **Dépendances** : non (custom)
* **Test** : `tests/cross_backend_softmax.rs`
* **Tolérance** : rtol=1e-5, atol=1e-6 (numerically stable)
* **Sprint** : D28

#### `scaled_dot_product_attention`

* **Op** : `scaled_dot_product_attention(q, k, v, mask)`
* **Fichier** : `cuda/attention/scaled_dot_product_attention.cu`
* **Signature CUDA** : `efe_sdpa_launch(stream, q, k, v, mask, out, b, h, t, d)`
* **Algorithme** :
  1. `scores = q @ kᵀ / √d_k` (cuBLAS sgemm batched)
  2. `scores += mask` (custom add kernel)
  3. `weights = softmax(scores, -1)` (réutilise softmax kernel)
  4. `out = weights @ v` (cuBLAS sgemm batched)
* **Tile size** : 128 (T4), 256 (A10/A100), 512 (H100)
* **Complexité** : `O(B × H × T² × d)` — quadratic in T.
* **Dépendances** : cuBLAS sgemm batched
* **Test** : `tests/cross_backend_sdpa.rs`
* **Tolérance** : rtol=1e-3 (matmul accumulation)
* **Sprint** : D28
* **Optimization D31** : flash-attention 2 pour T grand (V100+).

#### `cone_project`

* **Op** : `cone_project(latents, anchors, max_dist)`
* **Fichier** : `cuda/efe_core/cone_project.cu`
* **Signature CUDA** : `efe_cone_project_launch(stream, latents, anchors, out, b, n, d, max_dist)`
* **Algorithme** :
  1. Pour chaque latent `i` (1 thread block) :
     a. Compute distances `d_j = ||latent[i] - anchors[j]||₂` for all j (warp loop)
     b. `j* = argmin_j d_j` (warp reduction)
     c. If `d_j* ≤ max_dist` : copy latent[i] → out[i]
     d. Else : project `out[i] = anchors[j*] + (latent[i] - anchors[j*]) × max_dist / d_j*`
* **Tile size** : 1 block per latent ; 32 threads per block
* **Complexité** : `O(B × N × D)`
* **Dépendances** : non (custom)
* **Test** : `tests/cross_backend_cone_project.rs`
* **Tolérance** : rtol=1e-5
* **Sprint** : D28

#### `weighted_mean`

* **Op** : `weighted_mean(values, weights, renormalize_l2)`
* **Fichier** : `cuda/efe_core/weighted_mean.cu`
* **Signature CUDA** : `efe_weighted_mean_launch(stream, values, weights, out, b, d, renorm)`
* **Algorithme** :
  1. Normalize weights : `wn = w / sum(w)` (1 reduction warp).
  2. Compute weighted sum : `out = Σ_i wn[i] × values[i]` (1 kernel matrix-vec).
  3. If `renorm` : divide by L2 norm.
* **Tile size** : 256
* **Complexité** : `O(B × D)`
* **Dépendances** : non (custom) ; potentielle cuBLAS gemv
* **Test** : `tests/cross_backend_weighted_mean.rs`
* **Tolérance** : rtol=1e-5
* **Sprint** : D28
* **Note** : Kahan compensated sum côté CPU. Côté GPU, FMA suffit.

#### `mean`, `std_dev`, `shannon_entropy`

Fiches similaires : tree-reduce en shared memory, 1 block par axis,
Welford pour `std_dev` numerically stable.

### §28.3 Fiches détaillées (P2 — support)

#### `matmul`

* **Op** : `matmul(a, b)`
* **Fichier** : `cuda/linalg/matmul.cu`
* **Signature CUDA** : N/A — utilise directement `cublasSgemm` via cudarc.
* **Algorithme** : `cublasSgemm` en mode `CUBLAS_PEDANTIC_MATH` pour
  déterminisme.
* **Complexité** : `O(MNK)` ; cuBLAS atteint ~80 % du peak.
* **Test** : `tests/cross_backend_matmul.rs` ; tolérance rtol=1e-3 (large
  matrices).

#### `dot`, `l2_norm` : `cublasSdot`, `cublasSnrm2`

#### `matmul_fused_gelu` : `cublasLtMatmul` avec epilog GELU

#### `gelu`, `sigmoid` : custom elementwise kernels (1 thread per element)

### §28.4 Fiches détaillées (P3 — cold path)

#### `multinomial_sample`

* **Op** : `multinomial_sample(probs, n, seed)`
* **Fichier** : `cuda/sampling/multinomial_sample.cu`
* **Algorithme** :
  1. cuRAND générateur initialisé avec seed.
  2. Build CDF par row (1 reduction prefix-sum).
  3. Binary search `n` uniform values dans CDF.
* **Dépendances** : cuRAND
* **Test** : Distribution Kolmogorov-Smirnov (pas tolérance numérique).
* **Sprint** : D29

#### `moe_reconcile`

* **Op** : `moe_reconcile(candidates, gate_logits, top_k)`
* **Algorithme** : top-k softmax + weighted sum.
* **Sprint** : D29 (Mode-2 prep).

#### `kl_divergence`

* **Op** : `kl_divergence(p, q, axis, eps)`
* **Algorithme** : `KL = Σ p × log((p+ε)/(q+ε))`. Cap à 50.
* **Sprint** : D29.

### §28.5 Tableau récap 23 ops

| # | Op | Type | Sprint | cuBLAS ? | Tile T4 | Tile A10 |
|--:|----|:----:|:------:|:--------:|:-------:|:--------:|
| 1 | `device` | Infra | D29 | n/a | n/a | n/a |
| 2 | `name` | Infra | D29 | n/a | n/a | n/a |
| 3 | `synchronize` | Infra | D29 | n/a | n/a | n/a |
| 4 | `matmul` | Linalg | D28 | ✅ Sgemm | n/a | n/a |
| 5 | `matmul_fused_gelu` | Linalg | D28 | ✅ LtMatmul | n/a | n/a |
| 6 | `dot` | Linalg | D28 | ✅ Sdot | n/a | n/a |
| 7 | `l2_norm` | Norm | D28 | ✅ Snrm2 | n/a | n/a |
| 8 | `l2_normalize` | Norm | D28 | non | 256 | 256 |
| 9 | `l2_distance` | Norm | D28 | non | 256 | 256 |
| 10 | `cosine_similarity_pair` | Norm | D28 | non | 32 | 32 |
| 11 | `cosine_anchor_blend` | EFE | D29 | non | 256 | 256 |
| 12 | `softmax` | Activ | D28 | non | 256 | 256 |
| 13 | `log_softmax` | Activ | D29 | non | 256 | 256 |
| 14 | `gelu` | Activ | D28 | non | 256 | 256 |
| 15 | `sigmoid` | Activ | D28 | non | 256 | 256 |
| 16 | `scaled_dot_product_attention` | Attn | D28 | ✅ Sgemm batched | 128 | 256 |
| 17 | `topk` | Reduc | D28 | non | 256 | 256 |
| 18 | `argmax` | Reduc | D28 | non | 256 | 256 |
| 19 | `cone_project` | EFE | D28 | non | 1×32 | 1×32 |
| 20 | `weighted_mean` | EFE | D28 | non | 256 | 256 |
| 21 | `moe_reconcile` | EFE | D29 | non | 256 | 256 |
| 22 | `multinomial_sample` | Sampling | D29 | n/a (cuRAND) | 32 | 32 |
| 23 | `kl_divergence` | Info | D29 | non | 256 | 256 |
| 24 | `shannon_entropy` | Info | D28 | non | 256 | 256 |
| 25 | `mean` | Stats | D28 | non | 256 | 256 |
| 26 | `std_dev` | Stats | D28 | non | 256 | 256 |

(NB : 26 entrées — 23 ops + 3 infra. Cf. §8.1.)

---

## §29. Annexes — Gantt + RACI + budget cloud GPU

### §29.1 Gantt prévisionnel

```
            D28 D29 D30 D31 D32
build.rs    ██
18 P1+P2 ops ██
5 P3 ops        ██
Migration       ██
CI matrix       ██
Multi-GPU NCCL      ██
Profiling + opt        ██
Soak 24h               ██
Streaming                  ██
Hybrid fusion              ██
Doc final                  ██

Releases :
  v0.5.0 — D30  (single-GPU 23/23)
  v0.6.0 — D32  (multi-GPU + streaming + hybrid)
```

### §29.2 RACI matrix

| Activité | Core | Ops | SDK | Sec | PM |
|----------|:----:|:---:|:---:|:---:|:--:|
| `build.rs` + nvcc setup | **R** | C | I | I | I |
| Kernel implementations | **R** | I | I | I | A |
| Cross-backend tests | **R** | I | I | I | A |
| Migration `CudaBackend` | **R** | C | I | I | A |
| CI matrix (GPU runners) | C | **R** | I | I | A |
| Multi-GPU NCCL | **R** | C | I | I | A |
| Profiling / optimisation | **R** | I | I | I | A |
| Soak + memcheck | C | **R** | I | I | A |
| Streaming + Hybrid | **R** | I | C | I | A |
| Documentation finale | **R** | I | I | I | C |

### §29.3 Budget cloud GPU (récap)

| Item | Coût | Cadence | Total annuel |
|------|:----:|:-------:|:-------------:|
| CI smoke (T4 spot) | 0.50 €/h × 50 h | mensuel | 300 € |
| CI full + bench (A10 spot) | 0.80 €/h × 30 h | mensuel | 290 € |
| DR drill multi-GPU (4×A100 spot) | 5 €/h × 4 h | trimestriel | 80 € |
| Soak 24h (T4 spot) | 0.50 €/h × 24 h | hebdo | 625 € |
| Profiling sessions (Lambda Labs A100) | 2.5 €/h × 20 h | mensuel | 600 € |
| **Total annuel CI/CD GPU** | | | **~1 900 €** |
| **Total Φ-D one-shot (5 sprints)** | | | **~500 €** |

Budget total Φ-D = **2 400 €** annualisés. Cf.
`compute/ROADMAP.md` Phase 2-3 estimation initiale (~335 €) =
maintenant raffinée avec CI continu.

---

## §30. Mises à jour du document

### §30.1 Versioning

* **v1.0.0 (2026-04-28)** — première rédaction post-D18.
* Toute découverte audit-time, défaut résiduel, ou changement scope
  déclenche une révision avec :
  * Nouvelle entrée CHANGELOG.md.
  * Bump du Statut (en haut).
  * Pin test garde dans `scripts/tests/test_efe_kernels_doc_drift.py`
    qui valide le sommaire + les 23 ops listées + les 6 instruments
    Prom.

### §30.2 Liens à maintenir

* `docs/PRODUCT_COMPLETION_ARCHITECTURE.md` §8 (Φ-D) — cohérence
  obligatoire.
* `crates/efe-core/src/compute/ROADMAP.md` Phase 2-3 — converger.
* `docs/compute/tolerances.md` — tolérances cross-backend.
* `crates/efe-core/src/compute/backend.rs` — trait source of truth ;
  toute modification du trait → bump version doc.

### §30.3 Critères d'archive

Ce document devient **historique** une fois GA 1.0 livré.
Les modifications post-GA partent dans `EFE_KERNELS_v1.1+_ARCHITECTURE.md`.

**Fin du document.**
