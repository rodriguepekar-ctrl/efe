<!-- SPDX-License-Identifier: UNLICENSED -->
# EFE · A1 · Phase 4 — Preview Lab (frontend SSE) · Architecture

> **Statut** : architecture (ultra-think, chirurgicale)
> **Phase** : 4 — premier livrable post-R3 · **version cible `0.3.0`**
> **Type** : livrable **visible** — l'UI qui rend tangible la valeur que R3 a chiffrée
> **Dépend de** : tout R3 (microcents, télémétrie, A/B, savings report v2) + le bus
> d'événements `StreamEvent` (stable depuis R2, `EVENT_SCHEMA_VERSION = 1.0.0`)

---

## 1. Résumé exécutif

R3 a rendu la valeur d'EFE **mesurable** (savings cost en microcents, télémétrie,
A/B). Mais elle reste **invisible** : un JSON de rapport, un endpoint `/metrics`,
des assertions de test. Phase 4 la rend **visible et interactive** — un *Preview
Lab* où l'on tape une requête, regarde le pré-mâchage se dérouler en direct
(robots SLM qui s'allument, tokens du gros LLM qui streament, verdict, métriques
de savings), et compare les modes côte à côte.

**Découverte structurante (audit du code)** : ~70 % du backend nécessaire **existe
déjà** et est versionné :

| Brique backend | État | Preuve |
|---|---|---|
| Protocole d'événements `StreamEvent` | ✅ **conçu POUR le SSE** | `src/events/stream.rs:2` : *« protocole SSE exposé à l'UI »*, `data: <json>\n\n`, `EVENT_SCHEMA_VERSION="1.0.0"`, 24 variants, `#[serde(tag="type")]` |
| Bus pub/sub `EventBus` | ✅ complet | `subscribe_filtered(filter_by_swarm)`, `InMemoryBus` (mpsc, cap 4K, drop+count), `NatsBus` (distribué) |
| Émission des events | ✅ déjà câblée | `mission_a` publie `SwarmStarted`→`RobotStarted/Finished`→`LlmTokenStreamed`→`PreChewingMetrics`→`SwarmFinished` |
| Serveur HTTP zéro-dép | ✅ pattern existant | `metrics_server.rs` (raw `tokio::net`, `/metrics`+`/healthz`) |
| Données savings | ✅ produit live | `CHEWING_SAVINGS_REPORT_v2.json` (R3.S9) |
| `axum 0.7` | ✅ déjà au workspace | `efe-gateway/Cargo.toml:38` |

**Donc Phase 4 = (a)** un **bridge SSE mince** (`efe-preview-server`, nouvelle crate
binaire qui *abonne* une connexion HTTP au bus et *streame* les events) **+ (b)** un
**frontend Next.js + Tailwind** consommant ce flux. Aucune réécriture du moteur.

---

## 2. Scope / Non-scope

### 2.1 In-scope

1. **`efe-preview-server`** (NEW crate binaire) : pont HTTP/SSE devant `efe-coordinator`.
   - `POST /v1/missions/{a|b|speculative}` → démarre une mission, **streame** ses
     `StreamEvent` en `text/event-stream`.
   - `GET /v1/savings-report` → sert `CHEWING_SAVINGS_REPORT_v2.json`.
   - `GET /v1/schema` → `{ event_schema_version, variants[] }` (contrat UI↔backend).
   - `GET /metrics`, `GET /healthz` → réexpose la télémétrie R3.S7.
   - `GET /` + assets → sert le build statique Next.js (mode production).
2. **`preview-lab/`** (NEW app Next.js, hors crates Rust) : 4 « scènes » (§ 8).
3. **Contrat de types** TS ↔ Rust (`StreamEvent` → `types.ts`) + test de non-dérive.
4. **Petit ajout coordinator** (chirurgical) : exposer le `swarm_id` au caller pour
   un abonnement sans course (cf. D3) — `Coordinator::run_mission_*_streamed`.

