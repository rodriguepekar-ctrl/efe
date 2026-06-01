# EFE — Performance Benchmarks

> **Statut** : v1.6, baselines `v0.1.0..v0.1.4` + `v0.1.6` (D6 accuracy infrastructure).
> Sprint D6 a câblé l'eval golden + ajouté 5 SLO `D6-*` (accuracy / macro-F1 / confiance
> / false-reject / latence client). L'accuracy gate FAIL en attendant D7 — signal
> volontaire pour driver la prochaine itération.
> **Mises à jour** : à chaque release majeure et automatiquement par
> CI à chaque merge sur `main` via `.github/workflows/bench.yml`.
> **Référence SLO** : voir [`SLO.md`](SLO.md).
> **Catalog** : la liste canonique des benches vit dans [`scripts/benches.list`](../scripts/benches.list) (Sprint D5).
> **Architecture** :
> - Sprint C (mocks) → [`docs/bench/bench_foundation_architecture.md`](docs/bench/bench_foundation_architecture.md)
> - Sprint D1 (production) → [`docs/bench/d1_real_strategies_architecture.md`](docs/bench/d1_real_strategies_architecture.md)
> - Sprint D2 (infrastructure mémoire) → [`docs/bench/d2_memory_real_arena_architecture.md`](docs/bench/d2_memory_real_arena_architecture.md)
> - Sprint D3 (compute pipeline) → [`docs/bench/d3_compute_pipeline_architecture.md`](docs/bench/d3_compute_pipeline_architecture.md)
> - Sprint D4 (gRPC server stack) → [`docs/bench/d4_server_rpc_architecture.md`](docs/bench/d4_server_rpc_architecture.md)
> - Sprint D5 (orchestration unifiée) → [`docs/bench/d5_unified_bench_orchestration_architecture.md`](docs/bench/d5_unified_bench_orchestration_architecture.md)

## 0. Sprint D1 — Stratégies de production (v0.1.1)

À partir de la release **v0.1.1**, EFE expose **deux suites de benches**
qui mesurent deux choses **différentes mais complémentaires** :

| Suite | Crate | Wiring | Couverture | Cas d'usage |
|-------|-------|--------|-----------:|-------------|
| `swarm_e2e` / `memory_e2e` | `efe-core` | **mocks** | ~5 % | Overhead pur du `SwarmController` — détecte les régressions d'orchestration |
| `real_strategies_e2e` / `real_memory` | `efe-server` | **`build_mvp_backend`** (production) | ~95 % | Ce que les clients gRPC paient réellement — gate les SLO de release |

**Lecture** : les SLO du § 2 (« Latence end-to-end ») sont désormais
calibrés sur la suite **réelle**, pas sur les mocks. Les mocks restent
benchés pour suivre l'overhead d'orchestration en isolation, mais leurs
chiffres sont à prendre avec la même précaution qu'un sanity test —
ils ne reflètent pas le coût RPC.

Les fixtures **v2** (`scripts/fixtures/realistic_*_v2.json`) ont des
embeddings Gaussiens (Box-Muller, σ = 0.1) au lieu de zéros, ce qui
garantit que `HeuristicMetaAnalyzer` produit un DAG de tâches non
dégénéré. Les fixtures **v1** (`realistic_*.json`) restent utilisées
par les benches mock pour préserver la comparabilité avec la baseline
v0.1.0.

### Cibles SLO réelles (v0.1.1, naïf)

| Spec | Méthode | Cible p95 | Source |
|------|---------|----------:|--------|
| `real_e2e_latency_naive::small` | full request | **≤ 400 ms** | `SLO-RD1-1` |
| `real_e2e_latency_naive::medium` | full request | **≤ 700 ms** | `SLO-RD1-2` |
| `real_e2e_latency_naive::large` | full request | **≤ 700 ms** | `SLO-RD1-3` |
| `real_e2e_throughput_naive::c8` | 8 concurrent (small) | **≤ 700 ms / batch** | `SLO-RD1-T1` |

> Cibles recalibrées Sprint D5 audit sur la multi-round réelle (~170 / ~340 / ~340 ms).

Les cibles **BLAS** (Apple Accelerate / OpenBLAS) sont définies dans
`scripts/parse_criterion_report.py::SLO_DEFINITIONS` et activées par
`cargo bench -p efe-server --features accelerate` (resp. `openblas`).
Elles ciblent un **gain × 2-3** sur `medium` et `large` (cf. § 5).

### Calibration host de référence

Apple M3 Pro (11 cœurs, 36 GB), macOS Darwin 24.x, rustc 1.85.0,
no-BLAS. Les runners CI ubuntu-latest sont ~1.5× plus lents — la
constante `CI_COEFFICIENT` dans `.github/workflows/bench.yml` applique
ce facteur aux cibles SLO automatiquement.

### Anti-régression : `tests/bench_realism_check.rs`

Avant qu'un bench D1 ne tourne, 4 tests d'intégration vérifient :

1. `rounds_executed >= 2` (le détecteur de consensus n'a pas
   court-circuité).
2. La sortie est un `Raw(centroid)` de la bonne dimension et
   non-nul.
3. 20 requêtes successives ne dérivent pas en confidence (preuve que
   `Mode1Backend::execute` reset bien le centroïde).
4. Les fixtures v2 ont une norme d'embedding ∈ ]0.5, 10.0[ (pas de
   regen accidentelle en zéros).

Si l'un trippe, le bench n'est pas exécuté — un nombre menteur est
pire qu'aucun nombre.

## 1. Configuration de référence (baseline `v0.1.0`)

| Champ | Valeur |
|-------|--------|
| Date de mesure | 2026-04-25 |
| Host | Apple M3 Pro, 11 cores, 36 GB |
| OS | Darwin 24.1.0 |
| Toolchain | rustc 1.85.0, cargo 1.85.0 |
| BLAS backend | pure Rust (no `accelerate`) |
| RUSTFLAGS | `-C target-cpu=native` |
| Profile | `bench` (release + debuginfo) |

## 2. Latence end-to-end — `swarm_e2e_<size>_latency`

Mesures du contrôleur (genesis + N rounds + dissolution) avec stratégies
**mockées**. Le bench isole l'overhead d'orchestration ; les chiffres
de calcul réel se trouvent dans les § « micro-benchs » ci-dessous.

| Spec | dim | N | rounds | médian | p95 (≈) | SLO | État |
|------|----:|--:|-------:|-------:|--------:|----:|:----:|
| **small** | 128 | 8 | 1* | **2.9 µs** | **3.0 µs** | 50 ms | ✅ |
| **medium** | 256 | 32 | 1* | **3.6 µs** | **3.6 µs** | 200 ms | ✅ |
| **large** | 256 | 128 | 1* | **4.2 µs** | **4.2 µs** | 1 s | ✅ |

\* La détection de convergence dans le mock retourne `Verdict::Converged`
au round 1 par défaut pour focaliser la mesure sur le coût d'un cycle
complet. Pour stresser le coût multi-rounds, voir
`crates/efe-core/benches/swarm_e2e.rs::bench_e2e_latency`.

### Marges vs SLO

* **SLO-1** (50 ms) : marge × 17 000 — l'overhead d'orchestration
  consomme < 0.01 % du budget.
* **SLO-2** (200 ms) : marge × 55 000.
* **SLO-3** (1 s) : marge × 240 000.

Conclusion : **l'orchestrateur n'est PAS le goulet d'étranglement**.
Quand on remplace les mocks par de vraies stratégies (Sprint Mode 1
Server, ou Phase 2 CUDA), 99.99 % du temps mesuré viendra du compute
réel.

## 3. Débit — `swarm_e2e_<size>_throughput_Nc<n>`

Mesures sous concurrence : N tasks tokio, chacune émettant 1 requête.

| Spec | Nc | médian / batch | req/s extrapolé |
|------|---:|---------------:|----------------:|
| small | 1 | 11.4 µs | ~88 K req/s |
| small | 4 | 24.4 µs (×4 = 97.6 µs) | ~164 K req/s |
| small | 16 | 48.9 µs (×16 = 782 µs) | ~327 K req/s |
| medium | 1 | 12.4 µs | ~80 K req/s |
| medium | 4 | 25.2 µs | ~159 K req/s |
| medium | 16 | 52.6 µs | ~304 K req/s |

**Lecture** : le scaling est presque parfait jusqu'à Nc=4 (4 worker
threads = 4× débit), puis sublinéaire à Nc=16 (sur-engagement,
effets de cache et de scheduling tokio).

### SLO-4 : ≥ 50 req/s @ Nc=4 small

Mesuré : **~164 K req/s** — marge × 3 280. ✅

## 3 bis. Pool / Arena infrastructure (Sprint D2, v0.1.2)

Mesures isolées de `efe_core::memory::{MemoryPool, Arena, HostBackend}`,
qui sous-tend **toute** requête EFE. Lecture clé : `real_e2e_latency_*`
(D1) inclut au minimum 1 acquire warm + 1 reset + 5 alloc_zeroed
(latents + mask + attention + confidence + scratch). Ces chiffres
sont les bornes inférieures théoriques.

### 3 bis.1 Pool acquire

| Régime | Bench key | médian (M3 Pro) | SLO | marge |
|--------|-----------|----------------:|----:|------:|
| warm | `pool_acquire_warm::default` | **80 ns** | ≤ 1 µs (`SLO-D2-1`) | ×12 |
| cold (1 MiB) | `pool_acquire_cold::1MiB` | **5.2 µs** | ≤ 1 ms (`SLO-D2-2`) | ×192 |
| concurrent (8c) | `pool_concurrent_acquire::c8` | **10.3 µs** | ≤ 50 µs (`SLO-D2-3`) | ×5 |
| saturated (4t/2s) | `pool_acquire_saturated::contention_4t_2slot` | **8.5 µs** | n/a (orientation) | — |

**Lecture cold-acquire** : la cible 1 ms anticipe un memset 1 MiB à
la BW UNCACHED de DDR4 (~3 GB/s) sur runners CI. Sur Apple Silicon
unified memory (M3 Pro ~150 GB/s peak), on mesure **5.2 µs** —
×192 sous la cible. Cette marge est volontaire : elle protège
contre les hosts plus lents (CI ubuntu-latest, hosts cloud bas de
gamme) sans nécessiter de re-tuning par déploiement.

### 3 bis.2 Arena allocation (memset bandwidth)

| count (f32) | bytes | médian (M3 Pro) | throughput | SLO |
|-------------|------:|----------------:|-----------:|----:|
| 1 | 256 (aligné) | **19.8 ns** | 192 MiB/s | ≤ 200 ns (`SLO-D2-4`) |
| 1 024 | 4 KiB | **49.8 ns** | 76 GiB/s | ≤ 2 µs (`SLO-D2-5`) |
| 1 000 000 | 4 MB | **23.6 µs** | 157 GiB/s | ≤ 2 ms (`SLO-D2-6`) |

À petit `count` la **structure du coût domine** (cursor.fetch_add +
align_up + DeviceSlice ctor). À grand `count` la **BW memset domine** —
on tape le pic du subsystème mémoire M3 (~157 GiB/s, prefetcher +
parallel write paths + L2 keeping the 4 MB zone). Le bench reporte
en `Throughput::Bytes` pour rendre la lecture immédiate :
> *« Quelle fraction de la BW memset hardware utilise-t-on ? »*

### 3 bis.3 Arena reset

| Bench key | médian | SLO |
|-----------|-------:|----:|
| `arena_reset::default` | **1.8 ns** | ≤ 50 ns (`SLO-D2-7`) ×27 |

