# Sprint D13 — Architecture : Observability dashboards (Grafana, Prometheus, Loki, Tempo)

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : suit D12 (catalog admin + versioning) et clôt
> la trilogie multi-tenant `D11 (isolation) → D12 (governance) →
> D13 (operability)`. D13 est le sprint qui transforme EFE en
> **service exploitable par une équipe SRE de partenaire** : le
> Helm chart livre des dashboards Grafana copy-paste, des alertes
> Prometheus runbook-linkées, un schéma de logs structurés stable
> et une couche traces OpenTelemetry exemplar-friendly. Sans D13,
> un partenaire en Tier 2 voit du trafic mais ne peut pas
> répondre à un incident.
>
> **Pré-requis** : avoir lu
> [`d11_per_tenant_isolation_architecture.md`](d11_per_tenant_isolation_architecture.md)
> (isolation, ServiceMonitor relabel `tenant`, PrometheusRule
> Tier 2), l'audit
> [`d11_isolation_report.md`](../audits/d11_isolation_report.md),
> [`d12_catalog_admin_versioning_architecture.md`](d12_catalog_admin_versioning_architecture.md)
> (CatalogService, audit log emission via
> `tracing::info!(target = "audit")`), et le
> [`d12_catalog_audit_report.md`](../audits/d12_catalog_audit_report.md)
> (11 findings corrigés post-Phase 2, 216/216 Python + 185/185
> Rust verts).
>
> **Hors-périmètre** : D13 NE touche PAS au plan de données
> classify (Mode 1 chain, Mode 2 future). D13 NE livre PAS de
> backend persistant pour les catalogues (déféré à D14). D13 NE
> remplace PAS Grafana/Prometheus existant — il y branche des
> ConfigMaps et des règles supplémentaires.

---

## Table des matières

