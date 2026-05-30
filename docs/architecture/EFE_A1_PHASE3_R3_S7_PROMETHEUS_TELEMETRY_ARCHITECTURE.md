# EFE A1 — Phase 3 R3.S7 — Prometheus telemetry

> **Statut** : architecture (à implémenter)
> **Sprint** : Phase 3 R3.S7
> **Priorité roadmap** : 🟢 nice-to-have
> **Pré-requis** : R3.S1 (cost µ¢), R3.S2 (SlmCircuitBreaker state), S8-R2
> (FailureMode), R3.S5 (tenant_id — à NE PAS exposer en label)
> **Fichiers cible** : `crates/efe-coordinator/src/metering.rs` (NEW),
> `src/metrics_server.rs` (NEW, feature-gated), `Cargo.toml` (feature
> `metrics`), `lib.rs`, `coordinator.rs`, `workflows/{mission_a,mission_b,
> speculative}.rs`, `tests/metering.rs` (NEW)

---

## 0. Résumé exécutif

R2 émet des `StreamEvent` SSE (19 variants) consommables par une UI, mais
**aucune métrique au format Prometheus** : impossible de brancher Grafana /
Alertmanager sur EFE en prod (taux d'erreur, coût cumulé, p99 latence, état
des circuits, savings…).

R3.S7 livre une couche de télémétrie Prometheus :

1. **`MetricsRegistry`** (`src/metering.rs`) — compteurs/histogrammes/gauges
   en atomiques lock-free, alimentés par un **hook lossless** au terme de
   chaque mission, + rendu au **format d'exposition Prometheus** fait main.
2. **Serveur `/metrics`** (`src/metrics_server.rs`) — un serveur HTTP/1.1
   minimal sur `tokio::net::TcpListener` (même pattern que le mock
   `local_slm.rs`), **gated** derrière une feature `metrics` (qui n'active que
   `tokio/net`).
3. **Hook RAII** dans les 3 workflows — capture **succès ET erreur** (les
   erreurs remontent via `?`) en 2 lignes par workflow, sans restructuration.

**Principes directeurs (hérités R3.S3→S6)** :
- **Zéro nouvelle dépendance** : pas de crate `prometheus`/`hyper`/`axum` — le
  format d'exposition est trivial à écrire, le serveur réutilise le précédent
  `TcpListener` de `local_slm.rs`.
- **Zéro dead config** : chaque métrique a une source de données réelle ; les
  items de la roadmap sans source live (`quality_pass_rate{mode="baseline"}`)
  sont **reframés** en counters idiomatiques, pas exposés comme gauges morts.
- **Cardinalité bornée** : `tenant_id` (R3.S5, non borné) n'est **jamais** un
  label Prometheus (anti explosion de cardinalité — décision §4.3).
- **Backward-compat** : `metrics: None` ⇒ overhead nul, comportement R3.S6
  inchangé.

**Critère central** : `curl http://localhost:9090/metrics` retourne un format
Prometheus valide exposant les familles de métriques R2, avec overhead
< 1 ms/mission.

---

## 1. Scope / Non-scope

### 1.1 Dans le scope

| # | Livrable | Détail |
|---|---|---|
| 1 | `MetricsRegistry` (counters/histogram/gauges atomiques) | `src/metering.rs` |
| 2 | Rendu format d'exposition Prometheus (texte) fait main | `render_prometheus()` |
| 3 | Hook RAII lossless dans les 3 workflows (succès + erreur) | `MissionMetricGuard` |
| 4 | `Coordinator.metrics: Option<Arc<MetricsRegistry>>` + builder | `coordinator.rs` |
| 5 | Serveur HTTP `/metrics` (TcpListener, feature `metrics`) | `src/metrics_server.rs` |
| 6 | Gauge circuit-state = scrape-time pull (état courant) | server + clients |
| 7 | 8+ tests (registry, render, intégration, HTTP endpoint) | `tests/metering.rs` |

### 1.2 Hors scope (déféré / reframé, §17)

- **`quality_pass_rate_gauge{mode="baseline"}`** — `baseline` est un mode de
  **benchmark offline** (`RunMode::Baseline`, savings_report.rs), jamais exécuté
  en live → aucune source. **Reframé** en counters `efe_verdicts_total{verdict}`
  (§4.4) d'où PromQL dérive un pass-rate.
- **Per-provider cost breakdown en Mission B** — le coût B est un agrégat de
  fan-out (somme N LLMs) sans ventilation par participant disponible (§4.5).
- **Push gateway / OTLP / exemplars** — S7 expose un endpoint *pull* simple.
- **TLS / auth sur `/metrics`** — bind localhost ; le scraping sécurisé est
  une préoccupation infra (reverse proxy), hors scope.
