# CUDA Kernels — Architecture de Complétion à 100 % (Φ-D)

> **Statut** : architecture détaillée, prête à implémenter
> (sprints D28 → D32 du roadmap
> `PRODUCT_COMPLETION_ARCHITECTURE.md` § 8).
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : ce document **clôt** Φ-D en passant de l'architecture
> théorique de `EFE_KERNELS_ARCHITECTURE.md` (2 482 lignes, format,
> trait `ComputeBackend`, contrat 23 opérations) et de l'état actuel
> du crate (`efe-kernels` 2 583 LOC Rust + 20 `.cu` files répartis
> en 8 dossiers) au **livrable mesurable + opposable** : une
> implémentation GPU production-grade qui ferme **GG-Φ-D.1
> (p99 ≤ 200 ms)**, **GG-Φ-D.2 (speedup ≥ 5×)**, **GG-Φ-D.3
> (multi-GPU 3× sur N=1024)**, et 24 h soak sans memleak.
>
> Sans ce sprint :
> - **SLO-P1-BUDGET-5** (= étage 5 inférence 26 ms) reste
>   `PENDING_GPU_RUNNER` → SLO-C-1 (200 ms p99 GPU) **non
>   opposable**.
> - **`crates/efe-kernels`** reste un crate non-compilable sur
>   les hôtes avec CUDA toolkit (le code stub est là, mais aucun
>   kernel CUBIN n'est généré ni bench).
> - **Le moat technique** d'EFE vs LLM chains (= 5-10× moins
>   cher) est **non démontré numériquement** → l'argumentaire
>   commercial reste théorique.
>
> **Pré-requis** : avoir lu
> [`EFE_KERNELS_ARCHITECTURE.md`](EFE_KERNELS_ARCHITECTURE.md) en
> intégralité (= format spec, choix toolchain, trait
> `ComputeBackend`), comprendre la doctrine **3 promesses
> opposables P1/P2/P3**, et connaître les sprints précédents (D16
> bundle format, M13 release CI, Φ-C bundle réel ce document
> consomme l'ONNX qu'il bench).

---

## Table des matières

