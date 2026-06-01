# `efe-bench` — Architecture détaillée de l'extension harness perf (M6)

> **Statut** : Draft v1.0.0 — Document de référence pour l'achèvement
> du module **M6 = `efe-bench`** entre l'AS-IS (post-D18 audit, 46
> tests + 6 workloads + 7 SLOs D18 + `MockBenchClient` only) et la
> cible **GA 1.0** (8 workloads + 1 Mock + 1 Tonic + `--gpu-shards`
> + ~70 tests + pre-prod gating opérationnel).
>
> **Auteur** : rodrigue.pekar@icloud.com
> **Date** : 2026-04-29.
> **Dépendances doc** :
>   * `docs/PRODUCT_COMPLETION_ARCHITECTURE.md` — roadmap globale
>     Φ-A→Φ-G.
>   * `docs/bench/d18_multi_replica_bench_d12_architecture.md` — architecture
>     bench D18 + 6 workloads canoniques + 7 SLOs.
>   * `docs/EFE_KERNELS_ARCHITECTURE.md` — substrat GPU CUDA (D31
>     multi-GPU).
>   * `docs/EFE_SERVER_ARCHITECTURE.md` — extension gRPC M4 (D32
>     streaming, D34 Mode-2, D37 Mode-3).
>   * `docs/EFE_CORE_EXTENSION_ARCHITECTURE.md` — extension M1 (D33
>     templates, D34 Mode-2 backend, D37/D38 Mode-3 + parallel).
>   * `docs/EFE_TELEMETRY_EXTENSION_ARCHITECTURE.md` — extension M5
>     (instruments + dashboards consommés par les SLO gates).
>
> **Portée** : 3 sprints non-contigus s'étalant sur **3 phases**
> (Φ-D / Φ-D / Φ-F). Architecture **complémentaire** aux extensions
> M1/M2/M4/M5 livrées en parallèle :
>
>   * **D29** (Φ-D) — `TonicBenchClient` (vrai client gRPC,
>     remplace `MockBenchClient` comme default ; `--allow-mock`
>     reste valid mais devient minoritaire).
>   * **D31** (Φ-D) — `--gpu-shards N` flag pour orchestrer un
>     bench multi-GPU.
>   * **D38** (Φ-F) — Mode-3 workload patterns (`mode3-k5`,
>     `mode3-k10`).
>
> **Cible GA** : **8 workloads + 2 clients (Mock + Tonic) + 7 SLOs
> + GPU sharding orchestration + pre-prod gating opérationnel**.

---

## Table des matières

§0. Méta, glossaire, conventions
§1. Mission de l'extension M6 (Φ-D → Φ-F)
§2. Position dans la topologie EFE post-M5
§3. État AS-IS au 2026-04-29 (post-D18 + post-M4 audit + post-M5 audit)
§4. Définition opérationnelle de M6 v1.0 GA
§5. Principes directeurs
§6. Invariants structurels
§7. Vue d'ensemble — 3 sprints, dependency graph
§8. Surface publique cible (8 workloads + 2 clients + flags)
§9. **Sprint D29 — `TonicBenchClient` (vrai gRPC)**
§10. **Sprint D31 — `--gpu-shards N` flag (multi-GPU bench)**
§11. **Sprint D38 — Mode-3 workloads (`mode3-k5`, `mode3-k10`)**
§12. Pre-prod gating workflow (post-D38)
§13. SLO mapping (M5 instruments → bench gates)
§14. Tests + pin tests structurels
§15. Critères Go/No-Go par sprint
§16. Risques transverses + mitigations
§17. Volumétrie code attendue
§18. Plans d'implémentation détaillés
§19. Annexes — CLI manuel post-D38
§20. Annexes — JSON output schema (BenchReport)
§21. Annexes — example CI/CD pipelines
§22. Annexes — RACI + budget
§23. Annexes — Gantt prévisionnel D29 → D38
§24. Mises à jour du document

---

## §0. Méta, glossaire, conventions

### §0.1 Conventions

* **AS-IS** = état du code à fin D18 + post-audit, v0.1.19+M5-audit-1.
* **TO-BE** = état cible défini par chaque sprint M6 extension.
* Les sprints D29, D31, D38 sont **non-contigus** : des sprints
  intercalaires (D30 SDK Go, D32 streaming, D33 templates, D34
  Mode-2, D35-D36 LLM routers, D37 Mode-3) ne touchent pas
  `efe-bench` directement.
* Les défauts résiduels sont notés **DT-B.x** (Defect Trace,
  Bench).
* **Workload** = un mix d'opérations gRPC (e.g. 95 % Execute +
  5 % RegisterCatalog) avec une RPS cible, une durée, et une
  concurrence.
* **SLO gate** = un seuil que le bench doit valider pour qu'un
  build soit accepté en pre-prod (e.g. p99 ≤ 200 ms).
* **Tonic** = la stack gRPC Rust officielle (cf. M4 server).
* **CI/CD pipeline gate** = un job CI qui exécute `efe-bench` et
  exit code != 0 si une SLO est violée.
* `[ref:fichier:ligne]` = pointeur dans le code source.

### §0.2 Glossaire

| Terme | Définition |
|-------|------------|
| **`efe-bench`** | Le crate Rust `crates/efe-bench/` qui héberge le harness performance + pre-prod gating. CLI `efe-bench` consomme un endpoint gRPC `efe-server`, applique un workload, et émet un report JSON avec les latences mesurées. |
| **`MockBenchClient`** | Implémentation `BenchClient` actuelle (D18) qui répond instantanément sans réseau. Utile pour valider l'harness lui-même mais **ne mesure PAS le vrai stack**. |
| **`TonicBenchClient`** | Nouvelle implémentation D29 qui ouvre un vrai canal gRPC vers `efe-server` via `tonic::transport::Channel`. Les latences mesurées reflètent le full stack (TLS + Auth + Rate-limit + handler + backend). |
| **`--allow-mock`** | Flag CLI déjà présent (D18) qui permet de lancer un bench avec MockBenchClient. Default `false` post-D29 → l'opérateur doit explicitement choisir Mock pour les bench harness self-tests. |
| **`--gpu-shards N`** | Nouveau flag D31 : le bench orchestre **N requêtes parallèles** ciblées sur **N GPUs distincts** via le label `gpu_shard_id` propagé jusqu'à `efe-kernels::CudaBackend`. Permet de mesurer le speedup multi-GPU. |
| **Workload pattern** | Une distribution d'opérations gRPC. AS-IS 6 patterns D18 ; TO-BE 8 (D18 + Mode-3 k=5 et k=10). |
| **`mode3-k5` / `mode3-k10`** | Workloads Mode-3 D38 : k=5 (default) ou k=10 (stress test, requiert opt-in `--allow-k-above-max`) pour valider SLO C-9 (parallel speedup ≥ 2.5×) et SLO C-3 (Mode-3 p99 ≤ 3 × Mode-1 p99). |
| **Pre-prod gate** | Étape CI/CD entre staging et prod où `efe-bench` exécute les 8 workloads + valide les 7 SLOs. Si UN seul SLO échoue, le build est REJETÉ. |
| **`BenchReport`** | Sortie JSON canonique avec `(workload, latency_p50, p95, p99, throughput, success_rate, slo_passed: bool)` par opération. Versionnée avec un schema_version pour anti-régression. |

### §0.3 Cible matérielle

* **CI runner** (Tier-1) — bench unitaires + workload smoke (1 CPU, 4 GiB RAM).
* **Pre-prod gate runner** (Tier-2) — bench complet 6+2 workloads (8 vCPU, 16 GiB RAM, optionnel 1× T4 GPU).
* **Multi-GPU runner** (Tier-3) — bench `--gpu-shards N` (8 vCPU, 32 GiB, 4× A10 GPUs).

### §0.4 Cibles fonctionnelles GA