### 2.2 Non-scope (explicite)

- ❌ Auth/multi-tenant production, billing UI (le Lab est un **outil dev**, bind
  localhost ; la prod passera par `efe-gateway`).
- ❌ Modifier le moteur de pré-mâchage, les missions, le savings report.
- ❌ Nouvelle dépendance **au workspace** (axum 0.7 y est déjà ; le Lab le réutilise
  dans une crate dédiée pour garder `efe-coordinator` lean).
- ❌ Persistance des sessions (le Lab est éphémère/in-mem ; l'historique = hors-scope).
- ❌ Le `NatsBus` distribué (le Lab utilise `InMemoryBus` ; NATS = déploiement multi-réplica, Phase 4+).

---

## 3. État actuel — inventaire chirurgical

### 3.1 Le protocole `StreamEvent` (déjà le contrat SSE)

24 variants, tous porteurs de `swarm_id` (corrélation) + `ts` (UTC), `#[serde(tag =
"type", rename_all = "SCREAMING_SNAKE_CASE")]`. Catégories :

- **Cycle de vie** : `SwarmStarted`, `SwarmFinished`, `SwarmFailed`, `Heartbeat`.
- **Pré-mâchage SLM** : `RobotStarted`, `RobotFinished`, `PheromoneDeposited`, `SlmCircuitOpened`.
- **Gros LLM** : `LlmTokenStreamed`, `LlmFinished`, `LlmFallbackTriggered`, `LlmFailed`.
- **Fact-check / vote** : `VerdictReady`, `VoteResolved`.
- **Speculative** : `DraftToken`, `SpeculationResolved`, `DrafterBypassed`, `DraftRejectedAsNoise`.
- **Savings** : `PreChewingMetrics { metrics: ChewingMetrics }` (le cœur économique).

`StreamEvent::tag()` (string filtrable) et `swarm_id()` (Option) existent déjà.
`Heartbeat` est **explicitement** prévu pour garder le SSE chaud (~15 s ; les proxies
coupent à 60 s d'inactivité — commentaire `stream.rs:245`).

### 3.2 Le bus `EventBus`

```
trait EventBus {
  async publish(event)                          // fire-and-forget, non bloquant
  subscribe(name) -> (token, Receiver<StreamEvent>)
  subscribe_filtered(name, filter) -> (token, Receiver<StreamEvent>)
  unsubscribe(token); dropped_count(token); flush(); subscriber_count()
}
```

Helpers prêts : `filter_by_swarm(swarm_id)`, `filter_by_tag`, `filter_and/or`.
`InMemoryBus` : mpsc borné (cap 4096/subscriber ; au-delà drop + compteur — **un
client SSE lent ne bloque jamais une mission**). `Coordinator.bus: Arc<dyn EventBus>`
est **public** + helper `Coordinator::subscribe()`.

### 3.3 Le pattern serveur (à mirrorer)

`metrics_server.rs` : `tokio::net::TcpListener`, `handle_conn` parse la request-line,
formate une réponse HTTP/1.1, bind `127.0.0.1:9090`, zéro dépendance. C'est le
**gabarit** ; mais pour un vrai routeur + SSE + body POST + CORS + statique, on
préfère **axum** (déjà au workspace) dans une crate séparée (cf. D1).

---

## 4. Décisions chirurgicales

### D1 — `efe-preview-server` : crate binaire séparée, axum (déjà au workspace)
On NE met PAS le serveur HTTP dans `efe-coordinator` (qui doit rester une lib lean,
feature `metrics` = `tokio/net` only). On crée **`crates/efe-preview-server`** qui
dépend de `efe-coordinator` (lib) + `axum 0.7` + `tower-http` (CORS, `ServeDir`).
axum est **déjà** une dépendance vetted du workspace (`efe-gateway`) → **zéro
nouvelle dépendance au workspace**, juste une réutilisation. Justification : un
bridge SSE+POST+statique hand-rollé en raw `tokio` serait fragile (parsing body,
chunked, CORS) ; axum donne `Sse<Stream>` natif, et l'isolation en crate dédiée
protège la pureté du coordinator.

### D2 — Modèle d'exécution : POST qui streame directement (fetch + ReadableStream)
Deux transports SSE possibles :
- **(Primaire) `POST /v1/missions/{type}` → réponse `text/event-stream`** : le client
  ouvre via `fetch()` + `ReadableStream` (lit le body en streaming). **Un seul
  aller-retour, aucune course** (le body de requête porte la mission ; le stream
  démarre sur la même connexion). C'est le choix par défaut.
- **(Alternative documentée) EventSource natif** : `EventSource` ne fait que `GET`.
  Pour l'offrir (reconnexion `Last-Event-ID`), on ajoute `POST /v1/missions` (renvoie
  `{swarm_id}`) puis `GET /v1/stream/{swarm_id}` (EventSource). Réservé à un besoin de
  reconnexion ; pas requis pour un outil dev.

### D3 — Abonnement sans course : exposer le `swarm_id` au caller (ajout chirurgical)
`run_mission_a` génère `swarm_id` **en interne** (`stream.rs`-side, `Uuid::now_v7()`).
Le bridge doit s'abonner **avant** que la mission publie `SwarmStarted`, sinon il rate
le 1er event. Deux options :
- **(Primaire) Ajout coordinator minimal** : `Coordinator::run_mission_a_streamed(req)
  -> (Uuid, Receiver<StreamEvent>, JoinHandle<CoordResult<MissionAResponse>>)` qui
  (1) génère `swarm_id`, (2) `bus.subscribe_filtered(filter_by_swarm(swarm_id))`, (3)
  spawn la mission **avec ce `swarm_id` injecté**, (4) renvoie le trio. ~15 lignes,
  `run_mission_a` accepte un `swarm_id: Option<Uuid>` (défaut = `now_v7`,
  backward-compat). **Garantit zéro event manqué.**
- **(Fallback sans changement backend)** : le bridge `subscribe()` (tout), spawn la
  mission, et **se verrouille sur le premier `SwarmStarted`** (adopte son `swarm_id`,
  jette les events d'autres swarms). Correct pour un Lab mono-utilisateur ; verbeux en
  concurrence. On documente, on préfère la voie primaire.

### D4 — Exposition & sécurité : localhost par défaut, bearer optionnel
Bind `127.0.0.1:8700` par défaut (comme `metrics_server` en localhost). CORS autorise
`http://localhost:3000` (Next.js dev). Pour exposer hors-localhost : variable
`EFE_PREVIEW_TOKEN` → exige `Authorization: Bearer …`. **Aucun secret LLM ne transite
par le front** : les clés API restent côté `efe-preview-server` (lu de l'env, comme le
coordinator). Le front ne voit que des `StreamEvent`.

### D5 — Contrat de types TS ↔ Rust + test de non-dérive
`StreamEvent` est `#[serde(tag="type")]` → mapping direct vers une union TypeScript
discriminée. On génère/maintient `preview-lab/src/lib/events.ts` (24 variants). Garde-
fou : `GET /v1/schema` renvoie `{ event_schema_version, variants: [...tags] }` ; au
boot, le front **assert** que sa version compile-time == celle du serveur, sinon
bannière « schema drift ». Côté Rust, un test vérifie que `StreamEvent::tag()` couvre
les 24 variants (exhaustivité du `match` déjà garantie par le compilateur).

### D6 — Le Lab = 4 scènes (le périmètre produit)
1. **Live Mission** : composer + visualisation pipeline + stream tokens + jauge savings.
2. **Savings Report** : le v2 (hero « R2 +0 % vs R3 −27 % », par mode, par fixture).
3. **A/B Lab** : 2 tables de scoring, N missions, ségrégation control/experiment.
4. **Resilience Theater** : déclenche chaos, observe `SlmCircuitOpened`,
   `LlmFallbackTriggered`, `DrafterBypassed`, `DraftRejectedAsNoise`.

### D7 — Mapping visuel : le pipeline comme flux horizontal
`Analyzer → Router → Compressor → GROS LLM → Verifier`, chaque nœud :
- s'**allume** sur `RobotStarted` (pulse),
- se **remplit** sur `RobotFinished` (latence + ✓/✗),
- le nœud LLM **streame** les `LlmTokenStreamed` (texte qui s'écrit),
- `PheromoneDeposited` alimente une **sidebar blackboard**,
- `PreChewingMetrics` remplit la **jauge triple** (latence / coût µ¢ / qualité),
- les events S8/R3.S3 (`SlmCircuitOpened`, fallback, noise) s'affichent en **overlays**.

---

## 5. Architecture cible

```
┌─────────────────────────────┐         ┌──────────────────────────────────────┐
│  Browser (Preview Lab)      │         │  efe-preview-server (NEW crate, axum)  │
│  Next.js + Tailwind + TS    │         │                                        │
│                             │  POST   │  POST /v1/missions/a   ──┐             │
│  fetch(POST, body=mission)  │ ──────► │   (body: MissionDTO)     │             │
│   ↳ ReadableStream reader   │ ◄────── │   resp: text/event-stream│             │
│      parse `data: {json}`   │  SSE    │            ▲             │             │
│                             │         │            │   run_mission_*_streamed │
│  scenes: Live / Savings /   │  GET    │   (swarm_id, Receiver, JoinHandle)    │
│          A/B / Resilience   │ ──────► │            │                          │
└─────────────────────────────┘         │   ┌────────┴─────────────────────┐    │
        │  GET /v1/savings-report        │   │ efe-coordinator (lib)         │   │
        │  GET /v1/schema                │   │  Coordinator { bus, providers,│   │
        │  GET /metrics                  │   │   clients, ab_test, metrics } │   │
        └────────────────────────────────┘   │  bus: Arc<dyn EventBus>       │   │
                                              │   InMemoryBus (mpsc, cap 4K)  │   │
                                              │  run_mission_a publishes →────┼─► SSE
                                              └───────────────────────────────┘   │
                                              │  reads CHEWING_SAVINGS_REPORT_v2  │
                                              └──────────────────────────────────┘
```

**Flux d'une mission (POST-streame, D2 primaire, D3 sans course)** :
1. `POST /v1/missions/a` body `{ user_text, task_class, available_llms, mission_opts }`.
2. Handler : `let (swarm_id, mut rx, join) = coord.run_mission_a_streamed(req);`
3. Handler retourne immédiatement un `Sse(stream)` où le stream = `rx` mappé en
   `Event::default().data(serde_json::to_string(&ev))`, + un tick `Heartbeat` 15 s.
4. À la réception de `SwarmFinished`/`SwarmFailed`, le stream se ferme (et `join`
   fournit la réponse finale, loggée/auditée côté serveur).

---

## 6. Contrat API du bridge (`efe-preview-server`)

| Méthode + route | Body / Query | Réponse | Rôle |
|---|---|---|---|
| `POST /v1/missions/a` | `MissionADTO` | `text/event-stream` (`StreamEvent`) | Mission A live |
| `POST /v1/missions/b` | `MissionBDTO` | `text/event-stream` | Mission B (vote) live |
| `POST /v1/missions/speculative` | `SpecDTO` | `text/event-stream` | Speculative live |
| `POST /v1/ab/run` | `{ control, experiment, ratio, n }` | `text/event-stream` (events + récap A/B final) | Scène A/B Lab |
| `GET /v1/savings-report` | — | `application/json` (le v2) | Scène Savings |
| `GET /v1/schema` | — | `{ event_schema_version, variants[] }` | Garde-fou drift |
| `GET /metrics` | — | `text/plain` Prometheus | Réexpose R3.S7 |
| `GET /healthz` | — | `200 ok` | Liveness |
| `GET /` + `/_next/*` | — | statique | Build Next.js (prod) |

**DTOs** (couche anti-corruption mince — le front n'envoie pas un `MissionARequest`
brut) : `MissionADTO { user_text, task_class, available_llms?, budget? }`. Le serveur
mappe DTO → `MissionARequest` (avec `with_default_scoring`, budget par défaut). Cela
isole le front des champs internes (`task_class_scoring_table`, `tenant_id`).

---

## 7. Modèle de données

**Côté backend : aucun nouveau type d'événement.** On réutilise `StreamEvent` +
`ChewingMetrics` + `SavingsReport` tels quels. Les seuls ajouts :
- 4 **DTOs** de requête (`MissionADTO`, `MissionBDTO`, `SpecDTO`, `AbRunDTO`) dans
  `efe-preview-server` (sérde, validation légère : `user_text` non vide, etc.).
- `Coordinator::run_mission_*_streamed` (helpers, D3) + `swarm_id: Option<Uuid>` sur
  les `run_mission_*` (défaut préserve le comportement actuel).

**Côté front : `events.ts`** — union discriminée TS miroir de `StreamEvent` (24
variants), + types `ChewingMetrics`, `SavingsReportV2` (miroir du JSON).

---

## 8. Le frontend — structure

```
preview-lab/
├── package.json                 (next, react, tailwindcss, typescript)
├── src/
│   ├── app/
│   │   ├── layout.tsx           (shell + nav 4 scènes)
│   │   ├── page.tsx             → Live Mission (scène 1)
│   │   ├── savings/page.tsx     → Savings Report (scène 2)
│   │   ├── ab/page.tsx          → A/B Lab (scène 3)
│   │   └── resilience/page.tsx  → Resilience Theater (scène 4)
│   ├── lib/
│   │   ├── events.ts            (union TS miroir StreamEvent + ChewingMetrics)
│   │   ├── useMissionStream.ts  (hook : fetch POST + ReadableStream → events[])
│   │   └── api.ts               (fetch savings-report, schema, metrics)
│   └── components/
│       ├── Pipeline.tsx         (flux Analyzer→Router→Compressor→LLM→Verifier)
│       ├── RobotNode.tsx        (état idle/running/done/failed + latence)
│       ├── TokenStream.tsx      (LlmTokenStreamed → texte qui s'écrit)
│       ├── SavingsGauge.tsx     (PreChewingMetrics → triplet latence/coût/qualité)
│       ├── BlackboardPanel.tsx  (PheromoneDeposited timeline)
│       ├── SavingsReportView.tsx(hero R2 0% vs R3 −27% + tables par mode)
│       ├── AbComparison.tsx     (control vs experiment, Δ)
│       └── OverlayToasts.tsx    (circuit open, fallback, noise rejection)
```

**Le hook `useMissionStream`** (cœur technique) :
```
async function* readSSE(resp): AsyncGenerator<StreamEvent> {
  const reader = resp.body.getReader(); const dec = new TextDecoder();
  let buf = "";
  for (;;) {
    const { value, done } = await reader.read(); if (done) break;
    buf += dec.decode(value, { stream: true });
    for (const frame of buf.split("\n\n")) { /* parse "data: {json}" → yield JSON */ }
    buf = /* reste partiel */;
  }
}
```
Réducteur React : `events[]` → état dérivé `{ robots: Map<id,RobotState>, tokens:
string, metrics?: ChewingMetrics, verdicts[], overlays[] }`. La scène re-render à
chaque event ; la `SavingsGauge` s'anime à l'arrivée de `PreChewingMetrics`.

---

## 9. Mapping `StreamEvent` → UI (les 24 → affordances)

| Event | Affordance UI |
|---|---|
| `SwarmStarted` | démarre la timeline, affiche `user_text` + `task_class` |
| `RobotStarted` / `RobotFinished` | nœud pipeline pulse → se remplit (latence, ✓/✗) |
| `PheromoneDeposited` | ligne dans la **BlackboardPanel** (auteur, kind) |
| `LlmTokenStreamed` | **TokenStream** écrit le token (provider coloré) |
| `LlmFinished` | nœud LLM finalise (tokens, coût, latence) |
| `VerdictReady` | badge fact-check (SUPPORTED/… + similarité) |
| `VoteResolved` | barres de score par LLM, gagnant surligné (Mission B) |
| `DraftToken` | **TokenStream** fantôme (draft spéculatif gris) |
| `SpeculationResolved` | badge **HOT/SLOW PATH** + `latency_saved_ms` |
| `PreChewingMetrics` | **SavingsGauge** : triplet latence / coût µ¢ / qualité |
| `SwarmFinished` | timeline se ferme, hash audit affiché |
| `SwarmFailed` | bandeau erreur (stage + message) |
| `Heartbeat` | (silencieux ; garde la connexion) |
| `LlmFallbackTriggered` | **OverlayToast** `from → to (reason)` |
| `LlmFailed` | nœud LLM en rouge (Mission B fan-out) |
| `SlmCircuitOpened` | **OverlayToast** ⚡ « circuit {robot} ouvert » |
| `DrafterBypassed` | overlay « drafter sauté → verifier » |
| `DraftRejectedAsNoise` | overlay « bruit rejeté : {signal} » + extrait |

---

## 10. Plan de tests

### 10.1 Backend (`efe-preview-server`)
- **T1 — SSE contract** : `POST /v1/missions/a` (Coordinator mock `MockLlmProvider`)
  → le stream contient, dans l'ordre, `SWARM_STARTED` … `SWARM_FINISHED`, chaque
  frame `data: {json valide}\n\n`.
- **T2 — Pas de course (D3)** : assert que `SWARM_STARTED` est **toujours** le 1er
  event reçu (le helper `run_mission_a_streamed` s'abonne avant de spawn).
- **T3 — Schema non-drift** : `GET /v1/schema` → `event_schema_version == "1.0.0"` +
  24 variants ; un test Rust assert l'exhaustivité de `tag()`.
- **T4 — Slow consumer** : un client qui ne lit pas ne bloque pas la mission (le bus
  drop + `dropped_count` > 0 ; la mission `join` réussit quand même).
- **T5 — Savings report** : `GET /v1/savings-report` → JSON parseable, contient
  `savings_ratio_microcents`.
- **T6 — CORS/auth** : preflight OK pour `localhost:3000` ; sans bearer (si
  `EFE_PREVIEW_TOKEN` posé) → 401.

### 10.2 Frontend (`preview-lab`)
- **T7 — Type contract** : test qui charge `GET /v1/schema` et assert que chaque tag
  serveur a un variant dans `events.ts` (et inversement).
- **T8 — Reducer** : events factices → état dérivé attendu (robots, tokens, metrics).
- **T9 (optionnel) — Playwright smoke** : taper une requête (backend mock) →
  pipeline se remplit → `SavingsGauge` affiche un ratio > 0.

---

## 11. Phasage (Phase 4 — 6 sprints)

| Sprint | Contenu | Livrable |
|---|---|---|
| **P4.S1** | `run_mission_*_streamed` + `swarm_id: Option<Uuid>` (D3) + tests T2 | abonnement sans course |
| **P4.S2** | `efe-preview-server` : axum, `POST /v1/missions/a`, SSE, `/healthz`, `/schema` (T1,T3,T4) | bridge minimal |
| **P4.S3** | DTOs B + Speculative + `/v1/savings-report` + `/metrics` réexposé (T5) | API complète |
| **P4.S4** | `preview-lab` Next.js : scène **Live Mission** (Pipeline, TokenStream, SavingsGauge) + `useMissionStream` (T7,T8) | la scène-titre |
| **P4.S5** | scènes **Savings Report** + **A/B Lab** (`POST /v1/ab/run`) | comparaisons |
| **P4.S6** | scène **Resilience Theater** + `ServeDir` statique + packaging (Docker compose dev) + tag `phase-4-preview-lab` | clôture v0.3.0 |

---

## 12. Risques & mitigations

| Risque | Impact | Mitigation |
|---|---|---|
| Proxy/CDN bufferise le SSE | events en rafale, pas live | `X-Accel-Buffering: no` + `Cache-Control: no-cache` ; `Heartbeat` 15 s |
| Consumer lent → drop d'events | UI incohérente | bus déjà borné+drop ; UI affiche `dropped_count` (bandeau « events perdus ») ; cap 4K large |
| Course `swarm_id` (D3) | rate `SwarmStarted` | helper `run_mission_*_streamed` s'abonne **avant** spawn (T2) |
| Schema drift TS↔Rust | UI casse en silence | `GET /v1/schema` + assert version au boot (D5, T7) |
| Clé LLM exposée au front | fuite secret | clés **uniquement** côté serveur ; le front ne voit que des events (D4) |
| axum = surface d'attaque | sécu | crate isolée, bind localhost, bearer optionnel, pas en hot path prod |
| Coût $ (le Lab tape de vrais LLM) | facture | mode `EFE_PREVIEW_MOCK=1` → `MockLlmProvider` (démo gratuite) ; sinon clés réelles |

---

## 13. Critères d'acceptation

- ✅ `cargo clippy --all-targets` (workspace) : 0 erreur, `efe-preview-server` inclus.
- ✅ T1–T6 backend verts (avec `MockLlmProvider`, offline).
- ✅ `POST /v1/missions/a` (mock) streame `SWARM_STARTED`…`SWARM_FINISHED`, frames SSE valides.
- ✅ Front Live Mission : pipeline se remplit, tokens streament, `SavingsGauge` affiche le triplet.
- ✅ Savings scène lit le v2 et montre le hero « R2 +0 % vs R3 −27 % ».
- ✅ Schema guard actif (versions concordent).
- ✅ `efe-coordinator` reste lean (aucune dép HTTP ajoutée ; tout dans `efe-preview-server`).
- ✅ Zéro nouvelle dépendance **au workspace** (axum/tower-http déjà présents/vetted).

---

## 14. Hors-scope (Phase 4+)

Auth/multi-tenant prod (via `efe-gateway`), persistance d'historique, `NatsBus`
multi-réplica, billing UI, intégration agents (direction B), déploiement managé.

---

## 15. Annexes

### A. Variables d'environnement
`EFE_PREVIEW_ADDR` (défaut `127.0.0.1:8700`), `EFE_PREVIEW_TOKEN` (bearer optionnel),
`EFE_PREVIEW_MOCK` (1 = MockLlmProvider), + les clés LLM (`ANTHROPIC_API_KEY`, …) en
mode réel, + `EFE_PREVIEW_CORS_ORIGIN` (défaut `http://localhost:3000`).

### B. Layout disque
```
crates/efe-preview-server/   (NEW : axum bridge ; bin `efe-preview-server`)
preview-lab/                 (NEW : Next.js app, hors workspace cargo)
```

### C. Pourquoi une crate séparée et pas un binaire dans efe-coordinator
`efe-coordinator` est une **lib** consommée par `efe-server`, les tests, etc. Y
ajouter axum/tower-http alourdirait tous ses conscommateurs et violerait son
contrat « `metrics` = `tokio/net` only ». Une crate binaire dédiée garde la frontière
nette : le moteur reste pur, le Lab est un satellite.

### D. Lien avec les autres directions produit
Le Preview Lab est la **console** : la scène A/B sert la direction B (optim. agents),
la couche `POST /v1/missions/*` est l'embryon de la direction A (proxy) — il suffira
d'ajouter un `POST /v1/chat/completions` (compat OpenAI) qui mappe vers une mission.

_Fin architecture Phase 4 — Preview Lab (frontend SSE)._