- **Métriques du bus/DAG/blackboard internes** — S7 cible les métriques
  métier (missions, coût, latence, résilience, savings).

### 1.3 Invariant de non-régression

> `metrics: None` (défaut) ⇒ aucune allocation, aucun atomique touché, le hook
> RAII se réduit à un test `Option::is_none` ⇒ comportement R3.S6
> byte-pour-byte. Feature `metrics` off ⇒ `metrics_server.rs` non compilé,
> `tokio/net` non activé.

---

## 2. Problème

R2/R3 produisent une mine de données par mission (`ChewingMetrics` : coût µ¢,
latences par phase, failure_mode, savings ; `FactCheck` verdicts ;
`SlmCircuitBreaker` état) mais elles ne sont **observables qu'au fil de l'eau**
via SSE, par un consommateur attaché. Pour de la **prod**, il faut :

- un endpoint **pull** standard (`/metrics`) que Prometheus scrape toutes les
  N secondes ;
- des **séries temporelles** agrégées (counters monotones, histogrammes) pour
  calculer taux d'erreur, p50/p99, coût/heure, savings cumulés, en PromQL ;
- des **gauges** d'état instantané (circuits ouverts).

Aujourd'hui : rien. Aucun moyen de brancher Grafana/Alertmanager.

---

## 3. État actuel du code (inventaire ancré)

| Élément | Emplacement | Constat |
|---|---|---|
| `ChewingMetrics` (coût µ¢ via response, latences/phase, failure_mode, savings_µ¢, selected_llm, tenant_id) | `src/metrics.rs:43-216` | source riche par mission |
| `MissionAResponse.cost_microcents` + `llm_used` + `verdicts` | `request.rs:128-181` | coût par provider attribuable |
| `MissionBResponse.total_cost_microcents` + `winner_llm` + `verdicts` | `request.rs:223-294` | coût **agrégé** fan-out |
| `SpeculativeResponse.cost_microcents` | `request.rs:~495` | coût mission spec |
| `FactCheck.verdict` (`"SUPPORTED"`/`"UNSUPPORTED"`/`"CONTRADICTED"`/`"INSUFFICIENT_EVIDENCE"`) | `request.rs:500-511` | source qualité **live** |
| `RunMode` (Baseline/S4Chewing/S6Routing/Full/Speculative) + `RunRecord.passed_quality` + `ModeSummary.quality_pass_rate` | `savings_report.rs:45-281` | qualité **offline** (benchmark) |
| `FailureMode` (None/SlmDown/LlmFallbackTriggered/VoteDegraded/DrafterBypassed/Composite) | `failure.rs:40-70` | source résilience |
| `SlmCircuitBreaker::state() -> SlmCircuitState` (Closed/Open/HalfOpen) | `efe-nano-app-coordinator` | gauge scrape-time |
| `coord.clients.slm_circuits: HashMap<String, SlmCircuitBreaker>` | `coordinator.rs:50` | itérable au scrape |
| `StreamEvent::SwarmFinished` (succès) / `SwarmFailed` (erreur) | `events/stream.rs` | **`SwarmFailed` émis SEULEMENT dans `mission_b.rs`** |
| `run_mission_a/b`/`run_speculative -> CoordResult<…>` | `workflows/*` | erreurs via `?` (early return), 1 seul `Ok(resp)` final |
| `EventBus::subscribe` (mpsc, lossy → `SubscriberLag`) | `events/bus.rs:96` | bus = lossy |
| `Coordinator{config,clients,bridge,blackboard,bus,dag,vote,policies}` | `coordinator.rs:180-200` | ajouter `metrics` |
| Précédent serveur HTTP in-process | `src/llm/providers/local_slm.rs` (tests) | `TcpListener::bind("127.0.0.1:0")` + HTTP/1.1 manuel + `Content-Length` + `Connection: close` |
| Dépendances | `Cargo.toml:30-52` | **pas de hyper/prometheus/axum** ; tokio **sans `net`** |
| tokio features | `Cargo.toml:31` | `["sync","time","rt-multi-thread","macros","signal"]` (pas `net`) |

**Observation décisive #1** : `SwarmFailed` n'est émis que par Mission B → un
subscriber bus ne verrait pas les erreurs de Mission A / Speculative. Couplé au
fait que le bus est lossy (mpsc + drop sous lag), **le bus est inadapté pour
des compteurs d'erreur fiables** → §4.1 retient le hook.

**Observation décisive #2** : le précédent `local_slm.rs` prouve qu'on sait
servir du HTTP/1.1 in-process sur `TcpListener` → §4.2 sert `/metrics` sans
nouvelle dépendance.

---

