# `efe-core` — Architecture détaillée de l'extension du moteur (M1)

> **Statut** : Draft v1.0.0 — Document de référence pour l'achèvement
> du module **M1 = `efe-core`** entre l'AS-IS (post-D18, Mode-1 CPU
> production-ready, ~5 000 LOC livrés) et la cible **GA 1.0**
> (5 nouveaux templates nano-agents, Mode-2 + Mode-3 backends,
> parallélisme k-swarm via `tokio::JoinSet`, HybridFusion canary
> CPU/GPU).
>
> **Auteur** : rodrigue.pekar@icloud.com
> **Date** : 2026-04-29.
> **Dépendances doc** :
>   * `docs/PRODUCT_COMPLETION_ARCHITECTURE.md` — roadmap globale Φ-A→Φ-G.
>   * `docs/EFE_KERNELS_ARCHITECTURE.md` — substrat GPU CUDA.
>   * `docs/EFE_FFI_ARCHITECTURE.md` — bindings PyO3.
>   * `docs/EFE_SERVER_ARCHITECTURE.md` — extension gRPC M4 (D21-D37
>     côté server).
>   * `M1_ARCHITECTURE.md` — première édition Mode-1 CPU MVP (D1-D9).
>
> **Portée** : 5 sprints non-contigus s'étalant sur **3 phases**
> (Φ-D / Φ-E / Φ-F). Architecture **complémentaire** au M4 server :
> les 6 sprints M4 (D21/D22/D27/D32/D34/D37) ont livré la couche gRPC
> qui appelle les algorithmes ; M1 livre maintenant les algorithmes
> eux-mêmes côté `efe-core`. Sans ce M1 extension, les sprints M4-D34
> et M4-D37 sont **architecturalement boiteux** (domain logic dans la
> couche transport).
>
>   * **D33** (Φ-D) — 5 nouveaux templates nano-agents (NaQual,
>     NaCausal, NaTemp, NaSafety, NaConsist).
>   * **D34** (Φ-E) — Mode-2 backend (`Mode2Controller` +
>     `LlmRouter` trait).
>   * **D37** (Φ-F) — Mode-3 backend (`Mode3Controller` +
>     `Mode3Aggregator` trait + 3 impls).
>   * **D38** (Φ-F) — Parallélisation k-swarm via `tokio::JoinSet`
>     dans `swarm/mode3/parallel.rs`.
>   * **D32 cross-cutting** (Φ-D) — `HybridFusion` trait pour canary
>     GPU/CPU shadow validation.

---

## Table des matières

§0. Méta, glossaire, conventions
§1. Mission de l'extension M1 (Φ-D → Φ-F)
§2. Position dans la topologie EFE post-M4
§3. État AS-IS au 2026-04-29 (post-D18 + post-M4 phase 1)
§4. Définition opérationnelle de M1 v1.0 GA
§5. Principes directeurs
§6. Invariants structurels
§7. Vue d'ensemble — 5 sprints, 5 livrables, dependency graph
§8. Surface publique cible (traits + types)
§9. **Sprint D33 — 5 nouveaux templates nano-agents**
§10. **Sprint D34 — Mode-2 backend (`Mode2Controller`)**
§11. **Sprint D37 — Mode-3 backend (`Mode3Controller`)**
§12. **Sprint D38 — Parallélisation k-swarm**
§13. **Sprint D32 cross-cutting — `HybridFusion`**
§14. Migration `efe-server` → `efe-core` (backport plan)
§15. Coexistence avec les 3 templates MVP
§16. Tests + pin tests structurels
§17. Critères Go/No-Go par sprint
§18. Risques transverses + mitigations
§19. SLO budget
§20. Volumétrie code attendue
§21. Plans d'implémentation détaillés sprint-par-sprint
§22. Annexes — trait + struct definitions complètes
§23. Annexes — RACI + budget
§24. Annexes — Gantt prévisionnel D33 → D38
§25. Mises à jour du document

---

## §0. Méta, glossaire, conventions

### §0.1 Conventions

* **AS-IS** = état du code à fin D18 + post-M4 phase 1 + post-M4 audit
  (audit fixes M4-A1..A8 livrés), v0.1.19+M4-audit-1.
* **TO-BE** = état cible défini par chaque sprint M1 extension.
* Les sprints D33, D34, D37, D38 sont **non-contigus** : des sprints
  intercalaires (D35-D36 LLM routers production, D39-D41 SDKs, …)
  ne touchent pas `efe-core` directement.
* Les défauts résiduels sont notés **DT-C.x** (Defect Trace, Core).
* Toute mention "le moteur" sans qualification = `efe-core::SwarmController`.
* Toute mention "le contrôleur Mode-N" = `efe-core::swarm::modeN::ModeNController`.
* `[ref:fichier:ligne]` = pointeur dans le code source.
* Notation `Φ-X` = phase produit (Φ-A boot → Φ-G GA, cf.
  `PRODUCT_COMPLETION_ARCHITECTURE.md`).

### §0.2 Glossaire

| Terme | Définition |
|-------|------------|
| **`efe-core`** | Le crate Rust `crates/efe-core/` qui héberge le swarm engine pure-Rust : types, traits, contrôleurs, stratégies. Ne dépend ni de `tonic`, ni de `pyo3`, ni de réseau. |
| **`SwarmController`** | Le contrôleur Mode-1 actuel (D1-D9). Orchestre genesis → fusion ×N → consensus → dissolution → alignment. |
| **`Mode2Controller`** | Nouveau contrôleur D34 qui ajoute le routage par LLM avant le swarm. Anchors générées dynamiquement par requête via [`LlmRouter`]. |
| **`Mode3Controller`** | Nouveau contrôleur D37 qui exécute k swarms parallèles + agrège les centroïdes via [`Mode3Aggregator`]. |
| **`LlmRouter`** | Trait Rust D34 qui génère anchors dynamiques pour Mode-2. Implems internes `MockRouter` (déterministe) + impls externes (OpenAI, Anthropic) qui restent dans `efe-server` car réseau. |
| **`Mode3Aggregator`** | Trait Rust D37 qui combine k centroïdes en un verdict final. Stratégies : `Mean`, `Weighted`, `Median`. |
| **`TemplateCode`** | Enum nano-agent template. AS-IS : 3 variantes (NaSem, NaLog, NaCrit) ; TO-BE D33 : 8 variantes (+5 nouvelles : NaQual, NaCausal, NaTemp, NaSafety, NaConsist). |
| **`HybridFusion`** | Trait D32 cross-cutting qui exécute deux `FusionStrategy` en parallèle (CPU + GPU) et compare les résultats. Détecte les régressions GPU pendant le rollout D28-D32. |
| **MVP set** | Les 3 templates initiaux (NaSem, NaLog, NaCrit) — préservés intacts pour backward compat. |
| **GA set** | Les 8 templates post-D33 (MVP + 5 nouveaux). |
| **k-swarm** | Mode-3 fan-out : k instances parallèles de Mode-1 ou Mode-2 avec seeds différents, agrégation finale. |
| **Canary fusion** | Pattern D32 où le GPU est rollé out progressivement avec validation continue contre le CPU baseline. |

### §0.3 Cible matérielle

Le moteur `efe-core` est **CPU-first** par construction. Les sprints
M1 extension préservent cette invariance :

* **CPU MVP** (T1) — `MvpCpuFusion` reste l'implémentation par défaut.
* **CPU SIMD** (T1+) — AVX2/NEON activés via `--features accelerate`
  ou `--features openblas`.
* **GPU CUDA** (T2/T3) — `CudaFusion` consommée via le trait
  `FusionStrategy` ; `HybridFusion` (D32) compose un CPU + un GPU
  pour la validation canary.

Les sprints M1 D33-D38 **ne nécessitent PAS de GPU** pour leur
livraison. D32 HybridFusion peut tourner avec deux backends CPU
(p.ex. naïf vs BLAS) en attendant le GPU.

### §0.4 Cibles fonctionnelles GA

```
GA v1.0 efe-core (M1) :
├── Types
│   ├── TemplateCode { NaSem, NaLog, NaCrit, NaQual, NaCausal,    (D33)
│   │                  NaTemp, NaSafety, NaConsist }
│   ├── Mode2Verdict / Mode3Verdict / SubVerdict                   (D34/D37)
│   └── HybridFusionReport (CPU+GPU diff)                          (D32)
│
├── Traits
│   ├── FusionStrategy                                             (existant)
│   ├── ConsensusDetector                                          (existant)
│   ├── Dissolver                                                  (existant)
│   ├── AlignmentProjector                                         (existant)
│   ├── GenesisStrategy                                            (existant)
│   ├── LlmRouter                                                  (D34, NEW)
│   ├── Mode3Aggregator                                            (D37, NEW)
│   └── HybridFusion                                               (D32, NEW)
│
├── Contrôleurs
│   ├── SwarmController                                            (existant — Mode-1)
│   ├── Mode2Controller                                            (D34, NEW)
│   └── Mode3Controller                                            (D37, NEW)
│
├── Templates
│   ├── crates/efe-core/src/agents/templates/                      (D33, NEW dir)
│   │   ├── mod.rs
│   │   ├── na_sem.rs        (existant — refactored)
│   │   ├── na_log.rs        (existant — refactored)
│   │   ├── na_crit.rs       (existant — refactored)
│   │   ├── na_qual.rs       (D33, NEW)
│   │   ├── na_causal.rs     (D33, NEW)
│   │   ├── na_temp.rs       (D33, NEW)
│   │   ├── na_safety.rs     (D33, NEW)
│   │   └── na_consist.rs    (D33, NEW)
│   │
│   └── Population mix rules (cardinality.rs étendu)
│
└── Parallélisme
    └── crates/efe-core/src/swarm/mode3/parallel.rs                 (D38, NEW)
```

---

## §1. Mission de l'extension M1 (Φ-D → Φ-F)

### §1.1 Énoncé opérationnel

L'extension M1 a **une seule mission** : **rapatrier dans `efe-core`
les algorithmes Mode-2/Mode-3 actuellement hébergés dans `efe-server`,
ET livrer les 5 nouveaux templates nano-agents requis pour la
production GA**.

Concrètement :

1. **Rapatrier** `Mode2ServiceImpl::execute_inner` (efe-server) →
   `Mode2Controller::execute` (efe-core).
2. **Rapatrier** `Mode3ServiceImpl::execute` (efe-server) →
   `Mode3Controller::execute` (efe-core).
3. **Rapatrier** `LlmRouter` trait + `Mode3Aggregator` trait
   (efe-server) → `efe-core`.
4. **Rapatrier** `services::synthetic_spec.rs` (efe-server) →
   primitives natives dans `efe-core::swarm::modeN::synthesis`.
5. **Étendre** `TemplateCode` enum de 3 → 8 variantes + livrer
   les 5 modules `agents/templates/na_*.rs` correspondants.
6. **Livrer** `tokio::JoinSet`-based parallel runner dans
   `swarm/mode3/parallel.rs` — réutilisable par efe-server, efe-ffi,
   efe-bench.
7. **Livrer** `HybridFusion` trait + impl par défaut pour le canary
   GPU/CPU shadow durant D28-D32.

### §1.2 Périmètre strict du document

Ce document **NE traite PAS** :

* La couche gRPC qui consomme ces APIs (déjà couverte par M4).
* Les bindings PyO3 qui consomment ces APIs (couverts par M3).
* Les implémentations LLM externes (`OpenAIRouter`, `AnthropicRouter`)
  qui restent dans `efe-server` car nécessitent réseau / tokio runtime
  HTTP. Le **trait** `LlmRouter` est dans `efe-core` ; les impls
  réseau dans `efe-server`.
* Les déploiements Helm / dashboard Grafana — ces surfaces consomment
  `efe-core` indirectement via `efe-server`.
* Les benchs `efe-bench` — qui consomment l'API publique livrée ici.

### §1.3 Mission complémentaire

L'extension M1 ouvre la voie à 3 capacités produit qui restaient
bloquées par l'architecture actuelle :

1. **`efe-ffi` peut exposer Mode-2/Mode-3 in-process** (PyO3) sans
   passer par gRPC → latence -10 ms p99, déterminisme exact.
2. **`efe-bench` peut bencher Mode-2/Mode-3** sans transport overhead
   → numbers réalistes pour la documentation produit.
3. **Tests intégration peuvent exercer Mode-2/Mode-3** sans booter
   un serveur tonic → CI runner-time -8 minutes par run.

---

## §2. Position dans la topologie EFE post-M4

### §2.1 Graph de dépendances post-D38

