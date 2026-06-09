# EFE — Phase 5 Inc.3 : Dashboard **live** — `DashboardBus` sur les seams substrat → SSE + UI vanilla 4 panneaux

> **Statut** : Architecture (gelée pour build) · **Date** : 2026-06-09 · **Phase** : 5 (Productisation) · **Inc.** : 3 (dashboard live)
> **Sur** : l'**axum 0.7 existant** (`efe-swarm-gateway` Inc.2 · pattern SSE `efe-preview-server`/`efe-server`)
> **Pré-requis** : Inc.1 (`efe-swarm-sdk`) · Inc.2 (`efe-swarm-gateway` : run + verify + `/metrics`, `SubstrateMetrics` câblé bout-en-bout).

---

## 0. Thèse — **voir** les petits agents se passer l'info, en parallèle, en temps réel

EFE, c'est *« des agents à très peu de paramètres, en grande quantité, qui se passent des
informations en parallèle »*. Aujourd'hui, c'est **vrai mais invisible** : le run finit en
millisecondes (keyless) ou en secondes (self-host), et tout le trafic inter-agent (écritures
`/cand/*`, ballots `/ballot/*`, messages de bus, scellement de provenance, brûlure de budget)
disparaît dans un résumé `SwarmRun` figé.

**Inc.3 rend l'essaim VISIBLE** : un `DashboardBus` se branche sur les **seams substrat déjà
existants** (le `WriteSink` uniforme de Phase 4, l'`EventBus`/`TopicRouter` broadcast, les registres
`ProvenanceMetrics`/`SubstrateMetrics`) et **diffuse** chaque micro-événement en **SSE** vers une
**UI vanilla** (zéro framework) à **4 panneaux** :

1. **Trafic inter-agent** — les agents (nœuds) et leurs messages (arêtes animées) : on *voit* l'info
   passer en parallèle (writes stigmergiques + events de bus).
2. **Consensus** — candidats qui arrivent, ballots des juges, pattern choisi (Vote/Borda/Condorcet),
   gagnant + confiance.
3. **Provenance** — records scellés, `traced_pct`, `provenance_root`, statut `verified`, ruptures.
4. **Budget** — dépense vs plafond, ventilation par agent, épuisement (politique).

> **L'argument produit** : *« regardez l'essaim penser — chaque agent, chaque message, chaque preuve,
> en direct — puis re-vérifiez la preuve vous-même (Inc.2). »* La transparence **opérationnelle**
> (live) **plus** la transparence **cryptographique** (le moat).

**Principe directeur (la discipline EFE)** : le dashboard est un **observateur**, pas un acteur. Il
**tape des seams existants sans réécrire un seul agent ni une seule topologie** — exactement comme
la provenance (Phase 4, 2 seams) et les métriques (Inc.2). Le flux live est **best-effort/lossy** ;
la **vérité durable** reste la chaîne de provenance (`verify_swarm`). On ne confond jamais les deux.

---

## 1. Le réel — ce qu'on réutilise (les seams sont **déjà là**)

### 1.1 Le seam `WriteSink` (Phase 4, uniforme) — prior art `ChannelSink`
```rust
// efe-blackboard/src/sink.rs
pub trait WriteSink: Send + Sync + 'static { fn on_write(&self, write: &BlackboardWrite); }
// BlackboardWrite { pointer: String, version: u64, agent_id: AgentId, at: SystemTime, bytes: usize }  (serde)

// DÉJÀ EXISTANT : un WriteSink → canal mpsc, lossy avec compteur de drop. Le patron exact du tap.
pub struct ChannelSink { tx: mpsc::Sender<BlackboardWrite>, dropped: AtomicU64 }
impl WriteSink for ChannelSink { fn on_write(&self, w) { if tx.try_send(w.clone()).is_err() { dropped += 1 } } }
```
- Le `SwarmContext` branche **un** `write_sink` (Phase 4) → toutes les topologies (T2/T-Consensus)
  ancrent leurs écritures `/cand/*`, `/ballot/*`, `/out/*` dessus. **On tee** ce seam : un
  `TeeSink([provenance_sink, dashboard_sink])` nourrit la provenance **et** le dashboard sans choix.