## 4. Décisions chirurgicales

### 4.1 Hook RAII lossless (vs bus subscription)

| Critère | Bus subscriber | **Hook RAII (retenu)** |
|---|---|---|
| Touche les workflows | non | oui (2 lignes/workflow) |
| Capture les erreurs | **non** (`SwarmFailed` seulement en B) | **oui** (via `Drop` sur `?`) |
| Pertes sous charge | **oui** (mpsc lag → drop) | **non** (atomique direct) |
| Verdicts/coût détaillés | partiel (PreChewingMetrics) | complet (réponse en main) |

Le bus séduit (zéro touche, cf. R3.S6) mais il **perd les erreurs** (A/Spec
n'émettent pas `SwarmFailed`) et **perd sous lag** — inacceptable pour des
compteurs `missions_total{outcome="error"}` / alerting. On retient le **hook**.

**Pattern RAII (`MissionMetricGuard`)** — capture les deux issues sans
restructurer (pas de rename `_inner`) :

```rust
pub async fn run_mission_a(coord, req) -> CoordResult<MissionAResponse> {
    let started = Instant::now();
    // Armé : si la fn `?`-retourne une Err, Drop enregistre outcome="error".
    let mut guard = MissionMetricGuard::arm(coord.metrics.clone(), MissionTag::MissionA, started);
    // … corps existant inchangé (peut ? early) …
    let resp = /* … Ok path … */;
    // Désarme + enregistre les métriques détaillées (coût, latences, verdicts).
    guard.record_success(&MissionAMetricView::from(&resp));
    Ok(resp)
}
```

- `Drop` est **synchrone** : l'enregistrement n'écrit que des atomiques (pas
  d'async requis). Sur `?`/panic → `outcome="error"` + latence.
- `record_success` désarme le guard (pas de double comptage) et enregistre les
  métriques détaillées extraites d'une *vue* par type (`MissionAMetricView`,
  etc.) qui sait où trouver `cost_microcents`, `verdicts`, `chewing_metrics`.
- `coord.metrics` est `Option<Arc<…>>` ; `arm(None, …)` ⇒ guard inerte (aucun
  atomique, invariant §1.3).

### 4.2 Zéro nouvelle dépendance

- **Rendu Prometheus fait main** : le [format d'exposition texte](#9-format)
  est `# HELP`/`# TYPE` + lignes `name{labels} value`. Trivial à formatter ;
  aucun besoin du crate `prometheus`.
- **Serveur HTTP/1.1 minimal** : `tokio::net::TcpListener` (précédent
  `local_slm.rs`), parse la request-line, route `GET /metrics` → 200
  `text/plain; version=0.0.4`, sinon 404. Pas de hyper/axum.
- **Feature `metrics = ["tokio/net"]`** : n'active `tokio/net` (requis pour
  `TcpListener`) **que** quand la télémétrie HTTP est compilée. Le
  `MetricsRegistry` + le rendu + le hook sont **toujours** compilés (atomiques
  + formatage, zéro dep) — seul `metrics_server.rs` est gated. Ainsi, brancher
  la télémétrie en bibliothèque (sans serveur) ne coûte aucune feature.

### 4.3 Cardinalité : `tenant_id` n'est JAMAIS un label

R3.S5 a introduit `tenant_id: Option<Uuid>` (non borné). L'exposer en label
Prometheus = **explosion de cardinalité** (1 série × tenants × autres labels)
→ OOM du TSDB. **Règle dure** : aucun label de cardinalité non bornée. Labels
autorisés (tous bornés) : `type` (3), `outcome` (2), `provider` (≈4),
`phase` (4), `kind` failure (≈6), `robot` (3), `state` (3), `verdict` (4),
`mode` (2). Le `tenant_id` reste dans l'audit chain (R3.S5), **pas** dans
Prometheus.

### 4.4 Rates → counters (reframe de `quality_pass_rate_gauge`)

La roadmap propose `efe_quality_pass_rate_gauge{mode="full"|"baseline"}`. Deux
problèmes : (1) un **rate pré-calculé en gauge** est un anti-pattern Prometheus
(les rates se dérivent de counters via PromQL, robustes aux resets) ; (2)
`baseline` n'a **pas de source live** (benchmark offline). **Reframe** :

```
efe_verdicts_total{verdict="SUPPORTED"|"UNSUPPORTED"|"CONTRADICTED"|"INSUFFICIENT_EVIDENCE"}
```

→ source = `FactCheck.verdict` des `verdicts` de chaque mission (live). PromQL :
`sum(rate(efe_verdicts_total{verdict="SUPPORTED"}[5m])) / sum(rate(efe_verdicts_total[5m]))`
= pass-rate, dérivé proprement. Le `mode="baseline"` est **descopé** (documenté
§17). C'est l'honnêteté anti-dead-config (cf. R3.S6 `EFE_CHAOS_RATE`).

### 4.5 Attribution du coût Mission B (agrégat fan-out)

`MissionBResponse.total_cost_microcents` est la **somme** des N LLMs ; on n'a
pas le coût par participant. Pour `efe_cost_microcents_total{provider}` :
- Mission A : `{provider=llm_used}` += `cost_microcents` (exact).
- Speculative : `{provider=selected_llm}` += `cost_microcents` (exact).
- Mission B : `{provider=winner_llm}` += `total_cost_microcents`
  **(approximation documentée** : le coût agrégé du fan-out est étiqueté par le
  gagnant du vote ; un futur sprint pourra ventiler si les coûts par
  participant deviennent disponibles). Tracé tel quel + `# HELP` mentionnant
  l'agrégation.

### 4.6 Histogramme fait main (buckets fixes)

Pas de crate histogram. `efe_latency_ms` = histogramme Prometheus classique :
buckets cumulatifs `le` + `_sum` + `_count`. Buckets fixes adaptés (ms) :
`[5, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000, 10000, +Inf]`. Chaque
`observe(phase, ms)` incrémente atomiquement les buckets `>= ms` (cumulatif) +
sum + count. Labels : `phase ∈ {analyzer, compressor, verifier, llm}` (drafter
spec → `llm` ; router négligeable, exclu).

### 4.7 Gauge circuit-state = scrape-time pull

Les circuits changent d'état de façon asynchrone (ouverture sur échecs). Un
gauge Prometheus reflète l'**état courant** → on le lit **au scrape**, pas via
hook. Le serveur, qui détient `Arc<CoordinatorClients>`, itère
`slm_circuits` et émet `efe_slm_circuit_state{robot, state} 1` pour l'état
actif (0 pour les autres), via `SlmCircuitBreaker::state()`.

---

## 5. Modèle de données (`src/metering.rs`)

```rust
// SPDX-License-Identifier: UNLICENSED
//! R3.S7 — Registre de métriques Prometheus (lock-free) + rendu texte.

use std::sync::atomic::{AtomicU64, Ordering};
use std::collections::HashMap;

/// Compteur monotone (u64 atomique).
#[derive(Debug, Default)]
struct Counter(AtomicU64);
impl Counter {
    fn add(&self, v: u64) { self.0.fetch_add(v, Ordering::Relaxed); }
    fn get(&self) -> u64 { self.0.load(Ordering::Relaxed) }
}

/// Gauge f64 (stocké en bits u64 atomiques — pas de dépendance).
#[derive(Debug, Default)]
struct GaugeF64(AtomicU64);
impl GaugeF64 {
    fn set(&self, v: f64) { self.0.store(v.to_bits(), Ordering::Relaxed); }
    fn get(&self) -> f64 { f64::from_bits(self.0.load(Ordering::Relaxed)) }
}

/// Histogramme à buckets fixes cumulatifs (ms).
pub const LATENCY_BUCKETS_MS: &[u64] =
    &[5, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000, 10000];
#[derive(Debug)]
struct Histogram {
    buckets: Vec<AtomicU64>,  // len = LATENCY_BUCKETS_MS.len()  (cumulatif au rendu)
    sum: AtomicU64,
    count: AtomicU64,
}
impl Histogram {
    fn observe(&self, v_ms: u64) {
        // incrémente le 1er bucket dont la borne >= v_ms (+ le rendu cumule)
        for (i, &b) in LATENCY_BUCKETS_MS.iter().enumerate() {
            if v_ms <= b { self.buckets[i].fetch_add(1, Ordering::Relaxed); break; }
        }
        self.sum.fetch_add(v_ms, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
    }
}

/// Registre central. Toutes les familles à labels bornés sont pré-allouées
/// dans des `HashMap<&'static str / enum, _>` figées (clés connues) → pas
/// d'allocation au hot path, juste un lookup + fetch_add.
#[derive(Debug)]
pub struct MetricsRegistry {
    // efe_missions_total{type, outcome}
    missions: HashMap<(MissionType, Outcome), Counter>,
    // efe_cost_microcents_total{provider}
    cost_microcents: DashMap<String, Counter>,        // provider borné (≈4) mais dynamique
    // efe_latency_ms{phase}
    latency: HashMap<Phase, Histogram>,
    // efe_failure_mode_total{kind}
    failure_mode: HashMap<FailureKind, Counter>,
    // efe_savings_microcents_total{mode}
    savings_microcents: HashMap<SavingsMode, Counter>, // full | speculative
    // efe_verdicts_total{verdict}
    verdicts: HashMap<Verdict, Counter>,
}

impl MetricsRegistry {
    #[must_use] pub fn new() -> Self { /* pré-alloue toutes les clés */ }

    /// Hook lossless appelé par le guard. `cost` = coût de la mission ;
    /// `provider` = llm facturé (A=llm_used, Spec=selected, B=winner).
    pub fn record_mission(&self, m: &MissionRecord) {
        self.missions[&(m.ty, m.outcome)].add(1);
        if m.outcome == Outcome::Success {
            self.cost_microcents.entry(m.provider.clone()).or_default().add(m.cost_microcents);
            for (phase, ms) in m.phase_latencies() { self.latency[&phase].observe(ms); }
            if let Some(mode) = m.savings_mode { self.savings_microcents[&mode].add(m.savings_microcents); }
            for v in &m.verdicts { self.verdicts[v].add(1); }
            if let Some(k) = m.failure_kind { self.failure_mode[&k].add(1); }
        }
    }

    /// Rendu format d'exposition Prometheus. `circuits` = snapshot scrape-time
    /// (robot, état) fourni par le serveur (§4.7).
    #[must_use]
    pub fn render_prometheus(&self, circuits: &[(String, SlmCircuitState)]) -> String { /* §9 */ }
}
```

**Notes**
- `cost_microcents` utilise `DashMap` (déjà dépendance) car `provider` est une
  `String` dynamique (bien que bornée en pratique) → évite d'énumérer les ids
  à la compilation.
- Les autres familles utilisent des clés **enum** figées (pré-allouées) → hot
  path = `HashMap::get` + `fetch_add`, O(1), sans allocation.
- `failure_kind`/`savings_mode`/`verdict` sont des enums internes mappés depuis
  `FailureMode`, le type de mission, et `FactCheck.verdict`.

`MissionRecord` est la **vue normalisée** construite par le guard depuis chaque
réponse (découple metering des types `request.rs`).

---

## 6. Catalogue des métriques

| Métrique | Type | Labels (bornés) | Source | Quand |
|---|---|---|---|---|
| `efe_missions_total` | counter | `type`, `outcome` | guard (succès/Drop) | fin mission |
| `efe_cost_microcents_total` | counter | `provider` | `*Response.cost_microcents` | succès |
| `efe_latency_ms` (histogram) | histogram | `phase` | `ChewingMetrics.*_latency_ms` | succès |
| `efe_failure_mode_total` | counter | `kind` | `ChewingMetrics.failure_mode` | succès dégradé |
| `efe_savings_microcents_total` | counter | `mode` (full\|speculative) | `ChewingMetrics.savings/spec_savings_microcents` | succès |
| `efe_verdicts_total` | counter | `verdict` | `*Response.verdicts[].verdict` | succès |
| `efe_slm_circuit_state` | gauge | `robot`, `state` | `SlmCircuitBreaker::state()` | **scrape** |

7 familles couvrant les 7 lignes de la roadmap (avec `quality_pass_rate`
reframé en `verdicts_total`, §4.4). Toutes cardinalités bornées (§4.3).

---

## 7. Intégration

### 7.1 Coordinator + builder

```rust
// coordinator.rs — Coordinator
pub metrics: Option<Arc<MetricsRegistry>>,   // None ⇒ télémétrie off (défaut)

// CoordinatorBuilder
fn metrics: Option<Arc<MetricsRegistry>>
#[must_use] pub fn metrics(mut self, reg: Arc<MetricsRegistry>) -> Self { self.metrics = Some(reg); self }
// build(): metrics: self.metrics ;  for_tests(): metrics: None
```

### 7.2 Hook RAII dans les 3 workflows

Chaque `run_*` : **2 lignes** (arm au début, record_success avant `Ok`).
`MissionMetricGuard` (dans metering.rs) :

```rust
pub struct MissionMetricGuard {
    reg: Option<Arc<MetricsRegistry>>,
    ty: MissionType,
    started: Instant,
    armed: bool,   // true → Drop enregistre une erreur
}
impl MissionMetricGuard {
    pub fn arm(reg: Option<Arc<MetricsRegistry>>, ty: MissionType, started: Instant) -> Self { … }
    /// Désarme + enregistre la mission réussie (vue extraite de la réponse).
    pub fn record_success(&mut self, view: &MissionRecord) {
        self.armed = false;
        if let Some(r) = &self.reg { r.record_mission(view); }
    }
}
impl Drop for MissionMetricGuard {
    fn drop(&mut self) {
        if self.armed {
            if let Some(r) = &self.reg {
                r.record_mission(&MissionRecord::error(self.ty, self.started.elapsed()));
            }
        }
    }
}
```

Conversion réponse → `MissionRecord` : un `From<&MissionAResponse>` etc. (ou
`MissionRecord::from_mission_a(&resp, elapsed)`) qui extrait `cost_microcents`,
`llm_used`/`winner_llm`/`selected_llm`, `verdicts`, `chewing_metrics`
(latences, failure_mode, savings, verifier_skipped→savings_mode).

### 7.3 Serveur HTTP `/metrics` (feature `metrics`)

```rust
// src/metrics_server.rs  (#[cfg(feature = "metrics")])
pub async fn serve_metrics(
    addr: SocketAddr,                     // défaut 127.0.0.1:9090
    registry: Arc<MetricsRegistry>,
    clients: Arc<CoordinatorClients>,     // pour le snapshot circuit (§4.7)
) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    loop {
        let (mut sock, _) = listener.accept().await?;
        let reg = registry.clone(); let cli = clients.clone();
        tokio::spawn(async move {
            // lire la request-line (jusqu'à "\r\n\r\n"), router :
            //   GET /metrics → 200 text/plain; version=0.0.4 + body
            //   sinon        → 404
            let circuits = snapshot_circuits(&cli);      // (robot, state)
            let body = reg.render_prometheus(&circuits);
            // écrire HTTP/1.1 200 + Content-Length + Connection: close + body
        });
    }
}
```

Pattern identique au mock `local_slm.rs` (bind, accept, write HTTP/1.1 manuel).

---

## 8. Serveur : détails

- **Bind** : `127.0.0.1:9090` par défaut (localhost — pas d'exposition réseau
  directe ; le scraping passe par un sidecar/reverse proxy en prod, §12).
- **Routing** : seulement `GET /metrics`. `GET /healthz` → `200 OK` (liveness).
  Toute autre route → `404`.
- **Robustesse** : 1 task/connexion, `Connection: close`, lecture bornée de la
  request-line (cap à 8 KiB pour éviter un slowloris trivial). Une connexion
  qui échoue ne tue pas la boucle `accept`.
- **Lancement** : `tokio::spawn(serve_metrics(addr, registry, clients))` par
  l'app hôte. EFE-coordinator fournit la fonction ; l'orchestration (quand
  démarrer) appartient au binaire serveur.

---

## 9. Format d'exposition Prometheus

Texte `text/plain; version=0.0.4`. Exemple rendu :

```
# HELP efe_missions_total Total missions by type and outcome.
# TYPE efe_missions_total counter
efe_missions_total{type="A",outcome="success"} 1240
efe_missions_total{type="A",outcome="error"} 7
# HELP efe_cost_microcents_total Cumulative cost in microcents by billed provider (Mission B = fan-out aggregate labeled by vote winner).
# TYPE efe_cost_microcents_total counter
efe_cost_microcents_total{provider="claude_claude-sonnet-4-6"} 558000
# HELP efe_latency_ms Per-phase latency distribution (ms).
# TYPE efe_latency_ms histogram
efe_latency_ms_bucket{phase="llm",le="5"} 0
efe_latency_ms_bucket{phase="llm",le="500"} 980
efe_latency_ms_bucket{phase="llm",le="+Inf"} 1240
efe_latency_ms_sum{phase="llm"} 743210
efe_latency_ms_count{phase="llm"} 1240
# HELP efe_slm_circuit_state Current SLM circuit state (1=active).
# TYPE efe_slm_circuit_state gauge
efe_slm_circuit_state{robot="analyzer",state="closed"} 1
efe_slm_circuit_state{robot="analyzer",state="open"} 0
```

**Règles** : labels échappés (`\`, `"`, `\n`), histogramme cumulatif avec
`le="+Inf"` = `_count`, ordre stable (HELP/TYPE une fois par famille). Un test
valide la conformité (parse-back minimal, §11).