```text
                       ┌─────────────────────────────────────────┐
                       │                Clients                  │
                       │  ┌──────────┐  ┌──────────┐  ┌─────────┐│
                       │  │  Python  │  │   Go     │  │  TS/JS  ││
                       │  │   SDK    │  │   SDK    │  │   SDK   ││
                       │  └────┬─────┘  └────┬─────┘  └────┬────┘│
                       └───────┼─────────────┼──────────────┼─────┘
                               │             │              │
                               ▼             ▼              ▼
                       ┌─────────────────────────────────────────┐
                       │           gRPC (M4 efe-server)          │
                       │ Mode1Service, Mode2Service, Mode3Service│
                       └─────────────────────────────────────────┘
                                          │
                       ┌──────────────────┴──────────────────────┐
                       │                                          │
                       ▼                                          ▼
            ┌─────────────────────┐                 ┌─────────────────────┐
            │   efe-ffi (M3)      │                 │   efe-server (M4)   │
            │   PyO3 in-process   │                 │   gRPC handlers     │
            └─────────┬───────────┘                 └──────────┬──────────┘
                      │                                         │
                      └──────────────────┬──────────────────────┘
                                         │
                                         ▼
                      ┌──────────────────────────────────────┐
                      │      M1 efe-core (POST-D38)          │
                      │                                      │
                      │ ┌──────────────────────────────────┐ │
                      │ │   SwarmController (Mode-1)       │ │
                      │ │   Mode2Controller (D34)          │ │
                      │ │   Mode3Controller (D37)          │ │
                      │ │   ParallelKSwarm (D38)           │ │
                      │ └──────────────────────────────────┘ │
                      │                                      │
                      │ ┌──────────────────────────────────┐ │
                      │ │   FusionStrategy (existant)      │ │
                      │ │   HybridFusion (D32, NEW)        │ │
                      │ │   ConsensusDetector              │ │
                      │ │   Dissolver / AlignmentProjector │ │
                      │ │   GenesisStrategy                │ │
                      │ └──────────────────────────────────┘ │
                      │                                      │
                      │ ┌──────────────────────────────────┐ │
                      │ │   Templates (D33, +5)            │ │
                      │ │   ├── NaSem / NaLog / NaCrit     │ │
                      │ │   ├── NaQual / NaCausal          │ │
                      │ │   ├── NaTemp / NaSafety          │ │
                      │ │   └── NaConsist                  │ │
                      │ └──────────────────────────────────┘ │
                      └──────────────┬───────────────────────┘
                                     │
                                     ▼
                      ┌──────────────────────────────────────┐
                      │    efe-kernels (M2)   GPU CUDA       │
                      │    Cf. EFE_KERNELS_ARCHITECTURE.md    │
                      └──────────────────────────────────────┘
```

### §2.2 Direction stricte des dépendances

* `efe-core` **ne dépend PAS** de `efe-server`, `efe-ffi`,
  `efe-kernels` (sauf `feature = "cuda"` — opt-in).
* `efe-server` dépend de `efe-core` (cf. M4 § 2.2).
* `efe-ffi` dépend de `efe-core` (cf. M3 § 2.3).
* `efe-bench` dépend de `efe-core` + `efe-server` (pour bencher
  l'API publique end-to-end).

Cette direction garantit que le moteur reste **utilisable in-process**
par les bindings PyO3, sans réseau, sans tonic, sans serde de proto.

### §2.3 Position vs `efe-server` (M4)

| Surface | efe-core (M1, post-D38) | efe-server (M4) |
|---------|:-----------------------:|:---------------:|
| Trait `LlmRouter` | ✅ déclare | ✅ implems réseau |
| Trait `Mode3Aggregator` | ✅ déclare + 3 impls | ✅ utilise via Arc |
| `Mode2Controller` | ✅ orchestrateur algorithmique | ❌ wrapper gRPC seul |
| `Mode3Controller` | ✅ orchestrateur algorithmique | ❌ wrapper gRPC seul |
| `tokio::JoinSet` k-swarm | ✅ dans `swarm/mode3/parallel.rs` | ❌ délégué à efe-core |
| `synthetic_spec` helper | ✅ primitive native | ❌ supprimé après backport |
| Templates NaQual..NaConsist | ✅ déclarés + impls | ❌ — consommés via SwarmSpec |
| `HybridFusion` | ✅ trait + impl | ❌ utilisé indirectement |

Après D38 + audit pass, **`efe-server` ne contient PLUS aucun
algorithme** — uniquement des handlers gRPC qui translatent proto ↔
domain et appellent les contrôleurs `efe-core`.

---

## §3. État AS-IS au 2026-04-29 (post-D18 + post-M4 phase 1)

### §3.1 Inventaire physique efe-core

```
crates/efe-core/src/
├── alignment/                  # M9 — cones + registry (livré D9)
│   ├── breach.rs    builder.rs  canonical.rs  compose.rs
│   ├── events.rs    evolution.rs  key.rs       mod.rs
│   ├── policy.rs    registry.rs   validator.rs
│   └── (~3 200 LOC)
├── collapse/                   # D5-D7 — collapse strategies + engine
│   ├── strategies/{abort,centroid_project,noise,partial_respawn,
│   │              rollback,stub}.rs
│   └── (~2 100 LOC)
├── compute/                    # D2-D3 — kernels CPU + cuda (D28)
│   ├── cpu/{activ,attention,consensus,linalg,linalg_blas,
│   │       sampling,shared,stats}.rs
│   ├── cuda/mod.rs             # D28 stub
│   └── (~1 800 LOC)
├── consensus/                  # D6 — multi-criteria detector
│   ├── config.rs    detector.rs   mod.rs    verdict.rs
│   └── (~600 LOC)
├── dissolution/                # D7 — alignment + extractors
│   ├── alignment.rs config.rs    extractors.rs  mod.rs
│   └── (~800 LOC)
├── els/                        # D2 — epistemic latent space
│   └── (~400 LOC)
├── errors/                     # D1 — EfeError taxonomy
│   └── (~250 LOC)
├── fusion/                     # D5 — MVP CPU fusion
│   ├── kernels/{aggregate,attention,clip,entropy,mix,project,softmax}.rs
│   ├── config.rs    mvp_cpu.rs   replay.rs    round.rs
│   ├── schedule.rs  stats.rs     strategy.rs  validator.rs
│   └── (~2 200 LOC)
├── genesis/                    # D4 — genesis engine + spawner
│   ├── cardinality.rs context.rs    dag.rs        engine.rs
│   ├── meta_analyzer.rs registry.rs   router.rs     spawn_kernel.rs
│   ├── spawner.rs
│   └── (~2 500 LOC)
├── memory/                     # D2 — memory pools
│   └── (~600 LOC)
├── streams/                    # D2 — stream pools
│   └── (~250 LOC)
├── swarm/                      # D1 — controller + traits + types
│   ├── arbiter.rs   controller.rs mod.rs        request.rs
│   ├── state.rs     strategies.rs verdict.rs
│   └── (~1 700 LOC, dont D9 calibration)
├── telemetry/                  # M11 — collector traits
│   └── (~50 LOC)
├── types.rs                    # 836 LOC — enum + struct publics
├── config.rs                   # 200 LOC
└── lib.rs                      # 90 LOC

Total : ~16 700 LOC
```

### §3.2 Surface gRPC consommée AS-IS

`efe-server` (M4 post-audit) consomme :

* `efe_core::SwarmController` — instancié dans `mode1.rs::build_mvp_backend()`.
* `efe_core::Request / Response / Verdict` — via `bridge::request::from_proto`.
* `efe_core::TraceContext` — via interceptors.
* `efe_core::types::*` — via `bridge::cone::from_proto` et `bridge::spec::from_proto`.

`efe-server` **NE consomme PAS** (mais devrait après D38) :

* `efe_core::swarm::mode2::*` — n'existe pas encore.
* `efe_core::swarm::mode3::*` — n'existe pas encore.

### §3.3 Templates AS-IS

```rust
// crates/efe-core/src/types.rs:118-160 (AS-IS)
#[non_exhaustive]
pub enum TemplateCode {
    NaSem = 0,
    NaLog = 1,
    NaCrit = 2,
}

impl TemplateCode {
    pub const MVP_SET: &'static [Self] = &[Self::NaSem, Self::NaLog, Self::NaCrit];

    pub const fn as_str(&self) -> &'static str { /* ... */ }
    pub const fn as_u8(&self) -> u8 { /* ... */ }
    pub fn from_str(s: &str) -> Option<Self> { /* ... */ }
}
```

Le doc-comment AS-IS dit explicitement :

> "MVP covers three classes. Further templates (NaMath, NaCode, NaCaus,
> NaMem, NaAlign, NaMed) arrive in phases P2 and P3."

D33 réalise une fraction de cette promesse (5 sur les 6 listés, avec
des renommages). Les noms définitifs validés en revue produit :

| AS-IS doc-comment | TO-BE D33 | Rationale |
|---|---|---|
| NaMath | (annulé) | Couvert par NaCausal+NaTemp pour reasoning numérique |
| NaCode | (annulé) | Hors-scope GA, prévu post-D40 |
| NaCaus | **NaCausal** | Renommé pour cohérence avec NaQual/NaSafety |
| NaMem | (annulé) | Mémoire = état du swarm, pas un template |
| NaAlign | **NaSafety** | Renommé pour clarté ; safety=alignement aux cibles |
| NaMed | (annulé) | Domain-specific, hors-scope GA |
| (nouveau) | **NaQual** | Quality-focused (cohérence, fluence) |
| (nouveau) | **NaTemp** | Temporal reasoning |
| (nouveau) | **NaConsist** | Internal consistency / contradiction detection |

### §3.4 Mode-2 AS-IS (post-M4 audit)

* Le code Mode-2 vit **entièrement dans `efe-server`** :
  * `crates/efe-server/src/services/mode2/router.rs` (402 LOC) —
    trait `LlmRouter` + 3 impls (`MockRouter`, `OpenAIRouter`,
    `AnthropicRouter`).
  * `crates/efe-server/src/services/mode2/mod.rs` (786 LOC) —
    `Mode2ServiceImpl` (gRPC handler) + `execute_inner()` helper
    (post-audit).
* `efe-core` **ne sait pas** que Mode-2 existe.
* Conséquence : `efe-ffi` PyO3 ne peut pas exposer `mode2_classify(...)`.
* Conséquence : `efe-bench` ne peut pas bencher Mode-2.

### §3.5 Mode-3 AS-IS (post-M4 audit)

* Idem Mode-2 :
  * `crates/efe-server/src/services/mode3/aggregator.rs` (365 LOC) —
    trait `Mode3Aggregator` + 3 impls (`MeanAggregator`,
    `WeightedAggregator`, `MedianAggregator`).
  * `crates/efe-server/src/services/mode3/mod.rs` (798 LOC) —
    `Mode3ServiceImpl` + `execute_sub_swarm()` helper.
* `tokio::task::JoinSet` est utilisé dans
  `Mode3ServiceImpl::execute()` mais inaccessible aux clients
  in-process.

### §3.6 HybridFusion AS-IS

* **N'existe pas**. `fusion/hybrid.rs` n'a jamais été écrit.
* La validation cross-CPU/GPU lors du rollout D28 (CudaBackend) est
  faite **manuellement** : opérateur compare `RegistryEvent` issus de
  deux pods identiques (l'un --features cuda, l'autre CPU-only) et
  vérifie la convergence verdict-par-verdict.
* Cette validation manuelle ne scale pas pour D32 (rollout production).

### §3.7 Synthèse AS-IS

| Sprint | État AS-IS | LOC manquantes | Effort |
|---|---|---|---|
| D33 templates | 3/8 livrés | ~600 (5 nouveaux modules) | 5 jours |
| D34 Mode-2 | 0% en efe-core | ~1 188 (backport + refactor) | 8 jours |
| D37 Mode-3 | 0% en efe-core | ~1 163 (backport + refactor) | 8 jours |
| D38 parallel | 0% (tokio::JoinSet dans efe-server uniquement) | ~280 (extraction) | 3 jours |
| D32 HybridFusion | 0% | ~450 (NEW) | 4 jours |

Total : ~3 681 LOC à livrer côté `efe-core`. **Tous prérequis présents
dans le workspace** (tokio, async-trait, sha2, dashmap déjà dispo
ou cheap à ajouter à la dep tree de efe-core).

---

## §4. Définition opérationnelle de `efe-core` v1.0 GA

### §4.1 Le contrat GA

À la fin de l'extension M1 (post-D38) :

* **Templates** : 8 variantes `TemplateCode` + 8 modules
  `agents/templates/na_*.rs` qui décrivent
  `(default_population_share, fusion_kernel_routing,
  consensus_weight)` pour chaque template.
* **Mode-2 backend** : `Mode2Controller<B>` générique sur tout
  backend `B: ExecuteBackend` (typiquement `Arc<SwarmController>`),
  qui orchestre cache + router + backend.
* **Mode-3 backend** : `Mode3Controller<B>` générique sur backend ;
  fan-out k swarms via `parallel::run_k_swarms(...)`.
* **k-swarm parallélisme** : `swarm::mode3::parallel::run_k_swarms<F>`
  fonction publique réutilisable.
* **HybridFusion** : trait + impl par défaut `CanaryHybridFusion<P, S>`
  qui compose primary + shadow `FusionStrategy`.

### §4.2 Versioning produit

| efe-core version | Sprint | Templates | Modes | HybridFusion |
|------------------|---|---|---|---|
| 0.1.x (D1-D9 MVP) | D9 | 3 (MVP) | Mode-1 only | — |
| 0.2.x (D33 livré) | D33 | 8 (GA) | Mode-1 only | — |
| 0.3.x (D34 livré) | D34 | 8 | Mode-1 + Mode-2 | — |
| 0.4.x (D37 livré) | D37 | 8 | Mode-1/2/3 | — |
| 0.5.x (D38 livré) | D38 | 8 | Mode-1/2/3 + parallel | — |
| 0.6.x (D32 livré) | D32 | 8 | Mode-1/2/3 | ✅ |
| **1.0.0 GA** | post-D38+D32 | 8 | Mode-1/2/3 + parallel + canary | ✅ |

### §4.3 Gating entre versions

Chaque bump mineur exige :

* Cargo.toml workspace.version bumpé.
* `bench_baselines/v0.X.Y/baseline.json` frozen avec les nouveaux
  metrics.
* Pin tests Python spécifiques au sprint dans
  `scripts/tests/test_dXX_*.py`.
