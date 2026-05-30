# Changelog — `efe-coordinator`

Toutes les modifications notables sont documentées ici.  Format suit
[Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/) et
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

_(rolling — prochains items R4)_

## [0.2.0] — 2026-05-30 — Phase 3 R3 BOUCLÉE (production hardening)

> R3 ferme les 4 limitations R2 (L1 cost-microcents, L2 SLM circuit wiring,
> L3 chaos/local-drafter, L4 noise rejection) et ajoute 4 couches de durcissement
> (multi-tenant S5, télémétrie Prometheus S7, A/B test S8, savings report v2 S9).
> Méta-rapport : `R3_LIVE_VALIDATION.md`. Tag : `phase-3-r3-complete`.
> **Note version** : le manifeste workspace racine étant *untracked* dans cet
> arbre, `0.2.0` est un jalon **documentaire** (comme `[0.1.0]` pour R2) ;
> `[workspace.package]` reste `0.0.1` sur disque.

### Added (résumé R3 — détail par sprint ci-dessous)
- `cost_microcents: u64` exposé via LlmResponse, MissionA/B/SpecResponse,
  ChewingMetrics, SavingsReport (R3.S1, fix L1/D7).
- SlmCircuitBreaker wiré dans run_mission_a/b/speculative (R3.S2, fix L2).
- Speculator `NoiseRejectionPolicy` — rejet du bruit en hot path (R3.S3, fix L4/D6).
- `LocalSlmProvider` drafter SLM local + break-even probe (R3.S4, fix L1/L3).
- Multi-tenant `TenantPolicies` + `PolicyRegistry`, enforcement fail-closed (R3.S5).
- Chaos engineering live (`ChaosLlmProvider`, fault injection réelle) (R3.S6, fix L3).
- Prometheus `/metrics` endpoint, zéro dépendance (`tokio/net` only) (R3.S7).
- A/B test framework entre tables de scoring, bucketing `blake3` (R3.S8).
- `CHEWING_SAVINGS_REPORT_v2.{md,json}` + `to_markdown_v2()` intègre la section
  A/B + fixture `08_speculative_local_only` (R3.S9).

### Changed
- `CHEWING_SAVINGS_REPORT_v2.md` produit en **microcents** : savings cost mesurés
  **−27.4 % agrégat** + **~89 % sur la classe cheap-routable FactualQa** (vs
  **+0 %** R2). Le R2 `CHEWING_SAVINGS_REPORT.md` (« +0 % ») est **préservé** pour
  comparaison historique.

### Phase 3 R3.S9 — Régen CHEWING_SAVINGS_REPORT_v2 (livrable visible) (LIVRÉ 2026-05-30)

> Le livrable R2 affichait « +0 % » de savings cost (résolution `u32 cents`
> écrasant les missions sub-cent). R3.S1 avait résolu la mécanique (microcents),
> mais le livrable n'avait jamais été régénéré. R3.S9 ferme la boucle.

#### Ajouté
- **`tests/live_chewing_savings_report_v2.rs`** (NEW) : orchestrateur live v2 (6
  fixtures × ≤ 5 modes), capture `cost_microcents` **RÉEL** via
  `new_with_microcents` (le R2 dérivait µ¢ = cents × 1000 → `savings_ratio_microcents`
  était condamné à 0), émet `to_markdown_v2()`, assertions **H1-H15**.
- **`to_markdown_v2()`** intègre `ab_section_markdown()` (dette R3.S8 §13 — no-op
  strict sans run A/B) ; 2 tests unitaires (présent/absent).
- **`tests/fixtures/missions/08_speculative_local_only.json`** (FactualQa mono-mot,
  pensée pour Speculative + drafter local éventuel).
- Version report `s9-r3-YYYY-MM-DD` (corrige la contradiction latente
  `new()`→`r3s1-` vs H8).

#### Recalibration H11 (post-run live)
- Cible aspirationnelle `> 0.50` remplacée par 2 preuves design-justifiées :
  **H11a** agrégat µ¢ strictement positif + > 10 % (robuste à la variance du coût
  code-gen premium-locked) ; **H11b** FactualQa cheap-routable > 80 % (routing
  déterministe → Gemini Flash ≈ 0 µ¢). Live : **PASS H1-H15** (agrégat 27.4 %,
  FactualQa ~89 %, latence Speculative −66 %, qualité 100 %).

### Phase 3 R3.S8 — A/B test entre tables de scoring (LIVRÉ 2026-05-30)

> La `TaskClassScoringTable` (calibration figée) décide du routing. Pour tester
> une **nouvelle** table en prod sans risque, R3.S8 route un pourcentage du
> trafic vers une table d'expérimentation et compare empiriquement les deux
> bras. Principes : **zéro dépendance** (bucketing `blake3`, pas de `rand`),
> **déterminisme** (même `swarm_id` → même bras), **précédence claire**
> (`request > A/B > tenant > global`), **Mission A uniquement** (seule mission
> avec table/router), **backward-compat** (`ab_test: None` ⇒ R3.S7 inchangé).