---

## 10. Overhead (< 1 ms/mission)

- Hook : 1× `HashMap::get` (clé enum) + quelques `fetch_add(Relaxed)` +
  itération bornée des phases (≤ 4) → **ordre de la centaine de ns**.
- `Relaxed` suffit (pas de synchro inter-métriques requise ; chaque compteur
  est indépendant).
- Aucune allocation au hot path (clés enum pré-allouées ; `cost_microcents`
  DashMap n'alloue qu'au 1er provider vu, ensuite lookup).
- Le **rendu** (formatage texte) n'a lieu qu'au **scrape** (toutes les N s),
  hors chemin mission.
- `metrics: None` ⇒ le guard ne touche rien (1 branche).

Un bench `criterion` (dev-dep présent) confirmera `record_mission` < 1 µs.

---

## 11. Plan de tests (8+)

### Registry / rendu (`tests/metering.rs` + unit `metering.rs`)
1. `metrics_registry_increments_counter` — `record_mission` succès ×N →
   `efe_missions_total{...success}` == N.
2. `metrics_registry_counts_error_outcome` — guard armé droppé sans
   `record_success` → `outcome="error"` == 1 (RAII path).
3. `metrics_registry_observes_histogram` — `observe(llm, 120)` → bucket `le=250`
   incrémenté, `_sum`/`_count` cohérents.