* Audit pass terminé (cf. M4 audit pattern).
* CHANGELOG.md entry avec audit fix table.

---

## §5. Principes directeurs

### §5.1 Principes opposables en revue PR

| ID | Principe | Application |
|----|---|---|
| **P-1** | `efe-core` reste pure-Rust algorithmique | Aucun import de `tonic`, `pyo3`, `reqwest` dans `efe-core` |
| **P-2** | Backward compat MVP set | `TemplateCode::MVP_SET` reste accessible et testé |
| **P-3** | Génériques sur `B: ExecuteBackend` | `Mode2Controller<B>` / `Mode3Controller<B>` génériques pour permettre stub backend en tests |
| **P-4** | Traits exposés, impls cachées | `LlmRouter` / `Mode3Aggregator` traits `pub` ; impls réseau dans `efe-server` |
| **P-5** | Synchronous fallback pour PyO3 | Toute API async expose une variante sync via `tokio::runtime::Handle::block_on` (consommée par `efe-ffi`) |
| **P-6** | Determinism par seed | Mode-2/3 prennent `seed: u64` ; mêmes inputs → mêmes outputs (SLO C-7) |
| **P-7** | k_max hardcoded | `swarm::mode3::K_MAX = 5` const partagée entre core et server |
| **P-8** | HybridFusion fail-open | Quand le shadow GPU diverge, log + alerte mais le primary CPU verdict est retourné — pas d'erreur cliente |
| **P-9** | Templates immutables | `TemplateCode` `#[non_exhaustive]` pour garder l'extension future possible sans break wire |

### §5.2 Ce que `efe-core` refuse d'implémenter

* **Network I/O** — pas de `reqwest`, pas de `hyper`, pas de gRPC. Si
  un trait nécessite réseau (LlmRouter externes), seul le trait est
  dans efe-core ; les impls sont dans efe-server.
* **Persistance disque pour Mode-2/3** — pas de PG, pas de Redis.
  Le cache Mode-2 est `DashMap<u64, RouterOutput>` in-memory uniquement
  (le cache distribué est DT-C-D34-CACHE-1, post-GA).
* **Métriques Prometheus** — `efe-core` émet via le trait
  `TelemetryCollector` (M11). Les implems Prom sont dans `efe-telemetry`.
* **Audit chain HMAC** — délégué à `efe-server::catalog::audit_hmac`.

---

## §6. Invariants structurels

### §6.1 Invariants de structure de fichiers

```
crates/efe-core/src/
├── agents/                    # NEW D33
│   └── templates/
│       ├── mod.rs             # exports + tests
│       ├── na_sem.rs          # refactor existant
│       ├── na_log.rs          # refactor existant
│       ├── na_crit.rs         # refactor existant
│       ├── na_qual.rs         # NEW D33
│       ├── na_causal.rs       # NEW D33
│       ├── na_temp.rs         # NEW D33
│       ├── na_safety.rs       # NEW D33
│       └── na_consist.rs      # NEW D33
├── swarm/
│   ├── mode2/                 # NEW D34
│   │   ├── mod.rs             # Mode2Controller
│   │   ├── router.rs          # LlmRouter trait + MockRouter
│   │   ├── cache.rs           # DashMap-backed anchor cache
│   │   └── synthesis.rs       # build_synthetic_request (backport)
│   └── mode3/                 # NEW D37
│       ├── mod.rs             # Mode3Controller
│       ├── aggregator.rs      # Mode3Aggregator trait + 3 impls
│       └── parallel.rs        # NEW D38 — tokio::JoinSet runner
├── fusion/
│   ├── hybrid.rs              # NEW D32 — HybridFusion trait + impl
│   └── (existant)
└── (existant)
```

### §6.2 Invariants de runtime

* **`efe-core` n'instancie JAMAIS un `tokio::runtime::Runtime`**.
  Tous les `async fn` consomment l'ambient runtime. Les appels sync
  bridges (PyO3) doivent fournir leur runtime via `block_on`.
* **`Arc<dyn LlmRouter>` est `Send + Sync`**. Toute impl le respecte.
* **`Arc<dyn Mode3Aggregator>` est `Send + Sync`**. Idem.
* **k_max = 5 est immutable runtime**. Pas d'env var ; pour changer,
  bump version et refaire le bench p99.

### §6.3 Invariants de versioning