```
GA v1.0 efe-bench (M6) :
├── Clients (2 total)
│   ├── MockBenchClient                              (existant D18)
│   └── TonicBenchClient                             (D29, NEW)
│
├── Workload patterns (8 total)
│   ├── read-heavy           (existant D18)
│   ├── write-heavy          (existant D18)
│   ├── balanced             (existant D18)
│   ├── burst                (existant D18)
│   ├── multi-tenant         (existant D18)
│   ├── soak                 (existant D18)
│   ├── mode3-k5             (D38, NEW)
│   └── mode3-k10            (D38, NEW)
│
├── CLI flags (canonical post-D38)
│   ├── --kind <workload>    (existant D18)
│   ├── --endpoint <addr>    (existant D18)
│   ├── --duration <secs>    (existant D18)
│   ├── --concurrency <n>    (existant D18)
│   ├── --target-rps <n>     (existant D18)
│   ├── --tenant <id>        (existant D18)
│   ├── --output <path>      (existant D18)
│   ├── --slo-check          (existant D18)
│   ├── --allow-mock         (existant D18, default false)
│   ├── --jwt-token <token>  (D29, NEW — ATTACHED to Tonic only)
│   ├── --tls-ca-file <p>    (D29, NEW — mTLS support)
│   ├── --gpu-shards <N>     (D31, NEW — orchestration multi-GPU)
│   └── --k-swarm <K>        (D38, NEW — pour mode3-k* workloads)
│
└── SLO gates (7 total)
    ├── D18-RPS-1   read-heavy ≥ 200 RPS               (existant D18)
    ├── D18-RPS-2   write-heavy ≥ 50 RPS               (existant D18)
    ├── D18-LAT-1   p99 read ≤ 50 ms                   (existant D18)
    ├── D18-LAT-2   p99 write ≤ 200 ms                 (existant D18)
    ├── D18-LAT-3   p99 burst ≤ 500 ms                 (existant D18)
    ├── D18-MT-1    multi-tenant fairness  ≥ 0.95      (existant D18)
    └── D18-SOAK-1  10-min soak no degradation         (existant D18)

    NB : SLO C-3 (Mode-3 p99 ≤ 3 × Mode-1 p99) et C-9 (parallel speedup ≥
    2.5×) sont VALIDÉES par les workloads `mode3-k5` (D38) en
    contraste avec `read-heavy` (Mode-1 baseline). Pas un SLO
    bench-natif mais un check dérivé via PromQL post-bench
    (cf. §13).
```

---

## §1. Mission de l'extension M6 (Φ-D → Φ-F)

### §1.1 Énoncé opérationnel

L'extension M6 a **trois missions** :

1. **Connecter le bench au vrai stack gRPC** (D29) : remplacer le
   `MockBenchClient` par un `TonicBenchClient` qui exécute des
   appels réseau réels vers `efe-server`. Sans ce client, les
   latences mesurées sont **factices** (Mock répond en quelques
   µs) et les SLOs latence ne valident rien de la production.

2. **Permettre l'orchestration multi-GPU** (D31) : ajouter un
   flag `--gpu-shards N` qui dispatche les requêtes sur N GPUs
   distincts via un label `gpu_shard_id` propagé jusqu'à
   `efe-kernels::CudaBackend`. Sans ce flag, le bench ne peut
   pas valider les SLOs perf des sprints D31+ (multi-GPU
   orchestration).

3. **Livrer les workloads Mode-3** (D38) : 2 nouveaux patterns
   `mode3-k5` et `mode3-k10` qui exécutent `Mode3Service.Execute`
   avec k sub-swarms parallèles. Sans ces workloads, les SLOs
   C-3 (Mode-3 p99 ≤ 3 × Mode-1 p99) et C-9 (parallel speedup
   ≥ 2.5×) sont **invérifiables en pre-prod gate**.

### §1.2 Périmètre strict du document

Ce document **NE traite PAS** :

* Les benchs Criterion (`criterion::Bencher`) — ils mesurent des
  fonctions Rust isolées (e.g. fusion kernels), couverts par
  `crates/efe-core/benches/` et `crates/efe-server/benches/`.
* Les benchs accuracy (golden datasets) — couverts par
  `eval/scripts/run_eval*.py`.
* L'observabilité du bench lui-même — l'observabilité système
  est M5 (`efe-telemetry`) ; le bench émet son propre report
  JSON, pas des métriques Prometheus.
* Les outils de monitoring (Grafana, alertes) — couverts par
  M5 extension.

### §1.3 Mission complémentaire — pre-prod gating

L'extension M6 ouvre la voie à la **CI/CD pre-prod gating** :

```yaml
# .github/workflows/pre-prod-gate.yml (cible post-D38)
jobs:
  bench-pre-prod:
    runs-on: tier-2-runner
    steps:
      - run: cargo build -p efe-bench --release
      - run: ./efe-server &  # boot serveur staging
      - run: |
          for kind in read-heavy write-heavy balanced burst multi-tenant soak; do
            ./target/release/efe-bench run \
              --endpoint $STAGING_ADDR \
              --jwt-token $TOKEN \
              --kind $kind \
              --duration 60 \
              --slo-check \
              --output report-$kind.json
          done
          # Mode-3 stress (D38)
          ./target/release/efe-bench run --kind mode3-k5 --slo-check ...
      - run: ./scripts/aggregate-bench-reports.sh report-*.json
        # exit 1 si une SLO échoue
```

Sans le `TonicBenchClient` (D29), ce gate n'est PAS opérable :
le `MockBenchClient` ne mesure pas le vrai stack.

---

## §2. Position dans la topologie EFE post-M5

### §2.1 Graph de dépendances post-D38

```text
             ┌─────────────────────────────────────────┐
             │       CI/CD Pipeline (GitHub Actions)   │
             │  ┌──────────────────┐  ┌──────────────┐ │
             │  │ pre-prod-gate.yml│  │ release.yml  │ │
             │  └────────┬─────────┘  └──────┬───────┘ │
             └───────────┼────────────────────┼────────┘
                         │                    │
                         ▼                    ▼
              ┌──────────────────────────────────────┐
              │       efe-bench CLI (M6)             │
              │  bench run --kind <X> --endpoint <Y> │
              └──────────────┬───────────────────────┘
                             │
                             │ TonicBenchClient (D29)
                             │ + --gpu-shards N (D31)
                             │ + Mode-3 workloads (D38)
                             ▼
              ┌──────────────────────────────────────┐
              │      efe-server (M4) — staging       │
              │ Mode1Service / Mode2Service /        │
              │ Mode3Service / CatalogService /      │
              │ AdminService                         │
              └──────────────┬───────────────────────┘
                             │
                             ▼
              ┌──────────────────────────────────────┐
              │   efe-core (M1) + efe-kernels (M2)   │
              │   SwarmController, Mode2Controller,  │
              │   Mode3Controller, CudaBackend       │
              └──────────────────────────────────────┘
```

### §2.2 Direction stricte des dépendances

* `efe-bench` dépend d'**`efe-core`** (pour les types `Request`,
  `Response`, etc. — déjà présent en D18).
* `efe-bench` dépend de **`tonic`** (D29 — nouveau) pour le
  client gRPC.
* `efe-bench` **NE dépend PAS** d'`efe-server` (le bench est un
  client externe, pas un module serveur).
* `efe-bench` **NE dépend PAS** d'`efe-telemetry` (le bench
  consomme `BenchReport` JSON, pas Prometheus).

### §2.3 Position vs `efe-server` (M4)

| Surface | efe-bench (M6, post-D38) | efe-server (M4) |
|---------|:------------------------:|:---------------:|
| Workloads | ✅ déclarés + 8 patterns | ❌ — consomme via gRPC |
| TonicBenchClient | ✅ (D29) | ❌ — répond aux requêtes |
| --gpu-shards | ✅ (D31) | ❌ — propage le label |
| Mode-3 workloads | ✅ (D38) | ❌ — consomme via gRPC |
| BenchReport JSON | ✅ versionné | ❌ |
| SLO gates | ✅ check post-bench | ❌ — déclare les SLOs |

---

