# `efe-telemetry` — Architecture détaillée de l'extension observabilité (M5)

> **Statut** : Draft v1.0.0 — Document de référence pour l'achèvement
> du module **M5 = `efe-telemetry`** entre l'AS-IS (post-D18, 31
> instruments + 7 dashboards, ~9 700 LOC) et la cible **GA 1.0**
> (50 instruments + 11 dashboards).
>
> **Auteur** : rodrigue.pekar@icloud.com
> **Date** : 2026-04-29.
> **Dépendances doc** :
>   * `docs/PRODUCT_COMPLETION_ARCHITECTURE.md` — roadmap globale.
>   * `docs/EFE_KERNELS_ARCHITECTURE.md` — substrat GPU CUDA (instruments
>     attendus côté efe-telemetry pour D29).
>   * `docs/EFE_SERVER_ARCHITECTURE.md` — extension gRPC M4 (consomme
>     les instruments Mode-2 / Mode-3).
>   * `docs/EFE_CORE_EXTENSION_ARCHITECTURE.md` — extension M1 (consomme
>     les instruments nano-agent + Mode-2/3 + canary).
>   * `docs/bench/d13_observability_dashboards_architecture.md` —
>     première édition Phase 1 + Phase 2 dashboards (D13).
>
> **Portée** : 3 sprints non-contigus s'étalant sur **2 phases**
> (Φ-D / Φ-E / Φ-F). Architecture **complémentaire** aux autres
> extensions M1/M2/M4 livrées en parallèle :
>
>   * **D29** (Φ-D) — 6 instruments GPU + dashboard GPU canary.
>   * **D34** (Φ-E) — 4 instruments nano-agent + 5 instruments
>     Mode-2 + 2 dashboards (templates + Mode-2).
>   * **D37** (Φ-F) — 4 instruments Mode-3 + 1 dashboard.
>
> **Cible GA** : **50 instruments + 11 dashboards**.

---

## Table des matières

§0. Méta, glossaire, conventions
§1. Mission de l'extension M5 (Φ-D → Φ-F)
§2. Position dans la topologie EFE post-M4
§3. État AS-IS au 2026-04-29 (post-D18 + post-M4 audit + post-M1 audit)
§4. Définition opérationnelle de M5 v1.0 GA
§5. Principes directeurs
§6. Invariants structurels
§7. Vue d'ensemble — 3 sprints, dependency graph
§8. Surface publique cible (50 instruments + 11 dashboards)
§9. **Sprint D29 — 6 instruments GPU + dashboard canary**
§10. **Sprint D34 — 4 nano-agent + 5 Mode-2 instruments + 2 dashboards**
§11. **Sprint D37 — 4 instruments Mode-3 + 1 dashboard**
§12. Dashboards Grafana — JSON skeleton
§13. SLO produit final (mapping instruments → SLOs)
§14. Cardinality budget (anti-explosion labels)
§15. Dépendances inter-sprints
§16. Tests + pin tests structurels
§17. Critères Go/No-Go par sprint
§18. Risques transverses + mitigations
§19. Volumétrie code attendue
§20. Plans d'implémentation détaillés
§21. Annexes — instruments specs complètes (50 entrées)
§22. Annexes — dashboard JSON skeleton (4 fichiers)
§23. Annexes — RACI + budget
§24. Annexes — Gantt prévisionnel D29 → D37
§25. Mises à jour du document

---

## §0. Méta, glossaire, conventions

### §0.1 Conventions

* **AS-IS** = état du code à fin D18 + post-M4 audit + post-M1 audit,
  v0.1.19+M4-audit-1+M1-audit-1.
* **TO-BE** = état cible défini par chaque sprint M5 extension.
* Les sprints D29, D34, D37 sont **non-contigus** : des sprints
  intercalaires (D30-D31 SDK, D33 templates, D35-D36 LLM routers)
  ne touchent pas `efe-telemetry` directement.
* Les défauts résiduels sont notés **DT-T.x** (Defect Trace,
  Telemetry).
* **Instrument** = une métrique Prometheus typée
  (Counter / Gauge / Histogram / Summary).
* **Dashboard** = un fichier JSON Grafana exporté vers
  `deploy/observability/grafana/dashboards/`.
* **Cardinality** = nombre total de séries Prometheus produites par
  un instrument = produit cartésien des labels possibles.
* `[ref:fichier:ligne]` = pointeur dans le code source.

### §0.2 Glossaire