* `TemplateCode` `#[non_exhaustive]` — ajout de variantes = minor bump.
* Suppression d'une variante = breaking change → major bump (≥ 2.0).
* Réordonnancement des `repr(u8)` valeurs = breaking (CUDA buffers
  encodent l'u8). NaSem stays = 0, NaLog = 1, NaCrit = 2 forever.

---

## §7. Vue d'ensemble — 5 sprints, 5 livrables, dependency graph

### §7.1 Tableau d'agrégation

| Sprint | Phase | Durée | Livrable principal | LOC | Pin tests | Bench |
|--------|---|---|---|--:|--:|---|
| **D33** | Φ-D | 5 j | 5 templates `na_*.rs` + `TemplateCode` étendu | ~600 | 8 | accuracy_template_diversity |
| **D34** | Φ-E | 8 j | `Mode2Controller<B>` + `LlmRouter` trait + cache | ~1 200 | 12 | mode2_e2e_latency |
| **D37** | Φ-F | 8 j | `Mode3Controller<B>` + `Mode3Aggregator` trait | ~1 200 | 14 | mode3_e2e_latency |
| **D38** | Φ-F | 3 j | `parallel::run_k_swarms<F>` + JoinSet plumbing | ~300 | 6 | mode3_k_parallel_throughput |
| **D32** | Φ-D (cross) | 4 j | `HybridFusion` trait + `CanaryHybridFusion` | ~450 | 7 | hybrid_canary_overhead |

Total : **~3 750 LOC** + **47 pin tests** + **5 bench scenarios**.

### §7.2 Dépendances inter-sprints

```text
                D33 (templates 5)
                     │
                     ├─────────────┐
                     ▼             ▼
                D34 (Mode-2)    D32 (HybridFusion)
                     │             │
                     ▼             │
                D37 (Mode-3)       │  (D32 cross-cutting,
                     │             │   peut shipper avant D34)
                     ▼             │
                D38 (parallel)     │
                     │             │
                     └─────────────┘
                            │
                            ▼
                    GA v1.0 efe-core
```

* **D33 préalable** à D34/D37 : Mode-2/3 routing dépend de la
  diversité templates pour produire des sub-verdicts utiles.
* **D34 préalable** à D37 : Mode-3 fan-out peut router en mode2,
  donc Mode2Controller doit exister.
* **D38 après D37** : la parallélisation k-swarm est une optimisation
  du contrôleur Mode-3 séquentiel.
* **D32 cross-cutting** : indépendant des autres ; peut shipper en
  parallèle de D33 (touchent fichiers différents : `fusion/hybrid.rs`
  vs `agents/templates/`).

### §7.3 Volumétrie code attendue

```
D33 templates :
  Refactor 3 existants  → -200 LOC (extraction de types.rs)
  5 nouveaux modules    → ~5 × 80 = 400 LOC
  Cardinality update    → +80 LOC
  Genesis routing       → +120 LOC
  Tests                 → +200 LOC
  Total                 → ~600 LOC

D34 Mode-2 :
  swarm/mode2/mod.rs            → ~280 LOC
  swarm/mode2/router.rs         → ~250 LOC (backport + simplif)
  swarm/mode2/cache.rs          → ~200 LOC
  swarm/mode2/synthesis.rs      → ~150 LOC (backport synthetic_spec)
  Tests + pin tests             → ~320 LOC
  Total                         → ~1 200 LOC

D37 Mode-3 :
  swarm/mode3/mod.rs            → ~300 LOC
  swarm/mode3/aggregator.rs     → ~250 LOC (backport)
  Tests + pin tests             → ~250 LOC
  parallel.rs (D38, livré ici)  → ~280 LOC
  Refactor efe-server pour appeler core → 120 LOC
  Total                         → ~1 200 LOC + parallel D38

D38 parallel :
  swarm/mode3/parallel.rs       → ~280 LOC
  Tests stress + cancellation   → ~150 LOC
  Total                         → ~430 LOC

D32 HybridFusion :
  fusion/hybrid.rs              → ~280 LOC
  fusion/hybrid_report.rs       → ~80 LOC
  Tests + pin tests             → ~140 LOC
  Total                         → ~500 LOC
```

---

## §8. Surface publique cible (traits + types)

### §8.1 Inventaire post-D38 (GA)

```rust
// crates/efe-core/src/lib.rs (post-D38)

pub mod agents;       // NEW D33
pub mod alignment;
pub mod collapse;
pub mod compute;
pub mod config;
pub mod consensus;
pub mod dissolution;
pub mod els;
pub mod errors;
pub mod fusion;
pub mod genesis;
pub mod memory;
pub mod streams;
pub mod swarm;
pub mod telemetry;
pub mod types;

// Re-exports — la "surface 1-line" que les consommateurs (efe-server,
// efe-ffi, efe-bench) importent.
pub use alignment::{
    AlignmentRegistry, BreachFilter, BreachReport, CanonicalCone,
    ConeKey, ConeMetadata, InMemoryRegistry, RegistryEvent,
};
pub use config::Config;
pub use errors::{EfeError, ErrorSeverity, Result};
pub use swarm::{
    controller::{ControllerConfig, SwarmController, SwarmControllerBuilder},
    mode2::{                    // NEW D34
        Mode2Controller, Mode2Verdict, RouterMetadata,
    },
    mode3::{                    // NEW D37
        Mode3Aggregator, Mode3Controller, Mode3Verdict, SubVerdict,
        AggregationMetadata, MeanAggregator, MedianAggregator,
        WeightedAggregator,
    },
    request::{DecodeMode, DecodedOutput, Request, Response, TraceContext},
    strategies::{
        AlignmentProjector, ConsensusDetector, Dissolver,
        FusionStrategy, GenesisStrategy, NullTelemetryCollector,
        TelemetryCollector,
    },
    verdict::Verdict,
};
pub use types::{
    AlignmentCone, ConvergenceThresholds, LatentDim, NanoAgentId,
    PopulationMix, RequestId, SwarmId, SwarmSpec, TemplateCode,
};
pub use fusion::{                // D32 cross-cutting
    HybridFusion, HybridFusionReport, CanaryHybridFusion,
};
pub use agents::templates::{     // D33
    NaSem, NaLog, NaCrit, NaQual, NaCausal, NaTemp, NaSafety, NaConsist,
};
```

### §8.2 Mapping consommateur → surface

| Consommateur | Surface principale | Sprint |
|---|---|---|
| `efe-server::services::mode2` | `Mode2Controller`, `LlmRouter` | D34 |
| `efe-server::services::mode3` | `Mode3Controller`, `Mode3Aggregator` | D37 |
| `efe-ffi::async_::Mode2Async` | `Mode2Controller` via PyO3 | D34→post |
| `efe-ffi::async_::Mode3Async` | `Mode3Controller` via PyO3 | D37→post |
| `efe-bench::benches::mode2_e2e` | `Mode2Controller` direct | D34 |
| `efe-bench::benches::mode3_k_parallel` | `parallel::run_k_swarms` | D38 |
| `efe-server::runtime::canary_fusion` | `HybridFusion` | D32 |

---

## §9. Sprint **D33** — 5 nouveaux templates nano-agents

### §9.1 Objectif

Étendre `TemplateCode` enum de 3 → 8 variantes et livrer les 5
modules `agents/templates/na_*.rs` qui décrivent leur comportement
(routage fusion, partage population, poids consensus).

Les 5 nouveaux templates :

| Template | u8 | Stable string | Rôle |
|---|--:|---|---|
| **NaQual** | 3 | `na-qual` | Quality-focused (cohérence, fluence) |
| **NaCausal** | 4 | `na-causal` | Causal reasoning (X → Y) |
| **NaTemp** | 5 | `na-temp` | Temporal awareness (before/after, durations) |
| **NaSafety** | 6 | `na-safety` | Refusal patterns + harmful content detection |
| **NaConsist** | 7 | `na-consist` | Internal consistency / contradiction |

### §9.2 Architecture du module

```text
crates/efe-core/src/agents/
├── mod.rs                # pub mod templates;
└── templates/
    ├── mod.rs            # ré-exports + trait NanoAgentTemplate
    ├── na_sem.rs         # extracted from genesis/registry.rs
    ├── na_log.rs         # extracted
    ├── na_crit.rs        # extracted
    ├── na_qual.rs        # NEW
    ├── na_causal.rs      # NEW
    ├── na_temp.rs        # NEW
    ├── na_safety.rs      # NEW
    └── na_consist.rs     # NEW
```

### §9.3 Trait `NanoAgentTemplate`

```rust
// crates/efe-core/src/agents/templates/mod.rs
//
// **Sprint D33** — chaque template implémente ce trait pour que le
// genesis engine + la fusion engine + le consensus detector puissent
// router les decisions algorithmiques.

use crate::types::TemplateCode;

/// Per-template behavioural descriptor. The genesis engine reads
/// `default_share()` to decide how many of each template to spawn.
/// The fusion engine reads `fusion_kernel()` to dispatch the right
/// kernel. The consensus detector reads `consensus_weight()` to
/// weight the template's contribution to the final verdict.
pub trait NanoAgentTemplate: Send + Sync + std::fmt::Debug + 'static {
    /// The enum variant this impl represents. Stable across versions.
    fn code(&self) -> TemplateCode;

    /// Default share in a balanced PopulationMix. Sum across all
    /// templates is 1.0. Used by `PopulationMix::balanced()`.
    fn default_share(&self) -> f32;

    /// Routing key for the fusion kernel. Returns the kernel id that
    /// `MvpCpuFusion` (and friends) should dispatch to for this
    /// template's nano-agents.
    fn fusion_kernel(&self) -> FusionKernelId;

    /// Weight in the multi-criteria consensus detector. Higher =
    /// template's vote counts more. Default 1.0.
    fn consensus_weight(&self) -> f32 { 1.0 }

    /// Optional centroid initialisation rule. Returns `None` = use
    /// the default (problem_embedding ± noise).
    fn centroid_init(&self) -> Option<CentroidInit> { None }
}

/// Kernel routing in the fusion engine.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FusionKernelId {
    /// Standard semantic kernel (NaSem default).
    Semantic,
    /// Logical-inference kernel (NaLog default).
    Logical,
    /// Diversification kernel (NaCrit default).
    Diversifying,
    /// **D33** — quality-scoring kernel (NaQual).
    Quality,
    /// **D33** — causal-graph kernel (NaCausal).
    Causal,
    /// **D33** — temporal-attention kernel (NaTemp).
    Temporal,
    /// **D33** — safety-classification kernel (NaSafety).
    Safety,
    /// **D33** — consistency-check kernel (NaConsist).
    Consistency,
}

#[derive(Clone, Debug)]
pub enum CentroidInit {
    /// Take problem_embedding + Gaussian noise (sigma).
    PerturbProblem { sigma: f32 },
    /// Take a fixed point in latent space (must match dim_latent).
    Fixed(Vec<f32>),
    /// Sample uniformly from `[-1, 1]^dim`.
    Uniform,
}
```

### §9.4 Détail des 5 nouveaux templates

#### `NaQual` — Quality

```rust
// crates/efe-core/src/agents/templates/na_qual.rs
use super::{FusionKernelId, NanoAgentTemplate};
use crate::types::TemplateCode;

/// **Sprint D33** — Quality-focused nano-agent template.
///
/// Optimises for output coherence (no contradictions internal to a
/// span), fluency (low cross-entropy on the dissolution decoder),
/// and pertinence (high cosine similarity to problem_embedding).
///
/// Default share : 15 % of a balanced mix. Higher than NaSem (50 %)
/// because Quality is a per-span scoring agent, not a generative one.
#[derive(Debug, Clone, Copy)]
pub struct NaQual;

impl NanoAgentTemplate for NaQual {
    fn code(&self) -> TemplateCode { TemplateCode::NaQual }
    fn default_share(&self) -> f32 { 0.15 }
    fn fusion_kernel(&self) -> FusionKernelId { FusionKernelId::Quality }
    fn consensus_weight(&self) -> f32 { 1.2 } // slightly higher than baseline
}
```

#### `NaCausal` — Causal reasoning

```rust
// crates/efe-core/src/agents/templates/na_causal.rs

/// **Sprint D33** — Causal-reasoning nano-agent template.
///
/// Specialises in detecting/scoring directed causal relations (X → Y)
/// vs spurious correlations. Used by Mode-3 sub-swarms when the query
/// involves "because", "if X then Y", or counterfactual phrasings.
///
/// Default share : 10 %. Activated more aggressively when the query
/// embedding has high cosine similarity with the causal-prototype
/// vector (computed at boot).
#[derive(Debug, Clone, Copy)]
pub struct NaCausal;

impl NanoAgentTemplate for NaCausal {
    fn code(&self) -> TemplateCode { TemplateCode::NaCausal }
    fn default_share(&self) -> f32 { 0.10 }
    fn fusion_kernel(&self) -> FusionKernelId { FusionKernelId::Causal }
    fn consensus_weight(&self) -> f32 { 1.0 }
}
```

#### `NaTemp` — Temporal

```rust
// crates/efe-core/src/agents/templates/na_temp.rs

/// **Sprint D33** — Temporal-aware nano-agent template.
///
/// Encodes ordering constraints (before/after), durations
/// (hours/days/years), recurrence (daily/weekly), and timeline
/// reasoning. Pairs with NaCausal for causal-temporal queries.
///
/// Default share : 8 %. Activated when query contains temporal
/// markers ("yesterday", "last week", "after 2024", etc.) detected
/// at genesis time via a regex pre-filter (PII-free).
#[derive(Debug, Clone, Copy)]
pub struct NaTemp;

impl NanoAgentTemplate for NaTemp {
    fn code(&self) -> TemplateCode { TemplateCode::NaTemp }
    fn default_share(&self) -> f32 { 0.08 }
    fn fusion_kernel(&self) -> FusionKernelId { FusionKernelId::Temporal }
    fn consensus_weight(&self) -> f32 { 1.0 }
}
```

#### `NaSafety` — Safety / refusal

```rust
// crates/efe-core/src/agents/templates/na_safety.rs

/// **Sprint D33** — Safety / refusal nano-agent template.
///
/// Detects harmful, illegal, or alignment-violating content
/// patterns. Outputs a high-magnitude vector along the "refusal"
/// direction in latent space when triggered ; otherwise outputs
/// near-zero (lets other agents dominate).
///
/// Default share : 5 % — small but always present. Critical for
/// production deployments that must comply with content policies.
/// Consensus weight 2.0 (a single refusal vote should outweigh
/// most positive votes).
#[derive(Debug, Clone, Copy)]
pub struct NaSafety;

impl NanoAgentTemplate for NaSafety {
    fn code(&self) -> TemplateCode { TemplateCode::NaSafety }
    fn default_share(&self) -> f32 { 0.05 }
    fn fusion_kernel(&self) -> FusionKernelId { FusionKernelId::Safety }
    fn consensus_weight(&self) -> f32 { 2.0 } // refusal trumps most votes
}
```

#### `NaConsist` — Consistency

```rust
// crates/efe-core/src/agents/templates/na_consist.rs

/// **Sprint D33** — Internal-consistency nano-agent template.
///
/// Detects contradictions inside a single response (e.g. "X is true"
/// + "X is false" elsewhere). Used by Mode-3 to discount sub-swarms
/// whose own internal centroids disagree across rounds — a proxy
/// for "this swarm is unstable".
///
/// Default share : 7 %. Consensus weight 1.5 (consistent agents
/// boost the verdict ; inconsistent agents drag it down).
#[derive(Debug, Clone, Copy)]
pub struct NaConsist;

impl NanoAgentTemplate for NaConsist {
    fn code(&self) -> TemplateCode { TemplateCode::NaConsist }
    fn default_share(&self) -> f32 { 0.07 }
    fn fusion_kernel(&self) -> FusionKernelId { FusionKernelId::Consistency }
    fn consensus_weight(&self) -> f32 { 1.5 }
}
```

### §9.5 Extension de `TemplateCode`

```rust
// crates/efe-core/src/types.rs (PATCH D33)

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[repr(u8)]
#[non_exhaustive]
pub enum TemplateCode {
    // MVP set (D1-D9, immutable forever).
    NaSem = 0,
    NaLog = 1,
    NaCrit = 2,

    // **Sprint D33** — GA set. Numeric values are stable across
    // versions ; the CUDA buffers in efe-kernels encode these as u8.
    NaQual = 3,
    NaCausal = 4,
    NaTemp = 5,
    NaSafety = 6,
    NaConsist = 7,
}

impl TemplateCode {
    /// MVP-era set (D1-D9). Preserved for backward compat.
    pub const MVP_SET: &'static [Self] = &[Self::NaSem, Self::NaLog, Self::NaCrit];

    /// **Sprint D33** — full GA set (8 templates).
    pub const GA_SET: &'static [Self] = &[
        Self::NaSem, Self::NaLog, Self::NaCrit,
        Self::NaQual, Self::NaCausal, Self::NaTemp,
        Self::NaSafety, Self::NaConsist,
    ];

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::NaSem => "na-sem",
            Self::NaLog => "na-log",
            Self::NaCrit => "na-crit",
            Self::NaQual => "na-qual",
            Self::NaCausal => "na-causal",
            Self::NaTemp => "na-temp",
            Self::NaSafety => "na-safety",
            Self::NaConsist => "na-consist",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "na-sem" => Some(Self::NaSem),
            "na-log" => Some(Self::NaLog),
            "na-crit" => Some(Self::NaCrit),
            "na-qual" => Some(Self::NaQual),
            "na-causal" => Some(Self::NaCausal),
            "na-temp" => Some(Self::NaTemp),
            "na-safety" => Some(Self::NaSafety),
            "na-consist" => Some(Self::NaConsist),
            _ => None,
        }
    }
}
```

### §9.6 Extension `cardinality.rs`

`PopulationMix::balanced()` doit être étendu pour produire le
share par défaut sur les 8 templates :

```rust
// crates/efe-core/src/genesis/cardinality.rs (PATCH D33)

impl PopulationMix {
    /// **Sprint D33** — Construct a balanced GA mix using each
    /// template's `default_share()`. Sum should be ~1.0 ;
    /// `total_pop` is split proportionally.
    pub fn balanced_ga(total_pop: u32) -> Result<Self, GenesisError> {
        use crate::agents::templates::{
            NaCausal, NaConsist, NaCrit, NaLog, NaQual, NaSafety, NaSem, NaTemp,
            NanoAgentTemplate,
        };
        let templates: Vec<(TemplateCode, f32)> = vec![
            (NaSem.code(), NaSem.default_share()),
            (NaLog.code(), NaLog.default_share()),
            (NaCrit.code(), NaCrit.default_share()),
            (NaQual.code(), NaQual.default_share()),
            (NaCausal.code(), NaCausal.default_share()),
            (NaTemp.code(), NaTemp.default_share()),
            (NaSafety.code(), NaSafety.default_share()),
            (NaConsist.code(), NaConsist.default_share()),
        ];
        let per: Vec<(TemplateCode, u32)> = templates
            .into_iter()
            .map(|(t, share)| (t, ((share * total_pop as f32).round() as u32).max(1)))
            .collect();
        Self::new(per)
    }
}
```

### §9.7 Tests + pin tests D33

* **Unit tests** : 8 templates × 4 invariants (code, default_share,
  fusion_kernel, consensus_weight) = 32 tests dans
  `crates/efe-core/src/agents/templates/mod.rs::tests`.
* **Pin tests Python** : `scripts/tests/test_d33_templates.py` :
  * `test_template_code_has_8_variants` — grep enum.
  * `test_each_template_has_module` — file existence.
  * `test_each_template_implements_trait` — grep `impl NanoAgentTemplate`.
  * `test_balanced_ga_sums_to_unity` — sum default_share ≈ 1.0.
  * `test_safety_consensus_weight_is_2_0` — institutional invariant.
  * `test_mvp_set_preserved` — backward compat anti-regression.
  * 2 tests anti-régression sur les u8 stables (NaSem=0, NaLog=1, NaCrit=2).
* **Bench** : `bench_template_diversity` — exécute Mode-1 avec
  MVP_SET vs GA_SET sur les golden datasets D7 + mesure
  `accuracy_macro_f1` ; GA_SET doit être ≥ MVP_SET sans dégrader la
  latence p99 de plus de 8 %.

### §9.8 Critères Go/No-Go D33

* GG-D33.1 : 8 templates compilent + 32 unit tests GREEN.
* GG-D33.2 : Pin tests Python `test_d33_templates.py` GREEN (8 tests).
* GG-D33.3 : Bench `accuracy_macro_f1` ≥ baseline D7 + 0.0
  (pas de régression).
* GG-D33.4 : Latence p99 Mode-1 avec GA_SET ≤ p99 MVP_SET × 1.08
  (max 8 % overhead).
* GG-D33.5 : Backward compat — MVP_SET teste GREEN.

---

## §10. Sprint **D34** — Mode-2 backend (`Mode2Controller`)

### §10.1 Objectif

Livrer dans `efe-core` la couche algorithmique Mode-2 :

* `Mode2Controller<B>` orchestrateur générique sur backend `B`.
* `LlmRouter` trait (déjà conçu en M4 dans efe-server, à backporter).
* `MockRouter` impl pure-Rust (déterministe, pas de réseau).
* `Mode2Cache` `DashMap` pour cache anchors keyed sur xxhash64.
* `synthesis::build_synthetic_request` (backport de
  `services/synthetic_spec.rs`).

Les impls réseau `OpenAIRouter`, `AnthropicRouter` **restent dans
`efe-server`** car elles dépendent de `reqwest` (HTTP client). Le
trait étant dans `efe-core`, elles peuvent quand même implémenter
`LlmRouter` sans cycle de dépendance.

### §10.2 Architecture du module

```text
crates/efe-core/src/swarm/mode2/
├── mod.rs          # Mode2Controller + Mode2Verdict + RouterMetadata
├── router.rs       # LlmRouter trait + MockRouter
├── cache.rs        # Mode2Cache (DashMap-backed)
└── synthesis.rs    # build_synthetic_request (backport)
```

### §10.3 `Mode2Controller<B>`

```rust
// crates/efe-core/src/swarm/mode2/mod.rs

pub mod cache;
pub mod router;
pub mod synthesis;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::errors::{EfeError, Result};
use crate::swarm::strategies::ExecuteBackend;
use crate::swarm::verdict::Verdict;
use crate::types::RequestId;

pub use cache::Mode2Cache;
pub use router::{
    DynamicAnchor, LlmRouter, LlmRouterError, MockRouter, RouterOutput,
};
pub use synthesis::build_synthetic_request;

/// Default cache TTL — 5 minutes per M4 § 13.4.
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(5 * 60);

/// Generic Mode-2 controller — orchestrates cache + router + backend.
///
/// `B` is the swarm execution backend (typically `Arc<SwarmController>`
/// in production, or a stub in tests). `B: ExecuteBackend + 'static`
/// so the controller can be cloned and shared across tasks.
///
/// **Sprint D34** — backport of `efe-server::services::mode2::Mode2ServiceImpl`
/// minus the gRPC/auth/rate-limit concerns (those stay in efe-server).
pub struct Mode2Controller<B: ExecuteBackend + 'static> {
    backend: Arc<B>,
    routers: HashMap<String, Arc<dyn LlmRouter>>,
    cache: Mode2Cache,
}