## §3. État AS-IS au 2026-04-29 (post-D18 + post-M4 audit + post-M5 audit)

### §3.1 Inventaire physique efe-bench

```
crates/efe-bench/src/
├── bin/
│   └── efe_bench.rs              # CLI (clap derive)
├── client.rs                     # BenchClient trait + MockBenchClient
├── error.rs                      # BenchError taxonomy
├── gen.rs                        # Workload generator (random ops)
├── lib.rs                        # public surface
├── metrics.rs                    # latency histograms + throughput
├── report.rs                     # BenchReport JSON output
├── runner.rs                     # workload executor (concurrency)
└── workload.rs                   # WorkloadOp + 6 D18 patterns

crates/efe-bench/tests/
└── e2e.rs                        # 7 e2e tests

Total : ~2 639 LOC + 46 unit tests + 7 e2e
```

### §3.2 Workloads canoniques AS-IS (D18)

```rust
// crates/efe-bench/src/workload.rs (AS-IS)

pub enum WorkloadOp {
    Execute,           // efe.v1.SwarmService.Execute
    RegisterCatalog,   // efe.v1.CatalogService.RegisterCatalog
    RegisterClass,
    RegisterSentence,
    ResolveCatalog,
    ListCatalogs,
}

impl Workload {
    pub fn canonical(label: &str) -> Result<Self, BenchError> {
        match label {
            "read-heavy"   => /* 95% Execute, 5% RegisterCatalog */,
            "write-heavy"  => /* 30% Execute, 70% Register* */,
            "balanced"     => /* 50/50 */,
            "burst"        => /* spike pattern */,
            "multi-tenant" => /* round-robin tenant_ids */,
            "soak"         => /* 10-min sustained */,
            _ => Err(BenchError::UnknownWorkload(...)),
        }
    }

    pub fn canonical_labels() -> impl Iterator<Item = &'static str> {
        ["read-heavy", "write-heavy", "balanced",
         "burst", "multi-tenant", "soak"].into_iter()
    }
}
```

### §3.3 Client AS-IS

```rust
// crates/efe-bench/src/client.rs (AS-IS)

pub trait BenchClient: Send + Sync + std::fmt::Debug {
    async fn execute(&self, op: WorkloadOp, tenant: &str) -> Result<Latency, BenchError>;
}

pub struct MockBenchClient {
    /* ... canned responses, optional fail_every, error injection ... */
}

impl BenchClient for MockBenchClient {
    async fn execute(&self, _op: WorkloadOp, _tenant: &str) -> Result<Latency, BenchError> {
        // Returns instant latency (~ µs) + canned outcome
    }
}

// Note doc-comment :
//   "would ship a TonicBenchClient impl that wraps the
//    real gRPC stack ; pre-D29 we ship MockBenchClient
//    only for testing"
```

### §3.4 CLI AS-IS

```bash
# CLI flags D18
efe-bench run \
  --kind <read-heavy|write-heavy|balanced|burst|multi-tenant|soak> \
  --endpoint 127.0.0.1:50051 \
  --duration 60 \
  --concurrency 32 \
  --target-rps 200 \
  --tenant acme \
  --output report.json \
  --slo-check \
  --allow-mock     # default false ; bench refuses to run without Tonic
                   # post-D29, but D18 has no Tonic so --allow-mock
                   # is REQUIRED for any actual D18 run.
```

### §3.5 Tests AS-IS

* **46 unit tests Rust** (cargo test -p efe-bench --lib).
* **7 e2e tests** (`tests/e2e.rs`) qui boot le serveur en mode mock
  + lance des workloads + valide le report JSON.

### §3.6 Synthèse AS-IS

| Sprint | État AS-IS | Gap | Effort |
|---|---|---|---|
| D29 TonicBenchClient | 0% (Mock only) | TonicBenchClient + tonic dep + JWT auth + mTLS option | 6 jours |
| D31 --gpu-shards | 0% (no flag) | Flag + label propagation + bench harness multi-shard | 4 jours |
| D38 Mode-3 workloads | 0% (no Mode3 op) | WorkloadOp::Mode3Execute + 2 patterns + k_swarm flag | 5 jours |

Total : ~15 jours-ingénieur. ~1 200 LOC à livrer.

---

## §4. Définition opérationnelle de `efe-bench` v1.0 GA

### §4.1 Le contrat GA

À la fin de l'extension M6 (post-D38) :

* **2 clients** (Mock + Tonic) avec `--allow-mock` qui rend Mock
  minoritaire (default = Tonic post-D29).
* **8 workload patterns** (6 D18 + 2 Mode-3).
* **`--gpu-shards N`** flag pour orchestration multi-GPU.
* **`--k-swarm K`** flag pour les workloads `mode3-k*`.
* **JWT + mTLS** support côté Tonic client (parité avec
  `efe-server` D22).
* **BenchReport JSON v2** avec `client_kind` (`mock` | `tonic`)
  + `gpu_shards` + `k_swarm` fields (rétro-compatible v1).
* **Pre-prod gating** : pipeline CI exécute les 8 workloads en
  ~10 min ; rejet de build sur SLO violation.

### §4.2 Versioning produit

| efe-bench version | Sprint | Clients | Workloads | Flags | Tests |
|-------------------|---|---|--:|---|--:|
| 0.1.x (D18 livré) | D18 | Mock | 6 | base | 46 |
| 0.2.x (D29 livré) | D29 | Mock + Tonic | 6 | +JWT, +mTLS | 56 |
| 0.3.x (D31 livré) | D31 | Mock + Tonic | 6 | +gpu-shards | 60 |
| 0.4.x (D38 livré) | D38 | Mock + Tonic | 8 | +k-swarm | 70+ |
| **1.0.0 GA** | post-D38 | Mock + Tonic | 8 | full | 70+ |

### §4.3 Gating entre versions

Chaque bump mineur exige :

* Cargo.toml workspace.version bumpé.
* Pin tests Python par sprint (`scripts/tests/test_d{29,31,38}_bench_*.py`).
* JSON schema rétro-compatible (anti-régression).
* CHANGELOG.md entry avec audit fix table si applicable.

---

## §5. Principes directeurs

### §5.1 Principes opposables en revue PR

| ID | Principe | Application |
|----|---|---|
| **B-1** | Default Tonic, Mock opt-in | Post-D29, `efe-bench run` sans `--allow-mock` requiert le TonicBenchClient. Mock devient explicite. |
| **B-2** | JSON output rétro-compatible | Tout nouveau field au `BenchReport` est `Option<T>` ou a un default ; les anciens consommateurs continuent de parser. |
| **B-3** | Workload mix immuable | Les ratios `mix: Vec<(WorkloadOp, u32)>` des 6 D18 workloads restent gelés (anti-régression sur les SLOs). Nouveaux workloads = nouveaux labels. |
| **B-4** | Latency = client-side | Le bench mesure depuis l'envoi de la requête côté client jusqu'à la réception. INCLUT TLS handshake + serde + auth. C'est la métrique cliente, pas la métrique server-side. |
| **B-5** | SLO gate fail-fast | Sur la première violation SLO, le CLI exit non-0 avec un message explicite. Pas de "best effort" silencieux. |
| **B-6** | Multi-tenant fairness via Jain index | Le SLO `D18-MT-1` calcule l'index de Jain sur les RPS par tenant ; ≥ 0.95 = équitable. Anti-régression sur la formule. |
| **B-7** | GPU sharding orchestration only | `efe-bench` propage le label `gpu_shard_id` mais ne **dispatch pas** les GPUs lui-même — c'est `efe-server` + `efe-kernels` qui l'utilisent. Le bench mesure le résultat. |
| **B-8** | Determinism via seed | Tout workload accepte `--seed N` qui rend la séquence d'ops reproductible (anti-régression sur les SLOs). |

### §5.2 Ce que `efe-bench` refuse d'implémenter

* **Bench sans seed** — refusé : la non-déterminisme rend les
  SLOs invérifiables.
