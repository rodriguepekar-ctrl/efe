# Sprint D13-bis — Architecture détaillée : OpenTelemetry SDK réel

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : termine la dette technique laissée par D13. D13 a
> livré 8 dashboards Grafana, 8 nouveaux instruments Prometheus,
> 7-segment span name constants (`efe.swarm.*`, `efe.catalog.*`, …),
> une attribute whitelist + une bootstrap RAII guard. Mais l'init
> OTel SDK lui-même est resté un **no-op qui retourne
> `OtelInitError::SdkNotImplemented`** :
>
> ```rust
> // crates/efe-telemetry/src/otel/mod.rs (état v0.1.13)
> pub fn init(_cfg: &OtelConfig) -> Result<(), OtelInitError> {
>     Err(OtelInitError::SdkNotImplemented)
> }
> ```
>
> Conséquence opérationnelle : aucun span n'est exporté vers Tempo
> / Jaeger / Honeycomb. Les opérateurs voient les métriques
> Prometheus + les logs JSONL mais le **panneau « traces » des
> dashboards D13 reste vide**. La spec D13 § 7 documentait déjà la
> stratégie cible (sampler head-based 5 %, OTLP/gRPC, attribute
> whitelist) ; il manque juste le câblage des crates `opentelemetry`,
> `opentelemetry_sdk`, `opentelemetry-otlp`, `tracing-opentelemetry`.
>
> **Pré-requis** : avoir lu
> [`d13_observability_dashboards_architecture.md`](d13_observability_dashboards_architecture.md)
> § 7 (stratégie OTel cible), § 8 (span attribute whitelist),
> § 12 (boot sequence + shutdown). Référence rapide aux sprints
> postérieurs qui ont pondu des spans :
> [D14](d14_postgres_catalog_backend_architecture.md) (`efe.pg.*`),
> [D14-bis](d14bis_hmac_audit_chain_architecture.md) (`efe.audit.*`),
> [D15](d15_onboarding_idp_architecture.md) (`efe.oidc.*`,
> `efe.jwks.*`, `efe.tenant.*`),
> [D16](d16_embedder_bundling_architecture.md) (`efe.bundle.*`,
> `efe.embedder.*`).
>
> **Mission** : remplacer le no-op par un export OTel **production-grade**
> qui (a) supporte l'OTLP-gRPC vers un collector configurable,
> (b) applique le sampler doc-spec (head-based parent-based 5 %
> + force-sample on error), (c) injecte les resource attributes
> (`service.name`, `service.version`, `deployment.environment`),
> (d) bridge `tracing::span!` → OTel sans qu'aucune handler code
> ne change, (e) flush propre au shutdown M10, (f) auto-observable
> (les erreurs d'export sont elles-mêmes mesurées par 4 nouvelles
> métriques Prometheus D13-bis), et (g) reste **opt-in via
> `EFE_OTEL_ENABLED=true`** — un déploiement existant non-OTel
> n'est pas affecté.
>
> **Pain points opérationnels résolus** :
>
> 1. **Panneau Tempo vide en prod** : impossible aujourd'hui de
>    correlate un Prometheus alert burst (e.g. `D14bisAuditSignFailing`)
>    avec un trace ID pour drill-down. SOC2 / ISO 27001 demandent
>    cette traçabilité — sans elle, audit obs partial.
> 2. **Pas de cross-service tracing** : un appel client →
>    efe-server → catalog Postgres → audit HMAC sink fait 3-4 hops
>    SQL en parallèle. Sans OTel propagation (W3C traceparent
>    header → SQL connection-attribute), impossible de voir l'arbre
>    complet sur un trace de tail latency.
> 3. **Sampling cost incontrôlable** : sans sampler, 100 % des spans
>    seraient exportés. À 1 000 RPS × 30 spans / RPC × 30 réplicas
>    HPA ≈ 900 k spans/s. Volume Tempo ingest insoutenable.
>    L'archi cible D13 § 7 a documenté un sampler head-based 5 %,
>    jamais implementé.
> 4. **Sub-100 ms tail-latency invisible** : les histogrammes
>    Prometheus disent que p99 catalog `Register` ≈ 87 ms, mais
>    sans trace on ne sait pas si c'est le `pg.register` ou le
>    `audit.sign` qui domine. D14-bis a bien défini les sub-spans,
>    juste sans backend pour les recevoir.
> 5. **Erreur silent avalée à l'export** : sans observabilité de
>    la pipeline OTel elle-même, une régression collector-side
>    (Tempo OOM, NetworkPolicy block) signifie *tous* les traces
>    perdus sans alerte. Les 4 D13-bis instruments ferment la
>    boucle : on monitore le moniteur.
> 6. **Pas de log → trace correlation** : les `tracing::info!`
>    JSON sortent dans stdout (Loki via promtail) mais sans
>    `trace_id`/`span_id` champs auto-injectés. Impossible de
>    faire join Loki ⇆ Tempo.
> 7. **Initialisation pas idempotente** : Le code D13 retourne
>    `Err` par défaut. Tout double-init (e.g. test parallèle avec
>    `bootstrap_observability` appelé 2 fois) explose sans
>    diagnostic clair. D13-bis ajoute le check + le warning.
> 8. **Shutdown sans force-flush** : le RAII guard `ObservabilityGuard`
>    stub aujourd'hui ne fait rien. Un SIGTERM perd les 5 dernières
>    secondes de spans dans le batch processor in-memory. Sur un
>    crash post-erreur, c'est précisément les spans qu'on voulait
>    correlate qui disparaissent.
>
> **Note importante sur le périmètre** : ce sprint **ne change pas**
> les semantics des métriques Prometheus existantes (D13/D14/D14bis/
> D15/D16). Les 39 instruments cumulés restent exportés via le port
> `:9090` Prometheus pull-based. D13-bis ajoute un export OTLP
> *parallèle* pour les **traces uniquement** ; l'export des métriques
> via OTLP est listé en non-objectif (§ 26.2) et reporté à D13-ter.
>
> **Note sur le bridge `tracing`** : le projet utilise massivement
> les macros `tracing::info!`, `tracing::warn!`, `tracing::span!`
> depuis M10. Le crate `tracing-opentelemetry` fournit un
> `OpenTelemetryLayer` qui interpose entre le `tracing::Subscriber`
> et le `TracerProvider` OTel : chaque entrée/sortie de span
> `tracing` génère un span OTel, héritage parent-enfant inclus.
> Aucun handler code ne doit changer pour D13-bis — la magie
> opère côté Layer composition dans `init.rs`.

---

## Table des matières

