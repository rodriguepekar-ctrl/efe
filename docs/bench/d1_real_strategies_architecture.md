# D1 — `real_strategies_e2e` Architecture complète, ultra-détaillée

---

**Document** : spécification d'architecture
**Phase** : D1 du Sprint C-bis (remédiation post-audit)
**Durée cible** : 1 jour (8 h budget, 1 h buffer)
**Auteur** : architecture & design — `rodrigue.pekar@icloud.com`
**Dépend de** :
- `docs/bench/bench_foundation_architecture.md` (Sprint C cadre)
- Audit ultrathink (verdict : couverture < 5 %)
- `crates/efe-server/src/mode1.rs::build_mvp_backend` (canon de production)
**Consommé par** : Phase 2 CUDA (référence chiffrée), `BENCHMARKS.md` v0.1.1
**Statut** : spec gelée pour implémentation

> **Mission** : transformer le bench actuel (qui mesure ~3 µs avec
> des mocks no-op) en un **véritable benchmark EFE de production**
> qui exécute les **5 stratégies réelles** (`MvpCpuFusion`,
> `GenesisEngine`, `MultiCriteriaDetector`, `DissolutionEngine`,
> `StrictAlignmentProjector`) et le wiring complet
> `build_mvp_backend` — exactement comme le serveur Mode 1.
>
> À l'issue de D1, les chiffres `swarm_e2e_*_latency` doivent passer
> de **3 µs (mocks)** à **1-50 ms (réel)**, et les SLO publiés dans
> `SLO.md` deviennent des engagements **mesurés**, pas théoriques.

---

## Table des matières