### 1.2 Le seam `EventBus`/`TopicRouter` — **broadcast multi-souscripteurs natif**
```rust
// efe-blackboard/src/eventbus.rs / topic_router.rs
pub struct SwarmEvent { swarm_id: SwarmId, published_by: AgentId, topic: String, payload: Value, published_at: SystemTime } // serde
pub struct EventBus { tx: broadcast::Sender<SwarmEvent>, … }          // tokio::broadcast (cap 256), lossy (Lagged)
impl TopicRouter {
    pub fn subscribe_prefix(&self, prefix: &str) -> MergedSubscription;  // ⇐ observe TOUT le trafic ("" = firehose)
    pub fn published_total(&self) -> u64;
}
```
- Le bus est un **`broadcast`** : ajouter un abonné (le dashboard) **ne perturbe pas** les consommateurs
  existants. `subscribe_prefix("")` = firehose de tous les messages inter-agent. **Aucune modification
  du bus** : on s'abonne, on draine, on relaie.

### 1.3 Les seams de métriques (Inc.2, déjà câblés bout-en-bout)
`ProvenanceMetrics` (records, traced_ratio, chain_breaks, overhead, bytes) + `SubstrateMetrics`
(`swarm_blackboard_writes_total`, `swarm_events_published_total`, propagation…). `render_prometheus()`
**et** `snapshot()` (serde). Le dashboard **échantillonne** ces snapshots (tick périodique) pour les
jauges (panneaux provenance + trafic agrégé).

### 1.4 Le pattern SSE (axum 0.7) — prior art `efe-preview-server` / `efe-server`
```rust
use axum::response::sse::{Event, KeepAlive, Sse};
async fn stream(...) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let s = async_stream::stream! { while let Some(ev) = sub.recv().await { yield Ok(Event::default().event(ev.kind()).json_data(ev)?); } };
    Sse::new(s).keep_alive(KeepAlive::default())
}
```
- `efe-preview-server` (Preview Lab) **est** un bridge HTTP/SSE axum 0.7 — patron direct (events typés,
  `keep_alive`, `async_stream`). Deps : `async-stream 0.3`, `tokio-stream`, `futures`.

---

## 2. Décision d'architecture — un crate **léger** `efe-swarm-dashboard` + intégration gateway

> **Séparer l'OBSERVABILITÉ (fan-in seams → fan-out) du TRANSPORT (SSE/HTTP) et de l'UI.**

- **`crates/efe-swarm-dashboard`** (cœur, **sans HTTP**, testable seul) : le `DashboardBus`, le type de
  fil `DashboardEvent` (serde), les taps (`TeeSink`, `DashWriteSink`, `EventTap`), le **ring buffer de
  rejeu** + le `broadcast` de diffusion. Deps : `efe-blackboard`, `efe-swarm-sdk`, `efe-swarm-provenance`,
  `tokio` (sync/rt), `serde`/`serde_json`, `uuid`.
- **`efe-swarm-gateway`** (Inc.2, étendu) : les **routes SSE** (`GET …/runs/:id/events`, `GET …/events`),
  le **modèle job** (exécution async qui alimente le bus), et sert l'**UI statique** (`GET /dashboard`).
  Deps ajoutées : `async-stream`, `tokio-stream`, `futures`.
- **`efe-swarm-sdk`** (étendu, additif) : `SwarmBuilder::with_observer(Arc<dyn SwarmObserver>)` — un hook
  **générique** (le SDK ignore le mot « dashboard ») qui tee le `WriteSink`, branche l'`EventTap` au
  `SwarmContext`, et notifie le cycle de vie (call/seal/finish). Le `DashboardBus` **implémente**
  `SwarmObserver`.