* **Bench accuracy** — délégué à `eval/scripts/run_eval*.py`.
* **Bench unit/criterion** — délégué à `crates/efe-*/benches/`.
* **Bench OS/network simulation** (chaos eng) — hors-scope.

---

## §6. Invariants structurels

### §6.1 Invariants de structure de fichiers

```
crates/efe-bench/src/
├── bin/efe_bench.rs                   (extension D29 + D31 + D38)
├── client.rs                          (NEW : pub mod tonic_client D29)
├── client/                            (NEW : sous-dossier post-D29)
│   ├── mod.rs                         (re-exports BenchClient + MockBenchClient + TonicBenchClient)
│   ├── mock.rs                        (deplacement de l'existant)
│   └── tonic_client.rs                (NEW D29)
├── error.rs                           (extension : +TonicError variant)
├── gen.rs                             (extension : +Mode3 op generator D38)
├── lib.rs                             (extension : re-exports D29/D38)
├── metrics.rs                         (extension : +gpu_shard label D31)
├── report.rs                          (extension : +client_kind, gpu_shards, k_swarm fields D29/D31/D38)
├── runner.rs                          (extension : +shard orchestration D31, +k_swarm propagation D38)
└── workload.rs                        (extension : +Mode3Execute op, +mode3-k5 / mode3-k10 patterns D38)

crates/efe-bench/tests/
├── e2e.rs                             (extension : tests Tonic + GPU shards + Mode-3)
└── tonic_integration.rs               (NEW D29 — boot real server, benchmark, validate)
```

### §6.2 Invariants de runtime

* **TonicBenchClient ne fail pas silencieusement** — toute erreur
  réseau (Connect refused, TLS handshake fail, JWT invalid)
  produit `BenchError::TonicError(...)` typé.
* **--gpu-shards N validation** — N ∈ [1, 16] (cap matériel) ;
  N=0 → erreur.
* **--k-swarm K validation** — K ∈ [1, 10] avec opt-in
  `--allow-k-above-max` requis pour K > 5.
* **Workload mix sum > 0** — refus si tous les weights sont 0.

### §6.3 Invariants de versioning JSON

```json
{
  "schema_version": 2,           // bumped from 1 in D29
  "client_kind": "tonic",        // NEW D29
  "gpu_shards": 2,               // NEW D31, Option<u32>
  "k_swarm": 5,                  // NEW D38, Option<u32>
  "workload_label": "mode3-k5",
  "operations": [
    {
      "op": "mode3_execute",      // NEW D38
      "count": 1234,
      "p50_ms": 12.3,
      "p99_ms": 89.5,
      "throughput_rps": 23.4,
      "success_rate": 0.998
    },
    /* ... */
  ],
  "slos": {
    "D18-RPS-1": { "target": 200, "actual": 245, "passed": true },
    /* ... */
  },
  "duration_seconds": 60,
  "concurrency": 32,
  "target_rps": 200,
  "tenant": "acme",
  "timestamp_unix_ms": 1735689600000
}
```

* **`schema_version`** monotone : v1 (D18) → v2 (D29 ajout
  `client_kind`).
* **Champs ajoutés** sont `Option<T>` — anciens parsers
  retournent `None` au lieu d'erreur.
* **Renommage** = breaking change → bump major.

---

## §7. Vue d'ensemble — 3 sprints, dependency graph

### §7.1 Tableau d'agrégation

| Sprint | Phase | Durée | Livrable principal | LOC | Pin tests | e2e |
|--------|---|---|---|--:|--:|---|
| **D29** | Φ-D | 6 j | `TonicBenchClient` + JWT/mTLS + JSON schema v2 | ~600 | 12 | 3 |
| **D31** | Φ-D | 4 j | `--gpu-shards N` flag + shard orchestration | ~250 | 6 | 1 |
| **D38** | Φ-F | 5 j | Mode-3 workloads (`mode3-k5`, `mode3-k10`) + `--k-swarm` flag | ~350 | 10 | 2 |

Total : **~1 200 LOC + 28 pin tests + 6 e2e**.

### §7.2 Dépendances inter-sprints

```text
                D29 (TonicBenchClient)
                     │
                     │ requires : tonic 0.x dep (workspace already)
                     │ requires : efe-server staging endpoint
                     │
                D31 (--gpu-shards)
                     │
                     │ requires : D29 TonicBenchClient (le label
                     │ gpu_shard_id passe via gRPC metadata)
                     │ requires : efe-server propagate to efe-kernels
                     │
                D38 (Mode-3 workloads)
                     │
                     │ requires : M4 D37 Mode3Service côté server
                     │ requires : M1 D37/D38 Mode3Controller côté core
                     │ requires : D29 TonicBenchClient (Mock ne
                     │ supporte pas Mode-3 RPC sémantiquement)
                     │
                     ▼
                M6 GA v1.0
```

### §7.3 Volumétrie code attendue

```
D29 TonicBenchClient :
  client/tonic_client.rs (NEW)            → ~280 LOC
  client/mod.rs (NEW, déplacement)         → ~30 LOC
  error.rs (+TonicError variant)           → +50 LOC
  bin/efe_bench.rs (+--jwt-token, +--tls-ca-file, default Tonic) → +80 LOC
  report.rs (+client_kind field, schema v2)  → +40 LOC
  Cargo.toml (+tonic 0.12 + tonic dep workspace)  → +10 LOC
  tests/tonic_integration.rs (NEW)          → ~150 LOC
  Total                                    → ~640 LOC

D31 --gpu-shards :
  bin/efe_bench.rs (+--gpu-shards flag)    → +30 LOC
  runner.rs (shard orchestration)          → +120 LOC
  metrics.rs (+gpu_shard label)            → +40 LOC
  report.rs (+gpu_shards field)            → +20 LOC
  tests/e2e.rs (+1 test)                   → +50 LOC
  Total                                    → ~260 LOC

D38 Mode-3 workloads :
  workload.rs (+Mode3Execute op, +2 patterns) → +120 LOC
  gen.rs (+Mode3 op generator)             → +60 LOC
  client/tonic_client.rs (+Mode3 dispatch) → +80 LOC
  bin/efe_bench.rs (+--k-swarm flag, +mode3-k* recognition) → +40 LOC
  report.rs (+k_swarm field)               → +20 LOC
  tests/e2e.rs (+2 tests)                  → +100 LOC
  Total                                    → ~420 LOC
```

Net : **~1 320 LOC** au-dessus de la cible architecturale.

---

## §8. Surface publique cible (8 workloads + 2 clients + flags)

### §8.1 Inventaire post-D38 (GA)

```rust
// crates/efe-bench/src/lib.rs (post-D38)

pub mod client;     // re-export BenchClient + Mock + Tonic
pub mod error;
pub mod gen;
pub mod metrics;
pub mod report;
pub mod runner;
pub mod workload;

pub use client::{BenchClient, MockBenchClient, TonicBenchClient};
pub use error::{BenchError, TonicError};
pub use report::{BenchReport, OperationReport, SloOutcome};
pub use workload::{Workload, WorkloadOp};
```

### §8.2 Mapping consommateur → surface

| Consommateur | Surface principale | Sprint |
|---|---|---|
| `efe-bench` CLI | `efe_bench::*` re-exports | D18+D29+D31+D38 |
| CI/CD pre-prod gate | `efe-bench` binary + JSON report | D29+ |
| Dev local profiling | `efe-bench run --kind <X>` | D18+ |
| Multi-GPU benchmark suite | `--gpu-shards N` | D31+ |
| Mode-3 perf validation | `--kind mode3-k5 --slo-check` | D38+ |

---

## §9. Sprint **D29** — `TonicBenchClient` (vrai gRPC)

### §9.1 Objectif

Livrer un `TonicBenchClient` qui ouvre un canal gRPC réel vers
`efe-server` via `tonic::transport::Channel`, avec support JWT
+ mTLS optionnel. Devient le client par défaut ; le `MockBenchClient`
reste accessible via `--allow-mock` (rétro-compat).