1. [Contexte & mission D13](#1-contexte--mission-d13)
2. [État post-D12 — observabilité existante vs gaps](#2-état-post-d12--observabilité-existante-vs-gaps)
3. [Personas opérateurs & inventaire des dashboards](#3-personas-opérateurs--inventaire-des-dashboards)
4. [Stack observabilité — Prometheus, Grafana, Loki, Tempo, Alertmanager](#4-stack-observabilité)
5. [Catalogue de métriques — existantes + 8 nouvelles métriques D13](#5-catalogue-de-métriques)
6. [Schéma des logs structurés (5 streams)](#6-schéma-des-logs-structurés)
7. [Traces OpenTelemetry + exemplars](#7-traces-opentelemetry--exemplars)
8. [Gestion de la cardinalité (budget 1 000 tenants)](#8-gestion-de-la-cardinalité)
9. [Spécifications des 6 dashboards persona-driven](#9-spécifications-des-6-dashboards-persona-driven)
10. [Alertes + runbooks linking](#10-alertes--runbooks-linking)
11. [Distribution Helm — sidecar + ConfigMaps + values](#11-distribution-helm)
12. [Plan de données multi-tenant (qui voit quoi)](#12-plan-de-données-multi-tenant)
13. [Cycle de vie des données — retention + downsampling + export](#13-cycle-de-vie-des-données)
14. [Modèle de sécurité Grafana — RBAC, scoped tokens, partner orgs](#14-modèle-de-sécurité-grafana)
15. [Threat model — 10 vecteurs observabilité-spécifiques](#15-threat-model)
16. [12 décisions de conception clés](#16-12-décisions-de-conception-clés)
17. [10 risques identifiés et mitigations](#17-10-risques-identifiés-et-mitigations)
18. [Plan jour-par-jour (6 j wallclock)](#18-plan-jour-par-jour-6-j-wallclock)
19. [SLOs ajoutés post-D13](#19-slos-ajoutés-post-d13)
20. [Extension `RESERVED_PREFIXES` et tests anti-régression](#20-extension-reserved_prefixes)
21. [Intégration CI — workflow `bench-observability.yml`](#21-intégration-ci)
22. [Critères d'acceptation (DoD)](#22-critères-dacceptation)
23. [Out of scope & roadmap follow-ups (D14+)](#23-out-of-scope)
24. [Annexes — PromQL, LogQL, runbook templates](#24-annexes)

---

## 1. Contexte & mission D13

### 1.1 État post-D12

Au sortir de D12, EFE a la maturité fonctionnelle suivante :

* **Multi-tenant complet** (D11) : auth JWT scoped par
  `tenant_id`, ResourceQuota + LimitRange + NetworkPolicy par
  namespace, ServiceMonitor avec relabel `tenant` automatique,
  4 alertes PrometheusRule Tier 2, cardinality bomb defense.
* **Governance des catalogues** (D12) : `CatalogService` (6
  RPCs), versioning monotone par `(tenant, name)`, 5 evolve
  ops (APPEND_CLASS / REMOVE_CLASS / UPDATE_THRESHOLDS /
  REPLACE_CLASS_ANCHORS / RECOMPILE), audit trail
  `created_at + created_by + parent_key + content_sha256`,
  scaffolding Rust prêt (handler gRPC déféré à D12-bis).
* **70 SLOs** dans le registre canonique : 26 fondations
  (D1-D9), 5 DX (D10), 7 multi-tenant (D11), 7 catalog (D12),
  reste répartis sur les sprints intermédiaires.
* **216 tests Python anti-régression + 185 tests Rust** post-
  audit. Aucune régression cross-sprint.
* **Observabilité côté production** : limitée à des métriques
  Prometheus exposées par `efe-server`, des PrometheusRules
  basiques M9-era, et une page how-to `observability.md` qui
  liste les métriques mais ne fournit aucun dashboard.

### 1.2 Le problème opérationnel

Un SRE qui hérite d'un déploiement EFE en Tier 2 doit
répondre à 5 questions, sur le terrain, sous pression :

1. **« Le service est-il en bonne santé en ce moment ? »** —
   liveness, readiness, error rate, p99 latency, saturation
   CPU/mem, throughput, par-tenant.
2. **« Quel tenant pose problème ? »** — quel namespace consomme
   excessivement, quel tenant a un taux d'erreur anormal, quels
   sont les top-N tenants sur les 24 dernières heures.
3. **« Quelle version de catalogue chaque tenant utilise ? »** —
   distribution des versions actives, tenants restés sur des
   versions vétustes, freshness moyenne.
4. **« Combien chaque tenant nous coûte ? »** — minutes de CPU,
   GB mémoire, RPM, anchors compilés, agrégés sur la fenêtre
   de facturation.
5. **« Quelqu'un attaque-t-il le système ? »** — taux d'auth
   failures, anomalies de pattern (spike de `evolution_op =
   recompile`, suppressions massives), origines géographiques
   inhabituelles.

Sans D13, **aucune de ces 5 questions n'a de réponse en
moins de 30 secondes**. Le SRE doit écrire ses propres requêtes
PromQL, lire la code source pour découvrir les métriques, et
n'a aucun visuel pour pivoter rapidement.

D13 transforme cette friction en **6 dashboards copy-paste**,
**10 alertes runbook-linked**, et une couche **traces +
exemplars** qui permet de pivoter d'une requête lente vers le
span Rust qui l'a produite.

### 1.3 Mission D13 (énoncé chirurgical)

> 1. **Livrer 6 dashboards Grafana JSON** au schéma v9.5+,
>    persona-driven, dans `deploy/observability/grafana/dashboards/d13/`.
>    Découverte automatique via le sidecar Grafana (label
>    `grafana_dashboard=1` sur les ConfigMaps Helm-rendered).
>
> 2. **Étendre le schéma de métriques** avec 8 nouvelles
>    métriques `efe_*` (5 histogrammes pour les 5 RPCs catalog,
>    1 gauge cardinality budget, 1 counter audit events, 1
>    counter alert fires) ; 7 recording rules pour les agrégats
>    `efe:*` (top-N par tenant, taux d'erreur par RPC, etc.).
>
> 3. **Schématiser 5 streams de logs structurés** (request, audit,
>    catalog evolution, classification, security) avec contrat
>    JSON strict, ingérables par Loki / promtail / Cloudwatch,
>    et tester la stabilité du contrat via un schéma JSON-Schema
>    versionné.
>
> 4. **Activer OpenTelemetry par défaut côté serveur**,
>    instrumenter les 6 RPCs catalog avec spans nommés
>    `efe.catalog.<rpc>`, propager les exemplars dans les
>    histogrammes Prometheus, et ajouter un datasource Tempo
>    optionnel via Helm.
>
> 5. **Linker chaque alerte à un runbook** versionné dans
>    `docs/runbooks/d13_*.md`. 10 runbooks à livrer (matching
>    les 10 alertes). CI test : aucune alerte sans `runbook_url`
>    annotation.
>
> 6. **Exposer 7 nouvelles SLOs `D13-OBS-*`** dans
>    `scripts/parse_criterion_report.py::SLO_DEFINITIONS`,
>    avec `RESERVED_PREFIX` `obs::dash::` étendu (l'existant
>    `obs::` reste, on ajoute `obs::dash::`).
>
> 7. **Documenter** : 4 pages Diataxis
>    (`docs/howto/observability-setup.md`,
>    `docs/howto/cardinality-management.md`,
>    `docs/reference/dashboard-catalog.md`,
>    `docs/explanation/observability-architecture.md`) +
>    10 runbooks.
>
> 8. **Multi-tenant safe** : tous les dashboards qui surfacent
>    de la donnée tenant sont par défaut **scoped** au tenant
>    de la session Grafana courante, jamais cross-tenant —
>    l'opérateur SaaS-de-EFE peut désactiver le filtre via une
>    variable Grafana, mais c'est explicit.
>
> 9. **Aucune régression** sur les 70 SLOs antérieurs, les 216
>    tests Python, les 185 tests Rust.

### 1.4 Définition du non-périmètre

D13 NE livre PAS :

* Un agent custom (Datadog, Newrelic, Splunk) — on reste sur
  l'écosystème open-source Prometheus/Grafana.
* Un APM intégré (deferred to D15+).
* Une app mobile pour incidents — Grafana mobile suffit.
* Un système de billing automatique — D13 surface le cost,
  l'export effectif est déjà couvert par
  `scripts/billing/aggregate_usage.py` (D11).
* La persistence Prometheus/Loki — c'est l'opérateur du
  cluster qui décide entre Thanos / Cortex / Mimir / VM.

---

## 2. État post-D12 — observabilité existante vs gaps

### 2.1 Ce qui existe déjà (à PRÉSERVER)

| Composant | Source | État | D13 fait quoi avec |
|-----------|--------|------|---------------------|
| Métriques `efe_grpc_request_duration_seconds` (histogram) | M9-era `crates/efe-telemetry/` | Stable, scrapable | Réutilise + ajoute des recording rules `efe:rpc_p99:5m` |
| Métriques `efe_classify_*` (Mode 1) | M9 | Stable | Réutilise dans le dashboard Operator-Overview |
| Métriques `efe_compile_anchors_seconds` | D8 | Stable | Réutilise dans Capacity-Planning |
| ServiceMonitor avec relabel `tenant` | D11 | Stable, opt-in via Tier 2 | Réutilise pour PromQL `by (tenant)` |
| 4 PrometheusRule Tier 2 (D11-MT-*) | D11 | Stable, gated | Étend avec 6 nouvelles alertes D13-OBS-* |
| `scripts/billing/aggregate_usage.py` | D11 | Stable | Source du dashboard Cost-Attribution |
| `tracing::info!(target = "audit")` events D12 | D12 | Stable, stdout | Nouvelle ingestion Loki via promtail |
| Documentation `docs/howto/observability.md` | M10 | Surface basique | Étend (les 4 nouvelles pages) ; reste indépendante |
| Grafana dashboard JSON `deploy/observability/grafana/dashboards/efe-overview.json` | D11 | 1 dashboard | Étend la collection avec 6 nouveaux ; ne casse pas l'existant |

### 2.2 Les gaps que D13 doit fermer

| Gap | Impact opérationnel | Solution D13 |
|-----|---------------------|--------------|
| Pas de dashboard SRE prêt-à-l'emploi | Le SRE écrit ses propres requêtes pendant un incident, MTTR explose | 6 dashboards JSON livrés via Helm |
| Pas de runbook linked aux alertes | Une alerte qui fire ne dit pas quoi faire | 10 runbooks `docs/runbooks/d13_*.md` + `runbook_url` annotation sur chaque alerte |
| Pas de traces actives par défaut | Impossible de trace un appel lent jusqu'au span Rust | OpenTelemetry on-by-default côté serveur, exemplars dans les histogrammes |
| Schéma de logs ad-hoc | Impossible de query `error_code = "..."` de manière stable | 5 streams JSON-Schema versionnés (`schema_version: 1.0`) |
| Cardinalité ingérable | Spike de tenants → cardinality bomb → Prom OOM | Recording rules `topk(20, ...)` + label `_other_` aggregation pour la longue traîne |
| Pas de persona-driven views | Un finance qui lit un dashboard SRE n'y comprend rien | 6 dashboards : Operator, Tenant Drill-Down, Catalog Versions, Cost, Audit, Capacity |
| Catalogue D12 invisible côté ops | Les SRE ne voient pas qui a déployé quel `register_catalog` | Audit Stream dashboard alimenté par les `tracing::info!(target = "audit")` events |
| Multi-tenant Grafana RBAC inexistant | Un partenaire pourrait voir les métriques d'un autre | Section 14 : Grafana orgs scoped + datasource teams + folder perms |
| Pas de SLO compliance heatmap | Compliance state global non-glanceable | Top de chaque dashboard : panel SLO heatmap (D6/D7/D8/D9/D11/D12/D13 families) |
| Pas de pipeline de tests dashboards | Un dashboard cassé en silence ne fail aucun build | `bench-observability.yml` workflow : `promtool check`, JSON-Schema validate, runbook link checker |

### 2.3 Boundary : ce que D13 NE FAIT PAS

* **Pas de remplacement Prometheus / Grafana** : on consomme,
  on n'opère pas. L'opérateur du cluster choisit son backend
  (vanilla Prom, Mimir, Thanos, Cortex, victoria-metrics, …).
* **Pas de log shipping turn-key** : on définit le schéma JSON,
  mais le ramassage (promtail → Loki, fluentd → CloudWatch, …)
  est dans les mains de l'opérateur. On fournit un example
  promtail config pour démarrer.
* **Pas de dashboard custom par tenant** : les 6 dashboards
  sont **génériques** ; chaque tenant voit SES données via les
  variables Grafana et les RBAC de son orga. La customisation
  (logos, branding) est out-of-scope.
* **Pas de pricing visible** : on surface les minutes CPU /
  GB mem / requests, l'opérateur SaaS-de-EFE applique son
  propre pricing. Pas de hardcoded $/unit.

---

## 3. Personas opérateurs & inventaire des dashboards

### 3.1 6 personas

| ID | Persona | Job-to-be-Done | Fréquence | Dashboards consommés |
|----|---------|----------------|-----------|----------------------|
| P1 | **SRE on-call** | Détecter et répondre à un incident en < 5 min | 24×7 (rotation) | 1, 6 |
| P2 | **Capacity SRE** | Anticiper la croissance, dimensionner | Hebdomadaire | 6 |
| P3 | **Support engineer (T1/T2)** | Investiguer un cas client (« mon classify est lent ») | Quotidien | 2 |
| P4 | **Catalog curator** (engineering / data scientist) | Suivre la qualité et la fraîcheur des catalogues | Hebdomadaire | 3 |
| P5 | **Finance / FinOps** | Réconcilier la facturation, optimiser le coût | Mensuel | 4 |
| P6 | **Security analyst** | Détecter des anomalies (auth failures, pattern atypique) | Continue + sur incident | 5 |

### 3.2 6 dashboards (inventaire)

| # | Nom (file slug) | Persona principal | Lock-step avec |
|---|-----------------|-------------------|----------------|
| 1 | `01_operator_overview` | P1 SRE on-call | M9 metrics + D11 alertes |
| 2 | `02_tenant_drill_down` | P3 Support T1/T2 | D11 ServiceMonitor |
| 3 | `03_catalog_version_distribution` | P4 Catalog curator | D12 CatalogService |
| 4 | `04_cost_attribution` | P5 Finance | D11 `aggregate_usage.py` |
| 5 | `05_audit_stream` | P6 Security | D12 audit log + D11 audit trail |
| 6 | `06_capacity_planning` | P2 Capacity SRE | D9 load tests + recording rules |

Chaque dashboard est détaillé en §9 avec : panels, queries
PromQL, variables, drill-down links, runbook references.

### 3.3 Hiérarchie de drilldown

```
                ┌──────────────────────────┐
                │  01 Operator Overview    │  ← entry point on-call
                │  (alerts, error rate,    │
                │   p99, throughput)       │
                └──────────┬───────────────┘
                           │ click on alert / panel
            ┌──────────────┼──────────────┐
            │              │              │
            ▼              ▼              ▼
    ┌───────────┐  ┌──────────────┐  ┌──────────────┐
    │ 02 Tenant │  │ 06 Capacity  │  │ 05 Audit     │
    │ Drill-Down│  │ Planning     │  │ Stream       │
    └─────┬─────┘  └──────────────┘  └──────────────┘
          │ click on tenant cell
          ▼
    ┌──────────────┐
    │ 03 Catalog   │  ← per-tenant version chain
    │ Version Dist │
    └──────────────┘
```

Le dashboard 04 (Cost) est volontairement isolé — il est consulté
indépendamment, généralement en mode batch mensuel.

---

## 4. Stack observabilité

### 4.1 Vue d'ensemble

```
                          ┌───────────────────────┐
                          │   efe-server (Rust)   │
                          │                       │
                          │  ▾ /metrics            │
                          │  ▾ stdout (logs JSON)  │
                          │  ▾ OTLP (traces)       │
                          └─────────┬─────────────┘
                                    │
        ┌───────────────────────────┼─────────────────────────────┐
        │                           │                             │
        ▼                           ▼                             ▼
┌───────────────┐           ┌───────────────┐           ┌────────────────┐
│  Prometheus   │           │  promtail /   │           │  OTel Collector│
│  (scrape)     │           │  fluentd      │           │                │
└──────┬────────┘           └──────┬────────┘           └────────┬───────┘
       │                           │                             │
       ▼                           ▼                             ▼
┌───────────────┐           ┌───────────────┐           ┌────────────────┐
│ Recording     │           │  Loki         │           │  Tempo / Jaeger │
│ rules (D13)   │           │  (D13 schema) │           │  (D13 spans)    │
└──────┬────────┘           └──────┬────────┘           └────────┬───────┘
       │                           │                             │
       └───────────────┬───────────┴─────────────┬───────────────┘
                       │                         │
                       ▼                         ▼
                ┌──────────────────────────────────┐
                │         Grafana                  │
                │  (6 D13 dashboards + alertes)    │
                │                                  │
                │  → Prometheus datasource         │
                │  → Loki datasource               │
                │  → Tempo datasource              │
                │  → Alertmanager via Prom         │
                └──────────────────────────────────┘
```

### 4.2 Composants — versions cibles

| Composant | Version cible | Lock-step | Notes |
|-----------|---------------|-----------|-------|
| Prometheus | 2.49+ | exemplar storage on | `--enable-feature=exemplar-storage` |
| Grafana | 10.2+ | datasource v2, auto-detect dashboards | Auto-discovery via sidecar |
| Loki | 2.9+ | LogQL v2, structured metadata | promtail 2.9+ pour le ramassage |
| Tempo | 2.3+ | exemplar correlation | OTLP/gRPC ingester |
| OpenTelemetry SDK Rust | 0.21+ | aligned avec tonic 0.10 | direct dep ajoutée à `efe-server/Cargo.toml` |
| OpenTelemetry SDK Python | 1.21+ | optional (extras_require) | Aligné sur D10 hybrid_decoder |
| Alertmanager | 0.26+ | route by `severity`+`tenant` label | Réutilise existing |

### 4.3 Why pas Datadog / Newrelic / Splunk ?

* **Vendor lock-in** : empêche les partenaires d'exporter leur
  propre data. EFE est un build-block, pas un produit
  managé — l'opérateur SaaS-de-EFE choisit son backend.
* **Coût** : Datadog scaling cardinality est cher au-delà de
  500 tenants ; à 1000+ tenants, Mimir/Thanos sur S3 est
  10× moins cher.
* **PromQL / LogQL standard** : transférable, scriptable, dans
  le DNA SRE moderne.

L'opérateur est libre d'**ajouter** un agent Datadog en parallèle
s'il préfère ; les métriques EFE sont scrapables en standard
Prometheus / OpenMetrics.

### 4.4 Pourquoi Loki et pas Elasticsearch ?

* **Structured-logging-first** : les events JSON D12 audit log
  sont nativement compatibles avec LogQL JSON parsing.
* **Cost** : Loki indexe seulement les labels (low-cardinality),
  contenu compressé sur object storage. ES indexe tout —
  facture trois fois plus cher pour le même usage.
* **Same Grafana datasource** : un dashboard peut mixer Prom +
  Loki dans une même row, switch sans changer de tab.

### 4.5 Pourquoi Tempo et pas Jaeger ?

* **Exemplar-friendly** : un panel Grafana peut sauter d'une
  bucket histogramme à un trace via le `traceID` exemplar —
  Tempo expose ça nativement.
* **Cheap storage** : Tempo n'indexe que `traceID` ; les spans
  vivent sur S3. À 100 traces/sec, c'est l'option la moins
  chère et la plus simple à opérer.
* **Jaeger reste compatible** — via l'export OTLP, l'opérateur
  peut router vers Jaeger s'il préfère. C'est un swap de
  datasource Grafana, rien dans le code EFE.

---

## 5. Catalogue de métriques

### 5.1 Métriques existantes (M9-D12) — recensement

D'abord, un inventaire complet de ce que `efe-server` expose
déjà sur `/metrics` (à jour post-D12) :

| Métrique | Type | Labels existants | Sprint d'origine |
|----------|------|------------------|------------------|
| `efe_grpc_requests_total` | counter | `service`, `method`, `code`, `tenant` | M9 (D11 ajoute `tenant`) |
| `efe_grpc_request_duration_seconds` | histogram | `service`, `method`, `code`, `tenant` | M9 |
| `efe_classify_total` | counter | `tenant`, `catalog`, `result` | M9 |
| `efe_classify_confidence` | histogram | `tenant`, `catalog`, `class` | M9 |
| `efe_compile_anchors_seconds` | histogram | `tenant`, `catalog`, `n_anchors_bucket` | D8 |
| `efe_swarm_round_count` | histogram | `tenant`, `task_size` | D5 |
| `efe_ratelimit_drops_total` | counter | `tenant`, `bucket` | D11 |
| `efe_idempotency_hits_total` | counter | `tenant` | D11 |
| `efe_authz_failures_total` | counter | `tenant`, `reason` | D11 |
| `efe_active_tenants` | gauge | (no labels) | D11 cardinality probe |

Toutes ces métriques sont **stables, contractuelles** post-
D11. D13 NE les renomme PAS.

### 5.2 Métriques NOUVELLES en D13

D13 ajoute **8 nouvelles métriques** pour combler les gaps
observabilité :

| Métrique D13 | Type | Labels | Justification | Cardinalité estimée |
|--------------|------|--------|---------------|---------------------|
| `efe_catalog_register_duration_seconds` | histogram | `tenant`, `result` | SLO D12-CT-1/-2 ; observable register p99 | ≤ 2 × `n_tenants` |
| `efe_catalog_evolve_duration_seconds` | histogram | `tenant`, `op`, `result` | SLO D12-CT-3 ; per-op latency | 5 × 2 × `n_tenants` |
| `efe_catalog_get_duration_seconds` | histogram | `tenant`, `include_compiled`, `result` | SLO D12-CT-4 | 2 × 2 × `n_tenants` |
| `efe_catalog_list_duration_seconds` | histogram | `tenant`, `result` | SLO D12-CT-5 | 2 × `n_tenants` |
| `efe_catalog_versions_per_name` | gauge | `tenant`, `name` | Visualiser l'épaisseur des chaînes ; alerte cardinalité | bornée par quotas D12 |
| `efe_audit_events_total` | counter | `tenant`, `action`, `op_tag` | Source du dashboard 05 Audit | `n_tenants` × 3 actions × 7 op_tags |
| `efe_alert_fires_total` | counter | `alertname`, `severity`, `tenant` | Self-monitoring des alertes | bornée par `n_alerts` × 3 sev |
| `efe_dashboard_query_duration_seconds` | histogram | `dashboard`, `panel`, `outcome` | SLO D13-OBS-1 dashboard load time | 6 × ~80 panels |

### 5.3 Recording rules — agrégats `efe:*`

7 recording rules ajoutées pour réduire la cardinalité sur les
queries fréquentes des dashboards :

```yaml
# deploy/observability/prometheus/rules.d13/recording.yaml
groups:
  - name: efe-d13-recording
    interval: 30s
    rules:
      # 1. Per-tenant p99 latency, all RPCs combined
      - record: efe:rpc_p99_latency_seconds:5m
        expr: |
          histogram_quantile(0.99,
            sum(rate(efe_grpc_request_duration_seconds_bucket[5m]))
              by (le, tenant)
          )

      # 2. Per-tenant error rate
      - record: efe:rpc_error_rate:5m
        expr: |
          sum(rate(efe_grpc_requests_total{code!="OK"}[5m])) by (tenant)
          / sum(rate(efe_grpc_requests_total[5m])) by (tenant)

      # 3. Top-20 tenants by request rate (long-tail aggregation)
      - record: efe:top20_tenants_by_rps:5m
        expr: |
          topk(20, sum(rate(efe_grpc_requests_total[5m])) by (tenant))

      # 4. Catalog evolution rate per op
      - record: efe:catalog_evolve_rate:5m
        expr: |
          sum(rate(efe_audit_events_total{action="evolve"}[5m]))
            by (op_tag, tenant)

      # 5. Active versions per (tenant, name)
      - record: efe:catalog_chain_depth:1m
        expr: |
          max(efe_catalog_versions_per_name) by (tenant, name)

      # 6. Per-tenant compute minutes (for cost dashboard)
      - record: efe:cpu_minutes_per_tenant:1h
        expr: |
          sum(rate(container_cpu_usage_seconds_total{
                namespace=~"tenant-.+"}[1h])) by (namespace) * 60

      # 7. Per-tenant RAM GB-hours
      - record: efe:ram_gb_hours_per_tenant:1h
        expr: |
          avg_over_time(
            sum(container_memory_working_set_bytes{
                  namespace=~"tenant-.+"}[1h]) by (namespace) [1h:5m]
          ) / 1e9
```

### 5.4 Cardinalité totale attendue

Estimation pour 100 tenants actifs (cible D11) :

| Famille | Estimation |
|---------|------------|
| `efe_grpc_*` (existing) | ~ 12 × 100 × 4 codes = 4 800 séries |
| `efe_classify_*` | ~ 5 × 100 × 10 catalogs = 5 000 séries |
| `efe_catalog_*` (D13 new) | ~ 5 RPCs × 2 result × 100 tenants = 1 000 |
| `efe_audit_events_total` | ~ 100 × 3 × 7 = 2 100 |
| `efe_alert_fires_total` | ~ 16 × 3 × 100 = 4 800 |
| Recording rules `efe:*` | ~ 200 |
| **Total estimé** | **~ 18 000 séries actives** |

Bien en-dessous du budget Prometheus pratique (~ 1M séries
sur un Prom node 16 GB / 4 cores). Le head-room laisse
absorber × 50 si la cardinalité explose temporairement.

---

## 6. Schéma des logs structurés

### 6.1 5 streams + JSON-Schema versionné

D13 normalise 5 streams de logs JSON. Chaque event écrit sur
stdout est valide vis-à-vis d'un JSON-Schema versionné dans
`deploy/observability/log-schemas/`.

| Stream | Source | Cadence | Use case |
|--------|--------|---------|----------|
| `request` | `efe-server::interceptors::request_log` | 1/req | Distributed trace correlation, error analysis |
| `audit` | `crates/efe-server/src/catalog/audit.rs` (D12) + cones (M9) | 1/mutation | Security analysis, compliance evidence |
| `classify` | `efe-server::services::swarm` | 1/classify | Per-classification telemetry, drift detection |
| `evolution` | `crates/efe-server/src/catalog/memory.rs` (D12) | 1/evolve | Change-history forensics |
| `security` | `efe-server::auth::*` | 1/auth-check | JWT failures, scope violations, source IP |

### 6.2 Champs communs (tous les streams)

```json
{
  "schema_version": "1.0",
  "stream": "request|audit|classify|evolution|security",
  "timestamp": "2026-04-27T14:30:00.123Z",
  "level": "INFO|WARN|ERROR",
  "request_id": "uuid-v4",
  "trace_id": "32-hex (otel)",
  "span_id": "16-hex (otel)",
  "tenant_id": "tenant-acme",
  "principal": "alice@acme.com",
  "service": "efe-server",
  "service_version": "0.1.13"
}
```

Les champs communs sont **obligatoires** sur tous les events,
sauf `principal` (absent sur les events anonymes pre-auth).

### 6.3 Schéma `request`

```json
{
  ...common,
  "stream": "request",
  "rpc": "Classify|Register|Evolve|Get|...",
  "duration_ms": 42,
  "status_code": "OK|UNAVAILABLE|...",
  "client_ip": "1.2.3.4",
  "user_agent": "efe-tool/0.1.13 python/3.11",
  "request_size_bytes": 1024,
  "response_size_bytes": 2048,
  "error_message": null
}
```

### 6.4 Schéma `audit` (mirror de D12 § 13)

```json
{
  ...common,
  "stream": "audit",
  "action": "register|evolve|delete",
  "resource_type": "catalog|cone",
  "resource_key": "tenant/name@vN",
  "parent_key": "tenant/name@v(N-1) or null",
  "op_tag": "register|append_class|remove_class|...|delete",
  "content_sha256": "hex-32",
  "description": "free text",
  "labels": {"env": "prod"}
}
```

### 6.5 Schéma `classify`

```json
{
  ...common,
  "stream": "classify",
  "catalog_key": "tenant/support@v3",
  "predicted_class": "billing",
  "confidence": 0.87,
  "tier": "high|medium|low|reject",
  "n_classes": 5,
  "n_swarm_rounds": 8,
  "swarm_id": "uuid-v4",
  "embedder_fingerprint": "st:mini:384"
}
```

### 6.6 Schéma `evolution`

```json
{
  ...common,
  "stream": "evolution",
  "from_key": "tenant/name@v2",
  "to_key": "tenant/name@v3",
  "op_tag": "append_class|...|recompile",
  "diff_summary": {"classes_added": ["churn"], "classes_removed": []},
  "validate_duration_ms": 12,
  "compile_duration_ms": 480
}
```

### 6.7 Schéma `security`

```json
{
  ...common,
  "stream": "security",
  "event": "jwt_invalid|scope_missing|tenant_mismatch|rate_limited",
  "reason": "expired|signature|scope:admin:catalog:write missing",
  "claimed_tenant": "tenant-acme",
  "actual_tenant": "tenant-globex",
  "scope_required": "admin:catalog:write",
  "scope_present": ["admin:catalog:read"]
}
```

### 6.8 Stabilité du schéma

* `schema_version` MAJOR change = surface-breaking (très rare).
* Ajout d'un champ optionnel = MINOR bump (e.g. `1.0` → `1.1`).
* Retrait d'un champ = MAJOR bump (e.g. `1.x` → `2.0`).
* Pinné par `scripts/tests/test_d13_log_schemas.py` qui valide
  10 events synthétiques contre les 5 schémas.

### 6.9 Pipeline de ramassage (exemple promtail)

```yaml
# deploy/observability/loki/promtail-config.example.yaml
scrape_configs:
  - job_name: efe-server
    kubernetes_sd_configs:
      - role: pod
        selectors:
          - role: pod
            label: "app.kubernetes.io/name=efe-server"
    relabel_configs:
      - source_labels: [__meta_kubernetes_pod_label_tenant_efe_ai_owner]
        target_label: tenant
      - source_labels: [__meta_kubernetes_namespace]
        target_label: namespace
    pipeline_stages:
      - json:
          expressions:
            stream: stream
            level: level
            request_id: request_id
            tenant_id: tenant_id
      - labels:
          stream:
          level:
          tenant_id:
      - timestamp:
          source: timestamp
          format: RFC3339Nano
```

Avec ce promtail config, Loki indexe `{stream, level, tenant_id}`
comme labels, le reste reste dans le contenu compressé. LogQL
queries deviennent :

```logql
# Toutes les erreurs d'auth pour acme dans la dernière heure
{tenant_id="tenant-acme", stream="security", level="ERROR"}

# Top 10 audit events par op_tag
sum by (op_tag) (rate({stream="audit"}[1h] | json))
```

---

## 7. Traces OpenTelemetry + exemplars

### 7.1 Pourquoi les traces

Sans traces, un classify lent est une boîte noire :

* « Le client SDK voit 800 ms ; pourquoi ? »
* « Quelle phase a coûté ? Embedding ? Cone match ? Fusion ? »
* « Y a-t-il un retry interne ? »

Avec OpenTelemetry + Tempo + Grafana exemplars, l'opérateur :

1. Ouvre dashboard 01 Operator Overview, voit le p99 spike.
2. Clique sur la bucket histogramme du panel latency.
3. Grafana affiche 3 exemplars (3 traceIDs réels du bucket).
4. Click sur un trace_id → Tempo waterfall view → identifie
   que `embedder.encode` prend 600 ms sur ce span.
5. Pivot vers les logs du même `request_id` pour confirmer.
6. **MTTR = 90 secondes** au lieu de 30 minutes.

### 7.2 Spans nommés (post-D12-bis)

| Span | Source | Attributs |
|------|--------|-----------|
| `efe.swarm.classify` | services/swarm.rs | `tenant`, `catalog_key`, `n_classes` |
| `efe.embedder.encode` | embedder/* | `model`, `n_tokens`, `dim` |
| `efe.cone.fit` | cone/* | `n_anchors`, `dim` |
| `efe.cone.classify` | cone/* | `class_name`, `distance` |
| `efe.fusion.round` | fusion/* | `round_idx`, `temperature` |
| `efe.catalog.register` | catalog/handler.rs (D12-bis) | `tenant`, `name`, `n_classes` |
| `efe.catalog.evolve` | catalog/handler.rs (D12-bis) | `from_key`, `op` |
| `efe.catalog.get` | catalog/handler.rs (D12-bis) | `key`, `include_compiled` |
| `efe.catalog.list` | catalog/handler.rs (D12-bis) | `tenant`, `page_size` |
| `efe.catalog.delete` | catalog/handler.rs (D12-bis) | `key`, `force` |
| `efe.auth.verify` | auth/jwt.rs | `subject`, `tenant`, `scopes` |
| `efe.ratelimit.acquire` | ratelimit/*.rs | `tenant`, `bucket`, `outcome` |

### 7.3 Sampling stratégie

* **Head-based, parent-based** par défaut, ratio 5 % (1 sur 20).
* **Always-sample** pour les codes `≥ 500` (errors).
* **Always-sample** pour les requests > 1 s (slow path).
* Configuration via `EFE_OTEL_SAMPLE_RATE` env var et
  `EFE_OTEL_FORCE_SAMPLE_ON_ERROR=true`.

### 7.4 Exemplars dans les histogrammes

Prometheus 2.49 supporte `--enable-feature=exemplar-storage`.
Chaque bucket d'un histogramme peut être annoté avec un
`traceID` exemplar.

`crates/efe-telemetry/src/histogram.rs` est étendu avec une
méthode `observe_with_exemplar(value, span_context)` qui :

1. Observe la valeur dans le bucket usuel.
2. Si Prom client est en mode exemplar et le sample est
   trace-sampled, attache le `traceID` au bucket.

### 7.5 Helm — opt-in datasource Tempo

```yaml
# deploy/helm/efe/values.yaml — ajout D13
observability:
  traces:
    enabled: false               # opt-in
    otelEndpoint: ""              # OTLP/gRPC endpoint
    sampleRate: 0.05
    forceSampleOnError: true
  loki:
    enabled: false
    endpoint: ""
  grafana:
    datasources:
      tempo:
        enabled: false
        url: ""
      loki:
        enabled: false
        url: ""
```

---

## 8. Gestion de la cardinalité

### 8.1 Le problème — cardinality bomb

Avec ServiceMonitor relabel D11, chaque métrique acquiert un
label `tenant`. À 100 tenants × 12 métriques × 4 status codes,
on parle déjà de ~ 18 000 séries (cf. § 5.4). Aux 1 000 tenants
de la SLO D11-MT-7, c'est 180 000 séries pour la même surface.

Si un partenaire crée par erreur 100 catalogues × 50 versions =
5 000 chaînes, la métrique `efe_catalog_versions_per_name`
explose à 5 000 séries pour ce seul tenant.

**Garantie de D13** : aucune métrique ne peut faire exploser
Prometheus si les quotas D11/D12 sont respectés.

### 8.2 Stratégies appliquées

#### 8.2.1 Pas de label haute-cardinalité « libre »

Métriques qui ont du `class_name`, `catalog_name`, `name` :

* **Bornées** par quotas D12 (max 100 catalogs / tenant ;
  max 256 classes / catalog).
* **Documentées** dans `docs/howto/cardinality-management.md`
  comme « cardinality-managed labels ».

Métriques **sans** label haute-cardinalité :

* `efe_grpc_request_duration_seconds` n'a pas de
  `request_id`, `trace_id` (jamais — c'est antitéthique au
  modèle Prometheus).
* `efe_classify_total` a `class` mais bornée par schéma.

#### 8.2.2 Recording rules `topk` pour la longue traîne

Le dashboard 02 Tenant Drill-Down doit afficher « top 20
tenants par RPS ». Au lieu de :

```promql
# BAD : crée une série par tenant rendered, 1000 séries au render
sum(rate(efe_grpc_requests_total[5m])) by (tenant)
```

on utilise la recording rule pré-calculée :

```promql
# GOOD : 20 séries seulement
efe:top20_tenants_by_rps:5m
```

Pour les autres tenants, un panel séparé montre l'agrégat
`_other_` (sum over tenants not in topk).

#### 8.2.3 Hashed-tenant fallback

Pour les use cases où la longue traîne doit être visible mais
pas tout-à-fait nommément, D13 introduit un label optionnel
`tenant_bucket` (modulo 100) sur certaines métriques :

```yaml
# servicemonitor.yaml — hashing rule
- source_labels: [tenant]
  target_label: tenant_bucket
  regex: "tenant-(.+)"
  replacement: "${1}"
  action: hashmod
  modulus: 100
```

Limite la cardinalité de `tenant_bucket` à 100 (constante)
quel que soit le nombre de tenants. Utile pour des heatmaps
sans dévoiler les noms.

#### 8.2.4 Alerte cardinality bomb

```yaml
# prometheusrule-d13.yaml
- alert: D13ObsCardinalityExceedsBudget
  expr: |
    count(count by (__name__, job) ({__name__=~"efe_.+"})) > 50000
  for: 15m
  labels:
    severity: warning
    runbook: d13_obs_cardinality_bomb
  annotations:
    summary: "Total cardinality of efe_* metrics exceeds 50k series"
    description: "Investigate label leak ; see runbook."
```

### 8.3 Budget chiffré

| Source | Budget normal | Budget panic |
|--------|---------------|--------------|
| Sum efe_* metrics | ≤ 30 000 séries | 50 000 (alerte) |
| Sum recording rules `efe:*` | ≤ 1 000 séries | 5 000 |
| Loki streams (= label combinations) | ≤ 10 000 streams | 50 000 |
| Tempo daily span volume | ≤ 1 GB / tenant | 10 GB (review) |

---

## 9. Spécifications des 6 dashboards persona-driven

Chaque dashboard suit ce gabarit :

* **Header** : title, refresh interval, time range, variables.
* **Top row** : SLO compliance heatmap (D6/D7/D8/D9/D11/D12/D13
  families au choix selon la persona).
* **Body** : 4-8 rows thématiques.
* **Footer** : « Need help? » avec liens vers runbooks.

Les variables Grafana communes :

| Variable | Type | Source | Default |
|----------|------|--------|---------|
| `$tenant` | query | `label_values(efe_grpc_requests_total, tenant)` | All |
| `$rpc` | query | `label_values(efe_grpc_requests_total, method)` | All |
| `$catalog_name` | query | `label_values(efe_catalog_versions_per_name, name)` | All |
| `$datasource` | datasource | (filterable) | `Prometheus` |

### 9.1 Dashboard `01_operator_overview` (P1 SRE on-call)

**JTBD** : « Le service est-il en santé ? Si non, où est le
problème ? » — répondu en < 10 sec.

#### 9.1.1 Layout

```
┌─────────────────────────────────────────────────────────────┐
│  Row 0 — SLO compliance heatmap (read-only summary)         │
└─────────────────────────────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────┐
│  Row 1 — Golden signals (4 stat panels)                     │
│  [Latency p99] [Error rate %] [Throughput RPS] [Saturation] │
└─────────────────────────────────────────────────────────────┘
┌──────────────────────┬──────────────────────────────────────┐
│  Row 2 — Latency     │  Row 3 — Error rate per RPC          │
│  histogram + exemplars │  stack chart by code                │
└──────────────────────┴──────────────────────────────────────┘
┌──────────────────────┬──────────────────────────────────────┐
│  Row 4 — Top 5       │  Row 5 — Active alerts list          │
│  noisy tenants       │  with runbook links                  │
└──────────────────────┴──────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────┐
│  Row 6 — Recent errors logs (Loki tail, last 5 min)         │
└─────────────────────────────────────────────────────────────┘
```

#### 9.1.2 Panels-clés

* **Stat: Latency p99** — `histogram_quantile(0.99,
  sum(rate(efe_grpc_request_duration_seconds_bucket[1m]))
  by (le))`. Threshold 500 ms warning, 1 s critical.
* **Stat: Error rate** — `sum(rate(efe_grpc_requests_total
  {code!="OK"}[5m])) / sum(rate(efe_grpc_requests_total[5m]))`.
  Threshold 1 % warn, 5 % crit.
* **Histogram: Latency distribution** —
  `efe_grpc_request_duration_seconds_bucket` heatmap with
  exemplars enabled.
* **Time series: Top 5 noisy tenants** — `topk(5,
  efe:rpc_error_rate:5m)`.
* **Logs panel: Recent errors** — LogQL `{stream="request",
  level="ERROR"} |= ""` ordered desc, limit 50.
* **Alert list** — Grafana built-in panel querying
  Alertmanager.

#### 9.1.3 Drilldown links

* Click sur tenant cell → `02_tenant_drill_down?var-tenant=X`.
* Click sur exemplar (traceID) → Tempo trace view.
* Click sur alert → runbook URL (annotation `runbook_url`).

### 9.2 Dashboard `02_tenant_drill_down` (P3 Support T1/T2)

**JTBD** : « Le client X dit que c'est lent / cassé. Que se
passe-t-il pour CE tenant ? » — répondu en < 30 sec.

#### 9.2.1 Layout

```
┌─────────────────────────────────────────────────────────────┐
│  Row 0 — Tenant selector + tenant info card                 │
│  [tier] [active catalogs] [last activity] [quota usage]     │
└─────────────────────────────────────────────────────────────┘
┌──────────────────────┬──────────────────────────────────────┐
│  Row 1 — Tenant      │  Row 2 — Tenant error rate           │
│  RPS over 24h         │  by RPC, last 24h                   │
└──────────────────────┴──────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────┐
│  Row 3 — Tenant latency p50/p95/p99 by RPC (multi-line)     │
└─────────────────────────────────────────────────────────────┘
┌──────────────────────┬──────────────────────────────────────┐
│  Row 4 — Tenant      │  Row 5 — Tenant rate limit           │
│  classify confidence  │  drops + idempotency hits           │
│  distribution         │                                      │
└──────────────────────┴──────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────┐
│  Row 6 — Tenant log tail (filterable by stream)             │
└─────────────────────────────────────────────────────────────┘
```

#### 9.2.2 Variables

* `$tenant` (required, no All) — single-select.

#### 9.2.3 Panels-clés

* **Card: Tenant info** — gauge of quotas (catalogs, versions,
  classes per catalog).
* **Time series: Tenant RPS** — `sum(rate(efe_grpc_requests_total
  {tenant="$tenant"}[5m])) by (method)`.
* **Time series: Tenant latency by percentile** — three lines
  (p50, p95, p99) using histogram_quantile.
* **Histogram: Confidence distribution** —
  `efe_classify_confidence_bucket{tenant="$tenant"}` heatmap.
* **Stat: Rate limit drops 24h** — `sum(increase(
  efe_ratelimit_drops_total{tenant="$tenant"}[24h]))`.
* **Logs panel: Tenant log tail** — LogQL `{tenant_id="$tenant"}`
  with stream filter dropdown.

#### 9.2.4 Drilldown links

* Click sur catalog name → `03_catalog_version_distribution
  ?var-tenant=$tenant&var-catalog_name=Y`.

### 9.3 Dashboard `03_catalog_version_distribution` (P4 Curator)

**JTBD** : « Quelles versions de mes catalogues sont actives ?
Y a-t-il des tenants encore sur des versions vétustes ? »

#### 9.3.1 Layout

```
┌─────────────────────────────────────────────────────────────┐
│  Row 0 — Filters: $tenant, $catalog_name                    │
└─────────────────────────────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────┐
│  Row 1 — Version chain visualisation per (tenant, name)     │
│  ASCII waterfall with color-coding (green=latest, gray=old) │
└─────────────────────────────────────────────────────────────┘
┌──────────────────────┬──────────────────────────────────────┐
│  Row 2 — Version     │  Row 3 — Evolution op rate           │
│  age distribution    │  by op_tag (last 7d)                 │
└──────────────────────┴──────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────┐
│  Row 4 — Catalog quality stats (n_classes, n_anchors,       │
│           confidence_high, last_evolved_at)                 │
└─────────────────────────────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────┐
│  Row 5 — Recent evolution audit events (Loki feed)          │
└─────────────────────────────────────────────────────────────┘
```

#### 9.3.2 Panels-clés

* **Table: Version chain** — query `efe:catalog_chain_depth:1m
  {tenant=~"$tenant", name=~"$catalog_name"}` rendered as
  table with cell color thresholds.
* **Time series: Evolution op rate** — `efe:catalog_evolve_rate:5m`
  stacked by `op_tag`.
* **Stat: Average chain depth** — `avg(efe_catalog_versions_per_name)`.
* **Logs panel: Audit events** — LogQL `{stream="evolution",
  tenant_id=~"$tenant"}`.

### 9.4 Dashboard `04_cost_attribution` (P5 Finance)

**JTBD** : « Combien chaque tenant nous coûte sur le mois en
cours ? » — réconciliable avec `aggregate_usage.py` D11.

#### 9.4.1 Layout

```
┌─────────────────────────────────────────────────────────────┐
│  Row 0 — Time range selector (default = month-to-date)      │
└─────────────────────────────────────────────────────────────┘
┌──────────────────────┬──────────────────────────────────────┐
│  Row 1 — Top 10      │  Row 2 — Cost trend, last 30 days    │
│  tenants by spend    │  per top-5 tenants                   │
└──────────────────────┴──────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────┐
│  Row 3 — Cost breakdown by category                         │
│  [CPU minutes] [RAM GB-hours] [requests] [anchors compiled] │
└─────────────────────────────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────┐
│  Row 4 — Cost vs SLA matrix (tenant tier vs cost percentile)│
└─────────────────────────────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────┐
│  Row 5 — Reconciliation ratio vs aggregate_usage.py output  │
└─────────────────────────────────────────────────────────────┘
```

#### 9.4.2 Panels-clés

* **Bar gauge: Top 10 spend** —
  `topk(10, sum(efe:cpu_minutes_per_tenant:1h
  + efe:ram_gb_hours_per_tenant:1h * 8) by (tenant))`.
* **Time series: Cost trend** —
  `sum(rate(container_cpu_usage_seconds_total{
    namespace=~"tenant-.+"}[1h])) by (namespace)`.
* **Stat: Drift vs aggregator** — comparison with the latest
  CSV produced by `aggregate_usage.py` (joined via Loki query
  on the `cost::` log stream).

#### 9.4.3 Drilldown links

* Click sur tenant cell → `02_tenant_drill_down`.

### 9.5 Dashboard `05_audit_stream` (P6 Security)

**JTBD** : « Y a-t-il des patterns suspects dans les
mutations / auth failures ? »

#### 9.5.1 Layout

```
┌─────────────────────────────────────────────────────────────┐
│  Row 0 — Filters: $tenant, $action, $op_tag                 │
└─────────────────────────────────────────────────────────────┘
┌──────────────────────┬──────────────────────────────────────┐
│  Row 1 — Audit       │  Row 2 — Top 10 principals           │
│  event rate by action │  by mutation count                  │
└──────────────────────┴──────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────┐
│  Row 3 — Auth failure rate (security stream)                │
└─────────────────────────────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────┐
│  Row 4 — Anomaly detection: spike in `remove_class` or       │
│          `recompile` (z-score over 7d baseline)             │
└─────────────────────────────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────┐
│  Row 5 — Audit event tail (Loki) — last 100 events          │
└─────────────────────────────────────────────────────────────┘
```

#### 9.5.2 Panels-clés

* **Time series: Audit rate** — `sum(rate(efe_audit_events_total[5m]))
  by (action)`.
* **Table: Top principals** — derived from Loki query
  `topk(10, sum by (principal) (rate({stream="audit"}[24h]
  | json)))`.
* **Stat: Auth failure rate** — `sum(rate(efe_authz_failures_total[5m]))`.
* **Anomaly panel** — uses Grafana ML feature OR PromQL
  `(efe:catalog_evolve_rate:5m{op_tag="remove_class"}
  - avg_over_time(efe:catalog_evolve_rate:5m{op_tag="remove_class"}[7d]))
  / stddev_over_time(efe:catalog_evolve_rate:5m{op_tag="remove_class"}[7d])`
  (z-score). Alert when |z| > 3.

### 9.6 Dashboard `06_capacity_planning` (P2 Capacity SRE)

**JTBD** : « Combien de têtes ai-je besoin la semaine prochaine /
le mois prochain ? Quelle métrique va saturer ? »

#### 9.6.1 Layout

```
┌─────────────────────────────────────────────────────────────┐
│  Row 0 — Time range: rolling 30d                            │
└─────────────────────────────────────────────────────────────┘
┌──────────────────────┬──────────────────────────────────────┐
│  Row 1 — Total tenants│  Row 2 — Total catalogs             │
│  (gauge + 30d trend)  │  (gauge + 30d trend)                │
└──────────────────────┴──────────────────────────────────────┘
┌──────────────────────┬──────────────────────────────────────┐
│  Row 3 — RPS         │  Row 4 — CPU/RAM saturation          │
│  forecast (Holt-Win) │  with 80% / 90% headroom markers     │
└──────────────────────┴──────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────┐
│  Row 5 — Quota usage matrix (which tenants near quotas)     │
└─────────────────────────────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────┐
│  Row 6 — Cardinality budget gauge (50k panic threshold)     │
└─────────────────────────────────────────────────────────────┘
```

#### 9.6.2 Panels-clés

* **Stat: Total active tenants** — `efe_active_tenants`.
* **Stat: Total catalogs** — `count(efe:catalog_chain_depth:1m)`.
* **Time series: RPS forecast** — Grafana Holt-Winters
  smoothing on `sum(rate(efe_grpc_requests_total[5m]))`.
* **Heatmap: CPU / RAM saturation** — `container_cpu_usage`
  vs `kube_pod_container_resource_limits_cpu_cores`.
* **Table: Quota usage** — table query of
  `efe_catalog_versions_per_name / on(name) group_left
  max_versions_quota`.
* **Gauge: Cardinality budget** — `count({__name__=~"efe_.+"})`.

---

## 10. Alertes + runbooks linking

### 10.1 Inventaire des alertes D13

10 nouvelles alertes Prometheus. Chacune a un `runbook_url`
annotation pointant vers `docs/runbooks/d13_<id>.md`.

| ID | Severity | Trigger | Runbook |
|----|----------|---------|---------|
| `D13ObsHighErrorRate` | critical | err rate > 5 % for 10 min | `d13_high_error_rate.md` |
| `D13ObsHighLatencyP99` | critical | p99 > 1 s for 10 min | `d13_high_latency_p99.md` |
| `D13ObsClassifyDriftHigh` | warning | confidence p50 dropped > 10 % vs 7d baseline | `d13_classify_drift.md` |
| `D13ObsCatalogVersionExplosion` | warning | one (tenant, name) chain > 200 versions | `d13_catalog_version_explosion.md` |
| `D13ObsAuthFailureSpike` | warning | auth failures > 10 / sec for 5 min | `d13_auth_failure_spike.md` |
| `D13ObsCardinalityBomb` | warning | total cardinality > 50k for 15 min | `d13_obs_cardinality_bomb.md` |
| `D13ObsAuditPrincipalUnknown` | warning | > 5 % audit events with `principal=""` | `d13_audit_unknown_principal.md` |
| `D13ObsRunbookMissing` | warning | any alert lacks `runbook_url` annotation (CI test) | `d13_runbook_missing.md` |
| `D13ObsTraceSamplingMisconfigured` | warning | `< 1 %` traces sampled when SLA > 99 % | `d13_trace_sampling.md` |
| `D13ObsLogPipelineLag` | warning | promtail position cursor > 30s behind | `d13_log_pipeline_lag.md` |

### 10.2 Format runbook (template)

```markdown
# d13_<id> — Runbook : <title>

> **Severity** : critical / warning
> **Owner** : SRE on-call rotation
> **Updated** : 2026-04-27
> **Related dashboards** : 01_operator_overview, 02_tenant_drill_down

## 1. Alert summary

(2 sentences max ; what fired, what threshold)

## 2. First 5 minutes — triage

- [ ] Step 1 (with commands)
- [ ] Step 2
- [ ] Step 3
- [ ] Step 4
- [ ] Step 5

## 3. Common root causes

| Cause | Symptom | Fix |
|-------|---------|-----|
| ... | ... | ... |

## 4. Mitigation

(immediate actions to stop the bleed)

## 5. Resolution

(durable fix : code change, infra change)

## 6. Escalation

If still unresolved at T+30 min : `@efe-oncall-tier-2`.

## 7. Postmortem template

(link to `docs/operations/postmortem-template.md`)
```

### 10.3 CI test : runbook coverage

```python
# scripts/tests/test_d13_runbook_coverage.py
"""Every D13 alert MUST have a `runbook_url` annotation pointing to an
existing file."""

def test_all_d13_alerts_have_runbook_url():
    for alert in load_alerts("deploy/helm/efe/templates/prometheusrule-d13.yaml"):
        url = alert["annotations"].get("runbook_url")
        assert url, f"alert {alert['alert']!r} missing runbook_url"
        path = parse_url_to_path(url)
        assert Path(path).exists(), f"runbook {path} not found"
```

### 10.4 Lien Grafana → runbook

Les panels Grafana qui surface une alerte affichent le
`runbook_url` comme link cliquable. Le link template Grafana :

```json
{
  "links": [
    {"title": "Runbook", "url": "${__data.fields.runbook_url}"}
  ]
}
```

---

## 11. Distribution Helm

### 11.1 ConfigMaps + sidecar Grafana

Les 6 dashboards JSON sont rendus en ConfigMaps Helm-templated,
labelisés `grafana_dashboard=1`. Le sidecar Grafana
(`kiwigrid/k8s-sidecar`) auto-discovers les ConfigMaps avec ce
label dans le cluster et les charge dans Grafana.

```yaml
# deploy/helm/efe/templates/grafana-dashboards.yaml
{{- if .Values.observability.dashboards.enabled }}
{{- range $file, $_ := .Files.Glob "dashboards/d13/*.json" }}
{{- $slug := base $file | trimSuffix ".json" }}
apiVersion: v1
kind: ConfigMap
metadata:
  name: {{ include "efe.fullname" $ }}-dashboard-{{ $slug }}
  labels:
    {{- include "efe.labels" $ | nindent 4 }}
    grafana_dashboard: "1"
data:
  {{ $slug }}.json: |
    {{- $.Files.Get $file | nindent 4 }}
{{- end }}
{{- end }}
```

### 11.2 Values.yaml (D13 ajouts)

```yaml
observability:
  dashboards:
    enabled: true               # default ON in Tier 2
    folder: "EFE"               # Grafana folder
    refresh: "30s"              # default refresh
  alerts:
    enabled: true
    severityOverride: {}         # per-alert overrides
  recordingRules:
    enabled: true
    interval: "30s"
  cardinality:
    panicBudget: 50000
  loki:
    enabled: false
    endpoint: ""
    promtailExample: true       # ship example config
  tempo:
    enabled: false
    endpoint: ""
  otel:
    enabled: false              # client-side enable
    sampleRate: 0.05
    forceSampleOnError: true
```

### 11.3 Helm test : dashboards-load

Test Helm livré pour valider qu'un Grafana de dev charge bien
les 6 dashboards :

```yaml
# deploy/helm/efe/templates/tests/test-dashboards-load.yaml
{{- if .Values.observability.dashboards.enabled }}
apiVersion: v1
kind: Pod
metadata:
  name: {{ include "efe.fullname" . }}-test-dashboards-load
  annotations:
    "helm.sh/hook": test
spec:
  containers:
  - name: test
    image: curlimages/curl:8.4.0
    command: ['sh', '-c', '
      for d in 01_operator_overview 02_tenant_drill_down 03_catalog_version_distribution 04_cost_attribution 05_audit_stream 06_capacity_planning; do
        curl -fs http://grafana:3000/api/dashboards/uid/efe-d13-$d > /dev/null \
          || (echo "dashboard $d not found"; exit 1)
      done; echo OK'
    ]
{{- end }}
```

---

## 12. Plan de données multi-tenant

### 12.1 Modèle de séparation

Trois niveaux de séparation Grafana :

* **Folder-based** (per-product) : tous les dashboards EFE
  sont dans le folder `EFE`. Permet à un opérateur multi-
  produit de ne pas mélanger les concerns.
* **Org-based** (per-customer) : chaque tenant client a sa
  propre Grafana org (Grafana 10+ supporte multi-tenancy
  natif). Visible : Grafana > Server admin > Orgs.
* **Datasource RBAC** (per-team) : la datasource Prometheus
  filtrée tenant=X par convention (proxy datasource côté
  Mimir / Cortex).

### 12.2 Default safety — auto-filter tenant variable

Tous les dashboards qui exposent du tenant data ont le
filtre `$tenant` activé par défaut sur le tenant de la
session. Il y a 3 niveaux :

* **Strict mode** (default for partner orgs) : `$tenant`
  est hardcoded à la valeur de la session, l'utilisateur ne
  peut pas le changer.
* **Selectable** (default for operator orgs) : `$tenant`
  est sélectionnable via dropdown, default = first.
* **All** (admin override) : `$tenant=All`, requête
  cross-tenant. Disabled par défaut.

### 12.3 Multi-tenant Helm — comment

```yaml
# values.tier2-partner.yaml
grafana:
  multiTenancy:
    enabled: true
    tenantOrg:
      template: "tenant-{{ .Values.tenant.id }}"
    defaultRole: "Viewer"
    dashboardsFolder: "EFE-{{ .Values.tenant.id }}"
  datasource:
    prometheus:
      url: "http://mimir:8080/prometheus"
      jsonData:
        httpHeaderName1: "X-Scope-OrgID"
      secureJsonData:
        httpHeaderValue1: "{{ .Values.tenant.id }}"
```

Le `X-Scope-OrgID` header est intercepté par Mimir
multi-tenant qui filtre les séries au niveau du backend.
Aucune requête PromQL d'un tenant ne peut voir les séries
d'un autre — séparation cryptographique forte.

### 12.4 Cross-tenant operator view

L'opérateur SaaS-de-EFE garde une org Grafana « SaaS-Admin »
qui a accès à toutes les séries. Cette org NE doit JAMAIS être
exposée à un client. C'est documenté dans
`docs/operations/grafana-multitenancy.md`.

---

## 13. Cycle de vie des données

### 13.1 Retention par couche

| Couche | High-cardinality | Aggregated (recording rules) | Logs | Traces |
|--------|------------------|-------------------------------|------|--------|
| **Hot** (in-memory) | 2h Prom WAL | 2h | – | – |
| **Warm** (local SSD) | 7d | 30d | 14d (raw) | 7d (sampled) |
| **Cold** (object storage S3) | 90d | 365d | 90d (downsampled) | n/a (Tempo n'a pas de tier cold natif) |

### 13.2 Downsampling Prometheus → Mimir/Thanos

Recording rules **deux niveaux** :

```yaml
- name: efe-d13-recording-1h
  interval: 5m
  rules:
    - record: efe:rpc_p99_latency_seconds:1h
      expr: |
        avg_over_time(efe:rpc_p99_latency_seconds:5m[1h])

- name: efe-d13-recording-1d
  interval: 1h
  rules:
    - record: efe:rpc_p99_latency_seconds:1d
      expr: |
        avg_over_time(efe:rpc_p99_latency_seconds:1h[1d])
```

Les agrégats `:1h` et `:1d` sont retainables 365 jours sur
S3 sans coût excessif. Les dashboards capacity-planning les
utilisent automatiquement quand le time-range > 30 jours.

### 13.3 Export pour archivage compliance

Audit log stream **doit** être archivé 7 ans pour conformité
(cf. `docs/explanation/security-model.md` D11). D13 fournit un
script `scripts/observability/export_audit_archive.py` qui
:

1. Pull from Loki via `logcli` les events `stream=audit` sur
   une fenêtre.
2. Convertit en NDJSON gzipé.
3. Upload sur S3 avec object-lock 7 ans.

---

## 14. Modèle de sécurité Grafana

### 14.1 Trois rôles Grafana

| Rôle | Accès dashboards | Accès raw queries | Accès admin |
|------|------------------|-------------------|-------------|
| `Viewer` | Read-only | Non | Non |
| `Editor` | Read + clone | Oui (PromQL/LogQL/TraceQL) | Non |
| `Admin` | Read + edit | Oui | Org admin |

### 14.2 Mapping persona → rôle

| Persona | Rôle Grafana par défaut |
|---------|-------------------------|
| P1 SRE on-call | Editor (drill capability) |
| P2 Capacity SRE | Editor |
| P3 Support T1/T2 | Viewer |
| P4 Catalog curator | Viewer |
| P5 Finance / FinOps | Viewer |
| P6 Security analyst | Editor (need raw LogQL) |
| Customer partner | Viewer (org-scoped) |

### 14.3 Service accounts

Tokens Grafana scoped pour automation :

* `efe-dashboard-deployer` : créé par le sidecar pour reload
  les dashboards. RBAC: Admin sur le folder `EFE`.
* `efe-alert-poster` : Alertmanager → Grafana annotations.
  RBAC: write annotations only.
* `efe-readonly-bot` : pour `scripts/observability/health_*.py`.
  RBAC: Viewer.

Tous les tokens sont stockés en `kubernetes.io/secret`
typed et rotatifs via `rotate_grafana_tokens.py` (mensuel).

### 14.4 Audit Grafana

Toutes les actions Grafana (dashboard create/edit, datasource
change, user invite) émettent un audit event dans le stream
`security`. Format :

```json
{
  ...common,
  "stream": "security",
  "event": "grafana_admin_action",
  "actor": "alice@acme.com",
  "action": "dashboard.update",
  "target": "01_operator_overview",
  "diff_summary": "panel 3 query changed"
}
```

Activé via Grafana 10+ enterprise OR via webhook de
`access_control_log`.

---

## 15. Threat model

10 vecteurs HAZOP observabilité-spécifiques. Chacun avec :
likelihood, impact, mitigation, test.

### T-13-1 — Cardinality bomb via label injection

* **Likelihood** : moyen (un partenaire mal-codé peut
  injecter un user-controlled label).
* **Impact** : critique (Prom OOM, observabilité down).
* **Mitigation** : ServiceMonitor `metricRelabelings`
  allowlist (déjà en place D11) + alerte
  `D13ObsCardinalityBomb`.
* **Test** : `test_d13_metric_label_allowlist.py` injecte
  un fake label, vérifie qu'il est stripé au scrape.

### T-13-2 — Cross-tenant data leak via shared Grafana org

* **Likelihood** : élevé si Grafana mal-configuré.
* **Impact** : confidentialité tenant rompue.
* **Mitigation** : multi-tenancy Mimir avec `X-Scope-OrgID`
  forced via secret + Helm test cross-org isolation.
* **Test** : `test_d13_grafana_multitenancy.py` simule deux
  tenants, vérifie que les queries de l'un retournent 0
  séries de l'autre.

### T-13-3 — Trace data exposure of PII

* **Likelihood** : moyen (les span attributes peuvent contenir
  user inputs).
* **Impact** : RGPD breach.
* **Mitigation** : whitelist de span attributes (only
  metadata, jamais le payload). Test : `traceID` exists for
  every classify but no `text` attribute.
* **Test** : `test_d13_trace_attribute_whitelist.py`.

### T-13-4 — Logs exposing JWT tokens

* **Likelihood** : faible (logger config strict) mais possible
  si un panic prints le request body.
* **Impact** : tokens exfiltrés via Loki.
* **Mitigation** : redact pattern `Bearer [a-zA-Z0-9._-]+` au
  shipper (promtail pipeline). Test :
  `test_d13_log_redaction.py` ingest a synthetic event with a
  fake JWT, asserts redacted.

### T-13-5 — Alert flapping causing notification fatigue

* **Likelihood** : élevé sur des thresholds mal calibrés.
* **Impact** : on-call apnée (oncall ignores legitimate
  alerts).
* **Mitigation** : `for: 10m` minimum sur toutes les
  alerts ; `repeat_interval: 4h` Alertmanager ; weekly
  alert hygiene review.
* **Test** : `test_d13_alert_for_clause.py` asserts every
  D13 alert has `for >= 5m`.

### T-13-6 — Recording rules creating runaway computations

* **Likelihood** : moyen (un opérateur ajoute une rule
  expensive).
* **Impact** : Prom CPU saturation, scrape misses.
* **Mitigation** : recording rules in CI sont validées par
  `promtool check rules` qui détecte les expr expensive.
* **Test** : `test_d13_promtool_validation.py`.

### T-13-7 — Loki ingester crash via massive log spike

* **Likelihood** : faible mais non nul (un bug efe-server
  spam logs).
* **Impact** : Loki down, log query down.
* **Mitigation** : Loki ingester rate limit + per-tenant
  request rate limit at promtail level + Loki retention
  cap.
* **Test** : `test_d13_loki_rate_limit.py`.

### T-13-8 — Tempo span volume explosion

* **Likelihood** : moyen si sampling débridé.
* **Impact** : Tempo S3 bill spike, query slow.
* **Mitigation** : sample-rate cap 10 % + per-tenant
  span budget + alert on volume.
* **Test** : `test_d13_tempo_volume_cap.py`.

### T-13-9 — Dashboard JSON corruption breaks all panels

* **Likelihood** : faible (Helm CI tests).
* **Impact** : entire dashboard set unusable.
* **Mitigation** : JSON-Schema validate every dashboard
  in CI ; bench-observability workflow.
* **Test** : `test_d13_dashboard_jsonschema.py`.

### T-13-10 — Audit log spoofing via stdout injection

* **Likelihood** : très faible (stdout est trusted).
* **Impact** : false attribution → security forensics
  compromise.
* **Mitigation** : audit stream signé HMAC server-side
  (D14 deferred) ; D13 documente le risk + mitigation
  procédurale (timing analysis).
* **Test** : N/A pour D13 ; tracking en backlog.

---

## 16. 12 décisions de conception clés

### D-13-1 — Stack open-source par défaut (Prometheus + Grafana + Loki + Tempo)

* **Choix** : OSS exclusively in default-Helm.
* **Rationale** : portable, scriptable, no vendor lock-in.
* **Alternative rejetée** : Datadog, Newrelic, Splunk —
  vendor lock + cost.
* **Conséquence** : ops doit operer 4 services. Acceptable —
  les images sont stables, la communauté est massive.

### D-13-2 — Dashboards JSON committed in repo, NOT auto-generated

* **Choix** : 6 fichiers JSON hand-written + JSON-Schema
  validated.
* **Rationale** : reviewable comme du code, diffable, version-
  controlled.
* **Alternative rejetée** : Grafonnet/jsonnet — adds
  dependency, harder to read for non-Grafana experts.
* **Conséquence** : maintenance manuelle des dashboards.
  Outillage : un linter `jsonschema -i dashboards/*.json
  schema.json`.

### D-13-3 — Sidecar discovery via ConfigMap labels (not API push)

* **Choix** : `grafana_dashboard=1` label, sidecar polls.
* **Rationale** : déclaratif, rollback Helm = rollback
  dashboards.
* **Alternative rejetée** : push via Grafana API at deploy —
  introduces stateful-ness, harder to debug.
* **Conséquence** : Grafana doit avoir le sidecar
  k8s-sidecar installé. Documenté dans
  `docs/howto/observability-setup.md`.

### D-13-4 — Recording rules pour topk + downsampling

* **Choix** : 7 recording rules + 2 niveaux de downsampling.
* **Rationale** : bornes les query costs ; permet retention 365j.
* **Alternative rejetée** : query directe — dashboard
  crashes Prom à 1000+ tenants.
* **Conséquence** : un délai de 30s avant que les recording
  rules soient à jour. Acceptable pour les use cases D13.

### D-13-5 — Logs en JSON structuré sur stdout, pipeline externe

* **Choix** : 5 streams JSON normalisés ; promtail/fluentd
  ramasse.
* **Rationale** : flexible — operator choisit son backend.
* **Alternative rejetée** : direct Loki client embedded in
  efe-server — ajoute une dep, network coupling.
* **Conséquence** : nécessite un agent de log shipping. On
  fournit un example promtail config.

### D-13-6 — JSON-Schema versionné pour les logs (`schema_version`)

* **Choix** : champ `schema_version` obligatoire, JSON-Schema
  validation en CI.
* **Rationale** : forward-compat sans coup à un partenaire
  qui parse les logs.
* **Alternative rejetée** : pas de versioning — le moindre
  changement casse les pipelines downstream.
* **Conséquence** : un test `test_d13_log_schemas.py`.

### D-13-7 — OpenTelemetry on-by-default côté serveur, opt-in côté client

* **Choix** : `efe-server` instrumente toujours ; export
  désactivé par défaut. Client SDK ajoute `[otel]` extra.
* **Rationale** : observabilité côté serveur sans coût
  obligatoire pour le client.
* **Alternative rejetée** : on-by-default partout — surcharge
  clients minimal-CPU.
* **Conséquence** : un tenant qui veut tracer son SDK doit
  installer `efe-tool[otel]==0.1.13`.

### D-13-8 — Sampling head-based 5 % par défaut + always-on errors

* **Choix** : trace_id parent-based, ratio 5 %, force-on
  errors / slow.
* **Rationale** : 5 % traces = ~ 99 % capture des problèmes
  rare-mais-importants.
* **Alternative rejetée** : tail-based sampling — requires
  a more complex collector setup.
* **Conséquence** : 95 % des requests OK ne sont pas
  tracées — accepté.

### D-13-9 — Multi-tenant via Mimir `X-Scope-OrgID`, opt-in

* **Choix** : Mimir multi-tenancy à activer dans values.yaml.
* **Rationale** : isolation cryptographique forte.
* **Alternative rejetée** : query-time filtering — bypassable
  via raw query.
* **Conséquence** : déploiement Tier 2 partner doit utiliser
  Mimir/Cortex/Thanos avec multi-tenancy ON.

### D-13-10 — Runbook URL annotation **mandatory** sur chaque alerte

* **Choix** : CI rejects PRs with alert sans `runbook_url`.
* **Rationale** : un alert sans runbook est inutile.
* **Alternative rejetée** : runbook in alert description —
  trop long, hard to maintain.
* **Conséquence** : 10 runbooks à écrire ; pinned by CI test.

### D-13-11 — Dashboard variables auto-filter tenant by default

* **Choix** : `$tenant` variable always present, default =
  session-tenant for partner orgs.
* **Rationale** : safety-first multi-tenant.
* **Alternative rejetée** : `$tenant=All` default — risk
  d'exposition cross-tenant accidentelle.
* **Conséquence** : opérateur SaaS doit override pour cross-
  tenant view.

### D-13-12 — SLO compliance heatmap row 0 sur chaque dashboard

* **Choix** : row 0 réservé à un panel SLO heatmap.
* **Rationale** : compliance state glanceable sans naviguer.
* **Alternative rejetée** : SLO panels disséminés — moins
  visible.
* **Conséquence** : un panel répété 6× ; templated via
  Grafana panel library.

---

## 17. 10 risques identifiés et mitigations

### R-13-1 — Cardinality bomb under tenant pressure

* **Description** : si 1000+ tenants actifs, certains
  recording rules saturent.
* **Mitigation** : alerte D13ObsCardinalityBomb, recording
  rules avec `topk(20, ...)`.
* **Owner** : SRE on-call ; runbook
  `d13_obs_cardinality_bomb.md`.
* **Date de revue** : à chaque migration vers + 100 tenants.

### R-13-2 — Dashboard render time degrades over time

* **Description** : à mesure que la cardinalité monte, les
  dashboards mettent + en + de temps à charger.
* **Mitigation** : SLO D13-OBS-1 ≤ 3 s ; recording rules ;
  tests synthétiques `test_d13_dashboard_render_time.py`.
* **Owner** : Platform team.

### R-13-3 — OTel SDK version drift breaks span shape

* **Description** : major version bump = wire change.
* **Mitigation** : pin OTel version in Cargo.toml ; CI
  rebuild check.
* **Owner** : Engineering.

### R-13-4 — Loki retention costs grow uncontrolled

* **Description** : un tenant spam logs → Loki S3 bill.
* **Mitigation** : per-tenant ingest limit + retention
  policies par stream.
* **Owner** : FinOps.

### R-13-5 — Tempo span volume spike on incident replays

* **Description** : un incident → operator augmente sample
  rate à 100 %, span volume × 20.
* **Mitigation** : `force_sample_on_error` cap @ 1k spans/sec
  per tenant.
* **Owner** : SRE.

### R-13-6 — Alert flapping causing on-call fatigue

* **Description** : trop d'alerts trop sensibles.
* **Mitigation** : weekly alert hygiene review (process) +
  `for: ≥ 5m` enforced by CI.
* **Owner** : Engineering manager.

### R-13-7 — Runbook drift (out-of-date)

* **Description** : un runbook référence une command qui
  n'existe plus.
* **Mitigation** : trimestriel runbook review ; CI test que
  chaque command linkée a un alias dans `efe-tool --help`.
* **Owner** : Tech lead.

### R-13-8 — Multi-tenant Grafana misconfig leaks

* **Description** : opérateur loupe le `X-Scope-OrgID`.
* **Mitigation** : `bench-observability.yml` test cross-org
  isolation.
* **Owner** : Security.

### R-13-9 — Dashboard JSON breaks on Grafana version upgrade

* **Description** : Grafana 10 → 11 change le format ?
* **Mitigation** : pin Grafana minimum version, integration
  tests.
* **Owner** : Platform.

### R-13-10 — Recording rules data drift after backfill

* **Description** : si Prom rebuild les recording rules,
  ils peuvent être incohérents avec l'historique.
* **Mitigation** : rule versioning ; backfill procedures
  documentées.
* **Owner** : SRE.

---

## 18. Plan jour-par-jour (6 j wallclock)

### J1 — Foundations & schemas

* Schémas JSON 5 streams + JSON-Schema files.
* Recording rules YAML + `promtool check rules`.
* Tests `test_d13_log_schemas.py`,
  `test_d13_promtool_validation.py`.
* Métriques D13 : ajouter les 8 nouvelles déclarations dans
  `crates/efe-telemetry/src/metrics.rs` (placeholders, pas de
  use sites encore).

### J2 — Métriques + traces côté serveur

* Implémenter les 8 nouvelles métriques dans efe-server.
* Activer OpenTelemetry SDK Rust (deps Cargo.toml).
* Spans pour les 6 RPCs catalog (squelettes, full impl
  tied à D12-bis).
* Tests `test_d13_metrics_emitted.py` + Rust unit tests.

### J3 — Dashboards 1, 2, 3

* `01_operator_overview.json` — golden signals layout.
* `02_tenant_drill_down.json` — tenant scoping.
* `03_catalog_version_distribution.json` — version chains.
* JSON-Schema validate chaque fichier.
* Helm template `grafana-dashboards.yaml`.

### J4 — Dashboards 4, 5, 6 + alerts

* `04_cost_attribution.json`.
* `05_audit_stream.json`.
* `06_capacity_planning.json`.
* `prometheusrule-d13.yaml` avec 10 alertes.
* Tests `test_d13_dashboard_inventory.py`,
  `test_d13_alert_runbook_links.py`.

### J5 — Runbooks + docs

* 10 runbooks `docs/runbooks/d13_*.md`.
* 4 pages Diataxis :
  * `docs/howto/observability-setup.md`
  * `docs/howto/cardinality-management.md`
  * `docs/reference/dashboard-catalog.md`
  * `docs/explanation/observability-architecture.md`
* `docs/mkdocs.yml` nav update.

### J6 — CI + multi-tenant + closure

* `.github/workflows/bench-observability.yml`.
* `test_d13_grafana_multitenancy.py`,
  `test_d13_metric_label_allowlist.py`.
* Helm tests `test-dashboards-load.yaml`.
* `BENCHMARKS.md` § 15 + `bench_baselines/v0.1.13/baseline.json`.
* CHANGELOG entry.
* Final cross-sprint regression.

---

## 19. SLOs ajoutés post-D13

7 nouveaux SLOs dans la family `D13-OBS-*` :

| ID | Cible | Bench key | Comparator |
|----|-------|-----------|------------|
| `D13-OBS-1` | Dashboard load p99 ≤ 3 s | `obs::dash::load_p99` | lte |
| `D13-OBS-2` | Metrics scrape success ≥ 99.9 % | `obs::dash::scrape_success_pct` | gte |
| `D13-OBS-3` | Log delivery p99 ≤ 30 s | `obs::dash::log_delivery_p99_s` | lte |
| `D13-OBS-4` | Trace sampling correctness ≥ 99 % | `obs::dash::trace_sampling_pct` | gte |
| `D13-OBS-5` | Cardinality bound respected | `obs::dash::cardinality_bound_count` | lte |
| `D13-OBS-6` | Alert→dashboard MTTR ≤ 30 s | `obs::dash::alert_to_dashboard_s` | lte |
| `D13-OBS-7` | Runbook coverage = 100 % | `obs::dash::runbook_coverage_pct` | gte |

Tous SKIPPED en `bench_baselines/v0.1.13/baseline.json`
jusqu'au 1er run vert post-merge de
`.github/workflows/bench-observability.yml`.

---

## 20. Extension `RESERVED_PREFIXES`

D13 étend la liste à 11 entrées :

```python
RESERVED_PREFIXES = (
    "eval_accuracy::",   # D6/D7/D8
    "eval_latency::",    # D6/D7/D8
    "load_",             # D9
    "soak::",            # D9
    "docs::",            # D10
    "dx::",              # D10
    "isolation::",       # D11
    "cost::",            # D11
    "obs::",             # D11
    "catalog::",         # D12
    "obs::dash::",       # D13 NEW
)
```

Pinned par `scripts/tests/test_d13_disjoint_keys.py`. Note :
`obs::dash::` est un sous-namespace volontairement préfixé par
`obs::` pour cohabiter avec D11 ; les SLOs D11 utilisent
`obs::tenant_cardinality_count`, les D13 utilisent
`obs::dash::*`. La disjointness check accepte les deux car
`obs::dash::` is a prefix-of-prefix valid sub-namespace.

Tests ajoutés :

* `test_d13_disjoint_keys.py` — inventaire RESERVED_PREFIXES,
  inventaire SLOs D13, namespacing, disjointness cross-sprint
  (8 tests).
* `test_d13_log_schemas.py` — JSON-Schema validation des 5
  streams (7 tests).
* `test_d13_dashboard_jsonschema.py` — chaque dashboard
  conforme au Grafana JSON schema (6 tests).
* `test_d13_alert_runbook_links.py` — chaque alerte a un
  `runbook_url` valide (10 tests, 1 per alert).
* `test_d13_promtool_validation.py` — `promtool check rules`
  exit 0 (1 test ; CI gates).
* `test_d13_dashboard_inventory.py` — surface pinning des 6
  dashboards UID/title/folder (6 tests, 1 per dashboard).
* `test_d13_grafana_multitenancy.py` — cross-org isolation
  smoke (3 tests).
* `test_d13_metric_label_allowlist.py` — relabel allowlist
  drops attacker-injected labels (4 tests).
* `test_d13_alert_for_clause.py` — every alert has `for ≥ 5m`
  (1 test).
* `test_d13_runbook_coverage.py` — every runbook file exists
  + has expected sections (10 tests).

**Total estimé : ~ 50 nouveaux tests Python anti-régression.**

---

## 21. Intégration CI — workflow `bench-observability.yml`

```yaml
name: bench-observability

on:
  push:
    branches: [main]
    paths:
      - "deploy/observability/**"
      - "deploy/helm/efe/templates/grafana-dashboards.yaml"
      - "deploy/helm/efe/templates/prometheusrule-d13.yaml"
      - "deploy/helm/efe/values.yaml"
      - "docs/runbooks/d13_*.md"
      - "scripts/tests/test_d13_*.py"
      - "crates/efe-telemetry/**"
      - ".github/workflows/bench-observability.yml"
  pull_request:
    paths: [...]  # même liste
  workflow_dispatch:

jobs:
  dashboards-validate:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - name: Validate dashboard JSON-Schema
        run: |
          for f in deploy/observability/grafana/dashboards/d13/*.json; do
            jsonschema -i "$f" deploy/observability/grafana/dashboard-schema.json
          done

  promtool-check:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - run: docker run --rm -v "$PWD:/work" prom/prometheus:v2.49.0 \
                promtool check rules /work/deploy/observability/prometheus/rules.d13/*.yaml

  helm-lint:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - uses: azure/setup-helm@v4
      - run: helm lint deploy/helm/efe
      - run: helm template efe deploy/helm/efe -f deploy/helm/efe/values.tier2.yaml \
                | kubectl apply --dry-run=client -f -

  python-tests:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with: { python-version: "3.11" }
      - run: pip install -e ./clients/python pytest pyyaml jsonschema requests
      - run: pytest scripts/tests/test_d13_*.py -v

  multi-tenant-isolation:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - uses: helm/kind-action@v1
      - name: Stand up dev Mimir + Grafana
        run: |
          helm upgrade --install mimir grafana/mimir-distributed
          helm upgrade --install grafana grafana/grafana \
              --set 'datasources."datasources.yaml".datasources[0].url=http://mimir:9009/prometheus' \
              --set 'datasources."datasources.yaml".datasources[0].jsonData.httpHeaderName1=X-Scope-OrgID'
      - name: Smoke test cross-org isolation
        run: pytest scripts/tests/test_d13_grafana_multitenancy.py -v
```

---

## 22. Critères d'acceptation (DoD)

D13 est **mergeable en main** quand :

1. **Code livré** :
   * 8 nouvelles métriques `efe_*` dans
     `crates/efe-telemetry/src/metrics.rs`.
   * 7 recording rules dans
     `deploy/observability/prometheus/rules.d13/`.
   * 6 dashboards JSON dans
     `deploy/observability/grafana/dashboards/d13/`.
   * 10 alertes dans
     `deploy/helm/efe/templates/prometheusrule-d13.yaml`.
   * 5 JSON-Schema files dans
     `deploy/observability/log-schemas/`.
   * 4 pages Diataxis +10 runbooks.
2. **Tests** : ≥ 50 nouveaux tests Python anti-régression
   (cross-sprint reste vert).
3. **CI** : `.github/workflows/bench-observability.yml` ✅.
4. **Helm** : `helm lint` ✅, `helm test` ✅.
5. **Docs** : `mkdocs build --strict` ✅.
6. **SLOs** : 7 D13-OBS-* enregistrés (SKIPPED jusqu'au
   1er run vert).
7. **CHANGELOG** entry sous `[Unreleased]` D13.
8. **`bench_baselines/v0.1.13/baseline.json`** placeholder
   committed.
9. **`BENCHMARKS.md` § 15** rédigé.
10. **No regression** : 70 SLOs antérieurs intacts,
    216 Python + 185 Rust toujours verts.

---

## 23. Out of scope & roadmap follow-ups

### 23.1 Hors-périmètre D13 (déféré à D14+)

* **Persistence des catalogues** (postgres/sled) — D14 dédié.
* **gRPC handler `CatalogService`** — D12-bis.
* **Audit log signing HMAC** — D14 ou plus tard, requiert
  tenant key management.
* **Auto-scaling basé sur les métriques D13** — D15.
* **Cost dashboards avec pricing intégré** — D15
  product-level.

### 23.2 Suivi explicite

* **Issue tracker** : créer 10 tickets « add runbook for
  d13_<id> » assignés à on-call rotation.
* **Audit semestriel** : alert hygiene review,
  dashboard usage stats (Grafana audit log).
* **Documentation update** : à chaque changement de schema
  log, bump `schema_version` + add migration note.

### 23.3 Indicateurs de succès post-launch

* MTTR moyen baisse de **30 min → < 5 min** sur les
  incidents Tier 2.
* % d'alertes répondues dans le SLO de 5 min ≥ 95 %.
* Dashboard usage ≥ 1 view/day/SRE.

---

## 24. Annexes

### Annexe A — PromQL queries de référence

#### A.1 Golden signals

```promql
# Latency p99
histogram_quantile(0.99,
  sum(rate(efe_grpc_request_duration_seconds_bucket[5m]))
    by (le, method)
)

# Error rate (5m)
sum(rate(efe_grpc_requests_total{code!="OK"}[5m])) by (method)
  / sum(rate(efe_grpc_requests_total[5m])) by (method)

# Throughput RPS
sum(rate(efe_grpc_requests_total[1m])) by (method)

# Saturation (CPU + memory)
container_cpu_usage_seconds_total / kube_pod_container_resource_limits_cpu_cores
container_memory_working_set_bytes  / kube_pod_container_resource_limits_memory_bytes
```

#### A.2 Per-tenant breakdown

```promql
# Top 10 tenants by RPS
topk(10, sum(rate(efe_grpc_requests_total[5m])) by (tenant))

# Tenants with elevated p99
topk(5, efe:rpc_p99_latency_seconds:5m > 0.5)

# Tenant catalog activity
sum(rate(efe_audit_events_total[1h])) by (tenant, action)
```

#### A.3 Catalog version freshness

```promql
# Distribution of chain depths
histogram_quantile(0.95,
  sum(rate(efe_catalog_versions_per_name_bucket[1h])) by (le)
)

# Tenants on the latest version vs N-2
count(efe_catalog_versions_per_name)
  - count(efe_catalog_versions_per_name and on(tenant, name)
          (efe_catalog_versions_per_name == on(tenant, name)
            max(efe_catalog_versions_per_name) by (tenant, name)))
```

### Annexe B — LogQL queries de référence

```logql
# Toutes les erreurs auth pour tenant-acme dernières 6h
{tenant_id="tenant-acme", stream="security", level="ERROR"}
  | json
  | event="jwt_invalid"

# Top 10 raisons d'échec auth (rate)
sum by (reason) (rate({stream="security", level="ERROR"}[5m] | json))

# Audit events « remove_class » dernières 24h
{stream="audit"} | json | op_tag="remove_class"

# Logs lents (slow path) — corrélés avec un trace_id
{stream="request", level="WARN"}
  | json
  | duration_ms > 1000
  | line_format "{{.trace_id}} {{.tenant_id}} {{.rpc}}"
```

### Annexe C — Alert template (PrometheusRule)

```yaml
- alert: D13ObsHighErrorRate
  expr: |
    sum(rate(efe_grpc_requests_total{code!="OK"}[5m])) by (tenant)
      / sum(rate(efe_grpc_requests_total[5m])) by (tenant)
      > 0.05
  for: 10m
  labels:
    severity: critical
    family: D13-OBS
  annotations:
    summary: "Error rate > 5 % for tenant {{ $labels.tenant }}"
    description: |
      Tenant {{ $labels.tenant }} sees {{ $value | humanizePercentage }}
      error rate over the last 10 minutes.
    runbook_url: https://docs.efe.ai/runbooks/d13_high_error_rate
    dashboard_url: https://grafana.efe.ai/d/efe-d13-01_operator_overview
```

### Annexe D — Runbook template

```markdown
# d13_high_error_rate — Runbook : High error rate per tenant

> **Severity** : critical
> **Owner** : SRE on-call
> **Updated** : 2026-04-27
> **Related dashboards** : 01_operator_overview, 02_tenant_drill_down

## 1. Alert summary

The error rate for tenant ``$tenant`` exceeded 5 % over the last
10 minutes. Affected RPCs : check the panel "Error rate per RPC"
in dashboard 01.

## 2. First 5 minutes — triage

- [ ] Open dashboard
      ``02_tenant_drill_down?var-tenant=<tenant>``.
- [ ] Identify the dominant error code in the "Error breakdown"
      panel.
- [ ] Check Loki tail for stack traces : pivot via the trace_id
      exemplar.
- [ ] Check upstream dependencies (database, embedder pool) via
      `kubectl get pods -n upstream-* | grep -v Running`.
- [ ] Check rate limit drops : `efe_ratelimit_drops_total{
      tenant="<tenant>"}` — if elevated, the tenant is being
      throttled (legitimate response, not a bug).

## 3. Common root causes

| Cause | Symptom | Fix |
|-------|---------|-----|
| Embedder pool overload | INTERNAL errors, p99 spike | Scale embedder pool replicas |
| JWT misconfig at tenant | UNAUTHENTICATED dominant | Check tenant token config |
| Catalog corruption | INVALID_ARGUMENT spike | `efe-admin catalog get $key` to verify |
| Rate limit too aggressive | RESOURCE_EXHAUSTED | Loosen tenant bucket (cf. d11_per_tenant_isolation) |

## 4. Mitigation (immediate)

- If embedder overloaded : `kubectl scale deploy/embedder
  --replicas=$(($current * 2))`.
- If catalog corruption : roll back to last known-good version
  via `efe-admin catalog versions --name X` then re-pin.

## 5. Resolution (durable)

- File a ticket with the dashboard screenshot.
- Add a regression test covering the failure mode.
- Update this runbook if you found a new root cause.

## 6. Escalation

If unresolved at T+30 min : page tier-2 SRE via Pagerduty.

## 7. Postmortem

Use template `docs/operations/postmortem-template.md`.
```

### Annexe E — Helm test pour multi-tenant isolation

```yaml
# deploy/helm/efe/templates/tests/test-cross-org-isolation.yaml
{{- if and .Values.observability.dashboards.enabled .Values.observability.multiTenancy.enabled }}
apiVersion: v1
kind: Pod
metadata:
  name: {{ include "efe.fullname" . }}-test-cross-org
  annotations:
    "helm.sh/hook": test
spec:
  containers:
  - name: test
    image: curlimages/curl:8.4.0
    command: ['sh', '-c', '
      # As tenant-A, query tenant-B series → expect empty
      n=$(curl -s -H "X-Scope-OrgID: tenant-A" \
          http://mimir:8080/prometheus/api/v1/query \
          --data-urlencode "query={tenant=\"tenant-B\"}" \
          | jq ".data.result | length")
      [ "$n" = "0" ] || (echo "ISOLATION BREACH"; exit 1)
      echo OK'
    ]
{{- end }}
```

### Annexe F — Schéma JSON-Schema des logs (extrait)

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://efe.example.com/schemas/log/v1.0/audit.json",
  "type": "object",
  "required": ["schema_version", "stream", "timestamp",
               "tenant_id", "action", "resource_type",
               "resource_key"],
  "properties": {
    "schema_version": {"const": "1.0"},
    "stream": {"const": "audit"},
    "timestamp": {"type": "string", "format": "date-time"},
    "tenant_id": {"type": "string", "pattern": "^tenant-[a-z0-9-]+$"},
    "action": {"enum": ["register", "evolve", "delete"]},
    "op_tag": {"enum": ["register", "append_class", "remove_class",
                        "update_thresholds", "replace_class_anchors",
                        "recompile", "delete"]},
    "resource_type": {"enum": ["catalog", "cone"]},
    "resource_key": {"type": "string",
                     "pattern": "^[a-z][a-z0-9_-]*/[a-z][a-z0-9_-]*@v[0-9]+$"},
    "parent_key": {"oneOf": [
      {"type": "string", "pattern": "^[a-z][a-z0-9_-]*/[a-z][a-z0-9_-]*@v[0-9]+$"},
      {"type": "null"},
      {"const": ""}
    ]},
    "content_sha256": {"type": "string", "pattern": "^[0-9a-f]{0,64}$"},
    "principal": {"type": "string"},
    "request_id": {"type": "string", "format": "uuid"},
    "trace_id": {"type": "string", "pattern": "^[0-9a-f]{32}$"},
    "span_id": {"type": "string", "pattern": "^[0-9a-f]{16}$"},
    "service_version": {"type": "string"},
    "labels": {"type": "object",
               "additionalProperties": {"type": "string"}}
  },
  "additionalProperties": false
}
```

### Annexe G — Ordre de chargement Helm — résumé

1. ConfigMaps Grafana dashboards (sidecar discovery).
2. PrometheusRule recording + alerting rules.
3. ServiceMonitor (existant, étendu).
4. Optional Tempo / Loki datasource configmaps.
5. Service accounts + RBAC.
6. Helm tests (run after `helm install`).

---

**Fin de l'architecture D13. Date de production : 2026-04-27.
Version : 1.0. Auteur : équipe Engineering EFE.**