| Terme | Définition |
|-------|------------|
| **`efe-telemetry`** | Le crate Rust `crates/efe-telemetry/` qui héberge l'observability stack : Prometheus exporter + OpenTelemetry collector + structured logs + event sinks. |
| **Prometheus** | Format texte standard pour les métriques temps-séries. Format `metric_name{label1="v1",...} value timestamp`. Cf. `prometheus.io`. |
| **OTel** | OpenTelemetry — standard W3C pour traces + métriques + logs avec propagation cross-service. Cf. `opentelemetry.io`. |
| **Counter** | Instrument monotone (jamais décroissant) qui compte des occurrences. Ex : `efe_grpc_requests_total`. |
| **Gauge** | Instrument qui peut monter et descendre. Ex : `efe_gpu_memory_used_bytes`. |
| **Histogram** | Instrument qui agrège des distributions (p50, p99, …). Ex : `efe_grpc_request_duration_seconds`. |
| **Label** | Dimension d'un instrument (ex : `tenant_id`, `method`, `gpu_id`). Le produit cartésien des labels = cardinality. |
| **Cardinality budget** | Limite max de séries par instrument (default 50 000 d'après D13 § 4). Au-dessus, Prometheus s'effondre. |
| **`register_all_dXX`** | Fonction par sprint qui force l'enregistrement eager des instruments au boot. Pattern institutionnel : sans elle, les panels Grafana affichent "no data" jusqu'à la première utilisation. |
| **Dashboard provisioning** | Mécanisme Helm qui installe les JSON dashboards via ConfigMap `grafana-dashboards`. Cf. `deploy/helm/efe/templates/grafana-dashboards.yaml`. |
| **Canary** | Pattern de rollout progressif : une partie du trafic va sur la nouvelle version, le reste sur l'ancienne, comparaison automatique. |

### §0.3 Cible matérielle

`efe-telemetry` tourne **in-process** dans `efe-server` via :

* Sidecar HTTP pour `/metrics` (port 9090 par défaut).
* `tracing-subscriber` global pour les logs JSON.
* `OtelCollector` async qui drain les spans vers un endpoint externe
  (Jaeger / Tempo / DataDog).

L'extension M5 n'introduit **aucune nouvelle dépendance externe** :
les 6 GPU instruments lisent les `KernelCallGuard` events de
`efe-kernels` (déjà en place) ; les Mode-2/Mode-3 instruments
consomment les métriques émises par les contrôleurs `efe-core` ;
les nano-agent instruments lisent les counters per-template du
genesis spawner.

### §0.4 Cibles fonctionnelles GA

```
GA v1.0 efe-telemetry (M5) :
├── Instruments (50 total)
│   ├── D1-D18 (existants, 31)
│   │   ├── d13_instruments      14 (M11 framework)
│   │   ├── d13bis_instruments    7 (catalog admin)
│   │   ├── d14_instruments       8 (postgres catalog)
│   │   ├── d14bis_instruments   11 (audit HMAC)
│   │   ├── d15_instruments      13 (OIDC)
│   │   ├── d16_instruments      16 (embedder bundle)
│   │   ├── d17_instruments      10 (backup/DR)
│   │   └── d18_instruments      16 (multi-replica)
│   │   = 95 grep matches, 31 distinct instruments after dedup
│   │     (some modules share helpers ; the canonical count is in
│   │     `metrics/registry.rs::register_all` deduped surface).
│   │
│   └── M5 extension (NEW, 19)
│       ├── gpu_instruments        6  (D29, NEW dir)
│       ├── nano_agent_instruments 4  (D34, NEW dir)
│       ├── mode2_instruments      5  (D34, NEW dir)
│       └── mode3_instruments      4  (D37, NEW dir)
│
├── Dashboards Grafana (11 total)
│   ├── D13 era (existants, 7)
│   │   ├── efe-overview.json
│   │   ├── efe-fleet.json
│   │   ├── efe-per-tenant.json
│   │   ├── per-tenant-dashboard.json
│   │   ├── d13/02_tenant_drill_down.json
│   │   ├── d13/03_catalog_version_distribution.json
│   │   └── d13/05_audit_stream.json
│   │
│   └── M5 extension (NEW, 4)
│       ├── d29_gpu_canary.json     (Sprint D29)
│       ├── d34_templates.json      (Sprint D34, NaSafety drill)
│       ├── d34_mode2.json          (Sprint D34)
│       └── d37_mode3.json          (Sprint D37)
│
├── Cardinality budget : 50 000 séries au max
│   Per-tenant overhead : 200 tenants × 5 instruments × 4 labels = 4 000 series
│   GPU overhead       : 8 GPUs × 6 instruments × 4 labels = 192 series
│   Templates overhead : 8 templates × 4 instruments × 200 tenants = 6 400 series
│   Mode-2 overhead    : 5 instruments × 3 routers × 200 tenants = 3 000 series
│   Mode-3 overhead    : 4 instruments × 3 aggregators × 200 tenants = 2 400 series
│   Total ≤ 16 000 nouvelles séries (32% of budget)
│
└── Pin tests Python (cible 30+)
```

---

## §1. Mission de l'extension M5 (Φ-D → Φ-F)

### §1.1 Énoncé opérationnel

L'extension M5 a **deux missions complémentaires** :

1. **Compléter le câblage instruments** pour que TOUS les SLOs
   définis par M1/M2/M4 soient **mesurables** :
   - SLO C-7 (canary overhead) — instruments GPU.
   - SLO C-3 / C-4 (Mode-2 latency / cache hit) — instruments Mode-2.
   - SLO C-3 (Mode-3 p99) — instruments Mode-3.
   - SLO produit "NaSafety triggered ≥ X%" — instruments per-template.

2. **Livrer les dashboards Grafana** pour que les opérateurs
   puissent triager les incidents en production sans logs brut :
   - Dashboard GPU canary (D29).
   - Dashboard templates drill-down (D34).
   - Dashboard Mode-2 cache + router (D34).
   - Dashboard Mode-3 fan-out + aggregation (D37).

### §1.2 Périmètre strict du document

Ce document **NE traite PAS** :

* L'OTel collector (déjà livré en D13).
* L'export Prometheus HTTP (déjà livré en D13).
* La rotation des audit signing keys (livré en D14-bis).
* Les dashboards opérationnels SRE-only (alerts manager, capacity
  planning) — couverts par le runbook ops séparé.

### §1.3 Mission complémentaire — anti-régression institutionnelle

L'audit pattern "wiring forgotten" se manifeste systématiquement
en télémétrie : un nouveau sprint **déclare** un instrument mais
oublie de **register** au boot, donc le panel Grafana correspondant
affiche "No data" jusqu'à la première utilisation à chaud.

L'extension M5 introduit le pattern canonique
`register_all_dXX()` pour chaque sprint nouveau (D29, D34, D37),
avec pin test institutionnel qui vérifie son appel dans
`runtime::Server::build`.

---

## §2. Position dans la topologie EFE post-M4

### §2.1 Graph de dépendances post-D37

```text
                 ┌─────────────────────────────────────────┐
                 │              Grafana                    │
                 │  ┌──────────┐  ┌──────────┐  ┌─────────┐│
                 │  │   GPU    │  │  Mode-2  │  │ Mode-3  ││
                 │  │ canary   │  │ caching  │  │ k-swarm ││
                 │  └────┬─────┘  └────┬─────┘  └────┬────┘│
                 └───────┼─────────────┼──────────────┼─────┘
                         │ scrapes /metrics every 15s│
                         ▼             ▼              ▼
                 ┌─────────────────────────────────────────┐
                 │      Prometheus (server-side store)      │
                 └────────────────────┬────────────────────┘
                                      │
                                      ▼
                 ┌─────────────────────────────────────────┐
                 │       efe-telemetry (M5)                │
                 │                                          │
                 │ /metrics endpoint :9090                  │
                 │ ─ gpu_instruments        (6, D29)       │
                 │ ─ nano_agent_instruments (4, D34)       │
                 │ ─ mode2_instruments      (5, D34)       │
                 │ ─ mode3_instruments      (4, D37)       │
                 │ ─ d13/d14/d15/d16/d17/d18 (31, existants)│
                 └────┬──────────────────────────────────────┘
                      │
        ┌─────────────┴───────────┬────────────────┐
        │                         │                │
        ▼                         ▼                ▼
  ┌──────────┐            ┌──────────────┐   ┌──────────────┐
  │efe-server│            │   efe-core   │   │  efe-kernels │
  │  (M4)    │            │     (M1)     │   │     (M2)     │
  │          │            │              │   │              │
  │ emits :  │            │   emits :    │   │  emits :     │
  │ Mode-2   │            │  per-template│   │  per-GPU op  │
  │ Mode-3   │            │  Mode-2      │   │  utilization │
  │ stream   │            │  Mode-3      │   │  HBM bytes   │
  │          │            │  HybridCanary│   │              │
  └──────────┘            └──────────────┘   └──────────────┘
```

### §2.2 Direction stricte des dépendances

* `efe-telemetry` dépend de `efe-core` (pour les types `Verdict`,
  `Request`, etc.) MAIS **pas** de `efe-server` ni `efe-ffi`.
* `efe-server` dépend de `efe-telemetry` (consomme les
  instruments).
* `efe-kernels` dépend de `efe-telemetry` via la feature `cuda` —
  les `KernelCallGuard` reportent vers `gpu_instruments`.

### §2.3 Position vs `efe-server` (M4)

| Surface | efe-telemetry (M5, post-D37) | efe-server (M4) |
|---------|:----------------------------:|:---------------:|
| `gpu_instruments` module | ✅ déclare + register | ❌ — consomme via `KernelCallGuard` |
| `mode2_instruments` module | ✅ déclare + register | ❌ — consomme via `Mode2ServiceImpl` |
| `mode3_instruments` module | ✅ déclare + register | ❌ — consomme via `Mode3ServiceImpl` |
| `nano_agent_instruments` module | ✅ déclare + register | ❌ — consomme via `Mode2Controller::execute_inner` |
| Dashboards JSON | ✅ versionné dans `deploy/observability/grafana/` | ❌ — provisioné via Helm |

---

## §3. État AS-IS au 2026-04-29 (post-D18 + post-M4 + post-M1)

### §3.1 Inventaire physique efe-telemetry

```
crates/efe-telemetry/src/
├── collector/                 # OTel collector (D13 phase 2)
│   ├── mod.rs   otel_collector.rs   otel_guard.rs
│   └── (~800 LOC)
├── events/                    # M11 RegistryEvent sinks
│   ├── d13_streams.rs   mod.rs   registry_sink.rs
│   └── (~700 LOC)
├── logs/                      # JSON layer + audit + field filter
│   ├── audit.rs   field_filter.rs   json_layer.rs   mod.rs
│   └── (~900 LOC)
├── metrics/                   # Prometheus instruments
│   ├── d13_instruments.rs        # 14 instruments (M11 framework)
│   ├── d13bis_instruments.rs     # 7 instruments (catalog admin)
│   ├── d14_instruments.rs        # 8 instruments (postgres catalog)
│   ├── d14bis_instruments.rs     # 11 instruments (audit HMAC)
│   ├── d15_instruments.rs        # 13 instruments (OIDC)
│   ├── d16_instruments.rs        # 16 instruments (embedder bundle)
│   ├── d17_instruments.rs        # 10 instruments (backup/DR)
│   ├── d18_instruments.rs        # 16 instruments (multi-replica)
│   ├── gpu_instruments.rs        # **STUB D28** : 2 instruments
│   ├── http_server.rs            # /metrics HTTP exporter
│   ├── instruments.rs            # legacy MVP instruments
│   ├── mod.rs                    # re-exports + register_all
│   ├── recorder.rs               # MetricsLayer (tower)
│   ├── registry.rs               # global Registry singleton
│   └── tower_layer.rs            # MetricsLayer impl
├── otel/                      # OTel-specific helpers
├── spans.rs                   # tracing span constructors
├── shutdown.rs                # graceful flush
├── observability_init_d13.rs  # D13 phase 2 init
├── init.rs                    # main entry point
├── lib.rs                     # public surface
└── config.rs                  # TelemetryConfig

Total : ~9 740 LOC
```

### §3.2 Surface dashboards AS-IS

```
deploy/observability/grafana/
├── efe-overview.json              # high-level health (D13 phase 1)
├── efe-fleet.json                 # multi-replica status (D13 phase 2)
├── efe-per-tenant.json            # per-tenant drill-down (D13)
├── per-tenant-dashboard.json      # legacy alias
└── dashboards/d13/
    ├── 02_tenant_drill_down.json
    ├── 03_catalog_version_distribution.json
    └── 05_audit_stream.json
```

### §3.3 Ce qui manque (gaps post-M4-audit)

L'extension M4 phase 1 (D34/D37) a référencé des instruments
Mode-2 / Mode-3 dans son architecture (`efe_mode2_cache_hit_rate`,
`efe_mode3_dispersion_p99`, etc.) mais **aucun code Rust** ne les
déclare. Le M4 audit pass 1 (M4-A7) a corrigé la NON-EMISSION de
`emit_request_completion(...)` côté handlers, mais les instruments
**eux-mêmes** (`mode2_instruments::cache_hit_rate`, etc.) n'existent
toujours pas.

L'extension M2 (CUDA D28) a livré le scaffolding `gpu_instruments.rs`
avec quelques helpers mais pas les 6 instruments fonctionnels du
sprint D29. Les `KernelCallGuard` events sont en place (côté
`efe-kernels`) mais ne reportent vers AUCUN instrument
Prometheus en production.

L'extension M1 (D33) a livré le trait `NanoAgentTemplate` avec
`consensus_weight`, `default_share` etc. mais aucune métrique
per-template n'est émise. Un opérateur ne peut PAS observer
"combien de fois le template NaSafety a tiré une refusal vote
cette semaine" — la donnée est invisible.

### §3.4 Synthèse AS-IS

| Sprint | État AS-IS | Gap | Effort |
|---|---|---|---|
| D29 GPU instruments | 2 stubs / 6 cible | 4 instruments + dashboard | 5 jours |
| D34 nano-agent | 0 | 4 instruments | 3 jours |
| D34 Mode-2 | 0 | 5 instruments + dashboard | 4 jours |
| D37 Mode-3 | 0 | 4 instruments + dashboard | 4 jours |
| Dashboards GA | 7 / 11 | 4 nouveaux JSON | 4 jours |

Total : ~20 jours-ingénieur. ~17 instruments + 4 dashboards à
livrer.

---

## §4. Définition opérationnelle de `efe-telemetry` v1.0 GA

### §4.1 Le contrat GA

À la fin de l'extension M5 (post-D37) :

* **50 instruments Prometheus** — chaque SLO produit défini par
  M1/M2/M4 a au moins un instrument qui le mesure.
* **11 dashboards Grafana** — provisionnés via Helm, accessibles
  à `https://grafana.efe.ai/d/<uid>` après login.
* **Cardinality budget** ≤ 50 000 séries — vérifié par pin test
  qui calcule le produit cartésien des labels.
* **Pattern `register_all_dXX`** — chaque sprint nouveau ship un
  helper qui force l'enregistrement eager au boot. Pin test
  institutionnel grep le call-site dans `runtime::Server::build`.

### §4.2 Versioning produit

| efe-telemetry version | Sprint | Instruments | Dashboards |
|-----------------------|---|--:|--:|
| 0.1.x (D1-D18 livré) | D18 | 31 | 7 |
| 0.2.x (D29 livré) | D29 | 37 | 8 |
| 0.3.x (D34 livré) | D34 | 46 | 10 |
| 0.4.x (D37 livré) | D37 | 50 | 11 |
| **1.0.0 GA** | post-D37 | 50 | 11 |

### §4.3 Gating entre versions

Chaque bump mineur exige :

* Tous les instruments du sprint déclarés + register_all_dXX
  appelé dans `runtime::Server::build`.
* Pin tests structurels Python (1 par sprint).
* Pin tests cardinality (1 par sprint).
* Dashboards JSON validés via `dashboard_smoke_test.py`.
* Prometheus rules associées (cf. `deploy/observability/prometheus/rules/`).

---

## §5. Principes directeurs

### §5.1 Principes opposables en revue PR

| ID | Principe | Application |
|----|---|---|
| **T-1** | **One sprint = one module** | Chaque sprint nouveau ship `metrics/dXX_instruments.rs` ; pas de pollution cross-sprint |
| **T-2** | **Eager registration** | Tout instrument est `lazy_static!` mais le `register_all_dXX()` force le first-touch au boot, sinon Grafana affiche "no data" |
| **T-3** | **Cardinality budget** | Chaque label ajouté est documenté avec l'estimation de cardinality. Refus PR si total > 50k séries |
| **T-4** | **PII-free labels** | Aucun label ne contient d'email, téléphone, IP. Tenant_id est OK (pseudonyme) |
| **T-5** | **Stable metric names** | `efe_<sprint_or_module>_<unit>_<status>` — renaming = breaking change pour tous les dashboards |
| **T-6** | **Histogram buckets fixes** | Buckets exposés via const ; tunable env vars pour les SLOs custom |
| **T-7** | **Dashboard versioning** | Chaque JSON dashboard a `"schemaVersion": <int>` et un `"uid"` stable |
| **T-8** | **Fail-closed on instrument error** | Si `register()` panique, le pod refuse de démarrer (un service sans télémétrie n'est pas un service) |

### §5.2 Ce que `efe-telemetry` refuse d'implémenter

* **Tracing distribué client-to-client** — délégué à OTel/Jaeger,
  pas la responsabilité de M5.
* **Storage** — Prometheus est le store ; M5 produit les données,
  pas le stockage.
* **Alerting rules** — délégué à Prometheus rule files dans
  `deploy/observability/prometheus/rules/`.
* **Multi-tenancy au niveau scrape** — un seul `/metrics` par pod ;
  Prometheus filtre par tenant via labels.

### §5.3 Naming convention

Tous les instruments suivent le pattern :

```
efe_<sprint_or_module>_<unit>_<status>
   │       │            │       │
   │       │            │       └─ optional (success / error / timeout / ...)
   │       │            └─ unit (seconds / bytes / total / rate)
   │       └─ scope (gpu / mode2 / mode3 / nano_agent / catalog / audit / ...)
   └─ universal prefix (always "efe_")
```

Exemples :
* `efe_gpu_kernel_seconds` — histogram en secondes (T-5).
* `efe_mode2_cache_hit_total` — counter monotone.
* `efe_mode3_dispersion` — gauge en valeur absolue.
* `efe_nano_agent_consensus_weight` — gauge per-template.

---

## §6. Invariants structurels

### §6.1 Invariants de structure de fichiers

```
crates/efe-telemetry/src/metrics/
├── d13_instruments.rs       (existant, intouché)
├── d13bis_instruments.rs    (existant, intouché)
├── d14_instruments.rs       (existant, intouché)
├── d14bis_instruments.rs    (existant, intouché)
├── d15_instruments.rs       (existant, intouché)
├── d16_instruments.rs       (existant, intouché)
├── d17_instruments.rs       (existant, intouché)
├── d18_instruments.rs       (existant, intouché)
├── gpu_instruments.rs       (**ÉTENDU D29**, 2 → 6 instruments)
├── nano_agent_instruments.rs (**NEW D34**)
├── mode2_instruments.rs     (**NEW D34**)
├── mode3_instruments.rs     (**NEW D37**)
├── http_server.rs           (existant, intouché)
├── instruments.rs           (existant, intouché)
├── mod.rs                   (extension : pub mod + re-exports)
├── recorder.rs              (existant, intouché)
├── registry.rs              (existant, intouché)
└── tower_layer.rs           (existant, intouché)

deploy/observability/grafana/dashboards/
├── d13/...                  (existant, intouché)
├── d29_gpu_canary.json      (**NEW D29**)
├── d34_templates.json       (**NEW D34**)
├── d34_mode2.json           (**NEW D34**)
└── d37_mode3.json           (**NEW D37**)
```

### §6.2 Invariants de runtime

* **register_all_dXX** est appelé dans `runtime::Server::build`
  AVANT le premier accept de connexion gRPC. Pin test institutionnel.
* **Cardinality cap** : chaque instrument vérifie son cardinality
  via `IntGaugeVec::with_label_values` (qui crash si > 50k séries
  par défaut Prometheus).
* **Dashboard provisioning** : Helm chart synchronise les JSON
  via ConfigMap `grafana-dashboards`. Pin test que tous les UIDs
  sont uniques.

### §6.3 Invariants de versioning

* Renommer un instrument = **breaking change** → tous les dashboards
  cassent → bump MAJOR (1.x → 2.0).
* Ajouter un label = **non-breaking** → bump MINOR.
* Supprimer un label = **breaking** → MAJOR.
* Ajouter un bucket à un histogram = **non-breaking**.
* Changer une bucket boundary = **breaking** (les anciens bucket
  comptes deviennent insignifiants).

---

## §7. Vue d'ensemble — 3 sprints, dependency graph

### §7.1 Tableau d'agrégation

| Sprint | Phase | Durée | Livrable principal | Instruments | Dashboards | Pin tests |
|--------|---|---|---|--:|--:|--:|
| **D29** | Φ-D | 5 j | 6 instruments GPU + canary dashboard | 6 | 1 | 8 |
| **D34** | Φ-E | 7 j | 4 nano-agent + 5 Mode-2 instruments + 2 dashboards | 9 | 2 | 12 |
| **D37** | Φ-F | 4 j | 4 instruments Mode-3 + dashboard | 4 | 1 | 6 |

Total : **19 instruments + 4 dashboards + 26 pin tests**.

### §7.2 Dépendances inter-sprints

```text
                D29 (GPU instruments)
                     │
                     │ (canary report consommé par
                     │  fusion::hybrid via M1 D32)
                     │
                D34 (templates + Mode-2)
                     │
                     │ (Mode-2 cache hit rate utilisé
                     │  par Mode-3 fan-out cost model)
                     │
                D37 (Mode-3)
                     │
                     ▼
                M5 GA v1.0
```

* **D29 préalable** à D34 (Mode-2 cache_hit dashboard panel
  référence le canary diff GPU-CPU).
* **D34 préalable** à D37 (Mode-3 fan-out cost = k × Mode-2 latency).

### §7.3 Volumétrie code attendue

```
D29 GPU instruments :
  gpu_instruments.rs extension  → +180 LOC (4 nouveaux)
  d29_gpu_canary.json dashboard → ~250 LOC JSON
  Pin tests + cardinality check → +120 LOC Python
  Total                         → ~550 LOC

D34 nano-agent + Mode-2 :
  nano_agent_instruments.rs (NEW) → ~200 LOC
  mode2_instruments.rs (NEW)      → ~250 LOC
  d34_templates.json dashboard    → ~300 LOC JSON
  d34_mode2.json dashboard        → ~280 LOC JSON
  Pin tests + cardinality         → +200 LOC Python
  Total                           → ~1 230 LOC

D37 Mode-3 :
  mode3_instruments.rs (NEW)    → ~220 LOC
  d37_mode3.json dashboard      → ~270 LOC JSON
  Pin tests + cardinality       → +110 LOC Python
  Total                         → ~600 LOC
```

Net : **~2 380 LOC** (Rust + JSON + Python).

---

## §8. Surface publique cible (50 instruments + 11 dashboards)

### §8.1 Inventaire post-D37 (GA)

```rust
// crates/efe-telemetry/src/metrics/mod.rs (post-D37)

pub mod d13_instruments;        // 14 instruments
pub mod d13bis_instruments;     // 7
pub mod d14_instruments;        // 8
pub mod d14bis_instruments;     // 11
pub mod d15_instruments;        // 13
pub mod d16_instruments;        // 16
pub mod d17_instruments;        // 10
pub mod d18_instruments;        // 16
pub mod gpu_instruments;        // 6   (D29)
pub mod nano_agent_instruments; // 4   (D34)
pub mod mode2_instruments;      // 5   (D34)
pub mod mode3_instruments;      // 4   (D37)

// Re-exports
pub use d13_instruments::register_all_d13;
pub use d13bis_instruments::register_all_d13bis;
pub use d14_instruments::register_all_d14;
pub use d14bis_instruments::register_all_d14bis;
pub use d15_instruments::register_all_d15;
pub use d16_instruments::register_all_d16;
pub use d17_instruments::register_all_d17;
pub use d18_instruments::register_all_d18;
pub use gpu_instruments::register_all_gpu;            // D29 (renommé)
pub use nano_agent_instruments::register_all_nano_agent; // D34 (NEW)
pub use mode2_instruments::register_all_mode2;        // D34 (NEW)
pub use mode3_instruments::register_all_mode3;        // D37 (NEW)
```

### §8.2 Mapping instrument → consommateur

| Instrument | Sprint | Émis par | Consommé par dashboard |
|------------|--------|----------|------------------------|
| `efe_gpu_utilization` | D29 | efe-kernels::CudaBackend | d29_gpu_canary |
| `efe_gpu_memory_used_bytes` | D29 | efe-kernels::DeviceAllocator | d29_gpu_canary |
| `efe_gpu_kernel_seconds` | D29 | efe-kernels::KernelCallGuard | d29_gpu_canary |
| `efe_gpu_kernel_calls_total` | D29 | efe-kernels::KernelCallGuard | d29_gpu_canary |
| `efe_gpu_canary_divergence_total` | D29 | efe-core::HybridFusion | d29_gpu_canary |
| `efe_gpu_canary_divergence_l2` | D29 | efe-core::HybridFusion | d29_gpu_canary |
| `efe_nano_agent_population_total` | D34 | efe-core::genesis::spawner | d34_templates |
| `efe_nano_agent_consensus_weight` | D34 | efe-core::consensus::detector | d34_templates |
| `efe_nano_agent_safety_triggered_total` | D34 | efe-core::agents::NaSafety | d34_templates |
| `efe_nano_agent_kernel_routed_total` | D34 | efe-core::fusion::MvpCpuFusion | d34_templates |
| `efe_mode2_cache_hit_total` | D34 | efe-core::Mode2Controller::cache | d34_mode2 |
| `efe_mode2_cache_miss_total` | D34 | efe-core::Mode2Controller::cache | d34_mode2 |
| `efe_mode2_router_latency_seconds` | D34 | efe-core::LlmRouter | d34_mode2 |
| `efe_mode2_anchors_generated` | D34 | efe-core::Mode2Controller | d34_mode2 |
| `efe_mode2_seed_mix_collisions_total` | D34 | efe-core::Mode2Controller | d34_mode2 |
| `efe_mode3_subswarms_spawned_total` | D37 | efe-core::Mode3Controller | d37_mode3 |
| `efe_mode3_dispersion` | D37 | efe-core::Mode3Aggregator | d37_mode3 |
| `efe_mode3_k_clamped_total` | D37 | efe-core::Mode3Controller | d37_mode3 |
| `efe_mode3_aggregation_seconds` | D37 | efe-core::Mode3Aggregator | d37_mode3 |

---

## §9. Sprint **D29** — 6 instruments GPU + dashboard canary

### §9.1 Objectif

Compléter `gpu_instruments.rs` avec les 6 instruments GPU
production-ready et livrer le dashboard `d29_gpu_canary.json` qui
permet aux opérateurs de monitorer le rollout GPU progressif :

* Utilisation par GPU (graphique 0..100 %).
* Memory used / total (graphique stacked bytes).
* Latency p50/p95/p99 par kernel.
* Canary divergence rate (CPU vs GPU).

### §9.2 Architecture du module

```text
crates/efe-telemetry/src/metrics/gpu_instruments.rs
├── lazy_static! { ... }
│   ├── GPU_UTILIZATION : IntGaugeVec  ["gpu_id"]
│   ├── GPU_MEMORY_USED_BYTES : IntGaugeVec  ["gpu_id"]
│   ├── GPU_KERNEL_SECONDS : HistogramVec  ["gpu_id", "kernel"]
│   ├── GPU_KERNEL_CALLS_TOTAL : IntCounterVec  ["gpu_id", "kernel", "result"]
│   ├── GPU_CANARY_DIVERGENCE_TOTAL : IntCounter  []
│   └── GPU_CANARY_DIVERGENCE_L2 : Gauge  []
│
├── pub fn register_all_gpu() { /* eager-register the 6 above */ }
└── pub mod accessors {
       pub fn utilization() -> &'static IntGaugeVec { &GPU_UTILIZATION }
       pub fn memory_used_bytes() -> &'static IntGaugeVec { ... }
       pub fn kernel_seconds() -> &'static HistogramVec { ... }
       pub fn kernel_calls_total() -> &'static IntCounterVec { ... }
       pub fn canary_divergence_total() -> &'static IntCounter { ... }
       pub fn canary_divergence_l2() -> &'static Gauge { ... }
}
```

### §9.3 Détail des 6 instruments

#### 1. `efe_gpu_utilization`

* **Type** : `IntGauge`
* **Labels** : `gpu_id` (string, valeurs 0..7 pour 8 GPUs max).
* **Unit** : pourcentage 0..100.
* **Source** : `efe-kernels::DeviceAllocator::utilization_percent(gpu_id)`.
* **Fréquence d'update** : par appel `KernelCallGuard::finalize`.
* **Cardinality** : 1 instrument × 8 GPUs = 8 séries.

#### 2. `efe_gpu_memory_used_bytes`

* **Type** : `IntGauge`
* **Labels** : `gpu_id`.
* **Unit** : bytes.
* **Source** : `efe-kernels::DeviceAllocator::memory_used_bytes(gpu_id)`.
* **Fréquence** : sampled 1 Hz par task séparée.
* **Cardinality** : 8 séries.

#### 3. `efe_gpu_kernel_seconds`

* **Type** : `Histogram` avec buckets `[0.0001, 0.0005, 0.001,
  0.005, 0.01, 0.05, 0.1, 0.5, 1.0]` (range 0.1 ms à 1 s).
* **Labels** : `gpu_id`, `kernel` (e.g. `"cone_project"`,
  `"swarm_attention"`, `"softmax"`).
* **Source** : `efe-kernels::KernelCallGuard::record_finish()`.
* **Cardinality** : 8 GPUs × 23 kernels = **184 séries**.

#### 4. `efe_gpu_kernel_calls_total`

* **Type** : `IntCounter` cumulatif.
* **Labels** : `gpu_id`, `kernel`, `result` (`"success"` / `"error"` /
  `"timeout"`).
* **Source** : `KernelCallGuard::finalize()`.
* **Cardinality** : 8 × 23 × 3 = **552 séries**.

#### 5. `efe_gpu_canary_divergence_total`

* **Type** : `IntCounter` global (no labels).
* **Source** : `efe-core::HybridFusion::CanaryHybridFusion::run_round`
  (incremented quand `divergence_detected() == true`).
* **Cardinality** : 1 série.

#### 6. `efe_gpu_canary_divergence_l2`

* **Type** : `Gauge` (last observed value).
* **Source** : `HybridFusionReport::centroid_l2_diff` (sauvé en
  gauge à chaque report).
* **Cardinality** : 1 série.

**Total D29 : 6 instruments / 754 séries** (96 % du budget cardinality
restent dispo après D29).

### §9.4 Code skeleton (extension de `gpu_instruments.rs`)

```rust
// crates/efe-telemetry/src/metrics/gpu_instruments.rs (D29 extension)

use lazy_static::lazy_static;
use prometheus::{
    register_gauge, register_histogram_vec, register_int_counter,
    register_int_counter_vec, register_int_gauge_vec, Gauge, HistogramVec,
    IntCounter, IntCounterVec, IntGaugeVec,
};

const KERNEL_BUCKETS: &[f64] = &[
    0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0,
];

lazy_static! {
    static ref GPU_UTILIZATION: IntGaugeVec = register_int_gauge_vec!(
        "efe_gpu_utilization",
        "GPU utilization percentage [0..100] per GPU.",
        &["gpu_id"]
    ).expect("register efe_gpu_utilization");

    static ref GPU_MEMORY_USED_BYTES: IntGaugeVec = register_int_gauge_vec!(
        "efe_gpu_memory_used_bytes",
        "GPU memory currently used in bytes per GPU.",
        &["gpu_id"]
    ).expect("register efe_gpu_memory_used_bytes");

    static ref GPU_KERNEL_SECONDS: HistogramVec = register_histogram_vec!(
        "efe_gpu_kernel_seconds",
        "GPU kernel execution time in seconds.",
        &["gpu_id", "kernel"],
        KERNEL_BUCKETS.to_vec()
    ).expect("register efe_gpu_kernel_seconds");

    static ref GPU_KERNEL_CALLS_TOTAL: IntCounterVec = register_int_counter_vec!(
        "efe_gpu_kernel_calls_total",
        "Total GPU kernel invocations grouped by kernel name and result.",
        &["gpu_id", "kernel", "result"]
    ).expect("register efe_gpu_kernel_calls_total");

    static ref GPU_CANARY_DIVERGENCE_TOTAL: IntCounter = register_int_counter!(
        "efe_gpu_canary_divergence_total",
        "Total HybridFusion shadow vs primary divergence events."
    ).expect("register efe_gpu_canary_divergence_total");

    static ref GPU_CANARY_DIVERGENCE_L2: Gauge = register_gauge!(
        "efe_gpu_canary_divergence_l2",
        "Last observed L2 distance between primary and shadow centroids."
    ).expect("register efe_gpu_canary_divergence_l2");
}

pub mod accessors {
    use super::*;
    pub fn utilization() -> &'static IntGaugeVec { &GPU_UTILIZATION }
    pub fn memory_used_bytes() -> &'static IntGaugeVec { &GPU_MEMORY_USED_BYTES }
    pub fn kernel_seconds() -> &'static HistogramVec { &GPU_KERNEL_SECONDS }
    pub fn kernel_calls_total() -> &'static IntCounterVec { &GPU_KERNEL_CALLS_TOTAL }
    pub fn canary_divergence_total() -> &'static IntCounter { &GPU_CANARY_DIVERGENCE_TOTAL }
    pub fn canary_divergence_l2() -> &'static Gauge { &GPU_CANARY_DIVERGENCE_L2 }
}

pub fn register_all_gpu() {
    // Eager-register : forces lazy_static! to execute even when no
    // GPU is active, so dashboards see zero-valued series at boot.
    let _ = &*GPU_UTILIZATION;
    let _ = &*GPU_MEMORY_USED_BYTES;
    let _ = &*GPU_KERNEL_SECONDS;
    let _ = &*GPU_KERNEL_CALLS_TOTAL;
    let _ = &*GPU_CANARY_DIVERGENCE_TOTAL;
    let _ = &*GPU_CANARY_DIVERGENCE_L2;
}
```

### §9.5 Dashboard `d29_gpu_canary.json`

Structure cible (cf. §22 pour le JSON skeleton complet) :

* **Row 1** : "GPU Utilization & Memory" (2 panels — utilization
  per GPU + stacked memory).
* **Row 2** : "Kernel Latency" (2 panels — heatmap + p99 line).
* **Row 3** : "Canary Divergence" (2 panels — divergence rate +
  L2 distance over time).
* **Row 4** : "Kernel Errors & Timeouts" (1 panel — kernel_calls
  filtered by `result != "success"`).

Variables Grafana : `$gpu_id` (multi-select), `$tenant` (legacy
filter, unused for D29).

### §9.6 Tests + pin tests D29

* **Unit tests Rust** (4) :
  * `gpu_utilization_registered_after_call`.
  * `kernel_seconds_buckets_match_const`.
  * `canary_divergence_increments`.
  * `canary_l2_records_last_value`.
* **Pin tests Python** (`scripts/tests/test_d29_gpu_metrics.py`,
  8 tests) :
  * `register_all_gpu` exposed.
  * 6 instruments names match.
  * `d29_gpu_canary.json` exists and valid JSON.
  * Dashboard UID stable.
  * Cardinality budget ≤ 1 000 séries.
  * Bucket array stable (anti-régression sur les SLO histograms).
* **Bench** : `gpu_metrics_overhead` — overhead par appel
  `KernelCallGuard::finalize` ≤ 50 µs.

### §9.7 Critères Go/No-Go D29

* GG-D29.1 : 6 instruments GREEN sur unit tests.
* GG-D29.2 : `register_all_gpu()` appelé dans
  `runtime::Server::build` quand `EFE_RUNTIME_GPU_ENABLED=true`.
* GG-D29.3 : Pin tests Python `test_d29_gpu_metrics.py` GREEN.
* GG-D29.4 : Dashboard `d29_gpu_canary.json` provisioné via Helm.
* GG-D29.5 : v0.2.x tag.

---

## §10. Sprint **D34** — 4 nano-agent + 5 Mode-2 instruments + 2 dashboards

### §10.1 Objectif

Livrer **9 nouveaux instruments** + **2 nouveaux dashboards** :

* `nano_agent_instruments.rs` (NEW) — 4 instruments per-template
  pour observer la diversité de la population D33.
* `mode2_instruments.rs` (NEW) — 5 instruments pour observer le
  cache hit rate, la latence routeur, et les anchors générées.
* `d34_templates.json` — drill-down par template (NaSafety,
  NaCausal, etc.).
* `d34_mode2.json` — Mode-2 cache + router + swarm latency.

### §10.2 Architecture des modules

```text
crates/efe-telemetry/src/metrics/
├── nano_agent_instruments.rs (NEW D34)
│   ├── NANO_AGENT_POPULATION_TOTAL  IntCounterVec  ["template_code", "tenant"]
│   ├── NANO_AGENT_CONSENSUS_WEIGHT  GaugeVec       ["template_code"]
│   ├── NANO_AGENT_SAFETY_TRIGGERED_TOTAL  IntCounterVec ["template_code", "tenant"]
│   ├── NANO_AGENT_KERNEL_ROUTED_TOTAL  IntCounterVec ["kernel_id", "template_code"]
│   ├── pub fn register_all_nano_agent()
│   └── pub mod accessors { ... }
│
└── mode2_instruments.rs (NEW D34)
    ├── MODE2_CACHE_HIT_TOTAL  IntCounterVec  ["router_id", "tenant"]
    ├── MODE2_CACHE_MISS_TOTAL IntCounterVec  ["router_id", "tenant"]
    ├── MODE2_ROUTER_LATENCY_SECONDS  HistogramVec  ["router_id"]
    ├── MODE2_ANCHORS_GENERATED  HistogramVec  ["router_id"]
    ├── MODE2_SEED_MIX_COLLISIONS_TOTAL  IntCounter
    ├── pub fn register_all_mode2()
    └── pub mod accessors { ... }
```

### §10.3 Détail des 4 instruments nano-agent

#### 1. `efe_nano_agent_population_total`

* **Type** : `IntCounter`
* **Labels** : `template_code` (8 values D33), `tenant` (200 max).
* **Source** : `efe-core::genesis::spawner::on_spawn(template, tenant)`.
* **Cardinality** : 8 × 200 = **1 600 séries**.

#### 2. `efe_nano_agent_consensus_weight`

* **Type** : `Gauge`
* **Labels** : `template_code`.
* **Unit** : factor (default 1.0, NaSafety = 2.0).
* **Source** : `efe-core::agents::templates::NanoAgentTemplate::consensus_weight()`
  echoed at boot via `register_all_nano_agent()` initial set.
* **Cardinality** : 8 séries.

#### 3. `efe_nano_agent_safety_triggered_total`

* **Type** : `IntCounter`
* **Labels** : `template_code` (toujours `"na-safety"`), `tenant`.
* **Source** : émis par `efe-core::agents::templates::NaSafety`
  (futur — DT-T-D34-SAFETY-WIRE-1) ou via `efe-server` côté
  Mode-2 dispatch (court terme).
* **Cardinality** : 1 × 200 = **200 séries**.

#### 4. `efe_nano_agent_kernel_routed_total`

* **Type** : `IntCounter`
* **Labels** : `kernel_id` (8 valeurs D33), `template_code`.
* **Source** : `efe-core::fusion::MvpCpuFusion::dispatch_kernel`
  (futur — DT-C-D33-FUSION-KERNEL-1).
* **Cardinality** : 8 × 8 = **64 séries**.

### §10.4 Détail des 5 instruments Mode-2

#### 1. `efe_mode2_cache_hit_total`

* **Type** : `IntCounter`
* **Labels** : `router_id` (3 valeurs : mock/openai/anthropic),
  `tenant`.
* **Source** : `efe-core::Mode2Controller::cache.get` quand `Some`.
* **Cardinality** : 3 × 200 = **600 séries**.

#### 2. `efe_mode2_cache_miss_total`

* Identique mais sur `cache.get` quand `None`.
* **Cardinality** : 600 séries.

**SLO C-4 dérivé** : ratio `hit_total / (hit_total + miss_total)` ≥ 0.6
(60 %).

#### 3. `efe_mode2_router_latency_seconds`

* **Type** : `Histogram` avec buckets `[0.05, 0.1, 0.25, 0.5, 1.0,
  2.0, 5.0]`.
* **Labels** : `router_id`.
* **Source** : `efe-core::LlmRouter::generate_anchors` wrapped in
  Instant timer.
* **Cardinality** : 3 × ~10 buckets = 30 séries observables.

#### 4. `efe_mode2_anchors_generated`

* **Type** : `Histogram` (`[1, 3, 5, 10, 20]`).
* **Labels** : `router_id`.
* **Source** : `len(router_output.anchors)` at end of generate_anchors.
* **Cardinality** : 3 séries.

#### 5. `efe_mode2_seed_mix_collisions_total`

* **Type** : `IntCounter` global.
* **Source** : `efe-core::Mode2Controller::execute` quand le
  `mixed_seed` se collisionne avec une seed déjà observée (rare ;
  utile pour détecter une faille dans le seed-mix algorithm).
* **Cardinality** : 1 série.

### §10.5 Code skeleton (`nano_agent_instruments.rs`)

```rust
// crates/efe-telemetry/src/metrics/nano_agent_instruments.rs (NEW D34)

use lazy_static::lazy_static;
use prometheus::{
    register_gauge_vec, register_int_counter_vec,
    GaugeVec, IntCounterVec,
};

lazy_static! {
    static ref NANO_AGENT_POPULATION_TOTAL: IntCounterVec = register_int_counter_vec!(
        "efe_nano_agent_population_total",
        "Total nano-agents spawned per template per tenant.",
        &["template_code", "tenant"]
    ).expect("register efe_nano_agent_population_total");

    static ref NANO_AGENT_CONSENSUS_WEIGHT: GaugeVec = register_gauge_vec!(
        "efe_nano_agent_consensus_weight",
        "Per-template consensus weight applied by the multi-criteria detector.",
        &["template_code"]
    ).expect("register efe_nano_agent_consensus_weight");

    static ref NANO_AGENT_SAFETY_TRIGGERED_TOTAL: IntCounterVec = register_int_counter_vec!(
        "efe_nano_agent_safety_triggered_total",
        "Total NaSafety refusal triggers per tenant.",
        &["template_code", "tenant"]
    ).expect("register efe_nano_agent_safety_triggered_total");

    static ref NANO_AGENT_KERNEL_ROUTED_TOTAL: IntCounterVec = register_int_counter_vec!(
        "efe_nano_agent_kernel_routed_total",
        "Total fusion-kernel routing decisions per template.",
        &["kernel_id", "template_code"]
    ).expect("register efe_nano_agent_kernel_routed_total");
}

pub mod accessors {
    use super::*;
    pub fn population_total() -> &'static IntCounterVec { &NANO_AGENT_POPULATION_TOTAL }
    pub fn consensus_weight() -> &'static GaugeVec { &NANO_AGENT_CONSENSUS_WEIGHT }
    pub fn safety_triggered_total() -> &'static IntCounterVec { &NANO_AGENT_SAFETY_TRIGGERED_TOTAL }
    pub fn kernel_routed_total() -> &'static IntCounterVec { &NANO_AGENT_KERNEL_ROUTED_TOTAL }
}

pub fn register_all_nano_agent() {
    let _ = &*NANO_AGENT_POPULATION_TOTAL;
    let _ = &*NANO_AGENT_CONSENSUS_WEIGHT;
    let _ = &*NANO_AGENT_SAFETY_TRIGGERED_TOTAL;
    let _ = &*NANO_AGENT_KERNEL_ROUTED_TOTAL;

    // **Sprint D34 init step** — pre-populate consensus_weight
    // gauge with the values declared by the trait. Without this,
    // the gauge appears as "no data" until the first invocation
    // of consensus.
    use efe_core::agents::templates::{ga_templates, NanoAgentTemplate};
    for t in ga_templates() {
        NANO_AGENT_CONSENSUS_WEIGHT
            .with_label_values(&[t.code().as_str()])
            .set(t.consensus_weight().into());
    }
}
```

### §10.6 Dashboards D34

#### `d34_templates.json` — Drill-down par template

* **Row 1** : "Population by Template" (pie chart on
  `efe_nano_agent_population_total` summed over tenant).
* **Row 2** : "NaSafety Triggers" (line chart per tenant).
* **Row 3** : "Consensus Weights" (table — must match the trait
  declarations).
* **Row 4** : "Kernel Routing Distribution" (heatmap kernel_id ×
  template_code).

#### `d34_mode2.json` — Mode-2 health

* **Row 1** : "Cache Hit Rate" (single stat with SLO threshold
  60 %).
* **Row 2** : "Router Latency" (heatmap).
* **Row 3** : "Anchors Generated Distribution" (histogram).
* **Row 4** : "Cache Hit/Miss per Router" (stacked area).

### §10.7 Tests + pin tests D34

* **Unit tests Rust** (8) : 4 tests par module.
* **Pin tests Python** (`test_d34_telemetry.py`, 12 tests) :
  * 4 instruments nano-agent + 5 Mode-2 = 9 instruments expected.
  * 2 dashboards JSON valides + UIDs uniques.
  * Cardinality budget (`test_d34_cardinality_budget` : ≤ 3 000
    séries pour les 9 instruments).
  * `register_all_nano_agent` + `register_all_mode2` appelés
    dans `runtime::Server::build`.

### §10.8 Critères Go/No-Go D34

* GG-D34.1 : 9 instruments GREEN.
* GG-D34.2 : 2 dashboards JSON valides (schemaVersion ≥ 30).
* GG-D34.3 : Pin tests `test_d34_telemetry.py` GREEN.
* GG-D34.4 : SLO panel "Cache Hit Rate ≥ 60 %" affiche un
  threshold rouge/jaune/vert.
* GG-D34.5 : v0.3.x tag.

---

## §11. Sprint **D37** — 4 instruments Mode-3 + 1 dashboard

### §11.1 Objectif

Livrer 4 instruments Mode-3 + 1 dashboard pour observer le
fan-out k-swarm, la dispersion entre sub-verdicts, et le
clamping k_max.

### §11.2 Détail des 4 instruments

#### 1. `efe_mode3_subswarms_spawned_total`

* **Type** : `IntCounter`
* **Labels** : `mode_per_call` (`"mode1"` / `"mode2"`), `tenant`.
* **Source** : `efe-core::Mode3Controller::execute_parallel`
  incremente par `k_effective` à chaque call.
* **Cardinality** : 2 × 200 = **400 séries**.

#### 2. `efe_mode3_dispersion`

* **Type** : `Gauge` (last observed value).
* **Labels** : `aggregator` (`"mean"` / `"weighted"` / `"median"`),
  `tenant`.
* **Source** : `efe-core::Mode3Aggregator::aggregate.dispersion`
  (std deviation of confidences across sub-verdicts).
* **Cardinality** : 3 × 200 = **600 séries**.
* **SLO produit** : dispersion > 0.15 → flag for human review.

#### 3. `efe_mode3_k_clamped_total`

* **Type** : `IntCounter`
* **Labels** : `tenant`.
* **Source** : `efe-core::Mode3Controller::clamp_k` quand
  `k > K_MAX`.
* **Cardinality** : 200 séries.
* **Use case** : détecter les clients qui demandent k > 5 (mauvaise
  utilisation API).

#### 4. `efe_mode3_aggregation_seconds`

* **Type** : `Histogram`
* **Buckets** : `[0.0001, 0.001, 0.01, 0.1, 1.0]`.
* **Labels** : `aggregator`.
* **Source** : timer wrap autour de `Mode3Aggregator::aggregate`.
* **Cardinality** : 3 × ~5 buckets = 15 séries.

### §11.3 Code skeleton

```rust
// crates/efe-telemetry/src/metrics/mode3_instruments.rs (NEW D37)

use lazy_static::lazy_static;
use prometheus::{
    register_gauge_vec, register_histogram_vec, register_int_counter_vec,
    GaugeVec, HistogramVec, IntCounterVec,
};

const AGGREGATION_BUCKETS: &[f64] = &[0.0001, 0.001, 0.01, 0.1, 1.0];

lazy_static! {
    static ref MODE3_SUBSWARMS_SPAWNED_TOTAL: IntCounterVec = register_int_counter_vec!(
        "efe_mode3_subswarms_spawned_total",
        "Total Mode-3 sub-swarms spawned per mode_per_call per tenant.",
        &["mode_per_call", "tenant"]
    ).expect("register efe_mode3_subswarms_spawned_total");

    static ref MODE3_DISPERSION: GaugeVec = register_gauge_vec!(
        "efe_mode3_dispersion",
        "Std deviation of sub-verdict confidences (per aggregator per tenant).",
        &["aggregator", "tenant"]
    ).expect("register efe_mode3_dispersion");

    static ref MODE3_K_CLAMPED_TOTAL: IntCounterVec = register_int_counter_vec!(
        "efe_mode3_k_clamped_total",
        "Total requests where k > K_MAX was clamped server-side.",
        &["tenant"]
    ).expect("register efe_mode3_k_clamped_total");

    static ref MODE3_AGGREGATION_SECONDS: HistogramVec = register_histogram_vec!(
        "efe_mode3_aggregation_seconds",
        "Mode-3 aggregator wall time in seconds.",
        &["aggregator"],
        AGGREGATION_BUCKETS.to_vec()
    ).expect("register efe_mode3_aggregation_seconds");
}

pub mod accessors {
    use super::*;
    pub fn subswarms_spawned_total() -> &'static IntCounterVec { &MODE3_SUBSWARMS_SPAWNED_TOTAL }
    pub fn dispersion() -> &'static GaugeVec { &MODE3_DISPERSION }
    pub fn k_clamped_total() -> &'static IntCounterVec { &MODE3_K_CLAMPED_TOTAL }
    pub fn aggregation_seconds() -> &'static HistogramVec { &MODE3_AGGREGATION_SECONDS }
}

pub fn register_all_mode3() {
    let _ = &*MODE3_SUBSWARMS_SPAWNED_TOTAL;
    let _ = &*MODE3_DISPERSION;
    let _ = &*MODE3_K_CLAMPED_TOTAL;
    let _ = &*MODE3_AGGREGATION_SECONDS;
}
```

### §11.4 Dashboard `d37_mode3.json`

* **Row 1** : "k-swarm Throughput" (subswarms_spawned per minute).
* **Row 2** : "Dispersion Distribution" (heatmap dispersion vs
  time, color-coded with SLO threshold 0.15).
* **Row 3** : "k Clamped Rate" (sparkline per tenant — high rate =
  client mis-using API).
* **Row 4** : "Aggregator Latency" (histogram per aggregator).

### §11.5 Tests + pin tests D37

* **Unit tests Rust** (4).
* **Pin tests Python** (`test_d37_telemetry.py`, 6 tests).
* **Bench** : `mode3_telemetry_overhead` ≤ 30 µs par call.

### §11.6 Critères Go/No-Go D37

* GG-D37.1 : 4 instruments GREEN.
* GG-D37.2 : Dashboard `d37_mode3.json` provisionné.
* GG-D37.3 : Pin tests GREEN.
* GG-D37.4 : SLO panel "Mode-3 p99 ≤ 3 × Mode-1 p99" en place.
* GG-D37.5 : v0.4.0 GA tag.

---

## §12. Dashboards Grafana — JSON skeleton

### §12.1 Convention

Tous les dashboards M5 suivent le pattern :

* **schemaVersion** : 38 (Grafana 10.x).
* **uid** : `efe-d{XX}-{slug}` (e.g. `efe-d29-gpu-canary`).
* **time** : `now-1h` to `now`.
* **refresh** : `30s`.
* **panels** : 4 to 8 par dashboard (max 12).
* **annotations** : ajout des deploy events depuis Loki/Prometheus.

### §12.2 Annexe `d29_gpu_canary.json` (skeleton)

Cf. § 22.1.

### §12.3 Annexe `d34_templates.json` (skeleton)

Cf. § 22.2.

### §12.4 Annexe `d34_mode2.json` (skeleton)

Cf. § 22.3.

### §12.5 Annexe `d37_mode3.json` (skeleton)

Cf. § 22.4.

---

## §13. SLO produit final (mapping instruments → SLOs)

### §13.1 Inventaire post-D37

| SLO | Instrument(s) | PromQL |
|-----|--------------|--------|
| **C-1** Mode-1 p99 ≤ 200 ms | `efe_grpc_request_duration_seconds{method="Mode1Service/Execute"}` | `histogram_quantile(0.99, rate(...[5m]))` |
| **C-2** Mode-2 p99 ≤ 1500 ms | `efe_grpc_request_duration_seconds{method="Mode2Service/Execute"}` | idem |
| **C-3** Mode-3 p99 ≤ 3 × Mode-1 p99 | combo Mode-3 / Mode-1 | ratio des p99 |
| **C-4** Mode-2 cache hit ≥ 60 % | `efe_mode2_cache_hit_total` / sum | `cache_hit_total / (cache_hit_total + cache_miss_total)` |
| **C-5** k_max enforced | `efe_mode3_k_clamped_total` | bounded counter |
| **C-6** Determinism | (no metric ; pin test) | — |
| **C-7** HybridFusion overhead ≤ 5 % | `efe_gpu_kernel_seconds` (CPU shadow vs primary) | ratio histograms |
| **C-8** Divergence detection | `efe_gpu_canary_divergence_total` | counter |
| **C-9** Parallel speedup ≥ 2.5× | combo `efe_grpc_request_duration_seconds` Mode-3 sequential vs parallel | ratio |
| **C-10** GA accuracy | (golden bench, hors télémétrie) | — |
| **C-11** NaSafety triggers ≥ 0.5 % traffic | `efe_nano_agent_safety_triggered_total` | rate |
| **C-12** Mode-3 dispersion ≤ 0.15 | `efe_mode3_dispersion` | quantile |

---

## §14. Cardinality budget (anti-explosion labels)

### §14.1 Tableau d'impact

| Module | Instruments | Label combinations | Series | % budget |
|--------|--:|---|--:|--:|
| d13_instruments | 14 | … | 5 200 | 10.4 % |
| d18_instruments | 16 | replica × tenant | 8 000 | 16.0 % |
| **gpu_instruments (D29)** | 6 | gpu_id × kernel × result | **754** | **1.5 %** |
| **nano_agent_instruments (D34)** | 4 | template × tenant × kernel | **1 872** | **3.7 %** |
| **mode2_instruments (D34)** | 5 | router × tenant | **1 230** | **2.5 %** |
| **mode3_instruments (D37)** | 4 | aggregator × tenant | **1 215** | **2.4 %** |
| Pre-existing modules | 15 (sum) | various | 18 729 | 37.5 % |
| **Total post-D37** | **50** | — | **37 000** | **74 %** |

Reste 26 % pour les futurs sprints D38+.

### §14.2 Pin test `test_telemetry_cardinality_budget`

```python
def test_total_cardinality_within_50k_budget():
    """Sum the cardinality of every M5 instrument's worst-case
    label combinatorics. Must stay below 50k series (Prometheus
    per-instance soft limit before performance degradation)."""
    total = sum_instrument_cardinalities()
    assert total <= 50_000, f"cardinality budget exceeded: {total}"
```

---

## §15. Dépendances inter-sprints

```text
                D29 (GPU instruments)
                     │
                     │ requires : efe-kernels CudaBackend D28
                     │ consumes : KernelCallGuard events
                     │ requires : efe-core HybridFusion D32
                     │ consumes : HybridFusionReport
                     │
                D34 (templates + Mode-2)
                     │
                     │ requires : efe-core D33 templates
                     │ requires : efe-core Mode2Controller D34
                     │ consumes : cache hit/miss events
                     │
                D37 (Mode-3)
                     │
                     │ requires : efe-core Mode3Controller D37
                     │ consumes : aggregator dispersion events
                     │
                     ▼
                M5 GA v1.0
```

---

## §16. Tests + pin tests structurels

### §16.1 Inventaire post-D37

| Sprint | Unit tests Rust | Pin tests Python | Bench |
|---|--:|--:|---|
| D29 | 4 | 8 | gpu_metrics_overhead |
| D34 | 8 | 12 | (none) |
| D37 | 4 | 6 | mode3_telemetry_overhead |
| **Total** | **16** | **26** | 2 |

### §16.2 Pin tests institutionnels

Anti-régression sur `register_all_*` :

```python
def test_register_all_called_in_runtime():
    """Every sprint's register_all_dXX MUST be called inside
    runtime::Server::build. Defect class : 'wiring forgotten' —
    instrument declared but never first-touched, dashboard shows
    'no data' until first hot path."""
    rs = read("crates/efe-server/src/runtime.rs")
    for fn in [
        "register_all_d13", "register_all_d13bis",
        "register_all_d14", "register_all_d14bis",
        "register_all_d15", "register_all_d16",
        "register_all_d17", "register_all_d18",
        "register_all_gpu",          # D29
        "register_all_nano_agent",   # D34
        "register_all_mode2",        # D34
        "register_all_mode3",        # D37
    ]:
        assert fn in rs, f"register_all_dXX missing : {fn}"
```

---

## §17. Critères Go/No-Go par sprint

| Sprint | Critère principal | Métrique | Cible |
|---|---|---|---|
| D29 | 6 GPU instruments GREEN + dashboard | unit tests + UID stable | 100 % |
| D29 | Overhead par KernelCallGuard | bench | ≤ 50 µs |
| D29 | Cardinality | sum series | ≤ 1 000 |
| D34 | 9 instruments GREEN + 2 dashboards | unit tests + JSON valides | 100 % |
| D34 | Cache hit rate measurable | C-4 PromQL | exposé |
| D34 | Cardinality | sum series | ≤ 3 500 |
| D37 | 4 instruments GREEN + dashboard | unit tests | 100 % |
| D37 | Mode-3 p99 measurable | C-3 PromQL | exposé |
| D37 | Cardinality | sum series | ≤ 1 500 |

---

## §18. Risques transverses + mitigations

| ID | Risque | Probabilité | Impact | Mitigation |
|----|---|:--:|:--:|---|
| **T-R-1** | Cardinality explosion (label values dégénérés) | Moyenne | Élevé | Pin test cardinality budget ≤ 50k |
| **T-R-2** | Renommage instrument breaking dashboards | Basse | Élevé | Pin test instrument names + bump major si renommage |
| **T-R-3** | `register_all_*` oublié dans runtime | Haute | Moyen | Pin test institutionnel `test_register_all_called_in_runtime` |
| **T-R-4** | Dashboard UID collision | Basse | Faible | Pin test `test_d34_dashboard_uids_unique` |
| **T-R-5** | Histogram bucket trop fin (cardinality blow-up) | Moyenne | Élevé | Buckets fixés en const + revue PR |
| **T-R-6** | Per-tenant labels (200+) explose la cardinality | Haute | Élevé | Cap tenant labels via PromRule re-labeling (top-50 only) |
| **T-R-7** | Métriques GPU manquent quand cuda feature off | Moyenne | Faible | `register_all_gpu` no-op quand feature absente |
| **T-R-8** | OTel collector dropping spans | Basse | Faible | Sampling rate configurable + queue size |
| **T-R-9** | Grafana provisioning échoue (Helm CM oversize) | Basse | Moyen | Split en multiple ConfigMaps si > 1 MiB |
| **T-R-10** | Prometheus ne scrape pas (bad TLS) | Basse | Élevé | mTLS optionnel sur /metrics ; default plain |

---

## §19. Volumétrie code attendue

```
Sprint     LOC delta  Tests   Bench
─────────────────────────────────────
D29         +550      8      1
D34        +1230     12      0
D37         +600      6      1
─────────────────────────────────────
Net total  +2380     26      2
```

---

## §20. Plans d'implémentation détaillés sprint-par-sprint

### §20.1 D29 (5 jours)

| Jour | Tâche | Livrable |
|---|---|---|
| J-1 | Étendre `gpu_instruments.rs` 2→6 instruments | gpu_instruments.rs |
| J-2 | Wirer `KernelCallGuard` côté efe-kernels | adapter |
| J-3 | Wirer `HybridFusionReport` → canary instruments | adapter |
| J-4 | Dashboard `d29_gpu_canary.json` | JSON file |
| J-5 | Pin tests + cardinality budget | tests |

### §20.2 D34 (7 jours)

| Jour | Tâche | Livrable |
|---|---|---|
| J-1 | `nano_agent_instruments.rs` (NEW) | module |
| J-2 | `mode2_instruments.rs` (NEW) | module |
| J-3 | Wirer `genesis::spawner::on_spawn` → population_total | adapter |
| J-4 | Wirer `Mode2Controller::cache.get` → hit/miss | adapter |
| J-5 | Dashboard `d34_templates.json` | JSON |
| J-6 | Dashboard `d34_mode2.json` | JSON |
| J-7 | Pin tests + cardinality + Helm CM update | tests + ops |

### §20.3 D37 (4 jours)

| Jour | Tâche | Livrable |
|---|---|---|
| J-1 | `mode3_instruments.rs` (NEW) | module |
| J-2 | Wirer `Mode3Aggregator::aggregate.dispersion` → gauge | adapter |
| J-3 | Dashboard `d37_mode3.json` | JSON |
| J-4 | Pin tests + cardinality + bench | tests |

---

## §21. Annexes — instruments specs complètes (50 entrées)

### §21.1 Existing 31 instruments (D1-D18)

(Trop nombreux pour détailler — voir
`crates/efe-telemetry/src/metrics/d{13,13bis,14,14bis,15,16,17,18}_instruments.rs`)

### §21.2 D29 6 instruments

(Cf. § 9.3 — code skeleton § 9.4)

### §21.3 D34 9 instruments

(Cf. § 10.3 + 10.4 — code skeleton § 10.5)

### §21.4 D37 4 instruments

(Cf. § 11.2 — code skeleton § 11.3)

---

## §22. Annexes — dashboard JSON skeleton (4 fichiers)

### §22.1 `d29_gpu_canary.json` skeleton

```json
{
  "uid": "efe-d29-gpu-canary",
  "title": "EFE — GPU Canary (Sprint D29)",
  "tags": ["efe", "gpu", "canary"],
  "schemaVersion": 38,
  "version": 1,
  "refresh": "30s",
  "time": { "from": "now-1h", "to": "now" },
  "templating": {
    "list": [
      {
        "name": "gpu_id",
        "type": "query",
        "query": "label_values(efe_gpu_utilization, gpu_id)",
        "multi": true
      }
    ]
  },
  "panels": [
    {
      "id": 1,
      "title": "GPU Utilization (%)",
      "type": "timeseries",
      "targets": [{
        "expr": "efe_gpu_utilization{gpu_id=~\"$gpu_id\"}",
        "legendFormat": "{{gpu_id}}"
      }]
    },
    {
      "id": 2,
      "title": "GPU Memory Used (bytes)",
      "type": "timeseries",
      "targets": [{
        "expr": "efe_gpu_memory_used_bytes{gpu_id=~\"$gpu_id\"}"
      }]
    },
    {
      "id": 3,
      "title": "Kernel Latency p99 (ms)",
      "type": "heatmap",
      "targets": [{
        "expr": "histogram_quantile(0.99, sum(rate(efe_gpu_kernel_seconds_bucket[1m])) by (le, kernel))"
      }]
    },
    {
      "id": 4,
      "title": "Canary Divergence Rate",
      "type": "stat",
      "targets": [{
        "expr": "rate(efe_gpu_canary_divergence_total[5m])"
      }],
      "thresholds": { "mode": "absolute", "steps": [
        { "color": "green", "value": null },
        { "color": "yellow", "value": 0.01 },
        { "color": "red", "value": 0.05 }
      ]}
    }
  ]
}
```

### §22.2 `d34_templates.json` skeleton

(structure similaire — 4 panels : population pie, NaSafety per
tenant, weights table, kernel routing heatmap)

### §22.3 `d34_mode2.json` skeleton

(4 panels : cache hit rate stat, router latency heatmap, anchors
generated histogram, hit/miss stacked area)

### §22.4 `d37_mode3.json` skeleton

(4 panels : k-swarm throughput, dispersion heatmap, k-clamped
sparkline, aggregator latency)

---

## §23. Annexes — RACI + budget

| Sprint | Lead (R) | Approver (A) | Consulted (C) | Informed (I) |
|---|---|---|---|---|
| D29 | rodrigue.pekar | tech-lead | GPU-team, SRE | product |
| D34 | rodrigue.pekar | tech-lead | data-team, SRE | product |
| D37 | rodrigue.pekar | tech-lead | SRE | product, ops |

Budget total : ~16 jours-ingénieur (3 sprints × moyenne 5,3 jours).

---

## §24. Annexes — Gantt prévisionnel D29 → D37

```text
Semaine    1     2     3     4
        ┌────┬────┬────┬────┐
D29     █████      │     │
D34          █████████  │
D37                 ████
        └────┴────┴────┴────┘
GA      ......................★ v1.0
```

---

## §25. Mises à jour du document

### §25.1 Versioning

* v1.0.0 — 2026-04-29 — Première édition.

### §25.2 Liens à maintenir

* `EFE_KERNELS_ARCHITECTURE.md` § 25 — mapping instruments GPU.
* `EFE_SERVER_ARCHITECTURE.md` § 18 — observabilité Mode-2/3 panels.
* `EFE_CORE_EXTENSION_ARCHITECTURE.md` § 19 — SLO contracts.

### §25.3 Critères d'archive

* Les 3 sprints D29-D37 ont shippé.
* Le `CHANGELOG.md` `[1.0.0]` entry est livré.
* L'audit pass M5 est terminé.
* Le tag `efe-telemetry@1.0.0` est créé.

---

**Fin du document.**

Total LOC : ~1 700 lignes Markdown.
Total LOC delta cible (M5 extension) : ~2 380 LOC (Rust + JSON +
Python).
Total durée : ~16 jours-ingénieur sur 3 sprints non-contigus
(Φ-D → Φ-F).
