# ADR-SWARM-010 : Le dashboard live rend l'essaim **visible** — sans réécrire un agent

- **Status** : Accepted · **Date** : 2026-06-09
- **Phase** : 5 (Productisation) · **Inc.** : 3 (dashboard live)
- **Related** : ADR-SWARM-006 (la provenance par-agent est le moat) ; ADR-SWARM-009 (l'endpoint
  `verify` est le produit) ; `docs/architecture/EFE_SWARM_PHASE5_INC3_DASHBOARD_LIVE_ARCHITECTURE.md` ;
  crates `efe-swarm-dashboard` (cœur) + `efe-swarm-gateway` (SSE + UI) ; seam `efe-blackboard::EventSink`

## Contexte

EFE, c'est *« des agents à très peu de paramètres, en grande quantité, qui se passent des
informations en parallèle »*. C'est **vrai** (Phases 1-4 : substrat, topologies, consensus, provenance)
mais **invisible** : un run keyless dure ~ms, un run self-host quelques secondes, et tout le trafic
inter-agent (écritures `/cand`, `/ballot`, `/out`, messages de bus, scellement, brûlure de budget)
disparaît dans un `SwarmRun` figé. Inc.2 a exposé le **moat** (re-preuve HTTP) ; il manquait la
transparence **opérationnelle** : *voir* l'essaim penser, en direct.

La question d'architecture : *comment diffuser chaque micro-événement d'un run vers une UI, en temps
réel, **sans** (a) réécrire un seul agent ou une topologie, (b) coupler le SDK à un transport HTTP,
(c) faire croire que ce flux est une source d'audit ?*

## Décision

**Un `DashboardBus` tape les seams substrat déjà existants via un hook générique `SwarmObserver`,
et la passerelle le diffuse en SSE vers une UI vanilla à 4 panneaux.**

1. **Taps sur les seams existants — zéro agent réécrit** (la discipline de la provenance, Phase 4) :
   - `WriteSink` (uniforme, Phase 4) **tee** → writes stigmergiques (`/cand`, `/ballot`, `/out`).
   - `EventSink` (**neuf**, pendant de `WriteSink` pour le bus) tapé au seam **`publish`** du
     `TopicRouter` — **pas** `subscribe_prefix`, qui est racé (ne capte que les topics déjà créés).
   - `Meter` (SDK) → chaque appel facturé porte la **vraie sortie** ⇒ candidats + ballots réels + budget.
   - `ProvenanceMetrics`/`SubstrateMetrics` → tick de jauges.
2. **Hook générique `SwarmObserver`** dans le SDK : le SDK **ignore** le mot « dashboard » ; il expose
   un trait (write_sink/event_tap/on_run_start/on_agent_call/on_seal/on_finish). `with_observer` est
   **additif et non-breaking** : sans observateur, le run est byte-identique à Inc.1/2.
3. **`DashboardBus` = rejeu + live** : ring borné (drop-oldest) **plus** `broadcast`. Un abonné tardif
   **rejoue** le passé (indispensable : le run finit avant que le navigateur se connecte) puis streame
   le tail. Le **mode job** (`POST …?stream=true` → `202`) exécute le run observé en tâche détachée.
4. **SSE + UI vanilla** (gateway) : `GET …/runs/:id/events` (rejeu+live, dédup par `seq`),
   `GET …/events` (firehose tenant), `GET /dashboard` (HTML/JS embarqué `include_str!`, `EventSource`
   natif, **zéro CDN/framework**, offline) — 4 panneaux : trafic inter-agent · consensus · provenance · budget.

**Honnêteté assumée** : le flux est **best-effort/lossy** (`broadcast` + ring). Le dashboard est un
**flux d'observation**, **pas** une source d'audit. La vérité durable reste la chaîne de provenance
(`verify_swarm`, ADR-009). On ne confond jamais le *montrer* et le *prouver*.

## Évidence (Inc.3 — tests, run keyless réel via HTTP)

| critère | vérification | verdict |
|---|---|---|
| run observé → séquence d'events attendue (start→write→candidate→ballot→budget→sealed→finish) | `efe-swarm-dashboard::observed_consensus_run_emits_expected_events` | ✓ |
| **ballots à scores réels** (depuis le `Meter`, pas inventés) | `ballot_scores_are_real_from_meter` | ✓ |
| ring borné (drop-oldest) + rejeu déterministe | `bus::ring_drops_oldest_when_full` | ✓ |
| `DashboardRegistry` **tenant-scopé** + borné | `registry::get_is_tenant_scoped`, `bounded_drops_oldest` | ✓ |
| job `?stream=true` → `202`, SSE **rejeu+live** (toutes familles de panneau), puis persiste + `verify` re-prouve | `gateway::job_streams_live_events_then_persists_and_verifies` | ✓ |
| UI servie sans dépendance externe (offline) | `gateway::dashboard_html_is_served` | ✓ |
| run inconnu → 404 ; isolation tenant | `events_of_unknown_run_is_404` | ✓ |
| **substrat câblé** (Inc.2) toujours réel (`swarm_blackboard_writes_total > 0`) | `gateway::metrics_endpoint_exposes_real_provenance_and_substrate` | ✓ |

**Non-régression** : efe-blackboard 98 (+22 intégration), topology 60, sdk 21, dashboard 12, gateway 22
(+1 ignored self-host), doctests OK — **0 warning**. Les changements (topology/sdk/blackboard) sont
**additifs** (champs optionnels, méthodes déléguées) : Inc.1/Inc.2 inchangés.

## Conséquences

- **Le livrable** : on **voit** les petits agents se passer l'info en parallèle — writes, messages,
  ballots à scores réels, scellement, budget — en direct, 4 panneaux ; puis on **re-prouve** le run
  (ADR-009). La thèse EFE rendue **visible ET vérifiable**.
- **Seam réutilisable** : `EventSink` (publish-tap) sert au-delà du dashboard (journaux, debug live).
  `SwarmObserver` est générique — un test, un exportateur, un autre frontend peuvent s'y brancher.
- **Différé (Inc.3.x / Inc.4)** : firehose `/events` ne montre que les runs **actifs à la connexion**
  (un flux de « nouveaux runs » viendrait ensuite) ; `AuthCtx` OIDC réel (dev : `Caller` par en-tête) ;
  TTL/éviction fine du registry ; latence de propagation par message (`AgentMessage.latency_ms`).
- **Risque tranché** : « le live est-il pris pour une preuve ? » → **non** : best-effort explicite,
  `verify_swarm` reste l'autorité ; le seam d'événements est **non-racé** (publish, pas subscribe).
