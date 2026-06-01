# M11 — Telemetry, Observability & Audit
## Architecture complète, ultra-détaillée — v1.0

---

**Document** : spécification d'architecture (contrat d'implémentation)

**Modules** :
- nouveau crate `efe-telemetry` (isole les dépendances lourdes : `opentelemetry`, `tracing-opentelemetry`, `prometheus`)
- `efe-core::telemetry` passe de stub à façade qui ré-exporte les types publics du nouveau crate

**Sprints ciblés** : Sprint 9 (logs JSON + metrics Prom + tracing-subscriber) → Sprint 10 (OTLP exporter + dashboards + alertes) → Sprint 11 (redaction + event sinks + audit log)

**Dépend de** :
- M1 Core Runtime (`TelemetryCollector`, `TelemetryGuard` traits, `SwarmId`, `Request`, `Response`, `FusionStats`)
- M9 Alignment Registry (consommateur du `RegistryEvent` channel pour produire métriques + logs d'audit)
- M10 API Server (nécessite `MetricsLayer` côté tower + flip `/metrics` HTTP sidecar)

**Consommé par** : tous les modules — instrumentation transverse. Prod consommateurs externes : Grafana / Tempo / Jaeger / Prometheus / Loki.

**Statut** : spec **gelée** pour implémentation. Changements de types publics (signatures de metrics, noms de spans, attributs OTel) = breaking changes qui nécessitent un ADR.

---

## Table des matières

1. [Mission et positionnement](#1-mission-et-positionnement)
2. [Scope MVP vs hors-périmètre](#2-scope-mvp-vs-hors-périmètre)
3. [Vue d'ensemble architecturale](#3-vue-densemble-architecturale)
4. [Layout du crate `efe-telemetry`](#4-layout-du-crate-efe-telemetry)
5. [Structured logs : `tracing-subscriber` + JSON](#5-structured-logs--tracing-subscriber--json)
6. [OpenTelemetry traces : tracer + OTLP exporter](#6-opentelemetry-traces--tracer--otlp-exporter)
7. [Prometheus metrics : registry + instruments + HTTP endpoint](#7-prometheus-metrics--registry--instruments--http-endpoint)
8. [Implémentation des traits `TelemetryCollector` / `TelemetryGuard`](#8-implémentation-des-traits-telemetrycollector--telemetryguard)
9. [Event sinks : M9 `RegistryEvent` et métier M1-M8](#9-event-sinks--m9-registryevent-et-métier-m1-m8)
10. [Propagation des traces : W3C traceparent](#10-propagation-des-traces--w3c-traceparent)
11. [Redaction et protection PII](#11-redaction-et-protection-pii)
12. [Configuration et wiring runtime](#12-configuration-et-wiring-runtime)
13. [Catalogue des signaux : logs, métriques, spans](#13-catalogue-des-signaux--logs-métriques-spans)
14. [Dashboards Grafana](#14-dashboards-grafana)
15. [Alertes Prometheus](#15-alertes-prometheus)
16. [Modèle de sécurité et threat model](#16-modèle-de-sécurité-et-threat-model)
17. [Budget de performance](#17-budget-de-performance)
18. [Stratégie de test](#18-stratégie-de-test)
19. [Dépendances inter-modules](#19-dépendances-inter-modules)
20. [Plan de sprint M11](#20-plan-de-sprint-m11)
21. [Plan de migration et rollout](#21-plan-de-migration-et-rollout)
22. [Matrice de risques et mitigations](#22-matrice-de-risques-et-mitigations)
23. [Critères d'acceptation](#23-critères-dacceptation)
24. [Annexes](#24-annexes)

---

## 1. Mission et positionnement

### 1.1 Mission fonctionnelle

M11 est le **module d'observabilité transverse** d'EFE. Il fournit une
infrastructure unique pour :

1. **Logs structurés** en JSON, horodatés, enrichis du contexte de la
   requête (request_id, swarm_id, tenant_id, trace_id), consommables par
   n'importe quel aggrégateur (Loki, CloudWatch, Datadog, ELK).
2. **Traces distribuées** au format OpenTelemetry, propagées de bout en
   bout : client SDK → gRPC → `SwarmController` → kernels M5 →
   dissolution M8, avec un span par phase et des attributs typés.
3. **Métriques temps-réel** exposées via un endpoint Prometheus `/metrics`
   sur un port séparé de l'API gRPC, couvrant latences, throughput,
   breach rate, utilisation HBM, convergence rate, etc.
4. **Audit log** des mutations gouvernées (M9 registry events, admin API
   calls de M10) sérialisé en JSON avec tenant + timestamp + payload.

En une phrase : *M11 transforme un processus `efe-server` opaque en un
service observable avec SLO mesurables, pannes explicables et audit
trail complet.*

### 1.2 Pourquoi M11 est critique

Sans M11, les pathologies opérationnelles suivantes persistent :

1. **Aucune visibilité sur les pannes.** Un crash avec
   `EfeError::NumericalExplosion` retourne `INTERNAL` au client mais
   laisse aucune trace côté ops pour diagnostiquer.
2. **SLO non mesurables.** Impossible de prouver contractuellement que
   p99 < 250 ms, breach_rate < 1 %, ou que le mode collapse recovery
   fonctionne en prod.
3. **Aucune corrélation cross-module.** Un breach remonté par M9 ne
   peut pas être relié au swarm M1 qui l'a déclenché sans IDs
   partagés et traces propagées.
4. **Dashboards opérationnels absents.** Les runbooks post-MVP ne
   peuvent pas s'appuyer sur des métriques — paging à l'aveugle.
5. **Audit de gouvernance impossible.** M9 accepte des `register_cone`
   et `evolve_cone` mais sans persistence auditée, la conformité
   (qui a fait quoi, quand) est impossible.
6. **Overhead non borné.** Sans mesure, on ne peut pas prouver que
   l'instrumentation ne dégrade pas la latence p99 de > 2 %.

### 1.3 Contraintes et garanties

| # | Contrainte | Justification |
|---|-----------|---------------|
| C1 | **Crate séparé** `efe-telemetry` | Isoler les deps lourdes (`opentelemetry`, `prometheus`, `tracing-opentelemetry`) du runtime core. |
| C2 | **Non-breaking** pour M1-M10 | Les traits `TelemetryCollector`/`TelemetryGuard` existent déjà ; M11 les implémente. |
| C3 | **Opt-in strict** | Un `efe-server` peut tourner sans aucun exporter (logs stdout seuls) ; tout exporter s'active via config. |
| C4 | **Overhead < 2 %** sur la latence p99 de l'API | Vérifié par benchmark criterion avant release. |
| C5 | **Zéro PII par défaut** | `raw_prompt` jamais loggué ; `--log-prompts` explicite requis. |
| C6 | **Déterminisme des noms** | Noms de métriques / spans / attributs figés dans la v1 — changement = breaking. |
| C7 | **Graceful export shutdown** | Flush des spans et métriques en cours lors du SIGTERM avant abort. |
| C8 | **Backend-agnostic** via OTLP | Tempo, Jaeger, Honeycomb, SigNoz — tous supportés par le même collector sidecar. |

### 1.4 Position dans le pipeline

```
  ┌──────────────────────────────────────────────────────────────┐
  │                     M10 efe-server                             │
  │   gRPC ── MetricsLayer ──┐                                     │
  │                           │ emit counter/histogram              │
  │                           ▼                                    │
  └───────────────────────────────────────────────┬─────────────┘
                 │                                 │
                 │ record_phase_complete           │
                 ▼                                 │
  ┌──────────────────────────────────────┐        │
  │  M1 SwarmController ─ TelemetryGuard │        │
  │   │                                    │       │
  │   └─ M5/M6/M7/M8 phases                │       │
  └──────────────────────────────────────┘        │
                 │                                 │
                 │ registry events                │
                 ▼                                 │
  ┌──────────────────────────────────────┐        │
  │  M9 AlignmentRegistry ─ mpsc channel │        │
  └──────────────┬───────────────────────┘        │
                 │                                 │
                 ▼                                 ▼
  ┌─────────────────────────────────────────────────────────────┐
  │                  M11 Telemetry pipeline                      │
  │                                                              │
  │  ┌──────────┐ ┌──────────┐ ┌──────────────┐ ┌──────────┐   │
  │  │ Tracing  │ │  Metrics │ │ Logs (JSON)  │ │  Audit   │   │
  │  │ (OTel)   │ │ (Prom)   │ │              │ │  events  │   │
  │  └────┬─────┘ └─────┬────┘ └──────┬───────┘ └────┬─────┘   │
  │       │             │              │               │        │
  │       ▼             ▼              ▼               ▼        │
  │  ┌─────────┐ ┌───────────┐ ┌────────────┐ ┌──────────┐      │
  │  │OTLP HTTP│ │/metrics   │ │ stdout /   │ │ audit.   │      │
  │  │exporter │ │ scraper   │ │ file sink  │ │ log.jsonl│      │
  │  └─────────┘ └───────────┘ └────────────┘ └──────────┘      │
  └─────────────────────────────────────────────────────────────┘
         │              │              │              │
         ▼              ▼              ▼              ▼
    Tempo/Jaeger  Prometheus/     Loki/stdout    Audit archive
                  Grafana
```

### 1.5 Garanties offertes aux modules et aux opérateurs

1. **Causalité bout-en-bout** : tout `traceparent` entrant produit un
   trace_id propagé jusqu'aux spans Tempo, visible dans les logs, les
   breach reports et les métriques via exemplars.
2. **Contrat de noms stable en v1** : `efe_grpc_request_duration_seconds`,
   `efe_swarm_rounds_total`, `efe_breach_magnitude`, etc. Figés.
3. **Flush garanti** : `SIGTERM` déclenche un flush OTLP (timeout 5 s)
   avant que le process ne sorte.
4. **Overhead audité** : bench CI par PR, régression > 10 % bloque le
   merge.
5. **PII-safe par défaut** : schéma de logs validé, `raw_prompt` exclu,
   alerte CI si quelqu'un tente de logger un champ blacklisted.

---

## 2. Scope MVP vs hors-périmètre

### 2.1 IN — livré dans le MVP

| Fonctionnalité | Sprint | Justification |
|---|---|---|
| `tracing-subscriber` init avec format JSON + EnvFilter | S9 | Remplace le format humain existant. |
| `TelemetryConfig` avec 3 sinks (stdout JSON, OTLP, Prometheus) | S9 | API de config unique. |
| Implémentation `TelemetryCollector` + `Guard` (vraie, pas stub) | S9 | Satisfait les traits M1 existants. |
| OpenTelemetry tracer + OTLP/gRPC exporter | S10 | Backbone traces distribuées. |
| W3C `traceparent` extraction + inject | S10 | Coordonné avec M10 §15. |
| Prometheus registry + HTTP sidecar `/metrics` (port 9090) | S9 | Scrapable. |
| 12 métriques MVP (listées §13.2) | S9-S10 | Couvre SLO + alertes. |
| Span tree : `grpc.request` → `swarm.execute` → `genesis.spawn` / `fusion.round` / `consensus.evaluate` / `dissolution` | S10 | Hiérarchie OTel canonique. |
| Consommateur M9 `RegistryEvent` → metrics + logs | S10 | Câblage cross-module. |
| Audit log sink (JSON lines file + rotation) | S11 | Conformité. |
| Redaction helpers (`redact_prompt`, `redact_anchors`) | S11 | PII-safe. |
| Graceful flush on SIGTERM | S11 | Pas de perte de signal. |
| Dashboards Grafana : `efe-api`, `efe-swarm`, `efe-alignment` (3 JSONs) | S11 | Prêts-à-importer. |
| Alertes Prometheus : 8 règles PagerDuty-ready | S11 | SLO enforcement. |
| Benchmark d'overhead CI | S11 | Budget C4 tenu. |

### 2.2 OUT — explicitement hors MVP

| Fonctionnalité | Motif | Sprint cible post-MVP |
|---|---|---|
| Profiling continu (pprof/pyroscope) | Nice-to-have, pas SLO-critical. | P1. |
| Structured logs → Loki direct push | Préférer stdout → promtail. | P2. |
| Custom OTLP over HTTP/JSON (on utilise gRPC) | Standard OTLP/gRPC suffit. | P3. |
| Log anonymization automatique (NER) | Hors scope ; redaction manuelle via blocklist. | P3. |
| Multi-backend exporter (Datadog, New Relic natifs) | OTLP collector traduit. | P2. |
| Exemplars Prometheus → Tempo links UI | Fonctionne end-to-end en bonus S10 si bandwidth. | P1. |
| Correlation log → trace dans Grafana Explore | Géré par Grafana (trace_id champ standard). | — (déjà possible). |
| Metric cardinality explosion guardrails | Alerte dashboard uniquement au MVP. | P1. |
| Python SDK telemetry integration | SDK instrumente côté client via OTel Python ; M11 ne gère que le serveur. | P2. |

### 2.3 Frontières claires avec M10 et M9

- **M11 n'implémente pas** la logique serveur gRPC (c'est M10). Il fournit
  le `MetricsLayer` (tower) et le `TracingLayer` consommés par M10.
- **M11 n'implémente pas** la logique de governance (c'est M9). Il lit le
  `mpsc channel` de `RegistryEvent` et produit logs + métriques +
  audit entries.
- **M11 ne manipule pas** les tenseurs — il observe les `FusionStats`
  produites par M5 via les traits M1 existants.
- **M11 ne propage pas** les erreurs métier — il les enregistre
  (counter `efe_errors_total{kind}`) et laisse M10 faire le mapping
  `EfeError → Status`.

---

## 3. Vue d'ensemble architecturale

### 3.1 Arborescence des fichiers

```
crates/efe-telemetry/
├── Cargo.toml
├── src/
│   ├── lib.rs                   # Façade + re-exports
│   ├── config.rs                # TelemetryConfig + Default
│   ├── init.rs                  # init() builder — entry point
│   ├── shutdown.rs              # flush + drain logic on SIGTERM
│   │
│   ├── logs/
│   │   ├── mod.rs
│   │   ├── json_layer.rs        # Custom tracing Layer emitting JSON
│   │   ├── field_filter.rs      # PII blocklist enforcement
│   │   └── audit.rs             # Separate JSON-lines sink for audit events
│   │
│   ├── tracing/
│   │   ├── mod.rs
│   │   ├── tracer.rs            # OtelTracerProvider + global init
│   │   ├── exporter.rs          # OTLP/gRPC client config
│   │   ├── propagation.rs       # W3C TraceContext extract/inject helpers
│   │   └── span_attrs.rs        # Attribute key constants (namespace "efe.*")
│   │
│   ├── metrics/
│   │   ├── mod.rs
│   │   ├── registry.rs          # Shared `prometheus::Registry`
│   │   ├── instruments.rs       # Counters, Histograms, Gauges (lazy statics)
│   │   ├── http_server.rs       # Tokio HTTP server exposing /metrics
│   │   └── tower_layer.rs       # `MetricsLayer` consumed by M10
│   │
│   ├── collector/
│   │   ├── mod.rs
│   │   ├── otel_collector.rs    # impl TelemetryCollector for OtelCollector
│   │   └── otel_guard.rs        # impl TelemetryGuard for OtelGuard
│   │
│   ├── events/
│   │   ├── mod.rs
│   │   └── registry_sink.rs     # Consumes M9 RegistryEvent stream
│   │
│   └── redaction.rs             # helpers: truncate_prompt, hash_id, etc.
│
└── tests/
    ├── integration_grpc_metrics.rs   # e2e: call → counter incremented
    ├── integration_trace_export.rs    # e2e: span → OTLP stub receives it
    ├── integration_registry_sink.rs   # e2e: M9 event → log + metric
    ├── integration_audit_log.rs
    └── integration_overhead_bench.rs  # validates < 2 % overhead
```

### 3.2 Dépendances externes nouvelles

| Crate | Version | Usage |
|---|---|---|
| `opentelemetry` | 0.24 | Core OTel API. |
| `opentelemetry_sdk` | 0.24 | Tracer + Span processor. |
| `opentelemetry-otlp` | 0.17 | OTLP exporter (gRPC). |
| `tracing-opentelemetry` | 0.25 | Bridge tracing → OTel. |
| `prometheus` | 0.13 | Registry + collectors. |
| `metrics` + `metrics-exporter-prometheus` | 0.23 / 0.15 | Idiomatic metrics API. |
| `axum` | 0.7 | Minimal HTTP server for /metrics. |
| `chrono` | 0.4 | Timestamps ISO-8601 in logs. |
| `tracing-serde` | 0.1 | JSON serialization of tracing events. |
| `tower-http` | 0.5 (déjà présent transitivement via tonic) | Trace/metric layers. |

Ces deps sont **isolées dans `efe-telemetry`** — `efe-core` ne les
absorbe pas. `efe-server` ajoute `efe-telemetry = { path = "..." }`.

### 3.3 Couplages internes

```
               ┌───────────┐
               │  init.rs  │  <── entry point
               └─────┬─────┘
                     │
        ┌────────────┼────────────┐
        ▼            ▼            ▼
   ┌────────┐  ┌──────────┐ ┌──────────┐
   │ logs/* │  │tracing/* │ │metrics/* │
   └────────┘  └──────────┘ └──────────┘
                     │            │
                     ▼            ▼
                ┌──────────────────────┐
                │     collector/*      │ ── impl TelemetryCollector
                └──────────┬───────────┘
                           │
                           ▼
                ┌──────────────────────┐
                │      events/*        │ ── consumes M9 mpsc stream
                └──────────────────────┘
```

---

## 4. Layout du crate `efe-telemetry`

### 4.1 `Cargo.toml`

```toml
[package]
name = "efe-telemetry"
version.workspace = true
edition.workspace = true
# ... (standard workspace fields)

[dependencies]
efe-core = { path = "../efe-core" }
tokio = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true, features = ["env-filter", "json"] }
tracing-serde = "0.1"
tracing-opentelemetry = "0.25"
opentelemetry = "0.24"
opentelemetry_sdk = { version = "0.24", features = ["rt-tokio"] }
opentelemetry-otlp = { version = "0.17", features = ["grpc-tonic"] }
prometheus = "0.13"
metrics = "0.23"
metrics-exporter-prometheus = "0.15"
axum = "0.7"
tower = { workspace = true }
tower-http = { version = "0.5", features = ["trace"] }
http = { workspace = true }
http-body-util = "0.1"
chrono = { version = "0.4", default-features = false, features = ["clock", "serde"] }
serde = { workspace = true }
serde_json = { workspace = true }
async-trait = { workspace = true }
anyhow = { workspace = true }
futures = { workspace = true }

[dev-dependencies]
tokio = { workspace = true, features = ["test-util", "macros"] }
tempfile = "3.10"
```

### 4.2 Re-exports publics (`lib.rs`)

```rust
//! EFE observability stack (M11).

#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod collector;
pub mod config;
pub mod events;
pub mod init;
pub mod logs;
pub mod metrics;
pub mod redaction;
pub mod shutdown;
pub mod tracing;

pub use collector::OtelTelemetryCollector;
pub use config::{LogFormat, OtlpConfig, TelemetryConfig};
pub use init::{TelemetryHandle, TelemetryInitError};
pub use metrics::tower_layer::MetricsLayer;
pub use shutdown::graceful_flush;
```

---

## 5. Structured logs : `tracing-subscriber` + JSON

### 5.1 Format JSON

Chaque log est une ligne JSON conforme à cette shape minimale :

```json
{
  "ts": "2026-05-14T12:34:56.789Z",
  "level": "INFO",
  "target": "efe_core::swarm::arbiter",
  "message": "round complete",
  "fields": {
    "round": 3,
    "variance": 0.0012,
    "active_agents": 128
  },
  "request_id": "01HWZR3Q7G8NMFZB0Y3SXE9GE5",
  "swarm_id": "a4b0...5fcd",
  "tenant_id": "acme",
  "trace_id": "0af7651916cd43dd8448eb211c80319c",
  "span_id": "b7ad6b7169203331"
}
```

### 5.2 `json_layer.rs`

- Custom `tracing::Layer` implementing `on_event` that serialises
  each event via `tracing_serde::AsSerde`.
- Extracts `request_id`, `swarm_id`, `tenant_id`, `trace_id`, `span_id`
  from the **current span's fields** (populated by the `TracingLayer` in
  M10 + `OtelCollector::begin_swarm`).
- Uses `chrono::Utc::now()` for the `ts` field.
- Writes to `std::io::Stdout` by default ; configurable output
  (`File`, `stderr`) via `TelemetryConfig::log_writer`.

### 5.3 `field_filter.rs` — PII blocklist

```rust
pub const REDACTED_FIELDS: &[&str] = &["raw_prompt", "prompt", "user_text"];

pub fn filter_field(name: &str, value: serde_json::Value) -> serde_json::Value {
    if REDACTED_FIELDS.contains(&name) {
        serde_json::json!("<redacted>")
    } else {
        value
    }
}
```

- Applied inside `JsonLayer::on_event` before serialisation.
- Turned off when `TelemetryConfig::log_prompts = true` (explicit
  developer override, emits a WARN at startup).

### 5.4 `audit.rs` — audit log

Séparé du flux principal pour pouvoir router indépendamment (file
rotation, retention policies). Écrit en JSON Lines dans
`logs/audit.log.jsonl` avec rotation horaire + compression gzip.

Chaque entrée :
```json
{
  "ts": "...",
  "event": "cone.registered",
  "tenant": "acme",
  "actor": "sub-from-jwt",
  "resource": "acme/chat-qa@v3",
  "details": { ... }
}
```

Consommé par `registry_sink.rs` (voir §9).

---

## 6. OpenTelemetry traces : tracer + OTLP exporter

### 6.1 Initialisation

```rust
pub fn init_tracer(cfg: &OtlpConfig) -> Result<TracerProvider, TelemetryInitError> {
    let exporter = opentelemetry_otlp::new_exporter()
        .tonic()
        .with_endpoint(&cfg.endpoint)
        .with_timeout(cfg.timeout)
        .with_metadata(cfg.headers.clone().into())
        .build_span_exporter()?;

    let provider = TracerProvider::builder()
        .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
        .with_resource(Resource::new(vec![
            KeyValue::new("service.name", "efe-server"),
            KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
            KeyValue::new("deployment.environment", cfg.environment.clone()),
        ]))
        .build();

    opentelemetry::global::set_tracer_provider(provider.clone());
    opentelemetry::global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );
    Ok(provider)
}
```

### 6.2 `tracing-opentelemetry` bridge

```rust
let tracer = provider.tracer("efe");
let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
```

Empilé sur `tracing-subscriber` :

```rust
tracing_subscriber::registry()
    .with(EnvFilter::try_from_default_env()?)
    .with(otel_layer)
    .with(JsonLayer::new(cfg.log_writer, cfg.log_prompts))
    .init();
```

### 6.3 Hiérarchie de spans standard

```
grpc.request                (name: "grpc.request", kind: server)
 ├─ attr: grpc.method = "/efe.v1.SwarmService/Execute"
 ├─ attr: grpc.tenant = "acme"
 ├─ attr: grpc.request_id = "..."
 └─ swarm.execute            (name: "swarm.execute", kind: internal)
     ├─ attr: swarm.id
     ├─ attr: swarm.n_agents
     ├─ genesis.spawn        (kind: internal)
     │   └─ attr: genesis.templates_loaded
     ├─ fusion.round         (répété N fois)
     │   ├─ attr: fusion.round = i
     │   ├─ attr: fusion.variance
     │   └─ attr: fusion.grad_norm
     ├─ consensus.evaluate
     │   └─ attr: consensus.verdict = "converged"|...
     └─ dissolution
         ├─ attr: dissolution.extractor = "weighted_mean"
         └─ attr: dissolution.breach = false
```

### 6.4 `span_attrs.rs` — namespace figé

```rust
/// Attribute keys — all efe-specific attrs use the `efe.*` namespace.
pub mod attrs {
    pub const SWARM_ID: &str = "efe.swarm.id";
    pub const TENANT_ID: &str = "efe.tenant.id";
    pub const REQUEST_ID: &str = "efe.request.id";
    pub const N_AGENTS: &str = "efe.swarm.n_agents";
    pub const DIM_LATENT: &str = "efe.swarm.dim_latent";
    pub const ROUND: &str = "efe.fusion.round";
    pub const VARIANCE: &str = "efe.fusion.variance";
    pub const GRAD_NORM: &str = "efe.fusion.grad_norm";
    pub const VERDICT: &str = "efe.consensus.verdict";
    pub const CONE_KEY: &str = "efe.alignment.cone_key";
    pub const BREACH_DELTA: &str = "efe.alignment.breach_delta";
    pub const BREACH_RADIUS: &str = "efe.alignment.breach_radius";
    pub const EXTRACTOR: &str = "efe.dissolution.extractor";
    // ...
}
```

Ces constantes **gèlent** les noms d'attributs pour la v1. Toute
modification nécessite un ADR + bump SEMVER mineur de `efe-telemetry`.

---

## 7. Prometheus metrics : registry + instruments + HTTP endpoint

### 7.1 `registry.rs`

```rust
pub fn global_registry() -> &'static prometheus::Registry {
    static REGISTRY: OnceLock<prometheus::Registry> = OnceLock::new();
    REGISTRY.get_or_init(prometheus::Registry::new)
}
```

Un **unique** registry global par process, partagé entre M10 tower layer,
l'event sink M9, et le telemetry collector M1.

### 7.2 `instruments.rs` — instruments MVP

```rust
lazy_static! {
    // gRPC.
    pub static ref GRPC_REQUESTS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("efe_grpc_requests_total", "Total gRPC requests handled"),
        &["method", "code", "tenant"],
    ).unwrap();

    pub static ref GRPC_REQUEST_DURATION: HistogramVec = HistogramVec::new(
        HistogramOpts::new(
            "efe_grpc_request_duration_seconds",
            "gRPC request latency",
        ).buckets(prometheus::exponential_buckets(0.001, 2.0, 12).unwrap()),
        &["method", "tenant"],
    ).unwrap();

    pub static ref GRPC_IN_FLIGHT: IntGaugeVec = IntGaugeVec::new(
        Opts::new("efe_grpc_requests_in_flight", "Concurrent gRPC requests"),
        &["method"],
    ).unwrap();

    // Swarm execution.
    pub static ref SWARM_ROUNDS_TOTAL: IntCounter = IntCounter::new(
        "efe_swarm_rounds_total",
        "Total fusion rounds executed across all swarms",
    ).unwrap();

    pub static ref SWARM_DURATION: HistogramVec = HistogramVec::new(
        HistogramOpts::new(
            "efe_swarm_duration_seconds",
            "Full swarm execution latency",
        ).buckets(prometheus::exponential_buckets(0.005, 2.0, 12).unwrap()),
        &["verdict", "tenant"],
    ).unwrap();

    pub static ref SWARM_CONFIDENCE: HistogramVec = HistogramVec::new(
        HistogramOpts::new(
            "efe_swarm_confidence",
            "Final confidence of converged swarms (0..1)",
        ).buckets(vec![0.5, 0.6, 0.7, 0.8, 0.9, 0.95, 0.99, 1.0]),
        &["tenant"],
    ).unwrap();

    // Errors.
    pub static ref ERRORS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("efe_errors_total", "Errors by kind"),
        &["kind", "tenant"],
    ).unwrap();

    // Alignment.
    pub static ref BREACHES_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new(
            "efe_breaches_total",
            "Alignment breaches reported to M9",
        ),
        &["tenant", "task", "context"],
    ).unwrap();

    pub static ref BREACH_MAGNITUDE: Histogram = Histogram::with_opts(
        HistogramOpts::new(
            "efe_breach_magnitude",
            "observed_delta - cone_radius at breach",
        ).buckets(vec![0.0, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0]),
    ).unwrap();

    // Registry (M9).
    pub static ref CONE_REGISTRATIONS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("efe_cone_registrations_total", "Cones registered / evolved"),
        &["event", "tenant"],
    ).unwrap();

    pub static ref POLICY_REJECTIONS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new(
            "efe_policy_rejections_total",
            "Policy violations by invariant code",
        ),
        &["reason"],
    ).unwrap();

    // Rate limiting (M10).
    pub static ref RATELIMIT_DROPS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("efe_ratelimit_drops_total", "Requests rejected by rate limit"),
        &["tenant"],
    ).unwrap();
}

pub fn register_all(registry: &Registry) -> Result<()> {
    registry.register(Box::new(GRPC_REQUESTS_TOTAL.clone()))?;
    registry.register(Box::new(GRPC_REQUEST_DURATION.clone()))?;
    // ... all of the above
    Ok(())
}
```

### 7.3 `http_server.rs` — sidecar `/metrics`

```rust
use axum::{routing::get, Router, response::IntoResponse};
use prometheus::{Encoder, TextEncoder};

async fn metrics_handler() -> impl IntoResponse {
    let mut buffer = Vec::new();
    let encoder = TextEncoder::new();
    let metric_families = global_registry().gather();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    (
        [("Content-Type", encoder.format_type())],
        buffer,
    )
}

pub async fn serve(addr: SocketAddr, shutdown: impl Future<Output = ()> + Send + 'static) -> Result<()> {
    let app = Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/healthz", get(|| async { "ok" }));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}
```

Port par défaut : 9090 (Prometheus). Configurable via `TelemetryConfig`.

### 7.4 `tower_layer.rs` — intégration M10

```rust
#[derive(Clone)]
pub struct MetricsLayer { /* ... */ }

impl<S> Layer<S> for MetricsLayer { /* wraps inner service */ }

impl<S> Service<Request<Body>> for MetricsService<S>
where S: Service<Request<Body>, ...>
{
    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let method = req.uri().path().to_string();
        let tenant = req.extensions().get::<RequestTenant>()
            .map(|t| t.0.clone())
            .unwrap_or_else(|| "anonymous".to_string());
        GRPC_IN_FLIGHT.with_label_values(&[&method]).inc();
        let timer = GRPC_REQUEST_DURATION
            .with_label_values(&[&method, &tenant])
            .start_timer();

        let fut = self.inner.call(req);
        Box::pin(async move {
            let res = fut.await;
            timer.observe_duration();
            GRPC_IN_FLIGHT.with_label_values(&[&method]).dec();
            let code = status_code_of(&res);
            GRPC_REQUESTS_TOTAL
                .with_label_values(&[&method, code, &tenant])
                .inc();
            res
        })
    }
}
```

Ajouté dans le `runtime.rs` de M10 :

```rust
let router = TonicServer::builder()
    .layer(efe_telemetry::MetricsLayer::new())
    .concurrency_limit_per_connection(...)
    .add_service(...)
```

---

## 8. Implémentation des traits `TelemetryCollector` / `TelemetryGuard`

### 8.1 `OtelTelemetryCollector`

```rust
pub struct OtelTelemetryCollector {
    tracer: Arc<opentelemetry::global::BoxedTracer>,
}

impl OtelTelemetryCollector {
    pub fn new() -> Self {
        Self { tracer: Arc::new(opentelemetry::global::tracer("efe")) }
    }
}

impl efe_core::TelemetryCollector for OtelTelemetryCollector {
    fn begin_swarm(
        &self,
        id: SwarmId,
        req: &Request,
    ) -> Box<dyn efe_core::TelemetryGuard> {
        let span = self
            .tracer
            .span_builder("swarm.execute")
            .with_attributes(vec![
                KeyValue::new(attrs::SWARM_ID, id.to_string()),
                KeyValue::new(attrs::TENANT_ID, req.tenant_id.clone()),
                KeyValue::new(attrs::REQUEST_ID, req.id.to_string()),
                KeyValue::new(attrs::N_AGENTS, req.spec.population.total() as i64),
                KeyValue::new(attrs::DIM_LATENT, req.spec.dim_latent.get() as i64),
            ])
            .start(&*self.tracer);
        Box::new(OtelGuard::new(id, req.tenant_id.clone(), span))
    }
}
```

### 8.2 `OtelGuard`

```rust
pub struct OtelGuard {
    swarm_id: SwarmId,
    tenant: String,
    span: opentelemetry::trace::Span,
    started_at: Instant,
    phase_spans: Vec<opentelemetry::trace::Span>,
}

impl efe_core::TelemetryGuard for OtelGuard {
    fn record_phase_complete(&mut self, phase: &str, elapsed: Duration) {
        // Emit a child event on the swarm span.
        self.span.add_event(
            format!("phase_complete:{phase}"),
            vec![KeyValue::new("phase.elapsed_ms", elapsed.as_millis() as i64)],
        );
    }

    fn record_phase_error(&mut self, phase: &str, err: &EfeError) {
        let kind = err.kind_label(); // &str from EfeError
        ERRORS_TOTAL
            .with_label_values(&[kind, &self.tenant])
            .inc();
        self.span.record_error(err);
        self.span.set_status(opentelemetry::trace::Status::error(kind));
    }

    fn record_round(&mut self, round: u32, stats: &FusionStats) {
        SWARM_ROUNDS_TOTAL.inc();
        self.span.add_event(
            format!("round:{round}"),
            vec![
                KeyValue::new(attrs::ROUND, round as i64),
                KeyValue::new(attrs::VARIANCE, stats.variance as f64),
                KeyValue::new(attrs::GRAD_NORM, stats.grad_norm as f64),
            ],
        );
    }

    fn record_event(&mut self, round: u32, event: &str) {
        self.span.add_event(
            event.to_string(),
            vec![KeyValue::new(attrs::ROUND, round as i64)],
        );
    }

    fn finalize(&mut self, total_ms: f32, stats: &FusionStats) {
        let verdict_label = stats.last_verdict_label();
        SWARM_DURATION
            .with_label_values(&[verdict_label, &self.tenant])
            .observe(total_ms as f64 / 1000.0);
        SWARM_CONFIDENCE
            .with_label_values(&[&self.tenant])
            .observe(stats.final_confidence as f64);
        self.span.end();
    }
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        // Defensive: if finalize() wasn't called (e.g., panic), end the span anyway.
        if !self.span.is_recording_ended() {
            self.span.end();
        }
    }
}
```

### 8.3 Intégration dans `SwarmControllerBuilder`

Le builder M1 accepte déjà `Arc<dyn TelemetryCollector>` — simplement
câbler :

```rust
let telemetry = Arc::new(efe_telemetry::OtelTelemetryCollector::new());
let controller = SwarmController::builder()
    .genesis(...)
    .fusion(...)
    // ...
    .telemetry(telemetry.clone())
    .build()?;
```

---

## 9. Event sinks : M9 `RegistryEvent` et métier M1-M8

### 9.1 `registry_sink.rs`

```rust
pub struct RegistrySink {
    audit: Arc<AuditLogger>,
}

impl RegistrySink {
    pub fn new(audit: Arc<AuditLogger>) -> Self { Self { audit } }

    pub async fn run(self, mut rx: mpsc::UnboundedReceiver<RegistryEvent>) {
        while let Some(event) = rx.recv().await {
            self.handle(event).await;
        }
    }

    async fn handle(&self, event: RegistryEvent) {
        match &event {
            RegistryEvent::Registered { key, .. } => {
                CONE_REGISTRATIONS_TOTAL
                    .with_label_values(&["registered", &key.tenant])
                    .inc();
                self.audit.record("cone.registered", key, None).await;
            }
            RegistryEvent::Evolved { from, to, op_kind } => {
                CONE_REGISTRATIONS_TOTAL
                    .with_label_values(&["evolved", &from.tenant])
                    .inc();
                self.audit.record(
                    "cone.evolved",
                    to,
                    Some(serde_json::json!({ "from": from.to_string(), "op": op_kind })),
                ).await;
            }
            RegistryEvent::Deleted { key } => {
                CONE_REGISTRATIONS_TOTAL
                    .with_label_values(&["deleted", &key.tenant])
                    .inc();
                self.audit.record("cone.deleted", key, None).await;
            }
            RegistryEvent::BreachReported { report } => {
                BREACHES_TOTAL
                    .with_label_values(&[
                        &report.cone_key.tenant,
                        &report.cone_key.task,
                        report.context.kind_label(),
                    ])
                    .inc();
                BREACH_MAGNITUDE.observe(report.magnitude() as f64);
                tracing::warn!(
                    tenant = %report.cone_key.tenant,
                    task = %report.cone_key.task,
                    swarm_id = %report.swarm_id,
                    observed_delta = report.observed_delta,
                    cone_radius = report.cone_radius,
                    "alignment breach"
                );
            }
            RegistryEvent::PolicyRejected { reason, .. } => {
                let code = extract_invariant_code(reason); // "I1".."I7"
                POLICY_REJECTIONS_TOTAL
                    .with_label_values(&[code])
                    .inc();
            }
        }
    }
}
```

### 9.2 Wiring au démarrage

```rust
// Dans efe-server/main.rs :
let registry = Arc::new(InMemoryRegistry::with_default_config()?);
let rx = registry.take_events_receiver().await.unwrap();
let audit = Arc::new(AuditLogger::open("logs/audit.log.jsonl")?);
let sink = RegistrySink::new(audit);
tokio::spawn(sink.run(rx));
```

---

## 10. Propagation des traces : W3C traceparent

### 10.1 Extract (côté server)

Déjà fait par M10 `AuthGuard::check` qui lit `traceparent` / `tracestate`
et les stocke dans `Request.extensions` sous `TraceContext`. M11 fournit
un helper pour **attacher** ce context à un span OTel :

```rust
pub fn attach_trace_context(span: &Span, ctx: &TraceContext) {
    let propagator = TraceContextPropagator::new();
    let mut carrier = HashMap::new();
    carrier.insert("traceparent".to_string(), ctx.traceparent.clone());
    if let Some(ts) = &ctx.tracestate {
        carrier.insert("tracestate".to_string(), ts.clone());
    }
    let parent_ctx = propagator.extract(&carrier);
    span.set_parent(parent_ctx);
}
```

### 10.2 Inject (côté client futur)

Pour les appels sortants (ex. M9 → service auth externe post-MVP) :

```rust
pub fn inject_current_span_into(headers: &mut HeaderMap) {
    let propagator = TraceContextPropagator::new();
    let mut carrier = HashMap::new();
    propagator.inject_context(&opentelemetry::Context::current(), &mut carrier);
    for (k, v) in carrier {
        if let Ok(val) = HeaderValue::from_str(&v) {
            headers.insert(HeaderName::from_str(&k).unwrap(), val);
        }
    }
}
```

---

## 11. Redaction et protection PII

### 11.1 Règle d'or

**Aucun prompt utilisateur n'atteint jamais les logs par défaut.** La
configuration `log_prompts = true` est une feature explicite qui émet un
`WARN` de démarrage, visible dans les logs, qui reste actif tant qu'elle
est activée.

### 11.2 `redaction.rs`

```rust
const REDACTED_MARKER: &str = "<redacted>";

pub fn redact_prompt(raw: &str) -> String {
    if raw.is_empty() { return String::new(); }
    // Hash + prefix to allow deduplication without leaking content.
    let hash = blake3::hash(raw.as_bytes()).to_hex();
    format!("<redacted:sha256={}>", &hash.as_str()[..16])
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } else { format!("{}…", &s[..max]) }
}

pub fn mask_token(token: &str) -> String {
    if token.len() < 12 { REDACTED_MARKER.to_string() } else {
        format!("{}…{}", &token[..4], &token[token.len() - 4..])
    }
}
```

### 11.3 Blocklist appliquée par `JsonLayer`

Liste (stable v1) : `raw_prompt`, `prompt`, `user_text`, `authorization`,
`jwt_secret`, `hs256_secrets`.

Le `tracing-subscriber` layer inspecte chaque champ de l'event ; si le
nom est blocklisted, la valeur est remplacée par `"<redacted>"` avant
sérialisation JSON.

### 11.4 Audit CI

Test CI qui grep récursivement `src/` pour `raw_prompt` utilisé dans un
macro `tracing::info!` / `tracing::debug!` / etc. → fail si trouvé sans
avoir traversé `redact_prompt`.

---

## 12. Configuration et wiring runtime

### 12.1 `config.rs`

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TelemetryConfig {
    pub service_name: String,
    pub environment: String,
    pub log_format: LogFormat,
    pub log_filter: String,       // e.g. "info,efe=debug"
    pub log_prompts: bool,        // PII risk — default false
    pub metrics_addr: Option<SocketAddr>, // e.g. 0.0.0.0:9090 ; None disables
    pub otlp: Option<OtlpConfig>, // None disables OTLP export
    pub audit_log_path: Option<PathBuf>,
    pub flush_timeout: Duration,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum LogFormat {
    Json,
    Pretty, // dev only
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OtlpConfig {
    pub endpoint: String,          // e.g. http://otel-collector:4317
    pub timeout: Duration,
    pub headers: HashMap<String, String>,
    pub environment: String,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            service_name: "efe-server".into(),
            environment: "dev".into(),
            log_format: LogFormat::Json,
            log_filter: "info,efe=debug".into(),
            log_prompts: false,
            metrics_addr: Some("0.0.0.0:9090".parse().unwrap()),
            otlp: None,
            audit_log_path: Some("logs/audit.log.jsonl".into()),
            flush_timeout: Duration::from_secs(5),
        }
    }
}
```

### 12.2 `init.rs`

```rust
pub struct TelemetryHandle {
    pub tracer_provider: Option<TracerProvider>,
    pub metrics_task: Option<JoinHandle<()>>,
    pub audit_task: Option<JoinHandle<()>>,
    pub flush_timeout: Duration,
}

impl TelemetryHandle {
    /// Graceful shutdown — flushes spans, drains audit writer.
    pub async fn shutdown(self) {
        if let Some(tp) = self.tracer_provider {
            let _ = tokio::time::timeout(
                self.flush_timeout,
                tokio::task::spawn_blocking(move || tp.shutdown()),
            ).await;
        }
        if let Some(h) = self.metrics_task { h.abort(); let _ = h.await; }
        if let Some(h) = self.audit_task { h.abort(); let _ = h.await; }
    }
}

pub fn init(cfg: TelemetryConfig) -> Result<TelemetryHandle, TelemetryInitError> {
    // 1. Prometheus registry + instruments.
    let registry = metrics::global_registry();
    metrics::instruments::register_all(registry)?;

    // 2. Tracer (optional).
    let tracer_provider = cfg.otlp.as_ref()
        .map(|c| tracing::tracer::init_tracer(c)).transpose()?;

    // 3. tracing-subscriber stack.
    let env_filter = EnvFilter::try_new(&cfg.log_filter)
        .unwrap_or_else(|_| EnvFilter::new("info"));
    let json_layer = match cfg.log_format {
        LogFormat::Json => JsonLayer::new(cfg.log_prompts).boxed(),
        LogFormat::Pretty => tracing_subscriber::fmt::layer().pretty().boxed(),
    };
    let subscriber = tracing_subscriber::registry().with(env_filter).with(json_layer);
    if let Some(tp) = &tracer_provider {
        let tracer = tp.tracer("efe");
        let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
        subscriber.with(otel_layer).init();
    } else {
        subscriber.init();
    }

    // 4. Prometheus HTTP server.
    let metrics_task = cfg.metrics_addr.map(|addr| {
        tokio::spawn(async move {
            let _ = metrics::http_server::serve(addr, std::future::pending()).await;
        })
    });

    // 5. Audit log writer.
    let audit_task = cfg.audit_log_path.as_ref().map(|path| {
        let writer = logs::audit::spawn_writer(path.clone());
        writer
    });

    Ok(TelemetryHandle {
        tracer_provider,
        metrics_task,
        audit_task,
        flush_timeout: cfg.flush_timeout,
    })
}
```

### 12.3 Wiring dans `efe-server/main.rs`

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let telemetry_cfg = TelemetryConfig::from_env();
    let telemetry = efe_telemetry::init(telemetry_cfg)?;

    let backend = build_backend().await?;
    let registry = build_registry().await?;

    // Hook telemetry collector into the controller.
    let telemetry_collector = Arc::new(efe_telemetry::OtelTelemetryCollector::new());
    let controller = SwarmController::builder()
        .telemetry(telemetry_collector)
        // ...
        .build()?;
    let backend = Arc::new(SwarmControllerBackend::new(controller));

    // Hook registry events into sink.
    if let Some(rx) = registry.take_events_receiver().await {
        let audit = telemetry.audit_logger();
        tokio::spawn(efe_telemetry::events::RegistrySink::new(audit).run(rx));
    }

    // Start gRPC server with MetricsLayer.
    let server = Server::builder()
        .config(server_cfg)
        .backend(backend)
        .registry(registry)
        .metrics_layer(efe_telemetry::MetricsLayer::new())
        .build()?;

    let shutdown = Shutdown::new();
    let serve = server.serve(addr, shutdown.clone());
    let result = serve.await;

    // Flush telemetry before exit.
    telemetry.shutdown().await;
    result
}
```

---

## 13. Catalogue des signaux : logs, métriques, spans

### 13.1 Logs structurés émis au MVP

| Event | Level | Source | Champs |
|---|---|---|---|
| `efe-server starting` | INFO | runtime.rs | `version`, `addr`, `auth_enabled` |
| `swarm spawned` | INFO | M3 | `swarm_id`, `n_agents`, `dim_latent`, `tenant_id` |
| `round complete` | INFO | M5 | `round`, `variance`, `grad_norm`, `active` |
| `consensus verdict` | INFO | M6 | `round`, `verdict`, `confidence` |
| `alignment breach` | WARN | M9 sink | `tenant`, `task`, `swarm_id`, `delta`, `radius` |
| `policy rejection` | WARN | M9 sink | `reason`, `attempted_key` |
| `internal error` | ERROR | bridge::error | `path`, `code` |
| `grpc.request.complete` | INFO | M10 MetricsLayer | `method`, `code`, `duration_ms`, `tenant` |
| `shutdown requested` | INFO | M10 runtime | — |

### 13.2 Métriques Prometheus MVP (12 total)

| Metric | Type | Labels | Purpose |
|---|---|---|---|
| `efe_grpc_requests_total` | counter | method, code, tenant | RPS, error rate |
| `efe_grpc_request_duration_seconds` | histogram | method, tenant | p50/p95/p99 |
| `efe_grpc_requests_in_flight` | gauge | method | concurrency |
| `efe_swarm_rounds_total` | counter | — | avg rounds |
| `efe_swarm_duration_seconds` | histogram | verdict, tenant | swarm latency |
| `efe_swarm_confidence` | histogram | tenant | confidence distribution |
| `efe_errors_total` | counter | kind, tenant | error categorisation |
| `efe_breaches_total` | counter | tenant, task, context | breach rate |
| `efe_breach_magnitude` | histogram | — | severity |
| `efe_cone_registrations_total` | counter | event, tenant | registry activity |
| `efe_policy_rejections_total` | counter | reason | invariant violations |
| `efe_ratelimit_drops_total` | counter | tenant | throttling |

### 13.3 Spans OpenTelemetry MVP

- `grpc.request` (server kind, root)
- `swarm.execute` (internal)
- `genesis.spawn` (internal, child of swarm.execute)
- `fusion.round` (internal, repeated)
- `consensus.evaluate` (internal)
- `dissolution` (internal)
- `alignment.project` (internal, child of dissolution)
- `admin.register_cone` (server kind)
- `admin.evolve_cone` (server kind)

---

## 14. Dashboards Grafana

### 14.1 Dashboard `efe-api` (gRPC health)

**Variables** : `tenant`, `method`.

**Panels** :
1. **Request rate** : `sum(rate(efe_grpc_requests_total[5m])) by (method)`.
2. **Latency p50/p95/p99** : `histogram_quantile(0.95, rate(efe_grpc_request_duration_seconds_bucket{method="$method"}[5m]))`.
3. **Error rate %** : `sum(rate(efe_grpc_requests_total{code!="OK"}[5m])) / sum(rate(efe_grpc_requests_total[5m])) * 100`.
4. **In-flight** : `efe_grpc_requests_in_flight`.
5. **Rate limit drops / min** : `rate(efe_ratelimit_drops_total[1m])`.

### 14.2 Dashboard `efe-swarm`

1. **Swarm latency p99 by verdict** : `histogram_quantile(0.99, rate(efe_swarm_duration_seconds_bucket[5m])) by (verdict)`.
2. **Confidence distribution** : heatmap of `efe_swarm_confidence_bucket`.
3. **Rounds per swarm** : `rate(efe_swarm_rounds_total[5m]) / rate(efe_swarm_duration_seconds_count[5m])`.
4. **Error breakdown** : pie chart of `efe_errors_total{kind}`.

### 14.3 Dashboard `efe-alignment`

1. **Breach rate by tenant/task** : `sum(rate(efe_breaches_total[5m])) by (tenant, task)`.
2. **Breach magnitude distribution** : heatmap.
3. **Cone registrations / min** : stacked counter by `event`.
4. **Policy rejection causes** : bar chart by `reason` (I1..I7).

Fichiers : `ops/grafana/efe-api.json`, `efe-swarm.json`, `efe-alignment.json`.

---

## 15. Alertes Prometheus

### 15.1 Règles MVP (8)

```yaml
# ops/prometheus/alerts-m11.yml
groups:
- name: efe-slo
  interval: 30s
  rules:
  - alert: EfeHighP99Latency
    expr: |
      histogram_quantile(0.99, sum by (le) (rate(efe_grpc_request_duration_seconds_bucket{method="/efe.v1.SwarmService/Execute"}[5m]))) > 0.25
    for: 5m
    labels: { severity: P1 }
    annotations:
      summary: "Execute p99 > 250ms for 5 min"

  - alert: EfeHighErrorRate
    expr: |
      sum(rate(efe_grpc_requests_total{code!="OK"}[5m]))
        / sum(rate(efe_grpc_requests_total[5m])) > 0.05
    for: 5m
    labels: { severity: P1 }

  - alert: EfeHighBreachRate
    expr: |
      sum(rate(efe_breaches_total[5m])) by (tenant)
        / sum(rate(efe_grpc_requests_total{code="OK"}[5m])) by (tenant) > 0.01
    for: 10m
    labels: { severity: P2 }

  - alert: EfePolicyRejectionSpike
    expr: sum(rate(efe_policy_rejections_total[5m])) > 1
    for: 10m
    labels: { severity: P2 }

  - alert: EfeRateLimitBurstByTenant
    expr: sum(rate(efe_ratelimit_drops_total[5m])) by (tenant) > 10
    for: 10m
    labels: { severity: P3 }

  - alert: EfeModeCollapseSpike
    expr: sum(rate(efe_errors_total{kind="mode_collapse"}[5m])) > 1
    for: 10m
    labels: { severity: P2 }

  - alert: EfeServerDown
    expr: up{job="efe-server"} == 0
    for: 2m
    labels: { severity: P0 }

  - alert: EfeMetricsEndpointStalled
    expr: time() - efe_metrics_last_scrape_ts > 120
    for: 5m
    labels: { severity: P2 }
```

---

## 16. Modèle de sécurité et threat model

### 16.1 Menaces

| # | Menace | Vecteur | Mitigation |
|---|---|---|---|
| T1 | PII dans les logs | `raw_prompt` loggué | Blocklist + CI grep + `log_prompts=false` par défaut. |
| T2 | Token JWT leaked dans logs | `authorization` header | Blocklist explicite + `mask_token` helper. |
| T3 | OTLP endpoint MitM | Attaquant intercepte spans | TLS obligatoire sur OTLP (config rejette `http://` en prod). |
| T4 | Prometheus endpoint exposé publiquement | Scraping externe | Port 9090 bind sur `127.0.0.1` par défaut ; sidecar network policy. |
| T5 | Log injection via traceparent | Attaquant injecte `\n` dans headers | `traceparent` validé format W3C strict avant usage. |
| T6 | Metric cardinality explosion | Tenant génère 10⁶ labels | Labels limités à `tenant, method, code, task, context, reason`, tous from finite sets. |
| T7 | Disk fill via audit log | Rotation manquante | Rotation horaire + purge > 30 jours via logrotate sidecar. |
| T8 | Secret dans config | `hs256_secrets` sérialisé | `Debug` manuel masque ; audit CI. |

### 16.2 Non-threats MVP

- **Signature des audit logs** : signature crypto des entrées non MVP
  (P2).
- **Tamper-evidence** : audit log non-append-only ; hash chaining P2.

---

## 17. Budget de performance

### 17.1 Cibles

| Opération | Budget |
|---|---|
| `tracing::info!` avec JSON layer | < 5 µs |
| `GRPC_REQUESTS_TOTAL.inc()` | < 50 ns |
| `GRPC_REQUEST_DURATION.observe()` | < 200 ns |
| Span begin/end (OTel) | < 2 µs |
| Overall overhead sur `Execute` p99 | < 2 ms (< 2 % du budget 250 ms) |
| Export batch OTLP (100 spans) | < 50 ms async, non-blocking |

### 17.2 Benchmarks CI

```rust
#[bench] fn bench_metrics_counter_inc();
#[bench] fn bench_metrics_histogram_observe();
#[bench] fn bench_tracing_info_with_json_layer();
#[bench] fn bench_otel_span_lifecycle();
#[bench] fn bench_grpc_execute_with_vs_without_telemetry();
```

---

## 18. Stratégie de test

### 18.1 Unit tests (~60)

- `json_layer.rs` : 8 tests — field filtering, timestamp format,
  trace_id/span_id extraction, redaction marker.
- `registry.rs` : 4 — global singleton, register/unregister, collisions.
- `instruments.rs` : 12 — un par metric, labels, buckets.
- `http_server.rs` : 3 — `/metrics` returns 200, content-type, `/healthz`.
- `tower_layer.rs` : 6 — increment on success, increment on error,
  in-flight delta, tenant extraction, method extraction.
- `otel_collector.rs` + `otel_guard.rs` : 10 — trait compliance, span
  attributes, drop-safe, error recording.
- `registry_sink.rs` : 8 — each `RegistryEvent` variant produces the
  expected metric + log.
- `redaction.rs` : 6 — prompt hash, truncate, mask_token edge cases.

### 18.2 Integration tests (~6)

1. `integration_grpc_metrics` — start server with MetricsLayer, send
   requests, scrape `/metrics`, assert counters.
2. `integration_trace_export` — start OTLP stub (in-memory gRPC server),
   send a request, assert OtlpExportRequest received with expected
   spans.
3. `integration_registry_sink` — inject M9 events, assert metrics +
   audit entries.
4. `integration_audit_log_rotation` — write > threshold, assert rotation.
5. `integration_shutdown_flush` — trigger shutdown mid-stream, assert
   spans exported before exit.
6. `integration_pii_redaction` — emit `raw_prompt` field, assert log
   output has `<redacted>`.

### 18.3 Regression CI

- **Overhead benchmark** : `bench_grpc_execute_with_vs_without_telemetry`
  must show < 2 % p99 delta. Fail CI if > 10 %.
- **Log schema validator** : parse stdout JSON from a canary run through
  `jsonschema` with the published schema ; fail if non-conforming.
- **PII blocklist grep** : `grep -rE "raw_prompt|user_text" src/ | grep
  tracing::` should be empty.

---

## 19. Dépendances inter-modules

### 19.1 Sortantes

| De | Vers | Surface |
|---|---|---|
| `efe-telemetry` | `efe-core` | `TelemetryCollector` trait, `SwarmId`, `Request`, `FusionStats`, `EfeError`, `RegistryEvent` |
| `efe-telemetry` | externes | voir §4.1 |

### 19.2 Entrantes

| De | Consomme | Surface |
|---|---|---|
| `efe-server` | `OtelTelemetryCollector`, `MetricsLayer`, `init`, `TelemetryConfig` | cat public §4.2 |
| (futur) `python/efe` | Spans côté client via OTel Python | pas de dep crate |

---

## 20. Plan de sprint M11

### 20.1 Sprint 9 — Logs + Metrics de base

| Jour | Livrable |
|---|---|
| J1-J2 | Crate `efe-telemetry` + `Cargo.toml` + `lib.rs` façade |
| J3 | `config.rs` + `TelemetryConfig` + defaults |
| J4 | `logs/json_layer.rs` + `logs/field_filter.rs` + 8 tests |
| J5 | `metrics/registry.rs` + `metrics/instruments.rs` (12 instruments) |
| J6 | `metrics/http_server.rs` + axum + tests |
| J7 | `metrics/tower_layer.rs` + integration avec M10 (ajout dans `Server::builder`) |
| J8 | `init.rs` avec TelemetryHandle + shutdown basique |
| J9 | Integration test `integration_grpc_metrics` |
| J10 | ADR-012 "Telemetry architecture" + doc |

**Gate S9** : metrics `/metrics` scrape OK, JSON logs produits,
MetricsLayer actif dans `efe-server`.

### 20.2 Sprint 10 — OTel + sinks

| Jour | Livrable |
|---|---|
| J1-J2 | `tracing/tracer.rs` + `tracing/exporter.rs` (OTLP gRPC) |
| J3 | `tracing/span_attrs.rs` (namespace figé) |
| J4 | `tracing/propagation.rs` + extract/inject helpers |
| J5-J6 | `collector/otel_collector.rs` + `otel_guard.rs` impl traits M1 (10 tests) |
| J7 | `events/registry_sink.rs` + M9 integration (8 tests) |
| J8 | Integration test `integration_trace_export` avec stub OTLP |
| J9 | Integration `integration_registry_sink` |
| J10 | Dashboards Grafana v1 (3 JSONs) |

### 20.3 Sprint 11 — Audit + redaction + hardening

| Jour | Livrable |
|---|---|
| J1 | `logs/audit.rs` + rotation horaire |
| J2 | `redaction.rs` + 6 tests |
| J3 | `shutdown.rs` + flush OTLP + audit drain |
| J4 | Règles Prometheus `alerts-m11.yml` |
| J5 | PII CI grep + schema validator |
| J6-J7 | Benchmarks d'overhead + gate régression |
| J8 | `integration_audit_log_rotation` + `integration_shutdown_flush` |
| J9 | Runbook ops : "Debugging a high-latency request via traces" |
| J10 | RC readiness review |

### 20.4 Jalons

- **J-M11.1 (fin S9)** : `/metrics` + JSON logs live en dev.
- **J-M11.2 (fin S10)** : traces OTLP exportées, 3 dashboards Grafana live.
- **J-M11.3 (fin S11)** : audit log + alertes + overhead < 2 % vérifié → prod-ready.

---

## 21. Plan de migration et rollout

### 21.1 Phase A — Dev internal (fin S9)

- `efe-server` local utilise `TelemetryConfig::default()` (stdout JSON +
  `/metrics` sur 9090, pas d'OTLP).
- L'équipe core teste les logs JSON et les dashboards localement via
  `docker-compose` qui spawne Prometheus.

### 21.2 Phase B — Staging (fin S10)

- OTLP collector déployé en sidecar (opentelemetry-collector-contrib).
- Config staging : `TelemetryConfig { otlp: Some(...), environment: "staging" }`.
- Traces visibles dans Grafana Tempo après 1 run du workflow CI e2e.

### 21.3 Phase C — Production interne (fin S11)

- Audit log monté sur volume persistant (`/var/log/efe/audit.log.jsonl`).
- Alertes Prometheus → PagerDuty avec runbooks liés.
- Rotation horaire + retention 30 jours via logrotate sidecar.

### 21.4 Rollback

Feature flag `telemetry.enabled` dans config. Désactivation → le
collecteur retombe sur `NullTelemetryCollector` (déjà existant dans
efe-core). Pas d'impact sur la logique métier.

---

## 22. Matrice de risques et mitigations

| # | Risque | Proba | Impact | Mitigation |
|---|---|---|---|---|
| R1 | Overhead > 2 % | Moyenne | Élevé | Bench CI obligatoire ; budget §17. |
| R2 | Cardinality explosion (label tenant) | Moyenne | Élevé | Labels finite sets ; alerte `Prometheus TSDB head exceeded`. |
| R3 | PII leak via log field | Faible | Critique | Blocklist + CI grep + tests. |
| R4 | OTLP collector indisponible → spans perdus | Moyenne | Moyen | Batch processor avec retry ; logs WARN sur backpressure. |
| R5 | Audit log disk full | Moyenne | Moyen | Rotation horaire + alerte disk > 80 %. |
| R6 | Metrics registry collision (double register) | Faible | Moyen | `register_all` fait via `OnceLock`. |
| R7 | Span non-fermé → memory leak OTel | Faible | Élevé | `Drop` impl sur guard force `end()`. |
| R8 | `tracing-opentelemetry` version drift | Faible | Moyen | Pin versions workspace ; CI matrix. |
| R9 | SIGTERM flush timeout | Moyenne | Moyen | `flush_timeout` configurable ; warn si atteint. |
| R10 | Noms de métriques changent entre versions | Faible | Élevé | Contrat figé + ADR obligatoire. |

---

## 23. Critères d'acceptation

### 23.1 Fonctionnels

- **CA-F1** : Un run d'`Execute` produit : 1 span `grpc.request`, 1 span
  `swarm.execute`, N spans enfants, 3+ log events JSON, 3+ métriques
  incrémentées.
- **CA-F2** : `/metrics` retourne tous les 12 instruments MVP avec
  valeurs cohérentes après 10 requêtes.
- **CA-F3** : `traceparent` entrant → trace_id propagé dans tous les
  logs et spans du même run.
- **CA-F4** : Chaque `RegistryEvent` M9 incrémente la bonne metric + log.
- **CA-F5** : `raw_prompt` ne figure jamais dans les logs stdout par
  défaut.
- **CA-F6** : SIGTERM → flush OTLP dans `flush_timeout`, pas de span
  perdu sur 100 runs.
- **CA-F7** : Overhead gRPC p99 < 2 % vs `NullTelemetryCollector`.
- **CA-F8** : 3 dashboards Grafana importables et fonctionnels sur
  Grafana 10+.
- **CA-F9** : 8 règles Prometheus syntaxiquement valides (`promtool
  check rules`).

### 23.2 Non-fonctionnels

- **CA-NF1** : Clippy pedantic -D warnings clean.
- **CA-NF2** : Couverture ≥ 80 % sur `efe-telemetry`.
- **CA-NF3** : `cargo doc` sans warning.
- **CA-NF4** : PII grep CI passe (aucun `raw_prompt` dans `tracing::*` sans
  `redact_*`).
- **CA-NF5** : JSON schema validator passe sur stdout d'un canary run.
- **CA-NF6** : Bench régression < 10 % d'une baseline S9.

### 23.3 Documentation

- **CA-D1** : ADR-012 "Telemetry architecture".
- **CA-D2** : Runbook `docs/runbooks/m11-observability.md` : "Debugging
  a high-latency request", "Adding a new metric".
- **CA-D3** : Schéma JSON public des logs.
- **CA-D4** : Liste des métriques + dashboards référencée dans
  `docs/api/telemetry.md`.

---

## 24. Annexes

### 24.1 Annexe A — Exemple end-to-end

#### A.1 Startup

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = TelemetryConfig {
        service_name: "efe-server".into(),
        environment: std::env::var("EFE_ENV").unwrap_or("dev".into()),
        log_format: LogFormat::Json,
        log_filter: "info,efe=debug".into(),
        log_prompts: false,
        metrics_addr: Some("0.0.0.0:9090".parse()?),
        otlp: std::env::var("EFE_OTLP_ENDPOINT").ok().map(|ep| OtlpConfig {
            endpoint: ep,
            timeout: Duration::from_secs(5),
            headers: HashMap::new(),
            environment: std::env::var("EFE_ENV").unwrap_or("dev".into()),
        }),
        audit_log_path: Some("/var/log/efe/audit.log.jsonl".into()),
        flush_timeout: Duration::from_secs(5),
    };
    let telemetry = efe_telemetry::init(cfg)?;
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "efe-server starting");
    // ... server setup ...
    server.serve(addr, shutdown).await?;
    telemetry.shutdown().await;
    Ok(())
}
```

#### A.2 Sample log line

```json
{"ts":"2026-05-14T12:34:56.789Z","level":"INFO","target":"efe_telemetry::events::registry_sink","message":"alignment breach","fields":{"tenant":"acme","task":"chat-qa","swarm_id":"01HWZR...","observed_delta":0.72,"cone_radius":0.5},"trace_id":"0af765...","span_id":"b7ad6b..."}
```

### 24.2 Annexe B — Glossaire

| Terme | Définition |
|---|---|
| OTel | OpenTelemetry, standard unifié pour traces + metrics + logs. |
| OTLP | OTel Protocol — transport gRPC/HTTP pour exporter vers un collector. |
| Span | Unité de travail dans un trace, avec parent/child. |
| Traceparent | Header W3C qui porte `trace_id` + `span_id`. |
| Exemplar | Trace liée à un bucket d'histogramme (pour drill-down). |
| Registry (Prom) | Collection centrale d'instruments à exposer. |
| Scrape | Action de Prometheus pull sur `/metrics`. |
| Redaction | Remplacement d'une valeur sensible par un marqueur. |

### 24.3 Annexe C — Variables d'environnement

| Var | Usage | Default |
|---|---|---|
| `EFE_ENV` | Environnement logique (dev/staging/prod) | `dev` |
| `EFE_LOG_FILTER` | `EnvFilter` passthrough | `info,efe=debug` |
| `EFE_LOG_PROMPTS` | Opt-in PII dans logs | `false` |
| `EFE_OTLP_ENDPOINT` | URL collector OTLP | unset (désactivé) |
| `EFE_METRICS_ADDR` | Bind Prometheus endpoint | `0.0.0.0:9090` |
| `EFE_AUDIT_LOG` | Path audit jsonl | `logs/audit.log.jsonl` |

### 24.4 Annexe D — FAQ

**Q1. Peut-on tourner sans Prometheus ?**
Oui, `metrics_addr = None` désactive le serveur HTTP. Les métriques
continuent d'être calculées en interne (registry in-memory) mais non
exposées.

**Q2. Quel est l'impact de `log_prompts = true` ?**
Activation produit un WARN continu tant qu'elle est active. Les prompts
sont sérialisés **sans redaction** — à réserver aux debug sessions dev.

**Q3. Est-ce que M11 dépend de tonic ?**
Indirectement via `opentelemetry-otlp`. `efe-telemetry` est utilisable
sans serveur gRPC (ex. batch workers post-MVP).

**Q4. Les spans sont-ils bloquants ?**
Non. Le `BatchSpanProcessor` d'OTel SDK bufferise en mémoire et exporte
en background tokio task. L'échec d'export n'impacte pas le runtime.

**Q5. Comment tester sans collecteur OTLP ?**
Laisser `otlp = None` en dev. Les spans restent visibles en logs via
`tracing-subscriber` (ils apparaissent comme events).

**Q6. Peut-on filtrer finement les logs par module ?**
Oui : `EFE_LOG_FILTER="warn,efe_core::fusion=trace,efe_server=debug"`.

**Q7. Format du trace_id dans les logs ?**
Hex sans tiret, 32 chars, conforme W3C. Compatible Grafana Tempo.

**Q8. Metric cardinality safe ?**
Les labels utilisent des ensembles finis : `tenant` (bounded par M10
auth), `method` (statique depuis proto), `code` (codes gRPC statiques),
`kind` (EfeError variants), `reason` (I1..I7 + 2 autres), `context`
(dissolution|fusion|manual). Pas de cardinality explosion possible.

**Q9. Impact sur la facturation cloud (logs egress) ?**
Logs stdout taillés pour < 500 bytes / request sur le chemin heureux.
Environ 10 KB / request en WARN/ERROR paths. Budget 100 req/s → 50 KB/s.
Acceptable.

**Q10. Que faire si l'OTLP collector plante ?**
`BatchSpanProcessor` retient jusqu'à 2048 spans en buffer, puis drop-old.
Une metric `otlp_export_failures_total` (non-MVP) mesure le taux.

### 24.5 Annexe E — Checklist revue PR M11

- [ ] `cargo fmt --check` OK
- [ ] `cargo clippy -D warnings` OK
- [ ] `cargo test -p efe-telemetry` OK
- [ ] `cargo doc --no-deps` sans warning
- [ ] Toute nouvelle metric a un nom `efe_<module>_<metric>_<unit>`
- [ ] Toute nouvelle metric a un bloc de doc
- [ ] Tout nouveau span a son key dans `span_attrs::attrs::*`
- [ ] PII grep CI passe
- [ ] Tests d'intégration MVP couverts
- [ ] ADR mis à jour si changement de nom ou de format
- [ ] Dashboards mis à jour si nouveau métrique pertinent
- [ ] CHANGELOG.md mis à jour

---

**Fin du document M11_ARCHITECTURE.md — v1.0.**

| Rôle | Nom | Date |
|---|---|---|
| Auteur | Platform / Observability Team | 2026-04-19 |
| Reviewer | SRE Lead | _pending_ |
| Approbateur | Platform Director | _pending_ |