4. `metrics_registry_renders_prometheus_format` — sortie contient `# TYPE`,
   `_bucket`, `le="+Inf"`, et parse-back des samples.
5. `metrics_registry_verdicts_and_savings` — verdicts SUPPORTED/UNSUPPORTED +
   savings full/speculative comptés.
6. `metrics_registry_none_is_zero_overhead` — `arm(None,…)` + drop ⇒ aucun
   panic, registry séparé reste à 0.
7. `cardinality_excludes_tenant_id` — un `MissionRecord` avec `tenant_id`
   n'ajoute aucun label tenant dans le rendu (anti §4.3).

### Intégration workflow
8. `coordinator_records_mission_metrics_after_run` — `Coordinator` avec
   `MetricsRegistry`, `run_mission_a` (mock) → `missions_total{A,success}`==1,
   `cost_microcents_total{provider=mock}` > 0, histogramme `llm` count==1.
9. `coordinator_records_error_outcome` — mock `with_error` sur tous les
   candidats → `run_mission_a` Err → `missions_total{A,error}`==1 (guard Drop).

### Serveur HTTP (`#[cfg(feature="metrics")]`)
10. `metrics_server_get_returns_200_prometheus` — bind `:0`, GET `/metrics` via
    reqwest → 200, `content-type` text/plain, body contient `efe_missions_total`.