### §9.2 Architecture du module

```text
crates/efe-bench/src/client/
├── mod.rs                 # pub trait BenchClient + re-exports
├── mock.rs                # MockBenchClient (déplacement de l'existant)
└── tonic_client.rs        # TonicBenchClient (NEW)
```

### §9.3 Trait `BenchClient` (rappel)

```rust
// crates/efe-bench/src/client/mod.rs (D29)

#[async_trait]
pub trait BenchClient: Send + Sync + std::fmt::Debug {
    async fn execute(
        &self,
        op: WorkloadOp,
        tenant: &str,
    ) -> Result<Latency, BenchError>;

    /// Optional : the kind of client (for the JSON report).
    fn kind(&self) -> ClientKind;
}

pub enum ClientKind {
    Mock,
    Tonic,
}
```

### §9.4 `TonicBenchClient` impl

```rust
// crates/efe-bench/src/client/tonic_client.rs (NEW D29)

use std::sync::Arc;
use std::time::Instant;

use tonic::transport::{Channel, ClientTlsConfig};
use tonic::metadata::MetadataValue;
use tonic::Request;
use async_trait::async_trait;

use crate::error::{BenchError, TonicError};
use crate::workload::WorkloadOp;
use crate::client::{BenchClient, ClientKind, Latency};

/// **Sprint D29** — Tonic-backed BenchClient.
///
/// Owns a `Channel` (cheap clone) and 5 service stubs (one per
/// gRPC service consumed by the bench harness). Each `execute`
/// call dispatches on `op`, encodes the proto request, sends via
/// the corresponding stub, and times the round-trip.
///
/// ## Auth
///
/// JWT token is propagated via `authorization: Bearer <token>`
/// metadata on every request. mTLS is opt-in via
/// `ClientTlsConfig::with_native_roots()` + client cert.
pub struct TonicBenchClient {
    channel: Channel,
    jwt_token: Option<MetadataValue<tonic::metadata::Ascii>>,
    /// Pre-built stubs (cloned per call, cheap).
    swarm_stub: SwarmServiceClient<Channel>,
    catalog_stub: CatalogServiceClient<Channel>,
    /// (D38) Mode-3 stub.
    mode3_stub: Option<Mode3ServiceClient<Channel>>,
}

impl TonicBenchClient {
    /// Build a new client. Connects synchronously (blocking).
    pub async fn connect(config: TonicConfig) -> Result<Self, BenchError> {
        let mut endpoint = Channel::from_shared(config.endpoint.clone())
            .map_err(|e| BenchError::TonicError(TonicError::InvalidEndpoint(e.to_string())))?;
        if let Some(tls) = config.tls {
            endpoint = endpoint.tls_config(tls).map_err(TonicError::from)?;
        }
        let channel = endpoint.connect().await.map_err(TonicError::from)?;

        let jwt_token = match config.jwt_token {
            Some(t) => {
                let v = MetadataValue::try_from(format!("Bearer {t}"))
                    .map_err(|e| BenchError::TonicError(
                        TonicError::InvalidJwtToken(e.to_string())
                    ))?;
                Some(v)
            }
            None => None,
        };

        Ok(Self {
            swarm_stub: SwarmServiceClient::new(channel.clone()),
            catalog_stub: CatalogServiceClient::new(channel.clone()),
            mode3_stub: Some(Mode3ServiceClient::new(channel.clone())),
            channel,
            jwt_token,
        })
    }

    /// Inject the bearer token into a Request<T>.
    fn with_auth<T>(&self, mut req: Request<T>) -> Request<T> {
        if let Some(tok) = &self.jwt_token {
            req.metadata_mut().insert("authorization", tok.clone());
        }
        req
    }
}

#[async_trait]
impl BenchClient for TonicBenchClient {
    async fn execute(&self, op: WorkloadOp, tenant: &str) -> Result<Latency, BenchError> {
        let started = Instant::now();
        let outcome = match op {
            WorkloadOp::Execute => self.dispatch_execute(tenant).await,
            WorkloadOp::RegisterCatalog => self.dispatch_register_catalog(tenant).await,
            WorkloadOp::RegisterClass => self.dispatch_register_class(tenant).await,
            WorkloadOp::RegisterSentence => self.dispatch_register_sentence(tenant).await,
            WorkloadOp::ResolveCatalog => self.dispatch_resolve_catalog(tenant).await,
            WorkloadOp::ListCatalogs => self.dispatch_list_catalogs(tenant).await,
            // D38 — Mode-3 dispatch.
            WorkloadOp::Mode3Execute { k_swarm } => {
                self.dispatch_mode3_execute(tenant, k_swarm).await
            }
        };
        let elapsed = started.elapsed();
        match outcome {
            Ok(()) => Ok(Latency { duration: elapsed, success: true }),
            Err(e) => {
                tracing::warn!(
                    target: "efe.bench.tonic",
                    op = ?op,
                    tenant,
                    error = %e,
                    "request failed"
                );
                Ok(Latency { duration: elapsed, success: false })
            }
        }
    }

    fn kind(&self) -> ClientKind {
        ClientKind::Tonic
    }
}
```

### §9.5 `TonicConfig` builder

```rust
pub struct TonicConfig {
    pub endpoint: String,                        // "http://127.0.0.1:50051"
    pub jwt_token: Option<String>,
    pub tls: Option<ClientTlsConfig>,            // None = plain h2c
}

impl TonicConfig {
    pub fn from_args(args: &Cli) -> Result<Self, BenchError> {
        let mut tls = None;
        if let Some(ca_path) = &args.tls_ca_file {
            let ca_pem = std::fs::read(ca_path).map_err(BenchError::Io)?;
            tls = Some(ClientTlsConfig::new()
                .ca_certificate(tonic::transport::Certificate::from_pem(ca_pem)));
        }
        Ok(Self {
            endpoint: args.endpoint.clone(),
            jwt_token: args.jwt_token.clone(),
            tls,
        })
    }
}
```

### §9.6 CLI extension

```bash
# CLI flags D29
efe-bench run \
  --kind read-heavy \
  --endpoint http://staging.efe.ai:50051 \
  --jwt-token $TOKEN                  # NEW D29
  --tls-ca-file /etc/ssl/efe-ca.pem   # NEW D29 — optional mTLS
  --duration 60 \
  --slo-check \
  --output report.json

  # `--allow-mock` reste valid mais devient minoritaire ;
  # absence de --allow-mock = TonicBenchClient par défaut.
```

### §9.7 Tests + pin tests D29

* **Unit tests** (8) :
  * `tonic_client_kind_returns_tonic`.
  * `tonic_config_from_args_with_jwt`.
  * `tonic_config_from_args_with_tls_ca`.
  * `tonic_config_endpoint_validation`.
  * `report_v2_includes_client_kind`.
  * `mock_kind_returns_mock` (rétro-compat).
  * `tonic_dispatch_execute_records_latency`.
  * `tonic_auth_header_propagated`.
* **Pin tests Python** (`scripts/tests/test_d29_bench_tonic.py`,
  12 tests) :
  * Module structure (`client/tonic_client.rs` exists).
  * `TonicBenchClient` declared with `pub struct` + `impl BenchClient`.
  * Default client = Tonic (parse CLI without --allow-mock).
  * JSON schema v2 (`client_kind` field present).
  * `--jwt-token` and `--tls-ca-file` flags present.
  * Cargo.toml has tonic 0.12 dep.
* **Integration test** (`tests/tonic_integration.rs`, 3 tests) :
  Spawn a real `efe-server` mock, connect Tonic client, run a
  small workload, validate the report.

### §9.8 Critères Go/No-Go D29

* GG-D29.1 : `TonicBenchClient` GREEN sur 8 unit tests.
* GG-D29.2 : `read-heavy` workload runnable contre staging réel
  via `efe-bench run --endpoint <staging>`.
* GG-D29.3 : JSON report v2 backward-compatible (anciens
  parsers v1 ne plantent pas).