impl<B: ExecuteBackend + 'static> Mode2Controller<B> {
    /// Construct with explicit cache TTL.
    pub fn with_ttl(
        backend: Arc<B>,
        routers: HashMap<String, Arc<dyn LlmRouter>>,
        cache_ttl: Duration,
    ) -> Self {
        Self {
            backend,
            routers,
            cache: Mode2Cache::new(cache_ttl),
        }
    }

    /// Construct with the M4 § 13.4 default TTL (5 minutes).
    pub fn new(
        backend: Arc<B>,
        routers: HashMap<String, Arc<dyn LlmRouter>>,
    ) -> Self {
        Self::with_ttl(backend, routers, DEFAULT_CACHE_TTL)
    }

    /// Default routers : `mock` only (no network deps in efe-core).
    /// Production callers (efe-server) inject `openai` / `anthropic`
    /// via `with_routers`.
    pub fn default_routers(dim: usize) -> HashMap<String, Arc<dyn LlmRouter>> {
        let mut m: HashMap<String, Arc<dyn LlmRouter>> = HashMap::new();
        m.insert(
            "mock".to_string(),
            Arc::new(MockRouter::with_dim(dim)) as Arc<dyn LlmRouter>,
        );
        m
    }

    /// Resolve a router by id ; map error to `EfeError::InvalidConfig`.
    pub(crate) fn resolve_router(&self, id: &str) -> Result<Arc<dyn LlmRouter>> {
        self.routers
            .get(id)
            .cloned()
            .ok_or_else(|| EfeError::InvalidConfig(
                format!("unknown llm_router_id: {id}"),
            ))
    }

    /// Execute a Mode-2 query end-to-end.
    ///
    /// Pipeline :
    ///   1. Resolve router by id.
    ///   2. Cache lookup (xxhash64(query, top_k, router_id)).
    ///   3. Generate anchors on miss.
    ///   4. Build synthetic SwarmSpec from (query, seed).
    ///   5. backend.execute(req).await.
    ///   6. Project to Mode2Verdict.
    pub async fn execute(
        &self,
        query_text: &str,
        top_k_anchors: u32,
        router_id: &str,
        tenant_id: &str,
        seed: u64,
        bypass_cache: bool,
    ) -> Result<Mode2Verdict> {
        let started = Instant::now();
        let router = self.resolve_router(router_id)?;
        let cache_key = Mode2Cache::key(query_text, top_k_anchors, router_id);

        let router_output = if bypass_cache {
            router
                .generate_anchors(query_text, top_k_anchors)
                .await
                .map_err(EfeError::from)?
        } else if let Some(hit) = self.cache.get(cache_key) {
            hit
        } else {
            let out = router
                .generate_anchors(query_text, top_k_anchors)
                .await
                .map_err(EfeError::from)?;
            self.cache.insert(cache_key, out.clone());
            out
        };

        // Mix the router's call_id into the seed so two distinct
        // routers produce distinct swarm trajectories on the same query.
        let mixed_seed = seed
            ^ router_output.llm_call_id.as_bytes().iter().fold(
                0u64,
                |acc, b| acc.wrapping_mul(0x100000001b3).wrapping_add(u64::from(*b)),
            );
        let req = build_synthetic_request(query_text, mixed_seed, tenant_id)?;
        let resp = self.backend.execute(req).await?;
        let total_ms = started.elapsed().as_secs_f32() * 1000.0;

        Ok(Mode2Verdict {
            verdict: resp.verdict,
            confidence: resp.confidence,
            request_id: resp.request_id,
            router: RouterMetadata {
                router_id: router_id.to_string(),
                anchors_generated: router_output.anchors.len() as u32,
                latency_ms: router_output.latency_ms,
                from_cache: router_output.from_cache,
                llm_call_id: router_output.llm_call_id,
            },
            total_ms,
        })
    }
}

/// Mode-2 verdict envelope. Mirrors `Verdict` but adds router metadata.
#[derive(Debug, Clone)]
pub struct Mode2Verdict {
    pub verdict: Verdict,
    pub confidence: f32,
    pub request_id: RequestId,
    pub router: RouterMetadata,
    pub total_ms: f32,
}

/// Router observability payload.
#[derive(Debug, Clone)]
pub struct RouterMetadata {
    pub router_id: String,
    pub anchors_generated: u32,
    pub latency_ms: u32,
    pub from_cache: bool,
    pub llm_call_id: String,
}
```

### §10.4 `LlmRouter` trait + `MockRouter`

```rust
// crates/efe-core/src/swarm/mode2/router.rs
//
// **Sprint D34** — Backport of `efe-server::services::mode2::router`.
// Externe routers (OpenAI, Anthropic) restent dans efe-server car
// dépendent de reqwest.

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::time::Instant;
use thiserror::Error;

use crate::errors::EfeError;

#[derive(Debug, Clone)]
pub struct DynamicAnchor {
    pub label: String,
    pub centroid: Vec<f32>,
    pub confidence: f32,
}

#[derive(Debug, Clone)]
pub struct RouterOutput {
    pub anchors: Vec<DynamicAnchor>,
    pub llm_call_id: String,
    pub from_cache: bool,
    pub latency_ms: u32,
}

#[derive(Debug, Error)]
pub enum LlmRouterError {
    #[error("router unavailable: {0}")]
    Unavailable(String),
    #[error("invalid router response: {0}")]
    InvalidResponse(String),
    #[error("top_k = {requested} exceeds cap {cap}")]
    TopKTooLarge { requested: u32, cap: u32 },
    #[error("dim mismatch: router={router_dim}, bundle={bundle_dim}")]
    DimMismatch { router_dim: usize, bundle_dim: usize },
    #[error("router I/O: {0}")]
    Io(String),
}

impl From<LlmRouterError> for EfeError {
    fn from(e: LlmRouterError) -> Self {
        EfeError::InvalidConfig(format!("LlmRouter: {e}"))
    }
}

#[async_trait]
pub trait LlmRouter: Send + Sync + std::fmt::Debug {
    fn id(&self) -> &'static str;

    async fn generate_anchors(
        &self,
        query_text: &str,
        top_k: u32,
    ) -> Result<RouterOutput, LlmRouterError>;

    async fn health(&self) -> Result<(), LlmRouterError> { Ok(()) }
}

/// Pure-Rust deterministic router. Produced anchors are SHA-256
/// derived ; same `(query, top_k)` ⇒ same output across runs.
#[derive(Debug, Clone)]
pub struct MockRouter {
    pub dim: usize,
    pub max_top_k: u32,
}

impl Default for MockRouter {
    fn default() -> Self { Self { dim: 384, max_top_k: 20 } }
}

impl MockRouter {
    pub fn with_dim(dim: usize) -> Self { Self { dim, ..Self::default() } }
}

#[async_trait]
impl LlmRouter for MockRouter {
    fn id(&self) -> &'static str { "mock" }

    async fn generate_anchors(
        &self,
        query_text: &str,
        top_k: u32,
    ) -> Result<RouterOutput, LlmRouterError> {
        if top_k > self.max_top_k {
            return Err(LlmRouterError::TopKTooLarge {
                requested: top_k,
                cap: self.max_top_k,
            });
        }
        let started = Instant::now();
        let mut anchors = Vec::with_capacity(top_k as usize);
        for idx in 0..top_k {
            let centroid = synth_centroid(query_text, idx, self.dim);
            anchors.push(DynamicAnchor {
                label: format!("mock_anchor_{idx}"),
                centroid,
                confidence: 1.0 - (idx as f32) * 0.05,
            });
        }
        let llm_call_id = format!(
            "mock-{}",
            &hex::encode(Sha256::digest(query_text.as_bytes()))[..16]
        );
        Ok(RouterOutput {
            anchors,
            llm_call_id,
            from_cache: false,
            latency_ms: started.elapsed().as_millis() as u32,
        })
    }
}

fn synth_centroid(query_text: &str, idx: u32, dim: usize) -> Vec<f32> {
    let mut hasher = Sha256::new();
    hasher.update(query_text.as_bytes());
    hasher.update(idx.to_le_bytes());
    let mut seed = hasher.finalize().to_vec();
    while seed.len() < dim * 4 {
        let next = Sha256::digest(&seed);
        seed.extend_from_slice(&next);
    }
    let mut v: Vec<f32> = seed
        .chunks_exact(4)
        .take(dim)
        .map(|c| {
            let raw = i32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            (raw as f32) / (i32::MAX as f32)
        })
        .collect();
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v { *x /= norm; }
    }
    v
}
```

### §10.5 `Mode2Cache` (DashMap-backed)

```rust
// crates/efe-core/src/swarm/mode2/cache.rs

use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use xxhash_rust::xxh64::Xxh64;

use super::router::RouterOutput;

#[derive(Clone)]
pub struct Mode2Cache {
    inner: Arc<DashMap<u64, CacheEntry>>,
    ttl: Duration,
}

#[derive(Clone)]
struct CacheEntry {
    output: RouterOutput,
    expires_at: Instant,
}

impl Mode2Cache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
            ttl,
        }
    }

    /// xxhash64 of (query, top_k, router_id).
    pub fn key(query: &str, top_k: u32, router_id: &str) -> u64 {
        let mut h = Xxh64::new(0);
        h.update(query.as_bytes());
        h.update(&top_k.to_le_bytes());
        h.update(router_id.as_bytes());
        h.digest()
    }

    pub fn get(&self, key: u64) -> Option<RouterOutput> {
        let entry = self.inner.get(&key)?;
        if entry.expires_at <= Instant::now() {
            return None;
        }
        let mut out = entry.output.clone();
        out.from_cache = true;
        Some(out)
    }

    pub fn insert(&self, key: u64, output: RouterOutput) {
        self.inner.insert(
            key,
            CacheEntry {
                output,
                expires_at: Instant::now() + self.ttl,
            },
        );
    }

    pub fn len(&self) -> usize { self.inner.len() }
    pub fn is_empty(&self) -> bool { self.inner.is_empty() }
}
```

### §10.6 `synthesis::build_synthetic_request`

Backport intégral de
`crates/efe-server/src/services/synthetic_spec.rs` vers
`crates/efe-core/src/swarm/mode2/synthesis.rs`. La logique est
identique ; seul le path d'import change (`crate::types::*` reste
identique parce que les deux crates partagent `efe_core::types`).

### §10.7 Tests + pin tests D34

* **Unit tests** (12) :
  * `mock_router_deterministic`, `mock_router_unit_norm`,
    `mock_router_top_k_cap`.
  * `cache_key_stable_per_triplet`, `cache_hit_flips_from_cache`,
    `cache_expires_after_ttl`.
  * `mode2_controller_resolves_router`, `mode2_controller_unknown_router`.
  * `mode2_controller_calls_backend_on_miss`,
    `mode2_controller_skips_backend_on_cache_hit_when_bypass_false`.
  * `mode2_verdict_carries_router_metadata`,
    `mode2_seed_mix_propagates_to_synthetic_spec`.
* **Pin tests Python** (`scripts/tests/test_d34_core_mode2.py`) :
  * Module structure, trait public, MockRouter exists.
  * Backport completeness — every helper from efe-server has a
    counterpart in efe-core.
* **Bench** (`crates/efe-bench/benches/mode2_e2e.rs`) :
  * Cache miss → router ms + backend ms ≤ 200 ms p99.
  * Cache hit → backend ms ≤ 50 ms p99.

### §10.8 Critères Go/No-Go D34

* GG-D34.1 : `Mode2Controller<B>` GREEN sur 12 unit tests.
* GG-D34.2 : Cache hit rate ≥ 60 % sur fixtures (SLO C-11).
* GG-D34.3 : Mode-2 p99 ≤ 1500 ms incl. mock router (SLO C-3).
* GG-D34.4 : `efe-server::services::mode2::Mode2ServiceImpl` refactor
  pour appeler `efe_core::Mode2Controller` (handler devient un thin
  wrapper de 80 LOC).
* GG-D34.5 : Pin tests `test_d34_core_mode2.py` GREEN.

---

## §11. Sprint **D37** — Mode-3 backend (`Mode3Controller`)

### §11.1 Objectif

Livrer dans `efe-core` :

* `Mode3Controller<B>` orchestrateur générique.
* `Mode3Aggregator` trait + 3 impls (`MeanAggregator`,
  `WeightedAggregator`, `MedianAggregator`).
* `SubVerdict` / `Mode3Verdict` / `AggregationMetadata` types.
* `K_MAX` const (= 5).
* `clamp_k(k)` utility.

Note : la parallélisation `tokio::JoinSet` est livrée dans le sprint
**D38** (cf. §12). D37 livre le contrôleur séquentiel ;
D38 ajoute le runner parallèle.

### §11.2 Architecture du module

```text
crates/efe-core/src/swarm/mode3/
├── mod.rs          # Mode3Controller + Mode3Verdict + clamp_k + K_MAX
├── aggregator.rs   # Mode3Aggregator trait + 3 impls
└── parallel.rs     # NEW D38 — tokio::JoinSet runner
```

### §11.3 `Mode3Controller<B>`

```rust
// crates/efe-core/src/swarm/mode3/mod.rs