- **`efe-swarm-topology`** (étendu, additif) : `SwarmContext.event_tap: Option<Arc<dyn EventTap>>` —
  analogue à `substrate_metrics` (Inc.2) ; les topologies T2/T-Consensus, après création de leur
  `TopicRouter`, **drainent** `subscribe_prefix("")` vers le tap (tâche détachée).

Ainsi : l'UI peut être **servie par le gateway** *ou* embarquée ailleurs ; le `DashboardBus` est
**réutilisable** (CLI live, tests) ; le SDK reste **pur** (un trait, pas de HTTP) ; **aucun agent
n'est modifié**.

---

## 3. Le `DashboardBus` — fan-in des seams → fan-out (live + rejeu)

```rust
// efe-swarm-dashboard/src/bus.rs
pub struct DashboardBus {
    tenant: TenantId,
    run_id: Uuid,
    seq: AtomicU64,                                   // ordre total monotone (le navigateur réordonne)
    tx: broadcast::Sender<DashboardEnvelope>,         // fan-out live (lossy : best-effort)
    ring: Mutex<VecDeque<DashboardEnvelope>>,         // rejeu borné (runs courts : le navigateur arrive après)
    ring_cap: usize,                                  // ex. 4096 events
    dropped: AtomicU64,                               // events perdus (diffusion lossy) — exposé en métrique
}

#[derive(Clone, Serialize)]
pub struct DashboardEnvelope { pub seq: u64, pub at_ms: u64, pub event: DashboardEvent }

impl DashboardBus {
    pub fn new(tenant: TenantId, run_id: Uuid, ring_cap: usize) -> Arc<Self>;
    pub fn emit(&self, event: DashboardEvent);        // seq++ → ring.push (drop-oldest) + tx.send (ignore NoSub)
    pub fn subscribe(&self) -> broadcast::Receiver<DashboardEnvelope>;  // live tail
    pub fn replay(&self) -> Vec<DashboardEnvelope>;   // snapshot du ring (catch-up)
    pub fn dropped(&self) -> u64;
}
```