* GG-D29.4 : Pin tests Python `test_d29_bench_tonic.py` GREEN.
* GG-D29.5 : v0.2.x tag.

---

## §10. Sprint **D31** — `--gpu-shards N` flag (multi-GPU bench)

### §10.1 Objectif

Ajouter le flag `--gpu-shards N` qui :
1. Valide N ∈ [1, 16] (ou erreur).
2. Propage le label `gpu_shard_id` (= idx % N) via gRPC metadata
   `x-efe-gpu-shard-id: <idx>` sur chaque requête.
3. `efe-server` lit la metadata et la propage à `efe-kernels`
   pour dispatcher sur le GPU correspondant.
4. Le bench mesure le speedup multi-GPU via les latences par
   shard.

### §10.2 Architecture

```text
crates/efe-bench/src/
├── bin/efe_bench.rs    (extension : +--gpu-shards N flag)
├── runner.rs            (extension : assign shard_id per worker)
├── metrics.rs           (extension : per-shard histograms)
└── report.rs            (extension : gpu_shards Option<u32>)
```

### §10.3 Détail du flag

```rust
// crates/efe-bench/src/bin/efe_bench.rs (PATCH D31)

#[derive(Parser)]
struct RunArgs {
    /* existant : --kind, --endpoint, etc. */

    /// **Sprint D31** — Number of GPU shards to orchestrate.
    /// Each worker thread is assigned `shard_id = worker_idx % gpu_shards`
    /// and propagates it via the `x-efe-gpu-shard-id` gRPC metadata.
    /// `efe-server` consumes the header and routes the request
    /// to `efe-kernels::CudaBackend` on the corresponding GPU.
    ///
    /// Default 1 = no sharding (single GPU).
    /// Max 16 = matches the largest deployed cluster (4× H100 nodes).
    #[arg(long, default_value = "1", value_parser = clap::value_parser!(u32).range(1..=16))]
    gpu_shards: u32,
}
```

### §10.4 Runner shard assignment

```rust
// crates/efe-bench/src/runner.rs (PATCH D31)

impl Runner {
    pub async fn run(&self, args: &RunArgs) -> Result<BenchReport, BenchError> {
        let shard_count = args.gpu_shards;
        let mut tasks = Vec::with_capacity(args.concurrency as usize);
        for worker_idx in 0..args.concurrency {
            let shard_id = worker_idx % shard_count;
            let client = self.client.clone();
            let workload = self.workload.clone();
            tasks.push(tokio::spawn(async move {
                worker_loop(client, workload, shard_id /* propagate */).await
            }));
        }
        // ... join all + aggregate per-shard metrics.
    }
}
```

### §10.5 Tests + pin tests D31

* **Unit tests** (4) : flag parse, shard_id assignment, range
  validation [1..16], header propagation.
* **Pin tests Python** (`test_d31_bench_gpu_shards.py`, 6 tests) :
  flag exists, default 1, max 16, header `x-efe-gpu-shard-id`
  documented, JSON report `gpu_shards` field present.
* **Integration test** (`tests/e2e.rs`, +1 test) : spawn 4
  shards, validate per-shard throughput is roughly equal
  (Jain index ≥ 0.95).

### §10.6 Critères Go/No-Go D31

* GG-D31.1 : 4 unit tests GREEN.
* GG-D31.2 : `--gpu-shards 4` produit un report avec
  per-shard latency.
* GG-D31.3 : Multi-GPU speedup mesuré ≥ 0.7 × N
  (e.g. 4 shards = 2.8× minimum).
* GG-D31.4 : Pin tests `test_d31_bench_gpu_shards.py` GREEN.
* GG-D31.5 : v0.3.x tag.

---

## §11. Sprint **D38** — Mode-3 workloads (`mode3-k5`, `mode3-k10`)

### §11.1 Objectif

Livrer 2 nouveaux workloads Mode-3 qui exécutent
`efe.v1.mode3.Mode3Service.Execute` avec `k=5` (default) ou
`k=10` (stress test, opt-in `--allow-k-above-max`).

### §11.2 WorkloadOp extension

```rust
// crates/efe-bench/src/workload.rs (PATCH D38)

pub enum WorkloadOp {
    Execute,
    RegisterCatalog,
    RegisterClass,
    RegisterSentence,
    ResolveCatalog,
    ListCatalogs,
    /// **Sprint D38** — Mode-3 fan-out execute.
    Mode3Execute {
        /// Number of parallel sub-swarms. Validated against
        /// K_MAX = 5 by `Mode3Service` server-side ; the bench
        /// can request k=10 with `--allow-k-above-max` for
        /// stress testing the clamping path.
        k_swarm: u32,
    },
}

impl WorkloadOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            /* existing 6 */
            Self::Mode3Execute { k_swarm: 5 }  => "mode3_execute_k5",
            Self::Mode3Execute { k_swarm: 10 } => "mode3_execute_k10",
            Self::Mode3Execute { k_swarm: _ }  => "mode3_execute",
        }
    }
}
```

### §11.3 Workload patterns

```rust
// crates/efe-bench/src/workload.rs (PATCH D38)

impl Workload {
    pub fn canonical(label: &str) -> Result<Self, BenchError> {
        match label {
            /* existing 6 */
            "mode3-k5" => Ok(Self {
                label: "mode3-k5".into(),
                mix: vec![(WorkloadOp::Mode3Execute { k_swarm: 5 }, 100)],
                target_rps: 50,        // Mode-3 is heavier than Mode-1
                duration: Duration::from_secs(60),
                concurrency: 16,
            }),
            "mode3-k10" => Ok(Self {
                label: "mode3-k10".into(),
                mix: vec![(WorkloadOp::Mode3Execute { k_swarm: 10 }, 100)],
                target_rps: 25,        // k=10 doubles the GPU pressure
                duration: Duration::from_secs(30),
                concurrency: 8,
            }),
            _ => Err(BenchError::UnknownWorkload(label.into())),
        }
    }

    pub fn canonical_labels() -> impl Iterator<Item = &'static str> {
        ["read-heavy", "write-heavy", "balanced",
         "burst", "multi-tenant", "soak",
         "mode3-k5", "mode3-k10",  // NEW D38
        ].into_iter()
    }
}
```

### §11.4 CLI extension

```bash
# CLI flags D38
efe-bench run \
  --kind mode3-k5 \                # 100% Mode3Execute k=5
  --endpoint http://staging.efe.ai:50051 \
  --jwt-token $TOKEN \
  --duration 60 \
  --slo-check \
  --output report.json

# k=10 stress test (above K_MAX=5 server-side, requires --allow-k-above-max
# OR the workload `mode3-k10` which sets it implicitly).
efe-bench run --kind mode3-k10 ...
```

### §11.5 Tonic dispatch

```rust
// crates/efe-bench/src/client/tonic_client.rs (PATCH D38)

impl TonicBenchClient {
    async fn dispatch_mode3_execute(
        &self,
        tenant: &str,
        k_swarm: u32,
    ) -> Result<(), BenchError> {
        let stub = self.mode3_stub
            .clone()
            .ok_or(BenchError::TonicError(TonicError::Mode3StubMissing))?;
        let req = Mode3ExecuteRequest {
            query_text: format!("bench query {}", uuid::Uuid::new_v4()),
            k: k_swarm,
            mode_per_call: "mode1".to_string(),
            aggregator: "mean".to_string(),
            tenant_id: tenant.to_string(),
            request_id: String::new(),
            seed: 0,
            top_k_anchors: 5,
            max_rounds: 0,
            llm_router_id: "mock".to_string(),
        };
        let mut req = self.with_auth(Request::new(req));
        // Propagate gpu_shard_id from worker scope.
        // ... (D31 wiring)
        let _ = stub.execute(req).await.map_err(TonicError::from)?;
        Ok(())
    }
}
```

### §11.6 Tests + pin tests D38

* **Unit tests** (6) :
  * `mode3_k5_workload_constructs`.
  * `mode3_k10_workload_constructs`.
  * `mode3_op_as_str_known_values`.
  * `canonical_labels_post_d38_count_8`.
  * `mode3_default_k_swarm_is_5`.
  * `mode3_above_k_max_requires_opt_in`.