1. [Mission, critères, contrainte critique](#1-mission-critères-contrainte-critique)
2. [État AS-IS — rappel chirurgical](#2-état-as-is--rappel-chirurgical)
3. [Wiring canonique de production (`build_mvp_backend`)](#3-wiring-canonique-de-production)
4. [Inventaire des stratégies réelles + leurs adapters](#4-inventaire-des-stratégies-réelles--leurs-adapters)
5. [Génération d'inputs réalistes](#5-génération-dinputs-réalistes)
6. [Choix de localisation : `efe-server/benches/`](#6-choix-de-localisation--efe-serverbenches)
7. [Plan en 7 phases (D1.1 → D1.7)](#7-plan-en-7-phases-d11--d17)
8. [Stratégie de mesure et per-phase profiling](#8-stratégie-de-mesure-et-per-phase-profiling)
9. [Stratégie de validation du bench lui-même](#9-stratégie-de-validation-du-bench-lui-même)
10. [Plan jour-par-jour (1 j)](#10-plan-jour-par-jour-1-j)
11. [Risques et mitigations](#11-risques-et-mitigations)
12. [Annexes](#12-annexes)

---

## 1. Mission, critères, contrainte critique

### 1.1 Mission

Délivrer un bench end-to-end qui :

1. Exécute **`build_mvp_backend(cfg).execute(req)`** — exactement la
   chaîne de production utilisée par `efe-server`.
2. Mesure la latence p50/p95 sur 3 tailles (small / medium / large).
3. Décompose le coût par phase (genesis, fusion N rounds, consensus
   N invocations, dissolution, alignment) pour identifier le hot path.
4. Mesure la mémoire heap réelle via `peak_alloc` (avec arena
   `MemoryPool::new` actives).
5. Verrouille les chiffres dans `BENCHMARKS.md` v0.1.1 et la
   baseline `v0.1.1`.

### 1.2 Critères de sortie

| # | Critère | Mesure |
|---|---------|:------:|
| C1 | Le bench utilise `build_mvp_backend` ou son équivalent direct | revue source |
| C2 | Aucun mock dans le chemin chaud du bench | source-tree grep `BenchMock`, `NullTelemetry` |
| C3 | Latence small mesurée > 100 µs (preuve qu'on mesure du compute réel) | criterion output |
| C4 | Décomposition par phase publiée | `BENCHMARKS.md` v0.1.1 §3 |
| C5 | Heap peak medium > 10 KB (preuve d'allocations réelles via arena) | `memory_real_arena` (ou équivalent) |
| C6 | Variance bench p99/p50 < 3 (preuve de stabilité sous compute réel) | criterion analyzer |
| C7 | `BENCHMARKS.md` v0.1.1 publié avec disclaimer levé | git diff |
| C8 | `SLO.md` recalibré sur mesures réelles | git diff |
| C9 | Baseline `v0.1.1` figée et CI configurée | `bench_baselines/v0.1.1/` |
| C10 | Tests régression : 1 test asserts `swarm_e2e_real_latency > 100 µs` | `tests/bench_realism_check.rs` |

### 1.3 Contrainte critique : le bench ne doit JAMAIS reverter aux mocks

Une fois D1 livré, **toute PR** qui réintroduit `BenchMock*` dans le
chemin chaud du bench `swarm_e2e` doit échouer en CI. Mécanisme :

* Test source-tree dans `bench_realism_check.rs` qui parse
  `efe-server/benches/real_strategies_e2e.rs` et asserts qu'aucun
  identifiant `Mock` ou `Null` n'apparaît dans le code (hors
  commentaires).
* Test runtime qui asserts que `swarm_e2e_real_small_latency` ≥
  100 µs (un mock no-op donne ~3 µs ; une régression aux mocks
  serait détectée).

---

## 2. État AS-IS — rappel chirurgical

### 2.1 Ce que le bench actuel mesure

Le bench `crates/efe-core/benches/swarm_e2e.rs` instancie :

```rust
.genesis(Arc::new(BenchMockGenesis { ... }))    // returns NanoAgentBatch::new_stub
.fusion(Arc::new(BenchMockFusion::new(dim)))    // returns synthetic_converging
.dissolver(Arc::new(BenchMockDissolver::new(dim))) // returns vec![0.05; dim]
.alignment(Arc::new(BenchMockAligner))          // returns identity, delta=0
.telemetry(Arc::new(NullTelemetryCollector))    // no-op
.consensus_factory(|| ScriptedDetectorBench)    // pre-baked verdicts
```

Toutes les stratégies retournent en quelques nanosecondes. Le bench
mesure exclusivement :

* Tokio task setup (`spawn`, `await`)
* `Arc::clone` × ~6 par requête
* Trait dispatch via `Arc<dyn>`
* `tracing::Span::current().record(...)` × ~3
* Verdict matching dans `controller::drive()`
* Construction et finalize du `TelemetryGuard`

**Total mesuré : ~3 µs.** C'est le **pur overhead d'orchestration**.

### 2.2 Ce qui n'est PAS mesuré (les vrais hot paths)

| Phase | Code production correspondant | Coût estimé* |
|-------|-------------------------------|--------------|
| Genesis | `GenesisEngine::spawn` → `MetaAnalyzer::decompose` (8 cosines + softmax + entropy) → `Router::select` → `Cardinality::estimate` → `DefaultSpawner::spawn` (arena alloc + zero-init + template instantiation) | **~200-500 µs** |
| Fusion (par round) | `MvpCpuFusion::run_round` → `compute_scores` (N²·D dot products) → `softmax_in_place_rows` (N²·exp) → `aggregate` (N²·D) → `beta_mix` (N·D) → `project_rows` (N·anchors·D) → `clip_and_check` | **~50-2000 µs** selon taille |
| × 8 rounds | (boucle) | **~400 µs - 16 ms** |
| Consensus | `MultiCriteriaDetector::evaluate` → `compute_signals` (6 signaux) + `ConfidenceScorer` + history update | **~10-30 µs** par round |
| Dissolution | `WeightedMeanExtractor::extract` (N·D moyenne pondérée) + `IdentityDecoder` | **~5-20 µs** |
| Alignment | `StrictAlignmentProjector::project` → `compute::cosine_anchor_blend` (N·D) + `l2_distance` + breach check | **~10-50 µs** |

\* Estimations basées sur les µ-benchs `compute_sprint{2,3}` et la
théorie (op count × débit f32 sur M1 Pro).

**Total estimé pour spec small (8 rounds, dim=128, N=8, anchors=4)** :

```
genesis    : ~300 µs
fusion ×8  : ~3 200 µs (~400 µs/round)
consensus  : ~80 µs (8 × 10)
dissolution: ~10 µs
alignment  : ~20 µs
overhead   : ~3 µs (mesure actuelle = TOUT le bench)
─────────
TOTAL      : ~3 600 µs ≈ 3.6 ms
```

Soit **~1 200× le chiffre actuel**. Le SLO « small p95 < 50 ms »
laisse alors une marge × 14 (cohérente, défendable). Avant D1, la
marge × 17 000 trahit que le bench est creux.

### 2.3 Pourquoi mesurer le réel matériellement

Trois raisons de fond :

1. **Confiance contractuelle**. Un SLO publié auprès d'un client
   doit être basé sur ce que le client paye (Mode 1 = `build_mvp_backend`),
   pas sur un microcosme sans rapport.
2. **Identification du goulet**. Avec mocks, on ne peut pas dire
   « 70 % du temps est dans le softmax » — alors que c'est
   probablement vrai. La décomposition par phase (D1.4) le montre.
3. **Phase 2 CUDA quantifiable**. Sans mesure CPU réelle, on ne
   peut pas annoncer « CUDA × 10 sur fusion » : on ne sait même
   pas combien de temps prend la fusion CPU.

---

## 3. Wiring canonique de production

Le serveur Mode 1 (`crates/efe-server/src/mode1.rs::build_mvp_backend`)
assemble les 12 composants suivants :

```text
┌─────────────────────────────────────────────────────────────────┐
│ build_mvp_backend(cfg: Mode1Config) -> Arc<dyn ExecuteBackend>  │
└──────────────────────┬──────────────────────────────────────────┘
                       │
       ┌───────────────┼───────────────┬───────────────┐
       ▼               ▼               ▼               ▼
   ┌────────┐    ┌──────────┐   ┌──────────┐   ┌─────────────┐
   │ 1. host│    │2. memory │   │3. stream │   │4. template  │
   │backend │    │  pool    │   │  pool    │   │  registry   │
   │HostBack│    │MemPool   │   │StreamPool│   │StubTemplate │
   │   end  │    │PoolConfig│   │size=cfg.n│   │mvp_defaults │
   │        │    │prealloc=8│   │          │   │ (3 templ)   │
   │        │    │max=64    │   │          │   │             │
   └────────┘    └──────────┘   └──────────┘   └─────────────┘
                       │               │               │
                       └───────────────┼───────────────┘
                                       ▼
                          ┌─────────────────────────┐
                          │ 5. genesis              │
                          │   GenesisEngine.build_  │
                          │   mvp(pool, registry,   │
                          │   stream_pool)          │
                          └────────────┬────────────┘
                                       │
                          ┌────────────▼────────────┐
                          │ 6. fusion_core          │
                          │ MvpCpuFusion::builder() │
                          │  .with_backend(host)    │
                          │  .with_replay(false)    │
                          │  .build()               │
                          └────────────┬────────────┘
                                       │
                          ┌────────────▼────────────┐
                          │ 7. last_centroid +      │
                          │    FusionAdapter        │
                          │ (bridge + extractor)    │
                          └────────────┬────────────┘
                                       │
       ┌───────────────────────────────┼───────────────┐
       ▼                               ▼               ▼
  ┌─────────┐                ┌──────────────┐   ┌─────────────┐
  │8.consen-│                │ 9. dissolver │   │10.alignment │
  │ sus    │                │ PassThrough  │   │ Adapter on  │
  │factory  │                │ Dissolver    │   │StrictAlign  │
  │(Multi   │                │              │   │mentProj.    │
  │Criteria)│                │              │   │             │
  └────┬────┘                └──────┬───────┘   └──────┬──────┘
       │                            │                  │
       └────────────┬───────────────┴──────────────────┘
                    ▼
          ┌─────────────────────┐
          │ 11. controller_cfg  │
          │ ControllerConfig::  │
          │ default()           │
          └──────────┬──────────┘
                    ▼
          ┌─────────────────────────────────────┐
          │ 12. SwarmController::builder()      │
          │       .genesis(...).fusion(...)     │
          │       .dissolver(...).alignment(...)│
          │       .telemetry(Null).config(...)  │
          │       .consensus_factory(...)       │
          │       .build()                      │
          └─────────────────────────────────────┘
                    ▼
          Mode1Backend wrapper (resets centroid before each request)
```

### 3.1 Implications pour le bench

* **Nous réutilisons `build_mvp_backend` directement**, pas une
  copie. Toute évolution du wiring de production se reflète
  automatiquement dans le bench.
* Une seule construction pour toute la suite (déplaçable via
  `criterion::Bencher::iter_with_setup` si besoin de reset entre
  itérations — typiquement non, car `Mode1Backend` reset déjà la
  cellule `LastCentroid` à chaque request).
* `Mode1Config::default()` donne des paramètres déjà validés en
  production. On ne s'écartera pas du défaut sauf à augmenter
  `max_arenas` à 256 pour absorber des bursts de bench concurrents.

---

## 4. Inventaire des stratégies réelles + leurs adapters

### 4.1 Tableau des composants mesurés

| # | Composant | Type | Coût opératoire | Sub-bench dédié ? |
|---|-----------|------|------------------|:------------------:|
| 1 | `HostBackend` | `DeviceBackend` | mémoire OS (alloc/dealloc) | non (transparent) |
| 2 | `MemoryPool` | resource | arena RAII, alloc per-request | oui (D1.4) |
| 3 | `StreamPool` | resource | semaphore acquire/release | non (audité B1-B3) |
| 4 | `StubTemplateRegistry::mvp_defaults` | static | lookup O(1) | non (négligeable) |
| 5 | `GenesisEngine.build_mvp` | strategy | meta + router + cardinality + spawner | **oui** (D1.4) |
| 6 | `MvpCpuFusion` | strategy | scores + softmax + aggregate + project + clip | **oui** (D1.4) |
| 7 | `FusionAdapter` | bridge | extract centroid post-round | inclus dans 6 |
| 8 | `MultiCriteriaDetector` (× 1 par requête) | strategy | 6 signaux + history + scorer | **oui** (D1.4) |
| 9 | `PassThroughDissolver` | strategy | lit `LastCentroid` cell | inclus dans 6 |
| 10 | `AlignmentAdapter::strict_default` | strategy | cosine_anchor_blend + breach | **oui** (D1.4) |
| 11 | `NullTelemetryCollector` | strategy | no-op (cohérent avec config bench) | non |
| 12 | `SwarmController` | orchestrateur | dispatch + state machine | déjà mesuré (E2E) |

### 4.2 Adapters

Trois adapters bridgent les traits `swarm` et les traits `core` :

| Adapter | Rôle | Coût |
|---------|------|------|
| `FusionAdapter::run_round` | délègue à `MvpCpuFusion::run_round`, capture centroïde | extract centroid (~5-20 µs) |
| `ConsensusAdapter::evaluate` | mappe `ConsensusVerdict` → `swarm::Verdict` | match (~ns) |
| `AlignmentAdapter::project` | wrap `StrictAlignmentProjector::project` (4 args) → `(Vec, f32)` (2 args) | wrap (~ns) |

Ces 3 adapters sont implémentés dans `efe-server/src/mode1.rs` —
**le bench les couvre automatiquement** en passant par `build_mvp_backend`.

---

## 5. Génération d'inputs réalistes

### 5.1 Pourquoi `[0.0; dim]` ne suffit pas

Le `problem_embedding` actuel des 3 fixtures (`realistic_*.json`)
est `[0.0] * dim`. Ce zéro propagé jusqu'au `HeuristicMetaAnalyzer`
produit des **cosines tous nuls** vis-à-vis des exemplars (qui sont
des vecteurs sinusoïdaux normalisés non-nuls).

Conséquence : `softmax(scores - max)` sur un vecteur de zéros
donne une distribution **uniforme**. Le DAG est donc `[1/3, 1/3, 1/3]`
indépendamment de l'input — **dégénéré**, mais légal. Le bench
s'exécute sans erreur, mais le `meta_analyzer` ne fait quasiment
aucun travail discriminant.

Pour un bench **représentatif**, on veut un embedding qui :

* Active une discrimination réelle entre exemplars (cosines variés
  dans `[-0.9, +0.9]`)
* Soit **déterministe** (seed fixed, valeurs immuables entre runs)
* Ait une norme finie raisonnable (pas zéro, pas overflow)
* Soit indépendant de la dimension (même processus pour 128 et 256)

### 5.2 Algorithme proposé

Pour chaque dimension `dim`, générer un embedding via PRNG seedé :

```python
def realistic_embedding(dim: int, seed: int = 42) -> list[float]:
    """Pseudo-Gaussian embedding via Box-Muller from a fixed seed."""
    rng = pcg64(seed)
    out = []
    while len(out) < dim:
        u1 = (rng.next_u32() / 2**32) + 1e-12
        u2 = (rng.next_u32() / 2**32)
        z = sqrt(-2 * log(u1)) * cos(2 * pi * u2)
        out.append(z * 0.1)   # scale 0.1 to keep ‖x‖ ≈ √(dim) · 0.1
    return out
```

**Propriétés** :
* `‖embedding‖₂` ≈ `√dim · σ = √128 · 0.1 ≈ 1.13` pour dim=128
* Toutes les composantes finies, distribution centrée sur 0
* Cosines avec exemplars sinusoïdaux non-dégénérés

### 5.3 Mise à jour des fixtures

Le générateur `scripts/gen_fixtures.py` doit produire `problem_embedding`
non-zero, mais **on garde les anchors orthogonaux** (one-hot) car
ils servent à valider l'invariant I5 d'`alignment::policy`.

Trois nouveaux fichiers :

```
scripts/fixtures/realistic_small_v2.json    (dim=128, real embedding)
scripts/fixtures/realistic_medium_v2.json   (dim=256)
scripts/fixtures/realistic_large_v2.json    (dim=256)
```

Convention de nommage `_v2` pour ne pas casser les benchs existants
qui dépendent de l'embedding zero.

### 5.4 Anchors

Les anchors actuels (one-hot orthogonaux) sont **OK pour D1**.
L'objectif du bench n'est pas la diversité d'anchors mais la
fidelité du chemin d'exécution. Les anchors orthogonaux maximisent
la séparation cosine, ce qui **stresse** maximalement le projector
(toutes les distances `|cos - 1|` sont visibles).

---

## 6. Choix de localisation : `efe-server/benches/`

### 6.1 Pourquoi pas dans `efe-core/benches/`

Le bench D1 doit utiliser `build_mvp_backend(cfg)`, qui vit dans
`efe-server/src/mode1.rs`. Un bench dans `efe-core/benches/` ne peut
pas dépendre de `efe-server` (sens topologique du graphe).

Trois options :

* **A. Bench dans `efe-server/benches/`** : utilise `build_mvp_backend`
  directement. ✅ pas de duplication, suit la production.
* **B. Bench dans `efe-core/benches/` avec wiring dupliqué** :
  re-implémente les 3 adapters + le wiring. ❌ ~150 LOC dupliqués,
  divergence garantie.
* **C. Extraire le wiring dans une lib partagée `efe-bench-fixtures/`** :
  surarchitecturé pour D1, propre pour D2+.

**Décision : Option A.** Le bench vit dans `efe-server/benches/`
et utilise `build_mvp_backend` tel quel. C'est aussi cohérent
sémantiquement : « benchmark Mode 1 server » = bencher la chaîne
que le serveur sert.

### 6.2 Implication CI

`scripts/bench_runner.sh` doit lancer aussi `efe-server` :

```bash
$CARGO bench -p efe-server --bench real_strategies_e2e -- ...
```

Le `bench-report.json` consolide les mesures `efe-core` ET
`efe-server` dans un seul fichier (le parser walk
`target/criterion/` qui est partagé).

---

## 7. Plan en 7 phases (D1.1 → D1.7)

### D1.1 — Dépendances `efe-server` (0.5 h)

**Fichier** : `crates/efe-server/Cargo.toml`

Ajouter :

```toml
[dev-dependencies]
criterion = { workspace = true, features = ["async_tokio"] }
peak_alloc = "0.3"

[[bench]]
name = "real_strategies_e2e"
harness = false

[[bench]]
name = "real_memory"
harness = false
```

Vérifier que `criterion` est bien dans `[workspace.dependencies]` du
`Cargo.toml` racine. Si non, l'ajouter.

### D1.2 — Fixtures v2 avec embeddings réalistes (0.5 h)

**Fichier** : `scripts/gen_fixtures_v2.py`

Étend `gen_fixtures.py` avec un embedder Box-Muller seedé. Génère
3 fichiers JSON `realistic_{small,medium,large}_v2.json` qui ont :

* `problem_embedding` : vecteur Gaussien standard ×0.1, dim=128/256
* Tout le reste identique aux fixtures `_v1`

Validation : un test `bench_smoke_realism.rs` parse le fichier et
vérifie `‖embedding‖₂ ∈ [0.5, 5.0]` (norme raisonnable).

### D1.3 — `efe-server/benches/common/mod.rs` (1.5 h)

**Fichier** : `crates/efe-server/benches/common/mod.rs` (~250 LOC)

Helpers partagés par `real_strategies_e2e.rs` et `real_memory.rs` :

* `load_realistic_v2_spec(name) -> SwarmSpec` — charge
  `realistic_*_v2.json` (réutilise le pattern existant de
  `efe-core/benches/common/mod.rs::load_realistic_spec`)
* `default_mode1_config(dim_latent) -> Mode1Config` — défaut
  Mode 1 paramétré par `dim_latent`
* `build_real_backend(spec) -> Arc<dyn ExecuteBackend>` — wrap
  `build_mvp_backend(default_mode1_config(spec.dim_latent.get()))`
* `build_real_request(spec) -> ExecuteRequest` — construit la
  requête (qui inclut `Request` + headers + tracing)

### D1.4 — `efe-server/benches/real_strategies_e2e.rs` (3 h)

**Fichier principal du sprint** (~500 LOC).

Structure :

```text
real_strategies_e2e.rs
├── bench_real_e2e_latency           # full pipeline end-to-end
│   ├── small  (8 ag, dim=128, 8 rounds)
│   ├── medium (32 ag, dim=256, 8 rounds)
│   └── large  (128 ag, dim=256, 8 rounds)
│
├── bench_real_e2e_throughput
│   ├── small × Nc=1, 4, 16
│   └── medium × Nc=1, 4
│
├── bench_real_per_phase             # decomposition
│   ├── genesis_only_real
│   ├── fusion_one_round_real
│   ├── consensus_evaluate_real
│   ├── dissolution_extract_decode_real
│   └── alignment_project_real
│
└── bench_real_blas_vs_naive
    └── medium × {pure_rust, accelerate}
```

Chaque sous-bench a son propre setup minimal :

* **e2e** : un `Arc<dyn ExecuteBackend>` partagé, `iter` appelle
  `backend.execute(req)`.
* **per_phase** : isole une seule strategy en construisant un
  controller avec mocks **partout SAUF** la stratégie testée. Donne
  une mesure **différentielle** : `t(real_genesis) - t(mock_genesis)`
  ≈ coût réel du genesis.

### D1.5 — `efe-server/benches/real_memory.rs` (1 h)

**Fichier** : `~200 LOC`

Identique à `memory_e2e.rs` (Sprint C) mais **avec backend réel**.
Utilise `peak_alloc::PeakAlloc` comme global allocator. Mesure :

* `real_memory_<size>_heap_peak`
* `real_memory_<size>_alloc_count`
* `real_memory_<size>_arena_lifecycle` (mesure spécifique au
  `MemoryPool` : alloc + zero + dealloc)

### D1.6 — Tests anti-régression (0.5 h)

**Fichier** : `crates/efe-server/tests/bench_realism_check.rs` (~100 LOC)

Trois tests :

```rust
#[test]
fn bench_uses_no_mock_in_hot_path() {
    let src = include_str!("../benches/real_strategies_e2e.rs");
    assert!(!src.contains("BenchMock"), "real bench must not use BenchMock*");
    assert!(!src.contains("NullTelemetry"), "real bench must not nullify telemetry");
    assert!(!src.contains("ScriptedDetector"), "must use real MultiCriteriaDetector");
    // The only exception is in `bench_real_per_phase` where we DO
    // use mocks for the OTHER strategies — but the file must
    // explicitly route through `build_mvp_backend` for the e2e bench.
    assert!(src.contains("build_mvp_backend"), "must use canonical wiring");
}

#[tokio::test]
async fn realism_check_small_takes_more_than_100us() {
    // If small p50 < 100µs, the bench has reverted to mocks.
    // 100µs is below the SLO ceiling but well above the 3µs mock floor.
    let backend = build_real_backend(load_realistic_v2_spec("small"));
    let req = build_real_request(load_realistic_v2_spec("small"));
    let t0 = std::time::Instant::now();
    backend.execute(req).await.unwrap();
    let elapsed = t0.elapsed();
    assert!(
        elapsed >= std::time::Duration::from_micros(100),
        "elapsed = {elapsed:?} < 100µs — bench has reverted to mocks?"
    );
}

#[tokio::test]
async fn realism_check_medium_p95_under_200ms() {
    // SLO smoke: medium should converge under 200ms (with margin
    // for dev hardware). If this fails, either real strategies have
    // regressed OR the SLO needs renegotiating.
    let backend = build_real_backend(load_realistic_v2_spec("medium"));
    let mut max = std::time::Duration::ZERO;
    for _ in 0..10 {
        let t0 = std::time::Instant::now();
        backend.execute(build_real_request(load_realistic_v2_spec("medium"))).await.unwrap();
        max = max.max(t0.elapsed());
    }
    assert!(max < std::time::Duration::from_millis(200), "medium p95 SLO breach: {max:?}");
}
```

### D1.7 — Mise à jour `bench_runner.sh`, `BENCHMARKS.md`, baseline `v0.1.1` (1 h)

* `bench_runner.sh` : ajouter une étape qui lance
  `cargo bench -p efe-server --bench real_strategies_e2e --bench real_memory`.
* `parse_criterion_report.py` : reconnaître les nouvelles clés
  (`real_e2e_*`, `real_memory_*`).
* `BENCHMARKS.md` : nouvelle section §3 avec mesures réelles.
  Disclaimer Sprint C levé.
* `bench_baselines/v0.1.1/baseline.json` : figée et committée.
* `SLO.md` : recalibré sur mesures réelles.

---

## 8. Stratégie de mesure et per-phase profiling

### 8.1 Décomposition de la latence

L'objectif n'est pas seulement de mesurer le total, mais
**d'identifier le hot path**. Trois techniques :

1. **Per-phase isolated** : lance le pipeline avec **une seule**
   strategy réelle et mocks pour les autres. Mesure le coût marginal
   de cette strategy.

2. **Full-stack avec instrumentation** : utilise un
   `TracingTelemetryCollector` de test qui enregistre la durée de
   chaque phase. Émet ensuite `target/bench-phase-breakdown.json`.

3. **Subtraction** : `t(real_full) - t(real_full_minus_one)` donne
   le coût de la strategy retirée. Utilisé pour cross-checker la
   méthode 1.

### 8.2 Ratio attendu

Théorie a priori sur spec medium (32 agents, dim=256, 8 rounds) :

```
                     proportion attendue
genesis (1×)         ~5-10 %
fusion (8×)          ~70-85 %
consensus (8×)       ~2-5 %
dissolution (1×)     ~1-3 %
alignment (1×)       ~1-3 %
controller overhead  ~1 %
```

Si la mesure D1.4 montre une distribution radicalement différente,
**c'est un bug** (probablement un kernel inefficace dans
`fusion::kernels`).

### 8.3 Variance attendue

Avec compute réel sous criterion, la variance sera **plus élevée**
qu'avec les mocks :

| Type | CV (coefficient variation) typique |
|------|:----------------------------------:|
| Mocks (avant D1) | < 1 % (rien à varier) |
| Real strategies (D1) | 3-10 % (cache, scheduler) |
| Real + concurrency | 10-25 % (contention) |

Le seuil de régression CI sera relevé en conséquence : passe de
10 % à **20 %** pour `real_*` benches.

---

## 9. Stratégie de validation du bench lui-même

### 9.1 Le bench ne doit PAS produire de résultats trompeurs

Trois invariants :

* **I1** : `latency_real_small > 50 µs` (preuve qu'on mesure un
  pipeline réel, pas un no-op)
* **I2** : `latency_real_medium > latency_real_small` (preuve que
  la taille a un effet — un bug pourrait masquer ça)
* **I3** : `heap_peak_real_medium > 10 KB` (preuve que les
  allocations arena sont réelles)

Codé dans `bench_realism_check.rs` (D1.6).

### 9.2 Comparaison avec les µ-benchs

Une vérification croisée : la somme des µ-benchs `compute_sprint{2,3}`
appelés par le pipeline doit être cohérente avec la mesure E2E.

```
Σ(matmul + softmax + ... ) ≈ fusion_full_round_real
fusion_full_round_real × 8 + autres ≈ swarm_e2e_real_medium_latency
```

À ±50 % près (overhead controller + adapters).

### 9.3 Réplicabilité

Run le bench 3 fois consécutives (cold + warm + warm). Les chiffres
doivent converger à ±10 % entre runs warm.

---

## 10. Plan jour-par-jour (1 j)

| Slot | Phase | Livrable |
|------|-------|----------|
| 09:00-09:30 | D1.1 | Cargo.toml dev-deps + 2 bench targets `efe-server` |
| 09:30-10:00 | D1.2 | `gen_fixtures_v2.py` + 3 nouveaux fichiers JSON |
| 10:00-11:30 | D1.3 | `efe-server/benches/common/mod.rs` |
| 11:30-12:30 | D1.4a | `real_strategies_e2e.rs::bench_real_e2e_latency` (3 sizes) |
| 12:30-13:00 | déjeuner | |
| 13:00-14:00 | D1.4b | `bench_real_per_phase` (5 phases isolées) |
| 14:00-14:30 | D1.4c | `bench_real_e2e_throughput` + `bench_real_blas_vs_naive` |
| 14:30-15:30 | D1.5 | `real_memory.rs` |
| 15:30-16:00 | D1.6 | `bench_realism_check.rs` (3 tests anti-régression) |
| 16:00-17:00 | D1.7 | `bench_runner.sh` + `BENCHMARKS.md` + baseline v0.1.1 + `SLO.md` recalibré |
| 17:00-18:00 | buffer | clippy + tests + git commit |

---

## 11. Risques et mitigations

| # | Risque | P | Impact | Mitigation |
|---|--------|:-:|:------:|------------|
| R1 | `build_mvp_backend` retourne une erreur sur defaults | Faible | Élevé | Test smoke `build_mvp_backend_succeeds_with_defaults` existe déjà, à vérifier en D1.1 |
| R2 | Bench timing dépasse 30 min en CI (large + Nc=16) | Moyen | Moyen | Skip `large` × Nc en CI, garder en local seulement |
| R3 | `peak_alloc` interfère avec le pool real | Moyen | Moyen | Isoler dans `real_memory.rs` séparé (pattern Sprint C R2) |
| R4 | `Mode1Config::default` consensus trop strict → ConvergenceTimeout | Moyen | Élevé | Pré-vérifier avec `realism_check_medium_p95_under_200ms` ; sinon overrider `consensus.confidence_threshold` |
| R5 | Variance > 25 % sur `large` (cache effects) | Élevé | Faible | Documenter dans BENCHMARKS.md ; large ≈ « guideline », pas SLO strict |
| R6 | Mesure `genesis` isolée nécessite ELS pré-existant | Moyen | Moyen | Construire un `MockController` qui appelle juste `genesis.spawn`; pas trivial |
| R7 | Embedding réaliste change les chiffres SLO | Moyen | Faible | C'est attendu — on RECALIBRE les SLO en D1.7 |
| R8 | `efe-server` n'a pas `criterion` en deps | Faible | Faible | D1.1 l'ajoute |

---

## 12. Annexes

### 12.1 Annexe A — Sample bench code

```rust
// crates/efe-server/benches/real_strategies_e2e.rs
//
// Sprint D1 — End-to-end benchmarks against the production wiring.

use std::sync::Arc;
use std::time::Duration;

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};
use efe_core::types::SwarmSpec;
use efe_server::services::ExecuteBackend;

mod common;
use common::{build_real_backend, build_real_request, load_realistic_v2_spec};

fn bench_real_e2e_latency(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("rt");

    for spec_name in ["small", "medium", "large"] {
        let spec = load_realistic_v2_spec(spec_name);
        let backend = build_real_backend(spec.clone());
        let mut group = c.benchmark_group(format!("real_e2e_{spec_name}_latency"));
        let (samples, secs) = match spec_name {
            "small" => (40, 30),
            "medium" => (30, 45),
            "large" => (15, 60),
            _ => unreachable!(),
        };
        group.sample_size(samples);
        group.measurement_time(Duration::from_secs(secs));
        group.bench_function("execute", |b| {
            let backend = backend.clone();
            let s = spec.clone();
            b.to_async(&rt).iter(|| {
                let backend = backend.clone();
                let s = s.clone();
                async move {
                    backend
                        .execute(black_box(build_real_request(&s)))
                        .await
                        .expect("execute ok")
                }
            });
        });
        group.finish();
    }
}

criterion_group!(
    name = benches;
    config = Criterion::default()
        .significance_level(0.05)
        .noise_threshold(0.20);   // 20% threshold for real measurements
    targets = bench_real_e2e_latency
);
criterion_main!(benches);
```

### 12.2 Annexe B — Liste exhaustive des fichiers

```
NEW
─────────────────────────────────────────────────────────────────────
crates/efe-server/benches/real_strategies_e2e.rs        ~500 LOC
crates/efe-server/benches/real_memory.rs                ~200 LOC
crates/efe-server/benches/common/mod.rs                 ~250 LOC
crates/efe-server/tests/bench_realism_check.rs          ~100 LOC
scripts/gen_fixtures_v2.py                              ~80 LOC
scripts/fixtures/realistic_small_v2.json                ~700 LOC
scripts/fixtures/realistic_medium_v2.json               ~2400 LOC
scripts/fixtures/realistic_large_v2.json                ~4500 LOC
bench_baselines/v0.1.1/baseline.json                    (auto)

MODIFIÉS
─────────────────────────────────────────────────────────────────────
crates/efe-server/Cargo.toml                            +20 LOC
scripts/bench_runner.sh                                 +15 LOC
scripts/parse_criterion_report.py                       +40 LOC
BENCHMARKS.md                                           +80 LOC
SLO.md                                                  ~10 LOC recalibrage
.github/workflows/bench.yml                             +20 LOC
```

**Total** : ~9 000 LOC nouveaux + ~190 LOC modifiés.

### 12.3 Annexe C — Décisions de design

| # | Décision | Alternative rejetée | Raison |
|---|----------|---------------------|--------|
| D1 | Bench dans `efe-server/benches/` | `efe-core/benches/` avec wiring dupliqué | Pas de duplication, suit production |
| D2 | Réutiliser `build_mvp_backend` | Wiring custom adapté au bench | Le bench DOIT mesurer ce que le client achète |
| D3 | Embeddings Gaussiens seedés | Embeddings random | Reproductibilité + non-dégénérescence du meta_analyzer |
| D4 | Fixtures `_v2` séparées | Modifier `_v1` | Préserve la rétrocompat des benchs Sprint C |
| D5 | `peak_alloc` dans `real_memory.rs` séparé | Tout dans 1 fichier | Évite l'interférence avec timing du e2e |
| D6 | CI threshold 20 % pour `real_*` | 10 % comme Sprint C | Variance plus élevée avec compute réel |
| D7 | Per-phase via mocks pour les autres | Tracing instrumentation | Plus simple, mesure différentielle propre |
| D8 | Test runtime `> 100 µs` | Test `> 1 ms` | 100 µs assez pour distinguer mocks (3 µs) sans flakiness |
| D9 | Baseline `v0.1.1` (pas `v0.2.0`) | Bumper minor | D1 est un patch sur v0.1, pas un palier majeur |
| D10 | Skip `large × Nc=16` en CI | Mesurer tout en CI | 30+ min CI inacceptable |

### 12.4 Annexe D — Checklist livraison

* [ ] `cargo bench -p efe-server --bench real_strategies_e2e --quick` produit des chiffres > 50 µs sur small
* [ ] `cargo bench -p efe-server --bench real_memory --quick` produit heap > 10 KB sur medium
* [ ] `cargo test -p efe-server --test bench_realism_check` 3/3 verts
* [ ] `bench_runner.sh` lance les 4 benchs (2 efe-core + 2 efe-server)
* [ ] `BENCHMARKS.md` v0.1.1 publié avec section « real strategies »
* [ ] `SLO.md` recalibré (les targets restent 50ms/200ms/1s mais sont maintenant DÉFENDABLES par mesure)
* [ ] `bench_baselines/v0.1.1/baseline.json` committée
* [ ] `bench.yml` lance `efe-server` benchs aussi
* [ ] `cargo clippy --all-targets -- -D warnings` 0 warning
* [ ] Disclaimer Sprint C levé dans `BENCHMARKS.md`

---

## Conclusion — pourquoi cette architecture

Cette architecture optimise simultanément **trois axes** :

1. **Fidélité** : le bench utilise `build_mvp_backend` — la même
   factory que le serveur Mode 1 utilise pour servir les vrais
   clients. Aucune divergence entre « ce qu'on bench » et « ce
   qu'on vend ».

2. **Décomposition** : per-phase isolées + e2e + mémoire = on
   identifie où va le temps. Sans cette décomposition, optimiser
   pour Phase 2 CUDA serait à l'aveugle.

3. **Anti-régression durable** : 3 tests `bench_realism_check.rs`
   verrouillent que personne ne peut, même par accident, ramener
   le bench aux mocks (gain 3 µs apparent, perte de toute
   signification).

À l'issue de ce sprint d'1 jour :

* Bench réaliste **× 1 000** plus précis (3 ms au lieu de 3 µs)
* `BENCHMARKS.md` v0.1.1 publié avec mesures **réelles**
* `SLO.md` chiffré **défendable** auprès d'un client
* CI bloque toute régression > 20 % sur le pipeline réel
* La Phase 2 CUDA peut promettre un speedup chiffré

— *Fin du document Architecture D1.*

---

*Pour démarrer l'implémentation, voir le commit suivant qui ouvre
D1.1 avec les dev-deps `efe-server` + bench targets.*
