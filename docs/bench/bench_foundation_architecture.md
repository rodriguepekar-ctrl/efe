# Sprint Bench Foundation — Architecture complète, ultra-détaillée

---

**Document** : spécification d'architecture
**Sprint** : C — Bench Foundation
**Durée cible** : 3-5 jours (4.75 j prévus, ~0.25 j buffer)
**Auteur** : architecture & design — `rodrigue.pekar@icloud.com`
**Dépend de** : Audit `swarm/` + `streams/` (✅ livré),
`docs/audits/swarm_streams_audit_report.md`,
ROADMAP `crates/efe-core/src/compute/ROADMAP.md`,
`docs/compute/tolerances.md`
**Consommé par** : Phase 2 CUDA (référence baseline CPU),
Mode 1 / `efe-server` (SLO publiés), `BENCHMARKS.md` externe
**Statut** : spec gelée pour implémentation

> **Mission** : transformer les **17 micro-benchmarks** d'opérations
> compute isolées en **véritable benchmark publishable** : un rapport
> reproductible, comparable et automatiquement régressionné qui
> couvre l'expérience utilisateur (requête → réponse via
> `SwarmController`) sous 3 axes (latence p50/p95/p99, débit,
> mémoire) sur 3 tailles d'essaim. À l'issue du sprint, la commande
> `cargo bench --bench swarm_e2e` produit un `BENCHMARKS.md`
> verrouillé par CI et la baseline `v0.1.0` figée.

---

## Table des matières

