# Sprint D2 — Architecture : `benches/memory_real_arena.rs`

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Auteur** : équipe perf, sous direction architecture.
> **Position dans la séquence** : suit D1 (`real_strategies_e2e`,
> `real_memory`) qui mesure la production wiring complète. D2 descend
> d'un cran dans la pile pour mesurer la couche **infra mémoire**
> (`MemoryPool` + `Arena` + `HostBackend`) sur laquelle TOUT le reste
> repose.
> **Prérequis** : avoir lu
> [`d1_real_strategies_architecture.md`](d1_real_strategies_architecture.md)
> ; les conventions de bench (criterion 0.5 + async_tokio, fixtures
> v2, suffixe `_naive`/`_blas`, baseline freezing) sont communes.

---

## Table des matières

1. [Contexte & mission](#1-contexte--mission)
2. [Gap analysis : ce qui n'est PAS mesuré aujourd'hui](#2-gap-analysis--ce-qui-nest-pas-mesuré-aujourdhui)
3. [Surface API à bencher (cartographie chirurgicale)](#3-surface-api-à-bencher-cartographie-chirurgicale)
4. [Architecture cible : structure du fichier](#4-architecture-cible--structure-du-fichier)
5. [Les 7 groupes criterion détaillés](#5-les-7-groupes-criterion-détaillés)
6. [12 décisions de conception clés](#6-12-décisions-de-conception-clés)
7. [9 risques identifiés et mitigations](#7-9-risques-identifiés-et-mitigations)
8. [Plan jour-par-jour (3 j wallclock)](#8-plan-jour-par-jour-3-j-wallclock)
9. [Calibration & nouveaux SLO `D2-*`](#9-calibration--nouveaux-slo-d2)
10. [Anti-régression & realism check](#10-anti-régression--realism-check)
11. [Intégration CI + parser + baseline v0.1.2](#11-intégration-ci--parser--baseline-v012)
12. [Critères d'acceptation (DoD)](#12-critères-dacceptation-dod)
13. [Out of scope & roadmap follow-ups](#13-out-of-scope--roadmap-follow-ups)

---

## 1. Contexte & mission

### 1.1 Pourquoi descendre d'un étage après D1

D1 a mesuré la **chaîne complète** `Mode1Backend.execute(req)` —
genesis + N rounds de fusion + dissolution + alignement + decode. Le
résultat agrège *toutes* les couches en un seul nombre p95 par
fixture. C'est la métrique-roi du SLO produit, mais elle est **opaque
à l'attribution** : quand `real_e2e_latency_naive::large` régresse de
20 %, on ne sait pas si c'est :

* la fusion qui a ralenti (matmul, softmax),
* le consensus detector qui a changé de stratégie,
* l'alignement qui a refait sa softmax sur les ancres,
* **le pool d'arènes qui a régressé sur l'acquire ou l'alloc**,
* le scheduler tokio sous-jacent.

Sprint **D2** isole l'**infrastructure mémoire** — la couche la plus
basse, sous toutes les autres. Tout swarm, en MVP comme en GPU,
**commence par `pool.acquire()`** et **alloue tout son state via
`arena.alloc_zeroed()`** ; une régression de 5 µs sur l'acquire se
voit ×N requêtes/s en charge, et les chiffres D1 ne la diagnostiquent
pas.

### 1.2 Mission D2 (énoncé chirurgical)

> Produire un bench criterion **`crates/efe-core/benches/memory_real_arena.rs`**
> qui mesure, en isolation totale et sans mocks, les coûts unitaires
> et concurrents de :
>
> 1. `MemoryPool::acquire(min_bytes, deadline)` (async),
> 2. `MemoryPool::try_acquire(min_bytes)` (sync),
> 3. `Arena::alloc_zeroed::<T>(count)` pour T ∈ {f32, u8} et
>    count ∈ {1, 1k, 1M},
> 4. `Arena::alloc_zeroed_raw(bytes)` (chemin sans lifetime),
> 5. `Arena::reset()` (le rollback inter-rounds),
> 6. Le **pattern d'allocation réel d'un swarm Mvp** (latents +
>    masks + attention + confidence) sur 3 tailles fixture.
>
> Le bench produit des chiffres p95/p99 stables, calibre 6 nouveaux
> SLO `D2-*`, et est gardé par 4 tests anti-régression dans
> `tests/pool_realism_check.rs`.

### 1.3 Localisation décidée : `efe-core/benches/`

D1 vit dans `efe-server/benches/` parce qu'il importe
`efe_server::mode1::build_mvp_backend`. **D2 est différent** :

* `MemoryPool`, `Arena`, `HostBackend`, `PoolConfig` vivent tous
  dans `efe_core::memory`.
* Aucune dépendance `efe-server` n'est nécessaire.
* Garder D2 dans `efe-core/benches/` :
  * **réduit le rebuild graph** (un changement dans `efe-server`
    n'invalide pas D2),
  * **expose le bench à toute consommation downstream** d'`efe-core`
    (potentiellement une crate alternative non-server demain),
  * **co-localise les benches mémoire** avec leur cible (principe de
    proximité de Crockford).

---

## 2. Gap analysis : ce qui n'est PAS mesuré aujourd'hui

### 2.1 Inventaire des suites bench existantes (post-D1)

| Suite | Fichier | Crate | Ce qu'elle mesure réellement | Touche `MemoryPool` ? |
|-------|---------|-------|------------------------------|:---------------------:|
| `swarm_e2e` | `efe-core/benches/swarm_e2e.rs` | core | overhead orchestrateur via mocks | ❌ (mocks) |
| `memory_e2e` | `efe-core/benches/memory_e2e.rs` | core | heap peak via mocks | ❌ (mocks) |
| `compute_sprint{2,3,4}` | `efe-core/benches/compute_*.rs` | core | matmul, softmax, kernels | ❌ |
| `real_strategies_e2e` | `efe-server/benches/real_strategies_e2e.rs` | server | wiring production complet | ⚠️ transitivement |
| `real_memory` | `efe-server/benches/real_memory.rs` | server | heap peak production | ⚠️ transitivement |
| `pool_*` *(D2)* | **manquant** | — | — | ✅ |

### 2.2 Test unitaire ≠ bench

Le module `pool::tests` (cf. `crates/efe-core/src/memory/pool.rs:307-454`)
contient **12 tests unitaires** :

* `acquire_returns_valid_arena`,
* `preallocated_arenas_are_hits`,
* `fresh_alloc_when_pool_cold`,
* `release_returns_to_pool`,
* `saturation_blocks_until_release`,
* `deadline_triggers_budget_exhausted`,
* `try_acquire_fails_on_saturation`,
* `try_acquire_succeeds_when_free`,
* `soak_no_leak_after_many_cycles`,
* `large_request_gets_fresh_arena`,
* `concurrent_acquires_respect_max_arenas`,
* (+ corrolaires dans `arena::tests`).

**Aucun n'a de garantie de latence**. `soak_no_leak_after_many_cycles`
fait 1 000 itérations mais ne mesure pas le temps. Si une refonte du
free-list passe d'un `Mutex<Vec<Arena>>` à un `crossbeam::SegQueue` et
ralentit l'acquire de 200 ns à 5 µs, **le test passe** et **le SLO
tombe en silence en prod**.

### 2.3 Métrique observabilité ≠ bench

`PoolMetrics` (cf. `crates/efe-core/src/memory/metrics.rs`) expose
`acquire_count`, `acquire_wait_us_total`, `hit_count`, `miss_count`,
`oom_count`, `arenas_allocated_total`, `arenas_active`, `bytes_active`.
Ces compteurs sont **utiles en prod** (Prometheus) mais ne donnent
**aucune distribution** : on a la moyenne via `wait_us_total /
acquire_count` mais pas le p95, pas la queue P99.

### 2.4 Conclusion : gap de 100 %

L'infrastructure mémoire — qui est sur le **chemin chaud de chaque
requête** — n'est aujourd'hui **pas du tout benchée**. C'est la
définition même d'un trou de couverture critique.

---

## 3. Surface API à bencher (cartographie chirurgicale)

L'exploration du code révèle l'API publique exacte que D2 va exercer.
Cette section sert de référence canonique pour l'implémentation.

### 3.1 `MemoryPool` (`efe_core::memory::MemoryPool`)

```rust
impl MemoryPool {
    pub fn new(config: PoolConfig, backend: Arc<dyn DeviceBackend>) -> Result<Self>;
    pub async fn acquire(&self, min_bytes: u64, deadline: Option<Instant>) -> Result<ArenaHandle>;
    pub fn try_acquire(&self, min_bytes: u64) -> Result<ArenaHandle>;
    pub fn metrics(&self) -> PoolMetricsSnapshot;
    pub fn max_arenas(&self) -> usize;
    pub fn backend_name(&self) -> &'static str;
}
impl Clone for MemoryPool { /* Arc-cloned */ }
```

Points critiques :

* **`acquire` est async** : prend un `permit` sur
  `tokio::sync::Semaphore(max_arenas)` puis lock `Mutex<Vec<Arena>>`.
  Si `max_arenas` slots libres, le permit est immédiat ; sinon,
  attente sur le semaphore.
* **`try_acquire` est sync** : utilise `try_acquire_owned()` sur le
  semaphore — fail-fast sur saturation, pas d'attente.
* **`min_bytes`** filtre la free-list : on cherche la première
  arena avec `capacity_bytes() >= min_bytes`. Si aucune, on alloue
  fresh.
* **`deadline`** sur `acquire` async : si `Some(t)` et l'attente
  excède `t`, retourne `BudgetExhausted { kind: "wall_clock" }`.
* **`Clone`** : c'est juste un `Arc::clone(&inner)`.

### 3.2 `Arena` (`efe_core::memory::Arena`)

```rust
impl Arena {
    pub fn new(size_bytes: u64, backend: Arc<dyn DeviceBackend>, device_id: u32) -> Result<Self>;
    pub fn alloc_zeroed<T: DeviceCopy>(&self, count: usize) -> Result<DeviceSlice<'_, T>>;
    pub fn alloc_zeroed_raw(&self, size_bytes: u64) -> Result<RawAllocation>;
    pub fn reset(&mut self); // & mut !
    pub fn cursor_bytes(&self) -> u64;
    pub fn capacity_bytes(&self) -> u64;
    pub fn available_bytes(&self) -> u64;
    pub fn generation(&self) -> u32;
    pub fn device_id(&self) -> u32;
}
```

Invariants critiques pour le bench :

1. **`Arena::new` memset toute la région** (cf. `arena.rs:94`,
   `backend.memset_zero(base, aligned)`). Donc construire une arena
   1 GiB coûte ~300-500 ms (memset bandwidth bound, ~3 GB/s sur
   Apple Silicon). **Ce coût est sur le chemin du `cold acquire`.**

2. **`alloc_zeroed` memset chaque allocation** (cf. `arena.rs:184`,
   `self.backend.memset_zero(ptr, aligned)`). Le coût d'allocation
   = O(taille). Pour `alloc_zeroed::<f32>(1_000_000)` → memset 4 MB
   → ~1.3 ms. **Le bump-pointeur seul prend < 100 ns** (un
   `fetch_add` atomique) ; **le memset est le coût dominant**.

3. **`reset` ne memset PAS** (cf. `arena.rs:198-201`). Juste :
   ```rust
   self.cursor.store(0, Ordering::Release);
   self.generation.fetch_add(1, Ordering::AcqRel);
   ```
   → ~10 ns sur Apple M3. Le memset suivant viendra des `alloc_zeroed`
   appelés après reset.

4. **Allocation lock-free** : `cursor: AtomicU64` ; `fetch_add` →
   alignement → memset. Concurrent depuis 8 threads = 8 × bump
   parallèle, chaque memset sur une region disjointe.

5. **Alignement 256 octets** (`TENSOR_ALIGNMENT`). `alloc_zeroed::<u8>(1)`
   consomme **256 bytes** de cursor (! le bench le mesure pour
   exposer le coût d'alignement).

6. **`alloc_zeroed_raw(0)` retourne sans bumper le cursor** (cf.
   `arena.rs:147-155`). Bench-able comme « no-op baseline ».

### 3.3 `ArenaHandle` (`efe_core::memory::ArenaHandle`)

```rust
impl ArenaHandle {
    pub fn arena(&self) -> &Arena;
    pub fn arena_mut(&mut self) -> &mut Arena;
    pub fn into_arena(self) -> Arena; // détache du pool !
}
```

* **Drop** rend l'arène au pool via `MemoryPoolInner::return_arena`,
  qui re-push dans la `free_list` et libère le permit semaphore.
* `into_arena` casse le RAII : à n'utiliser **jamais dans le bench**
  (test path uniquement).

### 3.4 `HostBackend` (`efe_core::memory::HostBackend`)

```rust
impl HostBackend {
    pub fn new() -> Self;
    pub fn live_allocations(&self) -> usize;
}
```

* `live_allocations` est un compteur atomique exposé pour les tests.
  D2 l'utilise pour détecter une **fuite d'arène** dans la boucle
  bench (`assert_eq!(backend.live_allocations(), 0)` post-bench).

### 3.5 `PoolConfig` (`efe_core::memory::PoolConfig`)

```rust
pub struct PoolConfig {
    pub default_arena_size: u64,   // défaut: 1 << 30 = 1 GiB
    pub max_arenas: usize,         // défaut: 8
    pub prealloc_arenas: usize,    // défaut: 4
    pub device_id: u32,            // défaut: 0
    pub debug_leak_check: bool,
}
impl PoolConfig {
    pub fn with_max_arenas(self, n: usize) -> Self;
    pub fn with_arena_size(self, bytes: u64) -> Self;
    pub fn with_prealloc(self, n: usize) -> Self;
}
```

* **Bench tuning** : 1 GiB par défaut est trop pour un bench (cold
  acquire dominé par le memset). On utilise **64 KiB ou 1 MiB** —
  `MIN_ARENA_BYTES = 4 KiB` est le plancher hard.
* **Alignement requis** : `default_arena_size % 256 == 0`. 64 KiB et
  1 MiB sont multiples de 256 → OK.

### 3.6 Synthèse : les 4 latences fondamentales que D2 doit exposer

| Mesure | Composant interne | Ordre de grandeur attendu | Pourquoi ça compte |
|--------|-------------------|---------------------------:|--------------------|
| **acquire warm** | semaphore.try + mutex.lock + Vec::swap_remove | **≤ 1 µs** | sur chaque `execute()` request |
| **acquire cold** | + Arena::new + memset region entière | **≤ 1 ms** (à 64 KiB) | au démarrage du pool, après éviction |
| **alloc_zeroed::<f32>(N)** | fetch_add + align_up + memset(N×4) | **N × ~3 ns** (memset BW) | sur chaque round de fusion |
| **reset** | atomic store + atomic add | **≤ 50 ns** | entre deux requêtes |

Tout écart > 2× sur l'une de ces 4 mesures = signal de régression
sérieux à diagnostiquer avant merge.

---

## 4. Architecture cible : structure du fichier

### 4.1 Localisation et déclaration Cargo

**Fichier** : `crates/efe-core/benches/memory_real_arena.rs`
**LOC cible** : ~500 lignes (commentaires denses inclus)
**Cargo.toml** :

```toml
[[bench]]
name = "memory_real_arena"
harness = false
```

Pas de nouvelle feature à introduire — `criterion`, `tokio` et
`peak_alloc` (non utilisé ici, voir §6 D6) sont déjà dans les
`dev-dependencies` de `efe-core` depuis Sprint C.

### 4.2 Structure du fichier (squelette)

```rust
//! D2 — bench infrastructure mémoire (MemoryPool + Arena réels).

#![allow(missing_docs)]

mod common; // partagé avec swarm_e2e/memory_e2e (existant)
            // → expose make_runtime() et load_realistic_spec()

use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};
use efe_core::memory::{
    Arena, ArenaHandle, DeviceBackend, HostBackend, MemoryPool, PoolConfig,
};

// =====================================================================
// Constantes bench-tuning
// =====================================================================
const BENCH_ARENA_SIZE: u64 = 1 << 20;     // 1 MiB (vs 1 GiB prod)
const BENCH_DEFAULT_PREALLOC: usize = 4;
const BENCH_DEFAULT_MAX: usize = 8;

// =====================================================================
// Helpers internes
// =====================================================================
fn make_pool(prealloc: usize, max: usize, arena_size: u64) -> Arc<MemoryPool>;
fn make_pool_default() -> Arc<MemoryPool>;
fn make_runtime() -> tokio::runtime::Runtime;

// =====================================================================
// Groupes criterion
// =====================================================================
fn bench_pool_acquire_warm(c: &mut Criterion);
fn bench_pool_acquire_cold(c: &mut Criterion);
fn bench_pool_acquire_saturated(c: &mut Criterion);
fn bench_pool_concurrent_acquire(c: &mut Criterion);
fn bench_arena_alloc_zeroed_typed(c: &mut Criterion);
fn bench_arena_reset(c: &mut Criterion);
fn bench_realistic_swarm_alloc_pattern(c: &mut Criterion);

criterion_group!(
    name = memory_real_arena_benches;
    config = Criterion::default();
    targets =
        bench_pool_acquire_warm,
        bench_pool_acquire_cold,
        bench_pool_acquire_saturated,
        bench_pool_concurrent_acquire,
        bench_arena_alloc_zeroed_typed,
        bench_arena_reset,
        bench_realistic_swarm_alloc_pattern,
);
criterion_main!(memory_real_arena_benches);
```

### 4.3 Helpers détaillés

```rust
/// Construit un pool bench-tuné. Réutilisable par tous les groupes.
///
/// `arena_size` doit être >= MIN_ARENA_BYTES (4 KiB) et multiple de
/// TENSOR_ALIGNMENT (256). Les défauts (BENCH_DEFAULT_*) garantissent
/// les deux.
fn make_pool(prealloc: usize, max: usize, arena_size: u64) -> Arc<MemoryPool> {
    let cfg = PoolConfig::default()
        .with_prealloc(prealloc)
        .with_max_arenas(max)
        .with_arena_size(arena_size);
    let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
    Arc::new(MemoryPool::new(cfg, backend).expect("memory pool"))
}

/// Pool par défaut : 4 prealloc / 8 max / 1 MiB par arène.
fn make_pool_default() -> Arc<MemoryPool> {
    make_pool(BENCH_DEFAULT_PREALLOC, BENCH_DEFAULT_MAX, BENCH_ARENA_SIZE)
}

/// Runtime tokio multi-threaded de 4 workers (matche le runtime D1).
fn make_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("tokio runtime")
}
```

---

## 5. Les 7 groupes criterion détaillés

Chaque groupe est documenté avec : *intention*, *setup*, *itération*,
*sample size*, *coût attendu*, *risques de mesure*. La somme des
groupes = ~500 LOC commentaires inclus.

### 5.1 `pool_acquire_warm` — chemin chaud nominal

**Intention** : mesurer le coût d'`acquire(small_min_bytes, None)`
quand la free-list a une arène disponible. C'est le 99 % des
acquires en prod (steady-state).

**Setup** :
```rust
let pool = make_pool_default(); // prealloc=4, max=8 → 4 free.
let rt = make_runtime();
```

**Itération** :
```rust
b.to_async(&rt).iter(|| {
    let pool = pool.clone();
    async move {
        let h = pool.acquire(1024, None).await.expect("warm acquire");
        black_box(&h);
        // drop(h) → return to pool. Implicit at scope end.
    }
});
```

**Sample size** : 200 (mesure attendue ~ sub-microseconde,
faible variance, beaucoup d'échantillons → CI tight).

**Coût décomposé attendu** :

| Étape | Coût attendu (M3 Pro) |
|-------|----------------------:|
| `metrics.acquire_count.fetch_add` | 5 ns |
| `Semaphore::acquire_owned().await` (slot libre) | 100 ns |
| `Mutex::lock` (free_list) | 30 ns |
| `Vec::iter().position` (taille 1-4) | 20 ns |
| `Vec::swap_remove` | 5 ns |
| `metrics.hit_count.fetch_add` | 5 ns |
| 2 × `metrics.{arenas,bytes}_active.fetch_add` | 10 ns |
| `ArenaHandle::new` (struct construction) | 5 ns |
| `ArenaHandle::drop` (return + semaphore release) | 100 ns |
| **Total** | **≈ 280 ns** |

**Cible SLO** : `p95 ≤ 1 µs` (laisse 3.5× de marge).

**Risque de mesure** : la latence est si faible que la **variance
de scheduler tokio** domine. Atténuation : `worker_threads=4` fixé,
`current_thread` runtime testé en variante § 5.4.

### 5.2 `pool_acquire_cold` — premier acquire après pool vide

**Intention** : mesurer le coût quand aucune arène n'est dispo dans
la free-list **mais le slot semaphore est libre** (donc on n'attend
pas, on alloue fresh). Domine au démarrage et après éviction massive.

**Setup** :
```rust
// Pool *vide* en prealloc, mais 4 slots dispos.
let pool = make_pool(0, 4, BENCH_ARENA_SIZE);
```

**Itération** :
```rust
b.to_async(&rt).iter(|| {
    let pool = pool.clone();
    async move {
        let h = pool.acquire(1024, None).await.expect("cold acquire");
        black_box(&h);
        // drop → arène retourne à la free-list.
        // *L'itération suivante sera donc "warm"* — voir IMPORTANT ci-dessous.
    }
});
```

**⚠ IMPORTANT — pourquoi `prealloc=0, max=4` est plus subtil
qu'il n'y paraît** :

* Itération 0 : free-list vide → fresh alloc (vraiment cold).
* Itération 1 : l'arène vient d'être rendue → **warm**.
* Itération 2..N : warm.

Pour mesurer **uniquement le cold path** on doit forcer la
ré-allocation à chaque itération. Deux options :

* **Option A (rejetée)** : `pool.into_arena_handle()` détache l'arène
  du pool → la free-list reste vide → cold à chaque iter. Mais cela
  **fuit la mémoire** (l'arène n'est jamais drop) et casse le bench
  à ~max_arenas itérations.
* **Option B (retenue)** : utiliser `iter_batched` avec un setup qui
  recrée le pool à chaque batch. La granularité reste petite et le
  coût de `MemoryPool::new(0)` (pas de prealloc) est négligeable
  devant un cold acquire de 1 MiB.

Implémentation Option B :

```rust
b.to_async(&rt).iter_batched(
    || make_pool(0, 1, BENCH_ARENA_SIZE), // setup: pool 0/1
    |pool| async move {
        let h = pool.acquire(1024, None).await.expect("cold");
        black_box(&h);
    },
    BatchSize::SmallInput,
);
```

**Sample size** : 50. Le cold acquire mesure essentiellement le
memset 1 MiB (~300 µs sur M3) ; on saturate vite.

**Coût décomposé attendu** :

| Étape | Coût attendu (M3 Pro, 1 MiB arena) |
|-------|-----------------------------------:|
| Semaphore acquire (slot libre) | 100 ns |
| Mutex + Vec iter (vide) | 50 ns |
| `Arena::new(1 MiB)` : `backend.allocate` | ~5 µs (mmap+lazy) |
| `Arena::new` : `backend.memset_zero(1 MiB)` | ~330 µs (3 GB/s BW) |
| `metrics.miss_count + arenas_allocated_total` | 10 ns |
| `ArenaHandle::new + drop` | 100 ns |
| **Total** | **≈ 335 µs** (1 MiB) |

**Cible SLO** : `p95 ≤ 1 ms` (3× la baseline attendue).

**Note d'interprétation** : ce nombre est *amorti* en prod par le
`prealloc_arenas`. Le coût compte *uniquement* lors d'un burst qui
dépasse les pré-allocations. La métrique sert à anticiper le pire
cas et à détecter une régression de `Arena::new` (si demain on
ajoute une logique anti-page-fault touchant chaque page).

### 5.3 `pool_acquire_saturated` — coût d'attente sur saturation

**Intention** : mesurer le temps pour acquérir une arène quand
**toutes les arènes sont sorties** et qu'on doit attendre qu'un
holder en libère une.

**Difficulté** : l'attente est *par construction* non-déterministe —
elle dépend du sleep que le holder fait avant de drop. Le bench doit
donc mesurer **l'overhead pur du chemin saturé** (semaphore wait +
notify), pas la durée du sleep elle-même.

**Setup** :
```rust
// Pool 1/1 : 1 prealloc, 1 max. Toujours saturé après 1 acquire.
let pool = make_pool(1, 1, BENCH_ARENA_SIZE);
```

**Itération (Option B retenue)** :

```rust
b.to_async(&rt).iter_custom(|iters| {
    let pool = pool.clone();
    async move {
        let mut total = Duration::ZERO;
        for _ in 0..iters {
            // 1. Acquire et hold.
            let h1 = pool.acquire(1024, None).await.expect("h1");
            // 2. Lance une task qui drop h1 après 1 ms.
            let pool2 = pool.clone();
            let release_after = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(1)).await;
                drop(h1);
                // h1 droppé ici uniquement — pool2 est juste pour partage.
                let _ = pool2;
            });
            // 3. Bench la 2e acquire (qui doit attendre le drop).
            let start = Instant::now();
            let h2 = pool.acquire(1024, None).await.expect("h2");
            total += start.elapsed();
            release_after.await.expect("join");
            drop(h2);
        }
        total
    }
});
```

**⚠ Problème** : la mesure inclut le `1 ms sleep`. On mesure ~1 ms par
itération, dominé par le sleep. **Solution** : retrancher le sleep en
post-traitement, OU utiliser une variante où le holder drop *avant*
de start le timer. La 2e variante est plus fiable :

```rust
// Variante "overhead-only": measure from semaphore wait to acquire success
// when a release has already happened (no sleep involved).
b.to_async(&rt).iter(|| {
    let pool = pool.clone();
    async move {
        let h = pool.acquire(1024, None).await.expect("h");
        // Pas d'attente : 1 prealloc, 1 max. h est le seul holder.
        // Mais on a quand même contesté le semaphore !
        black_box(&h);
    }
});
```

C'est ESSENTIELLEMENT identique à `acquire_warm` avec un pool de
taille 1 — pas vraiment "saturated".

**Décision finale (compromis pragmatique)** : on mesure deux variantes :

* `pool_acquire_saturated::wait_short` : holder release après
  100 µs ; on mesure le *total* incluant l'attente. Indicateur du
  *coût brut d'attente* sous saturation modérée.
* `pool_acquire_saturated::contention_only` : pool 2 max, 4 tasks
  qui se concurrencent pour 2 slots ; on mesure le throughput. Plus
  réaliste de la prod sous charge.

Implémentation `contention_only` (proche de § 5.4) :

```rust
b.to_async(&rt).iter(|| {
    let pool = pool.clone(); // pool 2 max, 0 prealloc
    async move {
        let tasks = (0..4).map(|_| {
            let p = pool.clone();
            tokio::spawn(async move {
                let h = p.acquire(1024, None).await.expect("a");
                black_box(&h);
            })
        }).collect::<Vec<_>>();
        for t in tasks {
            t.await.expect("join");
        }
    }
});
```

**Sample size** : 30 (variance haute due au scheduler).

**Cible SLO** : `p95 ≤ 200 µs` pour `contention_only` (4 tasks × 50 µs
sur 2 slots, lock contention modérée). N/A pour `wait_short` (mesure
d'orientation, pas un SLO bloquant).

### 5.4 `pool_concurrent_acquire` — scaling sous charge

**Intention** : mesurer le **throughput** du pool quand N tasks tokio
acquerraient/libèreraient des arènes en parallèle. Détecte la
contention sur le `Mutex<Vec<Arena>>`.

**Setup** :
```rust
let pool = make_pool(8, 8, BENCH_ARENA_SIZE); // 8 dispo dès le départ
```

**Itération** :
```rust
const N_CONCURRENT: usize = 8;
group.throughput(Throughput::Elements(N_CONCURRENT as u64));
b.to_async(&rt).iter(|| {
    let pool = pool.clone();
    async move {
        let mut handles = Vec::with_capacity(N_CONCURRENT);
        for _ in 0..N_CONCURRENT {
            let p = pool.clone();
            handles.push(tokio::spawn(async move {
                let h = p.acquire(1024, None).await.expect("a");
                black_box(&h);
            }));
        }
        for h in handles {
            h.await.expect("join");
        }
    }
});
```

**Sample size** : 50.

**Coût attendu** : N×N_CONCURRENT × (acquire_warm + tokio::spawn
overhead). À 8 concurrent, ~ 280 ns × 8 + 1 µs (spawn) × 8 = ~10.5 µs.

**Cible SLO** : `p95 ≤ 50 µs` pour le batch entier (= ~6 µs/op).

**Note** : ce bench réplique en partie `pool_acquire_warm` ; sa
valeur ajoutée est de **détecter une contention non-linéaire** sur
le mutex (free_list lock). Si on remplace `Mutex` par une lock-free
queue dans le futur, ce bench devrait montrer une accélération
super-linéaire.

### 5.5 `arena_alloc_zeroed_typed` — coût d'allocation typée

**Intention** : mesurer `Arena::alloc_zeroed::<T>(count)` pour T en
{f32, u8, u32} et `count` en {1, 1024, 1_000_000}. C'est le coût
*par allocation* sur le chemin chaud du fusion kernel.

**Important — le memset domine** : pour `count=1_000_000` sur f32,
on memset 4 MB. À 3 GB/s c'est 1.3 ms. Le bump pointeur seul est
< 50 ns. **Ce bench mesure essentiellement la bande passante memset**
du backend, modulo l'overhead `fetch_add` + `align_up`.

**Setup** :
```rust
// Une grosse arène (16 MiB) pour pouvoir alloc 1M f32 sans réinit.
let arena_size = 16 * (1 << 20);
let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
let mut arena = Arena::new(arena_size, backend, 0).expect("arena");
```

**Itération avec `iter_batched_ref`** (l'arène doit être reset entre
batches pour éviter HbmAllocFailed) :

```rust
group.bench_with_input(BenchmarkId::new("f32", count), &count, |b, &n| {
    b.iter_batched_ref(
        || {
            arena.reset();
            ()
        },
        |_| {
            let s = arena.alloc_zeroed::<f32>(n).expect("alloc");
            black_box(s);
        },
        BatchSize::SmallInput,
    );
});
```

**Subtilité** : `arena.reset()` requiert `&mut self`. Si on capture
`arena` dans la closure, on ne peut pas avoir 2 refs mutables. On
doit donc soit :
* utiliser `Arc<Mutex<Arena>>` (lock à chaque iter, contamine la
  mesure), soit
* recréer une arène fresh par batch (coût de creation domine), soit
* utiliser `iter_batched_ref` avec une closure qui capture arena
  par `&mut` (Rust accepte si la closure n'est appelée qu'une fois
  par fonction `bench_with_input`).

**Solution choisie** : recréer une arène **par groupe** (pas par
itération) en utilisant un seul `bench_with_input` qui possède son
arène :

```rust
fn bench_arena_alloc_zeroed_typed(c: &mut Criterion) {
    let mut group = c.benchmark_group("arena_alloc_zeroed_typed");
    group.throughput(Throughput::Elements(1));
    let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());

    for &count in &[1usize, 1024, 1_000_000] {
        // Arène dédiée à ce sous-bench, dimensionnée juste pour `count`.
        let arena_size = (count * std::mem::size_of::<f32>())
            .next_multiple_of(256)
            .max(MIN_ARENA_BYTES as usize) as u64
            * 2; // marge ×2 pour les batches
        let mut arena = Arena::new(arena_size, backend.clone(), 0)
            .expect("arena");

        group.bench_with_input(
            BenchmarkId::new("f32", count), &count,
            |b, &n| {
                b.iter_custom(|iters| {
                    let mut total = Duration::ZERO;
                    for _ in 0..iters {
                        // Reset *avant* la mesure.
                        arena.reset();
                        let start = Instant::now();
                        let s = arena.alloc_zeroed::<f32>(n).expect("alloc");
                        total += start.elapsed();
                        black_box(s);
                    }
                    total
                });
            },
        );
    }
    group.finish();
}
```

**Sample size** : 100 (count=1) ; 30 (count=1M, ms-scale).

**Cibles SLO** :

| count | T | bytes | cible p95 |
|------:|---|------:|----------:|
| 1 | f32 | 256 (aligné) | ≤ 200 ns |
| 1k | f32 | 4 096 | ≤ 2 µs |
| 1M | f32 | 4 MB | ≤ 2 ms |

### 5.6 `arena_reset` — coût de remise à zéro

**Intention** : mesurer le coût de `Arena::reset(&mut self)`. Comme
on l'a noté en §3.2, c'est juste `cursor = 0` + `gen += 1`. **Ce
bench documente la borne inférieure** ; un jour future si on ajoute
un memset-on-reset (debug feature, ou pour zéroïser les secrets), ce
bench montrera le surcoût.

**Setup** :
```rust
let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
let mut arena = Arena::new(BENCH_ARENA_SIZE, backend, 0).expect("arena");
// Faire quelques allocations pour avoir un cursor non-trivial.
let _ = arena.alloc_zeroed::<u8>(4096).expect("warm");
```

**Itération** :
```rust
b.iter(|| {
    arena.reset();
    black_box(&arena);
});
```

**Sample size** : 1000 (mesure ns-scale, beaucoup d'échantillons).

**Cible SLO** : `p95 ≤ 50 ns`.

### 5.7 `realistic_swarm_alloc_pattern` — pattern Mvp réel

**Intention** : reproduire la séquence d'allocations qu'un swarm Mvp
fait pendant un round de fusion, **dans l'ordre exact, avec les
tailles réelles**. C'est le bench le plus représentatif du coût
mémoire qu'une requête paie effectivement.

**Pattern réel d'un round de fusion Mvp** (extrait de
`MvpCpuFusion::run_round`) :

| Allocation | Type | Taille | Justification |
|-----------|------|-------:|---------------|
| `latents` | `f32` | `n_agents × dim_latent` | état épistémique |
| `mask` | `u8` | `n_agents` | actif/inactif |
| `attention_weights` | `f32` | `n_agents²` | similarités pairwise |
| `confidence` | `f32` | `n_agents` | poids des messages |
| (scratch) | `f32` | `n_agents × dim_latent` | softmax, drift, etc. |

3 tailles fixture (matchées sur les v2 de D1) :

| name | dim | n_agents | latents bytes | attention bytes | total bytes |
|------|----:|---------:|--------------:|----------------:|------------:|
| small | 128 | 8 | 4 096 | 256 | ~12 KiB |
| medium | 256 | 32 | 32 768 | 4 096 | ~70 KiB |
| large | 256 | 128 | 131 072 | 65 536 | ~330 KiB |

**Setup** : un pool 4/8 d'arènes 1 MiB suffit.

**Itération (par taille)** :
```rust
for (name, dim, n_agents) in fixtures {
    let pool = make_pool_default();
    let scratch_factor = 2; // double-buffer pour scratch
    group.bench_with_input(
        BenchmarkId::from_parameter(name), &(dim, n_agents),
        |b, &(dim, n_agents)| {
            b.to_async(&rt).iter(|| {
                let pool = pool.clone();
                async move {
                    let handle = pool.acquire(1024, None).await.expect("acq");
                    let arena = handle.arena();
                    // 1. latents
                    let latents = arena.alloc_zeroed::<f32>(n_agents * dim).expect("l");
                    // 2. mask
                    let mask = arena.alloc_zeroed::<u8>(n_agents).expect("m");
                    // 3. attention
                    let att = arena.alloc_zeroed::<f32>(n_agents * n_agents).expect("a");
                    // 4. confidence
                    let conf = arena.alloc_zeroed::<f32>(n_agents).expect("c");
                    // 5. scratch
                    let sc = arena.alloc_zeroed::<f32>(n_agents * dim).expect("s");
                    black_box((latents, mask, att, conf, sc));
                    // drop(handle) → return.
                }
            });
        },
    );
}
```

**Sample size** : 100 (small) / 50 (medium, large).

**Cibles SLO** :

| fixture | total bytes | cible p95 |
|---------|-------------|----------:|
| small | ~12 KiB | ≤ 5 µs |
| medium | ~70 KiB | ≤ 30 µs |
| large | ~330 KiB | ≤ 150 µs |

**Note d'interprétation** : ce bench combine
*pool_acquire_warm + 5×alloc_zeroed*. Les chiffres SHOULD être
≈ `1 µs (acquire) + 5 × alloc_zeroed`. Si on voit un excès de
30 % par rapport à cette somme, c'est un signal d'un overhead
de séquencement (alignement multiple, page faults sur l'arène).

---

## 6. 12 décisions de conception clés

| #  | Décision | Alternative rejetée | Rationale |
|----|----------|---------------------|-----------|
| D1 | `efe-core/benches/` (vs `efe-server/`) | server-side avec import sym | la cible (`MemoryPool`) est dans core ; aucune dépendance server ; rebuild graph plus serré |
| D2 | `BENCH_ARENA_SIZE = 1 MiB` (vs prod 1 GiB) | 1 GiB partout | cold acquire dominé par memset 1 GiB ≈ 300 ms → bench impraticable ; 1 MiB capture la même structure de coût avec mesures stables |
| D3 | Tokio runtime `multi_thread, 4 workers` (vs `current_thread`) | single-thread | matche D1 ; expose la concurrence du semaphore |
| D4 | `iter_batched(Pool::new, ...)` pour cold path | `into_arena_handle` détaché | évite la fuite et garde la mesure honnête |
| D5 | Bench inclut `try_acquire` ? | non | redondance forte avec `acquire warm` ; ajout différé en D2.5 si valeur prouvée |
| D6 | Pas de `peak_alloc` ici | global allocator partout | timings doivent rester pristine ; heap profiling = D2.5 séparé (`memory_real_arena_peak.rs`) |
| D7 | Pas de cfg `_blas`/`_naive` ici | mirroir de D1 | aucune kernel BLAS dans ce bench ; le suffixe ajoute du bruit visuel sans valeur |
| D8 | `alloc_zeroed_raw` non benché | bench séparé | `alloc_zeroed` couvre 99 % des call sites ; raw est pour les NanoAgentBatch typés en runtime |
| D9 | Sample sizes adaptés par groupe (50/100/200/1000) | uniforme à 50 | mesures ns-scale demandent plus de samples pour CI tight |
| D10 | Pas de fixture v2 chargée | réutiliser realistic_*.json | bench infrastructure = pas de spec utilisateur ; les paramètres (dim, n_agents) sont des constantes choisies pour matcher les 3 tailles |
| D11 | `Arc<MemoryPool>` partout | `&MemoryPool` borrowed | tokio::spawn nécessite `'static` ; `MemoryPool: Clone` (Arc-cloned) coûte ≈ 0 |
| D12 | 3 tailles fixture (small/medium/large) | 5 ou plus | aligne sur D1 et SLO.md ; matrice plus large → time-budget CI explose |

---

## 7. 9 risques identifiés et mitigations

### R1 — Bench dominé par le memset, pas par les structures qu'on prétend mesurer

**Impact** : `arena_alloc_zeroed_typed` à `count=1M` mesure
essentiellement la bande passante memset (~3 GB/s sur Apple
Silicon). Une régression de la free-list lock de 100 ns serait
**invisible** dans ce sous-bench.

**Mitigation** :
* Le sous-bench `count=1` isole l'overhead structurel (cursor
  fetch_add + align_up + memset 256 octets minimum).
* Les nombres `count=1` et `count=1M` doivent être lus comme **deux
  régimes distincts** — l'un mesure la structure, l'autre la BW
  hardware. La doc BENCHMARKS.md le formalise.

### R2 — Variance scheduler tokio

**Impact** : `pool_acquire_warm` à 280 ns nominal a une variance
absolue dominée par le scheduler tokio (~5-50 µs spike sporadique
sur park/unpark de worker thread).

**Mitigation** :
* `worker_threads=4` fixé.
* `criterion.noise_threshold(0.10)` (10 % par défaut) — peut être
  bumpé à 0.15 si la variance CI le justifie.
* Sample size 200 → CI tight via la loi des grands nombres.
* En dernier ressort : variant `current_thread` pour comparaison
  hors-band (non SLO).

### R3 — Empreinte mémoire du bench

**Impact** : 4 prealloc × 1 MiB = 4 MiB par invocation de pool. Avec
les 7 groupes qui créent chacun 1+ pools, on tape ~50 MiB peak. CI
runners ubuntu-latest ont 7 GiB RAM → **OK** mais à surveiller si on
augmente `BENCH_ARENA_SIZE` futurement.

**Mitigation** :
* Documenter `peak_rss ≈ 50 MiB attendu` dans BENCHMARKS.md.
* Si on passe à 64 MiB par arena (pour stresser memset BW), prévoir
  un `cfg(not(debug_assertions))` qui désactive le bench sur runners
  contraints.

### R4 — `Arena::reset` requiert `&mut self`

**Impact** : on ne peut pas avoir 2 références mutables à la même
arène, donc `iter_batched_ref` avec une closure capture par
`&mut arena` est subtil.

**Mitigation** :
* Pattern `iter_custom(|iters| async move { for ... { arena.reset(); ... } })`
  qui sérialise les itérations dans une seule closure et possède
  l'arène localement.
* Test smoke en `cfg(test) mod` : `assert_eq!(arena.generation(), n)`
  après `n` resets.

### R5 — Ordre des allocations matters dans `realistic_swarm_alloc_pattern`

**Impact** : bumper le cursor dans un ordre `latents → mask → attention`
vs `attention → latents → mask` produit des **alignement gaps
différents** (attention en f32 carré peut être large, mask en u8 est
pade beaucoup avec align 256). Le bench doit refléter l'**ordre
réel** du fusion kernel pour être prédictif.

**Mitigation** :
* Ordre figé sur `MvpCpuFusion::run_round` (lecture du code).
* Test anti-régression dans `tests/pool_realism_check.rs` : on
  vérifie que `arena.cursor_bytes()` post-séquence matche une valeur
  attendue calculée via `align_up(...)` à la main.

### R6 — Fuite d'arène silencieuse

**Impact** : si une closure bench oublie de drop le handle (capture
'static dans tokio::spawn sans await + drop), une arène fuit
définitivement et le pool finit par échouer en `BudgetExhausted`
après `max_arenas` itérations.

**Mitigation** :
* Tous les benches utilisent `drop(handle)` explicite ou laisse le
  scope finir (RAII).
* Test smoke : avant et après chaque groupe, on assert
  `pool.metrics().arenas_active == 0`.
* `HostBackend::live_allocations()` checké en post-bench dans
  `tests/pool_realism_check.rs`.

### R7 — `Mutex<Vec<Arena>>` poison sur panic

**Impact** : si une closure bench panic alors qu'elle tient le
mutex (impossible directement, car il est interne au pool, mais via
un drop hook custom dans le futur), tous les acquires suivants
panic sur `expect("free_list mutex poisoned")`.

**Mitigation** :
* Aucune closure bench n'accède directement au mutex (encapsulé
  dans le pool).
* Si jamais un cas survient en CI, criterion s'arrête net et on a
  un stack trace clair → pas de mesure muette.

### R8 — Premier call à `tokio::time::sleep` allume une horloge globale

**Impact** : le tout premier `sleep` invoqué dans un test tokio
allume le timer driver, ce qui peut prendre ~100 µs et contaminer
la première itération si le bench start avant le warmup criterion.

**Mitigation** :
* Pas de `sleep` dans les benches D2 (saturated::contention_only
  s'en passe via barriers).
* Si jamais re-introduit en D2.5, ajouter un `sleep(Duration::ZERO)
  warmup` dans le setup.

### R9 — Calibration host-dépendante

**Impact** : 1 µs sur Apple M3 Pro ≈ 1.5 µs sur ubuntu-latest
(skylake-server 2.6 GHz). Les SLO doivent appliquer le coefficient
CI 1.5.

**Mitigation** : les nouveaux SLO `D2-*` héritent de la même logique
`CI_COEFFICIENT` que D1 dans `bench.yml` ; aucun travail
supplémentaire requis.

---

## 8. Plan jour-par-jour (3 j wallclock)

### D2.1 (½ j) — Helpers `common/mod.rs` étendu

* Ajouter `make_runtime()` (déjà présent en partie) et un nouveau
  `make_pool(prealloc, max, arena_size) -> Arc<MemoryPool>` au
  module `common/mod.rs` partagé entre `swarm_e2e`, `memory_e2e`,
  et le nouveau `memory_real_arena`.
* Tests de fumée smoke dans `cfg(test) mod`.

**DoD** :
- [ ] `make_pool(2, 4, 65536)` retourne un pool valide.
- [ ] `make_runtime()` produit un runtime multi-threaded 4 workers.

### D2.2 (½ j) — Squelette `memory_real_arena.rs` + 1er groupe

* Créer le fichier avec les imports, constantes, et `bench_pool_acquire_warm`.
* Faire passer `cargo build -p efe-core --benches`.
* Faire passer `cargo bench -p efe-core --bench memory_real_arena -- --quick`.

**DoD** :
- [ ] `cargo bench --bench memory_real_arena` produit 1 mesure
      stable sur `pool_acquire_warm`.

### D2.3 (1 j) — Groupes pool (cold, saturated, concurrent)

* Implémenter `bench_pool_acquire_cold` (Option B, `iter_batched`).
* Implémenter `bench_pool_acquire_saturated` (variante
  `contention_only`).
* Implémenter `bench_pool_concurrent_acquire`.
* Affiner les sample sizes en fonction des distributions observées.

**DoD** :
- [ ] 4 groupes pool produisent des mesures dans l'ordre de grandeur
      attendu (cold ~ 1000× warm).
- [ ] Pas de fuite : `pool.metrics().arenas_active == 0` post-bench.

### D2.4 (½ j) — Groupes arène (alloc, reset)

* `bench_arena_alloc_zeroed_typed` avec 3 tailles count.
* `bench_arena_reset`.

**DoD** :
- [ ] `alloc_zeroed::<f32>(1)` < 200 ns.
- [ ] `reset` < 50 ns.

### D2.5 (½ j) — Pattern réel + tests anti-régression + docs

* `bench_realistic_swarm_alloc_pattern` × 3 fixtures.
* `tests/pool_realism_check.rs` : 4 tests (voir § 10).
* Mise à jour `BENCHMARKS.md` § 4 / nouveau § 9.
* Mise à jour `parse_criterion_report.py::WANTED_PREFIXES` +
  nouveaux SLO `D2-*` dans `SLO_DEFINITIONS`.
* Mise à jour `bench_runner.sh` (ajout de `cargo bench --bench memory_real_arena`).
* `bench_baselines/v0.1.2/baseline.json` (placeholder).

**DoD** :
- [ ] `cargo test -p efe-core --test pool_realism_check` 4/4 pass.
- [ ] `cargo clippy -p efe-core --benches -- -D warnings` clean.
- [ ] BENCHMARKS.md a une section dédiée § 4 « Memory pool / arena
      (D2) ».
- [ ] `parse_criterion_report.py` reconnaît les groupes D2.
- [ ] CI bench.yml run-passe (path filter inclut `memory/`).

---

## 9. Calibration & nouveaux SLO `D2-*`

Référence host : Apple M3 Pro, 11 cores, 36 GB, macOS Darwin 24,
rustc 1.85.0, `BENCH_ARENA_SIZE = 1 MiB`, no-BLAS.

| ID | Nom | Bench key | Cible p95 (M3 Pro) |
|----|-----|-----------|-------------------:|
| `SLO-D2-1` | Pool acquire warm | `pool_acquire_warm::default` | ≤ **1 µs** |
| `SLO-D2-2` | Pool acquire cold | `pool_acquire_cold::1MiB` | ≤ **1 ms** (amorti) |
| `SLO-D2-3` | Pool concurrent (8c) | `pool_concurrent_acquire::c8` | ≤ **50 µs** (batch entier) |
| `SLO-D2-4` | Arena alloc f32(1) | `arena_alloc_zeroed_typed::f32_1` | ≤ **200 ns** |
| `SLO-D2-5` | Arena alloc f32(1k) | `arena_alloc_zeroed_typed::f32_1024` | ≤ **2 µs** |
| `SLO-D2-6` | Arena alloc f32(1M) | `arena_alloc_zeroed_typed::f32_1000000` | ≤ **2 ms** |
| `SLO-D2-7` | Arena reset | `arena_reset::default` | ≤ **50 ns** |
| `SLO-D2-8` | Swarm alloc small | `realistic_swarm_alloc_pattern::small` | ≤ **5 µs** |
| `SLO-D2-9` | Swarm alloc medium | `realistic_swarm_alloc_pattern::medium` | ≤ **30 µs** |
| `SLO-D2-10` | Swarm alloc large | `realistic_swarm_alloc_pattern::large` | ≤ **150 µs** |

**Coefficient CI** : ×1.5 appliqué automatiquement par `bench.yml` —
identique à D1.

**Justification des cibles** : chaque cible = 3× la mesure attendue
(décomposée en § 5). Marge ×3 absorbe le bruit scheduler + variance
hardware ; au-delà de 3×, on est dans une régression structurelle
qui mérite l'investigation.

---

## 10. Anti-régression & realism check

**Fichier** : `crates/efe-core/tests/pool_realism_check.rs` (nouveau)
**Pattern** : miroir du `tests/bench_realism_check.rs` côté
efe-server.

### Test 1 — `pool_acquire_does_not_leak`

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pool_acquire_does_not_leak() {
    let pool = make_pool(2, 4, 64 * 1024);
    let backend = pool.metrics(); // snapshot before
    for _ in 0..1000 {
        let h = pool.acquire(1024, None).await.expect("a");
        drop(h);
    }
    let after = pool.metrics();
    assert_eq!(after.arenas_active, 0);
    assert_eq!(after.bytes_active, 0);
    // Pas plus d'arenas allouées que max_arenas.
    assert!(after.arenas_allocated_total <= 4);
}
```

### Test 2 — `arena_alloc_pattern_matches_expected_cursor`

```rust
#[test]
fn arena_alloc_pattern_matches_expected_cursor() {
    let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
    let arena = Arena::new(64 * 1024, backend, 0).expect("arena");

    // Reproduit le pattern exact de realistic_swarm_alloc_pattern::small
    // dim=128, n_agents=8.
    let _l = arena.alloc_zeroed::<f32>(8 * 128).expect("l");
    let cursor_after_l = arena.cursor_bytes();
    // 8 * 128 * 4 = 4096 → aligné à 256 → 4096 (déjà multiple).
    assert_eq!(cursor_after_l, 4096);

    let _m = arena.alloc_zeroed::<u8>(8).expect("m");
    let cursor_after_m = arena.cursor_bytes();
    // 8 → aligné à 256 → 4096 + 256 = 4352.
    assert_eq!(cursor_after_m, 4352);
    // ... (suite du pattern)
}
```

Si demain un changement de `TENSOR_ALIGNMENT` modifie ces nombres,
ce test échoue et force une revue de SLO `D2-8/9/10`.

### Test 3 — `pool_saturation_respects_max_arenas`

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pool_saturation_respects_max_arenas() {
    let pool = Arc::new(make_pool(0, 2, 64 * 1024));
    let mut tasks = vec![];
    for _ in 0..8 {
        let p = pool.clone();
        tasks.push(tokio::spawn(async move {
            let h = p.acquire(512, None).await.expect("a");
            tokio::time::sleep(Duration::from_millis(5)).await;
            drop(h);
        }));
    }
    for t in tasks {
        t.await.expect("join");
    }
    // Jamais alloué plus de 2 arenas malgré 8 tasks.
    assert!(pool.metrics().arenas_allocated_total <= 2);
}
```

### Test 4 — `host_backend_no_dangling_after_pool_drop`

```rust
#[test]
fn host_backend_no_dangling_after_pool_drop() {
    let backend = Arc::new(HostBackend::new());
    {
        let pool = MemoryPool::new(
            PoolConfig::default()
                .with_max_arenas(4)
                .with_prealloc(2)
                .with_arena_size(64 * 1024),
            backend.clone(),
        ).expect("pool");
        assert_eq!(backend.live_allocations(), 2);
        drop(pool);
    }
    assert_eq!(backend.live_allocations(), 0);
}
```

---

## 11. Intégration CI + parser + baseline v0.1.2

### 11.1 `parse_criterion_report.py` — extension

Ajouter au tuple `WANTED_PREFIXES` :

```python
WANTED_PREFIXES = (
    "swarm_e2e_", "memory_e2e_",                         # Sprint C
    "real_e2e_", "real_phase_", "real_memory_",          # Sprint D1
    "pool_acquire_", "arena_alloc_", "arena_reset",      # Sprint D2
    "realistic_swarm_alloc_pattern",                     # Sprint D2
)
```

Ajouter à `SLO_DEFINITIONS` les 10 nouveaux SLO `D2-*` listés en
§9, avec la convention `bench_key = "<group>::<id>"`.

### 11.2 `bench_runner.sh` — ajout du run

Dans la section « Sprint C », ajouter le nouveau bench :

```bash
if [ "$ONLY" != "real" ]; then
  echo "==> Running memory_real_arena bench (Sprint D2, infra)"
  $CARGO bench -p efe-core --bench memory_real_arena -- \
      --output-format bencher $CRITERION_FLAGS \
      > "target/bench-memory-real-arena.txt" 2>&1 \
      || echo "WARN: memory_real_arena returned non-zero (continuing)"
fi
```

### 11.3 `.github/workflows/bench.yml` — paths

Aucun ajout nécessaire : le path filter inclut déjà
`crates/efe-core/src/**` et `crates/efe-core/benches/**` qui couvrent
le nouveau bench.

Étendre l'`Upload bench report artefact` :

```yaml
path: |
  ...
  target/bench-memory-real-arena.txt
```

### 11.4 `bench_baselines/v0.1.2/baseline.json` — placeholder

Stratégie identique à D1 / v0.1.1 : créer un fichier placeholder
avec `results: {}` et un `_note` documentant que la vraie baseline
sera le premier run propre post-merge sur `main`. Comparaison vs
v0.1.1 reste active pour les benches D1 ; les D2 sont isolés.

### 11.5 `BENCHMARKS.md` — nouvelle section § 4

Insérer entre la section actuelle § 3 (Throughput) et § 4 (Mémoire) :

```markdown
## 4. Pool / Arena infrastructure (Sprint D2, v0.1.2)

Mesures isolées de la couche `efe_core::memory::{MemoryPool, Arena}`
qui sous-tend toute requête EFE. Ces chiffres sont lus comme des
**bornes inférieures** : tout `real_e2e_latency_*` (D1) inclut au
minimum 1 acquire warm + 5 alloc_zeroed + 1 reset.

### 4.1 Pool acquire

| Régime | Bench key | médian | p95 | SLO |
|--------|-----------|-------:|----:|----:|
| warm | `pool_acquire_warm::default` | ~280 ns | ≤ 1 µs | SLO-D2-1 |
| cold | `pool_acquire_cold::1MiB` | ~330 µs | ≤ 1 ms | SLO-D2-2 |
| concurrent (8c) | `pool_concurrent_acquire::c8` | ~10 µs | ≤ 50 µs | SLO-D2-3 |

### 4.2 Arena allocation (memset bandwidth)

| count (f32) | bytes | médian | SLO |
|-------------|-------|-------:|----:|
| 1 | 256 (aligné) | ~150 ns | SLO-D2-4 |
| 1 024 | 4 KiB | ~1.5 µs | SLO-D2-5 |
| 1 000 000 | 4 MB | ~1.4 ms | SLO-D2-6 |

À grandes tailles, le coût est ∝ bande passante memset (~3 GB/s sur
Apple Silicon, ~6 GB/s sur skylake-server).

### 4.3 Pattern réel par fixture

| fixture | total bytes | médian | SLO |
|---------|------------:|-------:|----:|
| small | ~12 KiB | ~3 µs | SLO-D2-8 |
| medium | ~70 KiB | ~20 µs | SLO-D2-9 |
| large | ~330 KiB | ~100 µs | SLO-D2-10 |
```

### 11.6 `docs/slo.md` — nouvelle table § 5

Mirror exact des SLO `D2-*` listés en §9 ; ajout après la table § 4
dédiée à D1.

---

## 12. Critères d'acceptation (DoD)

Pour considérer Sprint D2 *done*, **tous** les critères suivants
doivent être satisfaits :

### 12.1 Code & build

- [ ] `crates/efe-core/benches/memory_real_arena.rs` créé, ~500 LOC.
- [ ] `crates/efe-core/Cargo.toml` ajout `[[bench]] name = "memory_real_arena"`.
- [ ] `crates/efe-core/benches/common/mod.rs` étendu avec `make_pool`
      / `make_runtime` (helpers réutilisables).
- [ ] `cargo build -p efe-core --benches --release` compile clean.
- [ ] `cargo clippy -p efe-core --benches -- -D warnings` clean.

### 12.2 Tests

- [ ] `crates/efe-core/tests/pool_realism_check.rs` créé, 4 tests
      (cf. § 10).
- [ ] `cargo test -p efe-core --test pool_realism_check` → 4/4 pass.
- [ ] `cargo test -p efe-core` (suite complète, ~1 421+ tests) →
      tous pass, aucun nouveau flake.

### 12.3 Bench actually runs

- [ ] `cargo bench -p efe-core --bench memory_real_arena -- --quick`
      complète sans erreur.
- [ ] 7 groupes criterion produisent des mesures.
- [ ] Aucune mesure n'est `+inf` ou `0` (signe de bench cassé).

### 12.4 Tooling

- [ ] `scripts/bench_runner.sh` lance le nouveau bench
      (section Sprint C, conditionnée par `ONLY`).
- [ ] `scripts/parse_criterion_report.py` reconnaît les groupes D2
      (extension `WANTED_PREFIXES` + `SLO_DEFINITIONS`).
- [ ] `bench_baselines/v0.1.2/baseline.json` placeholder créé.
- [ ] `.github/workflows/bench.yml` upload artefact étendu.

### 12.5 Documentation

- [ ] `BENCHMARKS.md` § 4 « Pool / Arena infrastructure » ajoutée.
- [ ] `docs/slo.md` § 5 « SLO D2 infrastructure » ajoutée.
- [ ] `docs/bench/d2_memory_real_arena_architecture.md` (ce
      document) référencé en intro de `BENCHMARKS.md`.
- [ ] Tous les commentaires de bench rationalisent leur sample size,
      cible SLO, et risque de mesure.

### 12.6 Validation finale

- [ ] Ordre de grandeur des mesures cohérent avec § 9 (×3 max d'écart).
- [ ] `pool_acquire_warm` < 1 µs sur M3 Pro (cible primaire).
- [ ] Aucune fuite d'arène après tous les benches
      (`HostBackend::live_allocations() == 0` checké).

---

## 13. Out of scope & roadmap follow-ups

### 13.1 Hors-scope D2 (rejeté pour focus)

* **Heap profiling avec `peak_alloc`**. Sera ajouté en **D2.5** dans
  un binaire séparé `memory_real_arena_peak.rs`. La séparation est
  **obligatoire** : `peak_alloc` comme global allocator pollue toutes
  les timings d'un même binaire (cf. D1 où `real_memory.rs` est
  disjoint de `real_strategies_e2e.rs`).
* **Bench du `StreamPool`**. Différent module (`efe_core::streams`),
  différent rôle (pour CUDA streams, no-op sur CPU). Sera **D3**.
* **Bench du backend GPU CUDA**. La feature `cuda` est gated et
  hors-scope MVP. Sera couvert quand on aura un runner Linux+GPU
  CI (Phase 2 roadmap).
* **Bench `try_acquire`**. Très proche de `acquire warm` ; pas de
  valeur additionnelle prouvée. À ré-évaluer si on observe une
  divergence en prod.
* **Comparaison BLAS vs naïve**. Aucun BLAS dans la couche pool/arena.
* **Mesure GIB-scale arenas**. `BENCH_ARENA_SIZE = 1 MiB` capture le
  pattern de coût ; passer à 1 GiB pour valider le memset coast à
  l'échelle prod = D4 (« long-running stress »).

### 13.2 Roadmap après D2

| Sprint | Cible | Justification |
|--------|-------|---------------|
| **D2.5** | `memory_real_arena_peak.rs` (heap peak D2) | Couvre l'autre moitié de l'observabilité mémoire ; séparation pour pristine timings |
| **D3** | `streams_real_pool.rs` (StreamPool bench) | Sera nécessaire dès Phase 2 CUDA pour qualifier la tokio::Semaphore vs cudaStream wait |
| **D4** | `long_running_stress.rs` | Cycle 100 000 acquires sur 1 h, vérifie pas de drift mémoire ni dégradation latence |
| **D5** | `kernel_micro_breakdown.rs` | Per-kernel cost (softmax, attention, mix) avec inputs réels — comble le dernier trou avant Phase 2 |

### 13.3 Métrique de succès post-D2

À 30 jours après merge D2, on doit pouvoir affirmer dans le board
performance :

> *« Toute régression > 50 % sur l'infrastructure mémoire EFE
>  produit un fail SLO bench-niveau dans les 24 h après le merge,
>  avec une attribution exacte à `pool::acquire`, `arena::alloc`,
>  ou `arena::reset`. »*

Si cette affirmation tient, D2 a rempli sa mission.

---

## Annexe A — Schéma de séquence : un acquire warm

```
Caller                  MemoryPool             Semaphore         Mutex<free_list>      Arena
  │                         │                      │                   │                  │
  ├ acquire(min, None) ────►│                      │                   │                  │
  │                         ├ metrics.acq+1        │                   │                  │
  │                         ├ slots.acquire_owned()►                   │                  │
  │                         │◄──── permit(t) ──────┤                   │                  │
  │                         ├ free_list.lock() ───────────────────────►│                  │
  │                         │◄──── guard ──────────────────────────────┤                  │
  │                         ├ iter().position(cap>=min)                │                  │
  │                         ├ swap_remove(idx)                         │                  │
  │                         │◄──── arena ──────────────────────────────┤                  │
  │                         ├ metrics.hit+1                            │                  │
  │                         ├ metrics.{arenas,bytes}_active+=1         │                  │
  │                         ├ ArenaHandle::new(arena, weak, permit)    │                  │
  │◄────── handle ──────────┤                      │                   │                  │
  │ ... use arena ...       │                      │                   │                  │
  ├ drop(handle) ──────────►│ (returns Arena to pool, releases permit) │                  │
  │                         ├ inner.return_arena(arena) ──────────────►│                  │
  │                         ├ metrics.{arenas,bytes}_active-=1         │                  │
```

Ce schéma est la **référence visuelle** pour le commentaire en tête
de `bench_pool_acquire_warm` ; il documente la chaîne d'opérations
benchée et explique les coûts de § 5.1.

## Annexe B — Coûts mémoire détaillés du pattern Mvp

Pour `realistic_swarm_alloc_pattern::medium` (dim=256, n_agents=32) :

| Allocation | T | count | bytes (utilisé) | aligned (cursor++) | cursor cumulé |
|-----------|---|------:|----------------:|-------------------:|--------------:|
| latents | f32 | 8 192 | 32 768 | 32 768 | 32 768 |
| mask | u8 | 32 | 32 | 256 | 33 024 |
| attention | f32 | 1 024 | 4 096 | 4 096 | 37 120 |
| confidence | f32 | 32 | 128 | 256 | 37 376 |
| scratch | f32 | 8 192 | 32 768 | 32 768 | 70 144 |
| **Total** | — | — | **69 824** | **70 144** | — |

Donc une arène 1 MiB contient 14 cycles avant exhaustion, mais
on reset entre chaque iteration → jamais d'épuisement.

## Annexe C — Diagramme dépendance des modules touchés

```
                 ┌──────────────────────┐
                 │  benches/memory_real │
                 │      _arena.rs       │
                 └──────────┬───────────┘
                            │ uses
        ┌───────────────────┼────────────────────┐
        ▼                   ▼                    ▼
┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐
│ benches/common  │  │ efe_core::memory│  │ tokio runtime   │
│   /mod.rs       │  │  ::{Pool,Arena} │  │ (multi_thread)  │
└─────────────────┘  └────────┬────────┘  └─────────────────┘
                              │
                       uses internally
                              │
        ┌─────────────────────┼─────────────────────┐
        ▼                     ▼                     ▼
┌────────────────┐  ┌─────────────────┐  ┌─────────────────┐
│ HostBackend    │  │ tokio::Semaphore│  │ Mutex<Vec<Arena>>│
│ (mmap+memset)  │  │  (slots cap)    │  │   (free-list)   │
└────────────────┘  └─────────────────┘  └─────────────────┘
```

**Aucune dépendance vers `efe-server`, `efe-telemetry`, ou les
crates de protobuf**. C'est ce qui justifie la localisation
`efe-core/benches/` (D1 §3).

---

## Annexe D — Checklist exécution architect → impl

À cocher au fur et à mesure de l'implémentation par le sprinter :

```
□ D2.1.1  common/mod.rs : ajouter `make_pool(prealloc, max, size)`
□ D2.1.2  common/mod.rs : ajouter `make_runtime()` si pas déjà
□ D2.1.3  common/mod.rs : tests smoke `cfg(test) mod`

□ D2.2.1  memory_real_arena.rs : skeleton + imports
□ D2.2.2  memory_real_arena.rs : constantes BENCH_*
□ D2.2.3  memory_real_arena.rs : bench_pool_acquire_warm
□ D2.2.4  Cargo.toml : ajout [[bench]] name = "memory_real_arena"
□ D2.2.5  cargo bench --quick : 1er run propre

□ D2.3.1  bench_pool_acquire_cold (Option B iter_batched)
□ D2.3.2  bench_pool_acquire_saturated (contention_only variant)
□ D2.3.3  bench_pool_concurrent_acquire (8c)
□ D2.3.4  validation manuelle des ordres de grandeur

□ D2.4.1  bench_arena_alloc_zeroed_typed (3 counts)
□ D2.4.2  bench_arena_reset

□ D2.5.1  bench_realistic_swarm_alloc_pattern (3 fixtures)
□ D2.5.2  tests/pool_realism_check.rs (4 tests)
□ D2.5.3  parse_criterion_report.py extension (WANTED + SLO)
□ D2.5.4  bench_runner.sh ajout
□ D2.5.5  BENCHMARKS.md § 4 + slo.md § 5
□ D2.5.6  bench_baselines/v0.1.2/baseline.json placeholder
□ D2.5.7  .github/workflows/bench.yml upload artefact path

□ Validation finale
  □ cargo test -p efe-core (full)
  □ cargo clippy -p efe-core --benches -- -D warnings
  □ cargo bench --bench memory_real_arena (full, non-quick)
  □ Vérification metric attendues vs §9 (3× max écart)
  □ Aucune fuite arène (HostBackend::live_allocations == 0)
```

---

**Fin de l'architecture D2.** Le sprinter peut maintenant implémenter
en suivant la checklist Annexe D, en revenant à §5 pour le détail
de chaque groupe et §7 pour les pièges à éviter.

---

> **Approbation requise avant implémentation** :
> - [ ] Architecte performance (revue technique).
> - [ ] Owner `efe_core::memory` (revue invariants).
> - [ ] Lead infra (revue intégration CI + baselines).