11. `metrics_server_unknown_route_404`.
12. `metrics_server_renders_live_circuit_state` — circuit ouvert →
    `efe_slm_circuit_state{robot,state="open"} 1` au scrape.

---

## 12. Backward-compat & sécurité

- **Backward-compat** : `metrics: None` (défaut) ⇒ overhead nul (§1.3). Feature
  `metrics` off ⇒ serveur non compilé, `tokio/net` non tiré.
- **PII** : `/metrics` n'expose **aucune** donnée utilisateur (pas de prompts,
  pas de réponses, pas de `tenant_id`) — uniquement des agrégats numériques à
  labels bornés. Vérifié par construction (§4.3) + test #7.
- **Surface réseau** : bind `127.0.0.1` par défaut ; pas d'auth (scraping via
  sidecar/reverse proxy en prod). Cap de lecture request-line (anti slowloris
  trivial).
- **Cardinalité** : labels bornés uniquement → pas d'OOM TSDB.

---

## 13. Plan de modification fichier-par-fichier

| Fichier | Action | Détail |
|---|---|---|
| `src/metering.rs` | **NEW** | `MetricsRegistry`, `Counter/GaugeF64/Histogram`, enums labels, `MissionRecord`, `MissionMetricGuard`, `render_prometheus` + unit tests |
| `src/metrics_server.rs` | **NEW** (`#[cfg(feature="metrics")]`) | `serve_metrics`, `snapshot_circuits`, routing HTTP/1.1 |
| `Cargo.toml` | patch | `[features] metrics = ["tokio/net"]` |
| `src/lib.rs` | patch | `pub mod metering;` + `#[cfg(feature="metrics")] pub mod metrics_server;` + re-exports |
| `src/coordinator.rs` | patch | `Coordinator.metrics` + builder `.metrics()` + `build()`/`for_tests()` |
| `src/workflows/mission_a.rs` | patch | guard arm + record_success (2 lignes) |
| `src/workflows/mission_b.rs` | patch | idem |
| `src/workflows/speculative.rs` | patch | idem |
| `tests/metering.rs` | **NEW** | tests 8-12 |
| `CHANGELOG.md` | patch | section R3.S7 |