Coût : 1 atomic store (cursor) + 1 atomic fetch_add (generation).
Aucun memset (les `alloc_zeroed` ultérieurs s'en chargent). Si une
future feature (zero-on-reset, anti-residue tenant) introduit un
memset ici, ce SLO trippe immédiatement.

### 3 bis.4 Pattern réel (Mvp swarm) par fixture

Reproduit la séquence exacte d'un round `MvpCpuFusion` : acquire +
reset + 5 alloc (latents + mask + attention + confidence + scratch).

| fixture | dim | n_agents | total cursor | médian | SLO |
|---------|----:|---------:|-------------:|-------:|----:|
| small | 128 | 8 | 8 960 B | **260 ns** | ≤ 5 µs (`SLO-D2-8`) ×19 |
| medium | 256 | 32 | ~70 KiB | **780 ns** | ≤ 30 µs (`SLO-D2-9`) ×38 |
| large | 256 | 128 | ~330 KiB | **2.36 µs** | ≤ 150 µs (`SLO-D2-10`) ×64 |

**Validation d'additivité** : `small ≈ pool_acquire_warm (80) + reset
(2) + 5 × alloc_small_arrays (~50 ea)` ≈ 332 ns ; on mesure 260 ns
(branch prediction et cache effects améliorent la séquence
optimisée). Cohérent à 25 % près.

### 3 bis.5 Anti-régression : `tests/pool_realism_check.rs`

4 tests d'intégration gardent le bench D2 :

1. **`pool_acquire_does_not_leak`** — 1 000 cycles → `arenas_active = 0`,
   `arenas_allocated_total ≤ max_arenas`.
2. **`arena_alloc_pattern_matches_expected_cursor_small`** — pin
   chiffré du `TENSOR_ALIGNMENT = 256` (cursor exact = 8 960 B après
   le pattern small).
3. **`pool_saturation_respects_max_arenas`** — 8 tasks vs 2 slots →
   ≤ 2 fresh allocations.
4. **`host_backend_no_dangling_after_pool_drop`** — pool drop libère
   tous les buffers backend.

Si l'un trippe, le bench D2 n'est pas exécuté — la mesure serait
au mieux indicative, au pire trompeuse.

## 3 ter. Compute pipeline (Sprint D3, v0.1.3)

Mesures de la **chaîne réelle de 6 ops** que `MvpCpuFusion::run_round_sync`
exécute, exposée au niveau `ComputeBackend` (pub API) :

```
matmul(Q, K_T) → softmax → matmul(attn, V) → cone_project →
shannon_entropy → std_dev
```

Lecture clé : la chaîne D3 explique **80-90 %** du temps mesuré par
D1 (`real_e2e_latency_*`). Les 10-20 % restants se répartissent entre
D2 (pool/arena) et l'orchestration controller (~5 %).

### 3 ter.1 Chain bench — la métrique-roi

| Régime | Bench key | médian (M3 Pro) | SLO | marge |
|--------|-----------|----------------:|----:|------:|
| small naïve | `compute_pipeline_chain_naive::small` | **14.7 µs** | 200 µs (`SLO-D3-1`) | ×14 |
| medium naïve | `compute_pipeline_chain_naive::medium` | **161 µs** | 500 µs (`SLO-D3-2`) | ×3 |
| large naïve | `compute_pipeline_chain_naive::large` | **1.29 ms** | 20 ms (`SLO-D3-3`) | ×16 |

**Avec `--features accelerate`** (Apple Accelerate, M3 Pro):

| Régime | Bench key | médian attendu | SLO |
|--------|-----------|---------------:|----:|
| small BLAS | `compute_pipeline_chain_blas::small` | ~14.7 µs (no dispatch, M·K·N=8 192 < 32 768) | 200 µs (`SLO-D3-1B`) |
| medium BLAS | `compute_pipeline_chain_blas::medium` | ~80 µs (×2 speedup) | 250 µs (`SLO-D3-2B`) |
| large BLAS | `compute_pipeline_chain_blas::large` | ~300 µs (×4 speedup) | 5 ms (`SLO-D3-3B`) |

### 3 ter.2 Per-phase attribution (medium naïve)

| Phase | Op | médian | % de la chaîne |
|-------|----|-------:|---------------:|
| 1 | matmul (Q·Kᵀ) | 74 µs | 46 % |
| 2 | softmax | 6.3 µs | 4 % |
| 3 | matmul (attn·V) | 20 µs | 13 % |
| 4 | cone_project | 31 µs | 19 % |
| 5 | shannon_entropy | 4.8 µs | 3 % |
| 6 | std_dev | 23 µs | 14 % |
| **Σ** | — | **159 µs** | **99 %** ← validation |
| Chain mesuré | — | **161 µs** | 100 % |

**Validation d'additivité** : 159 / 161 = **98.8 %** de la chaîne
attribuée aux 6 phases. L'écart de 1.2 % couvre la variance criterion
+ allocator churn entre les ops. Sur la fixture *large*, l'additivité
reste à 95 %. Si elle tombe sous 80 % dans le futur, signal de
"composition cost" à investiguer (ex : page faults sur les tenseurs
intermédiaires).

### 3 ter.3 Sweep `n_agents` (dim=256, naïve)

| n_agents | chain time | scaling |
|---------:|-----------:|--------:|
| 8 | 31 µs | — |
| 16 | 70 µs | ×2.2 (≈ 2²/√2) |
| 32 | 161 µs | ×2.3 |
| 64 | 391 µs | ×2.4 |
| 128 | 1.14 ms | ×2.9 |
| 256 | 3.65 ms | ×3.2 |

**Lecture** : entre n=64 et n=256, le scaling tend vers ×4 par
doublement de n (= O(n²)) — confirme que les matmuls dominent à
grand n. À petit n, l'overhead constant (allocator + criterion)
contamine la pente.

### 3 ter.4 Anti-régression : `tests/compute_pipeline_realism.rs`

3 tests d'intégration gardent le bench D3 :

1. **`chain_produces_finite_outputs_on_all_fixtures`** — aucune
   valeur NaN / ±Inf dans aucun tenseur intermédiaire des 3
   fixtures.
2. **`chain_shapes_match_expected`** — pin chiffré sur les shapes
   contractuelles (scores : N×N, attention : N×N, aggregated : N×D,
   projected : N×D, entropy : N, stats : D).
3. **`attention_rows_sum_to_one`** — chaque row d'attention somme
   à 1.0 ± 1e-5 après softmax (détecte un bug d'axe silencieux).

Si l'un trippe, le bench D3 n'est pas exécuté.

## 3 quater. Server RPC stack (Sprint D4, v0.1.4)

Mesures du **wire gRPC réel** sur loopback TCP : tonic codec +
interceptors + bridge + backend. C'est ce qu'un client gRPC paie en
plus de la chaîne backend (D1).

### 3 quater.1 RPC full vs minimal

| Régime | Bench key | médian (M3 Pro) | SLO |
|--------|-----------|----------------:|----:|
| full small | `server_rpc_full_stack::small` | **180 ms** | 500 ms (`SLO-D4-1`) ×2.8 |
| full medium | `server_rpc_full_stack::medium` | **369 ms** | 800 ms (`SLO-D4-2`) ×2.2 |
| full large | `server_rpc_full_stack::large` | **369 ms** | 800 ms (`SLO-D4-3`) ×2.2 |
| minimal small | `server_rpc_minimal_stack::small` | 179 ms | n/a (référence) |
| minimal medium | `server_rpc_minimal_stack::medium` | 395 ms | n/a (référence) |
| minimal large | `server_rpc_minimal_stack::large` | 361 ms | n/a (référence) |

**Lecture critique** : le `Δ_interceptors = full − minimal` est
**sous le seuil de bruit du loopback** (~10 %). Sur les 3 fixtures,
les latences sont dominées par le backend multi-round (~95+ % du
temps), et l'overhead RPC (tonic codec + interceptors + bridge) est
sub-millisecond. Conclusion : *la pile gRPC d'efe-server est
optimale ; aucune action de tuning n'est requise sur cette couche*.

### 3 quater.2 RPC concurrent (small, c=8, minimal stack)

| Bench key | médian (M3 Pro) | throughput | SLO |
|-----------|----------------:|-----------:|----:|
| `server_rpc_concurrent::c8` | **389 ms** | ~20.5 ops/s | 800 ms (`SLO-D4-T1`) ×2.0 |

**Pourquoi `minimal_stack` pour le concurrent** : le rate-limiter
par défaut rejette 8 RPCs simultanés du même tenant *anonyme* —
ce qui produit `Status::ResourceExhausted` et fait planter le bench.
Le concurrent group désactive donc rate-limit et idempotency pour
mesurer la **capacité serveur** sous charge, pas la rejection
behavior. La rejection est testée en intégration séparément.

### 3 quater.3 Codec proto round-trip (encode + decode)

| Fixture | bytes encodés | médian (M3 Pro) |
|---------|--------------:|----------------:|
| small (8/128) | ~5 KiB | **2.3 µs** |
| medium (32/256) | ~36 KiB | **5.8 µs** |
| large (128/256) | ~140 KiB | **11.6 µs** |

À ~150 KiB/11.6 µs = **~12 GB/s codec throughput**. `prost` est
hautement optimisé sur les types répétés (anchors, embedding) ;
le codec ne sera jamais le goulot d'étranglement de la pile EFE.

### 3 quater.4 Anti-régression : `tests/server_rpc_realism_check.rs`

3 tests d'intégration gardent le bench D4 :

1. **`rpc_full_stack_returns_ok_on_all_fixtures`** — RPC OK + body
   well-formed (swarm_id non-vide, confidence ∈ [0, 1], rounds ≥ 1)
   sur les 3 fixtures.
2. **`rpc_idempotency_short_circuits_on_repeat`** — 2e call avec
   même request_id < 1/5 du 1er call (cache hit < 1 ms vs ~180 ms).
3. **`rpc_concurrent_clients_do_not_serialize`** — 8 RPCs concurrents
   < 1/2 du temps série équivalent (preuve de parallelisme).

Si l'un trippe, le bench D4 n'est pas exécuté.

## 4. Mémoire — `memory_e2e_<size>_heap`

Mesures du heap peak (KB) via `peak_alloc::PeakAlloc`. Reset entre
itérations pour capturer le peak local d'un seul `execute()`.

| Spec | médian (timing du capture) | p95 (≈) |
|------|---------------------------:|--------:|
| small | 2.7 µs | 2.7 µs |
| medium | 2.9 µs | 3.0 µs |
| large | 3.3 µs | 3.4 µs |

**Note** : la mesure exposée par le timing criterion est le **temps**
nécessaire pour exécuter `execute()`+`PEAK_ALLOC.peak_usage_as_kb()`,
pas la valeur du peak en lui-même. La valeur peak retournée est
black-boxée pour empêcher l'optimiseur de l'éliminer mais n'est pas
reportée par criterion. Pour la valeur peak en KB, lire la sortie
debug du bench dans `target/bench-memory-e2e.txt`.

### SLO-5 : heap peak medium < 50 MB

Mesuré : **< 5 MB** (mocks ont un footprint minuscule). Marge × 10. ✅

## 5. Comparaison BLAS vs naïf — `swarm_e2e_blas_vs_naive`

| Backend | Spec | médian | p95 (≈) | speedup |
|---------|------|-------:|--------:|--------:|
| `pure_rust` | medium | **3.4 µs** | 3.4 µs | 1.0× (réf) |
| `accelerate` | medium | (à mesurer) | — | — |
| `openblas` | medium | (à mesurer) | — | — |

**Note** : la mesure BLAS nécessite un build dédié
(`cargo bench --features accelerate`). À reproduire via
`scripts/bench_runner.sh BLAS_BACKEND=accelerate`.

Sur les **mocks**, le speedup BLAS est nul (aucun matmul réel n'est
appelé). Le test du speedup BLAS se fait sur les micro-benchs compute
(cf. `compute_sprint4` qui mesure ×40 sur matmul 256²).

## 6. Micro-benchmarks compute (Sprints 2-4)

Référence pour la couche compute — ces chiffres ne sont PAS soumis
aux SLO de ce document, mais documentent le coût des ops sous-jacentes.

| Op | Taille | médian (µs) |
|----|--------|-----------:|
| `matmul` (naïf) | 256² | ~1 350 |
| `matmul` (BLAS Accelerate) | 256² | ~42 |
| `matmul_fused_gelu` | 256² | ~50 |
| `softmax` | 1 024 | ~5 |
| `gelu` | 1 024 | ~3 |
| `cone_project` (B=128, N=10, d=128) | — | ~200 |
| `weighted_mean` (B=4096, d=128) | — | ~85 |
| `moe_reconcile` (B=128, K=16, d=256) | — | ~220 |
| `multinomial_sample` (n=1000) | — | ~32 |

(Source : `cargo bench --bench compute_sprint{2,3,4}`)

## 7. Reproductibilité (Sprint D5 unified runner)

À partir de Sprint **D5**, **tous** les 10 benches sont pilotés
par un **runner unifié** consommant un catalog déclaratif unique
[`scripts/benches.list`](../scripts/benches.list). Ajouter un bench
= 1 ligne dans le catalog ; aucune autre modification dans
`bench_runner.sh`, `bench.yml`, ou `parse_criterion_report.py`.

### 7.1 CLI grammar

```bash
# Defaults : tier=default, MODE=quick, BLAS=none. Run la suite
# headline (~7 benches sur 10) en mode --quick.
scripts/bench_runner.sh

# Filter par bench(s) explicite(s)
BENCHES=swarm_e2e,server_rpc scripts/bench_runner.sh

# Filter par sprint (D2, D3, D4, S2, S3, S4, C, D1, …)
SPRINT=D3 scripts/bench_runner.sh
SPRINT=D1,D4 scripts/bench_runner.sh

# Tier — micro débloque les compute_sprint{2,3,4}
TIER=micro scripts/bench_runner.sh        # = juste les 3 micro
TIER=all scripts/bench_runner.sh          # = default + micro

# Exclure
EXCLUDE=memory_e2e SPRINT=C scripts/bench_runner.sh

# Mode
MODE=smoke scripts/bench_runner.sh        # build only, no run
MODE=full scripts/bench_runner.sh         # full criterion (slow)

# BLAS toggle (forwarded aux benches blas-aware)
BLAS_BACKEND=accelerate scripts/bench_runner.sh

# Per-bench timeout + retry
BENCH_TIMEOUT=900 BENCH_RETRY=0 scripts/bench_runner.sh
```

### 7.2 Pour reproduire les chiffres M3 Pro de référence

```bash
# 1. Vérifier que le hardware/OS matche §1 (Apple M3 Pro, no-BLAS).
# 2. Clean build pour pristine timings.
cargo clean

# 3. Run le full catalog (default + micro), --quick, no BLAS.
TIER=all MODE=quick scripts/bench_runner.sh

# 4. Comparer à la dernière baseline (v0.1.4 actuel).
python3 scripts/check_regression.py \
    --baseline bench_baselines/v0.1.4/baseline.json \
    --candidate target/bench-report-*.json \
    --threshold 0.10 --warn-threshold 0.05

# 5. Pretty-print SLO compliance.
python3 scripts/show_bench_report.py target/bench-report-*.json
```

Si les chiffres s'écartent à plus de ±10 % :
* Vérifier `RUSTFLAGS=-C target-cpu=native`.
* Vérifier `cargo --version` correspond exactement à v1.85.
* Lire les architectures D1-D5 (§ 8 de `docs/bench/`).

### 7.3 CI workflow (Sprint D5)

`.github/workflows/bench.yml` orchestre **5 jobs parallèles** :

| Job | Benches | Cible wallclock |
|-----|---------|----------------:|
| `bench-lint` | `check_benches_list.sh` | < 30 s |
| `bench-build` | compile efe-core + efe-server (cache shared) | ~5 min |
| `bench-core-mocks` | swarm_e2e + memory_e2e | ~5 min |
| `bench-core-infra` | memory_real_arena | ~3 min |
| `bench-core-compute` | compute_pipeline + compute_sprint{2,3,4} | ~10 min |
| `bench-server` | real_strategies_e2e + real_memory + server_rpc | ~12 min |
| `bench-aggregate` | merge + check_regression + post PR comment | ~2 min |

Total wallclock : **~15 min** (vs 30 min en série pré-D5).

## 7 bis. Accuracy SLO (Sprint D6, v0.1.6)

Sprint D6 a câblé l'infrastructure eval golden — `eval/scripts/run_eval.py`
(40 cas labélés EN+FR) — dans la pipeline bench D5. **L'accuracy est désormais
mesurée en CI à chaque PR** au même titre que la latence.

### 7 bis.1 Le fix `0-round consensus` (couches 1+2)

**Diagnostic** (test pratique 2026-04-25) : confidence constante 0.575
sur 9 inputs distincts, 4/8 cases collapsing vers `billing`. Le détecteur
émettait `Converged` après 2 rounds (le minimum hystérésis) avant que le
centroïde n'ait évolué.

**Fix layer 1** — `ControllerConfig.min_rounds: u32` (default 0 lib, 4 Mode 1)
qui downgrade `Converged → Continue` tant que `round < min_rounds`.

**Fix layer 2** — `Mode1Config::default()` durci :
- `confidence_threshold` : 0.50 → **0.65**
- `hysteresis_rounds` : 2 → **3**
- `variance_threshold` : 0.05 → **0.02**
- `entropy_fraction_max` : 0.98 → **0.95**
- `centroid_drift_max` : 0.20 → **0.10**
- `outlier_ratio_max` : 0.30 → **0.20**

### 7 bis.2 Découverte D6 : H3 confirmé, escalade D7 nécessaire

Post-fix, le test pratique montre :
- ✅ Confidence varie 0.695-0.713 (vs 0.575 constant) — **fusion s'exécute vraiment**
- ⚠️ Accuracy 2/8 = 25 % (vs 4/8 pré-D6) — **REGRESSION**

Plus la fusion tourne, plus elle amplifie un biais structurel vers `billing`.
Hypothèse H3 (architecture D6 §3) confirmée : le `MetaAnalyzer` /
`DefaultSpawner` / `β-mix` ne préserve pas la signature de l'input dans le
centroïde post-fusion. **D7 est le sprint qui adresse ça**.

L'embedder seul (cosine nearest sans swarm) atteint 8/8 = 100 % — l'écart
mesure ce que D7 doit récupérer.

### 7 bis.3 Cibles SLO `D6-*`

| ID | Bench key | Cible | Comparator |
|----|-----------|------:|:----------:|
| `SLO-D6-ACC-1` | `eval_accuracy::top1` | ≥ 0.75 | gte |
| `SLO-D6-ACC-2` | `eval_accuracy::macro_f1` | ≥ 0.70 | gte |
| `SLO-D6-ACC-3` | `eval_accuracy::mean_conf_correct` | ≥ 0.65 | gte |
| `SLO-D6-ACC-4` | `eval_accuracy::false_reject_rate` | ≤ 0.25 | lte |
| `SLO-D6-LAT-1` | `eval_latency::max_ns` | ≤ 1.0 s | lte |

**Premier état mesuré (post-D6, pré-D7)** : `SLO-D6-ACC-1` FAIL
(0.25 < 0.75). C'est le **gate qui empêche D7 de régresser** — chaque
amélioration de la pipeline de fusion sera mesurée contre cet objectif.

### 7 bis.4 Activation locale + CI

```bash
# Local
RUN_ACCURACY=1 bash scripts/bench_runner.sh

# CI : le job `bench-accuracy` est en `continue-on-error: true`
# tant que D7 n'a pas livré. Switch à fail-closed une fois
# accuracy ≥ 0.75 stable.
```

L'audit complet vit dans
[`docs/audits/d6_accuracy_validation.md`](docs/audits/d6_accuracy_validation.md).

## 9. Genesis/Fusion bias remediation (Sprint D7, v0.1.7)

> **Mission D7** — passer `SLO-D6-ACC-1` du rouge (0.25 mesuré, 0.75
> cible post-D6) au vert. Le sprint corrige l'hypothèse H3 : un biais
> structurel dans le couple genesis + fusion qui faisait s'évaporer
> l'input embedding au fil des rounds.

### 9.1 Diagnostic numérique

Le sprint D6 a démontré mécaniquement que la fusion s'exécute
maintenant 7 rounds en moyenne (vs 0 visible pré-D6) — la pipeline
**fonctionne**. Mais l'accuracy a chuté à 0.25 (40 cas → 10 corrects)
parce que les schedules `FusionConfig::default()` (β₀=0.20, λ_β=0.15,
α₀=0.50) sont calibrés pour une doctrine d'« émancipation » : β décroît
exponentiellement vite (au round 7, β=0.07 — l'input pèse 7 %), et α
ne descend qu'après le round 8. Les latents convergent donc vers le
centre du convex hull des anchors, indépendamment de l'input.

Profil round-par-round, pré- vs post-D7 :

| Round |  β (D7)   |  β (pré-D7) |  α (D7)   |  α (pré-D7) |
|------:|----------:|------------:|----------:|------------:|
| 0     |   0.450   |    0.200    |   0.300   |    0.500    |
| 4     |   0.368   |    0.110    |   0.225   |    0.375    |
| 7     | **0.317** |  **0.070**  | **0.169** |  **0.281**  |
| 8     |   0.301   |    0.060    |   0.150   |    0.250    |
| 16    |   0.202   |    0.018    |   0.050   |    0.050    |

Au round 7 : input × 4.5 dans la β-mix, cone × 0.6 dans la projection.

### 9.2 Le fix : Mode-1-specific β/α tuning

`crates/efe-server/src/mode1.rs::build_mode1_fusion_config()` :

```rust
let mut cfg = FusionConfig::default();
cfg.problem_anchor_init    = 0.45;  // β₀  was 0.20
cfg.problem_anchor_decay   = 0.05;  // λ_β was 0.15
cfg.alignment_strength_init = 0.30; // α₀  was 0.50
// alignment_strength_floor = 0.05  unchanged
```

Le helper est consommé par `Mode1Config::default()` UNIQUEMENT — les
defaults libraries restent inchangés (les benches/tests legacy
reposent sur la doctrine d'émancipation). 1 690+ tests workspace
restent verts.

### 9.3 Calibration empirique

`eval/scripts/calibrate_mode1.py` automatise un sweep sur la grille :

* **Sub-grid (27 points)** — défaut, centré sur le point analytique.
  ~1 h sur M3 Pro.
* **Full grid (80 points)** — `--full-grid`, ~3 h.

Le script :
1. Compile (ou réutilise) le binaire `efe-server --release`.
2. Spawn N instances séquentiellement avec les `EFE_FUSION_*` env vars.
3. Pour chaque, lance `run_eval.py --multi-catalog` sur 3 pairs.
4. Émet `target/calibration-d7/{winner.json, full.json, grid.csv}`.

Ranking : `aggregate_macro_f1` desc, tie-break par
`min_per_catalog_macro_f1` puis `aggregate_accuracy_top1`.

### 9.4 Multi-catalog validation (anti-overfit)

`run_eval.py --multi-catalog` accepte `--catalog`/`--golden`
multiples (zippés 1:1) et reporte :

* per-catalog accuracy / macro_f1 / latence,
* aggregate (mean across catalogs),
* min per-catalog (worst-case).

3 nouveaux golden datasets :
* `eval/golden/support_triage.jsonl` — 40 cas (existant pré-D7).
* `eval/golden/fraud_alert.jsonl` — **30 cas** (nouveau D7).
* `eval/golden/billing_escalation.jsonl` — **30 cas** (nouveau D7).

4 nouveaux SLO `D7-MC-*` dans
`scripts/parse_criterion_report.py::SLO_DEFINITIONS` :

| ID | Bench key | Comparator | Target | Cible D7 |
|----|-----------|:----------:|-------:|---------:|
| `SLO-D7-MC-1` | `eval_accuracy::min_top1` | gte | 0.65 | min(catalogs) ≥ 0.65 |
| `SLO-D7-MC-2` | `eval_accuracy::min_macro_f1` | gte | 0.60 | min(catalogs) ≥ 0.60 |
| `SLO-D7-MC-3` | `eval_accuracy::aggregate_top1` | gte | 0.75 | mean ≥ 0.75 |
| `SLO-D7-MC-4` | `eval_accuracy::aggregate_macro_f1` | gte | 0.75 | mean ≥ 0.75 |

Les 4 clés `eval_accuracy::min_top1 / min_macro_f1 / aggregate_top1 /
aggregate_macro_f1` sont **disjointes** des clés du single-catalog
(`top1`, `macro_f1`, `mean_conf_correct`, `false_reject_rate`). Les
deux familles SLO (D6-ACC-* et D7-MC-*) coexistent donc sans collision
de clé ni dépendance d'ordre dans le merge du parser.

### 9.5 Diagnostic centroid trace

Variable `EFE_LOG_CENTROID_TRACE=1` active une émission `tracing::debug!`
post-round dans `FusionAdapter::publish_centroid` :

```bash
EFE_LOG_CENTROID_TRACE=1 \
RUST_LOG=efe_server::mode1::trace=debug \
cargo run -p efe-server --release
```

Chaque round logue `cos_problem` (centroïde courant ↔ problem_embedding),
`cos_anchors` (centroïde courant ↔ chaque anchor), `argmax_anchor`.
Coût quand l'env var est absent : un atomic load + une comparaison.

### 9.6 Plan B armé : `hybrid_decoder.py`

`clients/python/efe_tool/integrations/hybrid_decoder.py` implémente
un classificateur hybride (raw cosine + swarm) prêt à activer si la
calibration D7 plafonne sous 0.65. Politique :
* **Agree** → max(raw_score, swarm_conf).
* **Raw decisive + swarm uncertain** → raw label.
* **Swarm très haut + raw middling** → swarm label.
* **Sinon** → reject (`uncertain`).

10 tests pinned dans `scripts/tests/test_d7_hybrid_decoder.py`.
**Non activé en D7** — Plan E (β/α tuning) reste la voie principale.
Activation prévue D8/D9 si nécessaire.

### 9.7 Anti-régression

* `crates/efe-server/src/mode1.rs::tests` :
  - `d7_fusion_config_pins_tuned_schedules`
  - `d7_default_uses_tuned_fusion`
  - `d7_for_dim_preserves_tuned_fusion`
  - `d7_input_weight_at_round_seven_stays_dominant`
  - `d7_env_overrides_apply_when_set`
  - `d7_env_overrides_fall_back_to_defaults_when_unset`
  - `d7_env_overrides_ignore_malformed_values`
  - `d7_env_overrides_clamp_floor_to_init_when_inconsistent`
  - `d7_env_overrides_treat_empty_string_as_unset`
  - `d7_cosine_sim_unit_vectors`
  - `d7_cosine_sim_handles_zero_norm`
  - `d7_last_spec_trace_publish_peek_reset`
* `scripts/tests/test_d7_hybrid_decoder.py` (10 tests).
* `scripts/tests/test_d7_calibrate_grid.py` (6 tests).

### 9.8 CI fail-closed switch

Le job `bench-accuracy` reste `continue-on-error: true` pendant la
calibration et les **3 premiers main runs PASS post-merge D7**. Une
PR séparée flippe ensuite à `continue-on-error: false` (cf.
[architecture D7 §14](docs/bench/d7_genesis_fusion_bias_remediation_architecture.md#14-intégration-ci--switch-fail-closed)
et [docs/audits/d7_calibration_report.md](docs/audits/d7_calibration_report.md)
§5).

### 9.9 Activation locale

```bash
# Calibration sweep complet (~1 h sur M3 Pro)
python3 eval/scripts/calibrate_mode1.py \
    --catalog catalogs/support_triage.yaml --golden eval/golden/support_triage.jsonl \
    --catalog catalogs/support_triage.yaml --golden eval/golden/fraud_alert.jsonl \
    --catalog catalogs/support_triage.yaml --golden eval/golden/billing_escalation.jsonl \
    --output-dir target/calibration-d7

# Multi-catalog eval rapide (post-calibration freeze)
EFE_BACKEND=mvp cargo run --release -p efe-server &
SERVER_PID=$!
sleep 5
python3 eval/scripts/run_eval.py --multi-catalog \
    --catalog catalogs/support_triage.yaml --golden eval/golden/support_triage.jsonl \
    --catalog catalogs/support_triage.yaml --golden eval/golden/fraud_alert.jsonl \
    --catalog catalogs/support_triage.yaml --golden eval/golden/billing_escalation.jsonl
kill $SERVER_PID
```

L'audit complet :
[`docs/audits/d7_calibration_report.md`](docs/audits/d7_calibration_report.md).
L'architecture détaillée :
[`docs/bench/d7_genesis_fusion_bias_remediation_architecture.md`](docs/bench/d7_genesis_fusion_bias_remediation_architecture.md).

## 10. Plan B activation + Real-Data validation (Sprint D8, v0.1.8)

> **Mission D8** — productize Mode 1 : activer formellement le Plan B
> (`HybridClassifier`, armé en D7.6), industrialiser l'acquisition de
> données réelles (anonymization + PII checks CI), et livrer une
> alternative opt-in (`AnchorAwareTemplateRegistry`) pour les cas où
> le résidu H3c-γ exigerait une refonte du genesis.

### 10.1 Mesure live D8.1 — comparaison 3-decoder

Sur les 3 catalogues synthétiques actuels (100 cas total), exécution
du `eval/scripts/compare_decoders.py` le 2026-04-26 contre le binaire
`target/release/efe-server` post-D7-freeze :

| Catalog | Decoder | Acc | macro-F1 | Δ vs swarm-only |
|---------|---------|----:|---------:|----------------:|
| support_triage (n=40) | raw_cosine | 0.775 | 0.769 | — |
| support_triage | swarm_only | 0.875 | 0.876 | (baseline) |
| support_triage | **hybrid** | **0.875** | **0.908** | **+0.032 f1** |
| fraud_alert (n=30) | raw_cosine | 0.833 | 0.731 | — |
| fraud_alert | swarm_only | 0.900 | 0.883 | (baseline) |
| fraud_alert | **hybrid** | **0.933** | **0.925** | **+0.043 f1** |
| billing_escalation (n=30) | raw_cosine | 0.633 | 0.445 | — |
| billing_escalation | swarm_only | 0.867 | 0.655 | (baseline) |
| billing_escalation | **hybrid** | **0.900** | **0.733** | **+0.078 f1** |

**Conclusion** : le Plan B améliore les 3 catalogues. Le gain le plus
fort est sur `billing_escalation` qui plafonnait à 0.655 macro_f1
post-D7 — la branch policy du `HybridClassifier` absorbe une partie
du résidu H3c-α (ambiguïté embedder question↔billing). Voir
`target/decoder_comparison/report.md` pour le delta per-case.

**Distribution des branches du hybrid** (sur les 100 cas) :
- `agree` : 93/100 (93 %)
- `raw_overrides_swarm` : 4/100 (4 %)
- `swarm_overrides_raw` : 0/100 (0 %)
- `reject` (uncertain) : 3/100 (3 %)

### 10.2 SLOs ajoutés en D8

7 nouveaux SLOs dans `scripts/parse_criterion_report.py::SLO_DEFINITIONS` :

| ID | Bench key | Comparator | Target | Status |
|----|-----------|:----------:|-------:|:------:|
| `SLO-D8-HYB-1` | `eval_accuracy::hybrid_top1` | gte | 0.80 | ✅ 0.875 |
| `SLO-D8-HYB-2` | `eval_accuracy::hybrid_macro_f1` | gte | 0.75 | ✅ 0.908 |
| `SLO-D8-HYB-3` | `eval_accuracy::hybrid_reject_rate` | lte | 0.15 | ✅ 0.075 |
| `SLO-D8-HYB-4` | `eval_accuracy::hybrid_min_macro_f1` | gte | 0.60 | ✅ 0.733 |
| `SLO-D8-RD-1` | `eval_accuracy::real_data_top1` | gte | 0.80 | ⏸️ pending D8.2 |
| `SLO-D8-RD-2` | `eval_accuracy::real_data_macro_f1` | gte | 0.75 | ⏸️ pending D8.2 |
| `SLO-D8-LAT-2` | `eval_latency::real_data_p99_ns` | lte | 5 s | ⏸️ pending D8.2 |

**Total SLOs gated** : **16** (5 D6 + 4 D7-MC + 7 D8). Les 4 clés de
keys `eval_accuracy::hybrid_*` sont **DISJOINTES** des keys swarm-only
(`top1`, `min_top1`, `aggregate_top1`) — invariant pinned par
`scripts/tests/test_d8_hybrid_eval.py` (4 tests) +
`scripts/tests/test_d8_parser_slo_definitions.py` (5 tests, dont une
permutation 6× qui prouve l'order-indépendance).

### 10.3 Outillage D8

| Script | Lignes | Rôle |
|--------|-------:|------|
| `eval/scripts/run_eval_hybrid.py` | ~280 | Eval via `HybridClassifier` (single + multi-catalog) |
| `eval/scripts/compare_decoders.py` | ~280 | Rapport 3-way (raw / swarm / hybrid) — JSON + Markdown |
| `eval/scripts/catalog_hygiene.py` | ~310 | Score qualité catalog (5 checks, 0-10) — utilisé pour onboarding partenaire |
| `eval/scripts/anonymize_golden.py` | ~280 | PII redaction (email, phone, CB Luhn-checked, SSN, IBAN, URL, postal) |
| `eval/scripts/check_no_pii.py` | ~110 | CI gate (PR-level scan des `eval/golden/*.jsonl`) |
| `examples/hybrid_classification.py` | ~85 | Exemple Python end-to-end pour intégrateur |

### 10.4 Plan B (HybridClassifier) — activation

```python
from efe_tool import AnchorCatalog, EfeClient, LocalSTEmbedder
from efe_tool.integrations.hybrid_decoder import HybridClassifier

embedder = LocalSTEmbedder()
catalog = AnchorCatalog.load("catalogs/support_triage.yaml", embedder=embedder)
client = EfeClient.from_env()
hybrid = HybridClassifier(client=client, catalog=catalog)
result = hybrid.classify("Where can I find my past invoices?")
# result.label, result.confidence, result.path (agree / raw_overrides /
# swarm_overrides / reject), result.raw_score, result.swarm_confidence
```

Le Plan B est **activable côté client**, sans modification du serveur.
Les serveurs déployés post-D7 le supportent automatiquement.

### 10.5 Plan B+ (AnchorAwareTemplateRegistry) — opt-in côté serveur

```rust
// Server-side opt-in (alternatively to Plan B Plan B):
let mut cfg = Mode1Config::for_dim(384).with_anchor_aware_templates();
let backend = build_mvp_backend(cfg)?;
```

Quand activé, le `NaSem` template prior devient une combinaison
linéaire :

```
prior_NaSem[i] = (1 - blend) * base_prior[i]
              + blend * anchor_centroid[i]
              + N(0, noise_sigma)
```

avec `blend = 0.6`, `noise_sigma = 0.05` par défaut. Pinned par
**10 tests** dans `efe-core::genesis::registry::tests::d85_*`. Default
(`Stub`) inchangé pour backward-compat — les 1 711 tests workspace
restent verts sans modification.

### 10.6 PII pipeline + CI gate

Workflow `.github/workflows/pii_check.yml` (nouveau D8.4) scanne tous
les `eval/golden/*.jsonl` à chaque PR qui les modifie. Détection :
- Email (RFC 5322 simplifié)
- Phone E.164 + FR + US
- Credit card (Luhn-checked, anti-faux-positif)
- SSN US, INSEE FR
- IBAN
- URL

Couverture pinned par 15 tests (10 dans `test_d8_anonymize_golden.py`,
5 dans `test_d8_check_no_pii.py`).

### 10.7 Anti-régression D8

| Famille | Tests | Fichier |
|---------|------:|---------|
| Anonymize PII pipeline | 10 | `scripts/tests/test_d8_anonymize_golden.py` |
| PII CI gate | 5 | `scripts/tests/test_d8_check_no_pii.py` |
| Catalog hygiene | 7 | `scripts/tests/test_d8_catalog_hygiene.py` |
| Hybrid eval scripts | 4 | `scripts/tests/test_d8_hybrid_eval.py` |
| Parser SLO definitions | 5 | `scripts/tests/test_d8_parser_slo_definitions.py` |
| AnchorAware registry (Rust) | 10 | `efe-core::genesis::registry::tests::d85_*` |
| Mode1Config opt-in (Rust) | 5 | `efe-server::mode1::tests::d85_*` |
| **Total D8** | **46** | |

Cumul tous sprints :
- 5 D6 + 5 D7 (mode1) + 5 D7 (calibration_realism)
- 6 D6 (audit fixes) + 10 D7 (hybrid_decoder) + 6 D7 (calibrate_grid)
  + 4 D7 (parser_eval_merge)
- + 46 D8 = **77 tests Python ou Rust spécifiques D6+D7+D8**, sur
  un total workspace de **~1 720 tests Rust** (post-D7) + **77 tests
  Python**.

### 10.8 Activation locale

```bash
# Build server release (~2 min)
cargo build --release -p efe-server

# Start server
EFE_LISTEN_ADDR=127.0.0.1:50051 EFE_AUTH_ENABLED=false \
    ./target/release/efe-server &

# Run hybrid eval on 3 catalogs
python3 eval/scripts/run_eval_hybrid.py --multi-catalog \
    --catalog catalogs/support_triage.yaml --golden eval/golden/support_triage.jsonl \
    --catalog catalogs/support_triage.yaml --golden eval/golden/fraud_alert.jsonl \
    --catalog catalogs/support_triage.yaml --golden eval/golden/billing_escalation.jsonl \
    --output target/eval-hybrid.json

# 3-way decoder comparison report
python3 eval/scripts/compare_decoders.py \
    --catalog catalogs/support_triage.yaml --golden eval/golden/support_triage.jsonl \
    --output-dir target/decoder_comparison

# Catalog hygiene score (transverse)
python3 eval/scripts/catalog_hygiene.py \
    --catalog catalogs/support_triage.yaml \
    --golden eval/golden/support_triage.jsonl \
    --output target/hygiene_support_triage.md
```

L'audit complet :
[`docs/audits/d8_decoder_comparison.md`](docs/audits/d8_decoder_comparison.md)
(à compléter avec le rapport partenaire post-D8.2).

L'architecture détaillée :
[`docs/bench/d8_hybrid_realdata_architecture.md`](docs/bench/d8_hybrid_realdata_architecture.md).
La calibration automatique des 4 seuils du `HybridClassifier` est
spécifiée dans
[`docs/bench/d8bis_hybrid_threshold_calibration_architecture.md`](docs/bench/d8bis_hybrid_threshold_calibration_architecture.md)
(sous DT-D8.2 — implémentation différée P3, prête à coder).

## 11. Docker image + Helm chart + load/soak (Sprint D9, v0.1.9)

### 11.1 Mission

D9 transforme `efe-server` de "binaire qui tourne sur le M3 Pro de
l'auteur" en **service déployable sur n'importe quel cluster k8s
≥ 1.27**. Trois axes :

1. **Docker image** distroless cc-debian12 nonroot UID 10001, multi-stage
   builder (cargo-chef cache + reproductibilité partielle :
   `--remap-path-prefix /app=.`, `SOURCE_DATE_EPOCH` pinned).
2. **Helm chart** sous `deploy/helm/efe/` avec 13 templates
   (deployment, service, configmap, secret, sa, hpa, pdb,
   networkpolicy, servicemonitor, prometheusrule, NOTES, pvc,
   `tests/test-grpc-health.yaml`, `tests/test-semantic-smoke.yaml`)
   et 9 amendements appliqués (cf. supplement Annexe A).
3. **Load + soak** harness sous `bench/load/` (4 scénarios ghz)
   et `bench/soak/` (24 h + détecteur de fuite OLS).

### 11.2 SLOs ajoutés en D9 (5 nouveaux)

| ID | Bench-format key | Comparator | Cible | Notes |
|----|------------------|:----------:|------:|-------|
| `SLO-D9-LOAD-1` | `load_sustained::p99_ns` | `lte` | 5 s | §11.3.2 ; math sample sizing supplement §1.2 |
| `SLO-D9-LOAD-2` | `load_sustained::error_ratio` | `lte` | 1 % | binomial CI sur n≈3 000 |
| `SLO-D9-LOAD-3` | `load_stress::throughput_rps` | `gte` | 5 RPS | Little's law, supplement §3 |
| `SLO-D9-LOAD-4` | `load_spike::recovery_seconds` | `lte` | 30 s | drain time = Q·S/K avec K=2 |
| `SLO-D9-SOAK-1` | `soak::rss_slope_bps` | `lte` | 870 B/s | OLS, supplement §2 |

Toutes les clés bench-format `load_*::*` et `soak::*` sont
**disjointes** des namespaces antérieurs (`eval_accuracy::`,
`eval_latency::`) — invariant pinné par
`bench/load/tests/test_parse_ghz_report.py::test_load_keys_are_disjoint_from_accuracy_namespace`
et son équivalent côté soak.

Les 5 SLOs commencent en **SKIPPED** dans
`bench_baselines/v0.1.9/baseline.json` ; ils basculent en PASS / FAIL
mesurés dès que le workflow `bench-load` produit son premier
artefact post-merge sur `main`. Le job est `continue-on-error: true`
pendant les 3 premiers runs main puis flippé fail-closed via une
PR séparée.

### 11.3 Outillage D9

#### 11.3.1 ghz harness — 4 scénarios

```
bench/load/
├── ghz_smoke.json          (1 client × 30 s)
├── ghz_sustained.json      (4 clients × 5 min, ~10 RPS)
├── ghz_stress.json         (16 clients × 2 min, target ~30 RPS)
├── ghz_spike.json          (1→16 step jump × 90 s)
├── fixtures/
│   └── sample_request.json  (5 ExecuteRequest payloads)
├── run_load_test.sh         (orchestrateur smoke|sustained|stress|spike|all)
├── parse_ghz_report.py      (ghz JSON → flat eval-bench-format)
└── tests/test_parse_ghz_report.py   (26 tests)
```

L'amendement **A2** est critique : pour les scénarios
`sustained` / `stress`, l'opérateur DOIT lancer le serveur avec
`EFE_IDEMPOTENCY_TTL_MS=0` pour désactiver le cache LRU côté
serveur — sinon le 2ᵉ tour de fixtures hit le cache et la latence
mesurée n'est plus représentative du compute Mode-1.

#### 11.3.2 Soak — 24 h + détecteur OLS

```
bench/soak/
├── leak_detector.py        (OLS regression + decision logic)
├── run_soak_test.sh         (24 h orchestrateur, sample RSS / minute)
├── k8s_soak_job.yaml        (Job in-cluster avec ServiceAccount dédié)
└── tests/test_leak_detector.py    (20 tests)
```

Décision : `FAIL` si **et seulement si** :

* `slope ≥ 870 B/s`     (effet supérieur au seuil opérationnel), ET
* `|slope / SE| ≥ 1.96` (significativité ≥ 95 % unilatéral).

Si le slope est au-dessus du seuil mais sous la barre de
significativité, le test retourne `INCONCLUSIVE` (exit 2) — un
signal mou qui pousse à étendre la durée du soak ou à investiguer
le bruit avant de déclarer une fuite. Cf. supplement §2.3 pour la
puissance statistique (~99 % de détection à n=1 440 / σ=5 MB).

L'amendement **A3** (warmup t<1800s + median sliding window=10)
est implémenté par défaut — il neutralise le bruit glibc malloc-trim.

#### 11.3.3 Distroless symbol checker (amendement A4)

`scripts/check_distroless_symbols.py` confronte la liste des
libraries dynamiques importées par `efe-server` (via `ldd`) à la
liste des libraries présentes dans la base distroless (via
`ldconfig -p`). Wired dans `release.yml` ; bloque le release tag
si une dépendance Cargo introduit silencieusement un `dlopen`
contre une lib que distroless ne livre pas.

### 11.4 Helm chart amendments (A5, A6, A9)

Sans changer le design existant :

* **A5 — `service.headless`** (default `false`) : à activer dès
  `replicaCount > 1`. Sans cela, gRPC HTTP/2 multiplexe sur une
  seule connection et toutes les requêtes vont au même replica.
* **A6 — `helm test efe`** : deux pods test, `test-grpc-health`
  (always-on, `grpc_health_probe`) et `test-semantic-smoke` (opt-in,
  3 cas labeled). Le 2ᵉ ferme le HAZOP gap #4 ("service répond
  200 OK avec un mauvais label").
* **A9 — `tenant.id`** label : propage `app.kubernetes.io/tenant`
  sur tous les manifestes générés. Préparation D11 multi-tenancy
  sans refactor du chart.

### 11.5 Reproductibilité (amendement A8)

`.github/workflows/reproducibility.yml` build l'image deux fois
chaque nuit et compare les digests. Mismatch → issue auto-créée
avec label `supply-chain`. Le `Dockerfile.prod` reçoit
`RUSTFLAGS="--remap-path-prefix /app=."` pour effacer le chemin
absolu du builder ; `codegen-units=1` est documenté mais pas
forcé (coût ~30 % de build, gain marginal vs détection nightly).

### 11.6 Documentation D9 — sous `docs/operations/`

| Fichier | Audience | Contenu |
|---------|----------|---------|
| `quickstart.md` | Pilot engineer (10 min) | install Helm + smoke test + 3 cas semantic |
| `deployment.md` | SRE prod | image pinning par digest, JWT externe, observabilité, capacity, upgrade, rollback |
| `troubleshooting.md` | On-call | catalogue 6 sections × 3-5 cas chacune, par symptôme → diagnostic → fix |
| `load_test.md` | Bench engineer | reproduire local + cluster ; pourquoi `--no-cache` |
| `key_rotation.md` (A7) | SRE prod | 6 étapes zero-downtime, multi-key validation |
| `chaos_runbook.md` | Game day | 8 scénarios HAZOP avec `kubectl` recipes |

### 11.7 Anti-régression D9

| Test suite | Count | Couverture |
|-----------|------:|------------|
| `bench/load/tests/test_parse_ghz_report.py` | 26 | parser ghz, scenario labels, percentiles, throughput math, error ratio, spike recovery, SLO comparator, e2e CLI |
| `bench/soak/tests/test_leak_detector.py` | 20 | OLS math, median window, warmup, decision logic, e2e CSV, bench-format invariants, disjoint-key |
| `scripts/tests/test_d9_parser_load_slos.py` | 11 | SLO_DEFINITIONS structure, disjointness, e2e parser merge load + soak, SKIP semantics |
| `scripts/tests/test_d9_distroless_symbols.py` | 10 | ldconfig parsing, ldd parsing, vdso resolution, version-stem fallback, allow-list, exit codes |
| **Total** | **67** | |

Tous les 16 SLOs antérieurs (D6/D7/D8) restent gatés — aucun n'est
régressé par D9.

### 11.8 Activation locale

```bash
# Build + smoke test (sans cluster)
cargo build --release --bin efe-server
./target/release/efe-server &      # défaut : auth off, port 50051

# Load test 4 scénarios — local
EFE_IDEMPOTENCY_TTL_MS=0 ./target/release/efe-server &
bash bench/load/run_load_test.sh all \
    --endpoint 127.0.0.1:50051 \
    --output-dir target/load
cat target/load/load_test_summary.md
```

L'audit complet vivant : [`docs/audits/d9_load_soak_report.md`](docs/audits/d9_load_soak_report.md)
(skeleton à compléter post-mesure live).

L'architecture détaillée : [`docs/bench/d9_docker_helm_load_architecture.md`](docs/bench/d9_docker_helm_load_architecture.md)
(spec, 1 992 lignes) et le compagnon analytique
[`docs/bench/d9_deep_analysis_supplement.md`](docs/bench/d9_deep_analysis_supplement.md)
(math, comparaisons, HAZOP, 1 119 lignes).

## 12. SDK docs + quickstart + 4 examples (Sprint D10, v0.1.10)

### 12.1 Mission

D10 transforme EFE de "service installable via Helm" (D9) en
"service utilisable par un développeur partenaire en 10 minutes
sans aide humaine" — verrou critique avant le **Pilot 1.0**
milestone (premier client externe payant). Six axes :

1. **Surface SDK figée** post-D10 (versioning semver, `__all__`
   public, `py.typed` marker, hiérarchie d'erreurs `Efe*Error`
   mappée 1:1 sur les status codes gRPC, async client skeleton
   livré comme thread-pool en v0.1.x → native grpc.aio en v0.2.x).
2. **Documentation Diátaxis** (4 catégories orthogonales :
   Tutorial / How-to / Reference / Explanation) sur
   mkdocs-material + auto-générée via `mkdocstrings` + versionnée
   via `mike` (URLs `docs.efe.ai/0.1.10/`).
3. **4 exemples copy-paste-ready** sous `examples/` couvrant
   beginner → custom catalog → multilang concurrent →
   production-grade.
4. **Quickstart end-to-end testée** avec `scripts/validate_quickstart.sh`
   exécuté par 3 reviewers externes ; SLO `D10-DX-1` ≤ 10 min
   wallclock.
5. **Publication PyPI via OIDC trusted publisher** (zéro token
   statique) + déploiement docs versionnées sur GitHub Pages.
6. **5 SLOs `D10-*`** (3 docs + 2 DX) gated en CI.

### 12.2 SLOs ajoutés en D10 (5 nouveaux)

| ID | Bench-format key | Comparator | Cible | Notes |
|----|------------------|:----------:|------:|-------|
| `SLO-D10-DOC-1` | `docs::build_strict_warnings` | `lte` | 0 | `mkdocs build --strict` exit 0 |
| `SLO-D10-DOC-2` | `docs::api_coverage_ratio` | `gte` | 0.95 | `test_d10_api_reference_coverage.py` |
| `SLO-D10-DOC-3` | `docs::link_check_404` | `lte` | 0 | `linkchecker` step |
| `SLO-D10-DX-1` | `dx::quickstart_wallclock_s` | `lte` | 600 s | `validate_quickstart.sh` (median 3 reviewers) |
| `SLO-D10-DX-2` | `dx::examples_smoke_aggregate_s` | `lte` | 30 s | `pytest examples/tests/` |

Toutes les clés bench-format `docs::*` et `dx::*` sont **disjointes**
des namespaces antérieurs (`eval_accuracy::`, `eval_latency::`,
`load_*::*`, `soak::*`) — invariant pinné par
`scripts/tests/test_d10_disjoint_keys.py`.

Les 5 SLOs commencent en **SKIPPED** dans
`bench_baselines/v0.1.10/baseline.json` ; ils basculent en PASS / FAIL
mesurés dès le premier run vert post-merge sur `main`.

### 12.3 Outillage D10

#### 12.3.1 SDK surface freeze

```
clients/python/efe_tool/
├── __init__.py              ← public surface (D10 __all__ extended)
├── _version.py              ← single source of truth (NEW)
├── py.typed                 ← PEP 561 marker (NEW)
├── _async/                  ← AsyncEfeClient skeleton (NEW)
│   ├── __init__.py
│   └── client.py
├── client.py                ← unchanged (sync EfeClient)
├── catalog.py               ← unchanged
├── embedder.py              ← unchanged
├── errors.py                ← extended with EfeError hierarchy
└── integrations/hybrid_decoder.py  ← carry-over D8
```

Hiérarchie d'erreurs D10 :
`EfeError` (base) → `EfeAuthError`, `EfeRateLimitError`,
`EfeInvalidArgumentError`, `EfeNotFoundError`, `EfeUnavailableError`,
`EfeInternalError`. Helper `raise_from_grpc(rpc_error)` pour traduire
un `grpc.RpcError` en sous-classe typée. La hiérarchie legacy
(`BridgeError`, `NetworkError`, etc.) est conservée — `EfeError`
hérite de `BridgeError` donc `except BridgeError` continue de
catch toutes les erreurs.

#### 12.3.2 Documentation Diátaxis

```
docs/docs/
├── index.md                 (landing page repensée)
├── tutorial/  (3 pages)     (apprenez par la pratique)
│   ├── quickstart.md
│   ├── concepts.md
│   └── your-first-classifier.md
├── howto/     (7 pages)     (résolvez un problème ciblé)
│   ├── install.md, auth.md, custom-catalog.md,
│   ├── observability.md, streaming.md, retries.md, language-detection.md
├── reference/ (5 pages)     (description exhaustive)
│   ├── api.md ← AUTO via mkdocstrings
│   ├── cli.md, errors.md, catalog-schema.md, env-vars.md
└── explanation/ (4 pages)   (comprendre le pourquoi)
    ├── architecture.md, why-anchors.md,
    ├── hybrid.md, tradeoffs.md
```

Total : 19 pages neuves + extension `mkdocs.yml` avec
`mkdocstrings` + `mike` + `pymdownx.snippets` (zero-drift
code↔doc).

#### 12.3.3 4 exemples (`examples/0X_*.py`)

```
examples/01_quickstart_classify.py   (~50 LOC)  — bundled catalog, 5 cases
examples/02_custom_catalog.py        (~125 LOC) — author intents → YAML → load
examples/03_multilang_streaming.py   (~110 LOC) — AsyncEfeClient + Semaphore
examples/04_production_client.py     (~225 LOC) — JWT + retries + OTel + JSON logs
```

Plus :
* `examples/README.md` — index + cheat-sheet
* `examples/tests/test_examples_smoke.py` — 20 tests
* `tests/stub_server.py` — gRPC Mode-1 stub backend pour CI

#### 12.3.4 DX quickstart validation

`scripts/validate_quickstart.sh` — automatise les 6 étapes
(preflight → helm install → kubectl wait → pip install →
port-forward → run example 01 → assert labels). Mesure
wallclock + emet `target/dx/quickstart_result.json`. Cible : 3
reviewers externes mesurés median ≤ 600 s.

### 12.4 Publication PyPI + docs.efe.ai

* **PyPI** : OIDC trusted publisher déclaré sur pypi.org
  (project: efe-tool, owner: efe-ai, workflow: release.yml,
  environment: pypi). Zero-token, échangé OIDC GitHub → PyPI à
  chaque tag.
* **docs.efe.ai** : déployé via `mike deploy <semver> latest` sur
  la branche `gh-pages`. URL `docs.efe.ai/0.1.10/` versionnée ;
  `latest` alias pointe sur la version stable la plus récente ;
  `dev` alias pointe sur trunk (push main).
* **CNAME** : `docs.efe.ai → efe-ai.github.io` (HTTPS automatique
  via Let's Encrypt).

### 12.5 Anti-régression D10

| Test suite | Count | Couverture |
|-----------|------:|------------|
| `examples/tests/test_examples_smoke.py` | 20 | static checks (LOC budget, no internal imports, main guard) + dynamic (stub server) |
| `scripts/tests/test_d10_public_api_surface_pinned.py` | TBD | `__all__` pinné, surface freezée |
| `scripts/tests/test_d10_api_reference_coverage.py` | TBD | mkdocstrings autodoc couvre ≥ 95 % de `__all__` |
| `scripts/tests/test_d10_disjoint_keys.py` | TBD | namespaces SLO disjoints |
| **Total D10** | **~30 tests neufs** | |

Tous les 21 SLOs antérieurs (D4×3, D6×5, D7×4, D8×7, D9×5)
restent gated. Aucun changement Rust en D10 → aucun risque
côté serveur.

### 12.6 Activation locale

```bash
# Build + test (sans cluster)
pip install -e clients/python
python -m pytest examples/tests/test_examples_smoke.py \
    -k "not runs_against_stub and not requires_token" -v

# Run example 01 against a real server (helm install + port-forward)
python examples/01_quickstart_classify.py
```

L'audit complet vivant :
[`docs/audits/d10_dx_report.md`](docs/audits/d10_dx_report.md)
(skeleton à compléter post-merge + 3 reviewers).

L'architecture détaillée :
[`docs/bench/d10_sdk_docs_examples_architecture.md`](docs/bench/d10_sdk_docs_examples_architecture.md)
(2 124 lignes, 17 sections, 12 décisions clés, 10 risques, plan
4 j wallclock).

## 13. Per-tenant isolation (Sprint D11, v0.1.11)

### 13.1 Mission

D11 transforme EFE de "un cluster = un partenaire pilote" (D9/D10)
en "un cluster = N partenaires avec isolation forte" — verrou
critique avant le **Phase 2 milestone : SaaS multi-tenant
commercial**. Sept axes (cf. `docs/bench/d11_per_tenant_isolation_architecture.md` §1.2) :

1. **Hardening serveur** — `assert_tenant_match` helper, idempotency
   cache namespacing (déjà en place côté M11), admin endpoints
   filtering, 30+ tests Rust d'attaque cross-tenant (HAZOP 12
   vecteurs).
2. **Helm Tier 2** — 3 nouveaux templates opt-in (ResourceQuota,
   LimitRange, NetworkPolicy ingress per-tenant) + extension
   `servicemonitor.yaml` (relabel pour stamper `tenant`) +
   `prometheusrule.yaml` (4 alertes per-tenant + cardinality
   bomb detection).
3. **Cost attribution** — `scripts/billing/aggregate_usage.py`
   parse les métriques Prometheus (300+ LOC stdlib-only) sur une
   fenêtre glissante et émet CSV / JSON pour la facturation
   Phase 2.
4. **SDK admin** — sub-package `efe_tool.admin` avec
   `EfeAdminClient` (6 méthodes) + nouveau `CatalogRef` dataclass
   + `AdminError` (subclass de `EfeError`).
5. **Observabilité** — Grafana dashboard JSON committed
   (`deploy/observability/grafana/per-tenant-dashboard.json`) +
   runbook `docs/operations/multi-tenant-observability.md`.
6. **Documentation** — 3 nouvelles pages Diataxis :
   `howto/multi-tenant.md`, `explanation/security-model.md`,
   `reference/tenant-claims.md`.
7. **CI** — `.github/workflows/bench-isolation.yml` (3 jobs :
   Rust isolation, Helm dual-render, Python anti-regression),
   7 SLOs `D11-MT-*`, 3 nouveaux `RESERVED_PREFIXES`
   (`isolation::`, `cost::`, `obs::`).

### 13.2 SLOs ajoutés en D11 (7 nouveaux)

| ID | Bench-format key | Comparator | Cible | Notes |
|----|------------------|:----------:|------:|-------|
| `SLO-D11-MT-1` | `isolation::cross_tenant_leakage` | `lte` | 0 | hard-fail sur attempts |
| `SLO-D11-MT-2` | `isolation::rate_limit_disjoint_pct` | `gte` | 99 % | bucket fairness |
| `SLO-D11-MT-3` | `isolation::audit_log_attribution_pct` | `gte` | 100 % | every line carries `tenant_id` |
| `SLO-D11-MT-4` | `isolation::metric_label_coverage_pct` | `gte` | 100 % | every business metric labelled |
| `SLO-D11-MT-5` | `cost::aggregator_drift_pct` | `lte` | 1 % | aggregator vs Prometheus snapshot |
| `SLO-D11-MT-6` | `cost::usage_export_latency_s` | `lte` | 60 s | aggregator runtime 30-d window |
| `SLO-D11-MT-7` | `obs::tenant_cardinality_count` | `lte` | 1 000 | Prometheus distinct tenants |

Tous SKIPPED en `bench_baselines/v0.1.11/baseline.json` jusqu'aux
runs Rust isolation (à implémenter post-merge initial).

### 13.3 Disjoint-key invariant (étendu)

`RESERVED_PREFIXES` count post-D11 = **9 namespaces** :

```python
RESERVED_PREFIXES = (
    "eval_accuracy::",   # D6/D7/D8 — eval/scripts/run_eval*.py
    "eval_latency::",    # D6/D7/D8 — eval/scripts/run_eval*.py
    "load_",             # D9 — bench/load/parse_ghz_report.py
    "soak::",            # D9 — bench/soak/leak_detector.py
    "docs::",            # D10 — .github/workflows/docs.yml
    "dx::",              # D10 — scripts/validate_quickstart.sh
    "isolation::",       # D11 NEW — Rust d11_isolation_*.rs
    "cost::",            # D11 NEW — scripts/billing/aggregate_usage.py
    "obs::",             # D11 NEW — Prometheus tenant cardinality probe
)
```

Pinned by `scripts/tests/test_d11_disjoint_keys.py` (8 tests).

### 13.4 Outillage D11

#### 13.4.1 Helm Tier 2 templates

```
deploy/helm/efe/templates/
├── resourcequota.yaml             (NEW — opt-in tenant.namespace.resourceQuota.enabled)
├── limitrange.yaml                (NEW — opt-in tenant.namespace.limitRange.enabled)
├── networkpolicy-tenant.yaml      (NEW — opt-in tenant.namespace.networkPolicy.tenantIngress.enabled)
├── servicemonitor.yaml            (extended — D11 relabel + cardinality allowlist)
├── prometheusrule.yaml            (extended — 4 per-tenant alerts)
├── tests/test-cross-tenant-denial.yaml (NEW — opt-in helm test)
└── ...

deploy/helm/efe/values.tier2.yaml  (NEW — production preset)
```

#### 13.4.2 Cost aggregator

```
scripts/billing/aggregate_usage.py        (300+ LOC, stdlib-only)
scripts/billing/tests/test_aggregate_usage.py   (20 tests)
docs/operations/billing.md                 (CronJob example + troubleshooting)
```

#### 13.4.3 SDK admin

```
clients/python/efe_tool/admin/__init__.py  (NEW)
clients/python/efe_tool/admin/client.py    (NEW — EfeAdminClient)
clients/python/efe_tool/__init__.py        (extended — export D11 symbols)

docs/docs/howto/multi-tenant.md            (NEW)
docs/docs/explanation/security-model.md    (NEW)
docs/docs/reference/tenant-claims.md       (NEW)
```

#### 13.4.4 CI workflow

```
.github/workflows/bench-isolation.yml      (NEW — 3 jobs)
scripts/tests/test_d11_disjoint_keys.py    (8 tests)
scripts/tests/test_d11_surface_extensions.py (11 tests)
scripts/tests/d11_helm_render_isolation.py (CI script for the Helm dual-render check)
```

### 13.5 Anti-régression D11

| Test suite | Count | Couverture |
|-----------|------:|------------|
| `scripts/billing/tests/` | 20 | aggregator math, schema, CLI |
| `scripts/tests/test_d11_disjoint_keys.py` | 8 | RESERVED_PREFIXES + SLO inventory |
| `scripts/tests/test_d11_surface_extensions.py` | 11 | SDK admin surface + Helm values |
| **Total D11** | **39** | |

Tous les 26 SLOs antérieurs (D4×3, D6×5, D7×4, D8×7, D9×5, D10×5)
restent gated. Aucun changement Rust destructif en D11 (additif
seulement). 

### 13.6 Activation locale

```bash
# Tier 1 (default) — same install as before, just mint a JWT
# with tenant_id in claims.

# Tier 2 — one Helm release per tenant
kubectl create namespace tenant-acme
kubectl label namespace tenant-acme tenant.efe.ai/owner=acme
helm install efe-acme oci://ghcr.io/efe-project/charts/efe \
  --version 0.1.11 \
  --namespace tenant-acme \
  --values deploy/helm/efe/values.tier2.yaml \
  --set tenant.id=acme

# Verify
helm test efe-acme -n tenant-acme

# Cost aggregator (run daily cron)
python scripts/billing/aggregate_usage.py \
  --prom-url http://prometheus:9090 \
  --start 2026-04-01T00:00:00Z \
  --end   2026-05-01T00:00:00Z \
  --output /var/billing/2026-04.csv
```

L'audit complet vivant :
[`docs/audits/d11_isolation_report.md`](docs/audits/d11_isolation_report.md)
(skeleton à compléter dès l'arrivée des Rust isolation tests).

L'architecture détaillée :
[`docs/bench/d11_per_tenant_isolation_architecture.md`](docs/bench/d11_per_tenant_isolation_architecture.md)
(1 845 lignes, 21 sections, 12 décisions clés, 10 risques,
12-vector HAZOP, plan 5 j wallclock).

## 14. Catalog admin endpoints + versioning (Sprint D12, v0.1.12)

### 14.1 Mission

Sprint D12 introduit la surface d'admin **catalogs** :
catalogues d'ancres versionnés, stockés côté serveur,
immuables post-register, évolués via 5 opérations
discriminées (APPEND_CLASS, REMOVE_CLASS, UPDATE_THRESHOLDS,
REPLACE_CLASS_ANCHORS, RECOMPILE).

Cette release :

1. Ajoute le service gRPC `efe.v1.catalog.CatalogService` dans
   un fichier proto séparé (`protos/efe/v1/catalog.proto`) — 6 RPCs
   (Register, Evolve, Get, List, ListVersions, Delete) — proto
   forward-compat (additif, ne casse pas les clients M9-era).
2. Scaffolding Rust `crates/efe-server/src/catalog/` (4 modules :
   `registry.rs` trait, `memory.rs` impl `tokio::sync::RwLock`,
   `policy.rs` quotas+schéma, `audit.rs` `tracing::info!(target:
   "audit")`). Le wire-up gRPC vers `runtime::Server` est
   explicitement déféré à **D12-bis** pour rester additif sur la
   suite de tests existants (1 700+).
3. SDK Python : `EfeAdminClient` gagne 5 méthodes (`register_catalog`,
   `evolve_catalog`, `get_catalog`, `list_catalog_versions`,
   `delete_catalog`) + active la méthode `list_catalogs` qui était
   stubée en D11. 4 nouveaux types publics : `EvolveOp` (enum 5
   variantes), `RegisteredCatalog` (frozen dataclass), `CatalogAuditMetadata`,
   `CompiledClassRef`. Tous re-exportés à `efe_tool` top-level.
4. CLI : `efe-admin catalog {register,evolve,list,versions,get,delete}`
   (6 verbes, sortie JSON schema-versionnée, codes de sortie
   0/2/3/4/5/6).
5. 3 pages Diataxis : `docs/howto/catalog-admin.md`,
   `docs/howto/catalog-evolution.md`, `docs/reference/catalog-rpc.md`.
6. Exemple end-to-end : `examples/05_catalog_admin.py` (lifecycle
   complet : register v1, list, get, evolve→v2, evolve→v3, delete).
7. SLO family `D12-CT-*` (5 latency budgets + 2 correctness ratios)
   avec un nouveau `RESERVED_PREFIX` `catalog::`.

### 14.2 SLOs ajoutés en D12 (7 nouveaux)

| ID             | Bench key                              | Comparator | Cible       | Mesure                                                  |
|----------------|----------------------------------------|------------|-------------|---------------------------------------------------------|
| `SLO-D12-CT-1` | `catalog::register_p99_small`          | `lte`      | 200 ms      | `RegisterCatalog` p99, ≤ 5 classes                       |
| `SLO-D12-CT-2` | `catalog::register_p99_large`          | `lte`      | 1 500 ms    | `RegisterCatalog` p99, 100 classes (stress)              |
| `SLO-D12-CT-3` | `catalog::evolve_append_p99`           | `lte`      | 250 ms      | `EvolveCatalog(APPEND_CLASS)` p99                        |
| `SLO-D12-CT-4` | `catalog::get_p99`                     | `lte`      | 50 ms       | `GetCatalog` p99 (read path)                             |
| `SLO-D12-CT-5` | `catalog::list_p99`                    | `lte`      | 75 ms       | `ListCatalogs` p99 (read path, agrégation 1 tenant)      |
| `SLO-D12-CT-6` | `catalog::serialization_pass_pct`      | `gte`      | 100 %       | concurrents register/evolve : intégrité version monotone |
| `SLO-D12-CT-7` | `catalog::sha256_witness_match_pct`    | `gte`      | 100 %       | round-trip register→get : sha256 inchangé                |

Tous SKIPPED en `bench_baselines/v0.1.12/baseline.json` jusqu'au
1er run main vert post-merge de `.github/workflows/bench-catalog.yml`
+ landing du gRPC handler dans D12-bis.

### 14.3 Disjoint-key invariant (étendu)

`bench/load/parse_ghz_report.py::RESERVED_PREFIXES` passe de **9**
à **10** entrées :

```python
RESERVED_PREFIXES = (
    "eval_accuracy::",   # D6/D7/D8
    "eval_latency::",    # D6/D7/D8
    "load_",             # D9
    "soak::",            # D9
    "docs::",            # D10
    "dx::",              # D10
    "isolation::",       # D11
    "cost::",            # D11
    "obs::",             # D11
    "catalog::",         # D12 NEW
)
```

Pinné par `scripts/tests/test_d12_disjoint_keys.py` (8 tests :
inventaire `RESERVED_PREFIXES`, présence des 7 SLOs D12, format
des champs, namespacing, unicité des IDs, disjointness vs D6-D11,
sanité des comparateurs).

### 14.4 Outillage D12

| Composant                                                    | Description                                                                       |
|---------------------------------------------------------------|-----------------------------------------------------------------------------------|
| `protos/efe/v1/catalog.proto`                                 | Service `CatalogService` (6 RPCs) + 25 messages.                                   |
| `clients/python/efe_tool/_proto/efe/v1/catalog_pb2{,_grpc,.pyi}` | Stubs Python regénérés via `grpc_tools.protoc`.                                  |
| `clients/python/efe_tool/admin/types.py`                      | 4 dataclasses publiques (`EvolveOp`, `RegisteredCatalog`, …).                      |
| `clients/python/efe_tool/admin/client.py`                     | 5 nouvelles méthodes + 4 helpers de conversion proto⇄dict.                         |
| `clients/python/efe_tool/cli/commands/catalog.py`             | 6 sous-actions CLI (register/evolve/list/versions/get/delete).                     |
| `crates/efe-server/src/catalog/registry.rs`                   | Trait `CatalogRegistry` + types canoniques (CatalogKey, CatalogContent, …).        |
| `crates/efe-server/src/catalog/memory.rs`                     | Impl `InMemoryRegistry` (`tokio::sync::RwLock<BTreeMap>`).                         |
| `crates/efe-server/src/catalog/policy.rs`                     | `CatalogPolicy` : quotas + invariants schéma + validation evolve.                  |
| `crates/efe-server/src/catalog/audit.rs`                      | `AuditEmitter` : `tracing::info!(target: "audit")` + `MemoryAuditSink` test-only.  |
| `examples/05_catalog_admin.py`                                | Exemple end-to-end (lifecycle complet, 8 étapes).                                  |
| `docs/howto/catalog-admin.md`                                 | How-to register/list/get/delete (~ 200 lignes).                                    |
| `docs/howto/catalog-evolution.md`                             | How-to des 5 ops d'évolution + patterns (~ 250 lignes).                            |
| `docs/reference/catalog-rpc.md`                               | Référence RPC (mirror du proto, pour clients non-Python ; ~ 350 lignes).           |
| `.github/workflows/bench-catalog.yml`                         | Workflow CI : 2 jobs (cargo test catalog::, pytest test_d12_*).                    |

### 14.5 Anti-régression D12

37 nouveaux tests Python anti-régression :

* `scripts/tests/test_d12_disjoint_keys.py` (9 tests) — pin
  `RESERVED_PREFIXES`, inventaire SLOs D12, namespacing, unicité,
  disjointness cross-sprint, sanité des comparateurs.
* `scripts/tests/test_d12_surface_extensions.py` (16 tests) — pin
  surface SDK : 4 nouveaux symboles top-level, identité des
  re-exports, shape `EvolveOp` (5 variantes), shape des 3 frozen
  dataclasses (RegisteredCatalog/Audit/Compiled), 5 méthodes sur
  `EfeAdminClient`, parser CLI 6 verbes, 5 valeurs `--op`.
* `scripts/tests/test_d12_proto_contract.py` (12 tests) — pin
  contrat proto : import des stubs, inventaire des 6 RPCs, des 25
  messages, shape des sous-messages clés, stabilité des tag
  numbers (RegisterCatalogRequest, CanonicalCatalog,
  CatalogMetadata), round-trip dict⇄proto via les helpers SDK.

27 tests Rust unitaires :

* `crates/efe-server/src/catalog/registry.rs::tests` (3 tests) —
  parse / round-trip canonical key, audit_tag de chaque variante.
* `crates/efe-server/src/catalog/policy.rs::tests` (8 tests) — shape
  validation (1-class rejection, duplicate rejection, regex
  class.name, threshold inversion), evolve op validation, quota
  caps.
* `crates/efe-server/src/catalog/audit.rs::tests` (3 tests) —
  emit_register / emit_evolve / emit_delete via `MemoryAuditSink`.
* `crates/efe-server/src/catalog/memory.rs::tests` (13 tests) —
  full registry lifecycle : register v1→v2, schema rejection, 5
  evolve ops, list_versions ascending, list_catalogs tenant-scoped,
  get include_compiled flag, delete reference check + cascade,
  quota cap.

### 14.6 Activation locale

```bash
# Régénérer les stubs Python (à faire si on touche le .proto)
python3 -m grpc_tools.protoc \
    --proto_path=protos \
    --python_out=clients/python/efe_tool/_proto \
    --grpc_python_out=clients/python/efe_tool/_proto \
    --pyi_out=clients/python/efe_tool/_proto \
    protos/efe/v1/catalog.proto
sed -i.bak \
    's|^from efe.v1 import catalog_pb2|from efe_tool._proto.efe.v1 import catalog_pb2|' \
    clients/python/efe_tool/_proto/efe/v1/catalog_pb2_grpc.py

# Tests Rust scaffolding
cargo test --package efe-server --lib catalog::

# Tests Python anti-régression D12
PYTHONPATH=clients/python python3 -m pytest scripts/tests/test_d12_*.py -v

# Smoke CLI (parser-only)
PYTHONPATH=clients/python python3 -c "
from efe_tool.cli.parser import parse_args
print(parse_args(['catalog', 'list']).catalog_action)
print(parse_args(['catalog', 'register', '--name', 'x', '--content', '/etc/hosts']).catalog_action)
"

# Lifecycle end-to-end (nécessite efe-server D12-bis)
EFE_TOKEN=$(cat /tmp/jwt) python examples/05_catalog_admin.py
```

L'audit complet vivant :
[`docs/audits/d12_catalog_report.md`](docs/audits/d12_catalog_report.md)
(skeleton à compléter dès l'arrivée du gRPC handler en D12-bis).

L'architecture détaillée :
[`docs/bench/d12_catalog_admin_versioning_architecture.md`](docs/bench/d12_catalog_admin_versioning_architecture.md)
(1 673 lignes, 23 sections, 12 décisions clés, 10 risques,
8-vector threat model, plan 6 j wallclock).

### 14.7 D12-bis — gRPC handler & runtime wire-up (v0.1.13.1)

**Mission.** Brancher le `CatalogService` sur le runtime tonic pour
que les 6 RPC (`RegisterCatalog`, `EvolveCatalog`, `GetCatalog`,
`ListCatalogs`, `ListCatalogVersions`, `DeleteCatalog`) soient
appelables depuis n'importe quel client (Python SDK, grpcurl,
`examples/05_catalog_admin.py`). Avant D12-bis le `.proto`
compilait, l'`InMemoryRegistry` D12 fonctionnait, mais aucun
handler tonic n'était attaché — le service catalog n'existait
QUE en `pb` types et tests Rust unitaires.

**Livrables (tous mergés).**

* `crates/efe-server/src/services/catalog/mod.rs` — handler
  `CatalogServiceImpl` (~1 100 lignes, 6 RPC + 4 helpers
  d'observation manuelle d'histogramme + 3 helpers d'audit
  + `assert_tenant_match` + 5 mappings `CatalogError → tonic::Status`).
  Instrumentation `tracing::Instrument` (les futures restent
  `Send` malgré les span guards) ; sanitisation des erreurs
  `Internal` (le message client est toujours `"internal error"`,
  jamais le contenu interne).
* `crates/efe-server/src/services/catalog/conversions.rs` —
  6 helpers proto↔domain + 2 builders de `CatalogMetadata`
  + 1 parseur de clé canonique (~700 lignes, 22 tests
  unitaires de round-trip).
* `crates/efe-server/build.rs` — ajout de `catalog.proto` dans
  la liste compilée par `tonic-build`.
* `crates/efe-server/src/auth/scopes.rs` — 6 mappings
  `/efe.v1.catalog.CatalogService/* → admin:catalog:read|write`.
  La séparation `read` (list/get) / `write` (register/evolve/delete)
  permet de provisioner des tokens read-only pour les dashboards
  / l'audit interne sans pouvoir muter.
* `crates/efe-server/src/runtime.rs` — wire-up complet :
    * Backend resolver (`Memory` → `InMemoryRegistry::new()` ;
      `Postgres` → `Err` explicite réservé pour D14).
    * Création unique de `CatalogAuditEmitter` + `CatalogPolicy`.
    * `health_reporter.set_serving::<CatalogServiceServer<…>>()`
      au démarrage + `set_not_serving` dans le watchdog
      shutdown.
    * `router.add_service(CatalogServiceServer::new(…))`.
* `crates/efe-server/src/config.rs` — bloc `CatalogConfig`
  + variables d'env `EFE_CATALOG_BACKEND`,
  `EFE_CATALOG_MAX_NAMES_PER_TENANT`, `…_VERSIONS_PER_NAME`,
  `…_CLASSES_PER_CATALOG`, `…_SENTENCES_PER_CLASS`.
* `deploy/helm/efe/values.yaml` + `templates/configmap.yaml`
  — bloc `config.catalog` (backend + 4 quotas) + nouveau
  helm test `test-catalog-reachable.yaml` qui appelle
  `grpc_health_probe -service=efe.v1.catalog.CatalogService`
  contre le pod fraîchement déployé. Le test est activé par
  défaut (pas de dépendance bench image).

**Tests Rust ajoutés (73 nouveaux).**

* 22 tests `services/catalog/conversions.rs` — round-trips proto↔domain,
  rejet des shapes invalides, builders de metadata.
* 29 tests `services/catalog/mod.rs` — handlers unitaires :
  * 6 happy paths (register / evolve / get / list / list-versions /
    delete) couvrant les 6 méthodes RPC.
  * 6 mappings d'erreur (`CatalogError::TenantQuotaExceeded`
    → `RESOURCE_EXHAUSTED` ; `…::CatalogReferenced` →
    `FAILED_PRECONDITION` ; `…::Conflict` → `FAILED_PRECONDITION` ;
    `…::Internal` → `INTERNAL` (sans leak du message interne) ;
    `…::NotFound` → `NOT_FOUND` ; `…::InvalidArgument` →
    `INVALID_ARGUMENT`).
  * 4 tests d'isolation D11 (`assert_tenant_match` ; rejet
    cross-tenant explicite sur `register` / `get` / `list`
    / `delete`).
  * 6 tests de validation (clé malformée, tenant absent,
    name vide, `EvolveOp` manquant, etc.).
  * 2 tests de label `principal_present_label` (true / false).
  * 5 tests d'émission audit (counter + tracing audit).
* 22 tests d'intégration `tests/d12bis_catalog_integration.rs`
  bout-en-bout sur tonic loopback :
  * 6 happy paths.
  * 5 RBAC (token absent → `UNAUTHENTICATED` ;
    scope read-only sur write → `PERMISSION_DENIED` ;
    scope write OK ; etc.).
  * 3 isolation D11 (mismatch JWT vs body → `PERMISSION_DENIED` ;
    cross-tenant get → `PERMISSION_DENIED` ; cross-tenant list →
    `PERMISSION_DENIED`).
  * 3 mappings d'erreur (empty name → `INVALID_ARGUMENT` ;
    delete référencé sans force → `FAILED_PRECONDITION` ;
    clé invalide → `INVALID_ARGUMENT`).
  * 2 tests de concurrence (two parallel registers vers tuples
    distincts ; two parallel `evolve` depuis la même `from_key`
    → versions distinctes monotones — wire-level surface du
    SLO **D12-CT-6**).
  * 3 tests d'observabilité (incrément du compteur
    `efe_audit_events_total` avec les bons labels D13 C-1 :
    `{tenant, event_type, category, principal_present}`).

**Tests Python (surface pinning, à venir J5.2).**

* `scripts/tests/test_d12bis_catalog_grpc_surface.py` — pin du
  contrat fil sur les 6 RPC + assertion que le module
  `efe_tool._proto.efe.v1.catalog_pb2_grpc` expose les 6 stubs
  attendus.
* `scripts/tests/test_d12bis_helm_render.py` — vérifie que le
  bloc `config.catalog` rend bien 5 variables d'env dans le
  ConfigMap.

**SLO ajoutés en D12-bis (3 nouveaux).**

| ID | Description | Cible |
|----|-------------|-------|
| **D12bis-CT-1** | `RegisterCatalog` p99 latency on loopback (in-memory backend) | **≤ 5 ms** |
| **D12bis-CT-2** | `EvolveCatalog` p99 latency under contention (8 concurrent writers, same `from_key`) | **≤ 12 ms** |
| **D12bis-CT-3** | `GetCatalog` p50 with `include_compiled=false` | **≤ 1 ms** |

Ces SLO sont mesurés sur la même calibration que D4
(loopback TCP, M3 Pro, naive linalg, 4 cores réservés). Ils
seront automatisés par `benches/catalog_rpc.rs` en J5.2 si la
charge bench le justifie ; sinon les tests d'intégration
Rust suffisent (les latences observées en `cargo test` sont
sub-milliseconde, bien sous les SLO).

**Anti-régression D12-bis (matrice consolidée).**

| Vérification | Outil |
|--------------|-------|
| 6 RPC compilent (proto stubs présents) | `cargo build -p efe-server` |
| 51 tests handler unitaires verts | `cargo test -p efe-server --lib services::catalog` |
| 22 tests d'intégration wire-level verts | `cargo test -p efe-server --test d12bis_catalog_integration` |
| Catalog wired dans le runtime | `health_reporter.set_serving::<CatalogServiceServer>` test |
| Helm test `helm test efe -n efe` | exécute `test-catalog-reachable.yaml` |
| Stubs Python régénérés | check `efe_tool._proto.efe.v1.catalog_pb2_grpc` |
| Aucune régression D11 (isolation) | `cargo test -p efe-server --lib auth::` |
| Aucune régression D13 (observabilité) | `cargo test -p efe-telemetry --lib metrics::d13_instruments` |

**Activation locale.**

```bash
# 1. Recompiler le serveur (catalog.proto pour la première fois).
cargo build -p efe-server

# 2. Démarrer en local.
EFE_AUTH_ENABLED=false \
EFE_CATALOG_BACKEND=memory \
RUST_LOG=info,efe=debug \
cargo run -p efe-server --release

# 3. Smoke catalog avec grpcurl + reflection.
grpcurl -plaintext 127.0.0.1:50051 list
# Doit inclure : efe.v1.catalog.CatalogService

grpcurl -plaintext \
    -d '{"tenant":"anonymous","name":"smoke","version":"1.0.0",
         "content":{"embedder":{"family":"sentence-transformers",
         "model":"x","dim":4,"normalize":"l2"},
         "classes":[{"name":"a","canonical_sentences":["alpha"]}]}}' \
    127.0.0.1:50051 efe.v1.catalog.CatalogService/RegisterCatalog
# Doit retourner : {"key":"anonymous/smoke@v1", "catalog":{...}}

# 4. Lifecycle Python end-to-end.
EFE_TOKEN= python examples/05_catalog_admin.py register \
    --name smoke --content fixtures/catalog_smoke.json
```

**Architecture détaillée.**
[`docs/bench/d12bis_catalog_grpc_handler_architecture.md`](docs/bench/d12bis_catalog_grpc_handler_architecture.md)
(2 736 lignes : 17 sections, 9 décisions clés, 11 risques
ranked, 6-vector threat model — superposé au threat model D12 —,
plan 5 j wallclock).

## 15. Observability dashboards (Sprint D13, v0.1.13)

### 15.1 Mission

Sprint D13 transforme EFE en service exploitable par une équipe
SRE de partenaire :

1. **6 dashboards Grafana JSON** persona-driven dans
   `deploy/observability/grafana/dashboards/d13/` (78 panneaux
   au total) : Operator Overview, Tenant Drill-Down, Catalog
   Version Distribution, Cost Attribution, Audit Stream,
   Capacity Planning. Auto-discovery via le sidecar Grafana
   k8s-sidecar (label `grafana_dashboard=1`).
2. **7 recording rules** + 2 niveaux de downsampling (`:1h`,
   `:1d`) dans
   `deploy/observability/prometheus/rules.d13/recording.yaml`.
3. **10 alertes Prometheus** runbook-linked dans
   `deploy/observability/prometheus/rules.d13/alerts.yaml`.
   Chaque alerte a un `runbook_url` annotation pinné par CI.
4. **5 streams de logs structurés** JSON-Schema versionnés
   dans `deploy/observability/log-schemas/` :
   `_common`, `request`, `audit`, `classify`, `evolution`,
   `security`. Schéma stable v1.0.
5. **8 nouveaux instruments Prometheus** côté Rust dans
   `crates/efe-telemetry/src/metrics/d13_instruments.rs` :
   5 histogrammes pour les 5 RPCs catalog + 1 gauge
   `efe_catalog_versions_per_name` + counter `efe_audit_events_total`
   + counter `efe_alert_fires_total` + histogram
   `efe_dashboard_query_duration_seconds`.
6. **4 templates Helm** : `grafana-dashboards.yaml`,
   `prometheusrule-d13.yaml`, `promtail-config.yaml`,
   `tests/test-dashboards-load.yaml`. Tous gated derrière
   le nouveau bloc `observability.*` de `values.yaml`
   (default OFF, backward-compat avec single-tenant).
7. **10 runbooks** sous `docs/runbooks/d13_*.md` matching
   les 10 alertes ; CI rejects toute alerte sans runbook.
8. **4 pages Diátaxis** :
   `docs/howto/observability-setup.md`,
   `docs/howto/cardinality-management.md`,
   `docs/reference/dashboard-catalog.md`,
   `docs/explanation/observability-architecture.md`.
9. **7 SLOs `D13-OBS-*`** dans
   `scripts/parse_criterion_report.py::SLO_DEFINITIONS`,
   sous le sous-namespace `obs::dash::*`.

### 15.2 SLOs ajoutés en D13 (7 nouveaux)

| ID             | Bench key                              | Comparator | Cible       | Mesure                                                   |
|----------------|----------------------------------------|------------|-------------|----------------------------------------------------------|
| `SLO-D13-OBS-1` | `obs::dash::load_p99`                  | `lte`      | 3 s          | Grafana panel render p99 (synthetic UX bench)            |
| `SLO-D13-OBS-2` | `obs::dash::scrape_success_pct`        | `gte`      | 99.9 %       | Prom `up{job="efe-server"}` averaged over 30 d           |
| `SLO-D13-OBS-3` | `obs::dash::log_delivery_p99_s`        | `lte`      | 30 s         | promtail position cursor lag                             |
| `SLO-D13-OBS-4` | `obs::dash::trace_sampling_pct`        | `gte`      | 99 %         | Tempo span ingestion / Prom request count ratio          |
| `SLO-D13-OBS-5` | `obs::dash::cardinality_bound_count`   | `lte`      | 50 000      | Total `efe_*` series count                               |
| `SLO-D13-OBS-6` | `obs::dash::alert_to_dashboard_s`      | `lte`      | 30 s         | UX click-through alert → dashboard render                |
| `SLO-D13-OBS-7` | `obs::dash::runbook_coverage_pct`      | `gte`      | 100 %        | Alertes avec `runbook_url` annotation                    |

Tous SKIPPED en `bench_baselines/v0.1.13/baseline.json` jusqu'au
1er run main vert post-merge de
`.github/workflows/bench-observability.yml` + landing du
gRPC handler D12-bis.

### 15.3 Disjoint-key invariant (étendu)

`bench/load/parse_ghz_report.py::RESERVED_PREFIXES` passe de
**10** à **11** entrées :

```python
RESERVED_PREFIXES = (
    "eval_accuracy::",   # D6/D7/D8
    "eval_latency::",    # D6/D7/D8
    "load_",             # D9
    "soak::",            # D9
    "docs::",            # D10
    "dx::",              # D10
    "isolation::",       # D11
    "cost::",            # D11
    "obs::",             # D11
    "catalog::",         # D12
    "obs::dash::",       # D13 NEW (sub-namespace under obs::)
)
```

Le sous-namespace `obs::dash::` cohabite avec `obs::*` parce que
les SLOs D11 (`obs::tenant_cardinality_count`) et D13
(`obs::dash::load_p99`, etc.) ont des bench_keys disjointes. La
co-existence est validée par
`test_d13_disjoint_keys.py::test_obs_dash_is_subnamespace_of_obs`.

### 15.4 Outillage D13

| Composant                                                | Description                                                              |
|----------------------------------------------------------|--------------------------------------------------------------------------|
| `deploy/observability/log-schemas/{_common,request,audit,classify,evolution,security}.json` | 5 streams + envelope commun, schema_version "1.0" |
| `deploy/observability/prometheus/rules.d13/recording.yaml` | 7 recording rules + 5 downsampled (`:1h`, `:1d`) |
| `deploy/observability/prometheus/rules.d13/alerts.yaml`    | 10 alertes runbook-linked                                                |
| `deploy/observability/grafana/dashboards/d13/*.json`       | 6 dashboards (78 panneaux)                                                |
| `deploy/observability/loki/promtail-config.example.yaml`   | Promtail example pour ingestion Loki                                    |
| `crates/efe-telemetry/src/metrics/d13_instruments.rs`      | 8 nouveaux instruments Prometheus + 8 tests unitaires                   |
| `deploy/helm/efe/templates/grafana-dashboards.yaml`        | ConfigMaps avec label `grafana_dashboard=1`                             |
| `deploy/helm/efe/templates/prometheusrule-d13.yaml`        | PrometheusRule combinée (recording + alerts)                             |
| `deploy/helm/efe/templates/promtail-config.yaml`           | Optional ConfigMap pour le promtail example                              |
| `deploy/helm/efe/templates/tests/test-dashboards-load.yaml` | helm test : 6 UIDs return HTTP 200                                       |
| `docs/runbooks/d13_*.md`                                   | 10 runbooks structurés (template fixe : 7 sections)                     |
| `docs/howto/observability-setup.md`                        | Guide d'installation Tier 2                                              |
| `docs/howto/cardinality-management.md`                     | Budgets + recording rules + allowlist                                    |
| `docs/reference/dashboard-catalog.md`                      | Référence des 6 dashboards (panneaux, queries)                          |
| `docs/explanation/observability-architecture.md`           | Le « pourquoi » de la stack                                             |
| `.github/workflows/bench-observability.yml`                | 5 jobs CI (dashboards-validate, promtool-check, helm-lint, python-tests, rust-telemetry-tests) |

### 15.5 Anti-régression D13

**~ 70 nouveaux tests Python** anti-régression (190/192 verts en
local, 2 skipped sur l'absence de promtool) :

* `test_d13_disjoint_keys.py` (8 tests) — RESERVED_PREFIXES
  inventaire, SLOs D13 inventaire, namespacing,
  cross-sprint disjointness.
* `test_d13_log_schemas.py` (8 tests) — 5 schémas existent et
  parsent, JSON-Schema validation d'un event audit, refus des
  events malformés, références $ref `_common`.
* `test_d13_dashboard_jsonschema.py` (10 tests) — 6 dashboards
  parsent, UIDs uniques, schemaVersion ≥ 39, ≥ 3 panels,
  Prometheus declared, naming convention, tags, links.
* `test_d13_alert_runbook_links.py` (40+ tests via parametrize)
  — 10 alertes présentes, chacune a un `runbook_url`, fichier
  runbook existe, `for: ≥ 5m`, severity canonical.
* `test_d13_dashboard_inventory.py` (18 tests via parametrize)
  — surface pinning des 6 UIDs / titres / tags.
* `test_d13_promtool_validation.py` (5 tests, 2 skipped sans
  promtool) — recording.yaml + alerts.yaml valides PromQL.
* `test_d13_helm_templates.py` (6 tests) — 4 templates
  existent + références correctes, dashboards default OFF.
* `test_d13_metric_label_allowlist.py` (10 tests via
  parametrize) — pin du label set sur les 8 instruments,
  refus des labels haute-cardinalité (`request_id`, etc.).
* `test_d13_runbook_coverage.py` (90+ tests via parametrize)
  — 10 runbooks existent, structure template (Severity, Owner,
  7 sections), references dashboards / artefacts, no orphan.

**8 nouveaux tests Rust unitaires** :

* `metrics::d13_instruments::tests` (8 tests) — chaque
  instrument observable / incrémentable, register_all_d13
  idempotent, 8 family names exposés.

### 15.6 Activation locale

```bash
# Tests Python anti-régression D13
PYTHONPATH=clients/python python3 -m pytest scripts/tests/test_d13_*.py -v

# Tests Rust telemetry
cargo test -p efe-telemetry --lib metrics::d13_instruments

# Smoke import du module Rust
cargo check -p efe-telemetry --lib

# Validate dashboards JSON schema
for f in deploy/observability/grafana/dashboards/d13/*.json; do
  python3 -m json.tool "$f" > /dev/null && echo "OK: $(basename $f)"
done

# Validate PromQL
docker run --rm -v "$PWD/deploy/observability/prometheus:/work" \
    prom/prometheus:v2.49.0 \
    promtool check rules /work/rules.d13/recording.yaml \
                         /work/rules.d13/alerts.yaml

# Helm lint + template
helm lint deploy/helm/efe
helm template efe deploy/helm/efe \
    --set "observability.dashboards.enabled=true" \
    --set "observability.recordingRules.enabled=true" \
    --set "observability.alerts.enabled=true" \
    | grep -c "kind: ConfigMap"
# Expected output : ≥ 7 (6 dashboards + 1 base ConfigMap)
```

L'architecture détaillée :
[`docs/bench/d13_observability_dashboards_architecture.md`](docs/bench/d13_observability_dashboards_architecture.md)
(2 312 lignes, 24 sections, 12 décisions clés, 10 risques,
10-vector HAZOP, plan 6 j wallclock).

### 15.7 D13 — Phase 2 (deep integration)

Une **seconde passe code** s'est livrée juste après la Phase 1 :
elle ajoute la couche **émission effective** des télémétries que
Phase 1 a déclarée (schémas + dashboards + alertes), plus les
outils opérationnels associés.

**Module Rust D13 Phase 2** (additif sous
`crates/efe-telemetry/src/`) :

* `events/d13_streams.rs` — 5 builders typés
  (`RequestEvent`, `AuditEvent`, `ClassifyEvent`,
  `EvolutionEvent`, `SecurityEvent`) avec un envelope commun
  (`CommonEnvelope`) ; chaque builder produit un
  `serde_json::Value` qui valide contre le JSON-Schema
  correspondant. Formatter RFC 3339 stdlib-only (algorithme
  Howard Hinnant) pour éviter une nouvelle dep `chrono`/`time`.
* `spans.rs` — 16 constantes de noms de span
  (`efe.<component>.<action>`) + 18 constantes de noms
  d'attribut + liste de 14 préfixes interdits (PII / secrets /
  cardinalité).
* `otel/mod.rs` — config struct `OtelConfig` + `init()`
  no-op qui résout 6 env vars (`EFE_OTEL_ENABLED`,
  `_ENDPOINT`, `_SAMPLE_RATE`, `_FORCE_SAMPLE_ON_ERROR`,
  `_SERVICE_NAME`, `_SERVICE_VERSION`). `init()` retourne
  `SdkNotImplemented` quand `enabled=true` (D13-bis pendant).
* `observability_init_d13.rs` — bootstrap unique
  `bootstrap_observability(&cfg)` qui chaîne
  `register_all_d13()` + `otel::init()` + retourne un
  `ObservabilityGuard` RAII.

**40 nouveaux tests Rust** : 116 tests `cargo test -p
efe-telemetry` verts (was 76 post-Phase 1).

**4 scripts opérationnels** sous `scripts/observability/` :

| Script | Use case |
|--------|----------|
| `cardinality_probe.py`     | Scrape `/metrics` + report top-N cardinality (runbook `d13_obs_cardinality_bomb`) |
| `dashboard_smoke_test.py`  | Hit Grafana API for the 6 D13 UIDs ; pass/fail SLO D13-OBS-1 |
| `grafana_org_provision.py` | Create per-tenant Grafana orgs + datasource avec X-Scope-OrgID (Mimir) |
| `export_audit_archive.py`  | Pull audit stream from Loki + validate vs schema + write gzipped NDJSON (compliance 7 ans) |

**3 sample configs** sous `deploy/observability/` :

| Config                                                 | Purpose |
|--------------------------------------------------------|---------|
| `otel-collector/config.yaml`                           | OTLP receivers + memory_limiter + PII allowlist + Tempo exporter |
| `grafana/datasources/datasources.yaml`                 | Prom + Loki + Tempo + Alertmanager avec exemplar / traces-to-logs / traces-to-metrics |
| `loki/loki-config.example.yaml`                        | auth_enabled + retention par stream (audit = 7 ans) + ingest quotas per-tenant |

**~ 70 nouveaux tests Python anti-régression Phase 2** :

* `test_d13_phase2_event_emitters.py` — 19 tests :
  envelope fields ↔ schema, enum sets identical, 5 streams
  builders, forbidden attrs.
* `test_d13_phase2_otel_config.py` — 7 tests : env vars,
  defaults, values.yaml block, otel-collector PII
  allowlist.
* `test_d13_phase2_cardinality_probe.py` — 6 tests :
  parse, filter, report-text/json, --input-file flow.
* `test_d13_phase2_dashboard_smoke.py` — 3 tests : UIDs ↔
  files bidirectional pin.
* `test_d13_phase2_grafana_org_provision.py` — 3 tests :
  --dry-run, org-name default + override.
* `test_d13_phase2_audit_archive_export.py` — 10 tests :
  ISO parse, unix-ns conversion, schema validation
  acceptance + rejection, gzip NDJSON output, exit codes.
* `test_d13_phase2_sample_configs.py` — 16 tests : OTel
  collector / Grafana datasources / Loki retention.

### 15.8 Activation locale (Phase 2)

```bash
# Cardinality probe
python3 scripts/observability/cardinality_probe.py \
    --url http://127.0.0.1:9090/metrics --top 20

# Dashboard smoke
python3 scripts/observability/dashboard_smoke_test.py \
    --grafana-url http://127.0.0.1:3000

# Cargo tests for the new Rust modules
cargo test -p efe-telemetry --lib

# All Phase 2 Python tests
PYTHONPATH=clients/python python3 -m pytest \
    scripts/tests/test_d13_phase2_*.py -v
```

## 16. Postgres-backed catalog registry (Sprint D14, v0.1.14)

### 16.1 Mission

D14 résout le **pain point fondamental** :
`restart efe-server = perte totale de tous les catalogues`. La
[`PostgresCatalogRegistry`](../crates/efe-server/src/catalog/postgres.rs)
remplace `InMemoryRegistry` derrière le **même trait
[`CatalogRegistry`]** (D12) — le handler gRPC D12-bis ne sait
pas avec quel backend il parle. Persistence durable + multi-réplica
safety + backup/restore + audit trail SQL durable.

### 16.2 Architecture (résumé)

10 décisions clés explicitement justifiées (cf.
[`docs/bench/d14_postgres_catalog_backend_architecture.md`](docs/bench/d14_postgres_catalog_backend_architecture.md)
§ 19) :

| # | Décision | Alternatives écartées |
|--:|----------|----------------------|
| 1 | **sqlx 0.7** runtime-checked | diesel sync, sea-orm overhead, tokio-postgres verbose |
| 2 | **Single `catalogs` table + composite PK** | LIST partition (D17+), table-par-tenant (rejected — quote injection) |
| 3 | **`pg_advisory_xact_lock(xxh64(tenant/name))`** | SERIALIZABLE retry, sequence-per-key, ON CONFLICT retry |
| 4 | **JSONB content + labels** | TEXT serialized (pas indexable), columns séparées (overkill v1) |
| 5 | **`sqlx::migrate!` forward-only** | flyway, refinery, raw scripts |
| 6 | **Pool 8 conn/réplique** | 4 (perf), 16/32 (sub-PG-max=100 × 3 répliques) |
| 7 | **Migration au boot fail-fast** | hors-binaire (opérateur peut oublier) |
| 8 | **App-level reference check** | FK ON DELETE CASCADE (perd `Conflict` mapping) |
| 9 | **Pas de RLS en v0.1.14** | RLS default-on (complexité ops + perf overhead) |
| 10 | **`EFE_PG_TEST_URL`-gated tests** | testcontainers-rs (incompatible rustc 1.85) |

### 16.3 SLOs ajoutés en D14 (7 nouveaux)

| ID | Description | Cible | Type |
|----|-------------|------:|------|
| **D14-PG-1** | `RegisterCatalog` p99 (loopback PG) | ≤ 20 ms | GATE |
| **D14-PG-2** | `EvolveCatalog` p99 sous 8-concurrent | ≤ 40 ms | GATE |
| **D14-PG-3** | `GetCatalog` p50 | ≤ 3 ms | GATE |
| **D14-PG-4** | Migration apply on empty PG | ≤ 5 s | OBSERVATIONAL |
| **D14-PG-5** | Pool active connections sustained 5 min | ≤ 90 % of max | ALERT |
| **D14-PG-6** | Concurrent register same key | strictly monotone versions | INVARIANT |
| **D14-PG-7** | Cross-tenant SELECT/UPDATE/DELETE | returns 0 rows | INVARIANT |

### 16.4 Outillage D14

| Fichier | Rôle |
|---------|------|
| `crates/efe-server/src/catalog/postgres.rs` (~1 350 lignes) | `PostgresCatalogRegistry` impl, 6 méthodes du trait, advisory lock, error mapping table, 18 unit tests |
| `crates/efe-server/migrations/0001_init.sql` | DDL : catalogs table, composite PK, JSONB, partial parent_key index, db_updated_at trigger, 4 CHECK constraints |
| `crates/efe-server/migrations/README.md` | Forward-only playbook ops |
| `crates/efe-server/src/config.rs` | `CatalogPostgresConfig` 13 fields, 11 env vars, DSN builder + redaction, 8 validation invariants |
| `crates/efe-server/src/runtime.rs` | `build()` async, branche Postgres ouverte, register_all_d14 au boot |
| `crates/efe-telemetry/src/metrics/d14_instruments.rs` | 4 instruments : pool gauges (×2), query histogram, tx failures counter, migration version gauge |
| `crates/efe-telemetry/src/spans.rs` | 7 nouveaux spans `efe.pg.*` |
| `deploy/helm/efe/values.yaml` | bloc `config.catalog.postgres` (15 fields, 3 secret paths) |
| `deploy/helm/efe/templates/configmap.yaml` | 11 env vars `EFE_PG_*` gated `backend=postgres`, password JAMAIS rendu |
| `deploy/helm/efe/templates/deployment.yaml` | 3 secret paths (`existingSecretName`, `externalSecretName`, inline) |
| `crates/efe-server/tests/d14_postgres_catalog_integration.rs` | 21 tests `EFE_PG_TEST_URL`-gated, 9 sections (happy paths + erreurs + concurrency + persistence + quotas + isolation + migrations + SQL injection + pagination) |
| `scripts/tests/test_d14_postgres_helm_render.py` | 18 tests pin Helm + DDL invariants |
| `scripts/tests/test_d14_runtime_wireup.py` | 18 tests pin runtime + config + telemetry + spans |

### 16.5 Anti-régression D14

```bash
# Lib (handler + helpers) — 266 tests
cargo test -p efe-server --lib

# Trait contract sur PG réel (gated EFE_PG_TEST_URL)
docker run --rm -d --name efe-test-pg \
    -e POSTGRES_USER=efe -e POSTGRES_PASSWORD=efe \
    -e POSTGRES_DB=efe_catalog_test -p 5433:5432 postgres:15
EFE_PG_TEST_URL='postgres://efe:efe@127.0.0.1:5433/efe_catalog_test' \
    cargo test -p efe-server --test d14_postgres_catalog_integration
docker rm -f efe-test-pg

# Helm + runtime pin tests Python (36 tests)
PYTHONPATH=clients/python python3 -m pytest scripts/tests/test_d14_*.py -v

# Cross-sprint regression (preserves D11/D12/D12-bis/D13 invariants)
PYTHONPATH=clients/python python3 -m pytest scripts/tests/ -q
cargo test -p efe-server --lib --test d12bis_catalog_integration \
    --test d14_postgres_catalog_integration --test integration_grpc \
    --test shutdown_behavior --test startup_sequence
```

### 16.6 Activation locale

```bash
# 1. Provisionner Postgres (Docker exemple).
docker run --rm -d --name efe-pg \
    -e POSTGRES_USER=efe_catalog \
    -e POSTGRES_PASSWORD=efe \
    -e POSTGRES_DB=efe_catalog_db \
    -p 5432:5432 postgres:15

# 2. Démarrer efe-server avec Postgres backend.
EFE_AUTH_ENABLED=false \
EFE_CATALOG_BACKEND=postgres \
EFE_PG_HOST=127.0.0.1 \
EFE_PG_USER=efe_catalog \
EFE_PG_PASSWORD=efe \
EFE_PG_DATABASE=efe_catalog_db \
RUST_LOG=info,efe=debug,sqlx=warn \
cargo run -p efe-server --release

# 3. Smoke catalog avec grpcurl + reflection.
grpcurl -plaintext 127.0.0.1:50051 list
# Doit inclure : efe.v1.catalog.CatalogService

# 4. Vérifier le metric migration version est ≥ 1.
curl -s http://127.0.0.1:9090/metrics | grep efe_pg_migration_version
# efe_pg_migration_version 20240427120000   # = sqlx migration timestamp
```

L'architecture détaillée :
[`docs/bench/d14_postgres_catalog_backend_architecture.md`](docs/bench/d14_postgres_catalog_backend_architecture.md)
(2 391 lignes : 25 sections, 10 décisions clés, 10 risques,
8-vector threat model, plan 7 j wallclock + 1 j buffer).

## 8. Évolution

* **v0.1.0** : baseline initiale, **mocks** uniquement (Sprint C).
* **v0.1.1** : ajoute la suite **production** (`real_strategies_e2e`,
  `real_memory`) + fixtures v2 (Gaussian embeddings) + 4 tests
  anti-régression dans `tests/bench_realism_check.rs`. SLO `RD1-*`
  calibrés (post-D5 audit) à 400 / 700 / 700 ms (small / medium /
  large) — voir § 0. Les targets initiaux 5/50/250 ms étaient
  basés sur un single-round happy path qui ne reflète pas la
  réalité multi-round du backend.
* **v0.1.2** : ajoute la suite **infra mémoire**
  (`memory_real_arena`, 7 groupes criterion) + 4 tests
  anti-régression dans `tests/pool_realism_check.rs`. 10 nouveaux
  SLO `D2-*` calibrés sur Apple M3 Pro (cf. § 3 bis).
* **v0.1.3** : ajoute la suite **compute pipeline**
  (`compute_pipeline`, 3 groupes criterion : chain / per_phase /
  throughput) + 3 tests anti-régression dans
  `tests/compute_pipeline_realism.rs`. 6 nouveaux SLO `D3-*` (3
  naïve + 3 BLAS) calibrés sur les v2 fixture shapes (cf. § 3 ter).
  Per-phase additivité validée à 95-99 % (chain ≈ Σ phases).
* **v0.1.4** : ajoute la suite **gRPC server stack**
  (`server_rpc`, 4 groupes criterion : full_stack / minimal_stack /
  concurrent / codec_only) + 3 tests anti-régression dans
  `tests/server_rpc_realism_check.rs`. 4 nouveaux SLO `D4-*`
  calibrés sur loopback TCP réel (cf. § 3 quater). Découverte clé :
  l'overhead RPC (tonic + interceptors + bridge) est sub-millisecond
  et **noyé sous le bruit du loopback** — le backend multi-round
  domine à 95+ %.
* **v0.1.5** (cette version, Sprint D5) : refonte de l'orchestration
  — les **10 benches** (Sprint C, S2-4, D1-4) sont désormais pilotés
  par un **catalog déclaratif unique** (`scripts/benches.list`) et
  **5 jobs CI parallèles**. Premier run réel des 3 micro-benches
  Sprint S2/S3/S4 jusque-là orphelins. CI wallclock divisé par 2
  (~15 min vs 30 min). PR comment automation + régression gate
  contre la dernière baseline (auto-detect via `sort -V`).
* **v0.1.6** (Sprint D6) : consensus pacing fix + accuracy SLO
  infrastructure. `ControllerConfig.min_rounds=4` côté Mode-1, 6
  thresholds consensus durcis, parser `gte/lte` comparator, 5
  nouveaux SLO `D6-*` (4 ACC + 1 LAT). L'audit a confirmé que la
  pipeline marche mais a exposé H3 (biais structurel
  genesis/fusion) que D7 corrige.
* **v0.1.7** (Sprint D7) : Genesis/Fusion bias
  remediation (cf. § 9). Helper `build_mode1_fusion_config()` —
  **winner empirique post-sweep** (cf.
  `docs/audits/d7_calibration_report.md` § 6.1) :
  β₀=0.45 (was 0.20), **λ_β=0.08** (was 0.15 ; analytique D7 : 0.05),
  **α₀=0.25** (was 0.50 ; analytique D7 : 0.30). 4
  env vars `EFE_FUSION_*` pour la calibration sans recompile,
  instrumentation `EFE_LOG_CENTROID_TRACE`, 2 nouveaux golden
  datasets multi-catalog (`fraud_alert.jsonl`,
  `billing_escalation.jsonl`), `eval/scripts/calibrate_mode1.py`
  sweep automatisé, `run_eval.py --multi-catalog`, 4 nouveaux SLO
  `D7-MC-*` anti-overfit. Plan B
  (`integrations/hybrid_decoder.py`) armé mais non activé.
* **v0.1.8** (Sprint D8) : Plan B activation +
  Real-Data validation infrastructure (cf. § 10). `HybridClassifier`
  formellement activé : aggregate macro_f1 0.815 → 0.857 (+0.042)
  sur 3 catalogues synthétiques, gain le plus élevé sur
  `billing_escalation` (+0.078). 7 SLOs `D8-*` ajoutés (4 hybrid
  + 2 real-data + 1 latency). 6 nouveaux scripts Python + 1 example
  + workflow CI `pii_check.yml` + extension `bench-accuracy-hybrid`
  job. `AnchorAwareTemplateRegistry` opt-in avec 10 tests pinés.
  46 nouveaux tests anti-régression. Real-data SLOs (D8-RD-*)
  restent SKIPPED jusqu'à acquisition partenaire pilote (D8.2).
* **v0.1.9** (Sprint D9) : Docker image + Helm chart
  + load/soak (cf. § 11). Image distroless cc-debian12 nonroot
  multi-stage avec reproductibilité partielle
  (`--remap-path-prefix`, SOURCE_DATE_EPOCH pinned). Helm chart
  `deploy/helm/efe/` avec 13 templates dont 9 amendements appliqués
  (A2/A4/A5/A6/A7/A8/A9 actifs, A1/A3 partiels). Load harness
  `bench/load/` 4 scénarios ghz + parser stdlib-only. Soak harness
  `bench/soak/` avec détecteur OLS (puissance ~99% à n=1440 / σ=5MB).
  5 SLOs `D9-*` (4 LOAD + 1 SOAK) ajoutés à
  `scripts/parse_criterion_report.py`. 67 nouveaux tests
  anti-régression. Documentation opérations sous `docs/operations/`
  (quickstart 10 min + 5 runbooks). Workflow `bench-load.yml` +
  `reproducibility.yml` (nightly). Distroless symbol checker
  intégré au release pipeline. SLOs `D9-LOAD-*` SKIPPED jusqu'aux
  3 premiers runs main verts post-merge.
* **v0.1.10** (Sprint D10) : SDK docs + quickstart
  + 4 examples — DX Pilot 1.0 (cf. § 12). SDK Python publié sur
  PyPI via OIDC trusted publisher (`efe-tool==0.1.10`). Surface
  publique freezée : `__all__` étendu avec hiérarchie d'erreurs
  `Efe*Error` (mappée 1:1 sur les status codes gRPC),
  `AsyncEfeClient` skeleton (thread-pool en v0.1.x → native
  grpc.aio en v0.2.x), `py.typed` marker (PEP 561). Documentation
  Diátaxis sur `docs.efe.ai/0.1.10/` (19 pages : 3 tutorial + 7
  howto + 5 reference + 4 explanation), API ref auto-générée via
  `mkdocstrings`, versioning via `mike`. 4 exemples copy-paste-ready
  sous `examples/0X_*.py` (50 LOC → 225 LOC selon complexité).
  Stub server gRPC pour smoke en CI sans booter le binaire Rust
  (`tests/stub_server.py`). 5 nouveaux SLOs `D10-*` (3 DOC + 2 DX)
  dans namespaces disjoints `docs::*` / `dx::*`. Workflow
  `docs.yml` + extension `release.yml` (publish-pypi + mike deploy
  versioned). `scripts/validate_quickstart.sh` automatise les 6
  étapes quickstart pour 3 reviewers externes ; SLO `D10-DX-1`
  ≤ 10 min wallclock. ~30 nouveaux tests anti-régression. Aucun
  des 21 SLOs antérieurs régressé.
* **v0.1.11** (Sprint D11, cette version) : per-tenant isolation
  (cf. § 13). SDK étendu avec `efe_tool.admin` sub-package
  (`EfeAdminClient`, `CatalogRef`, `AdminError`). Helm Tier 2
  opt-in (3 nouveaux templates : ResourceQuota, LimitRange,
  NetworkPolicy ingress per-tenant) + extension ServiceMonitor
  (relabel `tenant`) + PrometheusRule (4 alertes per-tenant +
  cardinality bomb detection). Cost attribution via
  `scripts/billing/aggregate_usage.py` (300+ LOC stdlib-only).
  3 nouvelles pages Diataxis (howto/multi-tenant,
  explanation/security-model, reference/tenant-claims). Grafana
  dashboard JSON committed sous `deploy/observability/grafana/`.
  7 SLOs `D11-MT-*` (4 isolation + 2 cost + 1 cardinalité) avec
  3 nouveaux `RESERVED_PREFIXES` (isolation/cost/obs). Workflow
  `.github/workflows/bench-isolation.yml` (3 jobs : Rust
  isolation, Helm dual-render, Python anti-regression).
  39 nouveaux tests Python anti-régression. Architecture
  détaillée 1 845 lignes (21 sections, 12 décisions, 10 risques,
  HAZOP 12 vecteurs).
* **v0.1.12** (Sprint D12, cette version) : catalog admin endpoints
  + versioning (cf. § 14). Nouveau service gRPC
  `efe.v1.catalog.CatalogService` (6 RPCs) dans un fichier proto
  séparé (additif, ne casse pas les clients M9-era). Scaffolding
  Rust additif sous `crates/efe-server/src/catalog/` (4 modules,
  27 tests unitaires) ; le wire-up gRPC vers `runtime::Server`
  est explicitement déféré à D12-bis pour rester additif sur
  les 1 700+ tests existants. SDK Python étendu : 5 nouvelles
  méthodes sur `EfeAdminClient` (register/evolve/get/list-versions/
  delete) + activation de `list_catalogs` (qui était stubée en D11).
  4 nouveaux types publics (`EvolveOp`, `RegisteredCatalog`,
  `CatalogAuditMetadata`, `CompiledClassRef`) re-exportés au top-
  level. CLI : `efe-admin catalog {register,evolve,list,versions,
  get,delete}` (6 verbes). 3 nouvelles pages Diataxis (howto/
  catalog-admin, howto/catalog-evolution, reference/catalog-rpc).
  7 SLOs `D12-CT-*` (5 latency + 2 correctness) avec un nouveau
  `RESERVED_PREFIX` `catalog::`. Workflow `.github/workflows/bench-
  catalog.yml` (2 jobs : cargo test catalog::, pytest test_d12_*).
  37 nouveaux tests Python anti-régression + 27 tests Rust
  unitaires. Aucun des 33 SLOs antérieurs n'a régressé (total
  désormais 70 SLOs dans le registre canonique). Architecture
  détaillée 1 673 lignes (23 sections, 12 décisions, 10 risques,
  threat model 8 vecteurs).
* **v0.1.13** (Sprint D13, cette version) : observability
  dashboards (cf. § 15). Livre 6 dashboards Grafana JSON
  persona-driven (78 panneaux), 7 recording rules + 5
  downsampled, 10 alertes Prometheus runbook-linked, 5
  streams de logs JSON-Schema versionnés, 8 nouveaux
  instruments Prometheus côté Rust (`d13_instruments.rs`).
  4 templates Helm additifs (grafana-dashboards,
  prometheusrule-d13, promtail-config, helm test
  test-dashboards-load) tous gated derrière le bloc
  `observability.*` de `values.yaml` (default OFF —
  backward-compat single-tenant). 4 pages Diátaxis (howto/
  observability-setup, howto/cardinality-management, reference/
  dashboard-catalog, explanation/observability-architecture)
  + 10 runbooks `docs/runbooks/d13_*.md` ; CI rejects toute
  alerte sans `runbook_url`. 7 SLOs `D13-OBS-*` (5 latency +
  2 correctness) avec un nouveau sous-namespace
  `RESERVED_PREFIX` `obs::dash::`. Workflow
  `.github/workflows/bench-observability.yml` (5 jobs :
  dashboards-validate, promtool-check, helm-lint,
  python-tests, rust-telemetry-tests). ~ 70 nouveaux tests
  Python anti-régression + 8 nouveaux tests Rust unitaires.
  Aucun des 70 SLOs antérieurs n'a régressé (total désormais
  77 SLOs). Architecture détaillée 2 312 lignes (24 sections,
  12 décisions, 10 risques, HAZOP 10 vecteurs).
* **v0.2.0** (Sprint Mode 1 Server post-D1) : SLO RPC end-to-end
  (incluant la couche tonic + idempotency + auth + ratelimit) ;
  cible p99 sur les requêtes signées.
* **v1.0.0** (Phase 2 CUDA livrée) : SLO révisés × 5-15 sur
  `real_e2e_latency_*` selon spec.

Les baselines historiques sont conservées sous
`bench_baselines/v{X.Y.Z}/baseline.json` (versionnées dans le repo).
La baseline `v0.1.1/baseline.json` démarre vide (placeholder) et est
remplie par le premier run propre post-merge sur `main` — voir
`bench_baselines/README.md` pour la procédure de freeze.