pub mod aggregator;
pub mod parallel;       // D38

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::errors::{EfeError, Result};
use crate::swarm::mode2::Mode2Controller;
use crate::swarm::strategies::ExecuteBackend;
use crate::swarm::verdict::Verdict;
use crate::types::RequestId;

pub use aggregator::{
    AggregationError, AggregationOutput, MeanAggregator, MedianAggregator,
    Mode3Aggregator, SubVerdictData, WeightedAggregator,
};

/// SLO S-12 hard cap on the parallelism factor.
pub const K_MAX: u32 = 5;

/// Effective k after clamping to `[1, K_MAX]`.
pub fn clamp_k(k: u32) -> (u32, bool) {
    if k == 0 {
        (3, false)
    } else if k > K_MAX {
        (K_MAX, true)
    } else {
        (k, false)
    }
}

pub struct Mode3Controller<B: ExecuteBackend + 'static> {
    mode1_backend: Arc<B>,
    mode2_controller: Option<Arc<Mode2Controller<B>>>,
    aggregators: HashMap<String, Arc<dyn Mode3Aggregator>>,
}

impl<B: ExecuteBackend + 'static> Mode3Controller<B> {
    pub fn new(
        mode1_backend: Arc<B>,
        mode2_controller: Option<Arc<Mode2Controller<B>>>,
        aggregators: HashMap<String, Arc<dyn Mode3Aggregator>>,
    ) -> Self {
        Self { mode1_backend, mode2_controller, aggregators }
    }

    pub fn default_aggregators() -> HashMap<String, Arc<dyn Mode3Aggregator>> {
        let mut m: HashMap<String, Arc<dyn Mode3Aggregator>> = HashMap::new();
        m.insert("mean".to_string(), Arc::new(MeanAggregator) as _);
        m.insert("weighted".to_string(), Arc::new(WeightedAggregator) as _);
        m.insert("median".to_string(), Arc::new(MedianAggregator) as _);
        m
    }

    pub fn resolve_aggregator(&self, id: &str) -> Result<Arc<dyn Mode3Aggregator>> {
        self.aggregators.get(id).cloned().ok_or_else(|| {
            EfeError::InvalidConfig(format!("unknown aggregator: {id}"))
        })
    }

    /// Sequential k-swarm execution. **D38** wires the parallel
    /// runner ; D37 ships the sequential reference.
    pub async fn execute_sequential(
        &self,
        query_text: &str,
        k: u32,
        mode_per_call: &str,
        aggregator_id: &str,
        tenant_id: &str,
        seed: u64,
        top_k_anchors: u32,
        router_id: &str,
    ) -> Result<Mode3Verdict> {
        let started = Instant::now();
        let (k_eff, k_clamped) = clamp_k(k);
        let aggregator = self.resolve_aggregator(aggregator_id)?;

        let mut sub_verdicts = Vec::with_capacity(k_eff as usize);
        for i in 0..k_eff {
            let seed_i = seed.wrapping_add(i as u64);
            let sv = self
                .execute_one_sub_swarm(
                    i,
                    seed_i,
                    mode_per_call,
                    query_text,
                    tenant_id,
                    top_k_anchors,
                    router_id,
                )
                .await?;
            sub_verdicts.push(sv);
        }

        let agg_out = aggregator.aggregate(&sub_verdicts).map_err(EfeError::from)?;
        Ok(Mode3Verdict {
            final_verdict: agg_out.final_verdict,
            sub_verdicts,
            aggregation: agg_out.into(),
            request_id: RequestId::new(),
            total_ms: started.elapsed().as_secs_f32() * 1000.0,
            k_clamped,
            k_effective: k_eff,
        })
    }

    pub(crate) async fn execute_one_sub_swarm(
        &self,
        swarm_idx: u32,
        seed: u64,
        mode: &str,
        query_text: &str,
        tenant_id: &str,
        top_k_anchors: u32,
        router_id: &str,
    ) -> Result<SubVerdictData> {
        let started = Instant::now();
        let (verdict, confidence) = match mode {
            "mode1" => {
                let req = crate::swarm::mode2::synthesis::build_synthetic_request(
                    query_text, seed, tenant_id,
                )?;
                let resp = self.mode1_backend.execute(req).await?;
                (resp.verdict, resp.confidence)
            }
            "mode2" => {
                let m2 = self.mode2_controller.as_ref().ok_or_else(|| {
                    EfeError::InvalidConfig(
                        "mode_per_call=mode2 but Mode-2 controller not wired".into(),
                    )
                })?;
                let v = m2
                    .execute(query_text, top_k_anchors, router_id, tenant_id, seed, false)
                    .await?;
                (v.verdict, v.confidence)
            }
            other => {
                return Err(EfeError::InvalidConfig(format!(
                    "unknown mode_per_call: {other}"
                )));
            }
        };
        let latency_ms = started.elapsed().as_millis() as u32;
        Ok(SubVerdictData {
            swarm_idx,
            verdict,
            confidence,
            latency_ms,
            mode_used: mode.to_string(),
            seed_used: seed,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Mode3Verdict {
    pub final_verdict: Verdict,
    pub sub_verdicts: Vec<SubVerdictData>,
    pub aggregation: AggregationMetadata,
    pub request_id: RequestId,
    pub total_ms: f32,
    pub k_clamped: bool,
    pub k_effective: u32,
}

#[derive(Debug, Clone)]
pub struct AggregationMetadata {
    pub strategy: String,
    pub total_confidence: f32,
    pub dispersion: f32,
    pub effective_n: u32,
}

impl From<AggregationOutput> for AggregationMetadata {
    fn from(o: AggregationOutput) -> Self {
        Self {
            strategy: o.strategy,
            total_confidence: o.total_confidence,
            dispersion: o.dispersion,
            effective_n: o.effective_n,
        }
    }
}
```

### §11.4 `Mode3Aggregator` trait + 3 impls

Backport intégral de `efe-server::services::mode3::aggregator` vers
`crates/efe-core/src/swarm/mode3/aggregator.rs`. Aucune logique
modifiée : les 3 impls (Mean, Weighted, Median) restent identiques.

Différence clé : la valeur `verdict: i32` du `SubVerdictData` post-M4
(qui était la représentation proto i32) devient `verdict: Verdict`
(le rich enum pure-Rust). Le mapping i32 ↔ Verdict reste dans la
couche bridge de `efe-server`.

### §11.5 Tests + pin tests D37

* **Unit tests** (14) :
  * `clamp_k_*` (3 tests).
  * `MeanAggregator` (3 tests : pick, tie-break, dispersion).
  * `WeightedAggregator` (2 tests).
  * `MedianAggregator` (2 tests : robust, tie-break).
  * `Mode3Controller::execute_sequential` (4 tests : happy path,
    k_clamped, mode2_not_wired_fails, aggregator_not_found_fails).
* **Pin tests Python** (`test_d37_core_mode3.py`, 14 tests) :
  * Module structure, K_MAX = 5, 3 aggregators present.
  * Aggregator IDs distinct, Mode3Controller exposes `execute_sequential`.
  * Backport completeness vs efe-server.

### §11.6 Critères Go/No-Go D37

* GG-D37.1 : 14 unit tests GREEN.
* GG-D37.2 : `efe-server::services::mode3::Mode3ServiceImpl` refactor
  pour appeler `efe_core::Mode3Controller` (handler devient ~100 LOC).
* GG-D37.3 : Aggregator dispersion ∈ `[0, sqrt(k)/2]` (sanity check
  statistique).
* GG-D37.4 : Pin tests `test_d37_core_mode3.py` GREEN.

---

## §12. Sprint **D38** — Parallélisation k-swarm

### §12.1 Objectif

Livrer `swarm::mode3::parallel::run_k_swarms` qui lance k tasks
en parallèle via `tokio::task::JoinSet` et collecte les
résultats en arrival order (pas en submit order).

### §12.2 Surface publique

```rust
// crates/efe-core/src/swarm/mode3/parallel.rs

use std::future::Future;
use tokio::task::{JoinError, JoinSet};

use crate::errors::{EfeError, Result};

/// Spawn `k` async tasks via `tokio::task::JoinSet` and collect
/// results. Returns each task's result in **arrival order** (not
/// submission order).
///
/// **Sprint D38** — extracted from
/// `efe-server::services::mode3::Mode3ServiceImpl::execute` so it
/// becomes reusable by `efe-ffi`, `efe-bench`, and tests.
///
/// `task_factory(idx)` is called eagerly for `idx in 0..k` and
/// must produce a `Future<Output = Result<T>>`. The futures are
/// spawned onto the ambient tokio runtime ; the function awaits
/// `JoinSet::join_next` until all complete.
///
/// On the first error, the function aborts the remaining tasks
/// (best-effort cancellation) and returns the error. This
/// fail-fast behaviour matches the Mode-3 SLO contract :
/// dispersion across sub-swarms is informational, but a
/// fundamental error in any sub-swarm should fail the whole call.
///
/// # Errors
///
/// * Returns the first inner `Result::Err` from any task.
/// * Returns `EfeError::Internal("join error: ...")` on a
///   `tokio::task::JoinError` (panic in a sub-task).
pub async fn run_k_swarms<T, F, Fut>(k: u32, task_factory: F) -> Result<Vec<T>>
where
    T: Send + 'static,
    F: Fn(u32) -> Fut + Send + Sync,
    Fut: Future<Output = Result<T>> + Send + 'static,
{
    let mut set: JoinSet<Result<T>> = JoinSet::new();
    let mut abort_handles = Vec::with_capacity(k as usize);
    for i in 0..k {
        let fut = task_factory(i);
        let h = set.spawn(fut);
        abort_handles.push(h);
    }

    let mut out = Vec::with_capacity(k as usize);
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(Ok(v)) => out.push(v),
            Ok(Err(e)) => {
                // Fail-fast : abort remaining tasks then surface
                // the first error.
                for h in &abort_handles {
                    h.abort();
                }
                return Err(e);
            }
            Err(join_err) => {
                for h in &abort_handles {
                    h.abort();
                }
                return Err(map_join_error(join_err));
            }
        }
    }
    Ok(out)
}

fn map_join_error(e: JoinError) -> EfeError {
    if e.is_cancelled() {
        EfeError::Internal("sub-task cancelled")
    } else {
        EfeError::Internal("sub-task panicked")
    }
}
```

### §12.3 Intégration dans `Mode3Controller`

```rust
// crates/efe-core/src/swarm/mode3/mod.rs (PATCH D38)

impl<B: ExecuteBackend + 'static> Mode3Controller<B> {
    /// **Sprint D38** — parallel k-swarm execution via
    /// `tokio::task::JoinSet`. Replaces `execute_sequential` for
    /// production traffic. Sequential variant kept for benchmarking
    /// vs parallel speedup.
    pub async fn execute_parallel(
        self: Arc<Self>,
        query_text: &str,
        k: u32,
        mode_per_call: &str,
        aggregator_id: &str,
        tenant_id: &str,
        seed: u64,
        top_k_anchors: u32,
        router_id: &str,
    ) -> Result<Mode3Verdict> {
        let started = Instant::now();
        let (k_eff, k_clamped) = clamp_k(k);
        let aggregator = self.resolve_aggregator(aggregator_id)?;

        let query = query_text.to_string();
        let tenant = tenant_id.to_string();
        let mode = mode_per_call.to_string();
        let router_id_owned = router_id.to_string();

        let sub_verdicts = parallel::run_k_swarms(k_eff, |i| {
            let me = self.clone();
            let query = query.clone();
            let tenant = tenant.clone();
            let mode = mode.clone();
            let router_id_owned = router_id_owned.clone();
            async move {
                let seed_i = seed.wrapping_add(i as u64);
                me.execute_one_sub_swarm(
                    i, seed_i, &mode, &query, &tenant, top_k_anchors, &router_id_owned,
                )
                .await
            }
        })
        .await?;

        let agg_out = aggregator.aggregate(&sub_verdicts).map_err(EfeError::from)?;
        Ok(Mode3Verdict {
            final_verdict: agg_out.final_verdict,
            sub_verdicts,
            aggregation: agg_out.into(),
            request_id: RequestId::new(),
            total_ms: started.elapsed().as_secs_f32() * 1000.0,
            k_clamped,
            k_effective: k_eff,
        })
    }
}
```

### §12.4 Tests + pin tests D38

* **Unit tests** (6) :
  * `run_k_swarms_collects_all_results_when_happy`.
  * `run_k_swarms_returns_first_error`.
  * `run_k_swarms_aborts_remaining_on_first_error`.
  * `run_k_swarms_handles_panic_via_join_error`.
  * `run_k_swarms_arrival_order_not_submit_order`.
  * `run_k_swarms_zero_tasks_returns_empty_vec`.
* **Pin tests Python** (`test_d38_parallel.py`, 4 tests).
* **Bench** : `mode3_k_parallel_throughput` :
  * `parallel/sequential` speedup ≥ 2.5× pour k=5 sur 8 vCPU.
  * Latence p99 parallel ≤ 1.4 × latence p99 d'un sub-swarm
    (idéalement = max sub-swarm latency).

### §12.5 Critères Go/No-Go D38

* GG-D38.1 : 6 unit tests GREEN.
* GG-D38.2 : Bench speedup ≥ 2.5× pour k=5.
* GG-D38.3 : `efe-server::services::mode3` consomme `execute_parallel`
  au lieu de son propre `JoinSet`.
* GG-D38.4 : Pin tests `test_d38_parallel.py` GREEN.

---

## §13. Sprint **D32** cross-cutting — `HybridFusion`

### §13.1 Objectif

Livrer un trait + une impl par défaut qui exécute deux
`FusionStrategy` en parallèle (typiquement CPU primary + GPU shadow)
et compare leurs verdicts pour détecter les régressions GPU
pendant le rollout D28-D32.

### §13.2 Trait `HybridFusion`

```rust
// crates/efe-core/src/fusion/hybrid.rs