1. [Mission & contrat opposable Φ-D](#1-mission--contrat-opposable-φ-d)
2. [Position vs `EFE_KERNELS_ARCHITECTURE.md` (= gap analysis)](#2-position-vs-efe_kernels_architecturemd--gap-analysis)
3. [État AS-IS chirurgical (= 30 % opérationnel)](#3-état-as-is-chirurgical--30--opérationnel)
4. [Décomposition canonique du budget GPU 200 ms](#4-décomposition-canonique-du-budget-gpu-200-ms)
5. [Surface 1 — Build pipeline production-grade](#5-surface-1--build-pipeline-production-grade)
6. [Surface 2 — Les 12 kernels qui ferment SLO-P1-BUDGET-7](#6-surface-2--les-12-kernels-qui-ferment-slo-p1-budget-7)
7. [Surface 3 — Memory & streams (VRAM + pinned host + arena)](#7-surface-3--memory--streams-vram--pinned-host--arena)
8. [Surface 4 — Numerical equivalence harness](#8-surface-4--numerical-equivalence-harness)
9. [Surface 5 — Multi-GPU NCCL (D30 sharded fusion)](#9-surface-5--multi-gpu-nccl-d30-sharded-fusion)
10. [Surface 6 — Observabilité (10 instruments + flame graphs)](#10-surface-6--observabilité-10-instruments--flame-graphs)
11. [Surface 7 — Défensive (8 garde-fous GPU runtime)](#11-surface-7--défensive-8-garde-fous-gpu-runtime)
12. [Surface 8 — Bench harness (Criterion + ghz + nightly soak)](#12-surface-8--bench-harness-criterion--ghz--nightly-soak)
13. [Surface 9 — SLO + gating CI (6 SLO Φ-D nouveaux)](#13-surface-9--slo--gating-ci-6-slo-φ-d-nouveaux)
14. [Surface 10 — Contractuelle (= attestation hardware fingerprint)](#14-surface-10--contractuelle--attestation-hardware-fingerprint)
15. [Matrice complète des artefacts (gap-filling)](#15-matrice-complète-des-artefacts-gap-filling)
16. [Plan d'implémentation D28 → D32 chirurgical](#16-plan-dimplémentation-d28--d32-chirurgical)
17. [Risques identifiés & mitigations (20 risques)](#17-risques-identifiés--mitigations-20-risques)
18. [Décisions de conception clés (18)](#18-décisions-de-conception-clés-18)
19. [Anti-régression vs CPU fallback](#19-anti-régression-vs-cpu-fallback)
20. [Critères d'acceptation (DoD à 100 %)](#20-critères-dacceptation-dod-à-100--)
21. [Hand-off & dépendances downstream](#21-hand-off--dépendances-downstream)
22. [Annexes (A → H)](#22-annexes-a--h)

---

## 1. Mission & contrat opposable Φ-D

### 1.1 La promesse

> **Φ-D Complete** : sur hardware NVIDIA T4 (16 GiB VRAM) + bundle
> MiniLM-L12-v2 fp16 + Mode-1, le **swarm fusion p99 ≤ 10 ms par
> round** (= 100 ms pour 10 rounds = budget P1 étage 7) et le
> **multi-GPU 4-shard speedup ≥ 3×** sur swarm N=1024.
>
> Sans CUDA kernels custom, l'ONNX CUDA EP générique consomme
> ~30 ms/round (= 300 ms pour 10 rounds) — incompatible avec
> SLO-C-1 (200 ms p99). Le gain ≥ 3× attendu de Φ-D est ce qui
> rend la promesse P1 mathématiquement défendable.

### 1.2 Le mot « 100 % » appliqué à Φ-D

**100 %** signifie ici, et seulement ici, l'union de **6 critères
mesurables** :

1. **`cargo build --workspace --features cuda`** réussit sur un
   hôte avec CUDA Toolkit 12.4+ (= aujourd'hui : la feature compile
   mais avec stubs).
2. **`cargo bench --features cuda`** produit un rapport JSON avec
   tous les 12 kernels (= aujourd'hui : 0 kernel benché).
3. **Numerical equivalence harness** atteste `|y_gpu - y_cpu| / |y_cpu|
   < 1e-5` sur tous les 12 kernels, sur 3 architectures
   (sm_75 T4, sm_80 A100, sm_86 A10), avec fp16 + fp32 (=
   aujourd'hui : 0 attestation).
4. **6 SLO Φ-D** (cf. § 13) tous PASS sur le runner self-hosted
   GPU T4 nightly (= aujourd'hui : 0 SLO).
5. **24 h soak** sur 200 RPS Mode-1 sans memleak (= variation
   RSS < 10 MiB/h) et sans NaN/Inf (= aujourd'hui : 0 soak run).
6. **CHANGELOG v0.6.0** taggé + audit interne chirurgical
   ≤ 8 défauts résolus (= aujourd'hui : v0.6.0 n'existe pas).

Avant la complétion : Φ-D est à ~30 % (= architecture documentée
+ scaffolding Rust + 20 fichiers `.cu` stubs). Après la complétion :
Φ-D est à 100 %, mesurable, signé, et opposable.

### 1.3 Périmètre exact de Φ-D

**Sont inclus** dans Φ-D :
- Pipeline de build (= `build.rs` → nvcc → multi-arch CUBIN ou PTX
  → liée statiquement).
- 12 kernels custom (cf. § 6) + leurs Rust wrappers.
- Memory subsystem (= VRAM arena + pinned host pool).
- Stream subsystem (= 4 streams par worker Tokio + priorities).
- Multi-GPU NCCL all-reduce pour swarm consensus (D30+).
- Numerical equivalence harness (= test régression vs CpuBackend).
- 6 SLO + bench harness Criterion.
- Observabilité (= 10 Prometheus instruments, dont 6 existants).
- Doc + runbooks + procédures de rotation GPU driver.

**Sont exclus** (réservés post-GA ou v1.1+) :
- Mode-2 / Mode-3 specific GPU optimisations (= Φ-E/F).
- Custom int8 quantization (= roadmap v2.0 quand stable).
- Apple Silicon Metal backend (= scope séparé v3.0).
- WASM SIMD substrate (= recherche).
- Diffusion / generative GPU ops (= EFE n'est pas un LLM).

### 1.4 Conditions de référence (= GA contractuelles)

| Paramètre | Valeur | Tolérance |
|-----------|--------|-----------|
| GPU baseline | NVIDIA T4 (sm_75, 16 GiB VRAM, 320 GB/s mem BW) | T4 / L4 / A10 / A100 / H100 (= 5 archs supportées) |
| CUDA Toolkit | 12.4 (compile) ; 12.0+ (runtime via PTX JIT) | ≥ 12.0 runtime |
| Driver NVIDIA | ≥ 535.x | ≥ 525.x |
| ONNX Runtime | 1.18.x (= aligné Φ-C bundle) | 1.17-1.20 |
| Bundle embedder | MiniLM-L12-v2 fp16 (= Φ-C) | celui livré v1.0.0 |
| Mode | Mode-1 (= zero-shot ; Mode-2/3 viennent de Φ-E/F) | n/a |
| Charge bench | 200 RPS sustained 10 min | 0 → 220 RPS |
| Multi-GPU | 4× T4 ou 2× A10 ou 1× A100 (NVLink optionnel) | min 2 GPUs |

Toute déviation suspend l'opposabilité.

---

## 2. Position vs `EFE_KERNELS_ARCHITECTURE.md` (= gap analysis)

### 2.1 Ce que l'archi existante couvre déjà (= NE PAS dupliquer)

`EFE_KERNELS_ARCHITECTURE.md` (2 482 lignes) couvre déjà
exhaustivement :

| § archi existante | Contenu | Action ce doc |
|-------------------|---------|----------------|
| § 1 — Mission | Vision + dépendance | Réutilise par référence |
| § 2 — Topologie | `efe-kernels → efe-core` (= jamais l'inverse) | Réutilise |
| § 8 — Contrat `ComputeBackend` | 23 ops × 4 priorités | Réutilise |
| § 9 — Choix toolchain | `cudarc` 0.10 + nvcc 12.4 + `cc-rs` build-dep | Réutilise |
| § 10 — `build.rs` design | Detection nvcc + multi-arch flag | Réutilise + étend (§ 5 ci-dessous) |
| § 11 — `cudarc` integration | Safe bindings, Send/Sync wrappers | Réutilise |
| § 12 — Organisation `.cu` | 8 dossiers thématiques | Confirmé + complète § 6 |
| § 13 — `CudaBackend` impl | Trait wiring | Étend par § 6 |
| § 14 — Streams + VRAM | Per-worker streams + arena | Étend par § 7 |
| § 15 — Multi-GPU NCCL | Architecture sharded fusion | Étend par § 9 |
| § 16 — Numerical equiv harness | Format des tests | Étend par § 8 |
| § 17 — Budget perf par GPU | Tile sizes T4/A100/H100 | Réutilise |
| § 18 — Observabilité | 6 Prometheus instruments | Étend par § 10 (4 nouveaux) |
| § 19 — Fallback CPU | Auto-détection | Réutilise |
| § 20-23 — Determinisme, sûreté, errors, tests | Listes complètes | Réutilise |

### 2.2 Ce que l'archi existante NE COUVRE PAS (= scope ce doc)

| Lacune identifiée | Pourquoi critique | Section ci-dessous |
|-------------------|-------------------|---------------------|
| **Gap analysis chirurgicale** entre archi théorique et état réel du crate | Sans, l'équipe ne sait pas où commencer | § 3 |
| **Budget GPU 200 ms décomposé en 12 kernels** | Sans, pas de SLO par kernel | § 4 |
| **Plan d'implémentation chronologique D28→D32** | Sans, l'exécution est ad-hoc | § 16 |
| **6 SLO Φ-D mesurables + gating CI** | Sans, "100 %" est non-mesurable | § 13 |
| **Attestation hardware fingerprint** | Sans, opposabilité juridique compromise | § 14 |
| **Matrice de 30+ artefacts à livrer** | Sans, on oublie des éléments | § 15 |
| **20 risques GPU-specific + mitigations** | Sans, premier incident = surprise | § 17 |
| **18 décisions de conception clés justifiées** | Sans, dérive technique possible | § 18 |
| **DoD chirurgical à 100 %** | Sans, "presque fini" n'est pas fini | § 20 |
| **Hand-off Φ-E (GPU budget restant pour Mode-2)** | Sans, Mode-2 bloque | § 21 |

→ Ce document est **purement complémentaire** à l'archi existante,
focalisé sur **comment atteindre 100 % opérationnel**.

---

## 3. État AS-IS chirurgical (= 30 % opérationnel)

### 3.1 Inventaire des fichiers existants

```
crates/efe-kernels/
├── Cargo.toml                 ✅ (features cuda, nccl, fp16, mock_gpu déclarées)
├── build.rs                   ✅ (skeleton complet ; nvcc detection ; stubs OK)
├── README.md                  ✅ (= pointer vers archi doc)
├── src/
│   ├── backend.rs            869 LOC  ⚠️  CudaBackend skeleton ; trait impl partial
│   ├── error.rs              273 LOC  ✅ taxonomie d'erreurs complète
│   ├── fallback.rs           252 LOC  ⚠️  CPU fallback ; non-testé vs prod use case
│   ├── kernels.rs            266 LOC  ⚠️  wrappers Rust ; appels CUDA stubbed
│   ├── lib.rs                124 LOC  ✅ feature gating + docs
│   ├── memory.rs             293 LOC  ⚠️  VRAM arena ; pas d'OOM tests
│   ├── multi_gpu.rs          117 LOC  ⚠️  NCCL stub seulement
│   ├── observability.rs      222 LOC  ⚠️  6 instruments déclarés ; pas de Histogram fusionné
│   └── streams.rs            167 LOC  ⚠️  StreamPool skeleton ; pas de priority API
├── cuda/
│   ├── include/efe.cuh       126 LOC  ✅ helpers communs
│   ├── include/efe_warp.cuh  133 LOC  ✅ warp reductions
│   ├── stub.cu                28 LOC  ✅ pour le mode no-CUDA
│   ├── activations/
│   │   ├── gelu.cu                    ⚠️  signature seulement
│   │   ├── log_softmax.cu             ⚠️  signature
│   │   ├── sigmoid.cu                 ⚠️  signature
│   │   └── softmax.cu                 ⚠️  signature ; PAS le fused softmax+entropy critique
│   ├── attention/
│   │   └── scaled_dot_product_attention.cu  ⚠️  partiel
│   ├── debug/
│   │   └── check_finite.cu            ⚠️  NaN/Inf scan
│   ├── efe_core/
│   │   ├── cone_project.cu            ⚠️  squelette
│   │   ├── moe_reconcile.cu           ⚠️  squelette
│   │   └── weighted_mean.cu           ⚠️  squelette
│   ├── linalg/
│   │   ├── cosine.cu                  ⚠️  squelette
│   │   ├── l2_distance.cu             ⚠️  squelette
│   │   └── l2_normalize.cu            ⚠️  squelette
│   ├── reductions/
│   │   ├── argmax.cu                  ⚠️  squelette
│   │   ├── kl_divergence.cu           ⚠️  squelette
│   │   ├── mean.cu                    ⚠️  squelette
│   │   ├── shannon_entropy.cu         ⚠️  squelette
│   │   ├── std_dev.cu                 ⚠️  squelette
│   │   └── topk.cu                    ⚠️  squelette
│   └── sampling/
│       └── multinomial_sample.cu      ⚠️  squelette
└── tests/                              ❌ vide

Légende :
 ✅ = production-ready (= testé + benché + zero TODO)
 ⚠️  = squelette présent mais incomplet (= signatures + 0 body, ou body mais 0 test)
 ❌ = absent
```

### 3.2 Verdict de l'état

- **Squelettes en place** : 100 % (= 20 fichiers `.cu` + 9 modules Rust).
- **Implémentation kernel** : ~20 % (= soft signatures, peu de body fonctionnel).
- **Tests** : 0 % (= dossier `tests/` vide ; pas de numerical equiv harness lancé).
- **Benches** : 0 % (= aucun bench Criterion).
- **Multi-GPU NCCL** : 5 % (= `multi_gpu.rs` 117 LOC stub seulement).
- **CI gating** : 0 % (= pas de workflow nightly GPU).
- **Documentation runbook** : 30 % (= la doc archi existe ; pas de runbook ops).

**Score global : 30 %.**

### 3.3 Critères "100 %" mesurables

Pour atteindre 100 %, chaque ligne du tableau ci-dessus doit
passer à ✅. Le chemin chirurgical est dans § 16.

---

## 4. Décomposition canonique du budget GPU 200 ms

### 4.1 Mapping P1 étages × kernels GPU

Le budget P1 (cf. `EFE_P1_SURFACE_ARCHITECTURE.md` § 2.1) alloue
26 ms à l'embedder (étage 5) et 100 ms à la fusion (étage 7) sur
T4 fp16. Φ-D doit livrer ces 2 budgets simultanément.

| Étage P1 | Budget | Kernels GPU mobilisés | Cible kernel p99 |
|----------|-------:|------------------------|------------------:|
| 5 (Embedder) | 26 ms | ONNX CUDA EP générique (= bundle) + nos kernels pour pre/post-proc | inférence 24 ms + pooling 1 ms + L2 norm 1 ms |
| 7 (Fusion, 10 rounds) | 100 ms (10 ms/round) | Fused softmax+entropy ×N + cone project ×N + weighted mean ×N | par round : softmax 3 ms + cone 4 ms + mean 2 ms + sync 1 ms |
| 8 (Consensus) | 1 ms | argmax + KL div | < 0.5 ms chacun |
| 9 (Dissolution+Align) | 6 ms | l2_normalize + cosine + topk + projection | 1.5 + 1.5 + 1 + 2 = 6 ms |

### 4.2 Détail du round de fusion (= 10 ms cible)

```
1 round de swarm fusion sur T4 fp16, N=128 agents, dim=384 :

┌────────────────────────────────────────────────────────────┐
│ k1. weighted_mean(agents)      (= centroïde local)         │
│     batched [N,D] → [1,D]                            1.5 ms │
├────────────────────────────────────────────────────────────┤
│ k2. cone_project(agents, centroid)                          │
│     batched [N,D] × [1,D] → [N,D]                    2.5 ms │
├────────────────────────────────────────────────────────────┤
│ k3. fused_softmax_entropy(scores)                           │
│     [N,K] → ([N,K] probs, [N] H)                     3.0 ms │
├────────────────────────────────────────────────────────────┤
│ k4. weighted_mean(agents, probs)                            │
│     [N,D] × [N,K] → [K,D]                            2.0 ms │
├────────────────────────────────────────────────────────────┤
│ k5. l2_normalize(centroids)                                 │
│     [K,D] → [K,D]                                    0.5 ms │
├────────────────────────────────────────────────────────────┤
│ sync (cudaStreamSync)                                0.5 ms │
└────────────────────────────────────────────────────────────┘
                                            TOTAL = 10.0 ms ✓
```

→ Cible **3 + 2.5 + 2 = 7.5 ms** consommé par 3 kernels ; les
2.5 ms restants couvrent l2_norm, sync, et la marge anti-jitter.

### 4.3 Importance du "fused" softmax+entropy

Faire `softmax` puis `entropy` séparément consomme **2 launches +
2 reads/writes du tableau** (= ~4 ms sur T4). Le **fused
kernel** combine les 2 ops + accumulate l'entropy via warp-reduce
en 1 pass = **3 ms** (~ -25 %). Différenciateur clé sur la latence.

---

## 5. Surface 1 — Build pipeline production-grade

### 5.1 Du `build.rs` actuel au build prod

L'état actuel (`build.rs` 80+ LOC) détecte `nvcc`, vérifie CUDA
≥ 12.4, et compile les `.cu` files. Ce qui manque pour la prod :

| # | Lacune actuelle | Fix |
|--:|-----------------|-----|
| 1 | Multi-arch single CUBIN (= 1 binaire par architecture GPU) | Compiler **fatbin** avec `-gencode=arch=compute_75,code=sm_75 -gencode=...,code=sm_80 -gencode=...,code=sm_86 -gencode=...,code=sm_89 -gencode=...,code=sm_90` |
| 2 | Pas de cache compilation (= 5 min cold) | Use `sccache` + `CARGO_TARGET_DIR` partagé en CI |
| 3 | Pas de fallback PTX-only (= cross-compile sur runner sans CUDA) | Distribuer PTX embarqué dans crate + JIT au runtime |
| 4 | Pas de signature des `.cu` files | Hash SHA-256 de chaque source + check dans `build.rs` pour détecter tampering |
| 5 | Pas de gate `nvcc` warnings = errors | `-Werror=all -Werror=deprecated-declarations` |
| 6 | Pas d'optimization profile | `-O3 -use_fast_math` pour release ; `-O0 -G` pour debug |
| 7 | Pas de PTX-Inspector debug | `-lineinfo` pour traces avec line numbers |
| 8 | Pas de version-check NCCL/cuBLAS | Detect via `cuda.h`/`nccl.h` headers + bail si version pré-2022 |

### 5.2 Architecture multi-arch CUBIN

```
Output flow du build :

cuda/activations/softmax.cu
            │
            ▼  nvcc -O3 -use_fast_math \
              -gencode=arch=compute_75,code=sm_75 \
              -gencode=arch=compute_80,code=sm_80 \
              -gencode=arch=compute_86,code=sm_86 \
              -gencode=arch=compute_89,code=sm_89 \
              -gencode=arch=compute_90,code=sm_90 \
              -gencode=arch=compute_90,code=compute_90    (= PTX fallback)
              -dc softmax.cu -o softmax.fatbin
            │
            ▼ ar rc libefe_kernels_cuda.a *.fatbin
            │
            ▼ rustc --link-arg=-lefe_kernels_cuda.a
            │
            ▼ Output : efe-server binary

Runtime selection :
  - Driver detects SM (e.g. 7.5 → T4) → loads sm_75 CUBIN.
  - Si SM ≥ 9.0 et pas de CUBIN pinné → JIT depuis PTX
    (compute_90 ptx embarqué).
```

### 5.3 Feature gating Cargo (validation des features actuelles)

```toml
[features]
default = []
cuda    = ["dep:cudarc", "dep:cc"]       # ✅ déjà déclaré
nccl    = ["cuda", "cudarc/nccl"]        # ✅ déjà déclaré
fp16    = []                              # ⚠️  déclaré mais pas implémenté
auto_check_finite = []                    # ✅
mock_gpu = []                             # ✅
ptx_only = ["cuda"]                       # ← NOUVEAU : utile pour cross-compile
```

### 5.4 CI matrix de builds à valider

| Configuration | Quand ? | Statut cible |
|---------------|---------|:------------:|
| `cargo build --workspace` (= no cuda) | Chaque PR | ✅ vert (= déjà OK) |
| `cargo build --workspace --features cuda` (sur runner T4) | Chaque PR si paths matchent | ✅ vert |
| `cargo build --workspace --features cuda,nccl` (sur runner multi-GPU) | Nightly | ✅ vert |
| `cargo build --workspace --features cuda,fp16` (sur T4) | Nightly | ✅ vert |
| Cross-compile Mac → Linux x86_64 GPU runtime via `ptx_only` | Manuel | ⏸ Φ-E+ |

### 5.5 Distribuabilité

Le binaire `efe-server` lié à `libefe_kernels_cuda.a` est :
- **Self-contained** : kernels embedded ; aucun fichier `.cu`
  ou `.so` séparé à shipper.
- **Hash-pinnable** : `efe_kernels_cuda_sha256` exposé via
  Prometheus gauge au boot ; permet aux opérateurs de vérifier
  qu'ils run la binaire prévue.
- **Compatible distroless** : aucune dépendance runtime hors CUDA
  driver (= déjà sur le node K8s via `nvidia.com/gpu`).

---

## 6. Surface 2 — Les 12 kernels qui ferment SLO-P1-BUDGET-7

### 6.1 Inventaire chirurgical des 12 kernels production-grade

| # | Kernel | Fichier source | Priorité | Cible p99 | Speedup vs CPU MVP |
|--:|--------|----------------|:--------:|----------:|:-------------------:|
| 1 | `fused_softmax_entropy` | `activations/softmax.cu` + ext | **P0** | 3 ms (N=128, K=64) | ~ 8× |
| 2 | `weighted_mean` (batched) | `efe_core/weighted_mean.cu` | **P0** | 1.5 ms (N=128, D=384) | ~ 6× |
| 3 | `cone_project` (batched) | `efe_core/cone_project.cu` | **P0** | 2.5 ms (N=128, D=384) | ~ 5× |
| 4 | `l2_normalize` (batched) | `linalg/l2_normalize.cu` | **P0** | 0.5 ms (K=64, D=384) | ~ 4× |
| 5 | `cosine_similarity` (batched) | `linalg/cosine.cu` | **P1** | 1.5 ms (N=128, M=N) | ~ 5× |
| 6 | `l2_distance` (batched) | `linalg/l2_distance.cu` | **P1** | 1.5 ms | ~ 5× |
| 7 | `mean_pooling` (over seq) | `reductions/mean.cu` | **P1** | 0.8 ms (B=1, S=128, D=384) | ~ 6× |
| 8 | `argmax` (= consensus) | `reductions/argmax.cu` | **P1** | 0.3 ms (N=64) | ~ 3× |
| 9 | `topk` (k=10 par défaut) | `reductions/topk.cu` | **P2** | 0.8 ms (N=1000, K=10) | ~ 5× |
| 10 | `kl_divergence` | `reductions/kl_divergence.cu` | **P2** | 0.5 ms | ~ 4× |
| 11 | `shannon_entropy` | `reductions/shannon_entropy.cu` | **P3** | 0.4 ms | ~ 3× |
| 12 | `moe_reconcile` (= Mode-2 hook) | `efe_core/moe_reconcile.cu` | **P3** | 1.2 ms | ~ 4× |

### 6.2 Détail du kernel **#1 fused_softmax_entropy** (le critique)

Le softmax+entropy fused est **le différenciateur** clé. Détail
de l'algo CUDA :

```cuda
// fused_softmax_entropy.cu (= P0 priorité absolue)
//
// Compute, in 1 kernel pass over `scores[N][K]` :
//   - probs[i][j] = softmax(scores[i])[j]
//   - H[i]        = -Σ probs[i][j] * log2(probs[i][j])
//
// Pourquoi fused : éviter 2 reads de `scores` + 1 write
// intermédiaire (= ~25 % bandwidth saved).
//
// Block layout : 1 block / row (= 1 block / agent).
// Thread layout : 32 threads / block = 1 warp (= K ≤ 64 typique).
// Shared memory : 32 * sizeof(half) = 64 B / block (cheap).
//
// Numerically stable : subtract max(scores) before exp() to
// avoid fp16 overflow.

extern "C" __global__ void fused_softmax_entropy_kernel(
    const half* __restrict__ scores,  // [N][K]
    half* __restrict__ probs,         // [N][K] output
    half* __restrict__ entropy,       // [N]    output
    const int N,
    const int K
) {
    const int row = blockIdx.x;
    if (row >= N) return;

    const half* row_scores = scores + row * K;
    half* row_probs = probs + row * K;

    const int tid = threadIdx.x;

    // Phase 1 : find max(scores[row]) for numerical stability
    half local_max = __float2half(-INFINITY);
    for (int j = tid; j < K; j += blockDim.x) {
        local_max = __hmax(local_max, row_scores[j]);
    }
    __shared__ half s_max;
    half global_max = warp_reduce_max(local_max);
    if (tid == 0) s_max = global_max;
    __syncthreads();

    // Phase 2 : exp(scores - max) and accumulate sum
    half local_sum = __float2half(0.0f);
    for (int j = tid; j < K; j += blockDim.x) {
        half v = hexp(row_scores[j] - s_max);
        row_probs[j] = v;
        local_sum = __hadd(local_sum, v);
    }
    __shared__ half s_sum;
    half global_sum = warp_reduce_sum(local_sum);
    if (tid == 0) s_sum = global_sum;
    __syncthreads();

    // Phase 3 : normalize + accumulate -Σ p * log2(p)
    half local_h = __float2half(0.0f);
    for (int j = tid; j < K; j += blockDim.x) {
        half p = __hdiv(row_probs[j], s_sum);
        row_probs[j] = p;
        // -p * log2(p) = -p * log(p) / log(2)
        if (__hgt(p, __float2half(1e-7f))) {
            local_h = __hsub(local_h, __hmul(p, hlog2(p)));
        }
    }
    half global_h = warp_reduce_sum(local_h);
    if (tid == 0) entropy[row] = global_h;
}
```

### 6.3 Stratégie de tile size par GPU type

L'archi existante (§ 17) note les tile sizes. Pour le softmax :

| GPU | Compute Cap | Optimal block size | SMs | Optimal grid |
|-----|------------:|:------------------:|:---:|:------------:|
| T4 | 7.5 | 32 (= 1 warp) | 40 | 128-256 blocks |
| L4 | 8.9 | 64 (= 2 warps) | 58 | 256-512 blocks |
| A10 | 8.6 | 64 | 72 | 512 blocks |
| A100 | 8.0 | 128 (= 4 warps) | 108 | 512-1024 blocks |
| H100 | 9.0 | 256 (= 8 warps) | 132 | 1024+ blocks |

→ Auto-tune via env var `EFE_CUDA_FUSION_TILE_SIZE` (= overridable
en prod ; default = lookup table par compute cap).

### 6.4 Rust wrappers (= `kernels.rs` extension)

```rust
// Snippet de l'API publique kernels.rs (= étendre les 266 LOC actuels)
impl CudaBackend {
    pub fn fused_softmax_entropy(
        &self,
        scores: &CudaTensor<f16>,        // [N][K]
        stream: &CudaStream,
    ) -> Result<(CudaTensor<f16>, CudaTensor<f16>), KernelError> {
        let n = scores.shape()[0];
        let k = scores.shape()[1];
        // Validate dims
        if k > 1024 {
            return Err(KernelError::InvalidShape("K > 1024".into()));
        }
        // Alloc outputs via arena
        let mut probs = self.alloc::<f16>(&[n, k], stream)?;
        let mut entropy = self.alloc::<f16>(&[n], stream)?;
        // Block/thread layout selon GPU type
        let block_size = self.tile_size_for_kernel(KernelId::FusedSoftmax);
        let grid_size = (n + block_size - 1) / block_size;
        // Launch
        unsafe {
            cudarc::raw::launch_kernel(
                self.fused_softmax_entropy_fn,
                (grid_size as u32, 1, 1),
                (block_size as u32, 1, 1),
                stream.raw(),
                &[
                    scores.device_ptr(),
                    probs.device_ptr_mut(),
                    entropy.device_ptr_mut(),
                    &(n as i32),
                    &(k as i32),
                ],
            )?;
        }
        // Instrument
        observability::kernel_launches_total(KernelId::FusedSoftmax).inc();
        // Async return (no sync ; caller decides)
        Ok((probs, entropy))
    }
}
```

---

## 7. Surface 3 — Memory & streams (VRAM + pinned host + arena)

### 7.1 État `memory.rs` actuel et gaps

`memory.rs` (293 LOC) déclare un `VramArena` mais n'a pas :
- **Tests OOM** (= ce qui se passe quand VRAM saturée)
- **Sub-allocations alignées** (= alignment 256 B pour cuBLAS)
- **Pinned host pool** pour les transfers async
- **Streams-aware sub-allocations** (= chaque stream a son arena)

### 7.2 Architecture mémoire production

```
┌─────────────────────────────────────────────────────────────────────┐
│  Per-process : 1 VramManager                                         │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │  Pool 1 : LongLived (= weights ONNX, immortel pendant run)   │   │
│  │  Cap : ~ 500 MiB (= bundle MiniLM fp16 + ORT graph buffers)  │   │
│  │  Strategy : alloc 1× at boot, libere at shutdown             │   │
│  └──────────────────────────────────────────────────────────────┘   │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │  Pool 2 : PerStreamArena × 4 streams                         │   │
│  │  Cap : ~ 200 MiB / stream (= 800 MiB total)                  │   │
│  │  Strategy : bump alloc ; reset() à chaque request fin         │   │
│  │  Alignment : 256 B (= cuBLAS requirement)                    │   │
│  └──────────────────────────────────────────────────────────────┘   │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │  Pool 3 : PinnedHostPool                                     │   │
│  │  Cap : ~ 100 MiB (= input/output staging)                    │   │
│  │  Strategy : free-list de buffers 1/16/64/256/1024 KiB        │   │
│  │  cudaHostAlloc(WriteCombined | Portable)                     │   │
│  └──────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
              VRAM total budget T4 : 1 500 MiB (= ~10 % de la 16 GiB ;
              le reste est pour le tenant load partagé entre 3 répliques)
```

### 7.3 Stratégie OOM

```rust
impl VramArena {
    pub fn alloc<T>(&mut self, n: usize) -> Result<DevicePtr<T>, OomError> {
        let bytes = n * size_of::<T>();
        let aligned = (bytes + 255) & !255;   // 256 B alignment
        if self.offset + aligned > self.cap {
            // OOM : essayer rétrécir le batch + retry au caller
            return Err(OomError {
                requested_bytes: aligned,
                available_bytes: self.cap - self.offset,
                pool_name: self.name.clone(),
            });
        }
        let ptr = self.base.add(self.offset);
        self.offset += aligned;
        Ok(unsafe { DevicePtr::from_raw(ptr) })
    }

    pub fn reset(&mut self) {
        self.offset = 0;
        // Note : VRAM n'est PAS dealloc, juste le bump-pointer reset.
        // Garantie : O(1) reset, pas de fragmentation.
    }
}
```

### 7.4 Pinned host pool (= zero-copy transfers async)

```rust
// Per-process singleton, lazy-init
pub struct PinnedHostPool {
    free_lists: [SegQueue<PinnedBuf>; 5],   // 1K, 16K, 64K, 256K, 1024K
    total_allocated: AtomicU64,
}

impl PinnedHostPool {
    pub fn acquire(&self, size: usize) -> PinnedBuf {
        let class = self.size_class(size);
        if let Some(buf) = self.free_lists[class].pop() {
            return buf;
        }
        // Cold path : cudaHostAlloc
        let bytes = self.bytes_for_class(class);
        let host_ptr = unsafe {
            let mut p = std::ptr::null_mut();
            cudarc::raw::cuda_host_alloc(&mut p, bytes, HOST_ALLOC_WRITE_COMBINED)
                .expect("cuda_host_alloc failed");
            p
        };
        self.total_allocated.fetch_add(bytes as u64, Ordering::Relaxed);
        PinnedBuf { ptr: host_ptr, class, pool: self.clone() }
    }
}

impl Drop for PinnedBuf {
    fn drop(&mut self) {
        // Return to free list (= no actual free)
        self.pool.free_lists[self.class].push(self.clone());
    }
}
```

---

## 8. Surface 4 — Numerical equivalence harness

### 8.1 La discipline d'équivalence numérique

Tout kernel CUDA livré doit produire des outputs **numériquement
équivalents** à la référence CPU (`CpuBackend`) avec une tolérance
documentée. Sans cela, le moteur swarm peut produire des
résultats différents selon l'hôte = catastrophique pour P3 audit.

### 8.2 Tolérances par opération

| Kernel | Tolérance fp32 (rtol) | Tolérance fp16 (rtol) | Pourquoi |
|--------|----------------------:|----------------------:|----------|
| weighted_mean | 1e-6 | 1e-3 | reduction order may differ |
| cone_project | 1e-5 | 1e-3 | normalize sensitivity |
| fused_softmax | 1e-5 | 1e-3 | exp() approximation in fp16 |
| l2_normalize | 1e-6 | 1e-3 | sqrt() roundoff |
| cosine_similarity | 1e-5 | 1e-3 | dot product accumulator |
| l2_distance | 1e-5 | 1e-3 | idem |
| argmax | 0 (exact) | 0 (exact) | int output |
| topk | 0 (exact ordering) | depend on ties | special handling |
| mean_pooling | 1e-6 | 1e-3 | reduction order |
| kl_divergence | 1e-5 | 1e-3 | log() approximation |
| entropy | 1e-5 | 1e-3 | log() approximation |
| moe_reconcile | 1e-5 | 1e-3 | reduce + softmax composite |

### 8.3 Harness `tests/numerical_equivalence.rs`

```rust
// tests/numerical_equivalence.rs (= livré D29)
#[test]
#[cfg(all(feature = "cuda", target_os = "linux"))]
fn fused_softmax_entropy_equivalent_to_cpu_within_tolerance() {
    let cuda = CudaBackend::auto_detect().expect("GPU runner");
    let cpu = CpuBackend::new();

    let test_cases = vec![
        (1, 64, 1.0),       // K=64 normal
        (8, 256, 1.0),      // K=256
        (32, 1024, 1.0),    // K=1024 max
        (1, 64, 100.0),     // large scores (= test stability)
        (1, 64, -100.0),    // very negative (= test underflow guard)
    ];

    for (n, k, scale) in test_cases {
        let scores: Vec<f32> = (0..n*k).map(|i| {
            (rng().gen::<f32>() - 0.5) * scale
        }).collect();

        let scores_gpu = cuda.upload(&scores, &[n, k]).unwrap();
        let scores_cpu = cpu.upload(&scores, &[n, k]).unwrap();

        let (probs_gpu, h_gpu) = cuda.fused_softmax_entropy(&scores_gpu).unwrap();
        let (probs_cpu, h_cpu) = cpu.fused_softmax_entropy(&scores_cpu).unwrap();

        // Compare probs : relative tolerance 1e-3 (fp16)
        for i in 0..n*k {
            let g: f32 = probs_gpu.as_slice()[i].to_f32();
            let c: f32 = probs_cpu.as_slice()[i];
            let rtol = if c.abs() < 1e-7 { 1e-3 } else { (g - c).abs() / c.abs() };
            assert!(rtol < 1e-3,
                "case (n={n}, k={k}, scale={scale}) elem {i}: gpu={g}, cpu={c}, rtol={rtol}");
        }

        // Compare entropy
        for i in 0..n {
            let g: f32 = h_gpu.as_slice()[i].to_f32();
            let c: f32 = h_cpu.as_slice()[i];
            let rtol = if c.abs() < 1e-7 { 1e-3 } else { (g - c).abs() / c.abs() };
            assert!(rtol < 1e-3,
                "entropy mismatch case ({n}, {k}, {scale}) i={i}: gpu={g}, cpu={c}");
        }
    }
}
```

### 8.4 Cross-architecture validation

Le harness est run en CI sur **3 architectures** : T4 (sm_75), A10
(sm_86), A100 (sm_80). Si un kernel diverge entre archs au-delà de
la tolérance, c'est un bug du fp16 path ou du tile size.

### 8.5 Property-based testing

Pour les kernels critiques (softmax, l2_normalize), ajout de tests
property-based via `proptest` :

```rust
proptest! {
    #[test]
    fn fused_softmax_probs_sum_to_one(scores in any_score_matrix(1..=32, 1..=512)) {
        let cuda = CudaBackend::auto_detect().unwrap();
        let (probs, _h) = cuda.fused_softmax_entropy(&scores).unwrap();
        for row in probs.rows() {
            let sum: f32 = row.iter().map(|p| p.to_f32()).sum();
            prop_assert!((sum - 1.0).abs() < 1e-3);
        }
    }

    #[test]
    fn fused_softmax_probs_non_negative(scores in any_score_matrix(1..=8, 1..=128)) {
        let cuda = CudaBackend::auto_detect().unwrap();
        let (probs, _) = cuda.fused_softmax_entropy(&scores).unwrap();
        for p in probs.iter() {
            prop_assert!(p.to_f32() >= 0.0);
        }
    }
}
```

---

## 9. Surface 5 — Multi-GPU NCCL (D30 sharded fusion)

### 9.1 Architecture sharding

Pour swarm N > 256, le sharding multi-GPU permet de **scaler
linéairement** :

```
N=1024 agents :

┌─────────────────────────────────────────────────────────────────┐
│  GPU 0 : agents [0..256]   (= 1/4 du swarm)                     │
│  GPU 1 : agents [256..512]                                       │
│  GPU 2 : agents [512..768]                                       │
│  GPU 3 : agents [768..1024]                                      │
└─────────────────────────────────────────────────────────────────┘
                            │
                            ▼  NCCL all-reduce
                                (= sum centroids cross-GPU)
                            │
            Each GPU has global centroid for next round
```

### 9.2 NCCL ops requis

| Op | Quand | Cost wrt N |
|----|-------|------------|
| `ncclAllReduce(SUM)` | Après chaque round → consensus global | O(D × log P) où P = nb GPUs |
| `ncclBroadcast` | Distribution du centroid initial | O(D) |
| `ncclAllGather` | Si on veut top-K agents cross-shard | O(N × D / P) |

### 9.3 État `multi_gpu.rs` actuel (= 117 LOC stub)

Le module a un `MultiGpuConfig` mais pas :
- Détection automatique du nombre de GPUs disponibles
- NCCL communicator initialization (= `ncclCommInitRank`)
- Fault-tolerance si 1 GPU lost (= drain + redistribute)
- Topology-aware (= NVLink vs PCIe ; affecte le hop cost)

### 9.4 Plan d'implémentation NCCL (D30)

```rust
// crates/efe-kernels/src/multi_gpu.rs (= étendre les 117 LOC)
pub struct NcclCluster {
    rank: i32,                            // ce GPU's rank
    world_size: i32,                      // total GPUs
    comm: cudarc::nccl::Comm,             // NCCL communicator
    streams: Vec<CudaStream>,             // 1 stream / GPU
    topology: NcclTopology,               // NVLink vs PCIe info
}

impl NcclCluster {
    pub fn init() -> Result<Self, NcclError> {
        // 1. Read EFE_NCCL_RANKS env or autodetect via mpirun-style
        let world_size = std::env::var("EFE_NCCL_WORLD_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| count_visible_gpus());
        let rank = std::env::var("EFE_NCCL_RANK")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        // 2. ncclUniqueId broadcast via etcd/k8s sidecar
        let unique_id = if rank == 0 {
            let id = nccl::get_unique_id()?;
            broadcast_unique_id(&id)?;
            id
        } else {
            wait_for_unique_id()?
        };
        // 3. ncclCommInitRank
        let comm = nccl::Comm::init(world_size, &unique_id, rank)?;
        // 4. Topology probe (= NVML query)
        let topology = NcclTopology::probe()?;
        Ok(Self { rank, world_size, comm, streams: vec![], topology })
    }

    pub fn all_reduce_sum<T: NcclElem>(
        &self,
        send: &CudaTensor<T>,
        recv: &mut CudaTensor<T>,
        stream: &CudaStream,
    ) -> Result<(), NcclError> {
        self.comm.all_reduce(send, recv, nccl::ReduceOp::Sum, stream.raw())?;
        observability::nccl_allreduce_seconds.observe(duration_since.last_op);
        Ok(())
    }
}
```

### 9.5 Fault-tolerance

| Failure | Detection | Recovery |
|---------|-----------|----------|
| 1 GPU lost (= drain) | `ncclAllReduce` returns NCCL_FAIL | Mark replica unhealthy ; drain ; K8s replaces pod |
| NCCL hang (= deadlock) | Timeout 10 s | Abort comm, restart process |
| PCIe link degraded | Latency p99 ↑ 10× | Alert `MULTI_GPU_DEGRADED` ; auto-shed |
| GPU heat throttling | NVML reports `THERMAL_LIMIT` | Alert ops ; throttle batch size |

---

## 10. Surface 6 — Observabilité (10 instruments + flame graphs)

### 10.1 6 instruments existants (`observability.rs` 222 LOC)

D'après l'archi § 18 :

1. `efe_gpu_kernel_duration_seconds` Histogram
2. `efe_gpu_memory_used_bytes` Gauge
3. `efe_gpu_utilization_ratio` Gauge
4. `efe_gpu_kernel_calls_total` Counter
5. `efe_gpu_oom_total` Counter
6. `efe_gpu_nccl_allreduce_seconds` Histogram

### 10.2 4 instruments à ajouter (Φ-D complétion)

| # | Métrique | Type | Labels | Source |
|--:|----------|------|--------|--------|
| 7 | `efe_gpu_pinned_host_pool_bytes` | Gauge | (no labels) | pinned host pool |
| 8 | `efe_gpu_arena_reset_total` | Counter | `pool` | per-stream arena resets |
| 9 | `efe_gpu_sm_occupancy_ratio` | Gauge | `gpu_id` | nvml-rs query 1×/s |
| 10 | `efe_gpu_driver_version_info` | Gauge | `driver`, `cuda`, `nccl` | au boot |

### 10.3 Flame graphs intégrés

Pour le debug post-mortem :
- **`nsight-compute` profile** automatique sur les 1 % de
  requêtes les plus lentes (= triggered par alerte `P1_BURN_FAST`)
- Profile uploadé vers S3 + lien dans la trace OTel
- Format `.ncu-rep` pour analyse offline

### 10.4 Dashboard Grafana `efe-gpu-kernels`

Panels :
1. p99 kernel duration par kernel (12 panels)
2. VRAM used % par GPU
3. SM occupancy par GPU
4. NCCL all-reduce latency
5. OOM events / hour
6. Pinned host pool utilization
7. Top 10 slowest requests (= drill into NSight profiles)

---

## 11. Surface 7 — Défensive (8 garde-fous GPU runtime)

### 11.1 Inventaire

| # | Garde-fou | Localisation | Effet |
|--:|-----------|--------------|-------|
| 1 | **VRAM OOM circuit breaker** | `memory.rs` | Shrink batch + retry 1 fois ; sinon fail-fast |
| 2 | **Kernel launch timeout** | `streams.rs` | 100 ms max par kernel (= 10× cible) → abort |
| 3 | **NaN/Inf scan post-kernel** | `auto_check_finite` feature | Off prod, on staging ; alerte si trouvé |
| 4 | **GPU heat throttling detector** | NVML probe 1×/s | Alert + auto-shed si throttling actif |
| 5 | **Driver compat check au boot** | `runtime.rs` | Refuse boot si CUDA driver < 525 |
| 6 | **PCIe bandwidth degradation** | benchmark au boot | Alerte si bw < 12 GB/s (= < 50 % expected) |
| 7 | **Multi-GPU rank consistency** | NCCL init | Tous les ranks ont même `world_size` ou abort |
| 8 | **Per-tenant VRAM quota** | `memory.rs` (D11 isolation) | Reject requests si tenant dépasse quota VRAM |

### 11.2 Détail garde-fou #1 — VRAM OOM circuit breaker

```rust
impl CudaBackend {
    pub async fn try_run_with_oom_retry<F, T>(&self, f: F) -> Result<T, BackendError>
    where
        F: Fn(usize) -> Result<T, OomError>,
    {
        let mut batch_size = self.default_batch_size();
        for attempt in 0..3 {
            match f(batch_size) {
                Ok(v) => return Ok(v),
                Err(OomError { .. }) if attempt < 2 => {
                    // Shrink + retry
                    batch_size = batch_size / 2;
                    observability::oom_retry_total.inc();
                    tracing::warn!("OOM ; retrying with batch={batch_size}");
                    continue;
                }
                Err(e) => return Err(BackendError::OomNoRecovery(e)),
            }
        }
        Err(BackendError::OomNoRecovery(OomError::default()))
    }
}
```

---

## 12. Surface 8 — Bench harness (Criterion + ghz + nightly soak)

### 12.1 12 benches Criterion (= 1 par kernel)

```
crates/efe-kernels/benches/
├── fused_softmax_entropy_bench.rs
├── weighted_mean_bench.rs
├── cone_project_bench.rs
├── l2_normalize_bench.rs
├── cosine_similarity_bench.rs
├── l2_distance_bench.rs
├── mean_pooling_bench.rs
├── argmax_bench.rs
├── topk_bench.rs
├── kl_divergence_bench.rs
├── shannon_entropy_bench.rs
└── moe_reconcile_bench.rs

Chacun bench :
  - 5 batch sizes (1, 4, 16, 32, 64)
  - 3 seq lens (= K) (64, 256, 1024)
  - fp32 et fp16
  → ~30 benches × 12 kernels = 360 datapoints / nightly run
```

### 12.2 1 bench end-to-end fusion

```
crates/efe-kernels/benches/
└── e2e_fusion_round_bench.rs    (= 10 rounds × N=128 × D=384)
                                  Cible : p99 ≤ 100 ms
```

### 12.3 Bench harness ghz (= reproduit P1 hot-path)

```yaml
# deploy/load-tests/gpu/fusion_sustained.yaml
proto: ../../crates/efe-server/proto/v1/swarm.proto
call: efe.v1.SwarmService.Execute
total: 240000
concurrency: 64
rps: 200
duration: 20m
host: efe-server-gpu-0:50051
```

### 12.4 24 h soak test

```bash
# Sur le runner GPU T4 self-hosted, nightly :
$ ghz --config deploy/load-tests/gpu/soak_24h.yaml &
$ pid=$!

# Monitoring :
$ while kill -0 $pid; do
    # 1×/min : log VRAM + RSS + NaN scan
    nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits >> vram.csv
    ps -o rss= -p $pid >> rss.csv
    sleep 60
  done

# Analyse :
$ python scripts/bench/analyze_soak.py vram.csv rss.csv
  → vram drift < 10 MiB/h ? ✓
  → rss drift < 5 MiB/h ? ✓
  → NaN/Inf count = 0 ? ✓
```

---

## 13. Surface 9 — SLO + gating CI (6 SLO Φ-D nouveaux)

### 13.1 Inventaire des 6 SLO Φ-D

| ID | Nom | Comparator | Seuil | Source |
|----|-----|:----------:|------:|--------|
| **SLO-CUDA-FUSION-ROUND** | p99 fusion round (T4 fp16 N=128) | lte | 10 ms | `e2e_fusion_round_bench.rs` |
| **SLO-CUDA-FUSION-TOTAL** | p99 fusion 10 rounds | lte | 100 ms | idem |
| **SLO-CUDA-SPEEDUP-CPU** | speedup vs CpuBackend | gte | 5.0× | `bench fusion_cpu_vs_gpu` |
| **SLO-CUDA-NUMERICAL** | rtol max sur 12 kernels fp16 | lte | 1e-3 | `tests/numerical_equivalence.rs` |
| **SLO-CUDA-MULTI-GPU-SPEEDUP** | speedup 4-GPU vs 1-GPU (N=1024) | gte | 3.0× | bench multi-GPU |
| **SLO-CUDA-SOAK-24H** | RSS drift soak | lte | 10 MiB/h | nightly soak |

### 13.2 CI workflow GPU nightly

```yaml
# .github/workflows/bench-cuda-kernels.yml
name: Bench CUDA Kernels (Φ-D)
on:
  schedule:
    - cron: '0 2 * * *'    # nightly 02:00 UTC
  workflow_dispatch:

jobs:
  cuda-build:
    runs-on: [self-hosted, gpu-t4]
    timeout-minutes: 60
    steps:
      - uses: actions/checkout@v4
      - name: Build with cuda
        run: cargo build --workspace --features cuda --release
      - name: Numerical equivalence harness
        run: cargo test --features cuda --test numerical_equivalence --release
      - name: Bench all 12 kernels
        run: cargo bench --features cuda --bench '*_bench'
      - name: SLO check
        run: |
          python eval/scripts/check_slo.py \
            --baseline bench_baselines/v0.6.0/cuda_slos.yaml \
            --measure target/criterion-report.json
```

---

## 14. Surface 10 — Contractuelle (= attestation hardware fingerprint)

### 14.1 Le `host_fingerprint_sha256` GPU-extended

L'attestation P1 (`EFE_P1_SURFACE_ARCHITECTURE.md` § 1.5) requiert
un `host_fingerprint`. Pour Φ-D, on étend pour inclure :

```rust
pub struct GpuFingerprint {
    pub driver_version: String,             // "535.104.05"
    pub cuda_runtime_version: String,       // "12.4"
    pub cuda_compile_version: String,       // "12.4" (= au build time)
    pub gpu_model: String,                  // "NVIDIA T4"
    pub gpu_compute_capability: (u8, u8),   // (7, 5)
    pub gpu_vram_gib: u32,                  // 16
    pub nccl_version: Option<String>,       // "2.21.5"
    pub efe_kernels_cuda_sha256: [u8; 32],  // hash de la lib statique
}
```

### 14.2 Clause SLA opposabilité

> §3.5 — Le `gpu_fingerprint_sha256` est inclus dans le rapport
> SLA mensuel. Tout changement (= driver upgrade, GPU swap, etc.)
> ré-ouvre la baseline et SLO-C-1 est temporairement suspendu
> jusqu'à re-baselining (= ≤ 24 h ouvrables).

---

## 15. Matrice complète des artefacts (gap-filling)

| # | Artefact | Statut actuel | Statut cible | Sprint |
|--:|----------|:--:|:--:|:--:|
| 1 | `docs/EFE_CUDA_KERNELS_COMPLETION_ARCHITECTURE.md` (ce doc) | ❌ | ✅ | D28 |
| 2 | `build.rs` multi-arch fatbin + sccache | ⚠️ | ✅ | D28 |
| 3 | 12 kernels `.cu` complets (= bodies + tests) | ⚠️ | ✅ | D28-D29 |
| 4 | `tests/numerical_equivalence.rs` (12 kernels × 5 cases) | ❌ | ✅ | D29 |
| 5 | `tests/property_based.rs` (= softmax + l2_norm) | ❌ | ✅ | D29 |
| 6 | `crates/efe-kernels/src/memory.rs` — pinned host pool | ⚠️ | ✅ | D29 |
| 7 | OOM circuit breaker + 4 chaos tests | ❌ | ✅ | D30 |
| 8 | `multi_gpu.rs` real NCCL impl | ⚠️ | ✅ | D30 |
| 9 | 4 nouveaux Prometheus instruments (#7-10) | ❌ | ✅ | D30 |
| 10 | Grafana dashboard `efe-gpu-kernels.json` (12 panels) | ❌ | ✅ | D30 |
| 11 | 12 benches Criterion (1 / kernel) | ❌ | ✅ | D29-D30 |
| 12 | 1 bench e2e_fusion_round | ❌ | ✅ | D30 |
| 13 | `.github/workflows/bench-cuda-kernels.yml` (nightly) | ❌ | ✅ | D30 |
| 14 | `bench_baselines/v0.6.0/cuda_slos.yaml` (6 SLO) | ❌ | ✅ | D30 |
| 15 | `deploy/load-tests/gpu/fusion_sustained.yaml` | ❌ | ✅ | D31 |
| 16 | `deploy/load-tests/gpu/soak_24h.yaml` + analyse script | ❌ | ✅ | D31 |
| 17 | `docs/runbook/gpu_oom_recovery.md` | ❌ | ✅ | D31 |
| 18 | `docs/runbook/gpu_driver_upgrade.md` | ❌ | ✅ | D31 |
| 19 | `docs/runbook/multi_gpu_failover.md` | ❌ | ✅ | D31 |
| 20 | `docs/runbook/cuda_kernel_regression.md` | ❌ | ✅ | D31 |
| 21 | `efe-server tune` subcommand (= auto-detect GPU + emit env) | ❌ | ✅ | D32 |
| 22 | 8 chaos tests (= GPU drain, NCCL hang, thermal, etc.) | ❌ | ✅ | D31 |
| 23 | Streaming RPC (cf. P1 § 3.5 fused softmax-style streaming) | ⚠️ | ✅ | D32 |
| 24 | Hybrid fusion (= CPU+GPU shadow validation) | ❌ | ✅ | D32 |
| 25 | `host_fingerprint_sha256` GPU-extended | ❌ | ✅ | D32 |
| 26 | Audit interne 8+ bugs | ❌ | ✅ | D32 |
| 27 | CHANGELOG v0.6.0 entry | ❌ | ✅ | D32 |
| 28 | Tag git `v0.6.0` | ❌ | ✅ | D32 |

→ **28 artefacts** dont 14 totalement absents, 14 partiellement en
place. Plan d'attaque § 16.

---

## 16. Plan d'implémentation D28 → D32 chirurgical

### D28 — Sprint Build + 3 kernels P0 (10 j wallclock)

| Jour | Tâche |
|----:|-------|
| J1 | Doc archi (ce doc) ; identifier les 12 kernels + cibles p99 |
| J2 | `build.rs` extension : multi-arch fatbin + sccache + signature check |
| J3 | Provisioning runner self-hosted GPU T4 (= AWS g4dn.xlarge) |
| J4 | Kernel `fused_softmax_entropy` (body + Rust wrapper) |
| J5 | Kernel `weighted_mean` (batched) |
| J6 | Kernel `cone_project` |
| J7 | Numerical equivalence harness skeleton (3 kernels) |
| J8 | First benches Criterion (3 kernels) |
| J9 | Audit interne sprint (= 4-8 bugs typiques) |
| J10 | Tag `v0.4.0-rc1` + run nightly GPU first time |

### D29 — Sprint 9 kernels restants + property tests (10 j)

| Jour | Tâche |
|----:|-------|
| J1-J3 | 6 kernels P1 + P2 (l2_normalize, cosine, l2_distance, mean_pooling, argmax, topk) |
| J4-J5 | 3 kernels P3 (kl_div, shannon_entropy, moe_reconcile) |
| J6 | Property-based tests softmax + l2_normalize |
| J7 | Pinned host pool implementation |
| J8 | OOM circuit breaker + tests chaos |
| J9-J10 | Sprint audit + tag `v0.4.0` (= bundle de tous les kernels) |

### D30 — Sprint Multi-GPU + observabilité (10 j)

| Jour | Tâche |
|----:|-------|
| J1-J3 | NCCL real impl + tests cross-rank |
| J4 | 4 nouveaux instruments Prometheus |
| J5 | Grafana dashboard `efe-gpu-kernels.json` |
| J6 | `bench_baselines/v0.5.0/cuda_slos.yaml` (6 SLO) |
| J7 | Workflow `bench-cuda-kernels.yml` nightly |
| J8 | First multi-GPU bench (= 4× T4) |
| J9-J10 | Audit + tag `v0.5.0` (= multi-GPU MVP) |

### D31 — Sprint Soak + Runbooks (10 j)

| Jour | Tâche |
|----:|-------|
| J1-J2 | 24 h soak test setup + first run |
| J3-J4 | 4 runbooks ops (oom, driver_upgrade, multi_gpu_failover, kernel_regression) |
| J5-J7 | 8 chaos tests (= GPU drain, NCCL hang, thermal, PCIe degrade, etc.) |
| J8 | `deploy/load-tests/gpu/*` yaml configs |
| J9-J10 | Audit + sprint freeze (= 0 P0/P1 bug) |

### D32 — Sprint Streaming + Tune + GA freeze (10 j)

| Jour | Tâche |
|----:|-------|
| J1-J3 | Streaming RPC implementation (cf. P1 § 3.5) |
| J4-J5 | Hybrid fusion (= CPU+GPU shadow validation) |
| J6 | `efe-server tune` subcommand |
| J7 | `host_fingerprint_sha256` GPU-extended |
| J8 | Final audit interne (= 8+ bugs résolus) |
| J9 | CHANGELOG v0.6.0 + freeze |
| J10 | **Tag `v0.6.0`** — Φ-D 100 % complete |

---

## 17. Risques identifiés & mitigations (20 risques)

| # | Risque | Sévérité | Probabilité | Mitigation |
|--:|--------|:--------:|:-----------:|------------|
| 1 | Custom kernels < 2× speedup vs ONNX CUDA EP | HIGH | MED | Plan B = utiliser ONNX CUDA EP générique ; SLO-C-1 reste 200 ms via Bundle § 6 |
| 2 | fp16 overflow / underflow non-prévu | HIGH | LOW | Numerical equiv harness + property tests + NaN scan |
| 3 | NCCL hangs en multi-GPU | HIGH | MED | Timeout 10 s + abort comm + alerts |
| 4 | Build CI > 30 min wallclock | MED | HIGH | sccache + parallel `nvcc -j` |
| 5 | T4 runner GPU CI outage | MED | MED | Backup runner L4 + skip gracefully |
| 6 | NVIDIA driver ABI break entre versions | HIGH | LOW | min/max version pinned ; alert au boot |
| 7 | Cross-arch CUBIN bloated (= 5 archs × 12 kernels) | LOW | MED | PTX fallback compress (~ 60 % smaller) |
| 8 | Cudarc 0.10 ABI break en 0.11 | MED | LOW | Pin patch version + tests cross-version |
| 9 | VRAM fragmentation (= long-running process) | MED | MED | Arena reset + memory profiler alerts |
| 10 | Memory leak silent (RSS drift 24h) | HIGH | LOW | Soak test gate + RSS panic threshold |
| 11 | Power throttling silent perf hit | MED | MED | NVML probe + alert si throttling > 10 % |
| 12 | Multi-tenant noisy neighbor | HIGH | MED | Per-tenant VRAM quota + rate limit |
| 13 | NCCL ne marche pas sur certains réseaux cloud | MED | LOW | Document network requirements (RDMA preferred) |
| 14 | T4 EOL (~ 2027) ; nouveaux GPUs SM différents | LOW | HIGH | Cover sm_75 to sm_90 ; plan rotation |
| 15 | Chaos test (= GPU drain) déclenche OOM cascading | HIGH | LOW | Isolation namespace + circuit breaker |
| 16 | nsight-compute profiling impact perf | LOW | MED | Trigger only sur p99 outliers (= 0.1 % traffic) |
| 17 | License (= cuBLASLt closed-source) bloque distribution | LOW | LOW | Apache-2.0 wrapping ; cf. NVIDIA EULA |
| 18 | Kernel cache poisoning (= JIT compromise) | LOW | LOW | Signed CUBIN + hash verification |
| 19 | NaN propagation cross-rank silent | HIGH | LOW | check_finite gate auto-shed |
| 20 | Apparent perf regression in real workload | HIGH | MED | Hybrid fusion shadow validation + auto-rollback |

---

## 18. Décisions de conception clés (18)

| # | Décision | Alternative rejetée | Justification |
|--:|----------|---------------------|---------------|
| 1 | `cudarc` 0.10 (vs `cust`, vs raw FFI) | raw FFI | Safe wrappers ; entretien actif ; nccl + cuBLAS integré |
| 2 | Multi-arch fatbin (vs PTX-only) | PTX-only | Latence boot ≤ 10 s ; PTX JIT = 60-120 s cold |
| 3 | Trait `ComputeBackend` (= partage code CPU/GPU) | duplicate code | Numerical equiv harness ; CPU est ground-truth |
| 4 | Per-stream arena bump allocator | malloc/free | Reset O(1) ; pas de frag |
| 5 | NCCL all-reduce (vs custom) | custom | Production-tested ; topology-aware |
| 6 | fp16 par défaut, fp32 fallback | fp32 partout | 35 % speedup ; tolerance acceptable |
| 7 | 1 stream / Tokio worker (= 4) | 1 stream global | Concurrence vraie + pas de contention |
| 8 | Sigstore Rekor pour `efe_kernels_cuda` SHA | self-host attest | Transparency log ; tier-attestable |
| 9 | T4 = baseline contractuelle (vs A100) | A100 baseline | T4 plus accessible ; A10/A100 = upgrade transparente |
| 10 | NCCL 4-GPU max v1.0 (= simple topology) | unbounded | T4 cluster typique ; A100 multi-node = v2.0 |
| 11 | Numerical tolerance rtol 1e-3 fp16 | 1e-2 | Stricte pour catch des bugs subtils |
| 12 | Soak 24 h obligatoire pour tag | smoke seulement | Catch les leaks lents (= multi-GPU NCCL slow) |
| 13 | Hybrid fusion (CPU shadow) en canary | direct GPU | Safety net ; auto-rollback si divergence |
| 14 | Auto-tune via env vars (vs persistent config) | persistent | Stateless ; supportable en debug |
| 15 | nsight profiling auto sur p99 outliers | manual seulement | Debug efficace post-mortem |
| 16 | `efe-server tune` subcommand | doc seulement | One-shot calibration ; reproducible |
| 17 | host_fingerprint inclut `efe_kernels_cuda_sha256` | identity hash only | Détect tampering binary post-build |
| 18 | Per-tenant VRAM quota (= D11 extension) | shared pool | Multi-tenant isolation contractuelle |

---

## 19. Anti-régression vs CPU fallback

### 19.1 Invariant : CPU + GPU = même résultat

Le `CpuBackend` reste la **ground-truth**. Tout résultat GPU
non-équivalent au CPU (= au-delà de la tolérance fp16 1e-3) est
**un bug bloquant**.

### 19.2 Gate de release

Aucun tag `v0.6.0` ne peut être créé si :
- `numerical_equivalence.rs` fail sur n'importe quel kernel
- `property_based.rs` fail
- Speedup CUDA < 2× sur n'importe quel kernel critique
- Soak 24 h leak > 10 MiB/h
- Au moins 1 SLO Φ-D rouge sur 7 jours consécutifs nightly

### 19.3 Hybrid fusion mode (= production canary)

Pendant 30 jours post-v0.6.0, le serveur tourne en **hybrid mode** :
- 1 % du trafic CPU shadow (= results compared cross-backend)
- Si > 0.1 % de mismatch → alerte critical + auto-rollback

---

## 20. Critères d'acceptation (DoD à 100 %)

### 20.1 Code

- [ ] 28 artefacts livrés (§ 15).
- [ ] `cargo build --workspace --features cuda` vert.
- [ ] 12 kernels CUDA bodies opérationnels (= ≠ stubs).
- [ ] Numerical equivalence harness PASS sur 12 kernels × 3 archs.
- [ ] Property-based tests PASS (= softmax + l2_norm).
- [ ] OOM circuit breaker + 4 chaos tests PASS.
- [ ] NCCL real impl (vs stub).

### 20.2 CI

- [ ] Job `bench-cuda-kernels.yml` nightly tourne sur runner T4.
- [ ] 6 SLO Φ-D gatés dans `check_slo.py`.
- [ ] Failure d'un SLO bloque la release tag.
- [ ] 24 h soak auto nightly + analyse RSS/VRAM uploadée.

### 20.3 Mesure live

- [ ] **p99 fusion round ≤ 10 ms** (SLO-CUDA-FUSION-ROUND) sur T4 fp16.
- [ ] **p99 fusion 10 rounds ≤ 100 ms** (SLO-CUDA-FUSION-TOTAL).
- [ ] **Speedup ≥ 5× vs CPU MVP** (SLO-CUDA-SPEEDUP-CPU).
- [ ] **rtol max ≤ 1e-3 fp16** (SLO-CUDA-NUMERICAL).
- [ ] **Multi-GPU 4× speedup ≥ 3×** (SLO-CUDA-MULTI-GPU-SPEEDUP).
- [ ] **RSS drift ≤ 10 MiB/h** soak (SLO-CUDA-SOAK-24H).

### 20.4 Documentation

- [ ] `docs/EFE_CUDA_KERNELS_COMPLETION_ARCHITECTURE.md` (ce doc).
- [ ] 4 runbooks `docs/runbook/gpu_*.md`.
- [ ] CHANGELOG v0.6.0 entry détaillée.
- [ ] `efe-server tune --help` documenté.

### 20.5 Contractuel

- [ ] `host_fingerprint_sha256` étendu inclut GPU info.
- [ ] Clause SLA §3.5 (= GPU fingerprint drift) validée juridiquement.
- [ ] Premier audit firm tier-1 atteste Φ-D.

### 20.6 Audit interne

- [ ] 8+ bugs trouvés et résolus durant Φ-D.
- [ ] 0 P0/P1 bug open au tag v0.6.0.
- [ ] No clippy warning sur `efe-kernels` (= `unwrap_used = deny`).

---

## 21. Hand-off & dépendances downstream

### 21.1 Dépendances entrantes

| Sprint | Livré | Ce que Φ-D consomme |
|--------|-------|---------------------|
| Φ-C (D25-D27) | Bundle réel MiniLM | Inputs ONNX pour bench GPU end-to-end |
| D16 | Bundle format | Loading mechanic |
| P1 | Latence opposable | SLO-P1-BUDGET-5 + 7 sources |
| D11 | Per-tenant isolation | VRAM quota per-tenant |
| D14-bis | Audit chain | Trace de chaque GPU verdict |

### 21.2 Dépendances sortantes

| Sprint downstream | Ce qu'il prend de Φ-D |
|-------------------|------------------------|
| Φ-E (D33-D36) | Budget GPU dispo pour Mode-2 (= 1500-200 = 1300 ms pour LLM) |
| Φ-F (D37-D39) | Multi-GPU scaling pour Mode-3 |
| Φ-G (D40-D42) | Attestation hardware fingerprint extended |
| GA v1.0.0 | SLO-C-1 200 ms p99 GPU PASS |

### 21.3 Risque ordre des sprints

Si Φ-D dépasse D32 :
- Φ-E (Mode-2) ne peut pas démarrer (= dépend du budget GPU)
- Décision : ship Φ-D partial (= 8 kernels au lieu de 12) si SLO
  primaire (= SLO-CUDA-FUSION-ROUND) PASS, et marquer P3 priorité
  kernels (#10, #11, #12) comme v1.1.

---

## 22. Annexes (A → H)

### Annexe A — Liste exhaustive des 6 SLO Φ-D (YAML)

```yaml
slos:
  - id: SLO-CUDA-FUSION-ROUND
    description: "p99 swarm fusion round (T4 fp16 N=128 D=384)"
    quantile: 0.99
    comparator: lte
    threshold_seconds: 0.010
    status: ACTIVE
    source: bench_e2e_fusion_round.rs

  - id: SLO-CUDA-FUSION-TOTAL
    description: "p99 swarm fusion 10 rounds total"
    quantile: 0.99
    comparator: lte
    threshold_seconds: 0.100
    status: ACTIVE

  - id: SLO-CUDA-SPEEDUP-CPU
    description: "Speedup CudaBackend vs CpuBackend (= fusion E2E)"
    comparator: gte
    threshold_ratio: 5.0
    status: ACTIVE

  - id: SLO-CUDA-NUMERICAL
    description: "Max rtol cross-arch sur 12 kernels fp16"
    comparator: lte
    threshold_ratio: 1.0e-3
    status: ACTIVE

  - id: SLO-CUDA-MULTI-GPU-SPEEDUP
    description: "4× T4 speedup vs 1× T4 sur N=1024"
    comparator: gte
    threshold_ratio: 3.0
    status: ACTIVE

  - id: SLO-CUDA-SOAK-24H
    description: "RSS drift 24h soak Mode-1 200 RPS"
    comparator: lte
    threshold_bytes_per_hour: 10485760    # 10 MiB
    status: ACTIVE
```

### Annexe B — Glossaire Φ-D

| Terme | Définition |
|-------|------------|
| **CUBIN** | Compiled binary CUDA pour une SM specific (= sm_75 etc.) |
| **PTX** | Portable bytecode CUDA ; JIT-compilable au runtime par driver |
| **fatbin** | Container multi-arch contenant plusieurs CUBINs + PTX fallback |
| **SM** | Streaming Multiprocessor (= unit de compute GPU NVIDIA) |
| **NCCL** | NVIDIA Collective Communications Library (= MPI pour GPUs) |
| **NVML** | NVIDIA Management Library (= telemetry GPU runtime) |
| **fp16** | 16-bit float IEEE half precision |
| **bf16** | brain-float 16 (= TPU + Ampere+ native) |
| **All-reduce** | Op collective : sum à tous les ranks (= consensus swarm) |
| **HBM** | High-Bandwidth Memory (= VRAM sur GPU server-class) |
| **PCIe** | Bus interconnect host-GPU ; NVLink est ~ 6× plus rapide |
| **rtol** | Relative tolerance (= `|a-b| / |b|`) pour numerical equiv |

### Annexe C — Hardware fingerprint étendu (JSON)

```json
{
  "host_fingerprint_sha256": "0a3b...c91f",
  "components": {
    "cpu_model": "Intel(R) Xeon(R) Gold 6248 @ 2.50 GHz",
    "cpu_cores": 8,
    "ram_gib": 32,
    "gpu": {
      "model": "NVIDIA T4",
      "vram_gib": 16,
      "compute_capability": "7.5",
      "driver_version": "535.104.05",
      "cuda_runtime_version": "12.4",
      "cuda_compile_version": "12.4",
      "nccl_version": "2.21.5",
      "efe_kernels_cuda_sha256": "abc...123"
    },
    "kernel": "Linux 5.15.0-86-generic",
    "rustc": "rustc 1.88.0",
    "container_image_sha": "sha256:abc..."
  }
}
```

### Annexe D — Commandes de vérification rapide

```bash
# Build with CUDA
$ cargo build --workspace --features cuda --release

# Run numerical equivalence harness
$ cargo test --features cuda --test numerical_equivalence --release

# Run all bench Criterion
$ cargo bench --features cuda --bench '*_bench'

# Run SLO check
$ python eval/scripts/check_slo.py \
    --baseline bench_baselines/v0.6.0/cuda_slos.yaml \
    --measure target/criterion-report.json

# 24h soak
$ ghz --config deploy/load-tests/gpu/soak_24h.yaml \
    --output /var/log/efe-soak-$(date +%Y%m%d).log &

# Auto-tune GPU
$ ./target/release/efe-server tune --gpu-type t4 \
    > /etc/efe/gpu-tuned.env

# Verify host fingerprint
$ ./target/release/efe-server fingerprint
```

### Annexe E — Mapping vers PRODUCT_COMPLETION C-*

| Contrat | Description | Couvert par Φ-D |
|---------|-------------|------------------|
| C-1 | Mode-1 GPU p99 ≤ 200 ms | **Directement** : SLO-CUDA-FUSION-TOTAL + bundle |
| C-2 | Mode-1 CPU p99 ≤ 800 ms | Indirect : CpuBackend reference |
| C-5 | Coût par classify ≤ 0.5 ¢ | Indirect : speedup 5× réduit coût compute |
| C-6 | Tokens LLM saved ≥ 90 % | n/a |
| C-7 | Confidence calibration Brier ≤ 0.10 | Indirect : numerical equivalence garantit même résultats |

### Annexe F — Procédure rotation GPU driver

Cf. `docs/runbook/gpu_driver_upgrade.md` (= livré D31).

### Annexe G — Roadmap v1.1+ post-GA Φ-D

| Version | Évolution |
|---------|-----------|
| v0.6.0 (GA Φ-D) | Multi-arch T4/A10/A100, 12 kernels, NCCL 4-GPU max |
| v0.7.0 | bf16 path (Ampere+) ; AMP autocast |
| v0.8.0 | NVLink-aware NCCL tree topology |
| v1.1.0 | int8 quantization (post-validation) |
| v1.2.0 | Multi-node NCCL (= 8+ GPUs) |
| v2.0.0 | Apple Silicon Metal substrate (= scope séparé) |

### Annexe H — Mode opérationnel "hybrid fusion" (canary)

Pendant les 30 jours post-tag v0.6.0 :
1. 99 % du trafic Mode-1 sur GPU.
2. 1 % shadow CPU (= bench parallèle).
3. Mismatch GPU↔CPU > 0.1 % → auto-rollback vers CPU only.
4. T+30 j : si 0 mismatch, hybrid mode désactivé ; GPU full prod.

---

— **fin du document** —