* **Pin tests Python** (`test_d38_bench_mode3.py`, 10 tests) :
  workloads list, op enum, k-swarm flag, BenchReport
  `k_swarm` field, dispatch via Tonic.
* **Integration test** (`tests/e2e.rs`, +2 tests) : run
  `mode3-k5` against a stub server, validate dispersion +
  total_ms.

### §11.7 Critères Go/No-Go D38

* GG-D38.1 : 6 unit tests GREEN.
* GG-D38.2 : `mode3-k5` runnable et SLO C-3
  (Mode-3 p99 ≤ 3 × Mode-1 p99) checkable post-bench via
  PromQL.
* GG-D38.3 : `mode3-k10` opt-in flag présent + clamp
  server-side observable via `efe_mode3_k_clamped_total`.
* GG-D38.4 : Pin tests Python GREEN.
* GG-D38.5 : v0.4.0 GA tag.

---

## §12. Pre-prod gating workflow (post-D38)

### §12.1 Pipeline CI/CD canonique

```yaml
# .github/workflows/pre-prod-gate.yml
name: Pre-prod Gate
on:
  push:
    branches: [main]
  pull_request:
    types: [opened, synchronize]

jobs:
  bench-pre-prod:
    runs-on: tier-2-runner          # 8 vCPU + 16 GiB
    timeout-minutes: 15
    steps:
      - uses: actions/checkout@v4
      - name: Build efe-bench + efe-server
        run: |
          cargo build --release -p efe-bench
          cargo build --release -p efe-server
      - name: Boot staging server
        run: |
          ./target/release/efe-server &
          echo $! > server.pid
          sleep 5
      - name: Run bench suite
        run: |
          for kind in read-heavy write-heavy balanced burst multi-tenant; do
            ./target/release/efe-bench run \
              --kind "$kind" \
              --endpoint http://127.0.0.1:50051 \
              --jwt-token "$TEST_TOKEN" \
              --duration 60 \
              --slo-check \
              --output "report-$kind.json" \
              || exit 1
          done
          # Mode-3 stress (D38)
          ./target/release/efe-bench run --kind mode3-k5 \
            --endpoint http://127.0.0.1:50051 \
            --jwt-token "$TEST_TOKEN" \
            --slo-check --output report-mode3-k5.json || exit 1
      - name: Aggregate reports
        run: ./scripts/bench/aggregate_reports.py report-*.json
      - name: Cleanup
        if: always()
        run: kill $(cat server.pid) || true
        # Each --slo-check returns non-zero on violation,
        # the `|| exit 1` propagates failure to GitHub status.
```

### §12.2 Workflow par sprint

* **Pre-D29** : pipeline ne peut PAS exister (Mock seul).
* **Post-D29** : pipeline opérable mais incomplet (pas de
  Mode-3, pas de multi-GPU).
* **Post-D38** : pipeline complet — 8 workloads, SLO gates,
  pre-prod block sur violation.

---

## §13. SLO mapping (M5 instruments → bench gates)

### §13.1 Inventaire post-D38

| SLO | Source | Target | Bench workload |
|-----|---|---|---|
| **D18-RPS-1** | bench-native | `read-heavy ≥ 200 RPS` | read-heavy |
| **D18-RPS-2** | bench-native | `write-heavy ≥ 50 RPS` | write-heavy |
| **D18-LAT-1** | bench-native | `read p99 ≤ 50 ms` | read-heavy |
| **D18-LAT-2** | bench-native | `write p99 ≤ 200 ms` | write-heavy |
| **D18-LAT-3** | bench-native | `burst p99 ≤ 500 ms` | burst |
| **D18-MT-1** | bench-native | `Jain ≥ 0.95` | multi-tenant |
| **D18-SOAK-1** | bench-native | `no degradation 10 min` | soak |
| **C-3** (M5) | Prometheus | `Mode3 p99 ≤ 3 × Mode1 p99` | mode3-k5 vs read-heavy |
| **C-9** (M5) | Prometheus | `parallel speedup ≥ 2.5×` | mode3-k5 vs sequential |
| **D31-GPU-1** | bench-native | `multi-GPU speedup ≥ 0.7 × N` | --gpu-shards |

---

## §14. Tests + pin tests structurels

### §14.1 Inventaire post-D38

| Sprint | Unit tests Rust | Pin tests Python | e2e |
|---|--:|--:|---|
| D29 | 8 | 12 | 3 |
| D31 | 4 | 6 | 1 |
| D38 | 6 | 10 | 2 |
| **Total** | **18** | **28** | **6** |

Cumul AS-IS + extension : 46 + 18 = **64 unit tests** + 7 + 6 = **13 e2e**.

### §14.2 Pin tests institutionnels

```python
# scripts/tests/test_m6_bench_extension_invariants.py
def test_m6_default_client_is_tonic():
    """Pre-D29 the bench refuses to run without --allow-mock.
    Post-D29, default = Tonic ; --allow-mock is an opt-in
    minority case."""
    rs = read("crates/efe-bench/src/bin/efe_bench.rs")
    # Default client construction in the runner must select Tonic
    # when --allow-mock is absent.
    assert "TonicBenchClient::connect" in rs

def test_m6_workloads_count_8():
    rs = read("crates/efe-bench/src/workload.rs")
    body = rs[rs.find("canonical_labels"):rs.find("\n}\n", rs.find("canonical_labels"))]
    assert "mode3-k5" in body
    assert "mode3-k10" in body
    # Existing 6 preserved.
    for k in ["read-heavy", "write-heavy", "balanced",
              "burst", "multi-tenant", "soak"]:
        assert k in body
```

---

## §15. Critères Go/No-Go par sprint

| Sprint | Critère principal | Métrique | Cible |
|---|---|---|---|
| D29 | TonicBenchClient runnable contre staging | bench `read-heavy` | report v2 OK |
| D29 | JSON schema rétro-compat | parser v1 sur v2 | no panic |
| D31 | multi-GPU speedup | bench `--gpu-shards 4` | ≥ 2.8× |
| D38 | Mode-3 SLO C-3 mesurable | bench `mode3-k5` | p99 mesurée |
| D38 | k clamping observable | `efe_mode3_k_clamped_total` | > 0 sur k10 |

---

## §16. Risques transverses + mitigations

| ID | Risque | Probabilité | Impact | Mitigation |
|----|---|:--:|:--:|---|
| **B-R-1** | Tonic dep ajoute ~50 MB au binary release | Basse | Faible | feature-gate `tls` ; build statique non requis |
| **B-R-2** | Mode-3 workload temps de boot serveur > 60s | Moyenne | Moyen | timeout configurable ; retry sur Connect refused 3× |
| **B-R-3** | --gpu-shards N > N_GPUs réels = SIGSEGV | Moyenne | Élevé | Validation côté server (D31 audit) ; bench fail-fast sur erreur |
| **B-R-4** | JWT token expiré pendant un soak run 10 min | Moyenne | Moyen | Token refresh hook (callback config) post-D29 |
| **B-R-5** | Renommage proto Mode3ExecuteRequest = breaking change | Basse | Élevé | Pin test JSON schema + tonic-build version pin |
| **B-R-6** | mode3-k10 sature CI runner Tier-1 | Haute | Moyen | Workload mode3-k10 sur Tier-2 only ; CI Tier-1 = mode3-k5 only |
| **B-R-7** | report.json rétro-compat v1→v2 cassé | Basse | Élevé | Pin test parser legacy ; new fields en `Option<T>` |
| **B-R-8** | TonicBenchClient lent par TLS handshake (cold start) | Haute | Faible | Pre-warm channel à `connect()` time, hors timer |

---

## §17. Volumétrie code attendue