---

## 14. Phasage d'implémentation (12 phases)

1. **Phase 1** — `Cargo.toml` feature `metrics = ["tokio/net"]`.
2. **Phase 2** — `metering.rs` : `Counter`, `GaugeF64`, `Histogram` + buckets +
   3 tests unit.
3. **Phase 3** — enums labels (`MissionType`, `Outcome`, `Phase`,
   `FailureKind`, `SavingsMode`, `Verdict`) + mapping depuis `FailureMode`/
   `FactCheck.verdict`.
4. **Phase 4** — `MetricsRegistry` (`new` pré-alloc, `record_mission`) + tests
   counter/histogram/verdicts.
5. **Phase 5** — `render_prometheus` (format §9) + test conformité + test
   cardinalité (#7).
6. **Phase 6** — `MissionRecord` + vues `from_mission_a/b/speculative` +
   `MissionMetricGuard` (arm/record_success/Drop) + test RAII erreur (#2).
7. **Phase 7** — `lib.rs` re-exports + `coordinator.rs` champ + builder.
8. **Phase 8** — wire les 3 workflows (guard, 2 lignes chacun). Compile-check.
9. **Phase 9** — tests intégration (#8, #9) via mocks.
10. **Phase 10** — `metrics_server.rs` (feature-gated) + `lib.rs` gated mod.
11. **Phase 11** — tests serveur HTTP (#10-12) + suite complète + clippy
    (`--features metrics` et défaut) exit 0 + bench overhead.
12. **Phase 12** — CHANGELOG + commit.

---

## 15. Risques & mitigations

| Risque | Prob. | Impact | Mitigation |
|---|---|---|---|
| Explosion cardinalité (`tenant_id`) | moyen | **critique** | labels bornés only, test #7 (§4.3) |
| Pertes de comptage (bus) | — | élevé | hook RAII lossless, pas le bus (§4.1) |
| Erreurs non comptées (A/Spec) | moyen | élevé | guard `Drop` capture les `?` (§4.1) |
| Double comptage succès+Drop | faible | moyen | `record_success` désarme (test #2) |
| Hang serveur (slowloris) | faible | bas | cap request-line + `Connection: close` |
| Nouvelle dépendance lourde | — | bas | **évitée** (rendu+serveur faits main) |
| Coût B mal attribué | moyen | bas | label winner + `# HELP` explicite (§4.5) |
| Overhead > 1 ms | faible | moyen | atomiques Relaxed, bench (#§10) |
| Métriques en prod par accident | faible | bas | `metrics:None` défaut + feature serveur off |

---

## 16. Critères d'acceptation

- [ ] `curl http://localhost:9090/metrics` → format Prometheus valide.
- [ ] 7 familles exposées (missions, cost, latency histogram, failure_mode,
      savings, verdicts, circuit_state) couvrant les 7 lignes roadmap.
- [ ] `efe_missions_total{outcome="error"}` incrémenté sur mission échouée
      (preuve lossless du hook RAII).
- [ ] **Aucun** label `tenant_id` (test #7).
- [ ] Overhead `record_mission` < 1 µs (bench) ⇒ < 1 ms/mission.
- [ ] `metrics: None` ⇒ comportement R3.S6 inchangé ; feature off ⇒ serveur
      non compilé.
- [ ] ≥ 12 tests PASS (offline + serveur) ; clippy `--features metrics --tests`
      exit 0 ; build par défaut inchangé.

---

## 17. Hors scope / suites futures

| Item | Sprint candidat | Raison |
|---|---|---|
| `quality_pass_rate{mode="baseline"}` | — | pas de source live ; reframé en `verdicts_total` (§4.4) |
| Ventilation coût per-provider Mission B | post-S7 | coûts par participant non disponibles (§4.5) |
| Push gateway / OTLP / exemplars | sprint dédié | S7 = pull simple |
| TLS/auth `/metrics` | infra | sidecar/reverse proxy |
| Métriques bus/DAG/blackboard internes | post-S7 | S7 = métriques métier |
| Métriques par tenant (agrégées, bornées) | post-S5/S7 | nécessite un schéma de bucketing tenant borné |

---

## 18. Annexe — pourquoi ce design est chirurgical

- **Lossless là où ça compte** : le hook RAII capture **succès ET erreur** en
  2 lignes/workflow, là où le bus (lossy + `SwarmFailed` seulement en B)
  aurait silencieusement sous-compté les erreurs — exactement le genre de gap
  que les sprints précédents traquent.
- **Zéro dépendance** : rendu Prometheus + serveur `TcpListener` faits main,
  réutilisant le précédent `local_slm.rs` — cohérent avec SplitMix64 (R3.S6)
  et le refus de `rand`.
- **Anti dead-config** : `quality_pass_rate{baseline}` (sans source live) est
  reframé en counters `verdicts_total` idiomatiques, pas exposé comme gauge
  mort (cf. `EFE_CHAOS_RATE` R3.S6, `retry_policy` R3.S5).
- **Sécurité de cardinalité** : `tenant_id` (R3.S5) explicitement banni des
  labels — un piège classique qui ferait tomber le TSDB, évité par conception
  et testé.
- **Granularité de feature juste** : le registre + le hook sont toujours
  compilés (zéro dep) ; seule la dépendance réseau (`tokio/net`) du serveur est
  derrière la feature `metrics` — brancher la télémétrie en lib ne coûte rien.
```