**Rejeu + live (le problème de timing tranché)** : un run keyless dure ~ms — le navigateur se connecte
*après*. Le SSE handler **draine d'abord le ring** (catch-up : tout ce qui s'est déjà passé), **puis**
streame le `broadcast` (tail live). Un run self-host (secondes) est vu en direct. Le ring est **borné**
(drop-oldest) : la mémoire est plafonnée ; un run géant tronque le début (le navigateur l'annonce).

**Lossy assumé (honnêteté)** : `broadcast` + `try_send` perdent des events sous rafale (`dropped`
exposé) — c'est un **flux d'observation**, pas la vérité. La **vérité durable** = la chaîne de
provenance (`verify_swarm`, Inc.2). Le dashboard **ne remplace jamais** la preuve ; il la *montre*.

**Registre** : `DashboardRegistry { runs: DashMap<Uuid, Weak<DashboardBus>>, ttl }` dans l'état gateway
— mappe `run_id → DashboardBus` (tenant-scopé), borné/TTL, nettoyé après fin de run + grâce.

---

## 4. `DashboardEvent` — la taxonomie de fil (le contrat SSE)

```rust
// efe-swarm-dashboard/src/event.rs   (enum tagué "kind", snake_case ; tout serde)
#[derive(Clone, Serialize)] #[serde(tag = "kind", rename_all = "snake_case")]
pub enum DashboardEvent {
    // ── cycle de vie ───────────────────────────────────────────────────────────
    RunStarted   { topology: String, mode: String, budget_cap_cents: u32, assumed_agents: usize },
    RunFinished  { elapsed_ms: u64, agents_used: usize, output_preview: String, verified: bool, root: Option<String> },

    // ── PANNEAU 1 : trafic inter-agent (seam WriteSink + EventBus) ───────────────
    AgentWrite   { pointer: String, version: u64, agent_id: String, bytes: usize, lane: Lane },   // /cand,/ballot,/out → lane
    AgentMessage { topic: String, from: String, bytes: usize, latency_ms: Option<f64> },          // SwarmEvent
    AgentCall    { agent_id: String, role: String, kind: String, cost_cents: u32 },               // seam Meter (SDK)

    // ── PANNEAU 2 : consensus (dérivé des writes /cand,/ballot + résultat) ───────
    Candidate    { variant: usize, value_preview: String },
    Ballot       { judge: String, scores: Vec<f64> },
    Decided      { pattern: String, winner_preview: String, confidence: f64, judges_used: usize },

    // ── PANNEAU 3 : provenance (seam ProvenanceMetrics + seal) ───────────────────
    Sealed       { records: usize, traced_pct: f64, root: Option<String>, verified: bool },

    // ── PANNEAU 4 : budget (seam BudgetGuard/Meter) ──────────────────────────────
    BudgetTick   { spent_cents: u32, max_cents: u32, per_agent: BTreeMap<String, u32>, exhausted: bool },

    // ── jauges agrégées (tick périodique des snapshots de métriques) ─────────────
    MetricsTick  { bb_writes: u64, events_published: u64, traced_ratio: f64, chain_breaks: u64 },
}

#[derive(Clone, Copy, Serialize)] #[serde(rename_all = "snake_case")]
pub enum Lane { Candidate, Ballot, Output, Other }   // classe le pointeur → couloir visuel
```
- **`*_preview`** : valeurs **tronquées** (ex. 120 chars) — un dashboard ne fuit pas de données
  massives ; les vraies sorties passent par `GET …/runs/:id` (Inc.2). PII : `output_preview` est
  tronqué **et** (option) passé par `efe-pii` si activé côté tenant.
- Chaque event devient une **frame SSE** : `event: <kind>\ndata: <json DashboardEnvelope>\n\n`.

---

## 5. Les taps — comment chaque seam alimente le bus (sans toucher aux agents)

### 5.1 Seam `WriteSink` → `TeeSink` (provenance **et** dashboard)
```rust
// efe-swarm-dashboard/src/tap.rs
pub struct DashWriteSink { bus: Arc<DashboardBus> }          // WriteSink → DashboardEvent
impl WriteSink for DashWriteSink {
    fn on_write(&self, w: &BlackboardWrite) {
        self.bus.emit(DashboardEvent::AgentWrite { pointer: w.pointer.clone(), version: w.version,
            agent_id: w.agent_id.to_string(), bytes: w.bytes, lane: Lane::classify(&w.pointer) });
        // dérive consensus : /cand/* → Candidate, /ballot/* → Ballot (lecture du payload via le recorder).
    }
}
// efe-blackboard (ou dashboard) : fan-out WriteSink — n'altère pas le contrat Phase 4.
pub struct TeeSink(Vec<Arc<dyn WriteSink>>);
impl WriteSink for TeeSink { fn on_write(&self, w) { for s in &self.0 { s.on_write(w); } } }
```
> Le SDK, en présence d'un observer, construit `sink = TeeSink([ProvenanceSink, DashWriteSink])` au lieu
> du seul `ProvenanceSink`. La provenance reste **byte-identique** ; le dashboard voit les mêmes writes.

### 5.2 Seam `EventBus` → `EventTap` (drain de `subscribe_prefix("")`)
```rust
pub trait EventTap: Send + Sync { fn on_event(&self, ev: &SwarmEvent); }   // topology.rs (analogue substrate_metrics)
pub struct DashEventTap { bus: Arc<DashboardBus> }
impl EventTap for DashEventTap { fn on_event(&self, ev) {
    self.bus.emit(DashboardEvent::AgentMessage { topic: ev.topic.clone(), from: ev.published_by.to_string(),
        bytes: ev.payload.to_string().len(), latency_ms: None }); } }
```
- La topologie (T2/T-Consensus), après avoir créé son `TopicRouter`, **spawn** une tâche :
  `let mut sub = router.subscribe_prefix(""); while let Some(ev) = sub.recv().await { tap.on_event(&ev) }`.
  Le bus étant `broadcast`, cet abonné **n'enlève rien** aux consommateurs. La tâche s'arrête à la
  quiescence (bus fermé → `recv → None`).

### 5.3 Seam `Meter`/`BudgetGuard` → `AgentCall` + `BudgetTick` (SDK)
- Le `Meter` (SDK Inc.1) enveloppe **chaque** appel d'agent (charge budget + capture provenance) :
  c'est le seam idéal pour `AgentCall` + `BudgetTick`. Sur `charge(who)`, il appelle
  `observer.on_agent_call(who, role, kind, cost)` puis émet le snapshot `guard.snapshot()` → `BudgetTick`.

### 5.4 Seam métriques → `MetricsTick` (échantillonnage)
- Un **ticker** (gateway, 250 ms) lit `prov_metrics.snapshot()` + `substrate_metrics.snapshot()` et
  émet `MetricsTick` sur le bus du run actif (jauges lissées). S'arrête à `RunFinished`.

---

## 6. Le modèle **job** — ce qui rend le live possible (async d'Inc.2 §7)

Inc.2 exécute `POST /run` **synchrone** (le run finit avant la réponse → rien à streamer en direct).
Inc.3 ajoute le **mode job** :

```
POST /v1/swarm/run?stream=true
  → 202 { run_id, status: "running", events: "/v1/swarm/runs/:id/events" }   (réponse IMMÉDIATE)
  → le serveur spawn la tâche : crée le DashboardBus(run_id), l'enregistre, exécute run_traced
    (observer = DashboardBus) ; à la fin : store.put (records), emit RunFinished, retient le SwarmRun.
GET /v1/swarm/runs/:id            → statut/résultat (poll) — { status, run? }
GET /v1/swarm/runs/:id/events     → SSE : replay(ring) puis live tail jusqu'à RunFinished
```
- **Compatibilité** : sans `?stream=true`, comportement Inc.2 **inchangé** (sync, pas de bus). Le job
  est **opt-in**.
- **Runs courts (keyless)** : le job finit vite, mais le **ring** garde tout → le navigateur rejoue la
  séquence complète (animation déterministe). **Runs longs (self-host)** : vus en direct.
- **Cycle de vie** : `RunStore` (durable, la preuve) **+** `DashboardRegistry` (éphémère, le live).
  Deux responsabilités, deux durées de vie.

---

## 7. Les routes SSE (gateway) + handler

| route | méthode | rôle |
|---|---|---|
| `POST /v1/swarm/run?stream=true` | POST | lance un **job**, renvoie `{run_id, status, events}` (202) |
| `GET /v1/swarm/runs/:id/events` | GET (SSE) | **replay + live** des `DashboardEvent` d'un run (tenant-scopé) |
| `GET /v1/swarm/events` | GET (SSE) | firehose **tenant** (tous les runs vivants du tenant) — vue cluster |
| `GET /v1/swarm/runs/:id` | GET | statut/résultat du job (poll) |
| `GET /dashboard` | GET | l'**UI vanilla** (HTML/JS/CSS statique embarqué) |

```rust
// routes/events.rs
async fn run_events(State(s): State<SwarmGatewayState>, caller: Caller, Path(id): Path<Uuid>)
    -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, GatewayError>
{
    caller.require_any_role(&["viewer","operator","admin","auditor"])?;
    let bus = s.dashboards.get(id, &caller.tenant).ok_or(GatewayError::NotFound)?;   // tenant-scopé
    let replay = bus.replay();
    let mut live = bus.subscribe();
    let stream = async_stream::stream! {
        for env in replay { yield Ok(Event::default().event(env.event.kind()).json_data(env)?); }   // catch-up
        loop {
            match live.recv().await {
                Ok(env) => { let last = env.is_finished(); yield Ok(Event::default().event(env.event.kind()).json_data(env)?); if last { break; } }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,                              // best-effort
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
```
- **Tenant scoping** : `dashboards.get(id, tenant)` ne rend le bus que pour le bon tenant (isolation,
  comme `RunStore`). Le firehose `/events` ne diffuse que les runs du tenant appelant.

---

## 8. L'UI vanilla — 4 panneaux, zéro framework

`GET /dashboard` sert **un seul** fichier (HTML + CSS + JS inline, embarqué `include_str!` → servi en
`Html<&'static str>` : **aucune dépendance** type `tower-http/ServeDir`, gateway léger). Le JS :

```js
const url = new URLSearchParams(location.search).get('run')
  ? `/v1/swarm/runs/${run}/events` : `/v1/swarm/events`;
const es = new EventSource(url);                 // SSE natif navigateur (pas de lib)
es.addEventListener('agent_write',   e => traffic.onWrite(JSON.parse(e.data)));
es.addEventListener('agent_message', e => traffic.onMessage(JSON.parse(e.data)));
es.addEventListener('candidate', e => consensus.onCandidate(JSON.parse(e.data)));
es.addEventListener('ballot',    e => consensus.onBallot(JSON.parse(e.data)));
es.addEventListener('decided',   e => consensus.onDecided(JSON.parse(e.data)));
es.addEventListener('sealed',      e => provenance.update(JSON.parse(e.data)));
es.addEventListener('budget_tick', e => budget.update(JSON.parse(e.data)));
es.addEventListener('metrics_tick',e => gauges.update(JSON.parse(e.data)));
```

| panneau | rendu | source |
|---|---|---|
| **1 · Trafic inter-agent** | `<canvas>` : nœuds=agents (cercles), arêtes=messages (animées, fade), couloirs par `Lane` ; compteur writes/s + events/s | `agent_write`, `agent_message`, `agent_call` |
| **2 · Consensus** | colonnes candidats `/cand/k` + grille ballots (juge × score, heatmap) ; bandeau `Decided` (pattern, gagnant, confiance) | `candidate`, `ballot`, `decided` |
| **3 · Provenance** | `traced_pct` (barre), `records` (compteur), `provenance_root` (mono, tronqué) + pastille `verified` (vert/rouge), `chain_breaks` | `sealed`, `metrics_tick` |
| **4 · Budget** | barre dépense/plafond (rouge à l'épuisement) + table par agent (top-N) ; badge politique | `budget_tick` |

- **Robustesse UI** : reconnexion auto (`EventSource` natif) ; `seq` pour ordonner ; bornage du DOM
  (ring visuel : N dernières arêtes). **Accessible offline** : aucun CDN, tout embarqué.
- **Esthétique « petits agents en parallèle »** : le canvas anime *réellement* les messages — c'est la
  démonstration vivante de la thèse EFE.

---

## 9. Changements requis (additifs, minimes) — *« vérifier que tout est branché »*

### 9.1 `efe-swarm-topology` (analogue à `substrate_metrics` d'Inc.2)
- `SwarmContext.event_tap: Option<Arc<dyn EventTap>>` + `with_event_tap(...)`.
- T2 `PipelineTopology::run` / T-Consensus `ConsensusTopology::run` : si `ctx.event_tap` présent,
  `tokio::spawn` un drain de `router.subscribe_prefix("")` → `tap.on_event(&ev)` (s'arrête à quiescence).
- `EventTap` trait (object-safe). **Aucune** topologie/agent réécrit·e.

### 9.2 `efe-swarm-sdk` (un hook générique, le SDK ignore « dashboard »)
```rust
pub trait SwarmObserver: Send + Sync {
    fn write_sink(&self) -> Option<Arc<dyn WriteSink>>;   // tee avec ProvenanceSink
    fn event_tap(&self)  -> Option<Arc<dyn EventTap>>;    // branché au SwarmContext
    fn on_run_start(&self, topology: &str, mode: &str, budget_cap: u32);
    fn on_agent_call(&self, who: &str, role: &str, kind: &str, cost_cents: u32, budget: &BudgetSnapshot);
    fn on_seal(&self, records: usize, traced_pct: f64, root: Option<&str>, verified: bool);
    fn on_finish(&self, run: &SwarmRun);
}
// SwarmBuilder::with_observer(Arc<dyn SwarmObserver>)  →  run_traced :
//   sink = match observer.write_sink() { Some(d) => TeeSink([prov, d]), None => prov };
//   ctx  = ctx.with_event_tap(observer.event_tap()?) ;
//   Meter appelle observer.on_agent_call(...) à chaque charge ;
//   après seal : observer.on_seal(...) ; en fin : observer.on_finish(&run).
```
- **Non-breaking** : sans observer, `run`/`run_traced` **inchangés** (le `TeeSink` n'apparaît que si un
  observer fournit un sink). Compatible Inc.1/Inc.2.

### 9.3 `efe-swarm-gateway`
- `SwarmGatewayState.dashboards: Arc<DashboardRegistry>` (run_id → Weak<DashboardBus>, tenant-scopé, TTL).
- Mode **job** (`?stream=true`) : spawn run_traced avec `observer = DashboardBus::new(...)` enregistré ;
  ticker métriques ; `RunFinished` à la fin.
- Routes SSE + `GET /dashboard` (UI embarquée).

---

## 10. Sécurité, isolation, honnêteté (règle absolue rappelée)
- **Aucune clé** : le dashboard montre le `provenance_root` (engagement public), **jamais** la clé
  tenant (HKDF, server-side). Identique à `verify` (Inc.2).
- **Tenant scoping** : `runs/:id/events`, `/events`, `/dashboard` — tout est scopé au tenant de l'appelant
  (`Caller`/`AuthCtx`). Le firehose ne fuit **jamais** d'un tenant à l'autre.
- **Previews tronqués** + hook `efe-pii` optionnel : le live ne déverse pas de données massives/PII.
- **Best-effort assumé** : flux lossy (`dropped` exposé) — **pas** une source d'audit. La preuve, c'est
  `verify_swarm` (Inc.2). On le **dit** dans l'UI (« live · best-effort ; preuve → /verify »).
- **DoS** : ring borné + TTL + plafond d'abonnés SSE par tenant ; le bus `broadcast` borne la mémoire.

---

## 11. Métriques & SLO
| métrique | cible |
|---|---|
| latence seam → frame SSE (p50) | **< 10 ms** |
| overhead dashboard sur un run (avec observer vs sans) | **< 5 %** (writes tee + emit lock-light) |
| events perdus en régime nominal (keyless) | **0** (ring ≥ events d'un run typique) |
| reconnexion SSE → état cohérent | **< 1 s** (replay du ring) |
| `/dashboard` sans dépendance externe (offline) | **assertion** (tout embarqué) |
- Nouvelles surfaces Prometheus : `efe_swarm_dash_events_emitted_total`, `…_dropped_total`,
  `…_active_subscribers`, `…_active_runs`.

---

## 12. Phasage de l'Inc.3
- **3.a — cœur** : crate `efe-swarm-dashboard` : `DashboardEvent`, `DashboardBus` (ring + broadcast),
  `TeeSink`/`DashWriteSink`/`DashEventTap`. Tests unitaires (emit→replay→subscribe, drop-oldest, lossy).
- **3.b — SDK** : `SwarmObserver` + `with_observer` ; tee du `WriteSink` ; `on_agent_call`/`on_seal`/
  `on_finish`. Tests (observer reçoit la séquence attendue d'un run keyless).
- **3.c — topology** : `SwarmContext.event_tap` + drain `subscribe_prefix("")` (T2/T-Consensus). Tests.
- **3.d — gateway** : `DashboardRegistry`, mode **job** (`?stream=true`), routes SSE (`runs/:id/events`,
  `events`), ticker métriques. Tests axum (`oneshot` : 202 job ; SSE replay non vide ; tenant scoping).
- **3.e — UI** : `dashboard.html` vanilla (4 panneaux, canvas trafic), `GET /dashboard`. Démo keyless
  (rejeu) + self-host (live). ADR-SWARM-010.

---

## 13. Arborescence cible
```
crates/efe-swarm-dashboard/
  src/{lib, bus, event, tap, registry}.rs
  tests/replay_and_live.rs
crates/efe-swarm-gateway/
  src/routes/{events, dashboard}.rs        # SSE + UI ; state.rs += dashboards ; routes/run.rs += mode job
  assets/dashboard.html                    # UI vanilla (embarquée via include_str!)
crates/efe-swarm-sdk/src/{sdk,observer}.rs # SwarmObserver + with_observer (additif)
crates/efe-swarm-topology/src/topology.rs  # SwarmContext.event_tap (additif) + drains T2/T-Consensus
docs/docs/adr/ADR-SWARM-010-live-dashboard-makes-the-swarm-visible.md
```

---

## 14. Risques & parades
| risque | parade |
|---|---|
| Run trop court pour être vu en direct | **ring de rejeu** : le SSE rejoue tout, puis tail live (`?stream=true` job) |
| Flux lossy pris pour une source d'audit | **honnêteté UI** + `dropped` exposé ; la preuve = `verify_swarm` (chaîne durable) |
| Coupler l'observabilité au SDK | trait **générique** `SwarmObserver` (le SDK ignore « dashboard ») ; crate séparé |
| Modifier les agents/topologies/bus | taps sur **seams existants** (`WriteSink` tee, `broadcast` multi-sub, métriques) — zéro agent réécrit |
| Fuite inter-tenant via le firehose | tout **tenant-scopé** (registry + routes) ; clé jamais exposée (root oui) |
| Overhead sur le chemin chaud | `emit` lock-léger + `try_send` (jamais bloquant) ; tee O(n sinks) ; ticker hors chemin |
| Dépendances UI lourdes / offline | **HTML/JS embarqué** (`include_str!`), `EventSource` natif, **aucun CDN** |

---

## 15. Définition de « fait » (Inc.3)
1. `efe-swarm-dashboard` : `DashboardBus` (ring + broadcast) + `DashboardEvent` (serde) + taps
   (`TeeSink`/`DashWriteSink`/`DashEventTap`) — tests verts (rejeu, live, lossy).
2. SDK `with_observer` (additif, non-breaking) : tee `WriteSink` + `event_tap` + `on_agent_call`/
   `on_seal`/`on_finish` — un run keyless produit la **séquence d'events attendue**.
3. topology `SwarmContext.event_tap` (additif) : T2/T-Consensus relaient le trafic de bus.
4. gateway : mode **job** (`?stream=true`), `GET …/runs/:id/events` (SSE **replay + live**),
   `GET …/events` (firehose tenant), `GET /dashboard` (UI vanilla 4 panneaux).
5. **Tenant scoping** partout ; **aucune clé** manipulée ; flux **best-effort** assumé (preuve = Inc.2).
6. Tests (axum `oneshot`) verts, **0 warning** ; démo keyless (rejeu) + self-host (live). ADR-SWARM-010.

> **Le livrable** : *on **voit** les petits agents se passer l'info en parallèle — writes stigmergiques,
> messages de bus, ballots, scellement, budget — en direct, à 4 panneaux ; puis on re-prouve le run
> (Inc.2). La thèse EFE, rendue visible **et** vérifiable.*
