# EFE — Phase 5 : Productisation — `swarm.run(task, topology, budget)`

> **Statut** : Architecture (gelée pour build) · **Date** : 2026-06-09 · **Durée** : ≈4-6 semaines · **Maturité** : 30 % → 80 %
> **Thème** : transformer le swarm (Phases 1-4) en **produit** : un SDK/API unifié, un dashboard live, des modes self-host/keyless, une gouvernance de budget — pour qu'un **dev externe** lance un essaim **vérifiable, plafonné en coût, et re-prouvable** en quelques minutes.
> **Crates cibles (neufs)** : `crates/efe-swarm-sdk` · `crates/efe-swarm-gateway` (ou extension d'`efe-server`)
> **Gate** : ne productiser qu'**après H-SWARM GO** — il l'est (ADR-SWARM-001, **GO conditionnel**).

---

## 0. Thèse — pourquoi *maintenant*

Les Phases 1-4 ont construit et **mesuré en réel** un essaim qui :
- bat le monolithe sur le **front coût/qualité** des tâches **vérifiables** (H-SWARM : or7 = +6 pts à **3,2× moins cher**) ;
- crack le **non-vérifiable** via consensus N-juges (S3 : juge réel **v=1,0** sur gemma3:12b) ;
- est **intégralement tracé et re-prouvable** par un tiers (Phase 4 : `verify_swarm`, overhead **0,0035 %** en réel).

Mais tout cela vit dans des **exemples Rust** et des traits internes. **Personne d'externe ne peut l'utiliser.** Phase 5 emballe ces substrats prouvés en un **produit** : une fonction, une API, un dashboard, une facture. Le différenciateur produit n'est pas « un autre orchestrateur d'agents » — c'est **le seul qui rend chaque run indépendamment vérifiable et plafonné en coût**, en self-host keyless.

> **Le slogan produit** : *`swarm.run(task, topology, budget)` → une réponse, son coût exact, et une preuve que personne ne peut falsifier.*

---

## 1. Le réel — l'inventaire (ce qu'on **réutilise**, sans réinventer)

### 1.1 Le client LLM (keyless / self-host / réseau) — `efe-nano::runtime::llm_client`
```rust
#[async_trait]
pub trait LlmClient: Send + Sync + 'static {
    async fn call(&self, request: LlmCallRequest) -> Result<LlmCallResponse, LlmCallError>;
    fn estimate_cost_cents(&self, request: &LlmCallRequest) -> f64;   // ← gouvernance budget
}
```
- **`BuiltinLlmClient::new()`** — **keyless**, déterministe (5 templates `m50.*`). Le mode **démo/dev**.
- **Self-host** — `llama-server`/Ollama local (le seam réseau déjà démontré : `swarm_all_phases_real.rs` appelle Ollama en local, **sans clé**).
- **`LlmRouterClient`** — réseau (Anthropic/OpenAI/Vertex via M21). **Les clés sont gérées par le routeur, server-side** — jamais par le SDK ni l'utilisateur.

### 1.2 La gouvernance de budget — `efe-multi-llm::budget::BudgetTracker`
```rust
pub fn new(max_cost_cents: u32) -> Self;
pub fn try_charge(&self, llm: &str, cents: u32) -> MultiLlmResult<u32>;  // ATOMIQUE, revert si dépasse (I13)
pub fn remaining(&self) -> u32;
pub fn per_llm_breakdown(&self) -> HashMap<String, u32>;
pub fn snapshot(&self) -> BudgetSnapshot;   // sérialisable (audit/observabilité)
```
**Invariant I13** : `try_charge` ne **dépasse jamais** le hard-cap, même en concurrence. C'est le cœur de `budget` dans `swarm.run`.

### 1.3 Le serveur HTTP — `efe-server` (**Axum 0.7** + Tonic) + `efe-meta-swarm-server`
- `efe-server` expose déjà `/v1/*` : runs d'agents, **`/v1/budgets`** (CRUD), `/v1/usage/*`, audit-export, et le **streaming SSE** `/runs/:id/steps/stream` (`async-stream`). Stack : `axum`, `tower`, `prometheus`, `redis`, `async-nats`.
- `efe-meta-swarm-server` expose `/metrics` (Prometheus) + `/healthz` (Hyper).
- **Manque** : aucun dashboard, aucun WebSocket, pas de route `swarm.run`. ⇒ on **étend** la stack axum existante.

### 1.4 Les couches à emballer — Phases 2/3/4
- **Topologie** : `Topology::run(&self, task: &Value, ctx: &SwarmContext) -> TopologyOutput` ; `SwarmContext{ fleet, write_sink }` ; `TopologyOutput{ output, agents_used, cost_units, elapsed_ms, quality }` ; `AgentFleet` (lookup **par nom**) ; adaptateurs `GvTopology`/`PipelineTopology`/`HierTopology`/`ConsensusTopology`.
- **Provenance** : `ProvenanceRecorder` + `ProvenanceSink` (→ `ctx.with_write_sink`) + `ProvenancedAgent` + `verify_swarm(records, key) -> SwarmProvenanceReport{ ok(), root, traced_pct, ... }` + `ProvenanceMetrics` (Prometheus sans-dép).

### 1.5 Le gate H-SWARM — ADR-SWARM-001 (**GO conditionnel**)
- **GO** : `or7` ≈ 0,91 (vs monolithe 0,851) à **3,2× moins cher** — *robuste ±σ*.
- **Conditionnel** : gains récupérables **où c'est vérifiable** (code qui compile, math, extraction validée par schéma, QA citée). Le **vote naïf échoue** ; l'**agrégateur est le moat** (Phase 3).
- ⇒ **Phase 5 est gated-GO**, mais le SDK doit **orienter par défaut vers la zone vérifiable** (où le moat est réel).

---

## 2. Le design — `efe-swarm-sdk` : l'API unifiée

### 2.1 L'entrée unique
```rust
// crates/efe-swarm-sdk/src/lib.rs
pub struct Swarm { /* fleet + client factory + key source + métriques */ }

impl Swarm {
    pub fn builder() -> SwarmBuilder;                 // config : mode client, tenant, provenance on/off
    pub async fn run(&self, task: Value, topology: TopologySpec, budget: Budget) -> SwarmRun;
}
```
- **`task: Value`** — l'entrée libre (texte, doc, question…).
- **`topology: TopologySpec`** — quel pattern (T1/T2/T3/Consensus) + ses agents (par nom depuis la flotte).
- **`budget: Budget`** — plafond de coût (cents) + politique de dépassement.
- **`SwarmRun`** — résultat **complet** : sortie, comptabilité, provenance (root + verdict), budget consommé, qualité.

### 2.2 `TopologySpec` — sélection ergonomique (sans connaître les internals)
```rust
pub enum TopologySpec {
    /// T1 — Generate-Verify : K générateurs + vérificateur a-posteriori (tâches VÉRIFIABLES).
    GenerateVerify { generator: String, verifier: String, k: usize },
    /// T2 — Pipeline : étages nommés chaînés (décompose une tâche dure).
    Pipeline { stages: Vec<String> },
    /// T3 — Hiérarchique : décompose → résout → fusionne.
    Hierarchical { decomposer: String, solver: String, merger: String },
    /// T-Consensus : K générateurs + panel de juges → Vote/Borda/Condorcet (non-vérifiable).
    Consensus { generator: String, judges: Vec<String>, k: usize, pattern: Pattern, escalate: bool },
    /// Auto : choisit T1 si la tâche est vérifiable (verifier fourni), sinon Consensus (§2.6).
    Auto,
}
```
Le SDK **assemble depuis la flotte par nom** (§3 de Phase 2) et **enveloppe chaque agent** en `ProvenancedAgent` (§2.5). `TopologySpec` → `Box<dyn Topology>` + `SwarmContext`.

### 2.3 `ClientMode` — keyless / self-host / réseau (le pattern `build_client`)
```rust
pub enum ClientMode {
    Keyless,                              // BuiltinLlmClient — démo/dev, déterministe, 0 dép réseau
    SelfHost { endpoint: String, model: String },  // llama-server / Ollama LOCAL (sans clé)
    Network  { router: String },          // M21 router — clés SERVER-SIDE, jamais ici
}
pub fn build_client(mode: &ClientMode) -> Arc<dyn LlmClient>;   // factory (réutilise le pattern existant)
```
> **Sécurité (règle absolue)** : le SDK **ne saisit, ne lit, ne stocke aucune clé**. `Keyless`/`SelfHost` sont keyless par construction ; `Network` délègue au routeur server-side (env/Vault). C'est le pattern `build_client` étendu, pas un nouveau chemin de secret.

### 2.4 `Budget` — gouvernance (réutilise `BudgetTracker`)
```rust
pub struct Budget {
    pub max_cents: u32,
    pub on_exceed: OnExceed,   // Stop (renvoie partiel) | Degrade (baisse K / saute juges) | Error
}
```
- Chaque appel d'agent : `client.estimate_cost_cents(req)` → `tracker.try_charge(model, cents)`.
- Si `BudgetExceeded` : selon `on_exceed`, on **arrête proprement** (résultat partiel + `BudgetSnapshot`), on **dégrade** (réduit K / saute la phase juges), ou on **erreur**. **Jamais de dépassement** (I13).
- `SwarmRun.budget = tracker.snapshot()` (dépense exacte, ventilation par modèle).

### 2.5 Provenance **auto-câblée** (le différenciateur produit)
À chaque `run` (si `provenance: true`, défaut) :
1. `recorder = ProvenanceRecorder::new(swarm_id, tenant)` ; `sink = ProvenanceSink::new(recorder)`.
2. `ctx = SwarmContext::standard().with_write_sink(sink)` ; agents enveloppés en `ProvenancedAgent`.
3. après `topo.run` : `sealed = recorder.seal(&key_source)` ; `report = verify_swarm(&sealed.records, &key)`.
4. `SwarmRun.provenance = { root: report.root_hex(), verified: report.ok(), traced_pct, records }`.
> **Chaque run produit un `provenance_root` re-vérifiable** par un tiers avec la seule clé tenant. C'est l'argument de vente : *« voici la preuve, pas notre parole »*.

### 2.6 `SwarmRun` — le résultat complet
```rust
pub struct SwarmRun {
    pub output: Value,                    // la réponse
    pub accounting: Accounting,           // agents_used, cost_units, elapsed_ms (de TopologyOutput)
    pub budget: BudgetSnapshot,           // dépense exacte (cents) + ventilation
    pub provenance: ProvenanceSummary,    // root, verified, traced_pct, records
    pub quality: QualitySignals,          // candidates, verified, subtasks
    pub topology: String,                 // "T-consensus", "T1:generate-verify"…
}
```
**`TopologySpec::Auto`** : si un `verifier` déterministe est résoluble (tâche vérifiable) → T1/Consensus avec vérif ; sinon Consensus N-juges. Reflète H-SWARM (le moat est dans la zone vérifiable).

---

## 3. Le design — l'API HTTP (`efe-swarm-gateway`, sur la stack axum d'`efe-server`)

```
POST /v1/swarm/run            → exécute swarm.run, renvoie SwarmRun (JSON)
GET  /v1/swarm/runs/:id/events→ SSE : flux live (trafic inter-agent, consensus, provenance) — §4
GET  /v1/swarm/runs/:id       → état + résultat d'un run
GET  /v1/swarm/verify/:id     → re-vérifie la provenance d'un run (verify_swarm) → {ok, root}
GET  /v1/budgets …            → RÉUTILISE les routes budget existantes d'efe-server
GET  /metrics                 → Prometheus (SubstrateMetrics + ProvenanceMetrics) — §4
GET  /                        → dashboard statique (§4)
```
- **Réutilise** : le routeur axum, l'auth tenant, le SSE (`async-stream`), les routes `/v1/budgets` d'`efe-server`.
- `POST /v1/swarm/run` : body `{ task, topology, budget, mode }` → `Swarm::run` → `SwarmRun`. Idempotence + tenant scoping comme les routes existantes.

---

## 4. Le design — **Dashboard live** (trafic inter-agent · consensus · provenance)

### 4.1 Le seam d'événements (réutilise le substrat)
Le dashboard a besoin d'un **flux live**. On le câble sur les seams Phase 1/4 existants, **sans toucher** les topologies :
- **Trafic inter-agent** : un `DashboardSink: WriteSink` (jumeau de `ProvenanceSink`, pattern `ChannelSink` non bloquant) + un abonné `EventBus`/`TopicRouter` → chaque écriture Blackboard + chaque publication de topic devient un événement SSE.
- **Consensus** : les signaux de `ConsensusTopology` (pattern choisi, accord, escalade, ballots) → événements.
- **Provenance** : `ProvenanceMetrics` (tracé, records, ruptures) + le `provenance_root` final + `verify_swarm.ok()`.
- **Budget** : `BudgetTracker.snapshot()` streamé (cents consommés en temps réel).

```rust
// efe-swarm-gateway : multiplexe les seams vers un canal SSE par run.
pub struct DashboardBus { tx: broadcast::Sender<DashboardEvent> }
pub enum DashboardEvent {
    AgentWrite { agent: String, pointer: String, version: u64 },
    TopicPublish { topic: String, by: String },
    Ballot { judge: String, pattern: String },
    Provenance { records: usize, traced_pct: f64, root: Option<String> },
    Budget { spent_cents: u32, remaining: u32 },
    Done { output: Value, verified: bool },
}
```

### 4.2 L'UI (minimale, dépendance-légère)
- **Page statique** servie par axum (`GET /`) — HTML + JS vanilla (ou un petit bundle), **pas de toolchain lourde** imposée. Consomme l'SSE.
- **4 panneaux** :
  1. **Trafic inter-agent** — graphe live des écritures `/cand/*`, `/ballot/*`, `/out/*` + publications de topics (le « bus=indice, blackboard=vérité » rendu visible).
  2. **Consensus** — candidats, ballots des juges, pattern (Vote/Borda/Condorcet), escalade, gagnant.
  3. **Provenance** — compteur de records, `traced_pct`, **`provenance_root`** + badge `verify_swarm: ✓`.
  4. **Budget** — jauge cents consommés / plafond, ventilation par modèle.
> Le dashboard **montre le produit** : on *voit* les petits agents passer l'info en parallèle, le consensus émerger, et la preuve se sceller — en direct.

---

## 5. Modes self-host / keyless (l'histoire de déploiement)
| mode | client | clés | usage |
|---|---|---|---|
| **Keyless** | `BuiltinLlmClient` | aucune | démo, CI, quickstart (TTFS) |
| **Self-host** | llama-server / Ollama local | **aucune** | prod souveraine (on-prem, air-gapped), le défaut « privacy » |
| **Network** | M21 router | **server-side** (env/Vault) | accès aux modèles frontières, clés jamais exposées |

Le **self-host keyless** est le **mode phare** : un essaim souverain, local, vérifiable — *exactement* ce que les providers ne peuvent pas offrir (cf. le moat Phase 4). Le SDK détecte le mode via config/env ; **le binaire ne contient aucun secret**.

---

## 6. Métriques & SLO (le tableau de bord du produit)
| métrique | cible | mesure |
|---|---|---|
| **time-to-first-swarm (TTFS)** | **< 15 min** | d'un dev externe : `git clone`/`cargo add` → premier `swarm.run` qui renvoie une sortie vérifiée (quickstart chronométré) |
| **démo bout-en-bout** | **passe** | dashboard live + `swarm.run` réel (self-host Ollama) sur tâche vérifiable → sortie correcte + `provenance_root` + `verify_swarm: ✓` |
| latence p50 du gateway (hors LLM) | < 5 ms | overhead SDK+HTTP+provenance vs latence agents |
| budget : 0 dépassement | **garanti** | I13 (`BudgetTracker`) — test de concurrence |

---

## 7. Le gate — **Lot P5** (avant de déclarer le produit)
**Pré-condition** : **H-SWARM GO** (✓, ADR-SWARM-001). On ne productise pas un essaim qui ne bat pas le monolithe — il le bat (zone vérifiable).
**Question** : *un dev externe atteint-il un `swarm.run` vérifié en < 15 min, et la démo e2e tourne-t-elle ?*
- **Harnais** : un **quickstart** (README + exemple) chronométré (TTFS) ; la **démo** (gateway + dashboard + self-host Ollama) sur 3 tâches vérifiables.
- **GO** : TTFS < 15 min **ET** démo e2e verte (sortie correcte + provenance vérifiée + budget respecté).
- **NO-GO** : friction d'onboarding > seuil → simplifier l'API/quickstart avant d'élargir.

---

## 8. Sécurité & clés (règle absolue, rappelée)
- Le SDK/gateway **ne manipule aucune clé**. Keyless/Self-host sont keyless ; Network délègue au routeur **server-side** (env `EFE_*`/Vault, jamais dans le binaire ni l'API).
- La provenance utilise la clé **HKDF par tenant** (server-side, Phase 4) — le dashboard affiche le `root`, **jamais** la clé.
- Tenant scoping + auth réutilisés d'`efe-server`.

---

## 9. Phasage (≈4-6 semaines · 30 %→80 %)
- **Inc.1 — SDK cœur (sem. 1-2)** : `efe-swarm-sdk` — `Swarm::run(task, topology, budget)` in-process, modes **Keyless + Self-host**, provenance auto-câblée, budget via `BudgetTracker`. `TopologySpec` (T1/T2/T3/Consensus/Auto). **Quickstart** + exemple. *SLO : TTFS mesuré.*
- **Inc.2 — Gateway HTTP (sem. 2-3)** : `POST /v1/swarm/run` (sync) sur la stack axum d'`efe-server` ; `/v1/swarm/verify/:id` ; réutilise `/v1/budgets` ; `/metrics` (Substrate+Provenance).
- **Inc.3 — Dashboard live (sem. 3-5)** : `DashboardBus` + seams (`DashboardSink`, abonné EventBus, ProvenanceMetrics) → **SSE** `/v1/swarm/runs/:id/events` ; UI statique 4 panneaux. *Le trafic inter-agent rendu visible.*
- **Inc.4 — Productisation & gate (sem. 5-6)** : mode **Network** (router server-side) ; gouvernance budget (dégradation gracieuse) ; **démo e2e** (self-host Ollama) ; **Lot P5** (TTFS + démo) → ADR-SWARM-008 (productisation GO).

---

## 10. Esquisse d'API publique (gelée pour Inc.1-4)
```rust
// efe-swarm-sdk
pub use sdk::{Swarm, SwarmBuilder, SwarmRun, Accounting, ProvenanceSummary};
pub use spec::{TopologySpec, Budget, OnExceed};
pub use client::{ClientMode, build_client};
// réexports pratiques
pub use efe_swarm_topology::Pattern;
pub use efe_multi_llm::budget::BudgetSnapshot;
```
- **Dépendances** : `efe-swarm-topology`, `efe-swarm-provenance`, `efe-nano` (LlmClient/BuiltinLlmClient), `efe-multi-llm` (BudgetTracker), `efe-audit-core` (HKDF). Gateway : + `axum`/`tower`/`async-stream` (déjà chez `efe-server`).
- **Pas de cycle** : le SDK dépend des couches basses ; le gateway dépend du SDK.

---

## 11. Arborescence cible
```
crates/efe-swarm-sdk/
  src/{lib, sdk, spec, client, budget, provenance, result}.rs
  examples/{quickstart.rs, run_consensus.rs, run_selfhost_ollama.rs}
crates/efe-swarm-gateway/         # ou module dans efe-server/src/v1/swarm/
  src/{lib, routes, sse, dashboard}.rs
  assets/index.html               # UI statique (4 panneaux, SSE)
docs/docs/adr/ADR-SWARM-008-productization-gated-on-h-swarm.md
scripts/swarm/EFE_SWARM_P5_<date>.{json,md}   # TTFS + démo e2e
```

---

## 12. Risques & parades
| risque | parade |
|---|---|
| Onboarding trop friction (TTFS > 15 min) | quickstart **keyless** par défaut (0 dép réseau), un seul appel `Swarm::run` |
| H-SWARM GO **conditionnel** sur-vendu | `TopologySpec::Auto` oriente vers la zone vérifiable ; doc honnête (le moat = tâches vérifiables) |
| Dashboard = toolchain front lourde | UI **vanilla** statique servie par axum, dépendance-légère ; pas de build JS imposé |
| Budget contourné en concurrence | `BudgetTracker` I13 (atomique, testé concurrence) — **jamais** de dépassement |
| Fuite de clé via le produit | keyless/self-host par défaut ; Network délègue server-side ; binaire sans secret |
| Deux serveurs (hyper meta + axum efe-server) | le gateway swarm vit dans la stack **axum** (`efe-server`) ; `/metrics` unifié |

---

## 13. Définition de « fait » (Phase 5 à 80 %)
1. `efe-swarm-sdk` : `Swarm::run(task, topology, budget)` — modes Keyless+Self-host+Network, provenance auto, budget plafonné. ✅ tests + quickstart.
2. Gateway : `POST /v1/swarm/run` + `/v1/swarm/verify/:id` + `/metrics` sur axum ; routes budget réutilisées.
3. Dashboard live : SSE 4 panneaux (trafic inter-agent, consensus, provenance, budget) — **on voit l'essaim travailler**.
4. **Lot P5** : TTFS d'un dev externe **< 15 min** + **démo e2e** verte (self-host, sortie vérifiée, `provenance_root`, budget respecté) → **ADR-SWARM-008** GO.
5. Modes self-host/keyless documentés ; **aucune clé** dans le produit.
6. Tout **dans le dossier**, 0 warning, gated sur **H-SWARM GO**.

> **Le livrable produit** : *un dev fait `swarm.run("classe ce doc", Consensus, budget=50¢)` — et reçoit la réponse, son coût exact (≤ 50¢), et un hash que son régulateur peut vérifier sans nous croire. En self-host, sans clé. C'est ça, productiser le moat.*