use async_trait::async_trait;
use std::sync::Arc;

use crate::els::EpistemicLatentSpace;
use crate::fusion::stats::FusionStats;
use crate::fusion::strategy::FusionStrategy;
use crate::memory::NanoAgentBatch;

/// Per-round comparison report of the primary and shadow fusion
/// strategies. Emitted at every fusion round when HybridFusion is
/// active. Consumed by `efe-telemetry::canary::HybridReporter`.
#[derive(Debug, Clone)]
pub struct HybridFusionReport {
    pub round: u32,
    pub primary_stats: FusionStats,
    pub shadow_stats: FusionStats,
    /// L2 distance between centroids (lower = better agreement).
    pub centroid_l2_diff: f32,
    /// Variance ratio shadow/primary (1.0 = perfect agreement).
    pub variance_ratio: f32,
    /// Confidence delta (shadow.confidence - primary.confidence).
    pub confidence_delta: f32,
    /// Wall time of the primary call (ms).
    pub primary_ms: f32,
    /// Wall time of the shadow call (ms).
    pub shadow_ms: f32,
}

impl HybridFusionReport {
    /// Returns true when the two strategies disagree significantly.
    /// Threshold tuned by D32 bench : centroid_l2 > 0.05 OR
    /// variance_ratio < 0.5 OR > 2.0.
    pub fn divergence_detected(&self) -> bool {
        self.centroid_l2_diff > 0.05
            || self.variance_ratio < 0.5
            || self.variance_ratio > 2.0
            || self.confidence_delta.abs() > 0.10
    }
}

/// Trait for fusion strategies that compose two underlying strategies.
/// Default impl is `CanaryHybridFusion`.
#[async_trait]
pub trait HybridFusion: Send + Sync + std::fmt::Debug {
    /// Run BOTH the primary and shadow strategies, compare, and
    /// return the primary's stats (the shadow is purely observational
    /// — fail-open per Principle P-8).
    async fn run_round_hybrid(
        &self,
        els: &EpistemicLatentSpace,
        batch: &NanoAgentBatch,
        round: u32,
    ) -> Result<(FusionStats, HybridFusionReport), crate::errors::EfeError>;
}

/// **Sprint D32** — Default canary hybrid impl. Fail-open : if the
/// shadow errors out, the primary's stats are still returned and a
/// warning is logged.
#[derive(Debug)]
pub struct CanaryHybridFusion<P, S>
where
    P: FusionStrategy + 'static,
    S: FusionStrategy + 'static,
{
    pub primary: Arc<P>,
    pub shadow: Arc<S>,
    pub config: CanaryConfig,
}

#[derive(Debug, Clone)]
pub struct CanaryConfig {
    /// Sampling rate (0.0..=1.0). 0.1 = run shadow on 10 % of
    /// rounds. Lower = lower overhead, less coverage.
    pub sampling_rate: f32,
    /// L2 divergence threshold. Reports above this are flagged.
    pub divergence_l2: f32,
    /// Whether to log a `tracing::error!` when divergence is
    /// detected (default true).
    pub log_on_divergence: bool,
}

impl Default for CanaryConfig {
    fn default() -> Self {
        Self {
            sampling_rate: 0.1,        // 10 %
            divergence_l2: 0.05,
            log_on_divergence: true,
        }
    }
}

#[async_trait]
impl<P, S> HybridFusion for CanaryHybridFusion<P, S>
where
    P: FusionStrategy + 'static,
    S: FusionStrategy + 'static,
{
    async fn run_round_hybrid(
        &self,
        els: &EpistemicLatentSpace,
        batch: &NanoAgentBatch,
        round: u32,
    ) -> Result<(FusionStats, HybridFusionReport), crate::errors::EfeError> {
        // Always run primary.
        let primary_started = std::time::Instant::now();
        let primary_stats = self
            .primary
            .run_round(els, batch, round)
            .await?;
        let primary_ms = primary_started.elapsed().as_secs_f32() * 1000.0;

        // Sample-gated shadow.
        let should_shadow = sample_gate(self.config.sampling_rate, round);
        let (shadow_stats, shadow_ms) = if should_shadow {
            let s_started = std::time::Instant::now();
            match self.shadow.run_round(els, batch, round).await {
                Ok(s) => (s, s_started.elapsed().as_secs_f32() * 1000.0),
                Err(e) => {
                    // Fail-open : log + return primary only.
                    tracing::warn!(
                        target: "efe.fusion.canary",
                        round,
                        error = %e,
                        "shadow fusion errored — fail-open ; primary verdict retained"
                    );
                    (primary_stats.clone(), 0.0)
                }
            }
        } else {
            (primary_stats.clone(), 0.0)
        };

        let report = HybridFusionReport {
            round,
            centroid_l2_diff: l2_diff(&primary_stats.centroid, &shadow_stats.centroid),
            variance_ratio: if primary_stats.variance > 0.0 {
                shadow_stats.variance / primary_stats.variance
            } else {
                1.0
            },
            confidence_delta: shadow_stats.confidence - primary_stats.confidence,
            primary_stats: primary_stats.clone(),
            shadow_stats,
            primary_ms,
            shadow_ms,
        };

        if report.divergence_detected() && self.config.log_on_divergence {
            tracing::error!(
                target: "efe.fusion.canary",
                round = report.round,
                centroid_l2 = report.centroid_l2_diff,
                variance_ratio = report.variance_ratio,
                conf_delta = report.confidence_delta,
                "GPU/CPU shadow divergence detected"
            );
        }

        Ok((primary_stats, report))
    }
}

fn sample_gate(rate: f32, round: u32) -> bool {
    // Deterministic gate based on round for reproducibility.
    let h = (round.wrapping_mul(2654435761)) as f32 / u32::MAX as f32;
    h < rate
}

fn l2_diff(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return f32::INFINITY;
    }
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).powi(2))
        .sum::<f32>()
        .sqrt()
}
```

### §13.3 Intégration runtime

```rust
// crates/efe-server/src/runtime.rs (PATCH D32)

// When EFE_FUSION_CANARY_ENABLED=true and EFE_FUSION_BACKEND=cpu+gpu :
let primary = Arc::new(MvpCpuFusion::new(...));
let shadow = Arc::new(CudaFusion::new(...));  // requires --features cuda
let hybrid = CanaryHybridFusion {
    primary,
    shadow,
    config: CanaryConfig {
        sampling_rate: parse_env("EFE_FUSION_CANARY_RATE").unwrap_or(0.1),
        ..Default::default()
    },
};

// hybrid implements HybridFusion ; the SwarmController accepts
// either FusionStrategy or HybridFusion via a small adapter.
```

### §13.4 Tests + pin tests D32

* **Unit tests** (7) :
  * `canary_runs_primary_only_when_sampling_rate_zero`.
  * `canary_runs_both_when_sampling_rate_one`.
  * `canary_failopen_on_shadow_error`.
  * `canary_divergence_threshold_l2`.
  * `canary_divergence_threshold_variance_ratio`.
  * `canary_divergence_threshold_confidence`.
  * `canary_report_carries_all_metrics`.
* **Pin tests** (`test_d32_hybrid_fusion.py`, 5 tests).
* **Bench** : `hybrid_canary_overhead` — au sampling 10 %, latence
  p99 ≤ 1.05 × latence primary p99.

### §13.5 Critères Go/No-Go D32

* GG-D32.1 : 7 unit tests GREEN.
* GG-D32.2 : Overhead ≤ 5 % au sampling 10 %.
* GG-D32.3 : Divergence detection alerte sur 100 % des cas où le GPU
  produit un verdict différent (test reproductible).
* GG-D32.4 : Pin tests `test_d32_hybrid_fusion.py` GREEN.

---

## §14. Migration `efe-server` → `efe-core` (backport plan)

### §14.1 Stratégie

La migration se fait **sprint par sprint**, jamais en big-bang :

1. **D34 ship** : `Mode2Controller` est livré dans efe-core ;
   `efe-server::Mode2ServiceImpl::execute_inner` est refactoré pour
   appeler `Mode2Controller::execute`.
2. **D37 ship** : `Mode3Controller` est livré ; `Mode3ServiceImpl`
   refactoré.
3. **D38 ship** : `parallel::run_k_swarms` ; `Mode3ServiceImpl`
   passe de son propre `JoinSet` à l'API publique efe-core.

### §14.2 Refactor `Mode2ServiceImpl` (post-D34)

AS-IS (post-M4 audit) — 786 LOC dans `services/mode2/mod.rs`.

TO-BE (post-D34) — ~280 LOC :

```rust
// crates/efe-server/src/services/mode2/mod.rs (POST-D34)

pub struct Mode2ServiceImpl {
    controller: Arc<efe_core::Mode2Controller<DynBackend>>,
    auth: Arc<AuthGuard>,
    rate_limiter: Option<Arc<BucketRegistry>>,
}

impl Mode2ServiceImpl {
    pub fn new(
        controller: Arc<efe_core::Mode2Controller<DynBackend>>,
        auth: Arc<AuthGuard>,
        rate_limiter: Option<Arc<BucketRegistry>>,
    ) -> Self {
        Self { controller, auth, rate_limiter }
    }
}

#[tonic::async_trait]
impl Mode2Service for Mode2ServiceImpl {
    async fn execute(
        &self,
        mut req: TRequest<Mode2ExecuteRequest>,
    ) -> Result<TResponse<Mode2ExecuteResponse>, Status> {
        let started = Instant::now();
        // Auth + scope + rate limit (unchanged) ...
        let inner = req.into_inner();

        // Delegate to efe-core Mode2Controller.
        let verdict = self.controller
            .execute(
                &inner.query_text, top_k, &router_id, &tenant,
                inner.seed, inner.bypass_cache,
            )
            .await
            .map_err(efe_core_err_to_status)?;

        // Project to proto Mode2ExecuteResponse (i32 verdict mapping).
        let resp = Mode2ExecuteResponse {
            verdict: verdict_to_proto_i32(&verdict.verdict),
            confidence: verdict.confidence,
            request_id: verdict.request_id.as_str().to_string(),
            total_ms: verdict.total_ms,
            router: Some(RouterMetadata {
                router_id: verdict.router.router_id,
                anchors_generated: verdict.router.anchors_generated,
                latency_ms: verdict.router.latency_ms,
                from_cache: verdict.router.from_cache,
                llm_call_id: verdict.router.llm_call_id,
            }),
        };
        emit_request_completion(EXECUTE_PATH, &tenant, &Ok(()), started);
        Ok(TResponse::new(resp))
    }
    // execute_streaming, router_health follow the same pattern
}
```

LOC réduction : 786 → ~280 = **-506 LOC** dans efe-server.

### §14.3 Refactor `Mode3ServiceImpl` (post-D37+D38)

AS-IS — 798 LOC.

TO-BE — ~290 LOC. Même pattern : appel direct à
`Mode3Controller::execute_parallel`, suppression du `JoinSet` local,
suppression des aggregators (importés depuis efe-core).

LOC réduction : 798 → ~290 = **-508 LOC**.

### §14.4 Suppression `services/synthetic_spec.rs`

Backport intégral vers `crates/efe-core/src/swarm/mode2/synthesis.rs`.
Le fichier `efe-server` est supprimé. Imports updatés dans tous les
sites consommateurs.

LOC réduction : 191 → 0 = **-191 LOC**.

### §14.5 Total LOC migration

| Surface | Avant D34/D37/D38 | Après | Δ |
|---|--:|--:|--:|
| `efe-server::services::mode2/` | 1 188 | 280 | **-908** |
| `efe-server::services::mode3/` | 1 163 | 290 | **-873** |
| `efe-server::services::synthetic_spec.rs` | 191 | 0 | **-191** |
| `efe-core::swarm::mode2/` | 0 | 1 200 | **+1 200** |
| `efe-core::swarm::mode3/` | 0 | 1 200 | **+1 200** |
| `efe-core::swarm::mode3::parallel.rs` | 0 | 280 | **+280** |
| `efe-core::fusion::hybrid.rs` | 0 | 360 | **+360** |
| `efe-core::agents/templates/` | 0 | 600 | **+600** |
| **NET** | **2 542** | **4 210** | **+1 668** |

Volumétrie nette **+1 668 LOC** mais **architecture propre** (transport
↔ algorithme).

---

## §15. Coexistence avec les 3 templates MVP

### §15.1 Backward compat invariants

* `TemplateCode::MVP_SET` reste accessible dans v0.2+ (D33 livré).
* `TemplateCode::NaSem`, `NaLog`, `NaCrit` gardent leurs `as_u8()`
  values (`0`, `1`, `2`).
* `PopulationMix::balanced()` (3 templates) reste accessible ;
  `PopulationMix::balanced_ga()` (8 templates) est l'API recommandée
  post-D33.
* Les golden datasets D7/D8 (catalog `support_triage`,
  `fraud_alert`, `billing_escalation`) tournent toujours avec
  MVP_SET sans modification.

### §15.2 Anti-régression

Pin test `scripts/tests/test_d33_backward_compat.py` :

* `test_mvp_set_unchanged` — assert `MVP_SET.len() == 3`.
* `test_mvp_template_u8_stable` — assert NaSem=0, NaLog=1, NaCrit=2.
* `test_mvp_template_strings_stable` — `"na-sem"`, `"na-log"`, `"na-crit"`.
* `test_balanced_3_templates_still_works` —
  `PopulationMix::balanced(60).is_ok()` avec 3 templates.

### §15.3 Migration progressive

Catalogs en production avec MVP_SET continuent à tourner. Les nouveaux
catalogs peuvent opter pour GA_SET via la config catalog (champ
`template_set: "mvp" | "ga"`). Default reste `mvp` pour 6 mois après
D33 ship.

---

## §16. Tests + pin tests structurels

### §16.1 Inventaire tests post-D38

| Sprint | Unit tests Rust | Pin tests Python | Bench |
|---|--:|--:|---|
| D33 templates | 32 | 8 | accuracy_template_diversity |
| D34 Mode-2 | 12 | 12 | mode2_e2e_latency |
| D37 Mode-3 | 14 | 14 | mode3_e2e_latency |
| D38 parallel | 6 | 4 | mode3_k_parallel_throughput |
| D32 HybridFusion | 7 | 5 | hybrid_canary_overhead |
| **Total** | **71** | **43** | **5 scenarios** |

### §16.2 Pin tests anti-régression institutionnels

Chaque sprint ajoute UN pin test qui vérifie que l'institution
"wiring forgotten" recurrence ne se répète pas :

```python
# scripts/tests/test_dXX_wiring_pin.py
def test_no_dead_field_in_modeN_controller():
    """Mode2Controller / Mode3Controller fields MUST be referenced
    in `execute()` body. Pre-fix audits found dead `mode2: Option<...>`
    field in Mode3ServiceImpl that was stored but never used (M4-A2).
    """
    rs = read("crates/efe-core/src/swarm/modeN/mod.rs")
    body = extract_fn_body(rs, "async fn execute")
    fields = extract_struct_fields(rs, "ModeNController")
    for field in fields:
        assert field in body, f"dead field: {field}"