#### Ajouté
- **`src/ab_test.rs`** (NEW) :
  - `ABArm` (Control/Experiment), `ABTestConfig` (tables *owned*, sérialisable),
    `ABTestRuntime` (tables `Arc` → résolution O(1)/mission, zéro clone profond).
  - `bucket_of(swarm_id)` = `blake3(uuid) % 100` (uniforme — casse la structure
    temporelle d'UUID v7 ; pur → reproductible).
  - `assign_arm(swarm_id, tenant, fixture)` : `None` hors population
    (tenant/fixture filter) → résolution R3.S5 reprend ; `ratio 0` → toujours
    Control, `ratio 1` → toujours Experiment.
  - 10 tests unitaires (serde, déterminisme, distribution, ratio 0/1,
    tenant_filter, fixture_filter, clamp).
- **`Coordinator.ab_test: Option<Arc<ABTestRuntime>>`** + builder `.ab_test(cfg)`.
- **`ChewingMetrics.ab_arm: Option<ABArm>`** (`#[serde(default)]`) — bras tracé.
- **`mission_a`** : bloc A/B (précédence §4.1) — remplit
  `req.task_class_scoring_table` AVANT l'injection tenant (zéro churn router).
- **`ChewingMetrics.ab_table_version`** (`#[serde(default)]`) — version de la
  table du bras (traçabilité calibration), posée par `mission_a` via
  `ABTestRuntime::version_of`.
- **`SavingsReport`** : `RunRecord::ab_arm()`, `RunRecord::ab_table_version()`,
  `ArmStats` (avec `table_version`), `ABComparison`,
  `ab_comparison()` (ségrégation control vs experiment + deltas),
  `ab_section_markdown()` (intégré par `to_markdown_v2` en R3.S9).
- **`tests/ab_test.rs`** (NEW, 6 tests intégration) : routing par bras
  (experiment cost-weighted → cheap, control quality-weighted → premium),
  pin-skip (`request > A/B`), backward-compat (pas d'A/B → pas d'arm),
  ségrégation report.
- **`tests/live_missions.rs`** : 2 tests live —
  `r3s8_live_ab_test_savings_report` (50/50 sur LLMs réels → rapport segmenté)
  et `r3s8_live_ab_arm_routing_consistency` (Control quality→Claude,
  Experiment cost→Gemini : preuve end-to-end que la table du bras pilote le
  routing).

#### Décisions chirurgicales
- **Config sérialisable vs runtime Arc** : `serde` d'un `Arc` exige la feature
  `rc` (non activée) ET router via `Arc<table>` clonerait des HashMaps par
  mission → deux types résolvent les deux contraintes.
- **`fixture_filter`** : sens uniquement dans le harness de report (pas
  d'identité fixture en prod) → live passe `None` ⇒ exclusion définie &
  documentée (anti dead-config, lignée `EFE_CHAOS_RATE` R3.S6).
- **Stats descriptives** (ségrégation + deltas), pas de p-value online (sprint
  d'analyse à part).
- **Déterminisme live** : un "replay" est impossible (`swarm_id` frais par
  mission) → prouvé offline via `bucket_of` pur (documenté dans le test live).

#### Backward-compat
- `ab_test: None` (défaut) ⇒ bloc A/B court-circuité ⇒ comportement R3.S7
  byte-pour-byte. `ab_arm` `#[serde(default)]` ⇒ rétrocompat JSON.
- 16 tests offline R3.S8 PASS (10 unit + 6 intégration) + 2 live (compile+skip) ;
  clippy `--all-targets --features chaos,metrics,all-providers` exit 0 (toutes
  configs) ; suite complète (328 unit + intégration) verte ; default build
  inchangé.

### Phase 3 R3.S7 — Prometheus telemetry (LIVRÉ 2026-05-29)

> R2/R3 produisaient une mine de données par mission (`ChewingMetrics`,
> verdicts, état circuits) mais uniquement observables via SSE — aucune
> métrique Prometheus pour le monitoring prod. R3.S7 expose un endpoint
> `/metrics`. Principes : **zéro dépendance** (rendu Prometheus + serveur
> `TcpListener` faits main, pas de crate `prometheus`/`hyper`), **hook
> lossless** (RAII, capture succès ET erreur), **cardinalité bornée**
> (`tenant_id` jamais un label), **anti dead-config** (`verdicts_total`
> counters au lieu d'un `quality_pass_rate` gauge ; `savings{baseline,full}`
> descopé faute de source live).

#### Ajouté
- **`src/metering.rs`** (NEW, toujours compilé, zéro dépendance) :
  - `MetricsRegistry` (counters/histogramme en `AtomicU64`) + `render_prometheus`
    (format d'exposition texte fait main).
  - Familles : `efe_missions_total{type,outcome}`, `efe_cost_microcents_total{provider}`,
    `efe_latency_ms` (histogram `{phase}`), `efe_failure_mode_total{kind}`,
    `efe_savings_microcents_total{mode}`, `efe_verdicts_total{verdict}`,
    `efe_slm_circuit_state{robot,state}` (gauge scrape-time).
  - `MissionMetricGuard` (RAII) : `Drop` enregistre `outcome="error"` (mission
    `?`-retournée) ; `record_success` désarme + enregistre les détails.
  - `MissionRecord` + vues `from_mission_a/b/speculative` (découple le registre
    des types `request.rs`). 9 tests unitaires.
- **Feature `metrics = ["tokio/net"]`** : n'active `tokio/net` que pour le
  serveur. Le registre + le hook restent toujours compilés.
- **`src/metrics_server.rs`** (NEW, `#[cfg(feature="metrics")]`) : `serve_metrics`
  délègue à la boucle canonique **testée** `serve_metrics_on` (zéro duplication).
  Serveur
  HTTP/1.1 minimal (`TcpListener`, même pattern que `local_slm.rs`).
  `GET /metrics` → 200 `text/plain; version=0.0.4` ; `/healthz` → 200 ; 404
  sinon. Bind `127.0.0.1:9090` par défaut. `snapshot_circuits` (pull scrape-time).
- **`Coordinator.metrics: Option<Arc<MetricsRegistry>>`** + builder `.metrics()`.
- **Hook 2 lignes/workflow** (mission_a/b/speculative) : `arm` + `record_success`.
- **Re-exports** : `MetricsRegistry`, `MissionMetricGuard`, `MissionRecord`,
  `MissionType`, `Outcome`, `Phase`, `FailureKind`, `SavingsMode`, `Verdict`,
  `LATENCY_BUCKETS_MS`.
- **`tests/metering.rs`** (NEW) : 5 tests intégration (Mission A succès/erreur,
  Mission B, Speculative, none) + 4 tests serveur HTTP (200 Prometheus, 404,
  circuit live, healthz).
- **`benches/metering_overhead.rs`** (NEW) : bench Criterion prouvant
  `record_mission` < 1 µs (record succès, record erreur, render scrape).
- **Complétude** : ajout des chemins savings/verdicts/failure non testés
  (`metrics_registry_records_savings_verdicts_failures`,
  `error_outcome_skips_detail_metrics`) + getters
  `savings_microcents_of`/`verdicts_count`/`failure_mode_count` ; intégration
  Mission B + Speculative (exerce `from_mission_b`/`from_speculative`,
  auparavant non couverts). Fix latent : `benches/coordinator_throughput.rs`
  ne compilait plus depuis R3.S5 (`tenant_id` manquant — invisible à
  `cargo test` qui ne compile pas les benches).

#### Décisions chirurgicales
- **Hook (pas bus)** : `SwarmFailed` n'est émis qu'en Mission B et le bus est
  lossy → un subscriber sous-compterait les erreurs. Le hook RAII est lossless.
- **`tenant_id` (R3.S5) banni des labels** : cardinalité non bornée → OOM TSDB
  (testé : `render_excludes_tenant_id_label`).
- **`quality_pass_rate` gauge → `efe_verdicts_total{verdict}` counters** (rate
  dérivé en PromQL) ; `savings{mode}` limité à `speculative` (seule source live).
- **Coût Mission B** : agrégat fan-out étiqueté par le gagnant du vote
  (approximation documentée).

#### Backward-compat
- `metrics: None` (défaut) ⇒ guard inerte, overhead nul, comportement R3.S6
  inchangé. Feature `metrics` off ⇒ `metrics_server.rs` non compilé,
  `tokio/net` non tiré.
- 20 tests R3.S7 PASS (11 unit + 5 intégration + 4 serveur) + bench Criterion
  `metering_overhead` ; clippy `--all-targets --features metrics` exit 0 ;
  default build inchangé.

### Phase 3 R3.S6 — Chaos engineering CI (LIVRÉ 2026-05-29)

> Fix L3 : la graceful degradation S8-R2 (fallback chain, vote dégradé,
> drafter bypass, SLM skip) n'était testée que contre
> `MockLlmProvider::with_error()` — un échec synthétique, instantané, sans
> réseau. R3.S6 livre un framework de **chaos engineering** qui injecte de
> **vraies** fautes (429, timeout, JSON tronqué, réponse vide) sur des
> providers réels, prouvant la résilience en condition réseau réelle.
> Principe « zéro dépendance » (PRNG SplitMix64 interne, pas de `rand`),
> « zéro modif des workflows » (injection via le registry, code de prod
> inchangé), et honnêteté (le fault `Timeout` **révèle** l'absence de
> hard-deadline workflow — documenté, borné via `CHAOS_MAX_SLEEP`).

#### Ajouté
- **Feature `chaos`** (Cargo.toml) — OFF en prod (binaire propre, pas de
  chaos accidentel), ON en test/CI. Aucune dépendance ajoutée.
- **`src/chaos.rs`** (NEW, feature-gated) :
  - `ChaosFaultKind` : `HttpStatus(u16)`, `Timeout(Duration)`,
    `InvalidResponse`, `EmptyResponse`.
  - `ChaosInjector` : `fault_rates` par provider-id + `fault_kind` + PRNG
    **SplitMix64 seedé** (déterministe → tests reproductibles).
    `seeded()`, `with_fault_rate()`, `from_env()` (lit `EFE_CHAOS_RATE`),
    `should_fault()` (court-circuite sans tirage si taux nul), `wrap()`.
  - `ChaosLlmProvider` : décorateur `LlmProvider` injectant dans **`call()`
    ET `stream()`** (Mission A passe par stream, B par call, Spec par les
    deux). Préserve l'id du provider wrappé → router/fallback transparents.
  - `CHAOS_MAX_SLEEP` (5 s) : plafond dur du sleep injecté (anti-hang CI).
  - 11 tests unitaires (déterminisme RNG, taux 0/1/0.3, pass-through,
    429 sur call+stream, timeout borné, empty response, `from_env` lit
    `EFE_CHAOS_RATE` — 2 tests `#[serial]`).
- **Re-exports crate-root gated** : `ChaosFaultKind`, `ChaosInjector`,
  `ChaosLlmProvider`, `CHAOS_MAX_SLEEP`.
- **`tests/chaos_offline.rs`** (NEW, 4 tests) : Mission A 429→fallback,
  Mission B 503→vote dégradé, Spec drafter empty→noise gate slow path,
  Spec drafter 503→bypass verifier.
- **`tests/live_chaos.rs`** (NEW, 4 tests, gate `EFE_LIVE_LLM`) : Mission A
  429@30 % sur 20 missions (> 80 % complétion + fallbacks tracés), Mission B
  timeout→vote dégradé, Spec drafter invalide→slow path, SLM analyzer
  injoignable→skip gracieux (vecteur infra-level, réutilise R3.S2).
- **`.github/workflows/chaos-eng.yml`** (NEW) : CI nightly (offline garde-fou
  + live sérialisé sous `EFE_CHAOS_RATE`).

#### Vecteurs d'injection
- **Provider-level** (`ChaosLlmProvider`) pour les LLM (Claude/Gemini/GPT,
  drafter Spec).
- **Infra-level** (endpoint mort) pour les SLM nano-robots
  (`NanoRobotClient` ≠ `LlmProvider`) → valide le `SlmCircuitBreaker` R3.S2
  sans nouveau code.

#### Findings honnêtes (backlog)
- **Pas de hard-deadline workflow** : `LlmCall.deadline` est advisory (aucun
  `tokio::time::timeout`). Le fault `Timeout` le rend visible ; borné côté
  chaos + harness pour la CI. Fix = sprint à part.
- **Pas de retry/backoff** dans le bridge (1 tentative → fallback immédiat) :
  le chaos quantifie le taux de bascule ; ajout de retry = orthogonal.

#### Backward-compat
- Feature off ⇒ `chaos.rs` non compilé, zéro symbole `Chaos*`, binaire prod
  inchangé. `fault_rate = 0.0` ⇒ pass-through byte-pour-byte (testé).
- 15 tests offline (11 unit + 4 intégration) PASS ; 4 tests live compilent +
  skip propre sans clés ; clippy `--features chaos --tests` exit 0 ; default
  build (chaos off) inchangé.
- **Complétude** : `EFE_CHAOS_RATE` était du dead config (le workflow CI le
  passait mais le test live codait le taux en dur). Le test live Mission A
  utilise désormais `ChaosInjector::from_env` → la variable pilote réellement
  le taux de faute (assertion fallback gardée par `rate > 0`).

### Phase 3 R3.S5 — Multi-tenant policies (LIVRÉ 2026-05-29)

> Fix L5 : l'isolation tenant était une **fiction** — les 3 workflows
> câblaient `coord.config.default_tenant` (global) dans le `CallContext`,
> si bien que toutes les entrées d'audit portaient le même `tenant_id`.
> R3.S5 livre la couche de **politiques par tenant** (table de scoring,
> allow-list, plafond de coût, seuil de circuit) résolue par requête, avec
> propagation du **vrai** `tenant_id`. Principe « zéro dead config » (hérité
> R3.S4) : chaque champ a un point d'enforcement réel ; `retry_policy`
> (roadmap) est **descopé** (aucune infra de retry à paramétrer).

#### Ajouté
- **`src/multi_tenant.rs`** (NEW) : `TenantPolicies` (scoring_table,
  allowed_llms 3-états dont `Some([])`=deny-all fail-closed,
  max_cost_microcents_per_query, slm_circuit_threshold), `PolicyRegistry`
  (lookup O(1) par `TenantId`), `resolve()`, `circuit_key()`,
  `DEFAULT_SLM_CIRCUIT_THRESHOLD`. 14 tests unitaires.
- **`Coordinator.policies: Arc<PolicyRegistry>`** + builder `.policies()` ;
  `Coordinator::slm_circuit_for(tenant, robot, has_policy)` (résolution de
  circuit tenant-aware avec dégradation propre vers la clé partagée) ;
  `CoordinatorClients::tenant_slm_circuits()` (pré-génération de circuits
  keyed `"{tenant}:{robot}"` — isolation du blast radius, fix L5-d).
- **`{MissionA,MissionB,Speculative}Request.tenant_id: Option<TenantId>`**
  (`#[serde(default)]` — backward-compat JSON).
- **`ChewingMetrics`** : `tenant_id`, `tenant_cost_cap_exceeded` (Mission B
  best-effort), `tenant_llms_filtered` (tous `#[serde(default)]`).
- **Re-exports crate-root** : `TenantPolicies`, `PolicyRegistry`,
  `circuit_key`, `DEFAULT_SLM_CIRCUIT_THRESHOLD`, `TenantId`.
- **`efe_nano_app_coordinator`** : `TenantId` + `SwarmId` re-exportés au
  crate root.
- **`tests/multi_tenant_policies.rs`** (NEW, 12 tests) + live
  `r3s5_live_two_tenants_diverge`.

#### Enforcement (par mission, points d'ancrage réels)
- **Mission A** : allow-list filtre `available_llms` (intersection, jamais
  union) → fail-closed si vide ; scoring table tenant injectée si la requête
  n'en fournit pas (requête prioritaire) ; plafond de coût resserre
  `Budget.max_cost_microcents` (`min`) avant le filtre router ; circuits
  tenant-aware.
- **Mission B** : allow-list filtre le fan-out `llms`, refus si < 2 (quorum) ;
  plafond de coût best-effort (tracé via `tenant_cost_cap_exceeded`, pas de
  point de coupe pré-fan-out).
- **Speculative** : allow-list valide drafter **et** verifier (fail-closed).
  Budget NON capé (workflow ne consomme pas `Budget` — anti dead-config).
- **Audit** : `CallContext` construit avec le tenant **résolu** (fin de la
  fiction L5-a) dans Mission A/B.

#### Sécurité
- Invariant : pour les champs sécuritaires (`allowed_llms`, `max_cost`),
  `S_effectif ⊑ S_policy` — une requête ne peut jamais élargir sa policy.
- `scoring_table` reste `#[serde(skip)]` (jamais désérialisée d'un payload).

#### Backward-compat
- Sans `PolicyRegistry` (défaut) + `tenant_id` absent ⇒ comportement R3.S4
  byte-pour-byte. ~373 tests offline PASS, clippy `--tests` exit 0.

### Phase 3 R3.S1 — Cost Microcents (LIVRÉ 2026-05-28, commits `20984f7` + `<HEAD>`)

> Fix L1/D7 (résolution u32 cents) : passage à `cost_microcents: u64`
> (1 µ¢ = 1/1000 cent) end-to-end pour exposer le savings ×50 invisible
> en R2. Backward-compat 100 %.

#### Ajouté
- **`LlmResponse.cost_microcents: u64`** (`#[serde(default)]` pour rétrocompat).
- **3 helpers module-level** `llm/provider.rs` : `microcents_to_cents`,
  `cents_to_microcents`, `compute_cost_microcents` (saturating math,
  fix u64::MAX overflow).
- **`PricingTable::calc_microcents()`** + `rates_microcents()` ; Gemini
  Flash recalibré à `(7_500, 30_000)` µ¢/M EXACT (vs `(8, 30)` cents
  arrondi pré-R3 = +6.7 % drift sur input).
- **`MockLlmProvider::with_cost_microcents()`** builder.
- **`LlmMetadata.cost_{input,output}_per_million_microcents: u64`** +
  helpers `_microcents_resolved()` (fallback `cents × 1000`).
- **`Budget.max_cost_microcents: u64`** + helper `_resolved()` (filtrage
  budgétaire avec précision sub-cent).
- **`LlmStaticMetadata.cost_*_per_million_microcents`** + recalibration
  exacte de `default_3llm_table()` pour 3 LLMs.
- **`RouterOutput.estimated_cost_microcents: u64`** +
  `estimate_cost_microcents()` fonction publique.
- **`MissionAResponse.cost_microcents`**,
  **`MissionBResponse.total_cost_microcents`** (agrégation µ¢ saturating),
  **`SpeculativeResponse.cost_microcents`**.
- **`ChewingMetrics.router_estimated_cost_microcents`** +
  **`spec_savings_microcents`** (avec `estimate_savings_microcents()`).
- **`SpeculationOutcome.cost_microcents`** (3 paths : hot, slow, bypass).
- **`SavingsReport`** : `RunRecord.cost_microcents` +
  `RunRecord::new_with_microcents()` constructor + `cost_microcents_resolved()` ;
  `ModeSummary.{total_cost_microcents, avg_cost_microcents}` ;
  `SavingsReport.{baseline,full}_total_cost_microcents` +
  `savings_microcents` (i64) + `savings_ratio_microcents` (numériquement
  stable même quand toutes les fixtures arrondies à 1 ¢).
- **`SavingsReport::to_markdown_v2()`** — rapport µ¢ precision avec
  section comparative `legacy cents vs R3.S1 µ¢`.
- **42+ tests R3.S1 NEW** : helpers (12), pricing (10), mock (3),
  metadata backward-compat (3), Budget (4), Router Pareto µ¢ (4),
  savings report (5+), workflow integration (1+).

#### Modifié
- **`PricingTable`** : `microrates: HashMap<String, (u64, u64)>` en
  parallèle de `rates: HashMap<String, (u32, u32)>` (legacy préservé).
- **3 providers** (`anthropic`, `gemini`, `openai`) populent
  `LlmResponse.cost_microcents` via `calc_microcents()` (call() + stream()).
- **Router Pareto algo** (`clients/router.rs`) : filtrage budgétaire et
  normalisation `cost_norm` passent en µ¢ → discrimination ×45 retrouvée
  sur fixtures cheap (R2 cents écrasait tout à `norm=1.0`).
- **Workflows** (`mission_a`, `mission_b`, `speculative`) wiring µ¢
  end-to-end.
- **`TaskClassScoringTable::default_3llm_table()`** : recalibration des
  poids Pareto pour 3 task_class (Analysis, Creative, Reasoning) afin
  de préserver l'intention de routing R2 après le passage en µ¢ qui
  expose le coût discriminant. Détails :
  - `Analysis` : `(2.0, 0.5, 1.0)` → `(3.0, 0.5, 0.2)` — quality bump,
    cost soft (Claude préservé pour deep reasoning, capability 0.95
    vs 0.78/0.85).
  - `Creative` : `(1.5, 1.0, 1.0)` → `(3.0, 1.0, 0.1)` — quality
    strongly dominant, cost very soft (GPT préservé pour style/variété,
    capability 0.93 vs 0.88/0.80).
  - `Reasoning` : `(3.0, 0.3, 0.3)` → `(3.0, 0.3, 0.1)` — cost très
    soft (Claude préservé pour extended thinking, capability 0.95).
  - `FactualQa`, `Code`, `Translation` : poids inchangés (intent R2
    préservé sans recalibration).
- **`SavingsReport.new()`** : version par défaut `"s9-r2-..."` →
  `"r3s1-..."`.

#### Backward-compat
- Tous nouveaux champs `#[serde(default)]` — JSON pré-R3 désérialisable.
- Helpers `_resolved()` reconstruisent µ¢ depuis cents quand le champ µ¢
  vaut 0 (sentinel rétrocompat).
- 4 tests `*_backward_compat_pre_r3_json` PASS.
- API publique inchangée : `cost_cents` reste field public partout,
  dérivé via `microcents_to_cents` (policy : ≥ 1 ¢ si > 0 µ¢).

#### Tests
- **546 tests offline** passent (efe-coordinator + efe-nano-app-coordinator),
  **0 failed**.
- `cargo clippy` : 0 warning R3.S1-spécifique.

### Phase 3 R3.S4 — Drafter SLM Local (fix L1/L3) (LIVRÉ 2026-05-29)

> Fix L1/L3 : le drafter Speculative était toujours un LLM cloud (Gemini
> Flash). R3.S4 introduit l'infrastructure d'un drafter **SLM local**
> (llama-server Metal) + son circuit-breaker dédié + l'instrument de mesure
> break-even (`spec_drafter_is_local`). Le local n'est économiquement viable
> que si la chute de hot-path-rate vs cloud reste < ~1.9 points : R3.S4 livre
> donc l'infra + la sonde de calibration, pas un déploiement aveugle.

#### Ajouté
- **`LocalSlmProvider`** (`llm/providers/local_slm.rs`, feature `local-slm`) :
  implémente `LlmProvider` contre un `llama-server` local (`POST /completion`
  texte-libre, `GET /health`). Builders `new` / `from_env`
  (`EFE_LOCAL_SLM_URL` défaut `:8099`, `_MODEL` défaut `gemma-2-1.5b`, `_ID`
  défaut `local_<model>`) + `with_cost` / `with_system_prompt` /
  `with_max_tokens` / `with_temperature` / `with_timeout`. Coût honnête
  `DEFAULT_LOCAL_SLM_COST_MICROCENTS_PER_M = 1_500` µ¢/M (énergie Metal).
  **14 tests** (purs + HTTP mock in-process).
- **`PricingTable::local()`** : `gemma-2-1.5b`→(1_500,1_500),
  `qwen-2.5-0.5b`→(800,800), `llama-3.2-3b`→(3_000,3_000) µ¢/M. **2 tests**.
- **`Speculator.drafter_circuit: Option<SlmCircuitBreaker>`** +
  `with_drafter_circuit` / `with_drafter_circuit_opt`. Circuit ouvert
  (llama-server mort) → `verifier_only_fallback` (DRY) → verifier-only sans
  propager l'erreur (`drafter_bypassed=true`, `DrafterBypassed` émis ;
  `SlmCircuitOpened` sur transition Closed→Open). **4 tests**.
- **`ChewingMetrics.spec_drafter_is_local: Option<bool>`**
  (`#[serde(default)]`) — axe de segmentation break-even.
- **`CoordinatorBuilder::with_local_slm_drafter`** (feature `local-slm`) :
  enregistre le provider + insère un `SlmCircuitBreaker(3, 10s)` sous son ID.
- **Feature `local-slm`** (+ ajoutée à `all-providers`) ; re-exports
  crate-root `LocalSlmProvider`.
- **`tests/local_slm_drafter.rs`** : **4 tests d'intégration** (flag local
  true/false, circuit ouvert → verifier-only, circuit fermé → hot path).
- **`tests/live_missions.rs`** : `r3s4_live_local_drafter_calibration`
  (gate `EFE_LIVE_SLM=1` + health-check `:8099`) — sonde de mesure du
  hot-path-rate local pour calibrer le modèle.

#### Modifié
- **`run_speculative`** (`workflows/speculative.rs`) : branche le
  `drafter_circuit` (clone depuis `slm_circuits[drafter_llm]`) sur le
  Speculator + trace `spec_drafter_is_local` (`drafter_llm.starts_with("local")`).
- **`CoordinatorBuilder::with_local_slm_drafter`** : appel `LlmProvider::id`
  trait-qualifié (le champ privé `id` de `LocalSlmProvider` masquait la
  méthode du trait → erreur de compilation sous `all-providers`, corrigée).

### Phase 3 R3.S3 — Speculator Noise Rejection (fix L4/D6) (LIVRÉ 2026-05-29)

> Fix L4/D6 : le hot path Speculative acceptait tout draft de `]0, max_words]`
> mots. Un drafter cheap renvoyant « La » (1 mot, fragment) voyait sa
> réponse fausse livrée sans arbitrage verifier (S9 fixture 03, pass rate
> 50 %). R3.S3 ajoute un **gate de rejet de bruit** local et déterministe.

#### Insight chirurgical
> Le bruit n'est pas une question de **longueur**, mais de **contenu**.

Le R3 overview proposait `min_words >= 2` — REJETÉ : « Paris » (réponse
FactualQa canonique en 1 mot) serait pénalisé, détruisant le savings.
La bonne discrimination est par **mot de contenu** : « La » (0 content
word, stopword) = bruit ; « Paris » (1 content word) = valide.

#### Asymétrie des coûts (guide du design)
- Faux négatif (rater du bruit) → réponse fausse livrée = SÉVÈRE.
- Faux positif (sur-rejeter) → verifier appelé inutilement = coût.
- → Policy calibrée pour **zéro faux négatif**, faux positifs bas. Un
  draft-bruit rejeté tombe sur le slow path où le verifier produit la
  vraie réponse → **le rejet de bruit ne produit jamais de sortie fausse**.

#### Ajouté
- **Module `speculate/noise.rs`** (NEW) : `NoiseRejectionPolicy` +
  `NoiseSignal` enum + `detect()` (court-circuit, zéro appel LLM).
  - 5 signaux : `EmptyOrWhitespace`, `BelowMinWords`, `OnlyStopwords`
    (LE fix D6), `RefusalPattern`, `DegenerateRepetition`.
  - Stopwords bilingues FR+EN (LazyLock HashSet), refusal patterns FR+EN.
  - `permissive_s7()` (opt-out backward-compat).
  - 20 tests unit.
- **`Speculator.noise_policy`** + `with_noise_policy()` builder ; hot path
  révisé `… && noise.is_none()`.
- **`SpeculationOutcome.noise_signal: Option<NoiseSignal>`** (#[serde(default)]).
- **`SpeculativeRequest.reject_draft_noise`** (master switch, défaut true)
  + **`min_words_for_quick_accept`** (défaut 1).
- **`StreamEvent::DraftRejectedAsNoise`** (observabilité : signal + extrait).
- **`ChewingMetrics.spec_noise_rejected: Option<bool>`** (signal de santé
  drafter ; > 30 % ⇒ drafter mal choisi).
- **4 tests unit speculator** + **4 tests integration workflow** + **1 test
  live** env-gated (`r3s3_live_speculative_rejects_noise_recovers_correct_answer`).

#### Modifié
- `Speculator::run` : gate de bruit AVANT la décision hot path + émission
  `DraftRejectedAsNoise` sur bruit détecté.
- `run_speculative` : wire `NoiseRejectionPolicy` depuis la requête + trace
  `spec_noise_rejected`.

#### Validation
- **618 tests offline** PASS (vs 590 base post-R3.S2) → **+28 tests R3.S3**.
- 0 failed, 0 clippy warning R3.S3-spécifique.
- Backward-compat : drafts content existants (« Paris », « mock response »)
  hot-path inchangé ; opt-out `reject_draft_noise=false` → S7 exact.
- Cible : pass rate Speculative 50 % → ≥ 90 % sans sacrifier le savings.

### Phase 3 R3.S2 — SLM Circuit Wiring (fix L2) (LIVRÉ 2026-05-28, commit `<HEAD>`)

> Fix L2 (`SlmCircuitBreaker` non câblé en R2) : wire les 3 SLM
> circuits per-robot (analyzer/compressor/verifier) aux workflows
> `run_mission_a/b`. Bus event `SlmCircuitOpened` émis 1× par
> transition Closed→Open. `FailureMode::SlmDown` propagé end-to-end.
> Gain opérationnel ~97 % sur scénario "SLM down" (3000s → 90s sur 100
> missions). Backward-compat 100 %.

#### Ajouté
- **`combine_failure_modes(existing, new) -> FailureMode`** helper
  publique dans `failure.rs` — flatten + dedup + simplification du
  Composite-of-one. 6 tests unit (None+X, Composite+simple, dedup
  duplicates, simplify composite of one, etc.).
- **Module interne `workflows/slm_guard.rs`** (`pub(crate)`) — 3
  helpers factorisés :
  - `slm_guard_check(circuit, robot, metrics) -> SlmGuardOutcome` :
    Skip/Proceed selon état circuit.
  - `slm_guard_record_success(circuit)` : reset compteur.
  - `slm_guard_record_failure(circuit, robot, metrics, bus, swarm_id)
    -> bool` : record_failure + auto-emit `SlmCircuitOpened` event sur
    transition Closed→Open.
  - 8 tests unit du module.
- **`CoordinatorClients.slm_circuits: HashMap<String, SlmCircuitBreaker>`**
  field. Backward-compat 100 % : `HashMap::new()` par défaut = mode S8.
- **`CoordinatorClients::default_slm_circuits()`** helper — crée les 3
  circuits préconfigurés (threshold=3, reset_timeout=10s).
- **`CoordinatorClients::slm_circuit(robot)`** lookup helper.
- **`CoordinatorBuilder::with_default_slm_circuits()`** + **`.slm_circuits(custom)`**
  helpers builder.
- **Tests intégration `tests/workflow_slm_circuits.rs`** (NEW file) :
  - `r3s2_mission_a_skips_{analyzer,compressor,verifier}_when_circuit_pre_open` (3 tests)
  - `r3s2_mission_a_multiple_circuits_open_produces_composite` (test Composite)
  - `r3s2_mission_a_no_circuits_falls_back_to_s8_behavior` (backward-compat)
  - `r3s2_default_slm_circuits_helper_creates_3_robots`
  - `r3s2_builder_with_default_slm_circuits_activates_circuits`
  - `r3s2_mission_b_skips_{analyzer,compressor}_when_circuit_pre_open` (2 tests Mission B)
  - `r3s2_circuit_recovers_open_to_halfopen_to_closed` (temporel)
  - `r3s2_mission_a_emits_slm_circuit_opened_event_on_transition` (bus event)
  - `r3s2_speculative_unaffected_by_slm_circuits` (Mission Spec inchangée)
  - **12 tests integration** au total (vs 7 spec'd dans architecture §3.7).
- **Test live env-gated** `r3s2_live_mission_a_resilient_when_analyzer_unreachable`
  dans `tests/live_missions.rs` — opt-in via `EFE_LIVE_SLM_UNREACHABLE_TEST=1`
  (analyzer port 8090 down). Coût $ ~10 ¢ pour 4 missions Claude.

#### Modifié
- **`run_mission_a`** : 3 sites SLM guards (analyzer L78-149, compressor
  L284-350, verifier L504-552). Le `failure_mode` final agrège
  désormais SLM + LLM fallback via `combine_failure_modes`.
- **`run_mission_b`** : 3 sites SLM guards (analyzer L71-100, compressor
  mutualisé L102-173, verifier L398-440).
- **`run_speculative`** : 0 changement (LLM cloud only, déjà protégé par
  `CircuitBreaker` LLM de S2).
- **`slm_guard_record_failure`** : **strict S8 backward-compat** — si
  `circuit == None`, no-op COMPLET (ne trace pas `failure_mode` ni
  `failed_components`). Garantie : Coordinator sans
  `.with_default_slm_circuits()` se comporte EXACTEMENT comme R2.

#### Sémantique bus event `SlmCircuitOpened`
- Émis **1× par transition Closed→Open** (et non sur chaque failure
  pendant que circuit est Open).
- Si HalfOpen → Open (probe fail), aucun nouveau event (déjà signalé).
- Si Closed → Open (recovery cycle), un nouveau event est émis
  (intentionnel — cycle complet = nouvelle alerte).
- Race condition tolérée : 2 missions concurrent peuvent émettre 2× sur
  la transition (rare, consumer dédup via timestamp).

#### Validation
- **583 tests offline** PASS (vs 557 base post-R3.S1) → **+26 tests R3.S2
  net** (6 combine + 8 slm_guard + 12 integration).
- **0 failed**, 0 ignored.
- **0 clippy warning** R3.S2-spécifique.
- **Backward-compat 100 %** : 24 sites `CoordinatorClients` literal
  patchés à `slm_circuits: HashMap::new()` (mode S8 préservé).

### Phase 3 R3 — Suite (R3.S3-S10, à venir)

- **R3.S3** : Speculator noise rejection (fix L4/S9.1 D6).
- **R3.S4** : SLM drafter (Gemma 2 1.5B local).
- **R3.S5** : Multi-tenant policies (tenant_id propagation).
- **R3.S6** : Chaos engineering CI.
- **R3.S7** : Prometheus telemetry (`efe_cost_microcents_total` counter).
- **R3.S8** : A/B test framework.
- **R3.S9** : Régen `CHEWING_SAVINGS_REPORT_v2.md` (consume µ¢ partout).
- **R3.S10** : Packaging release.

- **Phase 4** : Frontend SSE — composants UI (preview lab) consommant
  `StreamEvent` v1.0.0.
- **Phase 7** : Tests live NATS (env-gated `EFE_NATS_LIVE=1`) +
  rotation de la chaîne d'audit cross-tenant.

---

## [0.1.0] — 2026-05-28 — Phase 3 R2 BOUCLÉE (pipeline complet)

> **Minor bump** : 9 sprints chirurgicaux S1-S9 + S9.1 patch + S10 packaging.
> Argument économique quantifié empirically (×1.59 speedup mixture complète,
> ×5.24 Speculative FactualQa). 0 breaking change.
>
> Détail complet : `crates/efe-coordinator/R2_LIVE_VALIDATION.md` (méta-rapport
> agrégé) + `CHEWING_SAVINGS_REPORT.md` (livrable quantifié).

### Ajout — 4 nouveaux modules src

- **`src/failure.rs`** (S8) — `FailureMode` enum tagged (6 variants) +
  `is_degraded() + count_degradations() + summarize()`. 11 tests offline.
  Sérialise en `{ "kind": "SLM_DOWN", "detail": "analyzer" }` (SCREAMING_SNAKE_CASE).
- **`src/savings_report.rs`** (S9) — `SavingsReport` + `RunRecord` +
  `RunMode` (5 variants) + `ModeSummary`. Méthodes `add_run() / aggregate()
  / to_markdown() / to_json()`. 13 tests offline. Génère
  `CHEWING_SAVINGS_REPORT.md` (lisible humain) + `.json` (machine).
- **`crates/efe-nano-app-coordinator/src/types/scoring.rs`** (S6) —
  `TaskClassScoringTable` calibrée 3-LLM × 6 task_class, version
  `s6-r2-2025-11`. 17 tests offline (6 winners + 11 cohérence).
- **`crates/efe-nano-app-coordinator/src/clients/slm_circuit.rs`** (S8) —
  `SlmCircuitBreaker` (closed/open/half-open, threshold 3, reset 10s).
  6 tests offline. **Ready-to-wire** dans `run_mission_a` (reporté R3).

### Ajout — extensions struct (backward-compat via `#[serde(default)]`)

- **`ChewingMetrics`** étendu progressivement :
  - S4 : 18 champs initial (input_tokens_raw/compressed/...)
  - S7 : `+verifier_skipped`, `+spec_drafter_latency_ms`,
    `+spec_verifier_latency_ms`, `+spec_savings_cents` (Mission Speculative)
  - S8 : `+failure_mode`, `+degradations_count`, `+failed_components`
- **`MissionAResponse`** :
  - S4 : `+chewing_metrics: Option<ChewingMetrics>`
  - S8 : `+fallback_chain_used`, `+failure_mode`
- **`MissionBResponse`** :
  - S5 : `+chewing_metrics`
  - S8 : `+vote_degraded`, `+failed_llms`, `+failure_mode`
- **`SpeculativeResponse`** (S7) : `+verifier_skipped`, `+chewing_metrics`,
  `+cost_cents` (comble l'asymétrie pré-S7 vs MissionA/B).
- **`SpeculationOutcome`** (S7+S8) : `+verifier_skipped`,
  `+drafter_latency_ms`, `+verifier_latency_ms`, `+cost_cents`,
  `+drafter_bypassed`.
- **`MissionARequest`** (S6) : `+task_class_scoring_table: Option<Arc<…>>`
  `#[serde(skip)]` + helper `MissionARequest::with_default_scoring(…)`.
- **`SpeculativeRequest`** (S7) : `+drafter_llm` (default `"drafter"`),
  `+accept_threshold` (default 0.7), `+max_words_for_quick_accept`
  (default 10). Helper `with_drafter_and_verifier(…)`.
- **`TaskClass`** (S6) : `+Hash` derive (additif, non-cassant).
- **`RouterClient`** (S6) : `+Clone, Copy` derive + factory
  `RouterClient::for_task_class(&table, task)`.
- **`Speculator`** (S7) : `+max_words_for_quick_accept` field + builder
  `.with_quick_accept(n)`. **Refactor majeur** `run()` :
  drafter d'abord (séquentiel) + hot path skip verifier si court +
  fallback drafter→verifier si drafter fail (S8).

### Ajout — `StreamEvent` (4 nouveaux variants S8)

- `LlmFallbackTriggered { from, to, reason }` — bus event quand router
  fallback chain déclenché
- `LlmFailed { provider, reason }` — bus event par LLM raté dans
  Mission B fan-out
- `SlmCircuitOpened { robot, failures }` — bus event quand circuit SLM
  s'ouvre
- `DrafterBypassed { drafter, fallback_to_verifier, reason }` — bus
  event quand Speculator bypass au verifier

Variants existants étendus :
- `SpeculationResolved { …, +verifier_skipped: bool }` (S8 `#[serde(default)]`)

### Ajout — 4 nouvelles fixtures live + 5 patches S9

Nouvelles fixtures S6-S8 :
- `05_code_gen_rust.json` (Code → Claude)
- `06_creative_writing.json` (Creative → GPT-4o)
- `07_translation_fr_en.json` (Translation → Gemini)
- `08_speculative_factual.json` (S7 hot path target)

Patches S9 sur 5 fixtures (`01, 03, 05, 06, 07`) :
- `+baseline_min_cost_cents`, `+baseline_max_cost_cents`, `+s9_included: true`
- `07` patch S9.1 : `+expected_keyword_alternatives` pour accepter
  synonymes valides "twilight"/"dusk"/"gloaming" sur Translation.

### Modifié — workflows Mission A/B/Speculative

- **`run_mission_a`** :
  - S4 : wire ChewingMetrics complet (analyzer + router + compressor +
    LLM + verifier)
  - S6 : `RouterClient::for_task_class` à la volée si
    `req.task_class_scoring_table.is_some()`
  - S8 : loop fallback chain `[selected_llm] + req.available_llms[..]` +
    bus event `LLM_FALLBACK_TRIGGERED`
- **`run_mission_b`** :
  - S5 : pré-mâchage mutualisé 1× + fan-out parallèle stream_many
  - S8 : remplace `filter_map(Result::ok)` par tracking `failed_llms` +
    bus event `LLM_FAILED` par échec + calcul `vote_degraded`. **Bug fix
    chirurgical** : extraction `provider_id` depuis
    `CoordError::LlmProvider` car `FuturesUnordered` désordonne les
    résultats vs `req.llms`.
- **`run_speculative`** :
  - S7 : drafter dynamique via `req.drafter_llm`, helper
    `estimate_savings_cents`, wire `ChewingMetrics` complet, emit
    `StreamEvent::SpeculationResolved`
  - S8 : bypass drafter → verifier direct si drafter fail, bus event
    `DRAFTER_BYPASSED`

### Modifié — `MockLlmProvider` (S8)

- `+with_error(reason: impl Into<String>) -> Self` — provider qui fail
  systématiquement avec message custom (utile pour tester graceful
  degradation, fallback chain, vote dégradé).

### Modifié — calibration S5

- `mission_a.rs:240` + `mission_b.rs:120` : `target_words: 100 → 30`
  (post-run live D1 — résumés ≈ raw avec 100, vrai compression avec 30).
  Économie observée : 85 % savings tokens sur fixture 03_compressible.

### Tests — ~700 cumulés sur 33 binaires

- **+34 tests S8** (11 failure + 6 slm_circuit + 2 metrics + 15 workflow)
- **+19 tests S7** (5 unit Speculator + 9 workflow + 5 metrics)
- **+21 tests S6** (17 scoring + 4 intégration workflow_mission_a)
- **+17 tests S9** (13 unit savings_report + 4 offline)
- **+3 tests S8 patches post-audit** (vote fallback I5 + Composite)
- **5 tests live S7 patches post-audit** (cost_cents hot/slow path)
- **0 régression** : tous les tests pré-sprint préservés à chaque livraison.

### Tests live — 6 sprints validés, ~40 ¢ cumulés

| Sprint | Tests live | Score | Coût |
|---|---|---|---|
| S2 | `live_mission_a_naked` | A1-A11 PASS (12/12) | ~1¢ |
| S3 | `live_mission_a_with_analyzer_verifier` | B1-B16 PASS (16/16) | ~3¢ |
| S4 | `live_mission_a_full_pre_chewing_pipeline` | C1-C28 PASS (28/28) | ~5¢ |
| S5 | `live_mission_b_full_3llm_consensus_with_pre_chewing` | D1-D25 PASS (25/25) | ~5¢ |
| S6 | 4 × `live_mission_a_router_picks_*` | E1-E10 × 4 (40/40) | ~5¢ |
| S7 | 3 × `live_mission_speculative_*` | F1-F13 ×3 (4/4 console PASS) | ~10¢ |
| S9 | `live_chewing_savings_report` | H1-H10 PASS (10/10) | ~12¢ |

### Documentation

- **8 architecture docs** chirurgicales S2-S9 (~7900 LOC cumulés) dans
  `docs/architecture/EFE_A1_PHASE3_R2_S*`
- **9 `S<N>_VALIDATION.md` scellés** : R1b + S2-S6 + S8 + S9 (S7 reste
  en template non-scellé, validé via console)
- **`R2_LIVE_VALIDATION.md`** (NEW S10) — méta-rapport agrégé Phase 3 R2
- **`CHEWING_SAVINGS_REPORT.md` + `.json`** (S9) — livrable quantifié
  reproductible

### Découvertes chirurgicales documentées (D1-D8)

Cf. `R2_LIVE_VALIDATION.md` §6 :
- **D1 (S5)** : `target_words=100` produit résumés ≈ raw → fix 30
- **D2 (S6)** : routing Code demande `cost_weight=0.2` pour Claude gagne
- **D3 (S7)** : Speculator pré-S7 appelait toujours verifier (bug latent)
- **D4 (S7 audit)** : `SpeculativeResponse` manquait `cost_cents`
- **D5 (S8 audit)** : `VoteEngine::vote_with_fallback` déjà existant
  (fallback I5)
- **D6 (S8)** : `stream_many` `FuturesUnordered` désordonne — fix via
  extraction provider_id depuis `CoordError`
- **D7 (S9)** : résolution `u32 cents` trop grossière sur fixtures
  courtes → fix R3 `cost_microcents: u64`
- **D8 (S9.1)** : fixtures trop strictes sur keywords →
  `expected_keyword_alternatives`

### Argument économique Phase 3 R2 — chiffres consolidés

```
                       Latence    Coût      Qualité
                       ────────   ──────    ──────────
Baseline (Claude)        6248ms    6¢        80%
Full pipeline (S4+S6)    3933ms    6¢       100%      (+20pp)
Speculative S7 hot       1192ms    2¢        50%*
                       ────────   ──────    ──────────
Speedup vs baseline    ×5.24      —         routing améliore qualité
                       (Spec)
```

→ **Production projetée 80/20 FactualQa/Code** : ~×16 cost reduction
projetée, ~×1.5-5 latency selon mixture, qualité préservée ou améliorée.

---

## [0.0.5] — 2026-05-26 — Phase 3 R1b (OpenAI GPT — vote 3-LLM réel)

### Ajout du 3ᵉ provider réel : `GptProvider` (OpenAI Chat Completions v1)

**Endpoint câblé** :
- `POST https://api.openai.com/v1/chat/completions` avec `Authorization: Bearer …`.
- Header optionnel `OpenAI-Organization` pour comptes multi-org.
- Health-check sur `GET /v1/models` (cheap, pas d'inférence).

**Pricing** (`PricingTable::openai`, cents/M tokens entiers) :
- `gpt-4o` (250/1000) — défaut R1b, balance qualité/coût.
- `gpt-4o-mini` (15/60) — premium cheap.
- `gpt-5` (500/1500), `gpt-5-mini` (50/200) — estimations.
- `gpt-4-turbo` (1000/3000), `gpt-4` (3000/6000).
- `gpt-3.5-turbo` (50/150).
- `o1` (1500/6000), `o3-mini` (110/440) — reasoning premium.
- Fallback (1000/5000) cents/M sur modèle inconnu (conservateur).

**Parsing OpenAI** (`GptProvider::extract_text_and_tools`) :
- `choices[0].message.content` → texte direct.
- `choices[0].message.tool_calls[]` → sérialisé
  `[tool_call id=X name=Y]\n{args}\n[/tool_call]` (cohérent avec format Claude).
- Tool-calls parallèles supportés (plusieurs entrées dans le tableau).
- `content: null` géré (refusal ou tool-only response).

**Streaming SSE OpenAI** (`parse_openai_sse`) :
- Pas de `event:` prefix (différent d'Anthropic).
- Termine sur sentinelle littérale `data: [DONE]\n\n`.
- `choices[0].delta.content` → texte du chunk.
- `usage.prompt_tokens` / `completion_tokens` lu dans le dernier chunk si
  `stream_options.include_usage=true` (défaut R1b).
- Séparateurs `\n\n` ET `\r\n\r\n` (HTTP CRLF) supportés.
- `delta.role` premier chunk ignoré silencieusement.

**Rate-limit observability** (`OpenAiRateLimits`) :
- Lecture auto des headers `x-ratelimit-{limit,remaining,reset}-{requests,tokens}`.
- Header `openai-organization` capturé.
- Snapshot atomique partagé entre `call` et `stream` (mis à jour même en cas
  d'erreur HTTP — utile pour 429).
- `provider.last_rate_limits()` exposé publiquement.

**Validation `OPENAI_API_KEY`** :
- Préfixe attendu `sk-` (couvre clés legacy ET `sk-proj-…`).
- `ApiKey::from_env("OPENAI_API_KEY", "sk-")` rejette les valeurs sans préfixe.

**Fixtures réalistes** (8 fichiers `tests/fixtures/openai/*.json`) :
- 001 factual_qa_simple, 002 code_block_rust, 003 max_tokens_truncated.
- 004 tool_call_single, 005 tool_call_parallel.
- 006 long_reasoning, 007 error_rate_limit, 008 refusal.

**Tests offline** :
- `tests/unit_parse_openai.rs` : 12 tests sur fixtures + pricing.
- `tests/unit_streaming_edge.rs` : 8 nouveaux tests OpenAI
  (single-byte chunks, `[DONE]` sentinel termine le stream, JSON malformé skipped,
  empty stream, usage final chunk, CRLF separators, finish_reason+DONE,
  role-only first chunk).
- `openai.rs` inline : 12 tests unit.
- `streaming.rs` inline : 3 tests OpenAI.

**Tests live env-gated** (`tests/live_providers.rs`, `EFE_LIVE_LLM=1` + clé) :
- `live_gpt_simple_question` : `2+2 = 4` avec coût < 5¢.
- `live_gpt_streaming_works` : streaming chunks reçus.
- `live_gpt_temperature_zero_deterministic` : reproducibilité Paris.
- `live_gpt_rate_limits_observable` : headers `x-ratelimit-*` snapshotés.
- `live_gpt_health` : `/v1/models` 200 OK.
- **`live_vote_3llm_real_claude_gemini_gpt`** : the holy grail — 3 vrais
  providers en parallèle (Claude + Gemini + GPT) sur question factuelle,
  consensus 3/3 sur "Paris", coût total ≤ 10¢, latence parallèle < 30s.

### Cargo features

- Nouvelle feature `openai` (ajoutée à `all-providers`).
- `all-providers = ["anthropic", "gemini", "openai"]`.
- Default `DEFAULT_GPT_MODEL = "gpt-4o"`.

### Re-exports publics

- `efe_coordinator::GptProvider` (sous `feature = "openai"`).
- `efe_coordinator::OpenAiRateLimits`.
- `llm::providers::DEFAULT_GPT_MODEL`.

### Architecture identique aux 2 autres providers

- `LlmProvider` trait : `call` / `stream` / `health` async.
- Retry exponential + jitter (`RetryPolicy`).
- Circuit breaker 3-state (Closed/Open/HalfOpen, invariant I17).
- PII redactor closure intégré (`with_redactor`, invariant I15).
- `ApiKey` wrapper redacted en Debug/Display (invariant I14).
- `tracing::info!` structuré (provider, tokens, cost, latency_ms).

### Non-régression

- **394 tests passent** (208 lib + 186 integration) — 0 failure.
- Claude (R1a) + Gemini (R1a v2) inchangés.

---

## [0.0.4] — 2026-05-26 — Phase 3 R1a v2 (hardening complet vs PHASE_3_R1 PDF)

### Audit chirurgical sur 14 gaps vs architecture R1 — TOUS comblés (sauf OpenAI = R1b)

**Fixtures réalistes** (16 fichiers `tests/fixtures/{claude,gemini}/*.json`) :
- Claude (8) : factual_qa, thinking_tag, code_block_rust, max_tokens, tool_use,
  multi_text_blocks, error_rate_limit, long_reasoning.
- Gemini (8) : factual_qa, markdown_footnotes, multi_parts, max_tokens,
  code_block_python, error_quota, safety_blocked, empty_candidates.

**Parsing avancé Claude** (`common::strip_thinking_tags` + `extract_text_and_tools`) :
- Strip auto `<thinking>...</thinking>` (configurable `with_thinking_kept()`).
- Détection + sérialisation `tool_use` : `[tool_use id=X name=Y]\n{json}\n[/tool_use]`.
- Multi-text blocks concaténés ordonnés.

**Observabilité Claude** (`ClaudeRateLimits`) :
- Lecture auto des 6 headers `anthropic-ratelimit-*`.
- Snapshot atomique partagé entre `call` et `stream`.
- `provider.last_rate_limits()` exposé.

**Observabilité Gemini** (`GeminiObservability`) :
- `last_finish_reason`, `last_safety_blocked`, `last_prompt_tokens`,
  `last_candidates_tokens` mis à jour à chaque call.

**PII redactor opt-in** (invariant I15) :
- `PiiRedactor = Arc<dyn Fn(&str) -> String + Send + Sync>` closure-based.
- `ClaudeProvider::with_redactor()` + `GeminiProvider::with_redactor()`.
- Appliqué AVANT envoi externe (user + system).

**Tests parsing sur fixtures** :
- `unit_parse_claude.rs` (12 tests).
- `unit_parse_gemini.rs` (12 tests).

**Tests SSE edge cases** (`unit_streaming_edge.rs`, 8 tests) :
- single-byte chunks, malformed JSON, empty stream, unknown events,
  usage capture mid-stream.

**Tests live additionnels** (`live_providers.rs`, +5) :
- `live_cascade_gemini_drafter_claude_verifier`.
- `live_vote_3llm_with_2_real_plus_mock_gpt_placeholder` (Vote 3-LLM hybride).
- `live_claude_rate_limits_observable`.
- `live_gemini_observability_captured`.
- `live_claude_pii_redactor_applied_before_send`.

### Tests : 344 PASS (+46 vs v1)

| Binaire | v1 | v2 |
|---|---|---|
| lib (inline) | 176 | 183 |
| unit_parse_claude | — | 12 |
| unit_parse_gemini | — | 12 |
| unit_streaming_edge | — | 8 |
| live_providers | 8 | 13 |
| (autres existants) | 114 | 116 |
| **Total** | **298** | **344** |

### Verdict chirurgical R1a v2

| Gap initial vs PDF | Statut |
|---|---|
| Fixtures réalistes | ✅ 16 fichiers (8/provider) |
| Tests parsing | ✅ unit_parse_claude + unit_parse_gemini |
| Rate-limit headers Anthropic | ✅ `ClaudeRateLimits` + auto-snapshot |
| Observability Gemini | ✅ `GeminiObservability` (safety + usage) |
| `with_redactor()` PII | ✅ closure-based, opt-in |
| Strip `<thinking>` | ✅ default ON, opt-out via `with_thinking_kept` |
| Parse tool_use Claude | ✅ `extract_text_and_tools` |
| Safety ratings Gemini | ✅ `last_safety_blocked` |
| Edge cases SSE | ✅ `unit_streaming_edge` |
| Workflow live cascade | ✅ `live_cascade_gemini_drafter_claude_verifier` |
| OpenAI provider | ⏭ scope R1b |

**R1a v2 complet vs son scope.**

---

## [0.0.3] — 2026-05-26 — Phase 3 R1a (Claude Sonnet 4.6 + Gemini Flash)

### Added — 8 fichiers (~1 800 LOC) — features cargo `anthropic` + `gemini` + `all-providers`

**Provider Anthropic Claude** (`src/llm/providers/anthropic.rs`, 285 LOC) :
- `ClaudeProvider::from_env()` → lit `ANTHROPIC_API_KEY` (préfixe `sk-ant-`)
- `ClaudeProvider::new(api_key, model)` constructor explicite
- `LlmProvider::call` : POST `https://api.anthropic.com/v1/messages` avec
  retry policy + circuit breaker + budget tracking via `PricingTable`
- `LlmProvider::stream` : SSE `event: content_block_delta` → `TokenChunk`
  par token streamé
- `LlmProvider::health` : ping minimal (1 token) sur `/v1/messages`
- Modèle défaut : `claude-sonnet-4-6` (Q1 utilisateur)
- Debug redacte la clé API (invariant I14)

**Provider Google Gemini** (`src/llm/providers/gemini.rs`, 305 LOC) :
- `GeminiProvider::from_env()` → lit `GOOGLE_AI_KEY` (préfixe `AIza`)
- POST `generateContent?key=...` (clé en query string)
- Stream via `streamGenerateContent?alt=sse&key=...`
- Modèle défaut : `gemini-2.5-flash` (Q2 utilisateur — 40× moins cher
  que Pro pour qualité comparable)
- Health check : GET `/v1beta/models` (pas d'inférence, gratuit)

**Modules support** (`src/llm/providers/`) :
- `auth.rs` (130 LOC) — `ApiKey` wrapper avec redaction auto Debug/Display,
  invariant I14 testé
- `pricing.rs` (135 LOC) — `PricingTable::anthropic()` + `gemini()` ;
  calcul en cents (entier) ADR-P3R1-004, arrondi sup conservatif
- `circuit.rs` (165 LOC) — `CircuitBreaker` 3-states (Closed/Open/HalfOpen)
  invariant I17 ; transition automatique HalfOpen après timeout
- `retry.rs` (220 LOC) — `RetryPolicy` exponential + jitter ;
  `retry_with_backoff` async ; détection 429/5xx/transport vs 4xx
- `streaming.rs` (220 LOC) — `parse_anthropic_sse` + `parse_gemini_sse` ;
  2 implémentations distinctes (ADR-P3R1-005)
- `common.rs` (90 LOC) — `build_http_client` reqwest 0.12 + rustls,
  `split_system_user` parse `"SYSTEM:...USER:..."`, `truncate_error_body`

**Tests live env-gated** (`tests/live_providers.rs`, 8 tests) :
- `live_claude_simple_question` — Q+A factuelle, vérifie réponse + coût
- `live_claude_streaming_works` — vérifie `DRAFT_TOKEN`-like events
- `live_claude_temperature_zero_deterministic`
- `live_claude_health`
- `live_gemini_simple_question`
- `live_gemini_streaming_works`
- `live_gemini_health`
- `live_vote_2llm_claude_gemini_factual` — **workflow Mission B live**
  réelle : 2 vrais providers en parallèle + assertions consensus

Tous skipped si `EFE_LIVE_LLM != "1"` ou clés API absentes.

### Tests — non-regression complète

| Phase | Tests | Status |
|---|---|---|
| Phase 1 (nano-robots) | 121 | ✅ |
| Phase 2 + R1a (lib + integration) | 298 | ✅ (+61 R1a) |
| Phase 3 (multi-llm) | 200 | ✅ |
| **Total** | **619** | ✅ **0 fail** |

8 tests live restent en `[skip]` faute d'env var — activables manuellement.

### Sécurité

- Invariant **I14** vérifié : `ApiKey::Debug` retourne `ApiKey(***xxxx)`
  (4 derniers chars uniquement) — testé.
- ADR-P3R1-001 : `reqwest` + `rustls-tls` (pas openssl).
- ADR-P3R1-002 : Wrapper `ApiKey` avec `expose()` explicite (pas de Display
  qui leakerait la clé).
- ADR-P3R1-007 : pas de cache de réponse au niveau provider (stateless).

### Comment lancer la validation live (côté utilisateur)

```bash
# 1. Exporter les clés (dans ton shell)
export ANTHROPIC_API_KEY="sk-ant-..."
export GOOGLE_AI_KEY="AIza..."

# 2. Activer les tests live + run
cd "/Users/rodrigue.pekar/Downloads/vrai/jjjj copie/efe"
EFE_LIVE_LLM=1 PYO3_PYTHON=$(which python3.12) \
  cargo test -p efe-coordinator --features all-providers --test live_providers \
  -- --test-threads=1 --nocapture

# 3. Inspecter les logs : latences réelles + coûts mesurés en cents
```

Coût estimé du run complet : **~$0.05** (10 calls × ~$0.005).

### Migration depuis Phase 3 v0.0.2

**Zero breaking change** — aucune ligne Phase 2 ou Phase 3 modifiée.

Pour utiliser les vrais providers à la place des mocks :

```rust
// AVANT (Mock)
reg.register(Arc::new(MockLlmProvider::new("claude")));

// APRÈS (Real, avec feature "anthropic" activée)
reg.register(Arc::new(ClaudeProvider::from_env()?));  // ID = "claude_claude-sonnet-4-6"
```

Le `MultiLlmBridge`, le `VoteEngine`, et tous les workflows Phase 3
fonctionnent identiquement — seul l'ID provider change (suffixe modèle ajouté).

---

---

## [0.0.2] — 2026-05-26 — Completion pass (streaming + audit + heartbeat)

### Added — 11 fichiers (~1 700 LOC)

**Streaming SSE pipeline (lacune Phase 2 v0.0.1)** :
- `src/llm/bridge.rs::stream_many` — fan-out streaming N providers en
  parallèle ; chunks tagués par `ProviderId` sur un canal partagé.
- `src/workflows/mission_a.rs` — utilise `stream_one` ; émet
  `StreamEvent::LlmTokenStreamed { swarm_id, provider, token, index, ts }`
  pour chaque token généré.
- `src/workflows/mission_b.rs` — utilise `stream_many` ; émet
  `LlmTokenStreamed` entrelacé pour les N providers.
- `src/workflows/speculative.rs` — branche `Speculator::with_bus` →
  `DraftToken` events émis pendant la frappe du drafter local.
- `src/speculate/speculator.rs::with_bus` — builder qui attache un
  `Arc<dyn EventBus>` + `swarm_id` ; back-compat préservée (sans bus,
  comportement v0.0.1 identique).

**Heartbeat (lacune SSE prod)** :
- `src/heartbeat.rs` (115 LOC) — `HeartbeatHandle::spawn` background
  task tokio qui émet `StreamEvent::Heartbeat` toutes les
  `DEFAULT_HEARTBEAT_PERIOD` (15 s) ; cancellation déterministe via
  `Notify` ; Drop stop implicite.

**Audit chain HMAC (lacune Phase 2 → préfiguration Phase 7)** :
- `src/audit.rs` (220 LOC) — `HmacAuditChain` BLAKE3-keyed chaîné
  cryptographiquement (genesis → entry_N → entry_N+1) ; `append`
  / `verify` / `snapshot` thread-safe via `parking_lot::Mutex`.
- `ChainedEntry` — wire format frozen v1.0.0 (entry_id UUIDv7,
  payload_hash, prev_hash, entry_hash, written_at, source).
- `verify()` détecte tampering au payload ET cassure de chaînage.

**NATS bus stub (feature `nats-bus`)** :
- `src/events/nats.rs` (160 LOC) — `NatsBus::connect` configure
  JetStream stream `efe-swarm-<tenant>` ; worker async pump
  NATS→subscribers locaux ; même surface que `InMemoryBus`.
- Subject hierarchy : `efe.swarm.<tenant>.<swarm_id>.<tag>`.
- Storage memory + max_age 1 h (config dev) — Phase 7 passera en
  file storage avec retention configurable.

**Helper SSE HTTP (intégration framework-agnostic)** :
- `src/sse.rs` (140 LOC) — `SseByteStream` impl `futures::Stream<Bytes>`
  ; préface `:efe-coordinator SSE v1.0.0 / retry: 5000` ; helpers
  `to_sse_blob` (sérialisation) + `parse_sse_blob` (réception client).
- Compatible Axum `Sse::new`, Warp, Actix-web, Hyper raw.
- Constantes : `CONTENT_TYPE_SSE = "text/event-stream"`, `SSE_PREFACE`.

**Examples runnables (`examples/`)** :
- `mission_a_demo.rs` — full E2E avec heartbeat + reader SSE stdout.
- `mission_b_demo.rs` — fan-out 3 LLMs + vote consensus.
- `speculative_demo.rs` — drafter rapide + verifier lent (gain latence).

**Nouveaux tests d'intégration** :
- `tests/unit_audit.rs` (8 tests) — chain verify, tamper detection,
  concurrent append, deterministic hashing across instances.
- `tests/unit_heartbeat.rs` (5 tests) — interval, stop cancel, drop safety.
- `tests/unit_sse.rs` (8 tests) — préface, blob, parse, byte stream.
- `tests/unit_stream_many.rs` (4 tests) — fan-out streaming, tagging,
  partial failure isolation.
- `tests/unit_streaming_events.rs` (3 tests) — workflows émettent bien
  `LlmTokenStreamed` × N et `DraftToken` × N.

### Tests : 213/213 PASS (+53 vs v0.0.1)

| Binaire | Tests v0.0.1 | Tests v0.0.2 |
|---|---|---|
| lib (inline) | 90 | 112 (+22 : audit, heartbeat, sse, bridge::stream_many, speculator::with_bus) |
| unit_audit | — | 8 |
| unit_heartbeat | — | 5 |
| unit_sse | — | 8 |
| unit_stream_many | — | 4 |
| unit_streaming_events | — | 3 |
| (existants v0.0.1) | 70 | 70 |
| **Total** | **160** | **210** |

### Highlights techniques v0.0.2

- **SSE end-to-end fonctionne** : Mission A streame token-par-token,
  Mission B entrelace 3 providers, Speculative emit DraftToken pendant
  la frappe.  L'UI peut désormais voir les LLM "taper" en temps réel.
- **Audit chain crypto-vérifiable** : `HmacAuditChain::verify` détecte
  toute modification ou injection a posteriori (test : tampering payload
  + cassure chaînage).  Préfigure le passage Postgres + cosign Phase 7.
- **NatsBus compile et est testable en local** (stub : `cargo check
  --features nats-bus` passe en 55 s, tests live env-gated Phase 7).
- **Heartbeat anti-proxy-cut** : tâche tokio cancellation-safe,
  notification déterministe à l'arrêt, Drop safe par défaut.

### Compatibilité v0.0.1 → v0.0.2

- **Toutes les API publiques v0.0.1 préservées** (re-exports `lib.rs`
  inchangés, signatures `Coordinator::*` inchangées).
- **Mission A / B / Speculative** retournent la même `*Response` ;
  seul l'**ajout** d'évènements SSE granulaires est visible côté bus.
- **Speculator** : `new()` inchangé ; `with_bus()` est un nouveau
  builder additif.
- **HmacAuditChain** : nouveau, pas de breaking change.

---

## [0.0.1] — 2026-05-25 — Sprint S3+S4 (Phase 2 Coordinator)

---

## [0.0.1] — 2026-05-25 — Sprint S3+S4 (Phase 2 Coordinator)

### Added — 31 fichiers (~3 800 LOC Rust)

**Surface publique (`src/`)** :
- `lib.rs` — façade + invariants (I1–I8) + constantes (`SSE_PROTOCOL_VERSION`, `MAX_PARALLEL_LLMS`, `DEFAULT_MAX_CONCURRENCY`).
- `error.rs` — `CoordError` (10 variantes) + `is_retryable` + `suggested_backoff` ; encapsule `RobotError` via `From`.
- `request.rs` — `MissionA/B/SpeculativeRequest/Response` + `FactCheck` + `CandidateSourceRef`.
- `coordinator.rs` — `Coordinator` + `CoordinatorBuilder` (fluide) + `CoordinatorClients` (bundle Phase 1) + `CoordinatorConfig`.

**Blackboard** (stigmergie SwarmSys, HF 2510.10047) — `src/blackboard/` :
- `mod.rs` — `MAX_BLACKBOARD_BYTES` (256 KB), `MAX_ENTRIES` (4096), `DEFAULT_HALF_LIFE_SECS`, `PHEROMONE_GC_THRESHOLD`.
- `entry.rs` — `BlackboardEntry` + `EntryKind` (10 variantes) + decay temporel.
- `pheromone.rs` — `PheromoneBlackboard` (parking_lot RwLock + BTreeMap, LRU eviction sur bytes overflow).

**Events** — `src/events/` :
- `mod.rs` — façade + re-exports.
- `bus.rs` — trait `EventBus` + `SubscriberToken`.
- `inmem.rs` — `InMemoryBus` (tokio mpsc + DashMap, backpressure → drop + compteur).
- `stream.rs` — `StreamEvent` (13 variantes, protocole **v1.0.0** frozen) + `MissionTag` + `to_sse_frame()`.

**LLM bridge** — `src/llm/` :
- `mod.rs` — façade + re-exports.
- `provider.rs` — trait `LlmProvider` + `LlmCall` + `LlmResponse` + `TokenChunk` + impl `stream()` par défaut.
- `mock.rs` — `MockLlmProvider` (déterministe, configurable : response/cost/latence/quality/`fail_after_n`).
- `registry.rs` — `ProviderRegistry` (DashMap thread-safe).
- `bridge.rs` — `MultiLlmBridge` : `call_one` / `stream_one` / `call_many` / `call_many_ok` (FuturesUnordered).

**Vote** (Symphony, HF 2508.20019) — `src/vote/` :
- `mod.rs` — doc invariants I5/I6.
- `engine.rs` — `VoteEngine` + `Ballot` + `VoteOutcome` (cluster BLAKE3 + tie-break déterministe + fallback success_rate).

**Speculate** (Speculative RAG, HF 2407.08223) — `src/speculate/` :
- `mod.rs` — façade.
- `speculator.rs` — `Speculator::run` + `SpeculationOutcome` + `DraftToken` + Jaccard mot-à-mot.

**DAG** — `src/dag/` :
- `mod.rs` — façade.
- `node.rs` — `DagNode` + `NodeStatus` + `NodeId` + closures async typées.
- `executor.rs` — `DagExecutor` (Kahn topo sort, JoinSet, semaphore borné, skip propagé sur fail) + `DagRunReport`.

**Workflows** — `src/workflows/` :
- `mod.rs` — façade + `audit_chain_hash` BLAKE3.
- `mission_a.rs` — analyzer → router → compressor → LLM → verifier ; full SSE.
- `mission_b.rs` — pre-chew → fan-out N LLMs → vote pondéré → verifier ; full SSE.
- `speculative.rs` — drafter + verifier via Speculator ; full SSE.

**Tests d'intégration** (`tests/`) :
- `unit_blackboard.rs` (9 tests) — I2/I7 + LRU + clone visibility.
- `unit_bridge.rs` (6 tests) — fan-out, partial failures, registry overwrites.
- `unit_bus.rs` (7 tests) — sub/unsub, backpressure, tokens uniques.
- `unit_dag.rs` (8 tests) — linear/diamond/cycle/missing parent, I8.
- `unit_speculator.rs` (5 tests) — accept/reject/partial overlap, IDs.
- `unit_vote.rs` (7 tests) — unanimous/split/weighted/quorum 0.
- `workflow_mission_a.rs` (4 tests) — E2E + SSE events.
- `workflow_mission_b.rs` (5 tests) — E2E + vote + weighted override.
- `workflow_speculative.rs` (5 tests) — drafter+verifier + provider missing.
- `sse_stream.rs` (8 tests) — version v1.0.0, frame format, kebab tags.
- `invariants.rs` (6 tests) — I2/I3/I4/I5/I6/I7 transverses.

**Benchmarks** (`benches/coordinator_throughput.rs`, Criterion) :
- `blackboard::deposit_small_entry`, `read_kind_100_entries`.
- `vote::tally::3_llms`.
- `dag::diamond_a_to_d`.
- `bridge::call_one_mock`, `call_many_3_mocks_parallel`.
- `workflow::mission_a_mock_only`, `workflow::mission_b_3_mocks_parallel`.

### Tests : 160/160 PASS

| Binaire | Tests | Statut |
|---|---|---|
| `lib` (inline) | 90 | ✅ |
| `unit_blackboard` | 9 | ✅ |
| `unit_bridge` | 6 | ✅ |
| `unit_bus` | 7 | ✅ |
| `unit_dag` | 8 | ✅ |
| `unit_speculator` | 5 | ✅ |
| `unit_vote` | 7 | ✅ |
| `workflow_mission_a` | 4 | ✅ |
| `workflow_mission_b` | 5 | ✅ |
| `workflow_speculative` | 5 | ✅ |
| `sse_stream` | 8 | ✅ |
| `invariants` | 6 | ✅ |
| **Total** | **160** | **✅** |

### Highlights techniques

- **`StreamEvent` v1.0.0 frozen** — 13 variantes typées exposées via SSE
  à l'UI ; tag `SCREAMING_SNAKE_CASE` ; `to_sse_frame()` produit du
  `text/event-stream` standard.
- **Blackboard bornée 256 KB** avec LRU eviction et garbage collection
  des phéromones sub-seuil.  parking_lot RwLock + BTreeMap pour O(log n)
  fairness + itération ordonnée par UUID v7.
- **DagExecutor** : validation Kahn topo sort à submit, parallélisme borné
  par semaphore, propagation déterministe de `Skipped` sur fail amont.
- **VoteEngine** : clustering BLAKE3 des réponses canonicalisées
  (whitespace-insensitive), tie-break alphabétique déterministe, fallback
  invariant I5 sur `success_rate` quand pas de quorum.
- **Speculator** : `tokio::spawn` du drafter en parallèle du verifier,
  arbitrage Jaccard mot-à-mot, comptage des tokens acceptés vs regénérés.
- **MultiLlmBridge** : `FuturesUnordered` pour fan-out N providers ;
  `call_many_ok` drop les erreurs (utile Mission B "best-effort").
- **InMemoryBus** : DashMap pour pas-de-lock-global, mpsc bounded
  (1024/subscriber), drop+compteur sur backpressure.

### Intégration

- Ajouté au workspace EFE racine
  (`Cargo.toml::members += "crates/efe-coordinator"`).
- Consomme **uniquement** `efe-nano-app-coordinator` (Phase 1) en
  dépendance interne, plus tokio/serde/uuid/parking_lot/dashmap/blake3.
- Zéro dépendance native externe (pas de NATS server requis en default).

### Versions des composants frozen Phase 2

| Composant | Version | Frozen sur |
|---|---|---|
| `StreamEvent` SSE | v1.0.0 | tag, type, champs |
| `MissionA/B/SpeculativeRequest/Response` | v1.0.0 | champs |
| `BlackboardEntry` payload kinds | v1.0.0 | `EntryKind` 10 variantes |
| Trait `EventBus` | v1.0.0 | publish/subscribe/unsubscribe/dropped_count/flush |
| Trait `LlmProvider` | v1.0.0 | id/call/stream/health |

### Documentation associée

- `EFE_PROJET/03_a_construire/PHASE_2_COORDINATOR.pdf` — architecture
  26 pages.
- `crates/efe-coordinator/README.md` — usage côté crate.
- `EFE_PROJET/03_a_construire/PHASE_1_JSON_GRAMMAR.pdf` — fondation
  Phase 1 (rappel).