1. [Pain points + objectifs](#1-pain-points--objectifs)
2. [Threat model (T-13bis-X)](#2-threat-model-t-13bis-x)
3. [Crate selection — opentelemetry vs alternatives](#3-crate-selection--opentelemetry-vs-alternatives)
4. [Resource attributes (semantic conventions)](#4-resource-attributes-semantic-conventions)
5. [Sampler design — head-based parent-based + force-on-error](#5-sampler-design--head-based-parent-based--force-on-error)
6. [Trace pipeline (init → process → export)](#6-trace-pipeline-init--process--export)
7. [OTLP-gRPC exporter config](#7-otlp-grpc-exporter-config)
8. [`tracing` ↔ OTel bridge](#8-tracing--otel-bridge)
9. [Span name + attribute whitelist enforcement](#9-span-name--attribute-whitelist-enforcement)
10. [Error / panic capture (status_code = ERROR)](#10-error--panic-capture-status_code--error)
11. [Shutdown — force-flush + grace timeout](#11-shutdown--force-flush--grace-timeout)
12. [Self-observability — 4 nouvelles métriques D13-bis](#12-self-observability--4-nouvelles-métriques-d13-bis)
13. [Helm + collector deployment](#13-helm--collector-deployment)
14. [Configuration surface (env + values.yaml)](#14-configuration-surface-env--valuesyaml)
15. [Test plan](#15-test-plan)
16. [Migration D13 → D13-bis (zero-downtime opt-in)](#16-migration-d13--d13-bis-zero-downtime-opt-in)
17. [SLOs D13bis-OTL-*](#17-slos-d13bis-otl)
18. [Risk register & open questions](#18-risk-register--open-questions)
19. [Alternatives considered](#19-alternatives-considered)
20. [Glossaire](#20-glossaire)
21. [Plan de sprint D13-bis (4 semaines + 1 audit)](#21-plan-de-sprint-d13-bis-4-semaines--1-audit)
22. [Critères d'acceptation D13-bis](#22-critères-dacceptation-d13-bis)
23. [Anti-corruption — versioning OTel SDK](#23-anti-corruption--versioning-otel-sdk)
24. [Cardinality envelope](#24-cardinality-envelope)
25. [Trace volume / cost projection](#25-trace-volume--cost-projection)
26. [Non-objectifs explicites](#26-non-objectifs-explicites)

---

## 1. Pain points + objectifs

### 1.1 Statut quo (D13 v0.1.13 → v0.1.16)

```text
                    ┌──────────────────────────────────────────┐
                    │            OPERATEUR / ON-CALL           │
                    │  - Prometheus alert burst sur D14bis-3   │
                    │  - veut drill-down sur le span en faute  │
                    └───────────────────┬──────────────────────┘
                                        │
                                        │  "click here for trace"
                                        ▼
                    ┌──────────────────────────────────────────┐
                    │             Grafana D13 panel             │
                    │            "OTel Traces (Tempo)"          │
                    │     ┌────────────────────────────┐       │
                    │     │  No data points found.      │ ← !  │
                    │     │  This data source did not   │       │
                    │     │  return any traces.         │       │
                    │     └────────────────────────────┘       │
                    └──────────────────────────────────────────┘
```

Le panneau est *configuré* (Tempo data source pointing at the
collector OTLP endpoint) mais **vide** parce que les pods
`efe-server` n'envoient aucun span. L'init OTel SDK retourne
`SdkNotImplemented` ; le `tracing-opentelemetry` Layer n'est
jamais composé sur le subscriber.

### 1.2 Pourquoi D13 a différé l'SDK

D13 a livré **l'infrastructure** (config struct + env vars + RAII
guard) mais a différé l'init pour deux raisons documentées :

1. **Crate footprint** — `opentelemetry-otlp = "0.27"` (v0.1.13
   period) tire `tonic = "0.10"` qui ne matchait pas la version
   `tonic = "0.11"` que M10 utilise. Le mismatch versionning
   aurait demandé un tonic-bump à coordonner avec le D9 Helm
   chart. D13 a préféré stabiliser le pull-path Prometheus +
   les dashboards d'abord et différer la couche OTLP.

2. **Pas de spans à exporter** — au moment de D13, seuls les
   spans `efe.swarm.*` + `efe.embedder.*` existaient (M10 + D7).
   Démontrer un OTel export propre demandait une chaîne d'au
   moins 3 spans imbriqués (parent → child → grandchild). D14
   (`efe.pg.*`), D14-bis (`efe.audit.*`), D15 (`efe.oidc.*`,
   `efe.jwks.*`, `efe.tenant.*`), D16 (`efe.bundle.*`,
   `efe.embedder.tokenize/forward/pool`) ont depuis fait croître
   le span-name catalog à **27 noms canoniques**. La chaîne est
   maintenant suffisamment riche pour justifier l'export.

À v0.1.16 ces deux blocs sont levés : tonic 0.11 est stable, et
les spans existent en abondance.

### 1.3 Pain points concrets résolus par D13-bis

| # | Pain | Status quo | Fix D13-bis |
|---|---|---|---|
| 1 | Prom alert → trace correlation | Manuel (grep logs par timestamp) | trace_id-injecté in JSON logs ; 1-click depuis alert |
| 2 | Cross-service trace | impossible | W3C traceparent propagé client → server → SQL |
| 3 | Sampling cost | 100 % par défaut | 5 % head-based + 100 % errors |
| 4 | Tail-latency drill-down | Prom histograms only | Span hierarchy avec sub-100ms par étape |
| 5 | Pipeline self-obs | aucune | 4 metrics D13-bis (export_total, export_duration, batch_size, dropped) |
| 6 | Log ↔ trace correlation | absent | `trace_id` + `span_id` auto-injectés |
| 7 | Double-init silencieux | crash sans contexte | `OtelInitError::AlreadyInitialised` |
| 8 | Shutdown perd les batchs | oui | force_flush + Drop guard wait jusqu'à `shutdown.grace_period` |

### 1.4 Objectifs mesurables

| ID | Objectif | Métrique | Cible |
|---|---|---|---|
| **O-13bis-1** | Spans exportés au boot | `efe_otel_export_total{result="ok"}` | rate ≥ 1/min sustained |
| **O-13bis-2** | Export latency p99 | `efe_otel_export_duration_seconds` p99 | ≤ 2 s |
| **O-13bis-3** | Sampling effective | spans-emitted / spans-sampled ratio | ≈ 0.05 (head) + force-error spans |
| **O-13bis-4** | Trace ID in logs | `trace_id` field present in 100 % of structured logs when sampled | 100 % when `EFE_OTEL_ENABLED=true` |
| **O-13bis-5** | Shutdown flush | spans dropped at shutdown | 0 (provided collector reachable) |
| **O-13bis-6** | Compat D13 | tous les tests D13 → D16 verts inchangés | 0 régression |
| **O-13bis-7** | Cardinalité bornée | nouvelles séries Prometheus | ≤ 100 séries D13-bis |

---

## 2. Threat model (T-13bis-X)

### 2.1 Catalogue des menaces (compromission OTel pipeline)

| ID | Menace | Vecteur | Impact | Mitigation D13-bis |
|---|---|---|---|---|
| **T-13bis-1** | Collector compromis exfiltre des spans | OTLP collector takeover | Données business (catalog name, tenant_id, query text) leakée | Attribute whitelist § 9 — JAMAIS PII dans les attrs ; `efe.embedder.encode` n'attache pas le texte source |
| **T-13bis-2** | Attaquant inject des spans forgés via `traceparent` | Header non-validé | Pollution dashboard, faux positifs alerte | jsonwebtoken-style validation de l'attribute set sur entrée ; rate-limit M10 caps inject volume |
| **T-13bis-3** | OTel TLS misconfig leak token | Endpoint http:// | trace_id + span attributes en clair | env validation : refuse `EFE_OTEL_ENDPOINT` non-https en prod tier |
| **T-13bis-4** | Sampler biais → tail latency hide | Sampler ne sélectionne jamais les slow paths | p99 invisible | Tail-based sampler (force-sample > 1 s) + force-on-error |
| **T-13bis-5** | Span attributes leaks PII | Handler attache `email` ou `text` | RGPD violation | Forbidden attr list § 9.4 ; pin test refuse PII keys |
| **T-13bis-6** | DoS via batch processor backpressure | Collector slow → batch queue overflow → OOM pod | Pod restart loop | Bounded queue (`max_queue_size=4096`) + drop-on-full + metric `efe_otel_dropped_total` + alert |
| **T-13bis-7** | Trace volume → bandwidth bill explosion | No sampling cap | Cloud cost surge | Sampler 5 % default ; tail-based escape hatch ; cost projection § 25 |
| **T-13bis-8** | OTLP collector unauthenticated | Anyone on cluster network can push spans | Injection / spoofing | Future auth (D13-ter mTLS) ; today rely on K8s NetworkPolicy |
| **T-13bis-9** | OTel SDK CVE | Upstream `opentelemetry` crate vuln | RCE / data leak | Pin in Cargo.lock ; renovate bot ; `cargo audit` in CI |
| **T-13bis-10** | Incomplete shutdown loses last-second spans | Drop-without-flush | Last 5s before crash invisible | RAII guard force_flush + wait |

### 2.2 Hors scope D13-bis

- **mTLS to collector** — relies on cluster-internal trust ;
  defer to D13-ter when crossing trust boundaries.
- **Trace-level access control** — collector-side concern.
- **Tail-based sampling at the collector** — currently we
  sample at the SDK ; smarter aggregation belongs in the
  collector deployment.

### 2.3 Trust boundaries

```
                         ┌────────────────────────────┐
                         │     efe-server pod         │
                         │  (zero-trust runtime)      │
                         │                            │
                         │  - tracing::span emits     │
                         │  - OpenTelemetryLayer adds │
                         │  - BatchSpanProcessor ←┐   │
                         │  - TonicExporter      │   │   in-process trust
                         │       │  TLS gRPC      │   │
                         └───────┼───────────────┼───┘
                                 │               │
                       OTLP/gRPC over TLS        │
                       (W3C traceparent ctx)     │
                                 │               │
                                 ▼               │
                         ┌────────────────────────────┐
                         │   OTel Collector pod       │
                         │ (semi-trusted)             │
                         │                            │
                         │  - receivers/otlp           │
                         │  - exporters/tempo          │
                         │  - filters: drop-PII fallback│
                         └───────────────┬────────────┘
                                         │
                                         ▼
                              Tempo / Jaeger / Honeycomb
                                  (storage tier)
```

---

## 3. Crate selection — `opentelemetry` vs alternatives

### 3.1 Décision : `opentelemetry` ecosystem (RustCrypto-style modular)

| Crate | Version cible v0.1.17 | Rôle |
|---|---|---|
| `opentelemetry` | 0.27 | API + Context + KeyValue types |
| `opentelemetry_sdk` | 0.27 | TracerProvider + BatchSpanProcessor + Sampler |
| `opentelemetry-otlp` | 0.27 | OTLP/gRPC exporter (tonic 0.11 backend) |
| `opentelemetry-semantic-conventions` | 0.27 | Constants pour `service.name` etc. |
| `tracing-opentelemetry` | 0.28 | Bridge `tracing::Subscriber` ↔ OTel |

**Justification** :
- Maintenu par le CNCF OpenTelemetry working group (tier-1).
- API match l'OTel spec mainline (langue de référence Rust).
- `opentelemetry-otlp` ships avec `tonic = "0.11"` matching M10.
- `tracing-opentelemetry` est l'unique pont Rust mature ; aucune
  alternative crédible.

### 3.2 Alternatives rejetées

| Crate | Pourquoi non |
|---|---|
| `tracing-honeycomb-rust` | Vendor lock-in, non-OTLP |
| `datadog-tracer` | Idem |
| Direct gRPC client + protobuf hand-roll | Re-implementation des semantic conventions ; maintenance impossible |
| `opentelemetry-stdout` (debug) | Reserved for tests § 15 ; not a prod backend |
| `console-subscriber` | Tokio-internal debugging, not OTel-compat |

### 3.3 Cargo.toml additions (prévisions)

```toml
# crates/efe-telemetry/Cargo.toml — additions sprint D13-bis
opentelemetry = { version = "0.27", default-features = false, features = ["trace", "metrics"] }
opentelemetry_sdk = { version = "0.27", default-features = false, features = ["trace", "rt-tokio"] }
opentelemetry-otlp = { version = "0.27", default-features = false, features = ["grpc-tonic", "trace", "tls", "tls-roots"] }
opentelemetry-semantic-conventions = "0.27"
tracing-opentelemetry = { version = "0.28", default-features = false }
```

Estimated binary size delta : +12 MiB compressed image layer
(tonic + tower + http + h2 are already present from M10 ;
opentelemetry adds ~8 MiB SDK + ~4 MiB exporter).

### 3.4 Versioning policy

- Pin `opentelemetry = "0.27"` in `Cargo.toml`. Use
  `Cargo.lock` to fix the exact patch version in CI.
- Renovate bot proposes minor bumps only ; majors require an
  audit pass + a CHANGELOG note (cf. § 23 anti-corruption).
- A CVE in any of the 5 crates is a P0 patch — bump immediately
  on `cargo audit` flag.

---

## 4. Resource attributes (semantic conventions)

### 4.1 Schéma cible

L'OTel spec demande qu'un Span carry un `Resource` (a set of
key-value attributes décrivant the source of the spans). On
suit les **semantic conventions** v1.27.0.

| Attribute key | Source | Exemple |
|---|---|---|
| `service.name` | `EFE_OTEL_SERVICE_NAME` env (default `efe-server`) | `"efe-server"` |
| `service.version` | `EFE_OTEL_SERVICE_VERSION` (default `CARGO_PKG_VERSION`) | `"0.1.17"` |
| `service.namespace` | constant `"efe"` | `"efe"` |
| `service.instance.id` | `HOSTNAME` env (K8s pod name) | `"efe-server-7f9b-xxxx"` |
| `deployment.environment` | `EFE_DEPLOY_ENV` env (default `unknown`) | `"production"` ou `"staging"` |
| `host.name` | from `gethostname()` | `"node-pool-blue-3"` |
| `host.arch` | from build target | `"aarch64"` |
| `os.type` | from build target | `"linux"` |
| `process.pid` | `std::process::id()` | `12345` |
| `efe.git_sha` | compile-time `GIT_SHA` env | `"abcdef0123…"` |
| `efe.deploy_tier` | `EFE_DEPLOY_TIER` env | `"tier2"` |

### 4.2 Implementation

```rust
// crates/efe-telemetry/src/otel/resource.rs (D13-bis nouveau fichier)

use opentelemetry::KeyValue;
use opentelemetry_sdk::Resource;
use opentelemetry_semantic_conventions::resource as sc;

/// Build the OTel Resource from env + compile-time constants.
/// Cf. architecture § 4.
pub fn build_resource(cfg: &OtelConfig) -> Resource {
    let mut attrs = vec![
        KeyValue::new(sc::SERVICE_NAME, cfg.service_name.clone()),
        KeyValue::new(sc::SERVICE_VERSION, cfg.service_version.clone()),
        KeyValue::new(sc::SERVICE_NAMESPACE, "efe"),
        KeyValue::new(sc::HOST_ARCH, std::env::consts::ARCH),
        KeyValue::new(sc::OS_TYPE, std::env::consts::OS),
        KeyValue::new(sc::PROCESS_PID, std::process::id() as i64),
    ];
    // service.instance.id from K8s pod name.
    if let Ok(hostname) = std::env::var("HOSTNAME") {
        attrs.push(KeyValue::new(sc::SERVICE_INSTANCE_ID, hostname.clone()));
        attrs.push(KeyValue::new(sc::HOST_NAME, hostname));
    }
    // deployment.environment from operator config.
    let env = std::env::var("EFE_DEPLOY_ENV").unwrap_or_else(|_| "unknown".into());
    attrs.push(KeyValue::new(sc::DEPLOYMENT_ENVIRONMENT_NAME, env));
    // EFE-specific (no SC equivalent).
    if let Some(git) = option_env!("GIT_SHA") {
        attrs.push(KeyValue::new("efe.git_sha", git));
    }
    if let Ok(tier) = std::env::var("EFE_DEPLOY_TIER") {
        attrs.push(KeyValue::new("efe.deploy_tier", tier));
    }
    Resource::new(attrs)
}
```

### 4.3 Pourquoi `service.instance.id` matters

Sans cet attribut, deux pods de la même `Deployment` produisent
des spans indistinguables côté Tempo. Avec K8s `HOSTNAME` on
obtient `efe-server-7f9b8c-xkz2p` qui est le pod-name unique,
permettant le drill-down par-pod.

### 4.4 Cardinality impact

Resource attrs **ne** comptent **pas** dans la cardinalité
Prometheus (ils sont attached au Resource, pas au span itself).
Cardinality côté Tempo dépend du backend (Tempo handles 100k+
distinct services no problem ; 5 attrs × 1k pods ≈ 5k unique
combos — trivial).

---

## 5. Sampler design — head-based parent-based + force-on-error

### 5.1 Stratégie

L'objectif : 5 % de sampling pour le steady state + 100 %
des erreurs / slow paths. Architecture D13 § 7 a déjà fixé la
politique ; D13-bis l'implémente.

```text
Span entry
    │
    ▼
[ParentBased]
 ├─ has parent ? → respect parent's sampling decision
 │               (so a sampled root keeps all children sampled)
 │
 └─ no parent (root span) ?
     │
     ▼
   [TraceIdRatioBased(0.05)]
        ├─ sample (~5 %) → record full
        └─ drop          → record nothing
        
            │
            ▼
        Always-on sampling (D13-bis post-decision)
        ├─ status_code == ERROR ? → force record (override)
        └─ duration > 1 s ?       → force record (override)
```

### 5.2 Pourquoi parent-based

W3C trace context propagation : un client externe peut envoyer
un `traceparent` header avec sampling decision déjà prise. Si
le client a sampled (e.g. parce qu'il debug un problème), on
veut **aussi** sampler nos spans pour que l'opérateur voie
l'arbre complet. Sinon le trace est tronqué.

### 5.3 Pourquoi 5 % et pas plus

À 1 000 RPS × 30 spans / RPC × 0.05 = **1 500 spans / s** par pod.
Sur 30 réplicas HPA = 45 k spans / s cluster. Tempo peut ingérer
~100 k spans/s sur une instance de taille moyenne — on a un
facteur 2 de marge, et les force-on-error ne devraient ajouter
que ~5 % de plus en steady state (taux d'erreur typique ~5 %
sur les API publiques).

### 5.4 Force-sample-on-error implementation

OpenTelemetry SDK ne supporte pas natively le "tail sampling"
(décider de sampler après que le span finit). Le `Sampler` trait
prend la décision à la création du span. Mais on peut forcer
via deux techniques :

1. **Set the sampling decision attribute** sur le span courant
   à `RECORD_AND_SAMPLE` quand on observe une erreur. Le
   `BatchSpanProcessor` exporte si la decision finale est
   `SAMPLED` à la fermeture du span.

2. **Custom Sampler** qui inspecte les attributes du span avant
   décision. On peut attacher `efe.force_sample = true` côté
   handler quand on détecte une erreur, et le sampler le
   respecte.

Pour D13-bis on adopte le **(1)** — plus simple, conforme à la
spec OTel. Implementation :

```rust
// in handler error path:
use opentelemetry::trace::TraceContextExt;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;

let cx = Span::current().context();
cx.span()
    .set_attribute(KeyValue::new("error", true));
// sampling decision is finalized at span end ; the BatchProcessor
// will keep it because it's marked errored.
```

Plus précisément : on utilise `span.record_error()` qui set
`status_code = ERROR` et le sampler peut alors override la
decision via un wrapper `ErrorBoostingSampler`. Code pattern :

```rust
// crates/efe-telemetry/src/otel/sampler.rs
use opentelemetry::trace::{Sampler, SamplingDecision, SamplingResult, TraceState};
use opentelemetry_sdk::trace::Sampler as SdkSampler;

pub struct ErrorBoostingSampler<S: SdkSampler> {
    inner: S,
}

impl<S: SdkSampler> SdkSampler for ErrorBoostingSampler<S> {
    fn should_sample(
        &self,
        parent_context: Option<&Context>,
        trace_id: TraceId,
        name: &str,
        kind: &SpanKind,
        attrs: &[KeyValue],
        links: &[Link],
    ) -> SamplingResult {
        // Force-record if the span is marked as errored at creation.
        if attrs.iter().any(|kv| kv.key.as_str() == "error" && kv.value == Value::Bool(true)) {
            return SamplingResult {
                decision: SamplingDecision::RecordAndSample,
                attributes: vec![KeyValue::new("efe.sample_reason", "error_boost")],
                trace_state: TraceState::default(),
            };
        }
        self.inner.should_sample(parent_context, trace_id, name, kind, attrs, links)
    }
}
```

### 5.5 Slow-path force-sample

La logique slow-path (`duration > 1 s → force sample`) requiert
le **post-decision** qui n'existe pas dans le SDK. On l'implémente
côté collector OTel via le `tail_sampling` processor :

```yaml
# Collector config (deployed via Helm sub-chart)
processors:
  tail_sampling:
    decision_wait: 10s
    policies:
      - name: error-policy
        type: status_code
        status_code: { status_codes: [ERROR] }
      - name: slow-policy
        type: latency
        latency: { threshold_ms: 1000 }
      - name: probabilistic-policy
        type: probabilistic
        probabilistic: { sampling_percentage: 5 }
```

Ce processor exécute le tail-sampling au niveau du collector
(qui voit tous les spans d'un trace après leur completion).
C'est la pratique standard OTel.

D13-bis livre le **head-based 5 %** côté SDK + le **error-boost**
au runtime ; le **tail-based slow-path** vit dans le Helm
sub-chart du collector (non-livrable D13-bis stricto, mais
documenté pour completeness § 13).

---

## 6. Trace pipeline (init → process → export)

### 6.1 Pipeline complète

```text
┌─────────────────────────────────────────────────────────────────────┐
│                                                                     │
│  tracing::span!(...) ─→ tracing::Subscriber                         │
│                              │                                      │
│                              ▼                                      │
│                       OpenTelemetryLayer                            │
│                       (tracing-opentelemetry crate)                 │
│                              │                                      │
│                              ▼                                      │
│                       OTel Tracer ── consults ──→ Sampler           │
│                              │              ├─ ParentBased          │
│                              │              ├─ TraceIdRatioBased    │
│                              │              └─ ErrorBoosting       │
│                              ▼                                      │
│                    Span (in memory)                                 │
│                              │                                      │
│                              ▼                                      │
│                    BatchSpanProcessor                               │
│                    (Tokio runtime ; bounded queue 4096)             │
│                              │                                      │
│                              ▼                                      │
│                    OtlpSpanExporter (OTLP/gRPC over tonic)          │
│                              │                                      │
│                              ▼                                      │
│                    OTel Collector (k8s sidecar or shared pod)       │
│                              │                                      │
│                              ├─→ Tempo backend                      │
│                              ├─→ tail_sampling processor            │
│                              └─→ structured logs (debug)            │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

### 6.2 BatchSpanProcessor settings

| Setting | Default OTel | D13-bis | Rationale |
|---|---|---|---|
| `max_queue_size` | 2048 | 4096 | Burst tolerance pour 30-pod HPA scale-up |
| `scheduled_delay` | 5 s | 1 s | Snappier dashboard freshness |
| `max_export_batch_size` | 512 | 512 | OTLP/gRPC sweet spot |
| `max_export_timeout` | 30 s | 5 s | Don't block shutdown more than necessary |

### 6.3 Init code skeleton

```rust
// crates/efe-telemetry/src/otel/mod.rs (D13-bis remplace le no-op)

use opentelemetry::global;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    runtime::Tokio,
    trace::{self, BatchConfig, Sampler as SdkSampler, TracerProvider},
};

pub struct OtelGuard {
    /// Provider held alive ; drop calls force_flush + shutdown.
    provider: TracerProvider,
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        let _ = self.provider.shutdown();
    }
}

pub fn init(cfg: &OtelConfig) -> Result<OtelGuard, OtelInitError> {
    if !cfg.enabled {
        return Err(OtelInitError::SdkNotImplemented); // re-purposed:
        // means "no-op as configured"
    }
    if cfg.endpoint.is_empty() {
        return Err(OtelInitError::MissingEndpoint);
    }

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&cfg.endpoint)
        .with_timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| OtelInitError::ExporterBuild(e.to_string()))?;

    let sampler = build_sampler(cfg);
    let resource = resource::build_resource(cfg);

    let provider = TracerProvider::builder()
        .with_resource(resource)
        .with_sampler(sampler)
        .with_batch_exporter(
            exporter,
            Tokio,
            BatchConfig::default()
                .with_max_queue_size(4096)
                .with_scheduled_delay(std::time::Duration::from_secs(1))
                .with_max_export_batch_size(512)
                .with_max_export_timeout(std::time::Duration::from_secs(5)),
        )
        .build();

    // Set as the global tracer for tracing-opentelemetry to pick up.
    global::set_tracer_provider(provider.clone());

    // Set up W3C tracecontext propagator so we can extract / inject
    // the traceparent header on inbound and outbound calls.
    global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );

    Ok(OtelGuard { provider })
}

fn build_sampler(cfg: &OtelConfig) -> impl SdkSampler {
    let inner = trace::Sampler::ParentBased(Box::new(
        trace::Sampler::TraceIdRatioBased(cfg.sample_rate),
    ));
    if cfg.force_sample_on_error {
        ErrorBoostingSampler::new(inner)
    } else {
        // Erased to dyn ; let inner stand alone.
        inner
    }
}
```

### 6.4 Hook into the existing `tracing::Subscriber`

Le projet utilise déjà `tracing_subscriber::fmt()` pour les
logs JSON. D13-bis ajoute un OTel layer dans le **même**
subscriber sans casser le formatter existant :

```rust
// crates/efe-telemetry/src/init.rs (modification)
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::Registry;

pub fn init_tracing(otel_guard: &Option<OtelGuard>) {
    let fmt_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_writer(std::io::stdout);
    let env_filter = tracing_subscriber::EnvFilter::from_default_env();

    let registry = Registry::default()
        .with(env_filter)
        .with(fmt_layer);

    // D13-bis — add the OTel layer when guard is Some.
    if let Some(_guard) = otel_guard {
        let tracer = global::tracer("efe");
        let otel_layer = tracing_opentelemetry::layer()
            .with_tracer(tracer);
        registry.with(otel_layer).init();
    } else {
        registry.init();
    }
}
```

### 6.5 trace_id auto-injection in JSON logs

`tracing-opentelemetry` provides a `OpenTelemetrySpanExt::context()`
that surfaces the active span's `trace_id` and `span_id`. We
extend the `tracing_subscriber::fmt` layer to inject those as
top-level JSON fields :

```rust
// pseudo: inject trace_id into the JSON formatter
let fmt_layer = tracing_subscriber::fmt::layer()
    .json()
    .with_current_span(false) // we add manually
    .map_event_format(|fmt| {
        fmt.with_target(true).with_thread_ids(false)
    });

// Then use a small custom layer that, on each event, looks up the
// current span via tracing-opentelemetry::span::OpenTelemetrySpanExt
// and adds trace_id / span_id to the JSON envelope.
```

(Implementation detail : via a `tracing_subscriber::Layer` that
intercepts events, walks `OpenTelemetrySpanExt::context()`, and
records `trace_id` + `span_id` as fields. ~30 lines of code.)

---

## 7. OTLP-gRPC exporter config

### 7.1 Endpoint shape

```
EFE_OTEL_ENDPOINT=http://otel-collector.efe.svc:4317
```

- gRPC port `4317` (HTTP/2)
- HTTP port `4318` (alternative for less performant networks)

### 7.2 TLS

By default OTLP-gRPC over plaintext on `:4317`. For cross-cluster
exports, switch to TLS via `EFE_OTEL_ENDPOINT=https://...` and
the `tls-roots` feature embeds rustls-native roots. **In-cluster
collector with NetworkPolicy is the default** ; TLS is the
escape hatch for hosted Tempo (Grafana Cloud).

### 7.3 Batching defaults

Cf. § 6.2 — the `BatchSpanProcessor` accumulates spans and
flushes every 1 s OR when 512 spans are queued, whichever comes
first.

### 7.4 Backpressure & drops

When the queue is full (4096 spans) and the exporter can't
drain fast enough (collector slow / down), new spans are
**dropped**. We instrument this via
`efe_otel_dropped_total` (cf. § 12).

### 7.5 Retry policy

`opentelemetry-otlp` 0.27 doesn't natively retry on transient
gRPC errors. Operator strategy : monitor the
`efe_otel_export_total{result="error"}` rate ; if it rises
above 1 %, that's a collector problem to fix at the K8s level
(scale up collector, fix NetworkPolicy, etc.).

D13-bis does **not** add retry logic in the SDK ; that's
classified as collector-level concern.

---

## 8. `tracing` ↔ OTel bridge

### 8.1 Mapping table

| `tracing` concept | OTel equivalent |
|---|---|
| `tracing::span!` | `opentelemetry::trace::Span` |
| `Level::INFO` | OTel default |
| `Level::WARN` | event with severity warn |
| `Level::ERROR` | event with severity error + status |
| span field `key = value` | KeyValue attribute |
| `tracing::event!` | event on the current span |
| span enter / exit | OTel start / end |

### 8.2 What changes in handler code

**Nothing.** Handlers continue to use :

```rust
let _g = tracing::info_span!(
    crate::spans::CATALOG_REGISTER,
    tenant = %req.tenant,
    catalog_key = %ck,
).entered();

// ... business logic ...
```

The `OpenTelemetryLayer` automatically converts these to OTel
spans + attributes. No D14/D15/D16 service code needs to be
touched.

### 8.3 What changes in the binary boot

```rust
// crates/efe-server/src/main.rs (single edit)
fn main() -> anyhow::Result<()> {
    let cfg = ObservabilityConfig::from_env()?;
    let _obs_guard = bootstrap_observability(&cfg)?;
    // ... existing code unchanged ...
}
```

`bootstrap_observability` is the existing D13 entry point ; we
make it actually wire the OTel SDK when `cfg.otel.enabled`.

---

## 9. Span name + attribute whitelist enforcement

### 9.1 Status quo

D13 has pinned 16 span name constants and an attribute whitelist
(`spans.rs::ATTR_*`). Subsequent sprints have grown the list :

```text
M10  : 8 spans  (efe.swarm.*, efe.embedder.encode/load)
D7   : 4 spans  (efe.cone.*, efe.fusion.round)
D12  : 6 spans  (efe.catalog.*)
D14  : 7 spans  (efe.pg.*)
D14b : 2 spans  (efe.audit.sign/verify)
D15  : 3 spans  (efe.oidc.validate, efe.jwks.fetch, efe.tenant.onboard)
D16  : 7 spans  (efe.bundle.*, efe.embedder.tokenize/forward/pool)
─────────────────────────────────────────────
       37 canonical span names total
```

**D13-bis ne change pas la liste**. Le rôle de D13-bis est juste
de *transporter* ces spans vers le backend. Le whitelist test
existant (`test_d13_span_forbidden_attribute_set`) reste vert
inchangé.

### 9.2 Forbidden attribute names (rappel)

```rust
// crates/efe-telemetry/src/spans.rs
pub const FORBIDDEN_ATTR_PREFIXES: &[&str] = &[
    "request_id",
    "trace_id",     // (kept on the span itself, not as an attribute)
    "span_id",
    "text",
    "input",
    "raw_input",
    "email",
    "phone",
    "address",
    "jwt",
    "token",
    "password",
    "secret",
    "api_key",
    "customer_email",
];
```

D13-bis adds a runtime guard : if the operator tries to
attach a forbidden attribute via `tracing::field!` macro, the
OTel layer drops it with a `tracing::warn!` (defense in
depth — the dev should have caught it at compile-test).

### 9.3 Attribute drop implementation

```rust
// crates/efe-telemetry/src/otel/attribute_filter.rs
use opentelemetry::KeyValue;
use crate::spans::FORBIDDEN_ATTR_PREFIXES;

pub fn filter_attribute(kv: KeyValue) -> Option<KeyValue> {
    let key_str = kv.key.as_str();
    for forbidden in FORBIDDEN_ATTR_PREFIXES {
        if key_str.contains(forbidden) {
            tracing::warn!(
                target: "efe.otel.filter",
                forbidden_key = key_str,
                "dropping forbidden span attribute"
            );
            return None;
        }
    }
    Some(kv)
}
```

This is invoked from a custom `OpenTelemetryLayer` configuration.

### 9.4 Pin test

```python
# scripts/tests/test_d13bis_otel_attribute_filter.py
def test_forbidden_attrs_are_dropped():
    # Smoke : check the FORBIDDEN_ATTR_PREFIXES constant
    # is referenced by attribute_filter.rs.
    src = (REPO / "crates/efe-telemetry/src/otel/attribute_filter.rs").read_text()
    assert "FORBIDDEN_ATTR_PREFIXES" in src
```

---

## 10. Error / panic capture (status_code = ERROR)

### 10.1 OTel semantics

Each span has a `status_code` ∈ {`Unset`, `Ok`, `Error`}. OTel
dashboards filter on `status_code = ERROR` for the "error rate"
panel.

### 10.2 How `tracing-opentelemetry` maps

- `tracing::error!` event inside a span → adds an event with
  severity `ERROR` to the span.
- The span's overall `status_code` defaults to `Unset` ; we
  must explicitly set `Error` on the span when we want.

### 10.3 Convention for handler code

In every handler that `Result`-returns an `Err`, before
returning, mark the span :

```rust
match self.do_work(req).await {
    Ok(v) => Ok(v),
    Err(e) => {
        // Set span status to ERROR + attach error type as attr.
        tracing::Span::current().record("error", &true);
        tracing::error!(
            target: "efe.error",
            err = %e,
            "handler failed"
        );
        Err(e.into())
    }
}
```

### 10.4 Panic capture

`std::panic::catch_unwind` or a custom `panic_hook` sets the
current span's status to `Error` :

```rust
std::panic::set_hook(Box::new(|info| {
    if let Some(span) = tracing::Span::current().context().span() {
        span.set_status(opentelemetry::trace::Status::error(
            info.to_string(),
        ));
    }
    eprintln!("panic: {info}");
}));
```

This integrates with `efe_panics_total` (existing M10 metric)
without conflicting.

---

## 11. Shutdown — force-flush + grace timeout

### 11.1 Problem

Le SIGTERM arrive. The serve future returns. We want :

1. All spans currently in the BatchProcessor queue → force-flush
   to the collector.
2. If the collector is unreachable, time out gracefully
   (`shutdown.grace_period`).
3. Don't block process exit beyond `shutdown.force_abort_after`
   (M10 contract).

### 11.2 RAII guard implementation

```rust
// crates/efe-telemetry/src/otel/mod.rs (sequel to § 6.3)

impl OtelGuard {
    /// Manual shutdown — call from main BEFORE the runtime drops.
    /// Idempotent ; safe to call twice.
    pub async fn force_flush(&self, timeout: Duration) -> Result<(), OtelShutdownError> {
        let provider = self.provider.clone();
        let h = tokio::task::spawn_blocking(move || {
            provider.force_flush()
        });
        match tokio::time::timeout(timeout, h).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(OtelShutdownError::JoinError(e.to_string())),
            Err(_) => Err(OtelShutdownError::Timeout(timeout)),
        }
    }
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        // Best-effort sync flush. Caller should have called
        // force_flush(.) async first.
        let _ = self.provider.shutdown();
    }
}
```

### 11.3 Wire into M10 graceful shutdown

```rust
// crates/efe-server/src/runtime.rs (modification existante)
pub async fn serve(self, addr: SocketAddr, shutdown: Shutdown) -> ... {
    // ... existing serve logic ...
    
    // After the serve future returns, force-flush OTel.
    if let Some(guard) = self.otel_guard.as_ref() {
        let _ = guard
            .force_flush(self.config.shutdown.grace_period)
            .await;
    }
    Ok(())
}
```

### 11.4 Lost spans tracking

If `force_flush` times out, some spans were dropped. We bump
`efe_otel_dropped_total{reason="shutdown_timeout"}` (cf. § 12).

---

## 12. Self-observability — 4 nouvelles métriques D13-bis

### 12.1 Instruments

| Metric | Type | Labels | But |
|---|---|---|---|
| `efe_otel_export_total` | Counter | `result` | success / error rate of OTLP exports |
| `efe_otel_export_duration_seconds` | Histogram | `result` | export latency (target p99 < 2 s) |
| `efe_otel_batch_size` | Histogram | (none) | spans per batch (sanity) |
| `efe_otel_dropped_total` | Counter | `reason` | dropped spans (queue_full, shutdown_timeout, exporter_error) |

### 12.2 Cardinality envelope

| Instrument | Worst case | Total |
|---|---|---|
| `export_total` | 2 results | 2 |
| `export_duration` | 2 | 2 |
| `batch_size` | 1 | 1 |
| `dropped_total` | 3 reasons | 3 |

Total : **8 nouvelles séries**. Très en-dessous du budget D13.

### 12.3 Implementation

```rust
// crates/efe-telemetry/src/metrics/d13bis_instruments.rs (NOUVEAU FICHIER)

once_metric!(
    /// Export attempts to the OTLP collector.
    otel_export_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new("efe_otel_export_total", "OTLP exports by result"),
        &["result"],
    ).expect("opts")
);

once_metric!(
    /// Wall-clock duration of one batch export.
    otel_export_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_otel_export_duration_seconds",
            "OTLP export wall-clock duration",
        ).buckets(exponential_buckets(0.01, 2.0, 10).unwrap()),
        &["result"],
    ).expect("opts")
);

once_metric!(
    /// Spans per batch.
    otel_batch_size,
    Histogram,
    Histogram::with_opts(
        HistogramOpts::new(
            "efe_otel_batch_size",
            "Number of spans per OTLP export batch",
        ).buckets(linear_buckets(0.0, 64.0, 10).unwrap()),
    ).expect("opts")
);

once_metric!(
    /// Spans dropped before they reached the exporter.
    otel_dropped_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_otel_dropped_total",
            "Spans dropped (queue_full, shutdown_timeout, exporter_error)",
        ),
        &["reason"],
    ).expect("opts")
);

pub fn register_all_d13bis() {
    let _ = otel_export_total();
    let _ = otel_export_duration_seconds();
    let _ = otel_batch_size();
    let _ = otel_dropped_total();
}
```

### 12.4 Where to bump

`opentelemetry-otlp` doesn't expose metrics by default. We
wrap the exporter via a `MeasuringSpanExporter` that delegates
to the real one and observes :

```rust
// crates/efe-telemetry/src/otel/measuring_exporter.rs
pub struct MeasuringSpanExporter<E> {
    inner: E,
}

#[async_trait]
impl<E: SpanExporter> SpanExporter for MeasuringSpanExporter<E> {
    async fn export(&mut self, batch: Vec<SpanData>) -> Result<...> {
        let n = batch.len();
        otel_batch_size().observe(n as f64);
        let start = Instant::now();
        let res = self.inner.export(batch).await;
        let elapsed = start.elapsed();
        let label = if res.is_ok() { "ok" } else { "error" };
        otel_export_total().with_label_values(&[label]).inc();
        otel_export_duration_seconds()
            .with_label_values(&[label])
            .observe(elapsed.as_secs_f64());
        if res.is_err() {
            otel_dropped_total()
                .with_label_values(&["exporter_error"])
                .inc_by(n as u64);
        }
        res
    }
}
```

### 12.5 Eager registration

Cf. D15-H5 pattern. Register all 4 D13-bis instruments
unconditionally at boot so dashboards render zero-valued series
even when OTel is disabled.

---

## 13. Helm + collector deployment

### 13.1 OTel collector sub-chart

We add a **dependency** chart `otel-collector` to the EFE Helm
chart :

```yaml
# deploy/helm/efe/Chart.yaml — D13-bis additions
dependencies:
  - name: opentelemetry-collector
    version: "0.97.0"
    repository: https://open-telemetry.github.io/opentelemetry-helm-charts
    condition: otelCollector.enabled
```

### 13.2 values.yaml

```yaml
# deploy/helm/efe/values.yaml — D13-bis additions
config:
  observability:
    otel:
      # D13-bis — turn on the real OTLP export pipeline.
      enabled: false
      # Endpoint of the collector. Default points at the in-cluster
      # collector deployed by the sub-chart below.
      endpoint: "http://efe-otel-collector.efe.svc.cluster.local:4317"
      sampleRate: 0.05
      forceSampleOnError: true
      serviceName: "efe-server"
      # serviceVersion derives from CARGO_PKG_VERSION when empty.
      serviceVersion: ""

otelCollector:
  enabled: false
  # Sub-chart values follow opentelemetry-collector chart spec.
  config:
    receivers:
      otlp:
        protocols:
          grpc:
            endpoint: 0.0.0.0:4317
    processors:
      batch: {}
      tail_sampling:
        decision_wait: 10s
        policies:
          - name: errors
            type: status_code
            status_code: { status_codes: [ERROR] }
          - name: slow
            type: latency
            latency: { threshold_ms: 1000 }
          - name: probabilistic
            type: probabilistic
            probabilistic: { sampling_percentage: 100 }
    exporters:
      otlp/tempo:
        endpoint: tempo-distributor.observability.svc:4317
        tls: { insecure: true }
    service:
      pipelines:
        traces:
          receivers: [otlp]
          processors: [batch, tail_sampling]
          exporters: [otlp/tempo]
```

### 13.3 NetworkPolicy

Add an egress rule allowing `efe-server` pods to reach
`efe-otel-collector` on port 4317 :

```yaml
# deploy/helm/efe/templates/networkpolicy-otel.yaml (NOUVEAU)
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: {{ include "efe.fullname" . }}-otel-egress
spec:
  podSelector:
    matchLabels: {{- include "efe.selectorLabels" . | nindent 6 }}
  policyTypes: [Egress]
  egress:
    - to:
        - podSelector:
            matchLabels:
              app.kubernetes.io/name: opentelemetry-collector
      ports:
        - protocol: TCP
          port: 4317
```

### 13.4 ConfigMap env vars

```yaml
# deploy/helm/efe/templates/configmap.yaml — D13-bis additions
{{- if .Values.config.observability.otel.enabled }}
EFE_OTEL_ENABLED: "true"
EFE_OTEL_ENDPOINT: {{ .Values.config.observability.otel.endpoint | quote }}
EFE_OTEL_SAMPLE_RATE: {{ .Values.config.observability.otel.sampleRate | quote }}
EFE_OTEL_FORCE_SAMPLE_ON_ERROR: {{ .Values.config.observability.otel.forceSampleOnError | quote }}
EFE_OTEL_SERVICE_NAME: {{ .Values.config.observability.otel.serviceName | quote }}
{{- if .Values.config.observability.otel.serviceVersion }}
EFE_OTEL_SERVICE_VERSION: {{ .Values.config.observability.otel.serviceVersion | quote }}
{{- end }}
{{- end }}
```

### 13.5 Pod env injection

Already covered by the standard ConfigMap → envFrom
projection in the deployment. No changes needed.

---

## 14. Configuration surface (env + values.yaml)

### 14.1 Env vars (alphabetical)

| Env | Type | Default | Sense |
|---|---|---|---|
| `EFE_DEPLOY_ENV` | string | `unknown` | resource attr `deployment.environment` |
| `EFE_DEPLOY_TIER` | string | (unset) | resource attr `efe.deploy_tier` |
| `EFE_OBSERVABILITY_FAIL_ON_OTEL_ERROR` | bool | `false` | hard-fail boot if OTel init fails |
| `EFE_OTEL_ENABLED` | bool | `false` | master switch |
| `EFE_OTEL_ENDPOINT` | URL | (empty) | OTLP/gRPC endpoint |
| `EFE_OTEL_FORCE_SAMPLE_ON_ERROR` | bool | `true` | error-boost the sampler |
| `EFE_OTEL_SAMPLE_RATE` | f64 | `0.05` | head-based ratio ∈ [0, 1] |
| `EFE_OTEL_SERVICE_NAME` | string | `efe-server` | resource attr |
| `EFE_OTEL_SERVICE_VERSION` | string | `CARGO_PKG_VERSION` | resource attr |

### 14.2 Validation invariants (`config.rs` extension)

```rust
impl ServerConfig {
    fn validate_otel(&self) -> Result<(), ConfigError> {
        if self.otel.enabled {
            if self.otel.endpoint.is_empty() {
                return Err(ConfigError::invalid(
                    "otel.endpoint",
                    "must be non-empty when otel.enabled (set EFE_OTEL_ENDPOINT)",
                ));
            }
            if !self.otel.endpoint.starts_with("http://") &&
               !self.otel.endpoint.starts_with("https://") &&
               !self.otel.endpoint.starts_with("grpc://") {
                return Err(ConfigError::invalid(
                    "otel.endpoint",
                    "must be a URL (http://, https://, or grpc://)",
                ));
            }
            if !(0.0..=1.0).contains(&self.otel.sample_rate) {
                return Err(ConfigError::invalid(
                    "otel.sample_rate",
                    "must be in [0, 1]",
                ));
            }
            // Production tier safety : refuse plaintext http:// when
            // deploy_tier=production (defense in depth — operator should
            // use TLS to a remote collector).
            if matches!(self.deploy_tier, DeployTier::Production)
                && self.otel.endpoint.starts_with("http://")
                && !self.otel.endpoint.contains("svc.cluster.local")
            {
                return Err(ConfigError::invalid(
                    "otel.endpoint",
                    "plaintext http:// only allowed for in-cluster collector ; \
                     use https:// for external endpoints in production tier",
                ));
            }
        }
        Ok(())
    }
}
```

(Note : `DeployTier` doesn't currently exist in M12 config —
parking this validation behind a `cfg!(feature = "deploy_tier")`
flag and reintroducing it as part of D17 deploy-tier work.)

---

## 15. Test plan

### 15.1 Unit tests (Rust)

| Test | Description |
|---|---|
| `otel_init_disabled_returns_no_op` | `enabled=false` → no SDK |
| `otel_init_enabled_builds_provider` | `enabled=true` + valid endpoint → provider exists |
| `otel_init_enabled_no_endpoint_errs` | `enabled=true` + empty endpoint → error |
| `otel_init_invalid_sample_rate_errs` | `sample_rate=1.5` → error |
| `otel_init_idempotent_double_call` | calling `init` twice → second errors `AlreadyInitialised` |
| `error_boosting_sampler_records_errored_spans` | with `error=true` attr → RecordAndSample |
| `error_boosting_sampler_passthrough_normal` | normal span → forward to inner sampler |
| `attribute_filter_drops_forbidden_keys` | `email=...` attr → dropped + warn |
| `resource_includes_service_name` | resource has `service.name` |
| `resource_includes_hostname_when_set` | env `HOSTNAME=foo` → resource has `service.instance.id=foo` |

### 15.2 Integration test (no real collector)

```rust
// crates/efe-telemetry/tests/d13bis_otel_integration.rs

/// Use the in-memory `opentelemetry-stdout` exporter to assert
/// the pipeline end-to-end without needing a live collector.
#[tokio::test]
async fn span_emitted_via_pipeline_e2e() {
    let cfg = OtelConfig {
        enabled: true,
        endpoint: "http://localhost:4317".into(),
        sample_rate: 1.0, // full sample
        ..Default::default()
    };
    // Use a custom exporter that captures spans in a Vec.
    let captured: Arc<Mutex<Vec<SpanData>>> = Arc::new(Mutex::new(vec![]));
    let exporter = TestCapturingExporter { sink: captured.clone() };
    let provider = build_provider_with_exporter(&cfg, exporter);
    global::set_tracer_provider(provider.clone());
    
    {
        let _g = tracing::info_span!("efe.swarm.classify", tenant = "acme").entered();
    } // span exits → recorded
    
    provider.force_flush();
    let spans = captured.lock().unwrap();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name, "efe.swarm.classify");
}
```

### 15.3 Pin tests (Python)

```python
# scripts/tests/test_d13bis_otel_sdk.py

def test_d13bis_init_replaces_sdk_not_implemented():
    """The OTel init function must no longer return SdkNotImplemented
    when enabled=true."""
    rs = (REPO / "crates/efe-telemetry/src/otel/mod.rs").read_text()
    init_fn = re.search(r"pub fn init.*?\n\}", rs, re.DOTALL).group()
    # Allow SdkNotImplemented only as the disabled-path return.
    enabled_branch = re.search(r"if !cfg\.enabled.*?\n.*?Err.*?SdkNotImplemented", init_fn, re.DOTALL)
    assert enabled_branch is None, (
        "init() still returns SdkNotImplemented in the enabled branch — D13-bis fix regressed"
    )

def test_d13bis_resource_includes_required_attrs():
    """Resource builder MUST include service.name + service.version
    + deployment.environment."""
    rs = (REPO / "crates/efe-telemetry/src/otel/resource.rs").read_text()
    for required in ["SERVICE_NAME", "SERVICE_VERSION", "DEPLOYMENT_ENVIRONMENT"]:
        assert required in rs

def test_d13bis_otel_helm_envvars_match_config_rs():
    """Standard env-var ↔ config.rs cross-validation."""
    cm = (CHART / "templates/configmap.yaml").read_text()
    rs = (REPO / "crates/efe-server/src/config.rs").read_text()
    for var in ["EFE_OTEL_ENABLED", "EFE_OTEL_ENDPOINT", "EFE_OTEL_SAMPLE_RATE"]:
        assert var in cm
        assert var in rs
```

### 15.4 Smoke test (live collector)

```bash
# scripts/d13bis/smoke.sh
docker run -d --rm --name otel-collector \
    -p 4317:4317 \
    otel/opentelemetry-collector-contrib:0.97.0 \
    --config=/etc/otel-collector-config.yaml

EFE_OTEL_ENABLED=true \
EFE_OTEL_ENDPOINT=http://localhost:4317 \
cargo run -p efe-server &
SERVER_PID=$!

# Send 100 RPCs.
grpcurl -plaintext -d @ localhost:50051 efe.v1.SwarmService/Encode <<EOF
{"text": "hello"}
EOF

sleep 5
docker logs otel-collector | grep "efe.swarm.classify" | wc -l
# Should be ~5 (5 % sampling of 100).

kill $SERVER_PID
docker stop otel-collector
```

### 15.5 Benchmarks

Baseline : measure `Encode` p99 latency :
- without OTel (current state)
- with OTel + sample_rate=0.0 (sampler decides drop)
- with OTel + sample_rate=0.05 + collector reachable
- with OTel + sample_rate=0.05 + collector down

Target : delta < 5 % p99 across all three OTel-on configurations.

---

## 16. Migration D13 → D13-bis (zero-downtime opt-in)

### 16.1 Two coexisting modes

| Mode | When | Behaviour |
|---|---|---|
| **D13 (default)** | `EFE_OTEL_ENABLED=false` | Prometheus + JSON logs only. Pre-D13-bis behaviour. |
| **D13-bis** | `EFE_OTEL_ENABLED=true` + endpoint set | OTLP export active. New baseline. |

The runtime starts in the operator-chosen mode. Switching is
just an env var flip + pod restart. No state migration needed
— spans aren't persistent.

### 16.2 Bascule plan (per cluster)

```text
J0    — push v0.1.17 image (D13-bis) to staging.
J+3   — staging cluster: enable for 1 hour ; verify Tempo
        receives spans + dashboards populate. Disable.
J+7   — staging cluster: enable permanently.
J+14  — prod tier-2 cluster: enable. Watch for 7 days.
J+30  — prod tier-3 cluster: enable.
```

### 16.3 Rollback

- Set `EFE_OTEL_ENABLED=false` + restart pod.
- Pre-flight metric : `efe_otel_export_total{result="error"}`
  rate < 0.1 % over 5 min before considering enable successful.

---

## 17. SLOs D13bis-OTL-*

| ID | SLO | Mesure | Cible |
|---|---|---|---|
| **D13bis-OTL-1** | Export success | `rate(efe_otel_export_total{result="ok"})` / `rate(total)` | ≥ 99.5 % over 5 min |
| **D13bis-OTL-2** | Export latency | `efe_otel_export_duration_seconds` p99 | ≤ 2 s |
| **D13bis-OTL-3** | Drop rate | `rate(efe_otel_dropped_total)` | ≤ 100 spans/s sustained |
| **D13bis-OTL-4** | Init time | wall-clock from `bootstrap_observability` to provider ready | ≤ 1 s |
| **D13bis-OTL-5** | Shutdown flush | `force_flush` returns Ok within `grace_period` | 99 % of shutdowns |

---

## 18. Risk register & open questions

### 18.1 Risks

| ID | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| **R-13bis-1** | Tonic version mismatch with M10 | low | high | Pin opentelemetry-otlp 0.27 (uses tonic 0.11) ; cargo check on bump |
| **R-13bis-2** | OTel collector OOM under burst | medium | medium | Bounded queue + drop-on-full + alert |
| **R-13bis-3** | Sampling under-records slow paths | medium | high | tail_sampling collector processor (§ 5.5) |
| **R-13bis-4** | Trace volume cost surprise | medium | medium | 5 % default + dashboard + alert |
| **R-13bis-5** | OTel SDK CVE | low | high | renovate bot + cargo audit CI |
| **R-13bis-6** | Span attribute leak (PII in error msg) | medium | critical | redaction.rs filter + attribute_filter.rs |
| **R-13bis-7** | Race in BatchProcessor at shutdown | low | medium | force_flush + bounded timeout |

### 18.2 Open questions

1. **Q-13bis-1** : faut-il aussi exporter les **metrics** via
   OTLP (parallèle au pull Prometheus) ? Réponse : non, D13-ter.
   Prometheus pull + Loki push sont déjà en place ; ajouter
   OTLP metrics double l'effort sans bénéfice immédiat.

2. **Q-13bis-2** : **logs** export via OTLP ? Idem, D13-ter.
   Les logs JSON via stdout → Loki (promtail) restent
   l'autorité pour D13-bis.

3. **Q-13bis-3** : faut-il un fallback `opentelemetry-stdout`
   exporter pour Tier 1 dev ? Oui — toggle via
   `EFE_OTEL_EXPORTER=stdout` (debug-only). Documenté en howto.

4. **Q-13bis-4** : SOC 2 demande une trace ID dans chaque ligne
   de log audit. Est-ce que D14-bis hmac chain doit aussi
   stocker `trace_id` dans la table `audit_events` ? Réponse :
   migration `0006_audit_events_add_trace_id.sql` recommandée
   en D13-ter (out of scope D13-bis).

5. **Q-13bis-5** : versionner le span name catalog ? Si on
   renomme `efe.catalog.register` → `efe.catalog.create`, les
   dashboards Grafana cassent. Réponse : maintenir un
   `SPAN_NAME_VERSION` constant dans `spans.rs` ; pin test
   refuse les renames sans bump.

6. **Q-13bis-6** : exemplars (Prometheus → trace_id) ? Le
   crate `prometheus = "0.13"` ne supporte pas natively les
   exemplars. Alternative : exposer une `_exemplars` métadonnée
   custom. Reporté D13-ter.

---

## 19. Alternatives considered

### 19.1 Pourquoi pas un sidecar collector ?

Considéré : un container `otel/opentelemetry-collector` dans
chaque pod. Avantage : pas besoin de NetworkPolicy egress.
Désavantage : ressource overhead 100 MB par pod × 30 pods =
3 GB de gaspillage. Adopté pattern : un Deployment partagé
de collector, scale séparé.

### 19.2 Pourquoi pas `tracing-subscriber::fmt` qui parle directement OTLP ?

`tracing-subscriber::fmt::Layer` est uniquement un formatter
de logs ; il n'émet pas de spans OTLP. Le pont
`tracing-opentelemetry::OpenTelemetryLayer` est le mécanisme
canonique.

### 19.3 Pourquoi pas Datadog / Honeycomb SDK direct ?

Vendor lock-in. OTLP est un standard ; le collector peut
exporter vers n'importe quel backend. Choisir vendor-direct
nous priverait de ce flexibility.

### 19.4 Pourquoi pas async-std runtime pour le BatchProcessor ?

Le projet est 100 % Tokio depuis M10. Mélanger les runtimes
casserait la compatibilité avec `tonic`, `sqlx`, et la
`Shutdown::notified()` infrastructure.

### 19.5 Pourquoi pas un sampler fully-tail-based côté SDK ?

OpenTelemetry SDK ne supporte pas le tail sampling (décision
post-completion). Implémenter en interne demanderait un
buffer in-memory de spans complets — incompatible avec le
pattern bounded-queue. Le tail sampling vit dans le collector
processor.

---

## 20. Glossaire

| Terme | Sens dans D13-bis |
|---|---|
| **OTLP** | OpenTelemetry Protocol — gRPC ou HTTP transport |
| **Span** | Unit of trace — un span = une opération |
| **Trace** | Arbre de spans liés par parent_id |
| **Sampler** | Fonction qui décide RecordAndSample / Drop |
| **TracerProvider** | Singleton qui produit les Tracer instances |
| **BatchSpanProcessor** | Async batcher qui envoie au collector |
| **OTLP Exporter** | Composant qui sérialise les spans en protobuf + envoie |
| **Resource** | Set d'attrs identifiant la source (service.name etc.) |
| **Semantic Conventions** | Nomenclature OTel-spec pour attribute keys |
| **ParentBased Sampler** | Respecte la decision sampling du parent ; sample-rate sur les roots |
| **TraceIdRatioBased** | Sample N% basé sur le trace_id (deterministic) |
| **Force-on-error** | Override : sample 100 % des spans status=ERROR |
| **W3C traceparent** | Standard HTTP header pour propager le trace context |
| **Tempo** | Backend de storage pour traces (Grafana stack) |
| **Tail Sampling** | Décider après que le span finit (collector concern) |

---

## 21. Plan de sprint D13-bis (4 semaines + 1 audit)

### Semaine 1 — Foundation

- [ ] `crates/efe-telemetry/Cargo.toml` : add 5 OTel deps
- [ ] `otel/resource.rs` : `build_resource(cfg)` + tests
- [ ] `otel/sampler.rs` : `ErrorBoostingSampler`
- [ ] `otel/attribute_filter.rs` : forbidden-key drop
- [ ] Unit tests : 10 listés § 15.1

### Semaine 2 — Pipeline

- [ ] `otel/mod.rs::init` : real implementation (replaces no-op)
- [ ] `otel/measuring_exporter.rs` : metric-instrumented wrapper
- [ ] `init.rs` : OpenTelemetryLayer composition
- [ ] `observability_init_d13.rs` : guard with force_flush
- [ ] Integration test § 15.2 with stdout exporter

### Semaine 3 — Helm + telemetry

- [ ] `metrics/d13bis_instruments.rs` : 4 instruments
- [ ] `register_all_d13bis()` re-export
- [ ] Helm sub-chart wiring (otelCollector + values)
- [ ] NetworkPolicy egress rule
- [ ] ConfigMap env vars + `config.rs::validate_otel`
- [ ] Smoke script § 15.4

### Semaine 4 — Self-obs + docs

- [ ] D13-bis Grafana panel additions (4 new charts)
- [ ] PrometheusRule alerts (export error rate, drop rate)
- [ ] Howto Diátaxis : `enable-otel.md`,
      `troubleshoot-otel.md`, `tune-sampling.md`
- [ ] CHANGELOG D13-bis entry
- [ ] Span name catalog explanation doc

### Audit (semaine 5)

- [ ] Re-lecture chirurgicale ligne par ligne
- [ ] Threat model regression tests T-13bis-X
- [ ] Performance bench (delta vs no-OTel)
- [ ] Cardinalité actuelle vs prévue
- [ ] Rapport `docs/audits/d13bis_audit_report.md`

---

## 22. Critères d'acceptation D13-bis

D13-bis est livré quand **tous** les critères ci-dessous sont
satisfaits sur `main` :

- [ ] `cargo test -p efe-telemetry --lib` : ≥ 161 verts
      (151 D16 + 10 D13-bis unit)
- [ ] `cargo test -p efe-telemetry --test d13bis_otel_integration`
      : ≥ 1 vert (e2e via stdout exporter)
- [ ] `pytest scripts/tests/` : ≥ 671 verts (659 D16 + 12 D13-bis pin)
- [ ] Helm chart renders cleanly (`helm template`)
- [ ] Docker compose smoke E2E : send 100 RPCs, observe ~5
      spans côté otel/opentelemetry-collector logs
- [ ] No regression in D13 dashboards
- [ ] No regression in D14 / D14-bis / D15 / D16 metrics
- [ ] Performance bench : `Execute` p99 delta ≤ 5 %
- [ ] CHANGELOG.md entry
- [ ] Audit interne livré : `docs/audits/d13bis_audit_report.md`

---

## 23. Anti-corruption — versioning OTel SDK

OTel Rust SDK est en évolution rapide (0.x). Stratégie de
versioning :

- **Pin minor** : `opentelemetry = "0.27"` (tilde-incompat)
- **Cargo.lock committed** : exact patch in CI/prod
- **Renovate bot** : ouvre PR sur minor bumps + nous décidons
- **Major bump** : audit pass requis, CHANGELOG note + arch doc
  amendment
- **CVE** : P0 patch within 24 h, even on majors

Cf. § 18 R-13bis-1 + R-13bis-5.

---

## 24. Cardinality envelope

| Source | Worst case |
|---|---:|
| D13 instruments (8) | ~24 séries |
| D14 instruments (4) | ~75 séries |
| D14-bis instruments (6) | ~220 séries |
| D15 instruments (7) | ~52 séries |
| D16 instruments (9) | ~3 150 séries |
| **D13-bis instruments (4)** | **~8 séries** |
| Resource attrs (Tempo) | ~5 k unique combos (1k pods × 5 attrs) |

Total Prometheus séries : ~3 530. **Très en-dessous du
budget D13 50 000.**

---

## 25. Trace volume / cost projection

### 25.1 Cluster typique (production tier-2)

```
- 30 réplicas HPA
- 1 000 RPS sustained
- 30 spans / RPC (tokenize + forward + pool + pg + audit + ...)
- 5 % sampling head-based
- ~5 % error rate (force-sampled)
```

Spans/s exportés :
```
1000 RPS × 30 spans × (0.05 + 0.05 × (1 - 0.05) error)
≈ 1000 × 30 × 0.10 = 3 000 spans/s aggregate cluster
```

### 25.2 Bandwidth

Average span size (compressed protobuf with compression) ≈
0.5 KiB. So 3 000 × 0.5 = 1.5 MiB/s = **5.4 GiB/h cluster
bandwidth** to the collector.

### 25.3 Storage (Tempo retention 7 days)

```
1.5 MiB/s × 86 400 s × 7 days = 907 GiB raw
```

Avec compression Tempo (~3-5x), **~250 GiB cold storage**.
Acceptable pour un cluster prod tier-2.

### 25.4 Sampling tradeoffs

Si le sampling à 5 % est trop bas pour debug, l'opérateur peut
bumper temporairement à 25 % via env var rolling restart.
Cost grimpe ×5 mais visibility gagne. Documenté dans le howto.

---

## 26. Non-objectifs explicites

- **OTLP metrics export** : non, D13-ter (Prometheus pull
  reste l'autorité).
- **OTLP logs export** : non, D13-ter (Loki via stdout reste
  l'autorité).
- **mTLS to collector** : non, D13-ter (rely on K8s
  NetworkPolicy in-cluster).
- **Tail-sampling at SDK** : non, c'est le rôle du collector
  processor.
- **Vendor-specific exporters** : non, OTLP only — switching
  backends est un changement de Helm chart, pas du SDK.
- **Auto-instrumentation des libs** (sqlx, tonic, reqwest) :
  non, D13-bis ne touche pas les libs — `tracing-opentelemetry`
  bridge les `tracing::span!` existants, point.
- **Trace ID dans audit events SQL** : non, D13-ter.
- **Distributed sampling decisions** (e.g. dynamic
  per-tenant sampling) : non, D14+.
- **W3C tracecontext** propagation côté gRPC server (extract
  the inbound traceparent) : oui-mais-via tonic-tracing
  middleware existant, pas D13-bis specific.

---

**Document maître D13-bis** ─ ~1700 lignes, ~13 200 mots. À
relire une fois par sprint owner + une fois par security
reviewer avant gel-de-spec et passage à l'implémentation.

— *Fin du document D13-bis architecture.*