```

### §16.3 Tests d'intégration

`tests/d34_d37_d38_integration.rs` (~400 LOC) :

* Boot une instance complète `Mode3Controller` avec
  `MockBackend` qui retourne des verdicts déterministes.
* Exécute Mode-2 et Mode-3 via les contrôleurs (pas via gRPC).
* Vérifie déterminisme (même seed → mêmes outputs sur 100 calls).
* Vérifie cache hit rate ≥ 60 % sur 1000 requêtes synthétiques.
* Vérifie speedup parallel ≥ 2.5× pour k=5.

---

## §17. Critères Go/No-Go par sprint

| Sprint | Critère principal | Métrique | Cible |
|---|---|---|---|
| D33 | Accuracy non-régressée | macro_f1 D7 baseline | ≥ baseline |
| D33 | Latence overhead | p99 GA vs MVP | ≤ 1.08× |
| D34 | Cache hit rate | Mode-2 fixtures | ≥ 60 % |
| D34 | Latence p99 | Mock router | ≤ 1500 ms |
| D34 | LOC migration | efe-server::mode2 | ≤ 280 |
| D37 | Aggregator dispersion | bound | `[0, sqrt(k)/2]` |
| D37 | LOC migration | efe-server::mode3 | ≤ 290 |
| D38 | Speedup parallel | k=5 vs sequential | ≥ 2.5× |
| D38 | Latence p99 | parallel vs sub-swarm | ≤ 1.4× max sub |
| D32 | Canary overhead | sampling 10 % | ≤ 5 % |
| D32 | Divergence detection | reproductible | 100 % |

---

## §18. Risques transverses + mitigations

| ID | Risque | Probabilité | Impact | Mitigation |
|----|---|:--:|:--:|---|
| **C-R-1** | LlmRouter trait dans efe-core force la dep `async-trait` (déjà présente) | Basse | Faible | Already in workspace deps |
| **C-R-2** | DashMap dep ajoutée à efe-core (prod-grade concurrent map) | Basse | Faible | DashMap est testé en prod par many — pas de risque |
| **C-R-3** | xxhash-rust dep ajoutée (non standard) | Basse | Faible | Hash uniquement — pas critique sécurité |
| **C-R-4** | Mode3Controller: génériques `<B>` lourds à debug | Moyenne | Moyen | Type alias `DynBackend = Arc<dyn ExecuteBackend>` pour caller-side simplif |
| **C-R-5** | Refactor efe-server casse les pin tests M4 | Haute | Moyen | Pin tests M4 mis à jour en même temps que migration ; CI gate |
| **C-R-6** | Backport Mode-2/3 perd un audit fix discrètement | Moyenne | Élevé | Pin tests régression M4-A1..A8 répliqués côté efe-core |
| **C-R-7** | Templates GA_SET dégrade accuracy MVP datasets | Moyenne | Élevé | Default reste MVP_SET ; opt-in GA_SET |
| **C-R-8** | HybridFusion overhead > 5 % en production | Moyenne | Moyen | Sampling 10 % par défaut ; tunable via env |
| **C-R-9** | tokio::JoinSet panic si runtime threadé absent | Basse | Élevé | `parallel::run_k_swarms` documente l'exigence runtime |
| **C-R-10** | LlmRouter externes (OpenAI/Anthropic) restent dans efe-server : asymmetrie | Faible | Faible | Documenté section §1.2 ; trait commun garantit compat |

---

## §19. SLO budget

### §19.1 SLO efe-core post-D38

| ID | Description | Cible | Mesure |
|---|---|---|---|
| C-1 | Mode-1 p99 latence | ≤ 200 ms | bench `real_strategies_e2e` |
| C-2 | Mode-2 p99 latence (mock router) | ≤ 1500 ms | bench `mode2_e2e_latency` |
| C-3 | Mode-3 p99 latence | ≤ 3 × Mode-1 p99 | bench `mode3_e2e_latency` |
| C-4 | Cache hit rate Mode-2 | ≥ 60 % | bench `mode2_cache_hit` |
| C-5 | k_max enforced | k ≤ 5 | unit test |
| C-6 | Determinism (same seed) | 100 % | pin test |
| C-7 | HybridFusion overhead | ≤ 5 % | bench `hybrid_canary_overhead` |
| C-8 | HybridFusion divergence detection | 100 % reproductible | unit test |
| C-9 | Parallel speedup (k=5) | ≥ 2.5× | bench `mode3_k_parallel_throughput` |
| C-10 | Templates GA accuracy | ≥ MVP baseline | bench `accuracy_template_diversity` |

---

## §20. Volumétrie code attendue

```
Sprint     LOC delta  Tests   Bench
─────────────────────────────────────
D33          +600     32      1
D34         +1200     12      1
D37         +1200     14      1
D38          +430      6      1
D32          +500      7      1
Migration   −1972      0      0   (extraction efe-server)
─────────────────────────────────────
Net total   +1958     71      5

Pin tests Python (43 tests) :
  test_d33_templates.py             8
  test_d33_backward_compat.py       4
  test_d34_core_mode2.py           12
  test_d37_core_mode3.py           14
  test_d38_parallel.py              4
  test_d32_hybrid_fusion.py         5
  test_dXX_wiring_pin.py            (institutional, 1 per sprint)
```

---

## §21. Plans d'implémentation détaillés sprint-par-sprint

### §21.1 D33 (5 jours)

| Jour | Tâche | Livrable |
|---|---|---|
| J-1 | Extraire types.rs::TemplateCode AS-IS, créer mod.rs + trait | trait NanoAgentTemplate |
| J-2 | Refactor 3 existants (NaSem/NaLog/NaCrit) en modules | 3 fichiers .rs |
| J-3 | Implémenter NaQual + NaCausal + NaTemp | 3 fichiers .rs + tests |
| J-4 | Implémenter NaSafety + NaConsist + cardinality étendu | 2 fichiers .rs + cardinality patch |
| J-5 | Pin tests Python + bench accuracy | tests + baseline frozen |

### §21.2 D34 (8 jours)

| Jour | Tâche | Livrable |
|---|---|---|
| J-1 | Setup `crates/efe-core/src/swarm/mode2/` dir + mod.rs | structure |
| J-2 | Backport `synthesis.rs` (synthetic_spec) | synthesis.rs + tests |
| J-3 | Backport `LlmRouter` trait + `MockRouter` | router.rs + 5 tests |
| J-4 | Implement `Mode2Cache` + xxhash key | cache.rs + 4 tests |
| J-5 | Implement `Mode2Controller<B>::execute` | mod.rs + 5 tests |
| J-6 | Bench `mode2_e2e_latency` | bench harness |
| J-7 | Refactor `efe-server::Mode2ServiceImpl` → use core | -506 LOC server |
| J-8 | Audit pass + pin tests + ship | release notes |

### §21.3 D37 (8 jours)

Idem pattern D34, focus aggregator + Mode3Controller séquentiel.

### §21.4 D38 (3 jours)

| Jour | Tâche | Livrable |
|---|---|---|
| J-1 | `parallel::run_k_swarms<F>` impl + 6 unit tests | parallel.rs |
| J-2 | `Mode3Controller::execute_parallel` + bench | speedup mesuré |
| J-3 | Refactor efe-server pour appeler execute_parallel | -150 LOC server |

### §21.5 D32 (4 jours)

| Jour | Tâche | Livrable |
|---|---|---|
| J-1 | `HybridFusion` trait + `HybridFusionReport` | trait + struct |
| J-2 | `CanaryHybridFusion<P, S>` impl + sampling | impl + 7 tests |
| J-3 | Bench `hybrid_canary_overhead` | overhead ≤ 5 % |
| J-4 | Pin tests + integration | release notes |

---

## §22. Annexes — trait + struct definitions complètes

### §22.1 `Mode2Controller<B>` trait bounds

```rust
impl<B> Mode2Controller<B>
where
    B: ExecuteBackend + Send + Sync + 'static,
{ /* ... */ }
```

### §22.2 `Mode3Controller<B>` trait bounds

```rust
impl<B> Mode3Controller<B>
where
    B: ExecuteBackend + Send + Sync + 'static,
{ /* ... */ }
```

### §22.3 `LlmRouter` complete signature

(Voir §10.4)

### §22.4 `Mode3Aggregator` complete signature

```rust
pub trait Mode3Aggregator: Send + Sync + std::fmt::Debug {
    fn id(&self) -> &'static str;
    fn aggregate(
        &self,
        sub_verdicts: &[SubVerdictData],
    ) -> Result<AggregationOutput, AggregationError>;
}
```

### §22.5 `HybridFusion` complete signature

(Voir §13.2)

---

## §23. Annexes — RACI + budget

| Sprint | Lead (R) | Approver (A) | Consulted (C) | Informed (I) |
|---|---|---|---|---|
| D33 | rodrigue.pekar | tech-lead | data-team (template diversity) | product, SRE |
| D34 | rodrigue.pekar | tech-lead | LLM-team (router contract) | product |
| D37 | rodrigue.pekar | tech-lead | bench-team | product |
| D38 | rodrigue.pekar | tech-lead | SRE (tokio runtime) | — |
| D32 | rodrigue.pekar | tech-lead | GPU-team | SRE, product |

Budget total : ~28 jours-ingénieur (5 sprints × moyenne 5,6 jours).

---

## §24. Annexes — Gantt prévisionnel D33 → D38

```text
Semaine    1     2     3     4     5     6
        ┌────┬────┬────┬────┬────┬────┐
D33     ████      │     │     │     │
D34          ████████   │     │     │
D32          ██████          (parallel D33)
D37                   ████████
D38                          ████  
        └────┴────┴────┴────┴────┴────┘
GA      ............................★ v1.0
```

Flexibilité :

* D32 cross-cutting **peut shipper en parallèle de D34** (touchent
  des fichiers disjoints).
* D38 **doit shipper après D37** (dépendance de surface API).
* D33 est le **prerequisite** pour les benchs accuracy de D34/D37.

---

## §25. Mises à jour du document

### §25.1 Versioning

* v1.0.0 — 2026-04-29 — Première édition (cette version).
* v1.x.0 — bump mineur quand un sprint ship.
* v2.0.0 — quand le contrat trait `LlmRouter` ou `Mode3Aggregator`
  change (breaking).

### §25.2 Liens à maintenir

* `M1_ARCHITECTURE.md` — première édition Mode-1 MVP (D1-D9).
* `EFE_SERVER_ARCHITECTURE.md` § 14 — section migration efe-server →
  efe-core post-D38.
* `EFE_FFI_ARCHITECTURE.md` § 13 — section "Mode-2/3 in-process"
  consommateur de cette extension.
* `bench_baselines/v0.X/baseline.json` — frozen post-D38.

### §25.3 Critères d'archive

Le présent document est archivé (déplacé dans `docs/archive/`) quand :

* Les 5 sprints D33-D38 ont shippé.
* Le `CHANGELOG.md` `[1.0.0]` entry est livré.
* L'audit pass M1 est terminé.
* Le tag `efe-core@1.0.0` est créé.

---

**Fin du document.**

Total LOC du document : ~1 650 lignes Markdown.
Total LOC delta cible (M1 extension) : ~3 750 LOC Rust + ~600 LOC tests
Python.
Total durée : ~28 jours-ingénieur sur 5 sprints non-contigus
(Φ-D → Φ-F).
