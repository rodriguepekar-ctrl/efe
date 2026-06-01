# Sprint D11 — Architecture : Per-tenant isolation (Phase 2 multi-tenant SaaS)

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : suit D10 (SDK docs + 4 examples publiés sur PyPI +
> docs.efe.ai versionnée via mike). D11 est **le sprint qui
> transforme EFE de "un cluster = un partenaire pilote" (D9/D10)
> en "un cluster = N partenaires avec isolation forte"** — verrou
> critique pour le **Phase 2 milestone : SaaS multi-tenant
> commercial**.
> **Pré-requis** : avoir lu
> [`d9_docker_helm_load_architecture.md`](d9_docker_helm_load_architecture.md)
> (l'amendement A9 a propagé `tenant.id` comme label k8s — le
> fondement de Tier 2), son compagnon
> [`d9_deep_analysis_supplement.md`](d9_deep_analysis_supplement.md)
> §11 (frontière D9↔D11), et
> [`d10_sdk_docs_examples_architecture.md`](d10_sdk_docs_examples_architecture.md)
> (la surface SDK figée sur laquelle D11 ajoute des hooks).

---

## Table des matières

1. [Contexte & mission D11](#1-contexte--mission-d11)
2. [État post-D10 — primitives multi-tenant déjà en place](#2-état-post-d10--primitives-multi-tenant-déjà-en-place)
3. [Modèle multi-tenant à 3 tiers (Soft / Namespace / Cluster)](#3-modèle-multi-tenant-à-3-tiers-soft--namespace--cluster)
4. [Threat model — 12 vecteurs d'attaque cross-tenant](#4-threat-model--12-vecteurs-dattaque-cross-tenant)
5. [Tier 1 — Soft isolation (le default D11)](#5-tier-1--soft-isolation-le-default-d11)
6. [Tier 2 — Namespace isolation (Helm chart variant)](#6-tier-2--namespace-isolation-helm-chart-variant)
7. [Tier 3 — Cluster dedicato (out of scope D11, documenté)](#7-tier-3--cluster-dedicato-out-of-scope-d11-documenté)
8. [Architecture SDK — surface multi-tenant](#8-architecture-sdk--surface-multi-tenant)
9. [Cost attribution — métriques par tenant](#9-cost-attribution--métriques-par-tenant)
10. [Migration path D9/D10 single-tenant → D11 multi-tenant](#10-migration-path-d9d10-single-tenant--d11-multi-tenant)
11. [Cardinalité Prometheus — bornes et stratégie](#11-cardinalité-prometheus--bornes-et-stratégie)
12. [12 décisions de conception clés](#12-12-décisions-de-conception-clés)
13. [10 risques identifiés et mitigations](#13-10-risques-identifiés-et-mitigations)
14. [Plan jour-par-jour (5 j wallclock)](#14-plan-jour-par-jour-5-j-wallclock)
15. [SLOs ajoutés post-D11](#15-slos-ajoutés-post-d11)
16. [Anti-régression & realism check](#16-anti-régression--realism-check)
17. [Intégration CI : workflow `bench-isolation.yml`](#17-intégration-ci--workflow-bench-isolationyml)
18. [Critères d'acceptation (DoD)](#18-critères-dacceptation-dod)
19. [Out of scope & roadmap follow-ups](#19-out-of-scope--roadmap-follow-ups)
20. [Annexes](#20-annexes)

---

## 1. Contexte & mission D11

### 1.1 État post-D10

D10 a livré la couche DX partenaire :

* SDK Python publié sur PyPI (`efe-tool==0.1.10`).
* Documentation Diátaxis sur `docs.efe.ai/0.1.10/`.
* 4 exemples copy-paste-ready sous `examples/`.
* Quickstart end-to-end testé (5 SLOs `D10-*` gated).
* Audit interne : 7 bugs CRITICAL/HIGH fixés, 175 tests
  cross-sprint verts.

Et pourtant, **ship un deuxième partenaire pilote sur le même
cluster est aujourd'hui un risque**. Voici pourquoi :

| Problème | État D10 | Impact si on ship N partenaires |
|----------|----------|---------------------------------|
| Pas de quota CPU/RAM par tenant | partagé librement | un tenant abusif sature le cluster pour les autres |
| Pas de NetworkPolicy ingress par tenant | tout pod du namespace voit le service | tenant A pourrait sniffer le trafic de tenant B |
| Idempotency cache namespacing | `request_id` brut comme clé | collision improbable mais non garantie zero |
| Catalog admin endpoints | tenant-aware mais non testés sous attaque | premier partenaire malveillant → fuite catalog |
| Per-tenant cost attribution | métriques Prometheus labelled mais pas agrégées | impossible de facturer / quotaer par usage |
| ResourceQuota / LimitRange | absents | OOM cascade ou starvation possible |
| Audit log attribution | tenant_id présent mais pas validé invariant | régression silent possible |

Le résultat : **EFE est techniquement multi-tenant capable mais
pas commerciellement multi-tenant ready**.

### 1.2 Mission D11 (énoncé chirurgical)

> 1. **Durcir le tenant scoping serveur** — ajouter un harness de
>    tests d'isolation cross-tenant (12 vecteurs HAZOP, cf. §4) qui
>    garantit que :
>
>    * Un JWT du tenant A ne peut **jamais** lire le catalog du tenant B.
>    * Un cone registry tenant A ne peut **jamais** être muté par
>      le tenant B (même avec scopes `admin:cone:*`).
>    * Le rate limiter d'un tenant ne déborde pas sur un autre.
>    * Les logs d'audit ne fuient pas de PII inter-tenant.
>
>    Pinned par 30+ tests Rust nouveaux (`crates/efe-server/tests/d11_isolation_*.rs`).
>
> 2. **Combler les gaps cache + admin** — namespacer la clé
>    d'idempotency cache (`<tenant_id>:<request_id>`), valider que
>    `AdminService` filtre les listings par tenant, et empêcher
>    qu'un cone register avec un `tenant` différent du JWT
>    réussisse.
>
> 3. **Helm chart Tier 2** — ajouter trois templates opt-in :
>    `templates/resourcequota.yaml`, `templates/limitrange.yaml`,
>    `templates/networkpolicy-tenant.yaml`. Activés via
>    `tenant.namespace.isolated: true`. Permet la stratégie
>    "un namespace par tenant, même chart, même image".
>
> 4. **Observabilité par tenant** — `PrometheusRule` étendue avec
>    alertes per-tenant (high error rate, RSS growth) ;
>    `ServiceMonitor` ajoute `relabel` pour stamper `tenant_id` à
>    partir du namespace. Grafana dashboard JSON livré avec
>    variable `tenant_id` (drill-down). Per-tenant log streams
>    documentées (Loki / Cloud Logging label patterns).
>
> 5. **Cost attribution** — `scripts/billing/aggregate_usage.py`
>    parse les métriques Prometheus sur une fenêtre glissante et
>    émet un CSV/JSON `tenant_id, requests, compute_seconds,
>    egress_bytes`. Source de vérité pour la facturation phase 2.
>
> 6. **SDK surface multi-tenant** — ajout de
>    `EfeClient.list_catalogs() -> list[CatalogRef]` (filtré par
>    tenant via le JWT) et publication d'un module
>    `efe_tool.admin` (sub-package) pour l'admin server-side
>    (register cone, evolve cone). Documentation Diátaxis
>    `howto/multi-tenant.md`.
>
> 7. **Définir 7 SLOs `D11-MT-*`** : 4 isolation + 2 cost + 1
>    cardinalité Prometheus. Gated en CI dès baseline post-merge
>    stable.

### 1.3 Localisation des changements

| Fichier | Modification | Sub-step |
|---------|--------------|----------|
| `crates/efe-server/src/auth/guard.rs` | Validation invariant `JWT.tenant_id == request.tenant_id` | D11.1 |
| `crates/efe-server/src/interceptors/idempotency.rs` | Namespacer clé cache : `<tenant_id>:<request_id>` | D11.1 |
| `crates/efe-server/src/admin.rs` | `list_catalogs` / `list_cones` filter par tenant | D11.1 |
| `crates/efe-server/src/admin.rs` | `RegisterCone` reject si `cone.tenant ≠ caller.tenant_id` | D11.1 |
| `crates/efe-server/tests/d11_isolation_cross_tenant.rs` | **Nouveau** — 12 tests d'attaque HAZOP | D11.1 |
| `crates/efe-server/tests/d11_isolation_rate_limit.rs` | **Nouveau** — bucket disjointness | D11.1 |
| `crates/efe-server/tests/d11_isolation_idempotency.rs` | **Nouveau** — cache namespace | D11.1 |
| `crates/efe-server/tests/d11_isolation_audit.rs` | **Nouveau** — log attribution invariant | D11.1 |
| `deploy/helm/efe/templates/resourcequota.yaml` | **Nouveau** — opt-in `tenant.namespace.resourceQuota.enabled` | D11.2 |
| `deploy/helm/efe/templates/limitrange.yaml` | **Nouveau** — opt-in | D11.2 |
| `deploy/helm/efe/templates/networkpolicy-tenant.yaml` | **Nouveau** — ingress restriction par namespace label | D11.2 |
| `deploy/helm/efe/templates/servicemonitor.yaml` | Étendu — relabeling pour stamp tenant_id depuis namespace | D11.2 |
| `deploy/helm/efe/templates/prometheusrule.yaml` | Étendu — 4 alertes per-tenant | D11.2 |
| `deploy/helm/efe/values.yaml` | Block `tenant.namespace.{resourceQuota,limitRange,networkPolicy}` | D11.2 |
| `deploy/helm/efe/values.tier2.yaml` | **Nouveau** — values overrides pour Tier 2 prod | D11.2 |
| `deploy/observability/grafana/per-tenant-dashboard.json` | **Nouveau** — variable + drill-down | D11.3 |
| `clients/python/efe_tool/__init__.py` | Export `list_catalogs`, `efe_tool.admin` | D11.4 |
| `clients/python/efe_tool/admin.py` | **Nouveau** — `EfeAdminClient` | D11.4 |
| `clients/python/efe_tool/client.py` | Étendu — `EfeClient.list_catalogs()` | D11.4 |
| `docs/docs/howto/multi-tenant.md` | **Nouveau** | D11.4 |
| `docs/docs/explanation/security-model.md` | **Nouveau** — threat model partner-facing | D11.4 |
| `docs/docs/reference/tenant-claims.md` | **Nouveau** — JWT claims contract | D11.4 |
| `scripts/billing/aggregate_usage.py` | **Nouveau** — Prometheus → CSV billing | D11.5 |
| `scripts/billing/tests/test_aggregate_usage.py` | **Nouveau** — golden output tests | D11.5 |
| `scripts/parse_criterion_report.py` | **Modifié** — 7 SLOs `D11-MT-*` | D11.6 |
| `scripts/tests/test_d11_*.py` | **Nouveau** — disjoint keys, surface invariants | D11.6 |
| `.github/workflows/bench-isolation.yml` | **Nouveau** — runs Rust isolation tests under tenant fuzz | D11.6 |
| `bench_baselines/v0.1.11/baseline.json` | **Nouveau** | D11.7 |
| `BENCHMARKS.md § 13` | **Modifié** | D11.7 |
| `docs/audits/d11_isolation_report.md` | **Nouveau** — sprint closure skeleton | D11.7 |
| `CHANGELOG.md` | Entrée Unreleased D11 | D11.7 |

---

## 2. État post-D10 — primitives multi-tenant déjà en place

D11 hérite d'une fondation solide. Inventaire chirurgical :

### 2.1 Auth & identité

| Composant | Fichier | Rôle |
|-----------|---------|------|
| `ClaimsV1` | `crates/efe-server/src/auth/jwt.rs` | JWT carrying `tenant_id`, `scopes`, `sub`, `exp`, `iss`, `aud` |
| Auth guard | `crates/efe-server/src/auth/guard.rs:134` | Extracts `tenant_id` from claims, binds to request |
| Token store | `crates/efe-server/src/auth/token_store.rs` | Multi-key validation (D8 rotation support) |
| Anonymous fallback | `auth/jwt.rs:51` | When `auth.enabled=false`, `tenant_id="anonymous"` (dev only) |

✅ Solid. D11 ajoute UNE invariant guard : `JWT.tenant_id ==
request.tenant_id` (catches manipulated request paths).

### 2.2 Domain layer

| Composant | Fichier | Rôle |
|-----------|---------|------|
| `Request.tenant_id` | `crates/efe-core/src/swarm/request.rs:20` | Tenant on every domain request |
| Cone registry | `crates/efe-core/src/alignment/registry.rs` | Per-(tenant, task) versioning |
| Cone policy | `crates/efe-core/src/alignment/policy.rs:110` | Max versions per (tenant, task) |
| Tracing | `swarm/controller.rs:190` | `tenant_id = %req.tenant_id` in spans |

✅ Solid. D11 ajoute un test cross-tenant fuzz qui vérifie que
**aucun chemin de code** ne lit un cone d'un tenant différent du
caller.

### 2.3 Rate limiting

| Composant | Fichier | Rôle |
|-----------|---------|------|
| `BucketRegistry` | `interceptors/ratelimit.rs` | Per-tenant token-bucket (LRU-bounded) |
| Hot path | `runtime.rs:114` | `Option<Arc<BucketRegistry>>` shared |

✅ Already tenant-keyed. D11 ajoute un test : tenant A à
saturation ne ralentit PAS tenant B (mesure de queue depth side
effect).

### 2.4 Telemetry

| Composant | Fichier | Rôle |
|-----------|---------|------|
| `record_request_completion` | `telemetry/metrics/recorder.rs:27` | `(method, code, tenant)` triple as Prometheus labels |
| Histogram | idem | `efe_grpc_request_duration_seconds{method, tenant}` |
| In-flight gauge | tower layer | NOT tenant-labelled (ok — pre-auth signal) |

✅ Tenant-labelled. D11 ajoute un test :
`assert_metric_label_present("tenant", *)` sur 100 % des familles
métriques d'usage.

### 2.5 Helm chart (D9.A9 + D10 INT1)

| Aspect | État D10 | D11 ajoute |
|--------|----------|-----------|
| `app.kubernetes.io/tenant` label | ✅ propagé via `efe.labels` | invariant pinné (test_d10_helm_label_propagation déjà actif) |
| ResourceQuota | ❌ | template opt-in |
| LimitRange | ❌ | template opt-in |
| NetworkPolicy ingress par tenant | ❌ | template opt-in |
| Per-tenant ServiceMonitor | partiel | relabel pour stamper depuis namespace |
| Per-tenant alertes | ❌ | 4 alertes `EfeTenant*` |

### 2.6 Ce qui n'existe PAS et que D11 livre

* Test harness cross-tenant attack (12 vecteurs).
* Idempotency cache key namespacing.
* Admin endpoints filtering proof.
* Cost attribution aggregator.
* SDK `list_catalogs` filtered.
* Helm Tier 2 templates.
* Grafana dashboard.

---

## 3. Modèle multi-tenant à 3 tiers (Soft / Namespace / Cluster)

Pas un seul modèle multi-tenant — un spectre. D11 documente trois
niveaux d'isolation et leur rapport coût/sécurité.

### 3.1 Tier 1 — Soft isolation (deployment partagé)

```
                ┌────────────────────────┐
                │  k8s namespace `efe`   │
                │                        │
                │  ┌──────────────────┐  │
                │  │ efe-server pod 1 │◀─┼──── tenant A request
                │  │   tenant-aware   │  │
                │  └──────────────────┘  │
                │  ┌──────────────────┐  │
                │  │ efe-server pod 2 │◀─┼──── tenant B request
                │  │   tenant-aware   │  │
                │  └──────────────────┘  │
                │                        │
                │  Prometheus per-tenant │
                │  ServiceMonitor labels │
                └────────────────────────┘
```

* **Isolation** : in-process, JWT + tenant scoping.
* **Cost** : ~$30-100/mois pour ~10 tenants à 1 RPS chacun
  (cf. D9 supplement §7 cost model, divisé par N tenants).
* **Threat model** : confiance dans l'application Rust (pas de
  bug → pas de fuite). Acceptable pour Pilot 1.0 → SaaS limited
  trust.
* **Limites** : un memory leak côté serveur impacte tous les
  tenants ; un OOM-kill termine tout le monde. Pas de quota CPU
  par tenant (un tenant à 100 RPS peut starve les autres).

### 3.2 Tier 2 — Namespace isolation (un Helm release par tenant)

```
   ┌────────────────────┐    ┌────────────────────┐
   │ namespace `tenant-a`│    │ namespace `tenant-b`│
   │  ┌───────────────┐ │    │  ┌───────────────┐ │
   │  │ efe-server pod│ │    │  │ efe-server pod│ │
   │  └───────────────┘ │    │  └───────────────┘ │
   │  ResourceQuota:    │    │  ResourceQuota:    │
   │    cpu=4 mem=8Gi   │    │    cpu=4 mem=8Gi   │
   │  LimitRange        │    │  LimitRange        │
   │  NetworkPolicy:    │    │  NetworkPolicy:    │
   │    ingress = own   │    │    ingress = own   │
   └────────────────────┘    └────────────────────┘
                ↑                         ↑
                │ NetworkPolicy denies cross-namespace
                │
         shared cluster, shared kube-controller-manager
```

* **Isolation** : k8s-level. ResourceQuota empêche le starve
  cross-tenant ; NetworkPolicy empêche les fuites réseau ;
  LimitRange empêche les pods géants.
* **Cost** : ~2× du Tier 1 (chaque tenant paie le minimum 2 pods
  PDB + 1 service + 1 SA + observability surface). Pour 10
  tenants : ~$300-600/mois.
* **Threat model** : confiance NON requise dans l'application Rust
  (k8s isolation porte les garanties).
* **Limites** : N namespaces = N ServiceMonitors,
  N PrometheusRules ; cardinality scale linéairement.

### 3.3 Tier 3 — Cluster dedicato (out of scope D11)

```
    ┌──────────────────┐    ┌──────────────────┐
    │ cluster tenant-a │    │ cluster tenant-b │
    │  full GKE/EKS    │    │  full GKE/EKS    │
    └──────────────────┘    └──────────────────┘
              ↑                       ↑
              │ DNS routing per-tenant │
```

* **Isolation** : maximum (pas de partage de control plane).
* **Cost** : ~5-10× du Tier 1.
* **Quand** : compliance régulée, residency contrainte (FR data,
  EU only), grand compte > $100k ARR/an.
* **Hors-scope D11** : tracé en roadmap D14+ (production cutover)
  ou D17 (multi-region).

### 3.4 Stratégie commerciale par tier

| Tier | Tarif (mensuel) cible | Justification |
|------|----------------------:|---------------|
| 1 (Soft) | $X-base | mutualisation maximale ; SaaS small |
| 2 (Namespace) | $X-base × 2 | isolation k8s ; SaaS mid-market |
| 3 (Cluster) | $X-base × 10 | isolation totale ; enterprise |

D11 ship Tier 1 et 2 ; Tier 3 documenté mais déployé via
infrastructure-as-code séparée (out of scope chart).

---

## 4. Threat model — 12 vecteurs d'attaque cross-tenant

Methodology HAZOP : "no", "more", "less", "reverse", "as well
as", "other than", "early", "late". Pour chaque guideword, un
ou plusieurs scénarios sur la dimension multi-tenant.

| # | Guideword | Scénario | Vecteur exact | Mitigation D11 | Test |
|--:|-----------|----------|---------------|----------------|------|
| 1 | NO | JWT absent | Client envoie un Execute sans Bearer token | Auth guard reject (UNAUTHENTICATED) | `d11_isolation_cross_tenant.rs::no_jwt` |
| 2 | OTHER | JWT signed by attacker | Token avec mauvaise signature | Auth guard reject (UNAUTHENTICATED) | déjà testé via `d8_*` ; carry-over |
| 3 | OTHER THAN | JWT tenant_id ≠ request tenant_id | JWT dit "acme", request body dit "globex" | D11 — guard.rs invariant fail | `d11_isolation_cross_tenant.rs::tenant_mismatch` |
| 4 | AS WELL AS | Catalog list across tenants | JWT acme, AdminService.list_catalogs() | filter par caller.tenant_id | `d11_isolation_cross_tenant.rs::list_filter` |
| 5 | REVERSE | Cone register with foreign tenant | JWT acme, RegisterCone with cone.tenant="globex" | `admin.rs` reject (PERMISSION_DENIED) | `d11_isolation_cross_tenant.rs::register_foreign_cone` |
| 6 | MORE | Tenant A overflows global rate limit | acme à 1000 RPS attendu 10 RPS | per-tenant bucket → globex non impacté | `d11_isolation_rate_limit.rs::cross_tenant_isolation` |
| 7 | LESS | Idempotency cache collision | acme et globex envoient même `request_id` UUIDv5 | namespace clé cache `<tenant_id>:<request_id>` | `d11_isolation_idempotency.rs::namespace_isolation` |
| 8 | EARLY | Audit log emits before tenant binding | log line manque `tenant_id` field | invariant : tracing span MUST carry tenant_id | `d11_isolation_audit.rs::log_attribution` |
| 9 | LATE | Audit log emits after tenant change | one Execute crosses two tenants | tracing span scoped to single request | `d11_isolation_audit.rs::span_scope` |
| 10 | NO control | Anonymous fallback in prod | `auth.enabled=false` shipped à un partenaire | Helm chart `secrets.inline.enabled=false` enforce | manual review checklist |
| 11 | MORE memory | Per-tenant LRU cache leak | 100 000 tenants × 10 KB cache = 1 GB | LRU bound max_tenants=10 000 | `d11_isolation_idempotency.rs::lru_eviction` |
| 12 | OTHER | Metrics cardinality DoS | tenant injects fake claims for cardinality bomb | metrics relabel : drop unknown tenants | `d11_isolation_metrics.rs::cardinality_bound` |

> **Coverage** : 12/12 vecteurs avec tests automatisés. La
> matrice pinned dans
> `docs/audits/d11_isolation_report.md` post-implementation.

---

## 5. Tier 1 — Soft isolation (le default D11)

### 5.1 Auth guard invariant (D11.1.1)

Currently (`auth/guard.rs:134`) :

```rust
let tenant = claims.tenant_id.clone();
// inject into request context
```

D11 adds an explicit cross-check whenever the request carries an
explicit tenant field (e.g. `ConeRef.tenant` in admin RPCs) :

```rust
// crates/efe-server/src/auth/guard.rs (D11 extension)

pub fn assert_tenant_match(
    claims_tenant: &str,
    request_tenant: &str,
) -> Result<(), Status> {
    if claims_tenant != request_tenant {
        // NB: log the mismatch but DO NOT echo the foreign tenant
        // in the error response (information leak).
        tracing::warn!(
            claims_tenant = %claims_tenant,
            request_tenant = "<redacted>",
            "tenant mismatch on signed request",
        );
        return Err(Status::permission_denied(
            "tenant_id in request does not match authenticated principal"
        ));
    }
    Ok(())
}
```

**Where it's called** :
* `admin.rs::register_cone` — caller's `cone.tenant` matches JWT
* `admin.rs::evolve_cone` — caller's `from_key.tenant` matches JWT
* `admin.rs::get_cone`, `delete_cone`, `list_versions` — same
* `swarm.rs::execute` — caller's `spec.alignment_cone_ref.tenant`
  matches JWT

> **Note importante** : `Execute` accepts an inline
> `AlignmentCone` (no tenant field) OR a `ConeRef` (with tenant).
> The check fires only on the `ConeRef` branch ; the inline branch
> is implicitly safe (tenant carries via JWT only).

### 5.2 Idempotency cache namespacing (D11.1.2)

Current cache key : `request_id` (UUIDv5 derived from
`(catalog.version, sha256(text))`).

Risk : two tenants with identical `request_id` (improbable but
possible via UUIDv5 deterministic generation if catalog versions
collide) share a cache entry.

D11 fix : prefix key with `tenant_id` :

```rust
// crates/efe-server/src/interceptors/idempotency.rs (D11 extension)

#[derive(Clone, Hash, Eq, PartialEq)]
pub struct CacheKey {
    pub tenant_id: String,
    pub request_id: String,
}

impl CacheKey {
    pub fn from(tenant_id: &str, request_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            request_id: request_id.to_string(),
        }
    }
}
```

Old code passed `&str` ; new passes `CacheKey`. Migration is
mechanical (~10 call sites). Tests :

* `d11_isolation_idempotency.rs::namespace_isolation` — same
  request_id, two different tenants → two different cache slots.
* `d11_isolation_idempotency.rs::lru_eviction` — LRU per-tenant
  bounded ; one tenant hammering doesn't evict other tenants'
  entries (fairness).

**Memory impact** : `tenant_id` adds ~32 bytes per cache slot
(typical short tenant identifier). With 10 000 cache slots,
overhead ≈ 320 KB. Negligible.

### 5.3 Admin endpoints — list filtering (D11.1.3)

Current `AdminService::list_versions` (cone listing) returns ALL
versions for a `(tenant, task)`. The protobuf already requires
`tenant` in the request. But the implementation MUST verify that
the JWT's `tenant_id` matches the requested `tenant` :

```rust
// crates/efe-server/src/admin.rs (D11 extension)

#[tonic::async_trait]
impl AdminService for AdminServiceImpl {
    async fn list_versions(
        &self,
        request: Request<ListVersionsRequest>,
    ) -> Result<Response<ListVersionsResponse>, Status> {
        let claims = self.auth.extract_claims(&request)?;
        let req = request.into_inner();
        // D11 — sanity check
        assert_tenant_match(&claims.tenant_id, &req.tenant)?;

        let keys = self.registry
            .list_versions(&req.tenant, &req.task)
            .map_err(|e| e.into_status())?;
        Ok(Response::new(ListVersionsResponse { keys }))
    }
    // ... same for register_cone, evolve_cone, get_cone, etc.
}
```

For an SDK-level "give me all my catalogs", D11 introduces a new
RPC `ListCatalogs` (no `tenant` arg in the request — the server
infers from JWT) :

```protobuf
// protos/efe/v1/admin.proto (D11 addition)

service AdminService {
    // ... existing RPCs
    rpc ListCatalogs(ListCatalogsRequest) returns (ListCatalogsResponse);
}

message ListCatalogsRequest {}

message ListCatalogsResponse {
    repeated CatalogRef catalogs = 1;
}

message CatalogRef {
    string tenant = 1;
    string name = 2;
    string version = 3;
    int32 num_classes = 4;
    int32 num_anchors = 5;
}
```

The server fills `tenant = caller.tenant_id` for each entry —
guaranteeing the response cannot leak foreign tenant catalogs.

### 5.4 Audit logger — tenant attribution invariant (D11.1.4)

Currently every `tracing::info!` / `warn!` in the request path
carries `tenant_id` via the span. D11 pins this with a regression
test :

```rust
// crates/efe-server/tests/d11_isolation_audit.rs

#[tokio::test]
async fn every_audit_log_line_carries_tenant_id() {
    let captured = LogCapture::new();
    let server = TestServer::with_log_capture(&captured);
    let _ = server.execute_for_tenant("acme", make_spec()).await;
    let _ = server.execute_for_tenant("globex", make_spec()).await;

    for line in captured.lines() {
        if line.is_request_path() {
            assert!(
                line.has_field("tenant_id"),
                "audit line missing tenant_id: {:?}", line
            );
            // The value must be one of the two tenants we sent —
            // never default "anonymous" or empty.
            let val = line.field_str("tenant_id");
            assert!(
                ["acme", "globex"].contains(&val),
                "unexpected tenant_id value: {:?}", val
            );
        }
    }
}
```

This catches the entire class of "regression where a refactor
strips the tenant span field" — silently.

### 5.5 Per-tenant Prometheus labels — invariant (D11.1.5)

Same idea — assert that 100 % of grpc-business metrics families
carry the `tenant` label :

```rust
// crates/efe-server/tests/d11_isolation_metrics.rs

#[tokio::test]
async fn business_metrics_have_tenant_label() {
    let server = TestServer::start().await;
    let _ = server.execute_for_tenant("acme", make_spec()).await;

    let metrics_text = server.scrape_metrics().await;
    let parsed = prometheus_parse::Scrape::parse(metrics_text.lines());

    let business_families = [
        "efe_grpc_requests_total",
        "efe_grpc_request_duration_seconds",
        "efe_classify_requests_total",
        "efe_classify_compute_seconds_total",
    ];

    for fam in business_families {
        let samples = parsed.samples_for(fam);
        assert!(!samples.is_empty(), "no samples for {}", fam);
        for s in samples {
            assert!(
                s.labels.contains("tenant"),
                "metric {} missing tenant label : {:?}",
                fam, s.labels,
            );
        }
    }
}
```

### 5.6 Cardinality budget for Prometheus (D11.1.6)

A single-tenant deploy has ~30 metric families × ~100 cardinality
points each (le-buckets, methods, codes) = 3 000 series. Adding a
`tenant` label multiplies by N tenants :

* N = 10 → 30 000 series (OK).
* N = 100 → 300 000 series (OK).
* N = 1 000 → 3 000 000 series (heavy ; needs Prometheus
  federation or per-tenant remote write).
* N = 10 000 → 30 M series (NOT OK — Prometheus single-instance
  limit).

D11 documents this scale ceiling and adds a **defensive relabel
rule** in `templates/servicemonitor.yaml` :

```yaml
metricRelabelings:
  # D11 — drop metrics where the tenant label exceeds the
  # cardinality budget. Catches the "tenant cardinality bomb"
  # attack vector (#12 in threat model). The list of approved
  # tenants is operator-managed via .Values.tenant.allowlist.
  - sourceLabels: [tenant]
    regex: "{{ .Values.tenant.allowlist | join \"|\" }}"
    action: keep
```

When `tenant.allowlist` is empty (default), no filtering happens
— Tier 1 trusts the JWT. Tier 2 / regulated deploys SHOULD set
the allowlist.

---

## 6. Tier 2 — Namespace isolation (Helm chart variant)

### 6.1 Activation

```yaml
# values.tier2.yaml — opt-in pour Tier 2
tenant:
  id: "acme"            # the tenant this release belongs to
  namespace:
    isolated: true       # toggle Tier 2 templates
    resourceQuota:
      enabled: true
      cpu: "4"
      memory: "8Gi"
      pods: "10"
    limitRange:
      enabled: true
      defaultCpu: "500m"
      defaultMemory: "512Mi"
      maxCpu: "2"
      maxMemory: "2Gi"
    networkPolicy:
      tenantIngress:
        enabled: true
        # Only namespaces with this label are allowed to call us.
        allowFromNamespaceLabels:
          tenant.efe.ai/owner: "acme"
```

```bash
# Per-tenant install
helm install efe-acme oci://ghcr.io/efe-project/charts/efe \
  --version 0.1.11 \
  --namespace tenant-acme --create-namespace \
  --values values.tier2.yaml \
  --set tenant.id=acme

# Label the namespace so other tenants' netpols can reference it.
kubectl label namespace tenant-acme tenant.efe.ai/owner=acme
```

### 6.2 ResourceQuota template

```yaml
{{- if and .Values.tenant.namespace.isolated .Values.tenant.namespace.resourceQuota.enabled }}
apiVersion: v1
kind: ResourceQuota
metadata:
  name: {{ include "efe.fullname" . }}-quota
  labels: {{- include "efe.labels" . | nindent 4 }}
spec:
  hard:
    requests.cpu:    {{ .Values.tenant.namespace.resourceQuota.cpu | quote }}
    requests.memory: {{ .Values.tenant.namespace.resourceQuota.memory | quote }}
    limits.cpu:      {{ .Values.tenant.namespace.resourceQuota.cpu | quote }}
    limits.memory:   {{ .Values.tenant.namespace.resourceQuota.memory | quote }}
    pods:            {{ .Values.tenant.namespace.resourceQuota.pods | quote }}
    services:        "5"
    persistentvolumeclaims: "0"   # EFE is stateless — block PVCs
{{- end }}
```

### 6.3 LimitRange template

```yaml
{{- if and .Values.tenant.namespace.isolated .Values.tenant.namespace.limitRange.enabled }}
apiVersion: v1
kind: LimitRange
metadata:
  name: {{ include "efe.fullname" . }}-limits
  labels: {{- include "efe.labels" . | nindent 4 }}
spec:
  limits:
    - type: Container
      default:
        cpu:    {{ .Values.tenant.namespace.limitRange.defaultCpu | quote }}
        memory: {{ .Values.tenant.namespace.limitRange.defaultMemory | quote }}
      defaultRequest:
        cpu:    {{ .Values.tenant.namespace.limitRange.defaultCpu | quote }}
        memory: {{ .Values.tenant.namespace.limitRange.defaultMemory | quote }}
      max:
        cpu:    {{ .Values.tenant.namespace.limitRange.maxCpu | quote }}
        memory: {{ .Values.tenant.namespace.limitRange.maxMemory | quote }}
{{- end }}
```

### 6.4 NetworkPolicy template

```yaml
{{- if and .Values.tenant.namespace.isolated .Values.tenant.namespace.networkPolicy.tenantIngress.enabled }}
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: {{ include "efe.fullname" . }}-tenant-ingress
  labels: {{- include "efe.labels" . | nindent 4 }}
spec:
  podSelector:
    matchLabels: {{- include "efe.selectorLabels" . | nindent 6 }}
  policyTypes: ["Ingress"]
  ingress:
    - from:
        # Only namespaces explicitly labelled as our tenant's
        # consumer can reach us. Cross-tenant ingress is blocked
        # at the L3 level — defense-in-depth on top of JWT.
        - namespaceSelector:
            matchLabels:
              {{- range $key, $val := .Values.tenant.namespace.networkPolicy.tenantIngress.allowFromNamespaceLabels }}
              {{ $key }}: {{ $val | quote }}
              {{- end }}
        # Plus the monitoring namespace (Prometheus scrapes /metrics).
        - namespaceSelector:
            matchLabels:
              kubernetes.io/metadata.name: monitoring
      ports:
        - port: {{ .Values.service.grpcPort }}
          protocol: TCP
        - port: {{ .Values.service.metricsPort }}
          protocol: TCP
{{- end }}
```

### 6.5 ServiceMonitor relabel pour stamp tenant_id

```yaml
# templates/servicemonitor.yaml (extension D11)
spec:
  endpoints:
    - port: metrics
      interval: 15s
      relabelings:
        # Stamp tenant_id from the namespace name (Tier 2 convention :
        # namespace = `tenant-<id>`, label = `tenant.efe.ai/owner=<id>`).
        - sourceLabels: [__meta_kubernetes_namespace]
          targetLabel: tenant
          regex: "tenant-(.+)"
          replacement: "$1"
        - sourceLabels: [__meta_kubernetes_namespace_label_tenant_efe_ai_owner]
          targetLabel: tenant
          regex: "(.+)"
          replacement: "$1"
```

The first rule extracts from namespace name (convention) ; the
second from the explicit label. The label takes precedence in
case of mismatch (the namespace creator's intent).

### 6.6 Per-tenant PrometheusRule

```yaml
# templates/prometheusrule.yaml (extension D11 — 4 alertes)
spec:
  groups:
    - name: efe.tenant
      rules:
        # Per-tenant high error rate
        - alert: EfeTenantHighErrorRate
          expr: |
            sum by (tenant) (
              rate(efe_grpc_requests_total{code!="OK"}[5m])
            )
            /
            sum by (tenant) (
              rate(efe_grpc_requests_total[5m])
            )
            > 0.05
          for: 5m
          labels:
            severity: warning
          annotations:
            summary: "Tenant {{`{{ $labels.tenant }}`}} error rate > 5%"

        # Per-tenant rate limit saturation
        - alert: EfeTenantRateLimitSaturated
          expr: |
            rate(efe_ratelimit_rejections_total[5m]) > 1
          for: 10m
          labels:
            severity: info
          annotations:
            summary: "Tenant {{`{{ $labels.tenant }}`}} hitting rate limit"

        # Per-tenant unusual cone register volume (DoS via versioning)
        - alert: EfeTenantConeRegisterStorm
          expr: |
            rate(efe_admin_register_cone_total[1h]) > 100
          for: 30m
          labels:
            severity: warning
          annotations:
            summary: "Tenant {{`{{ $labels.tenant }}`}} > 100 cones/hr"

        # Cardinality bomb detection
        - alert: EfeTenantCardinalityExplosion
          expr: |
            count(count by (tenant) (efe_grpc_requests_total)) > 1000
          for: 10m
          labels:
            severity: critical
          annotations:
            summary: "Tenant cardinality > 1000 — likely attack or misconfig"
```

---

## 7. Tier 3 — Cluster dedicato (out of scope D11, documenté)

Tier 3 reqs :
* Dedicated GKE/EKS cluster per tenant.
* Cluster-level CIS benchmark compliance (kube-bench).
* Per-cluster Helm release (same chart, same image — no fork).
* Per-cluster cosign verification keys (separate trust roots).
* Per-cluster Vault/external-secret instance.

Roadmap : D14 (production cutover for first paid customer) might
ship Tier 3 if regulatory ; D17 (multi-region) certainly.

D11 documents :
* `docs/docs/howto/tier3-cluster-isolation.md` — operator guide
  (placeholder, updated when first Tier 3 deploy lands).

---

## 8. Architecture SDK — surface multi-tenant

### 8.1 New public symbols

```python
# clients/python/efe_tool/__init__.py — D11 additions
__all__ += [
    # Multi-tenant
    "list_catalogs",      # shortcut for `EfeClient.list_catalogs()`
]
```

`efe_tool.admin` becomes a public sub-package :

```python
# clients/python/efe_tool/admin/__init__.py
"""Admin operations — register / evolve cones, list catalogs.

Public surface introduced in v0.1.11 (Sprint D11). Methods on
this client require a JWT with `admin:cone:*` scopes.
"""
from .client import EfeAdminClient

__all__ = ["EfeAdminClient"]
```

```python
# clients/python/efe_tool/admin/client.py
"""High-level admin client wrapping the gRPC AdminService."""
from dataclasses import dataclass


@dataclass(frozen=True)
class CatalogRef:
    tenant: str
    name: str
    version: str
    num_classes: int
    num_anchors: int


class EfeAdminClient:
    def __init__(self, *, endpoint: str, jwt: str, insecure: bool = False):
        ...

    def list_catalogs(self) -> list[CatalogRef]:
        """List catalogs visible to the authenticated tenant.

        The server filters by `caller.tenant_id` ; this method NEVER
        returns catalogs from other tenants.
        """
        ...

    def list_cone_versions(self, tenant: str, task: str) -> list[str]:
        """List cone versions ; `tenant` MUST match the JWT's tenant."""
        ...

    def register_cone(self, ...) -> ConeHandle: ...
    def evolve_cone(self, ...) -> ConeHandle: ...
    def get_cone(self, key: str) -> CanonicalCone: ...
    def delete_cone(self, key: str, *, force: bool = False) -> None: ...
```

### 8.2 New `EfeClient.list_catalogs()` shortcut

```python
# clients/python/efe_tool/client.py — D11 addition
class EfeClient:
    # ... existing methods ...

    def list_catalogs(self) -> list[CatalogRef]:
        """List catalogs visible to my tenant (proxies to AdminService)."""
        from .admin.client import EfeAdminClient
        with EfeAdminClient(
            endpoint=self._endpoint, jwt=self._jwt, insecure=self._insecure,
        ) as admin:
            return admin.list_catalogs()
```

### 8.3 New how-to + reference pages

* `docs/docs/howto/multi-tenant.md` — partner-facing : how to
  use the SDK in a multi-tenant context (use one EfeClient per
  tenant ; never share JWT across tenants).
* `docs/docs/explanation/security-model.md` — threat model
  partner-facing (the 12 vectors at human-readable level).
* `docs/docs/reference/tenant-claims.md` — JWT claims contract
  (`tenant_id`, `scopes`, `aud`, ...).

---

## 9. Cost attribution — métriques par tenant

### 9.1 Required metrics (already exist, just need aggregation)

| Metric | Already labelled? | D11 role |
|--------|:----------------:|----------|
| `efe_grpc_requests_total{tenant}` | ✅ | source of "per-tenant request count" |
| `efe_grpc_request_duration_seconds_sum{tenant}` | ✅ | "per-tenant compute seconds" approximation |
| `efe_classify_compute_seconds_total{tenant}` | NEW | precise classify-only compute |
| `efe_egress_bytes_total{tenant}` | NEW | upload bytes (gRPC response sizes) |

Two new metrics — both implementable in 1 file change (~30 LOC
in `crates/efe-telemetry/src/metrics/recorder.rs`).

### 9.2 Aggregator script

```python
# scripts/billing/aggregate_usage.py — D11 NEW
"""Aggregate per-tenant usage from a Prometheus instance over a
billing period and emit CSV / JSON.

Usage::
    python scripts/billing/aggregate_usage.py \\
        --prom-url http://prometheus.monitoring.svc:9090 \\
        --start 2026-04-01T00:00:00Z \\
        --end   2026-05-01T00:00:00Z \\
        --output target/billing/2026-04.csv
"""
import argparse
import csv
import json
from datetime import datetime
from typing import Any

import urllib.request


def query_range(prom_url: str, query: str, start: str, end: str, step: str = "1h"):
    params = urllib.parse.urlencode({
        "query": query, "start": start, "end": end, "step": step,
    })
    url = f"{prom_url}/api/v1/query_range?{params}"
    with urllib.request.urlopen(url, timeout=30) as resp:
        return json.load(resp)


def aggregate(prom_url: str, start: str, end: str) -> dict[str, dict]:
    """Return {tenant: {requests, compute_seconds, egress_bytes}}."""
    out: dict[str, dict] = {}

    # Total requests per tenant over the window.
    requests_q = 'sum by (tenant) (increase(efe_grpc_requests_total[1h]))'
    r = query_range(prom_url, requests_q, start, end)
    for s in r["data"]["result"]:
        tenant = s["metric"].get("tenant", "_unknown")
        total = sum(float(v) for _, v in s["values"])
        out.setdefault(tenant, {})["requests"] = int(total)

    # Compute seconds per tenant.
    compute_q = 'sum by (tenant) (increase(efe_classify_compute_seconds_total[1h]))'
    r = query_range(prom_url, compute_q, start, end)
    for s in r["data"]["result"]:
        tenant = s["metric"].get("tenant", "_unknown")
        total = sum(float(v) for _, v in s["values"])
        out.setdefault(tenant, {})["compute_seconds"] = round(total, 2)

    # Egress bytes per tenant.
    egress_q = 'sum by (tenant) (increase(efe_egress_bytes_total[1h]))'
    r = query_range(prom_url, egress_q, start, end)
    for s in r["data"]["result"]:
        tenant = s["metric"].get("tenant", "_unknown")
        total = sum(float(v) for _, v in s["values"])
        out.setdefault(tenant, {})["egress_bytes"] = int(total)

    return out


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n", 2)[0])
    p.add_argument("--prom-url", required=True)
    p.add_argument("--start", required=True)
    p.add_argument("--end", required=True)
    p.add_argument("--output", required=True)
    args = p.parse_args()

    usage = aggregate(args.prom_url, args.start, args.end)

    with open(args.output, "w", newline="") as f:
        if args.output.endswith(".csv"):
            w = csv.writer(f)
            w.writerow(["tenant", "requests", "compute_seconds", "egress_bytes"])
            for tenant, m in sorted(usage.items()):
                w.writerow([
                    tenant,
                    m.get("requests", 0),
                    m.get("compute_seconds", 0.0),
                    m.get("egress_bytes", 0),
                ])
        else:
            json.dump(usage, f, indent=2)
    return 0


if __name__ == "__main__":
    main()
```

### 9.3 Tests

`scripts/billing/tests/test_aggregate_usage.py` — golden output
on a fixture Prometheus response (mocked HTTP).

### 9.4 Frequency + storage

* **Daily** — operator runs the aggregator at the end of each
  day, appends to a monthly CSV.
* **Monthly billing close** — last-of-month run, archive to S3
  bucket per tenant.
* **D11 ships the script and docs** ; the cron / scheduler is
  ops-team's responsibility (cf. `docs/operations/billing.md`).

---

## 10. Migration path D9/D10 single-tenant → D11 multi-tenant

### 10.1 Compatibility guarantees

* Existing `tenant.id="default"` deployments continue to work.
* Anonymous fallback (no JWT, no tenant) continues to work for
  dev environments.
* All current SLOs unchanged.
* SDK API surface : additive only (new `list_catalogs()`, new
  `efe_tool.admin` sub-package).

### 10.2 Migration steps for an existing single-tenant deploy

```bash
# Step 1 — verify your deploy is on chart 0.1.11+
helm list -n efe -o json | jq '.[] | .chart'

# Step 2 — your existing values.yaml already has `tenant.id: "default"`
# (D9.A9). No change needed for Tier 1.

# Step 3 (Tier 2 only) — switch to namespace isolation.
# Move out of the shared `efe` namespace into `tenant-default`.
helm uninstall efe -n efe
kubectl create namespace tenant-default
kubectl label namespace tenant-default tenant.efe.ai/owner=default
helm install efe-default oci://ghcr.io/efe-project/charts/efe \
  --version 0.1.11 \
  --namespace tenant-default \
  --values values.tier2.yaml
```

### 10.3 Onboarding a NEW tenant (Tier 1 default flow)

```bash
# Just create a JWT signing key with tenant_id="acme" claim in your IDP.
# No deploy change needed — the same EFE pods serve every tenant.

# Verify the JWT works:
EFE_TOKEN=$(curl -s ...) python examples/01_quickstart_classify.py
```

### 10.4 Onboarding a NEW tenant (Tier 2)

```bash
# Step 1 — create namespace
kubectl create namespace tenant-acme
kubectl label namespace tenant-acme tenant.efe.ai/owner=acme

# Step 2 — install
helm install efe-acme oci://ghcr.io/efe-project/charts/efe \
  --version 0.1.11 \
  --namespace tenant-acme \
  --values values.tier2.yaml \
  --set tenant.id=acme

# Step 3 — provision JWT signing key
# (your IDP: create a key with tenant_id="acme" in claims)

# Step 4 — verify
helm test efe-acme -n tenant-acme
```

### 10.5 Decommission tenant (Tier 2)

```bash
helm uninstall efe-acme -n tenant-acme
kubectl delete namespace tenant-acme   # purges quota, limits, netpol
```

The chart has no PVCs, no persistent state outside the namespace
— uninstall is total. ✓

---

## 11. Cardinalité Prometheus — bornes et stratégie

### 11.1 Math des séries

Chaque famille métrique a un nombre de séries ≈ produit de ses
labels. Pour EFE post-D11 :

```
efe_grpc_requests_total{method, code, tenant}
  = N_methods × N_codes × N_tenants
  = 4       × 5       × N
  = 20 × N

efe_grpc_request_duration_seconds_bucket{method, tenant, le}
  = 4 × N × 12 (le buckets)
  = 48 × N

efe_classify_compute_seconds_total{tenant}
  = N

Sum (D11 added or scaled) ≈ 70 × N + ~3000 (D8/D9/D10 baseline)
```

| N tenants | Series count | Prometheus 1-instance OK? |
|----------:|-------------:|:-------------------------:|
| 10 | 700 + 3000 ≈ 3 700 | ✅ |
| 100 | 7 000 + 3000 ≈ 10 000 | ✅ |
| 1 000 | 70 000 + 3 000 ≈ 73 000 | ✅ |
| 10 000 | 700 000 + 3 000 ≈ 703 000 | ⚠️ near limit (1M default) |
| 100 000 | 7 000 000 ≈ 7 M | ❌ federation required |

### 11.2 Stratégies de borne (D11 livre option 1, document option 2)

**Option 1 — `tenant.allowlist`** (D11 default) : SLO `D11-MT-7`
mesure cardinality, alerte si > 1 000. Operator action : re-set
allowlist or move to Tier 2 (which natively splits cardinality
per Prometheus instance via federation).

**Option 2 — Per-tenant Prometheus federation** (D14+) : multiple
Prometheus instances, one per logical group of ≤ 1 000 tenants.
Documented in `docs/docs/howto/observability-at-scale.md`
(D11 stub).

### 11.3 Cardinality-bomb attack mitigation

Threat #12 : a tenant with admin scope creates a `tenant_id=acme'`
and `tenant_id=acme.` (typo variants) → blows the metrics
cardinality. Mitigation :

* JWT issuer is the source of truth for tenant_id.
* The IDP must validate that `tenant_id` is in a whitelisted set
  (ops-team responsibility, NOT EFE-server).
* Helm chart's `tenant.allowlist` adds a defense-in-depth filter.
* PrometheusRule alert `EfeTenantCardinalityExplosion` fires at
  >1000 distinct values within 10min.

---

## 12. 12 décisions de conception clés

| # | Décision | Alternative rejetée | Justification |
|--:|----------|---------------------|---------------|
| 1 | 3 tiers (Soft/Namespace/Cluster) | un seul modèle | Spectre coût/sécurité, partner peut graduer |
| 2 | Tier 1 default (soft) | Tier 2 default | Mutualisation maximale ; pricing Tier 2 sera 2× |
| 3 | Tenant identity = JWT claim | header / URL path | Auth-coupled, can't be spoofed via path manipulation |
| 4 | `tenant_id` en CHAQUE label métrique | label conditionnel | Predictable cardinality ; un seul allowlist filter |
| 5 | Idempotency key = `<tenant>:<request_id>` | UUID-only | Defense-in-depth contre collision UUIDv5 |
| 6 | RegisterCone reject foreign tenant | accept + log | Hard fail beats audit-only ; 0 leakage prouvée |
| 7 | NetworkPolicy ingress namespace-label-based | IP-based | k8s-native, cluster-portable |
| 8 | ResourceQuota = un seul pour `requests` AND `limits` | split limits | Bursting cap = guaranteed reservation pour la fairness |
| 9 | LimitRange forces defaults | optionnel | Prevent "I forgot to set resources.limits" pod crashing the namespace |
| 10 | Cardinality bomb = alert, pas hard reject | drop offending labels | Operator visibility ; auto-drop hides real attacks |
| 11 | Cost aggregator = Python script (pas service) | dedicated micro-service | Daily cron suffit ; éviter une dep operationnelle |
| 12 | `EfeAdminClient` separate from `EfeClient` | méthodes admin sur client | Surface clean : ne pas mélanger user-flow et admin-flow |

---

## 13. 10 risques identifiés et mitigations

| # | Risque | Sévérité | Probabilité | Mitigation |
|--:|--------|:--------:|:-----------:|------------|
| 1 | Idempotency cache key migration breaks rolling upgrade | HIGH | MED | Dual-key support (read both, write new) for 2 versions, drop in v0.1.13 |
| 2 | Tenant allowlist drift between IDP and Helm | HIGH | MED | Document IDP as source of truth ; Helm allowlist is defense-in-depth only |
| 3 | NetworkPolicy blocks Prometheus scrape | HIGH | LOW | Tier 2 template explicitly allows monitoring namespace |
| 4 | Cost aggregator missing data on Prometheus restart | MED | MED | Use `increase()` not `delta()` ; documented in script docstring |
| 5 | Cone register storm DoS | MED | LOW | Per-tenant rate limit on AdminService.register_cone ; alert at 100/hr |
| 6 | Multi-tenant audit log volume explosion | MED | MED | Loki / log aggregator handles ; documented in observability.md |
| 7 | JWT replay across tenants if IDP misconfigured | HIGH | LOW | Audience claim (`aud=efe.v1`) verified ; document IDP best practices |
| 8 | Tier 2 ResourceQuota too tight for legit traffic | MED | MED | Per-tenant defaults documented ; tunable via values |
| 9 | Cardinality alert fires on legit growth | LOW | HIGH | Threshold tunable ; default 1000 ; ops escalation playbook |
| 10 | Migration from non-tenant deploy breaks for existing partner | HIGH | MED | Compat layer : `tenant_id="default"` works as before ; migration optional |

---

## 14. Plan jour-par-jour (5 j wallclock)

### D11.1 (1.5 j) — Server hardening + tests cross-tenant
* Auth guard `assert_tenant_match` helper.
* `CacheKey` namespacing.
* `AdminService` filter invariants.
* New metric : `efe_classify_compute_seconds_total{tenant}`.
* New metric : `efe_egress_bytes_total{tenant}`.
* 4 test files (`d11_isolation_*.rs`) — 30+ tests.

### D11.2 (1 j) — Helm Tier 2 templates
* `templates/{resourcequota,limitrange,networkpolicy-tenant}.yaml`.
* `templates/servicemonitor.yaml` extension (relabel).
* `templates/prometheusrule.yaml` extension (4 alerts).
* `values.tier2.yaml` example.
* `helm lint` + `helm template` smoke.
* 2 new helm-test pods (cross-tenant access denial).

### D11.3 (½ j) — Observability artefacts
* Grafana dashboard JSON (`deploy/observability/grafana/per-tenant-dashboard.json`).
* Operations doc `docs/operations/multi-tenant-observability.md`.

### D11.4 (½ j) — SDK additions
* `efe_tool.admin.EfeAdminClient`.
* `EfeClient.list_catalogs()`.
* Diátaxis pages : `howto/multi-tenant.md`,
  `explanation/security-model.md`, `reference/tenant-claims.md`.

### D11.5 (½ j) — Cost attribution
* `scripts/billing/aggregate_usage.py`.
* `scripts/billing/tests/test_aggregate_usage.py` (golden tests).
* `docs/operations/billing.md`.

### D11.6 (½ j) — CI integration
* `.github/workflows/bench-isolation.yml` (runs Rust isolation
  tests under tenant fuzz on every PR touching auth/admin/...).
* `parse_criterion_report.py` extension : 7 SLOs `D11-MT-*`.
* `scripts/tests/test_d11_*.py` (disjoint keys, surface
  invariants).

### D11.7 (½ j) — Audit + closure
* `docs/audits/d11_isolation_report.md` skeleton.
* `BENCHMARKS.md § 13`.
* `bench_baselines/v0.1.11/baseline.json`.
* `CHANGELOG.md` Unreleased D11 entry.

---

## 15. SLOs ajoutés post-D11

### 15.1 Liste des 7 nouveaux SLOs

| ID | Métrique | Comparator | Cible | Display | Source |
|----|----------|:----------:|------:|:-------:|--------|
| `D11-MT-1` | `isolation::cross_tenant_leakage` | `lte` | 0 | count | Rust isolation tests count |
| `D11-MT-2` | `isolation::rate_limit_disjoint_pct` | `gte` | 0.99 | ratio | per-tenant bucket fairness measurement |
| `D11-MT-3` | `isolation::audit_log_attribution_pct` | `gte` | 1.0 | ratio | every audit line carries tenant_id |
| `D11-MT-4` | `isolation::metric_label_coverage_pct` | `gte` | 1.0 | ratio | 100 % of business metrics have tenant label |
| `D11-MT-5` | `cost::aggregator_drift_pct` | `lte` | 0.01 | ratio | aggregator output ≈ Prometheus snapshot (1% drift max) |
| `D11-MT-6` | `cost::usage_export_latency_s` | `lte` | 60 | s | aggregator runtime on a 30-day window |
| `D11-MT-7` | `obs::tenant_cardinality_count` | `lte` | 1000 | count | Prometheus distinct tenant values |

### 15.2 Format dans `bench_baselines/v0.1.11/baseline.json`

```json
{
  "version": "1.0",
  "timestamp": "PENDING_FIRST_GREEN_RUN",
  "results": {},
  "slo_compliance": {
    "SLO-D11-MT-1": {
      "name": "Cross-tenant data leakage count",
      "target_ns": 0,
      "measured_ns": null,
      "passed": null,
      "skipped": true,
      "comparator": "lte",
      "display_as": "duration"
    },
    "SLO-D11-MT-2": {
      "name": "Rate limit disjointness ratio (per-tenant fairness)",
      "target_ns": 990000000,
      "comparator": "gte",
      "display_as": "ratio",
      "skipped": true
    },
    "SLO-D11-MT-3": {
      "name": "Audit log tenant_id attribution coverage",
      "target_ns": 1000000000,
      "comparator": "gte",
      "display_as": "ratio",
      "skipped": true
    },
    "SLO-D11-MT-4": {
      "name": "Business metrics tenant label coverage",
      "target_ns": 1000000000,
      "comparator": "gte",
      "display_as": "ratio",
      "skipped": true
    },
    "SLO-D11-MT-5": {
      "name": "Cost aggregator drift vs Prometheus",
      "target_ns": 10000000,
      "comparator": "lte",
      "display_as": "ratio",
      "skipped": true
    },
    "SLO-D11-MT-6": {
      "name": "Cost aggregator export wallclock (30d window)",
      "target_ns": 60000000000,
      "comparator": "lte",
      "display_as": "ratio",
      "skipped": true
    },
    "SLO-D11-MT-7": {
      "name": "Tenant cardinality (distinct tenant label values)",
      "target_ns": 1000,
      "comparator": "lte",
      "display_as": "duration",
      "skipped": true
    }
  }
}
```

### 15.3 Disjoint-key invariant (étendu)

Nouveaux préfixes ajoutés à `RESERVED_PREFIXES` :

```python
RESERVED_PREFIXES = (
    "eval_accuracy::",
    "eval_latency::",
    "load_",
    "soak::",
    "docs::",
    "dx::",
    "isolation::",   # D11 NEW
    "cost::",        # D11 NEW
    "obs::",         # D11 NEW (cardinality observability)
)
```

Test `scripts/tests/test_d11_disjoint_keys.py` extends the
existing `test_d10_disjoint_keys.py` family with the same shape.

---

## 16. Anti-régression & realism check

### 16.1 Rust workspace tests

```bash
cargo test --workspace
# Plus the new D11 isolation harness:
cargo test --package efe-server --test 'd11_*'
```

D11 ajoute ~30 tests Rust (4 files × ~7 tests each).

### 16.2 Helm chart tests

```bash
helm lint deploy/helm/efe
helm template deploy/helm/efe -f values.tier2.yaml | yq .   # 16 manifests
helm install efe-test deploy/helm/efe -f values.tier2.yaml --dry-run
```

### 16.3 Python tests

```bash
pytest scripts/billing/tests/
pytest scripts/tests/test_d11_*.py
pytest examples/tests/test_examples_smoke.py
```

D11 ajoute ~12 tests Python.

### 16.4 Realism check sur un cluster minikube

```bash
# Tier 1
helm install efe oci://...  --version 0.1.11 -n efe --set replicaCount=2
# verify single-tenant flow still works
python examples/01_quickstart_classify.py

# Tier 2 — install for tenant-a
kubectl create namespace tenant-a
kubectl label namespace tenant-a tenant.efe.ai/owner=a
helm install efe-a oci://...  --version 0.1.11 \
    -n tenant-a --values values.tier2.yaml --set tenant.id=a
# verify tenant-a flow
EFE_JWT=$(jwt-sign tenant=a) python examples/01_quickstart_classify.py

# Adversarial check : tenant-a JWT against tenant-b deployment
# should NOT classify (UNAUTHENTICATED).
```

---

## 17. Intégration CI : workflow `bench-isolation.yml`

```yaml
name: bench-isolation

# Sprint D11 — runs the cross-tenant attack harness on every PR
# touching auth, admin, or interceptors. Catches regressions in
# the isolation guarantees that single-tenant smoke would miss.

on:
  push:
    branches: [main]
    paths:
      - "crates/efe-server/src/auth/**"
      - "crates/efe-server/src/admin.rs"
      - "crates/efe-server/src/interceptors/**"
      - "crates/efe-server/tests/d11_*.rs"
      - "deploy/helm/efe/templates/{networkpolicy*,resourcequota,limitrange}.yaml"
      - ".github/workflows/bench-isolation.yml"
  pull_request:
    paths:
      - "crates/efe-server/src/auth/**"
      - "crates/efe-server/src/admin.rs"
      - "crates/efe-server/src/interceptors/**"
      - "crates/efe-server/tests/d11_*.rs"
      - "deploy/helm/efe/templates/{networkpolicy*,resourcequota,limitrange}.yaml"
  workflow_dispatch:

permissions:
  contents: read

jobs:
  isolation-tests:
    runs-on: ubuntu-22.04
    timeout-minutes: 20
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { toolchain: "1.85.0" }
      - uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target
          key: ${{ runner.os }}-cargo-isolation-${{ hashFiles('Cargo.lock') }}
      - run: |
          cargo test --workspace --release --test 'd11_*' \
              -- --test-threads 1
        # `--test-threads 1` so the rate-limit-isolation test
        # gets predictable timing.
        timeout-minutes: 15

  helm-multi-tenant-render:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - uses: azure/setup-helm@v4
      - run: helm lint deploy/helm/efe
      - run: |
          # Render Tier 2 with two synthetic tenants and verify
          # the rendered manifests have non-overlapping selectors.
          for tenant in acme globex; do
            helm template "efe-$tenant" deploy/helm/efe \
              --namespace "tenant-$tenant" \
              --values deploy/helm/efe/values.tier2.yaml \
              --set tenant.id="$tenant" \
              > "/tmp/render-$tenant.yaml"
          done
          # Run the cross-tenant overlap detector
          python3 scripts/tests/d11_helm_render_isolation.py \
              /tmp/render-acme.yaml /tmp/render-globex.yaml
```

---

## 18. Critères d'acceptation (DoD)

### 18.1 Code
- [ ] `assert_tenant_match` helper + 4 call sites updated.
- [ ] `CacheKey` struct + idempotency interceptor migrated.
- [ ] `AdminService::list_catalogs` RPC + filter by JWT tenant.
- [ ] 4 new Rust isolation test files — 30+ tests passing.
- [ ] 2 new Prometheus metrics : compute_seconds, egress_bytes.

### 18.2 Helm
- [ ] 3 new opt-in templates (resourcequota, limitrange,
      networkpolicy-tenant).
- [ ] ServiceMonitor relabel + PrometheusRule extension.
- [ ] `values.tier2.yaml` example.
- [ ] `helm lint` + `helm template` clean.
- [ ] 2 new helm-test pods (cross-tenant denial).

### 18.3 SDK
- [ ] `efe_tool.admin` sub-package published.
- [ ] `EfeClient.list_catalogs()` shortcut.
- [ ] `__all__` extended ; surface freeze tests updated.

### 18.4 Observability
- [ ] Grafana dashboard JSON committed.
- [ ] Operations doc `multi-tenant-observability.md`.

### 18.5 Cost attribution
- [ ] `aggregate_usage.py` + tests.
- [ ] `docs/operations/billing.md`.

### 18.6 Documentation
- [ ] `docs/docs/howto/multi-tenant.md`.
- [ ] `docs/docs/explanation/security-model.md`.
- [ ] `docs/docs/reference/tenant-claims.md`.
- [ ] `BENCHMARKS § 13`.

### 18.7 CI
- [ ] `.github/workflows/bench-isolation.yml` actif sur main + PR.
- [ ] 7 SLOs `D11-MT-*` dans `parse_criterion_report.py`.
- [ ] `bench_baselines/v0.1.11/baseline.json` (placeholder
      SKIPPED).

### 18.8 Anti-régression
- [ ] Aucun des 26 SLOs antérieurs régressé.
- [ ] `cargo test --workspace` vert.
- [ ] 12 vecteurs HAZOP couverts par tests automatisés.

---

## 19. Out of scope & roadmap follow-ups

### 19.1 Hors-scope D11

- ❌ **Tier 3 (cluster dedicato)** — D14+ if regulatory.
- ❌ **Per-tenant TLS (mTLS)** — D14+.
- ❌ **Per-tenant cosign verification keys** — D14+.
- ❌ **OAuth2 / OIDC IDP integration** — D15 (Phase 3 SaaS).
- ❌ **Per-tenant catalog versioning UI** — D12 (admin
  endpoints already RPC-callable).
- ❌ **Multi-region per-tenant** — D17.
- ❌ **GDPR data residency hooks** — D17.
- ❌ **Per-tenant usage-based pricing API** — Phase 3 / billing
  integration (out of EFE engineering scope).

### 19.2 Roadmap après D11

| Sprint | Cible | Justification |
|--------|-------|---------------|
| **D11.bis** | Reviewer feedback resolution | First multi-tenant partner pilot |
| **D12** | Catalog admin endpoints UI + hot-reload | UX for the 5+ catalog tenant |
| **D13** | Grafana dashboards (full SRE pack) | Phase 2 production observability |
| **D14** | Production cutover (premier client payant) | Pilot → paid |
| **D15** | OAuth2 / SaaS-grade IDP | Phase 3 begins |

### 19.3 Métrique de succès post-D11

À 30 jours après merge D11 :

> *« 2+ partenaires pilotes coexistent sur le même cluster Tier 1
> avec tenant_id distincts. Aucun ticket support sur "je vois les
> données de l'autre". Le cost aggregator produit un rapport
> mensuel par tenant cohérent à ±1 %. Les 7 SLOs `D11-MT-*` sont
> verts depuis ≥ 3 weeks. »*

---

## 20. Annexes

### 20.1 Annexe A — Full JWT claims contract (D11)

```json
{
  "iss": "https://idp.your-corp.com/",
  "aud": "efe.v1",
  "sub": "user-or-service-account-id",
  "exp": 1714137600,
  "iat": 1714134000,
  "tenant_id": "acme",
  "scopes": ["classify:read", "classify:write", "admin:cone:read"]
}
```

* `tenant_id` is **required** for every authenticated request.
* `scopes` controls which RPCs the principal can invoke. D11
  scopes :
  * `classify:read` — `Execute`, `ExecuteStreaming`,
    `DescribeSpec`.
  * `classify:write` — currently same as `read` (reserved for
    future per-tenant catalog write).
  * `admin:cone:read` — `GetCone`, `ListVersions`, `ListCatalogs`.
  * `admin:cone:write` — `RegisterCone`, `EvolveCone`,
    `DeleteCone`, `BreachHistory`.

### 20.2 Annexe B — `values.tier2.yaml` complet

```yaml
# values.tier2.yaml — production Tier 2 multi-tenant template.
#
# Use this with:
#   helm install efe-<tenant> oci://...  --version 0.1.11 \
#     -n tenant-<tenant> -f values.tier2.yaml \
#     --set tenant.id=<tenant>

replicaCount: 2
service:
  headless: true
autoscaling:
  enabled: true
  minReplicas: 2
  maxReplicas: 8

# Image — pin by digest in real production.
image:
  repository: ghcr.io/efe-project/efe
  tag: "0.1.11"

# Auth required — Tier 2 never disables auth.
config:
  auth:
    enabled: true
    jwtAudience: "efe.v1"
    secretsFile: "/etc/efe/secrets/hs256"

secrets:
  externalSecretName: efe-jwt-prod   # managed by ESO

# D11 — Tier 2 isolation block.
tenant:
  # `id` is overridden via `--set tenant.id=<...>` at install.
  id: "must-override"
  namespace:
    isolated: true
    resourceQuota:
      enabled: true
      cpu: "4"
      memory: "8Gi"
      pods: "10"
    limitRange:
      enabled: true
      defaultCpu: "500m"
      defaultMemory: "512Mi"
      maxCpu: "2"
      maxMemory: "2Gi"
    networkPolicy:
      tenantIngress:
        enabled: true
        allowFromNamespaceLabels:
          # Defaults — override per tenant.
          tenant.efe.ai/owner: "must-override"

# Observability — opt-in cardinality allowlist.
serviceMonitor:
  enabled: true
prometheusRule:
  enabled: true

# Optional cardinality safeguard.
tenant:
  allowlist: []   # empty = no metric drop ; populate for high-trust deploys
```

### 20.3 Annexe C — Onboarding partner cheat-sheet

```
# 1. Validate the partner is on chart 0.1.11+
helm list -A | grep efe

# 2. Decide tier based on sensitivity:
#    - Tier 1 (shared) → fast onboard, $X/mo
#    - Tier 2 (namespace) → moderate isolation, $2X/mo
#    - Tier 3 (cluster) → max isolation, $10X/mo

# 3. For Tier 2, follow §10.4. For Tier 1:
#    - Just generate a JWT with tenant_id=<partner>.
#    - Test: EFE_JWT=$(idp-token) python examples/01_quickstart_classify.py

# 4. Add the partner to:
#    - Operations doc roster.
#    - Cost aggregator allowlist.
#    - PagerDuty service if enterprise.

# 5. Verify isolation:
#    - Try tenant_a JWT against tenant_b's RPC paths → must 401/403.
#    - Confirm tenant_a metrics do NOT appear under tenant_b in Grafana.
```

### 20.4 Annexe D — Glossary

| Term | Definition |
|------|------------|
| **Tenant** | Logical owner of a stream of EFE traffic. JWT claim ; not a k8s primitive. |
| **Tier 1** | Soft isolation in shared deployment (default D11). |
| **Tier 2** | Namespace isolation (one Helm release per tenant). |
| **Tier 3** | Cluster isolation (one cluster per tenant ; out of scope D11). |
| **Tenant claim** | The `tenant_id` field of a JWT issued by the IDP. |
| **Allowlist** | Set of `tenant_id` values the cluster expects to see. |
| **Cardinality bomb** | Attack where a tenant injects many distinct labels to OOM Prometheus. |
| **Cross-tenant leakage** | Any path where tenant A's data is accessible from tenant B's principal. |

### 20.5 Annexe E — Checklist d'exécution architect → impl

```
□ D11.1.1 auth/guard.rs::assert_tenant_match + 4 call sites
□ D11.1.2 CacheKey struct + idempotency.rs migration
□ D11.1.3 admin.rs::list_catalogs + filter
□ D11.1.4 metrics : compute_seconds_total, egress_bytes_total
□ D11.1.5 d11_isolation_cross_tenant.rs (12 tests)
□ D11.1.6 d11_isolation_rate_limit.rs (3 tests)
□ D11.1.7 d11_isolation_idempotency.rs (5 tests)
□ D11.1.8 d11_isolation_audit.rs (4 tests)
□ D11.1.9 d11_isolation_metrics.rs (3 tests)

□ D11.2.1 templates/resourcequota.yaml
□ D11.2.2 templates/limitrange.yaml
□ D11.2.3 templates/networkpolicy-tenant.yaml
□ D11.2.4 templates/servicemonitor.yaml extension
□ D11.2.5 templates/prometheusrule.yaml extension (4 alerts)
□ D11.2.6 values.yaml block + values.tier2.yaml
□ D11.2.7 templates/tests/test-cross-tenant-denial.yaml
□ D11.2.8 helm lint + render Tier 2 dual-tenant smoke

□ D11.3.1 deploy/observability/grafana/per-tenant-dashboard.json
□ D11.3.2 docs/operations/multi-tenant-observability.md

□ D11.4.1 efe_tool/admin/__init__.py + client.py
□ D11.4.2 EfeClient.list_catalogs() shortcut
□ D11.4.3 howto/multi-tenant.md
□ D11.4.4 explanation/security-model.md
□ D11.4.5 reference/tenant-claims.md
□ D11.4.6 surface_pinned test extension

□ D11.5.1 scripts/billing/aggregate_usage.py
□ D11.5.2 scripts/billing/tests/test_aggregate_usage.py
□ D11.5.3 docs/operations/billing.md

□ D11.6.1 .github/workflows/bench-isolation.yml
□ D11.6.2 parse_criterion_report.py — 7 D11 SLOs
□ D11.6.3 scripts/tests/test_d11_disjoint_keys.py
□ D11.6.4 scripts/tests/test_d11_surface_extensions.py

□ D11.7.1 bench_baselines/v0.1.11/baseline.json
□ D11.7.2 BENCHMARKS.md § 13
□ D11.7.3 docs/audits/d11_isolation_report.md skeleton
□ D11.7.4 CHANGELOG.md Unreleased D11

□ Validation finale
  □ helm install Tier 2 dual-tenant on minikube
  □ Cross-tenant attack harness 30+ tests vert
  □ Cost aggregator dry-run on synthetic Prometheus
  □ All previous SLOs (26 inherited) green
  □ docs.efe.ai/0.1.11/ accessible (post-tag)
```

### 20.6 Annexe F — Cardinality budget worked example

```
Cluster: 50 tenants
Metrics:
  efe_grpc_requests_total{method, code, tenant}
    methods: 4 (Execute, ExecuteStreaming, DescribeSpec, Health/Check)
    codes: 5 (OK, UNAUTHENTICATED, RESOURCE_EXHAUSTED, INVALID_ARGUMENT, INTERNAL)
    tenants: 50
    series = 4 × 5 × 50 = 1 000

  efe_grpc_request_duration_seconds_bucket{method, tenant, le}
    le buckets: 12
    methods: 4
    tenants: 50
    series = 4 × 50 × 12 = 2 400

  efe_classify_compute_seconds_total{tenant}
    series = 50

  efe_egress_bytes_total{tenant}
    series = 50

Total D11 added: 3 500 series for 50 tenants.

Pre-D11 (D8/D9/D10): ~3 000 series (no tenant labels on the
existing families that are tenant-scoped).

Total post-D11 with 50 tenants: ~6 500 series.

Prometheus 1-instance default budget: 1M series.
Margin: 153× — comfortable.

Operator alert threshold (D11-MT-7): 1000 distinct tenants.
At 1000 tenants × ~70 series/tenant = 70k series + baseline = 73k.
Still 14× margin — alert is conservative.
```

### 20.7 Annexe G — Frontière D11 ↔ D12+

D11 ships :
* All multi-tenant primitives.
* Tier 1 + Tier 2 in production.
* SLOs gated.

D11 does NOT ship :
* **Catalog hot-reload** — tracked for D12.
* **Per-tenant audit log signing** — tracked for D14.
* **Tier 3 cluster isolation** — tracked for D14+.
* **OAuth2 / OIDC** — tracked for D15.
* **Multi-region** — tracked for D17.

D11 hooks for D12+ :
* AdminService RPCs are RPC-callable ; D12 builds a UI on top.
* `efe_tool.admin.EfeAdminClient` is the canonical SDK ; D12
  scripts can use it.
* All metrics are tenant-labelled ; D13 dashboards can group
  freely.
* `tenant_id` claim contract is documented ; D15 IDPs (OAuth)
  must respect it.

---

**Fin de l'architecture D11.** Le sprinter peut implémenter en
suivant Annexe E, en revenant à §5 pour Tier 1, §6 pour Tier 2,
§4 pour le threat model.

> **Approbation requise avant implémentation** :
> - [ ] Lead sécu (revue threat model + isolation invariants)
> - [ ] Lead infra (revue Helm Tier 2 templates)
> - [ ] Owner produit (revue pricing tier strategy)
> - [ ] Lead dev (revue SDK admin sub-package)