1. [Mission, justification et critères de sortie](#1-mission-justification-et-critères-de-sortie)
2. [État AS-IS — infrastructure courante](#2-état-as-is--infrastructure-courante)
3. [État TO-BE — vision cible](#3-état-to-be--vision-cible)
4. [Trois axes de mesure](#4-trois-axes-de-mesure)
5. [Trois tailles d'essaim](#5-trois-tailles-dessaim)
6. [Contrats SLO chiffrés](#6-contrats-slo-chiffrés)
7. [Plan en 9 phases (C1–C9)](#7-plan-en-9-phases-c1c9)
8. [Spécification fichier-par-fichier](#8-spécification-fichier-par-fichier)
9. [Plan jour-par-jour](#9-plan-jour-par-jour)
10. [Stratégie de validation du benchmark lui-même](#10-stratégie-de-validation-du-benchmark-lui-même)
11. [Reproductibilité et métadonnées](#11-reproductibilité-et-métadonnées)
12. [CI et détection de régression](#12-ci-et-détection-de-régression)
13. [Risques et mitigations](#13-risques-et-mitigations)
14. [Préparation Phase 2 CUDA](#14-préparation-phase-2-cuda)
15. [Annexes](#15-annexes)

---

## 1. Mission, justification et critères de sortie

### 1.1 Mission

Délivrer une **suite de benchmarks de fondation** qui permet de
publier des chiffres de performance EFE :

* **Reproductibles** : pour `(commit, host, dataset, seed)` identique,
  chiffres identiques à ±5 %.
* **Comparables** : entre versions, entre backends (naïf ↔ BLAS ↔
  CUDA), entre tailles d'essaim.
* **Régressionnables** : un commit qui dégrade la perf de plus de
  10 % sur un benchmark de référence est bloqué en CI.
* **Publishables** : un fichier `BENCHMARKS.md` mis à jour
  automatiquement à chaque release, lisible par un client externe.

### 1.2 Justification (5 raisons)

1. **L'audit `swarm/streams/` a fermé les pré-requis durs**
   (cf. `swarm_streams_audit_report.md` §11). Pas de deadlock,
   pas de fuite, pas de variance non-contrôlée. La condition
   nécessaire pour bencher fiablement est satisfaite.
2. **Les 17 micro-benchs existants ne reflètent pas l'expérience
   utilisateur**. Une requête utilisateur traverse `genesis →
   fusion → consensus → dissolution`, pas un appel matmul isolé.
3. **Sans baseline, la Phase 2 CUDA n'a pas de référence** pour
   prouver le ×40. Le sprint Bench Foundation crée cette baseline.
4. **Sans `BENCHMARKS.md` ni `SLO.md`, on ne peut pas publier
   d'engagement de performance** auprès d'un utilisateur — la
   conformité contractuelle (RGPD / SOC 2 nécessitent souvent un
   SLO documenté).
5. **Sans CI bench, chaque PR introduit silencieusement de la
   dette de perf**. Sprint 1-5 a accumulé ~33 000 LOC sans une
   seule mesure de régression.

### 1.3 Critères de sortie (12)

| # | Critère | Mesure |
|---|---------|:------:|
| C1 | Bench `swarm_e2e` couvre 3 tailles (small / medium / large) | `cargo bench --bench swarm_e2e` exécute 3 groupes |
| C2 | Latence p50/p95/p99 mesurée et publiée | criterion histogram + table dans `BENCHMARKS.md` |
| C3 | Débit (req/s) mesuré sous concurrence | groupe dédié `swarm_e2e_throughput` |
| C4 | Mémoire RSS / heap mesurée par scénario | bench `memory` via `peak_alloc` |
| C5 | Métadonnées de reproductibilité capturées | `bench-report.json` avec `host`, `cpu`, `cargo`, `rustc`, `git`, `BLAS_BACKEND`, `RUSTFLAGS` |
| C6 | `BENCHMARKS.md` initial publié avec mesure J0 | fichier au repo, commit signé |
| C7 | `SLO.md` publié avec 5 SLO chiffrés | fichier au repo |
| C8 | Baseline `v0.1.0` figée via `criterion --save-baseline` | dossier `target/criterion/v0.1.0/` versionné |
| C9 | CI workflow `bench.yml` détecte régression > 10 % | `.github/workflows/bench.yml` |
| C10 | Comparaison naïf vs BLAS sur le **pipeline** (pas op isolée) | groupe `swarm_e2e_blas_vs_naive` |
| C11 | 3 fixtures réalistes JSON disponibles | `scripts/fixtures/realistic_{small,medium,large}.json` |
| C12 | `cargo bench` ET `cargo test --release` verts | aucune régression introduite |

---

## 2. État AS-IS — infrastructure courante

### 2.1 Inventaire

| Composant | Path | LOC | Couverture |
|-----------|------|----:|------------|
| Bench compute Sprint 2 | `crates/efe-core/benches/compute_sprint2.rs` | 162 | 6 ops compute |
| Bench compute Sprint 3 | `crates/efe-core/benches/compute_sprint3.rs` | 230 | 8 ops compute |
| Bench compute Sprint 4 | `crates/efe-core/benches/compute_sprint4.rs` | 126 | 3 ops + BLAS |
| Smoke test gRPC | `scripts/smoke.sh` | 224 | post-deployment, 10 s budget |
| Fixture toy spec | `scripts/fixtures/spec.json` | 16 | dim=4, 4 agents |
| Fixture toy execute | `scripts/fixtures/execute.json` | 21 | dim=4, smoke seul |
| **TOTAL** | | **779** | 17 µ-benchmarks isolés |

### 2.2 Ce qui MANQUE

```
                                          MANQUE (TIER 1)
Bench end-to-end pipeline                   ❌ aucun
Bench mémoire RSS / heap                    ❌ aucun
Bench débit (concurrent execute)            ❌ aucun
Datasets réalistes (dim=128/256, N=8/32/128) ❌ aucun
Métadonnées reproductibilité                ❌ aucune
BENCHMARKS.md                               ❌ aucun
SLO.md                                      ❌ aucun
Baseline criterion versionnée               ❌ aucune
CI workflow bench                           ❌ aucun
Comparaison BLAS vs naïf sur pipeline       ❌ aucune
                                          MANQUE (TIER 2)
PyTorch reference comparator                ❌ aucun
PR commenter perf regression                ❌ aucun
Cross-platform CI matrix (M1 / x86)         ❌ aucun (.github bench)
```

### 2.3 Verdict

**11 gaps en TIER 1** bloquent un benchmark publié. C'est précisément
ce que ce sprint comble.

---

## 3. État TO-BE — vision cible

### 3.1 Vue d'ensemble

```
                    ┌────────────────────────────────────────┐
                    │  cargo bench --bench swarm_e2e         │
                    └──────────────────┬─────────────────────┘
                                       │
                  ┌────────────────────┼────────────────────┐
                  ▼                    ▼                    ▼
           ┌─────────────┐    ┌──────────────┐    ┌──────────────┐
           │ Latency     │    │ Throughput   │    │ Memory       │
           │ p50/p95/p99 │    │ req/s @ Nc   │    │ RSS / heap   │
           └──────┬──────┘    └──────┬───────┘    └──────┬───────┘
                  │                  │                   │
                  └──────────────────┼───────────────────┘
                                     ▼
                    ┌─────────────────────────────────┐
                    │  scripts/bench_runner.sh         │
                    │  (capture host metadata)         │
                    │  → bench-report-{ts}-{sha}.json  │
                    └────────────────┬─────────────────┘
                                     ▼
                    ┌─────────────────────────────────┐
                    │  BENCHMARKS.md (auto-généré)    │
                    │  + criterion --baseline v0.1.0  │
                    └────────────────┬─────────────────┘
                                     ▼
                    ┌─────────────────────────────────┐
                    │  CI: .github/workflows/bench.yml│
                    │  régression > 10% → fail        │
                    └─────────────────────────────────┘
```

### 3.2 Délivrables comptés

| # | Délivrable | Format | Versionné ? |
|---|------------|--------|:-----------:|
| 1 | `crates/efe-core/benches/swarm_e2e.rs` | Rust criterion | ✅ |
| 2 | `crates/efe-core/benches/memory_e2e.rs` | Rust criterion + peak_alloc | ✅ |
| 3 | `scripts/fixtures/realistic_small.json` | JSON | ✅ |
| 4 | `scripts/fixtures/realistic_medium.json` | JSON | ✅ |
| 5 | `scripts/fixtures/realistic_large.json` | JSON | ✅ |
| 6 | `scripts/bench_runner.sh` | Bash | ✅ |
| 7 | `BENCHMARKS.md` | Markdown | ✅ |
| 8 | `SLO.md` | Markdown | ✅ |
| 9 | `target/criterion/v0.1.0/` baseline figée | criterion JSON | ✅ (sous-arbre dédié) |
| 10 | `.github/workflows/bench.yml` | YAML | ✅ |
| 11 | `crates/efe-core/tests/bench_smoke_test.rs` | Rust | ✅ |
| 12 | `BENCHMARKS.md` + `bench_baselines/v0.1.0/baseline.json` (rapport standalone ⏸ deferred — clôture consolidée dans ces deux artefacts plutôt qu'un report séparé) | Markdown / JSON | ✅ |

---

## 4. Trois axes de mesure

### 4.1 Axe 1 — Latence

**Métrique** : temps mur d'une requête end-to-end via
`SwarmController::execute(req)`.

**Découpage** :
* p50 (médiane) : référence pour l'utilisateur typique
* p95 : critère SLO (cf. §6)
* p99 : queue de distribution, doit rester bornée (preuve qu'aucun
  deadlock résiduel)

**Outil** : `criterion` avec `sample_size = 100`, `measurement_time
= 30s` minimum sur les bench longs.

**Bench cibles** :
```
swarm_e2e_small_latency       (8 agents, dim=128, R=8)
swarm_e2e_medium_latency      (32 agents, dim=256, R=8)
swarm_e2e_large_latency       (128 agents, dim=256, R=8)
```

### 4.2 Axe 2 — Débit (throughput)

**Métrique** : requêtes par seconde sous concurrence soutenue.

**Découpage** :
* `Nc=1` : séquentiel (≈ 1/p50)
* `Nc=4` : parallélisme modéré (1 cœur ≈ saturation MVP CPU)
* `Nc=16` : sur-engagement (mesure backpressure)

**Outil** : `criterion::Throughput::Elements(N)` avec
`tokio::spawn` workers.

**Bench cibles** :
```
swarm_e2e_small_throughput_Nc1
swarm_e2e_small_throughput_Nc4
swarm_e2e_small_throughput_Nc16
swarm_e2e_medium_throughput_Nc1
swarm_e2e_medium_throughput_Nc4
```

### 4.3 Axe 3 — Mémoire

**Métrique** :
* **Heap peak** (allocations Rust) via `peak_alloc::PeakAlloc`
* **RSS** (système) via `getrusage` Linux ou `mach_task_self` macOS
* **Allocations cumulées** (count)

**Découpage** : par scénario (small / medium / large) + par
champ d'allocation (genesis / fusion / dissolution / total).

**Outil** : crate `peak_alloc` (~50 LOC d'overhead, fournit un
allocator wrapper compatible avec `criterion`).

**Bench cibles** :
```
memory_e2e_small_heap_peak     (cible : < 5 MB)
memory_e2e_medium_heap_peak    (cible : < 50 MB)
memory_e2e_large_heap_peak     (cible : < 500 MB)
memory_e2e_alloc_count_small   (compteur d'allocations)
```

---

## 5. Trois tailles d'essaim

### 5.1 `realistic_small.json`

Représentation : un cas d'usage Mode 1 typique, mono-utilisateur,
question simple.

```json
{
  "spec": {
    "dim_latent": 128,
    "problem_embedding": [<128 valeurs uniform [-0.1, 0.1]>],
    "population": [
      { "template_code": "na-sem", "count": 4 },
      { "template_code": "na-prag", "count": 4 }
    ],
    "max_rounds": 8,
    "alignment_cone": {
      "anchors": [<4 anchors de dim 128, orthogonaux>],
      "max_projection_distance": 0.3
    }
  },
  "decode_mode": { "text": { "max_tokens": 256 } }
}
```

**Caractéristiques** :
* 8 agents au total, dim=128
* 4 anchors d'alignement
* Latents : 8 × 128 × 4 = **4 KB** (f32)
* Anchors : 4 × 128 × 4 = **2 KB** (f32)
* Compute par round : 8×8 attention + softmax + matmul ≈ **800 µs sur M1 Pro**
* Cible latence p95 : **< 50 ms** (8 rounds + genesis + dissolution)

### 5.2 `realistic_medium.json`

Cas d'usage Mode 1 avec essaim de réflexion plus profonde.

```json
{
  "spec": {
    "dim_latent": 256,
    "population": [
      { "template_code": "na-sem", "count": 16 },
      { "template_code": "na-prag", "count": 8 },
      { "template_code": "na-meta", "count": 8 }
    ],
    "max_rounds": 8,
    "alignment_cone": {
      "anchors": [<8 anchors de dim 256>],
      "max_projection_distance": 0.25
    }
  }
}
```

**Caractéristiques** :
* 32 agents, dim=256
* 8 anchors
* Latents : 32 × 256 × 4 = **32 KB**
* Cible p95 : **< 200 ms**

### 5.3 `realistic_large.json`

Cas d'usage industriel : 128 agents, scénario stress.

```json
{
  "spec": {
    "dim_latent": 256,
    "population": [
      { "template_code": "na-sem", "count": 64 },
      { "template_code": "na-prag", "count": 32 },
      { "template_code": "na-meta", "count": 32 }
    ],
    "max_rounds": 8,
    "alignment_cone": {
      "anchors": [<16 anchors de dim 256>],
      "max_projection_distance": 0.2
    }
  }
}
```

**Caractéristiques** :
* 128 agents, dim=256
* 16 anchors
* Latents : 128 × 256 × 4 = **128 KB**
* Cible p95 : **< 1 s**

### 5.4 Tableau de synthèse

| Spec | dim | N | anchors | Latents | Anchors | Cible p95 | Cible p99 |
|------|----:|--:|--------:|--------:|--------:|----------:|----------:|
| small | 128 | 8 | 4 | 4 KB | 2 KB | **50 ms** | **100 ms** |
| medium | 256 | 32 | 8 | 32 KB | 8 KB | **200 ms** | **400 ms** |
| large | 256 | 128 | 16 | 128 KB | 16 KB | **1 s** | **2 s** |

---

## 6. Contrats SLO chiffrés

Le `SLO.md` publié engage **5 SLO** mesurables :

| SLO ID | Engagement | Cible | Mesure |
|:------:|------------|:-----:|--------|
| **SLO-1** | Latence p95 d'une requête small | **< 50 ms** | `swarm_e2e_small_latency` p95 |
| **SLO-2** | Latence p95 d'une requête medium | **< 200 ms** | `swarm_e2e_medium_latency` p95 |
| **SLO-3** | Latence p95 d'une requête large | **< 1 s** | `swarm_e2e_large_latency` p95 |
| **SLO-4** | Débit minimum sous concurrence Nc=4 | **≥ 50 req/s** small | `swarm_e2e_small_throughput_Nc4` |
| **SLO-5** | Mémoire heap peak medium | **< 50 MB** | `memory_e2e_medium_heap_peak` |

### 6.1 Convention de validation

* Chaque PR exécute `cargo bench --bench swarm_e2e` en CI.
* La baseline `v0.1.0` est consultée via
  `criterion --baseline v0.1.0`.
* **Régression > 10 %** sur un SLO bloque la PR.
* **Régression > 5 %** émet un warning (review humaine).

### 6.2 Exception

L'environnement CI peut être plus lent qu'une station locale.
**Coefficient de pondération** : on multiplie la cible par `1.5×`
sur le runner GitHub Actions (`ubuntu-latest`, 2 vCPU). Documenté
dans `SLO.md` §3.

---

## 7. Plan en 9 phases (C1–C9)

### C1 — Fixtures réalistes (0.5 jour)

**Livrables** :
* `scripts/fixtures/realistic_small.json`
* `scripts/fixtures/realistic_medium.json`
* `scripts/fixtures/realistic_large.json`
* `scripts/fixtures/README.md` (documentation des 3 specs)

**Génération** :
* Anchors : grille orthogonale `e_i` (one-hot) shifted légèrement,
  pour éviter la dégénérescence (cf. invariant I5 alignment).
* Problem embedding : zero (déterministe).
* Counts par template : mix réaliste (50% NaSem, 25% NaPrag, 25% NaMeta).

**Validation** : chaque JSON passe `SwarmSpec::validate()` via un
test unitaire dans `bench_smoke_test.rs`.

### C2 — Bench end-to-end `swarm_e2e.rs` (1 jour)

**Fichier** : `crates/efe-core/benches/swarm_e2e.rs` (~400 LOC)

**Structure** :
```
fn bench_e2e_latency(c: &mut Criterion) {
    for spec_name in ["small", "medium", "large"] {
        let spec = load_realistic_spec(spec_name);
        let controller = build_controller(&spec);
        let mut group = c.benchmark_group(format!("swarm_e2e_{spec_name}_latency"));
        group.sample_size(100);
        group.measurement_time(Duration::from_secs(30));
        group.bench_function("execute", |b| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            b.to_async(&rt).iter(|| async {
                let req = build_request(&spec);
                controller.execute(req).await.expect("ok")
            });
        });
    }
}

fn bench_e2e_throughput(c: &mut Criterion) {
    for &nc in &[1, 4, 16] {
        for spec_name in ["small", "medium"] {
            let spec = load_realistic_spec(spec_name);
            let controller = Arc::new(build_controller(&spec));
            // Use criterion::Throughput::Elements
            ...
        }
    }
}

criterion_group!(
    name = benches;
    config = Criterion::default()
        .significance_level(0.05)
        .noise_threshold(0.10);  // 10% threshold for regression
    targets = bench_e2e_latency, bench_e2e_throughput
);
```

**Mocks** : pour ne PAS bencher la lenteur de stratégies LLM réelles,
le bench utilise les mocks (`MockGenesis`, `MockFusion`, etc.) qui
exécutent leur logique compute sans I/O. Le bench mesure la **latence
du contrôleur + compute**, pas la stratégie applicative.

### C3 — Bench mémoire `memory_e2e.rs` (0.5 jour)

**Fichier** : `crates/efe-core/benches/memory_e2e.rs` (~200 LOC)

**Dépendance** : `peak_alloc = "0.3"` (à ajouter en `dev-dependencies`).

**Structure** :
```
use peak_alloc::PeakAlloc;

#[global_allocator]
static PEAK_ALLOC: PeakAlloc = PeakAlloc;

fn bench_memory_peak(c: &mut Criterion) {
    for spec_name in ["small", "medium", "large"] {
        let spec = load_realistic_spec(spec_name);
        let controller = build_controller(&spec);
        let mut group = c.benchmark_group(format!("memory_e2e_{spec_name}_heap"));
        group.bench_function("peak_kb", |b| {
            b.iter(|| {
                PEAK_ALLOC.reset_peak_usage();
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    let req = build_request(&spec);
                    controller.execute(req).await.expect("ok");
                });
                PEAK_ALLOC.peak_usage_as_kb()
            });
        });
    }
}
```

**Note** : le `#[global_allocator]` est défini PAR fichier de bench.
On ne touche pas l'allocator global du crate (il reste l'allocator
système par défaut).

### C4 — Script `bench_runner.sh` (0.5 jour)

**Fichier** : `scripts/bench_runner.sh` (~120 LOC)

**Capture metadata** :
```bash
# Host
uname -a
sysctl -n hw.model     # macOS
sysctl -n hw.ncpu
# Toolchain
rustc --version --verbose
cargo --version
# Backend
echo "BLAS_BACKEND=${BLAS_BACKEND:-none}"
echo "RUSTFLAGS=${RUSTFLAGS:-}"
# Git
git rev-parse HEAD
git status --short
git rev-list --count HEAD
# Time
date -u +%FT%TZ
```

**Sortie** : `target/bench-report-{timestamp}-{sha8}.json`
contenant les metadata + résultats criterion parsés.

**Schéma JSON** :
```json
{
  "version": "1.0",
  "timestamp": "2026-04-25T14:30:00Z",
  "git_sha": "abc1234",
  "host": { "os": "Darwin 24.0.0", "cpu": "Apple M1 Pro", "ncpu": 10, "ram_gb": 32 },
  "toolchain": {
    "rustc": "1.85.0",
    "cargo": "1.85.0",
    "blas_backend": "accelerate",
    "rustflags": "-C target-cpu=native"
  },
  "results": {
    "swarm_e2e_small_latency": { "p50_us": 12300, "p95_us": 28500, "p99_us": 41200 },
    "swarm_e2e_medium_latency": { ... },
    ...
  },
  "slo_compliance": {
    "SLO-1": { "target_us": 50000, "measured_us": 28500, "passed": true },
    ...
  }
}
```

### C5 — `SLO.md` + tests d'assertion (0.5 jour)

**Fichier 1** : `SLO.md` (~150 LOC, racine du projet)

Documente les 5 SLO chiffrés (cf. §6), leurs conditions de mesure,
les coefficients CI, et le processus de relaxation.

**Fichier 2** : `crates/efe-core/tests/bench_slo_assertions.rs` (~80 LOC)

```rust
//! Asserts that the latest bench-report.json respects every SLO.
//! Run via `cargo test --release --test bench_slo_assertions`.

#[test]
#[cfg_attr(not(feature = "slo_check"), ignore)]
fn slo_1_small_latency_p95_under_50ms() {
    let report = load_latest_bench_report();
    let p95_us = report.results["swarm_e2e_small_latency"].p95_us;
    let target_us = 50_000;
    let coeff = if is_ci_runner() { 1.5 } else { 1.0 };
    let bound = (target_us as f64 * coeff) as u64;
    assert!(p95_us <= bound, "SLO-1 violated: p95={p95_us}µs > bound={bound}µs");
}
```

### C6 — `BENCHMARKS.md` initial (0.5 jour)

**Fichier** : `BENCHMARKS.md` (~250 LOC, racine du projet)

Format type :

```markdown
# EFE Benchmarks

> Mesures publiées du référentiel `efe-core`.
> Mises à jour automatiquement par CI à chaque merge sur `main`.
> Dernière exécution : `2026-04-25` (commit `abc1234`).

## Configuration de référence

| Champ | Valeur |
|-------|--------|
| Host | Apple M1 Pro, 10 cores, 32 GB |
| OS | Darwin 24.0.0 |
| Toolchain | rustc 1.85, cargo 1.85 |
| BLAS | accelerate |
| Flags | `-C target-cpu=native` |

## Latence end-to-end

| Spec | dim | N | p50 | p95 | p99 | SLO |
|------|----:|--:|----:|----:|----:|:---:|
| small | 128 | 8 | 12.3 ms | 28.5 ms | 41.2 ms | ✅ |
| medium | 256 | 32 | 89 ms | 152 ms | 198 ms | ✅ |
| large | 256 | 128 | 480 ms | 870 ms | 1 100 ms | ✅ |

## Throughput sous concurrence

| Spec | Nc | req/s |
|------|---:|------:|
| small | 1 | 81 |
| small | 4 | 285 |
| small | 16 | 410 |
| medium | 1 | 11 |
| medium | 4 | 38 |

## Mémoire

| Spec | heap peak | allocations |
|------|----------:|------------:|
| small | 0.8 MB | 142 |
| medium | 12 MB | 583 |
| large | 78 MB | 2 100 |

## Comparaison BLAS vs naïf (medium spec)

| Backend | p50 | p95 | speedup |
|---------|----:|----:|--------:|
| pure Rust | 124 ms | 198 ms | 1.0× (réf) |
| accelerate | 89 ms | 152 ms | **1.39×** |
```

### C7 — Baseline `v0.1.0` figée (0.25 jour)

**Action** :
```bash
cargo bench --bench swarm_e2e --bench memory_e2e \
    -- --save-baseline v0.1.0
git add target/criterion/v0.1.0/  # filtré par .gitignore exception
git commit -m "feat(bench): freeze v0.1.0 baseline for regression detection"
```

**Configuration** : `.gitignore` ajoute exception sur
`target/criterion/v0.1.0/` pour le tracker.

### C8 — CI workflow `bench.yml` (0.5 jour)

**Fichier** : `.github/workflows/bench.yml` (~100 LOC)

**Triggers** : `pull_request` sur `main`, `push` sur `main`.

**Steps** :
1. Checkout
2. Setup Rust (cache toolchain + target)
3. Build release benches
4. `cargo bench --bench swarm_e2e -- --baseline v0.1.0`
5. Parser le rapport criterion
6. Comparer chaque bench à la baseline
7. Si régression > 10 % → fail CI
8. Si régression 5-10 % → comment PR avec warning
9. Upload `bench-report.json` comme artefact

**Note importante** : la CI standard sur `ubuntu-latest` est lente
et variable. Pour des chiffres stables on cible un runner self-hosted
ARM ou un dedicated GitHub runner. Pour Sprint Bench Foundation, on
documente la limite et on tolère un coefficient `1.5×`.

### C9 — Comparaison BLAS vs naïf sur pipeline (0.5 jour)

**Fichier** : `crates/efe-core/benches/swarm_e2e.rs` (extension)

Ajouter un groupe `swarm_e2e_blas_vs_naive` qui exécute le bench
medium en deux configurations :
* `cargo bench --bench swarm_e2e` (pur Rust, sans feature)
* `cargo bench --bench swarm_e2e --features accelerate` (BLAS macOS)

**Stratégie** : un seul `criterion_group` qui appelle deux variantes
du contrôleur, l'un avec `CpuBackend::new()` (pur), l'autre avec
`CpuBackend::with_features(...)` si l'API existe (sinon directement
selon la feature flag).

Le delta mesuré confirme/infirme le speedup BLAS hors matmul isolé
(le sprint S4 Light a mesuré ×40 sur matmul seul, mais le pipeline
complet inclut beaucoup de softmax / reduce / sampling qui ne
bénéficient pas de BLAS).

---

## 8. Spécification fichier-par-fichier

### 8.1 Crates / dépendances à ajouter

`crates/efe-core/Cargo.toml` :

```toml
[dev-dependencies]
peak_alloc = "0.3"

[[bench]]
name = "swarm_e2e"
harness = false

[[bench]]
name = "memory_e2e"
harness = false
```

### 8.2 Liste des nouveaux fichiers (12)

```
crates/efe-core/benches/swarm_e2e.rs           NEW [~400 LOC]
crates/efe-core/benches/memory_e2e.rs          NEW [~200 LOC]
crates/efe-core/tests/bench_smoke_test.rs      NEW [~150 LOC]
crates/efe-core/tests/bench_slo_assertions.rs  NEW [~80 LOC]
scripts/bench_runner.sh                        NEW [~120 LOC]
scripts/parse_criterion_report.py              NEW [~80 LOC]
scripts/fixtures/realistic_small.json          NEW [~50 LOC]
scripts/fixtures/realistic_medium.json         NEW [~150 LOC]
scripts/fixtures/realistic_large.json          NEW [~600 LOC]
scripts/fixtures/README.md                     NEW [~40 LOC]
.github/workflows/bench.yml                    NEW [~100 LOC]
BENCHMARKS.md                                  NEW [~250 LOC]
SLO.md                                         NEW [~150 LOC]
# bench_foundation_report.md                  ⏸ DEFERRED — clôture consolidée dans BENCHMARKS.md + bench_baselines/v0.1.0/baseline.json
```

**Total nouveaux LOC** : ~2 370 (le rapport standalone de ~400 LOC
n'a pas été produit ; sa fonction est couverte par `BENCHMARKS.md`
et la baseline `v0.1.0` figée).

### 8.3 Fichiers modifiés

```
crates/efe-core/Cargo.toml          [+10 LOC : 2 bench targets + dep]
.gitignore                          [+1 LOC : exception baseline]
README.md                           [+15 LOC : section "Performance"]
crates/efe-core/src/compute/ROADMAP.md  [+20 LOC : Sprint C livré]
```

---

## 9. Plan jour-par-jour

### Jour 1 — Fixtures + bench skeleton

| Slot | Phase | Livrable |
|------|-------|----------|
| Matin | C1 | 3 fixtures réalistes + README + test smoke |
| Après-midi | C2 (1/2) | `swarm_e2e.rs` skeleton + bench_e2e_latency small |

### Jour 2 — Bench latence + débit

| Slot | Phase | Livrable |
|------|-------|----------|
| Matin | C2 (2/2) | bench_e2e_latency medium + large + bench_e2e_throughput |
| Après-midi | C3 | `memory_e2e.rs` complet + dependency `peak_alloc` |

### Jour 3 — Reproductibilité + SLO

| Slot | Phase | Livrable |
|------|-------|----------|
| Matin | C4 | `bench_runner.sh` + `parse_criterion_report.py` |
| Après-midi | C5 | `SLO.md` + `bench_slo_assertions.rs` |

### Jour 4 — Baseline + comparaison + CI

| Slot | Phase | Livrable |
|------|-------|----------|
| Matin | C6 | `BENCHMARKS.md` initial |
| Après-midi | C7 + C8 | Baseline `v0.1.0` figée + `bench.yml` |

### Jour 5 — BLAS vs naïf + clôture

| Slot | Phase | Livrable |
|------|-------|----------|
| Matin | C9 | bench BLAS vs naïf + analyse |
| Après-midi | clôture | `BENCHMARKS.md` + `bench_baselines/v0.1.0/baseline.json` + ROADMAP (rapport standalone ⏸ deferred — clôture consolidée) |

### Buffer

5 jours nominaux × 8 h - 4.75 j × 8 h = ~2 h de buffer pour
imprévus (typiquement, un bench flaky qui demande retuning).

---

## 10. Stratégie de validation du benchmark lui-même

Un bench peut être incorrect (mesurer la mauvaise chose, biais,
variance). Il faut **valider le bench**, pas juste l'exécuter.

### 10.1 Tests de validation

`crates/efe-core/tests/bench_smoke_test.rs` :

```rust
#[test]
fn realistic_specs_validate() {
    for name in ["small", "medium", "large"] {
        let raw = std::fs::read_to_string(
            format!("scripts/fixtures/realistic_{name}.json")
        ).unwrap();
        let spec: SwarmSpec = serde_json::from_str(&raw).unwrap();
        spec.validate().expect("realistic spec must validate");
    }
}

#[tokio::test]
async fn bench_smoke_swarm_e2e_small_executes() {
    // Sanity: the same code path the bench will execute must succeed
    // at least once before we trust the bench numbers.
    let spec = load_spec("small");
    let controller = build_default_controller(&spec);
    let req = Request::for_tests(spec);
    let resp = controller.execute(req).await.expect("smoke ok");
    assert!(resp.confidence > 0.0);
}
```

### 10.2 Variance ≤ 5 % sur runs successifs

Le bench inclut un test :
```rust
#[test]
#[cfg(feature = "bench_validation")]
fn bench_e2e_small_variance_under_5_percent() {
    let measurements = run_n_iterations(50);
    let mean = mean(&measurements);
    let stddev = stddev(&measurements);
    let cv = stddev / mean;
    assert!(cv < 0.05, "CV={cv} exceeds 5% — bench is flaky");
}
```

### 10.3 Sanity contre micro-benchs

Le bench end-to-end small est **borné inférieurement** par la somme
des micro-benchs des ops compute appelées. Si le E2E small mesure
moins que la somme des µ-benchs, c'est bug (mesure vide ou stripped).

---

## 11. Reproductibilité et métadonnées

### 11.1 Champs métadonnées capturés

| Catégorie | Champ | Source |
|-----------|-------|--------|
| Host | OS, version | `uname -a` |
| Host | CPU model | `sysctl -n hw.model` (mac) / `lscpu` (linux) |
| Host | NCPU, RAM | `sysctl -n hw.ncpu`, `hw.memsize` |
| Toolchain | rustc version | `rustc --version --verbose` |
| Toolchain | target features | `rustc --print target-features --target $TARGET` |
| Toolchain | RUSTFLAGS | env `$RUSTFLAGS` |
| Build | profile | `release` |
| Build | features actives | `--features ...` |
| Build | BLAS_BACKEND | `accelerate` / `openblas` / `none` |
| Git | SHA | `git rev-parse HEAD` |
| Git | branch | `git branch --show-current` |
| Git | dirty? | `git status --porcelain` |
| Time | UTC ISO 8601 | `date -u +%FT%TZ` |
| Bench | criterion version | `Cargo.lock` |

### 11.2 Schéma JSON sortant

Voir §C4. Le schéma est versionné (`"version": "1.0"`) pour
permettre des évolutions sans casser les parsers downstream.

### 11.3 Détermination de stabilité

Un bench est **considéré stable** ssi :
1. CV (coefficient de variation) sur 50 runs < 5 %
2. p99 / p50 < 3 (queue raisonnable)
3. Aucun outlier détecté par criterion (`--noise-threshold 0.10`)

---

## 12. CI et détection de régression

### 12.1 Workflow `bench.yml`

```yaml
name: Bench
on:
  pull_request:
    branches: [main]
  push:
    branches: [main]

jobs:
  bench:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }
      - uses: dtolnay/rust-toolchain@1.85
      - uses: Swatinem/rust-cache@v2
      - name: Run benches
        run: |
          cargo bench --bench swarm_e2e -- --baseline v0.1.0 \
            --noise-threshold 0.10 --output-format bencher \
            > bench-output.txt
      - name: Parse and compare
        run: python3 scripts/parse_criterion_report.py
      - name: Check regression
        run: |
          if grep -q "REGRESSED" bench-output.txt; then
            echo "::error::Bench regression > 10% detected"
            exit 1
          fi
      - uses: actions/upload-artifact@v4
        with:
          name: bench-report-${{ github.sha }}
          path: target/bench-report-*.json
```

### 12.2 Workflow tolérance

Sur runner GHA `ubuntu-latest` (2 vCPU shared) :
* coefficient `1.5×` appliqué aux SLO
* runs répétés 3× et p95 médian retenu
* outliers > 2σ écartés automatiquement par criterion

---

## 13. Risques et mitigations

| # | Risque | P | Impact | Mitigation |
|---|--------|:-:|:------:|------------|
| R1 | CI runner trop lent / variable | Élevé | Moyen | Coefficient 1.5×, runs ×3, prefer self-hosted M1 |
| R2 | `peak_alloc` global allocator interfère avec criterion | Faible | Élevé | Bench séparé `memory_e2e.rs`, pas dans `swarm_e2e.rs` |
| R3 | Baseline `v0.1.0` rapidement obsolète | Moyen | Faible | Process documenté pour mise à jour à chaque release majeure |
| R4 | Mocks ne reflètent pas la production | Élevé | Moyen | Documenté ; mode 1 server bench séparé en Sprint suivant |
| R5 | `criterion --baseline` API change | Faible | Moyen | Pin `criterion = "0.5"` exact |
| R6 | Bench flaky sur GHA = false positives | Élevé | Élevé | Coefficient + retry, 5 % warn / 10 % fail |
| R7 | Allocations cumulées dépendantes de tokio version | Moyen | Faible | Pin `tokio` et documenter dans BENCHMARKS.md |
| R8 | `target/criterion/v0.1.0/` fichiers binaires lourds | Faible | Faible | git LFS si > 1 MB ; sinon commit standard |

---

## 14. Préparation Phase 2 CUDA

### 14.1 Réutilisation directe

Le sprint Bench Foundation produit l'infrastructure que la Phase 2
CUDA hérite :
* `swarm_e2e.rs` se lance avec `--features cuda` sans changement
* `bench_runner.sh` capture `BLAS_BACKEND=cuda` automatiquement
* Baseline `v0.1.0` (CPU) sert de référence pour le speedup CUDA

### 14.2 Métriques attendues Phase 2

Les SLO Phase 2 attendus, par dérivation des micro-benchs S4 (×40
sur matmul) :

| Spec | SLO CPU (sprint C) | SLO CUDA Phase 2 cible |
|------|:------------------:|:----------------------:|
| small | 50 ms | < 5 ms |
| medium | 200 ms | < 15 ms |
| large | 1 s | < 50 ms |

### 14.3 Le bench valide la Phase 2

À l'issue de la Phase 2 CUDA, on relance `cargo bench --bench swarm_e2e
--features cuda` et on compare à la baseline CPU. Si le speedup est
< 5×, la Phase 2 a un problème (probablement bandwidth-bound ou
dispatch overhead).

---

## 15. Annexes

### 15.1 Annexe A — Sample bench code (`swarm_e2e.rs`)

```rust
//! End-to-end benchmark of the SwarmController.
//!
//! Run via:
//!   cargo bench --bench swarm_e2e -- --baseline v0.1.0
//!
//! Three sizes: small (8 agents), medium (32), large (128).
//! Three axes: latency (p50/p95/p99), throughput (req/s), and via
//! the companion `memory_e2e.rs`, heap peak.

use std::sync::Arc;
use std::time::Duration;

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};
use efe_core::fusion::FusionStats;
use efe_core::swarm::{
    ControllerConfig, Request, SwarmArbiter, SwarmController, Verdict,
};
use efe_core::types::{LatentDim, SwarmSpec};

mod common;
use common::{build_mock_controller, load_realistic_spec};

fn bench_e2e_latency(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("rt");

    for spec_name in ["small", "medium", "large"] {
        let spec = load_realistic_spec(spec_name);
        let controller = Arc::new(build_mock_controller(&spec));
        let mut group = c.benchmark_group(format!("swarm_e2e_{spec_name}_latency"));
        group.sample_size(50);
        group.measurement_time(Duration::from_secs(20));
        group.bench_function("execute", |b| {
            let ctrl = controller.clone();
            let s = spec.clone();
            b.to_async(&rt).iter(|| {
                let ctrl = ctrl.clone();
                let s = s.clone();
                async move {
                    let req = Request::for_tests(s);
                    ctrl.execute(black_box(req))
                        .await
                        .expect("execute ok")
                }
            });
        });
        group.finish();
    }
}

fn bench_e2e_throughput(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("rt");

    for &nc in &[1usize, 4, 16] {
        for spec_name in ["small", "medium"] {
            let spec = load_realistic_spec(spec_name);
            let controller = Arc::new(build_mock_controller(&spec));
            let mut group = c.benchmark_group(format!(
                "swarm_e2e_{spec_name}_throughput_Nc{nc}"
            ));
            group.throughput(Throughput::Elements(nc as u64));
            group.sample_size(20);
            group.bench_function("batch", |b| {
                let ctrl = controller.clone();
                let s = spec.clone();
                b.to_async(&rt).iter(|| {
                    let ctrl = ctrl.clone();
                    let s = s.clone();
                    async move {
                        let mut handles = Vec::with_capacity(nc);
                        for _ in 0..nc {
                            let c2 = ctrl.clone();
                            let s2 = s.clone();
                            handles.push(tokio::spawn(async move {
                                c2.execute(Request::for_tests(s2)).await
                            }));
                        }
                        for h in handles {
                            h.await.expect("join").expect("execute");
                        }
                    }
                });
            });
            group.finish();
        }
    }
}

criterion_group!(
    name = benches;
    config = Criterion::default()
        .significance_level(0.05)
        .noise_threshold(0.10);
    targets = bench_e2e_latency, bench_e2e_throughput
);
criterion_main!(benches);
```

### 15.2 Annexe B — Sample `bench_runner.sh`

```bash
#!/usr/bin/env bash
# scripts/bench_runner.sh — capture metadata and run benches.
#
# Output:
#   target/bench-report-{timestamp}-{sha8}.json

set -euo pipefail

TS=$(date -u +%Y%m%dT%H%M%SZ)
SHA=$(git rev-parse --short=8 HEAD)
OUT="target/bench-report-${TS}-${SHA}.json"

# Capture metadata
HOST_OS=$(uname -s)
HOST_VER=$(uname -r)
HOST_NCPU=$(getconf _NPROCESSORS_ONLN)
case "$HOST_OS" in
  Darwin)
    HOST_CPU=$(sysctl -n machdep.cpu.brand_string)
    HOST_RAM_GB=$(($(sysctl -n hw.memsize) / 1024 / 1024 / 1024))
    ;;
  Linux)
    HOST_CPU=$(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2 | xargs)
    HOST_RAM_GB=$(($(grep MemTotal /proc/meminfo | awk '{print $2}') / 1024 / 1024))
    ;;
esac

RUSTC_VERSION=$(rustc --version)
CARGO_VERSION=$(cargo --version)
BLAS_BACKEND="${BLAS_BACKEND:-none}"
RUSTFLAGS="${RUSTFLAGS:-}"

# Run benches (criterion json output)
cargo bench --bench swarm_e2e --bench memory_e2e \
    -- --output-format bencher 2>&1 | tee target/bench-stdout.txt

# Parse via Python helper
python3 scripts/parse_criterion_report.py \
    --metadata-os "$HOST_OS" \
    --metadata-cpu "$HOST_CPU" \
    --metadata-ncpu "$HOST_NCPU" \
    --metadata-ram-gb "$HOST_RAM_GB" \
    --metadata-rustc "$RUSTC_VERSION" \
    --metadata-blas "$BLAS_BACKEND" \
    --metadata-sha "$SHA" \
    --metadata-timestamp "$TS" \
    --output "$OUT"

echo "Bench report written to: $OUT"
```

### 15.3 Annexe C — Décisions de design

| # | Décision | Alternative rejetée | Raison |
|---|----------|---------------------|--------|
| D1 | Mocks au lieu de stratégies réelles | Stratégies LLM | Bench mesure le contrôleur, pas le LLM |
| D2 | criterion 0.5 (déjà dans dep) | bencher / iai | Compatibilité existante |
| D3 | `peak_alloc` pour mémoire | dhat / heaptrack | Léger, pas de tooling externe |
| D4 | 3 tailles fixes au lieu de paramétré | Sweep param | Lecture / publication plus simple |
| D5 | SLO en `cargo test --release` | YAML séparé | Test = code = versionné |
| D6 | Coefficient CI 1.5× | Pas de coeff | GHA ubuntu trop variable sinon |
| D7 | Baseline `v0.1.0` versionnée dans repo | Storage externe | Auto-suffisant, pas de S3 |
| D8 | `bench_runner.sh` Bash, pas Python | Python entier | Bash + Python parser = simple, pas de venv |
| D9 | `swarm_e2e.rs` séparé de `memory_e2e.rs` | Tout dans 1 fichier | global_allocator conflict |
| D10 | Pas de PyTorch reference cette sprint | Comparaison ref | Out-of-scope ; Sprint suivant |

### 15.4 Annexe D — Checklist livraison

* [ ] 3 fixtures JSON valident
* [ ] `swarm_e2e.rs` compile + tourne sur les 3 tailles
* [ ] `memory_e2e.rs` compile + capture peak heap
* [ ] `bench_runner.sh` produit JSON valide
* [ ] `BENCHMARKS.md` rendu lisible avec mesures J0
* [ ] `SLO.md` 5 SLO chiffrés
* [ ] `target/criterion/v0.1.0/` figée et committée
* [ ] `bench.yml` exécute en < 10 min sur GHA
* [ ] `bench_smoke_test.rs` 100 % vert
* [ ] `bench_slo_assertions.rs` 100 % vert sur runner local
* [ ] Régression simulée (timer factice) → CI fail comme attendu
* [ ] ROADMAP mis à jour avec Sprint C livré

---

## Conclusion — pourquoi cette architecture

Cette architecture optimise simultanément **trois axes** :

1. **Complétude** : 3 axes (latence, débit, mémoire) × 3 tailles
   d'essaim = 9 mesures principales. Couvre 95 % des questions perf
   réalistes.

2. **Reproductibilité chirurgicale** : metadata complète, baseline
   versionnée, coefficient CI documenté. Un chiffre publié peut
   être ré-exécuté à 6 mois d'écart pour vérification.

3. **Régression-safe** : chaque PR exécute les benchs et bloque
   automatiquement à > 10 % de dérive. Évite que les Sprints
   futurs (Phase 2 CUDA, Mode 1 server) introduisent silencieusement
   des dégradations.

À l'issue des 4.75 jours :

* 17 µ-benchs préservés + **9 nouveaux benchs E2E**
* `BENCHMARKS.md` + `SLO.md` publishables
* Baseline `v0.1.0` figée
* CI workflow actif
* Zéro régression introduite (1 421 tests existants restent verts)

La Phase 2 CUDA peut alors démarrer sur une base mesurée et
verrouillée.

— *Fin du document Sprint Bench Foundation.*

---

*Pour démarrer l'implémentation, voir le commit suivant qui ouvre
la Phase C1 avec les 3 fixtures réalistes.*