```
Sprint     LOC delta   Tests   e2e
─────────────────────────────────────
D29          +640      8       3
D31          +260      4       1
D38          +420      6       2
─────────────────────────────────────
Net total   +1320     18       6

Pin tests Python (28 nouveaux) :
  test_d29_bench_tonic.py        12
  test_d31_bench_gpu_shards.py    6
  test_d38_bench_mode3.py        10
```

---

## §18. Plans d'implémentation détaillés sprint-par-sprint

### §18.1 D29 (6 jours)

| Jour | Tâche | Livrable |
|---|---|---|
| J-1 | Setup `client/` subdir + déplacer mock.rs | structure |
| J-2 | TonicConfig + endpoint parsing | tonic_client.rs partial |
| J-3 | TonicBenchClient::connect + 6 dispatch helpers | tonic_client.rs complete |
| J-4 | JWT propagation + mTLS optional | auth integration |
| J-5 | JSON schema v2 + report.client_kind field | report.rs |
| J-6 | Pin tests Python + integration test boot real server | tests |

### §18.2 D31 (4 jours)

| Jour | Tâche | Livrable |
|---|---|---|
| J-1 | --gpu-shards flag + validation | bin/efe_bench.rs |
| J-2 | Runner shard assignment + metadata propagation | runner.rs |
| J-3 | Per-shard metrics + report.gpu_shards | metrics.rs + report.rs |
| J-4 | e2e test 4-shard + Jain index validation | tests |

### §18.3 D38 (5 jours)

| Jour | Tâche | Livrable |
|---|---|---|
| J-1 | WorkloadOp::Mode3Execute + workload patterns | workload.rs |
| J-2 | gen.rs Mode-3 op generator + tonic dispatch | gen.rs + tonic_client.rs |
| J-3 | --k-swarm flag + --allow-k-above-max | bin/efe_bench.rs |
| J-4 | report.k_swarm field + integration tests | report.rs + tests |
| J-5 | Pin tests + CI/CD pipeline gate update | tests + .github |

---

## §19. Annexes — CLI manuel post-D38

```text
efe-bench - Performance harness for EFE.

USAGE:
  efe-bench <COMMAND>

COMMANDS:
  run     Run a workload against an endpoint and produce a report.
  list    List the canonical workload labels.
  version Print the bench harness version.

RUN OPTIONS:
  --kind <LABEL>           Workload : read-heavy | write-heavy | balanced |
                           burst | multi-tenant | soak | mode3-k5 |
                           mode3-k10
  --endpoint <ADDR>        gRPC endpoint URL (e.g. http://127.0.0.1:50051).
  --duration <SECS>        Run duration (default per-workload).
  --concurrency <N>        Worker count (default per-workload).
  --target-rps <N>         Target sustained RPS (default per-workload).
  --tenant <ID>            Tenant id (default "acme").
  --output <PATH>          Write JSON report to PATH.
  --slo-check              Exit non-0 on any SLO violation.
  --jwt-token <TOKEN>      Bearer JWT for authentication. **D29**
  --tls-ca-file <PATH>     CA cert for mTLS. **D29**
  --gpu-shards <N>         Orchestrate N GPU shards (1..16). **D31**
  --k-swarm <K>            Mode-3 k value (1..5 default; 6..10 with
                           --allow-k-above-max). **D38**
  --allow-k-above-max      Allow k > 5 (server clamps; bench measures it).
  --allow-mock             Use MockBenchClient (default false post-D29).
  --seed <N>               Deterministic seed (default 0 = random).

EXAMPLES:
  # Read-heavy bench against staging.
  efe-bench run --kind read-heavy --endpoint $STAGING --jwt-token $TOK \
    --slo-check --output read.json

  # Multi-GPU 4-shard burst.
  efe-bench run --kind burst --gpu-shards 4 --endpoint $STAGING ...

  # Mode-3 k=5 (default).
  efe-bench run --kind mode3-k5 --endpoint $STAGING ...
```

---

## §20. Annexes — JSON output schema (BenchReport v2)

```json
{
  "schema_version": 2,
  "client_kind": "tonic",
  "gpu_shards": 4,
  "k_swarm": 5,
  "workload_label": "mode3-k5",
  "workload_mix": [
    {"op": "mode3_execute_k5", "weight": 100}
  ],
  "duration_seconds": 60,
  "concurrency": 16,
  "target_rps": 50,
  "tenant": "acme",
  "endpoint": "http://staging.efe.ai:50051",
  "operations": [
    {
      "op": "mode3_execute_k5",
      "count": 2941,
      "success_count": 2935,
      "error_count": 6,
      "p50_ms": 145.0,
      "p95_ms": 287.5,
      "p99_ms": 412.3,
      "throughput_rps": 49.0,
      "success_rate": 0.998
    }
  ],
  "shards": [
    {"shard_id": 0, "throughput_rps": 12.3, "p99_ms": 410.0},
    {"shard_id": 1, "throughput_rps": 12.1, "p99_ms": 412.5},
    {"shard_id": 2, "throughput_rps": 12.4, "p99_ms": 411.2},
    {"shard_id": 3, "throughput_rps": 12.2, "p99_ms": 413.1}
  ],
  "slos": {
    "D18-LAT-2": {"target": 200, "actual": 145, "passed": true},
    "C-3": {"target": "Mode3 p99 <= 3x Mode1 p99", "computed": "deferred", "passed": null}
  },
  "duration_actual_seconds": 60.04,
  "started_at_unix_ms": 1735689600000,
  "ended_at_unix_ms": 1735689660040,
  "harness_version": "1.0.0",
  "harness_git_sha": "abc1234"
}
```

---

## §21. Annexes — example CI/CD pipelines

(Cf. §12.1 above for canonical workflow YAML.)

---

## §22. Annexes — RACI + budget

| Sprint | Lead (R) | Approver (A) | Consulted (C) | Informed (I) |
|---|---|---|---|---|
| D29 | rodrigue.pekar | tech-lead | SRE (staging endpoint) | product, sec |
| D31 | rodrigue.pekar | tech-lead | GPU-team | SRE |
| D38 | rodrigue.pekar | tech-lead | M1 Mode-3 team | product |

Budget total : ~15 jours-ingénieur (3 sprints × moyenne 5 jours).

---

## §23. Annexes — Gantt prévisionnel D29 → D38

```text
Semaine    1     2     3     4
        ┌────┬────┬────┬────┐
D29     ██████      │     │
D31           ████  │     │
D38                █████  │
        └────┴────┴────┴────┘
GA      .................★ v1.0
```

* D31 démarre dès que D29 ship (TonicBenchClient pré-requis).
* D38 démarre après D31 (le label `gpu_shard_id` est utilisé
  par les workloads Mode-3 pour orchestration multi-GPU si
  `--gpu-shards > 1`).

---

## §24. Mises à jour du document

### §24.1 Versioning

* v1.0.0 — 2026-04-29 — Première édition (cette version).

### §24.2 Liens à maintenir

* `docs/bench/d18_multi_replica_bench_d12_architecture.md` — section
  workloads canoniques (D18 baseline).
* `docs/EFE_TELEMETRY_EXTENSION_ARCHITECTURE.md` § 13 — mapping
  SLO → instruments consommés post-bench.
* `docs/EFE_KERNELS_ARCHITECTURE.md` § 25 — wiring `gpu_shard_id`
  côté efe-kernels.
* `docs/EFE_SERVER_ARCHITECTURE.md` § 14 — header `x-efe-gpu-shard-id`
  côté efe-server interceptors.

### §24.3 Critères d'archive

* Les 3 sprints D29/D31/D38 ont shippé.
* Le `CHANGELOG.md` `[1.0.0]` entry est livré.
* L'audit pass M6 est terminé.
* Le tag `efe-bench@1.0.0` est créé.

---

**Fin du document.**

Total LOC du document : ~1 350 lignes Markdown.
Total LOC delta cible (M6 extension) : ~1 320 LOC Rust + ~280 LOC
Python tests + ~150 LOC YAML CI.
Total durée : ~15 jours-ingénieur sur 3 sprints non-contigus
(Φ-D → Φ-F).
