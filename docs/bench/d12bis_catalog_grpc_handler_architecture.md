# Sprint D12-bis — Architecture : `CatalogService` gRPC handler (translation layer)

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : ferme la dette critique D12 (« in-memory registry
> existe mais aucun handler gRPC ne l'expose ») identifiée par
> l'analyse MVP comme blocker #1. Sans D12-bis, le SDK
> `EfeAdminClient.register_catalog` retourne `Unavailable`, le CLI
> `efe-admin catalog register` échoue, les 5 dashboards D13 qui
> dépendent de `efe_catalog_*` sont vides, et 5 alertes D13 sur 10
> ne tirent jamais.
>
> **Périmètre** : un translation layer **mince** entre :
>
>   * la couche **wire** : 6 RPCs proto déjà définis dans
>     `protos/efe/v1/catalog.proto` (D12) ;
>   * la couche **domaine** : trait `CatalogRegistry` + impl
>     `InMemoryRegistry` (D12) + audit + policy ;
>   * la couche **observabilité** : 8 instruments + 16 spans +
>     5 streams typés (D13).
>
> **Hors-périmètre** : aucun nouveau modèle de données, aucun
> nouvel RPC, aucun nouveau SLO. D12-bis **active** ce qui existe.
>
> **Pré-requis** : avoir lu
> [`d12_catalog_admin_versioning_architecture.md`](d12_catalog_admin_versioning_architecture.md)
> (le trait CatalogRegistry, les 5 evolve ops, les quotas),
> [`d12_catalog_audit_report.md`](../audits/d12_catalog_report.md)
> + [`d12_catalog_audit_report.md`](../audits/d12_catalog_audit_report.md)
> (audit fixes C-1 cascade + H-4 principal threading),
> [`d13_observability_dashboards_architecture.md`](d13_observability_dashboards_architecture.md)
> § 5.2 (8 instruments) + § 7.2 (16 span constants) + § 6 (5 log
> streams), et
> [`d13_observability_audit_report.md`](../audits/d13_observability_audit_report.md)
> (audit fix C-1 `principal_present` label + M-1 init wiring).

---

## Table des matières

1. [Contexte & mission D12-bis](#1-contexte--mission-d12-bis)
2. [État des lieux post-D13 — pré-conditions remplies](#2-état-des-lieux-post-d13)
3. [Pré-condition #0 — régénération des stubs Rust tonic](#3-pré-condition-0)
4. [Architecture du translation layer](#4-architecture-du-translation-layer)
5. [Auth context — extraction du JWT](#5-auth-context)
6. [RBAC : scope checks par RPC + tenant assertion](#6-rbac-scope-checks)
7. [Conversions proto ↔ trait](#7-conversions-proto-trait)
8. [Error mapping `CatalogError` → `tonic::Status`](#8-error-mapping)
9. [Observability emission par RPC](#9-observability-emission)
10. [Concurrency model — le handler fait confiance au trait](#10-concurrency-model)
11. [Configuration + `ServerConfig` knobs](#11-configuration)
12. [Wire dans `runtime::Server`](#12-wire-dans-runtime-server)
13. [Tests stratégie — unit + intégration + concurrence + isolation](#13-tests-stratégie)
14. [Extension du workflow `bench-catalog.yml`](#14-extension-du-workflow)
15. [Helm extensions — `catalog.backend` + `catalog.policy.*`](#15-helm-extensions)
16. [Migration prep D14 — postgres-ready par construction](#16-migration-prep-d14)
17. [Threat model — 10 vecteurs handler-spécifiques](#17-threat-model)
18. [12 décisions de conception clés](#18-12-décisions-de-conception-clés)
19. [10 risques identifiés et mitigations](#19-10-risques-identifiés)
20. [Plan jour-par-jour (5 j wallclock)](#20-plan-jour-par-jour)
21. [SLOs activés — D12-CT-1..7 passent de SKIPPED à measured](#21-slos-activés)
22. [Anti-régression D12-bis](#22-anti-régression)
23. [Critères d'acceptation (DoD)](#23-critères-dacceptation)
24. [Out of scope & roadmap D14+](#24-out-of-scope)
25. [Annexes — code skeletons, ServerConfig YAML, tonic test patterns](#25-annexes)

---

## 1. Contexte & mission D12-bis

### 1.1 État post-D13

Au sortir des sprints D11 → D13, EFE a accumulé un **excellent socle
non-câblé** :

* **Proto** (D12) : `protos/efe/v1/catalog.proto` définit
  `CatalogService` avec 6 RPCs et 25 messages. Les stubs Python
  sont générés et utilisés par le SDK + CLI ; **les stubs Rust ne
  sont PAS générés** (`build.rs` ne liste pas le fichier).
* **Trait domaine** (D12) :
  `crates/efe-server/src/catalog/registry.rs` expose le trait
  async `CatalogRegistry`. L'impl `InMemoryRegistry` est testée
  (29 tests post audit-fix C-1 cascade + H-4 principal). Le
  trait est stable.
* **Policy + audit** (D12) : `policy.rs` (quotas + validation
  schéma + 5 evolve ops) et `audit.rs` (`AuditEmitter` qui
  émet via `tracing::info!(target: "audit", ...)`) sont câblés
  à l'impl in-memory.
* **Observabilité** (D13) :
  * 8 instruments Prom déclarés dans `d13_instruments.rs` et
    enregistrés au boot via `register_all_d13()` (D13 M-1 fix).
  * `efe_audit_events_total` a le label `principal_present`
    (D13 C-1 fix).
  * 16 constantes de span dans `spans.rs`.
  * 5 builders typés JSON dans `events::d13_streams` qui
    produisent des `serde_json::Value` validant contre les
    JSON-Schemas.
* **SDK + CLI** (D12) : `EfeAdminClient.register_catalog` et
  consorts sont implémentés et testés contre les stubs Python
  proto. Les 6 verbes CLI (`register`/`evolve`/`list`/`versions`/
  `get`/`delete`) parsent et résolvent leurs arguments.
* **Dashboards + alerts** (D13) : 6 dashboards Grafana et 10
  alertes Prom référencent les métriques `efe_catalog_*` et
  `efe_audit_events_total`.

### 1.2 Le problème opérationnel

**Tout ce qui précède est dormant**. Aucun handler gRPC tonic
n'implémente `CatalogService`. Conséquences en cascade :

| Surface | Symptôme observable côté client |
|---|---|
| SDK `EfeAdminClient.register_catalog` | `EfeUnavailableError: Unimplemented` (le service n'est pas exposé par le serveur) |
| CLI `efe-admin catalog register` | Idem (l'erreur SDK remonte) |
| Example `examples/05_catalog_admin.py` | Échoue au step [1/8] |
| Dashboards 03 (catalog versions), 05 (audit) | "no data" sur tous les panels qui dépendent de `efe_catalog_*` |
| Alertes `D13ObsClassifyDriftHigh`, `D13ObsCatalogVersionExplosion`, `D13ObsAuditPrincipalUnknown`, `D13ObsHighErrorRate` (sur catalog RPCs) | Ne firent jamais (recording rules vides) |
| SLOs `D12-CT-1..7` (7 SLOs) | SKIPPED dans `bench_baselines/v0.1.12/baseline.json` (gating sur le 1er run vert qui ne peut pas exister) |

L'ironie : les Python anti-régression tests (~ 240 tests) **passent
tous**, parce qu'ils valident la surface SDK + CLI + schéma proto
+ artefacts statiques, pas l'exécution end-to-end via gRPC.

### 1.3 Mission D12-bis (énoncé chirurgical)

> 1. **Régénérer les stubs Rust tonic** pour `catalog.proto` en
>    ajoutant le fichier à `crates/efe-server/build.rs`. Vérifier
>    que `crate::proto::v1::catalog` (et le sous-module
>    `catalog_service_server`) est disponible.
>
> 2. **Implémenter `CatalogServiceImpl`** dans
>    `crates/efe-server/src/services/catalog.rs` — un struct
>    `CatalogServiceImpl` qui détient un
>    `Arc<dyn CatalogRegistry>` + `Arc<AuditEmitter>` +
>    `Arc<CatalogPolicy>`, et qui implémente le trait
>    `catalog_service_server::CatalogService` généré par tonic
>    pour les 6 RPCs.
>
> 3. **Threading l'auth context** : extraire le `principal`
>    (`sub` claim), le `tenant_id`, et la liste de scopes depuis
>    les extensions de la `tonic::Request<T>` (l'interceptor JWT
>    M9/D11 les y a placés). Pour chaque RPC :
>    * Vérifier le scope requis (`admin:catalog:read` ou
>      `:write` selon le verbe).
>    * Vérifier `assert_tenant_match` (le tenant claimed dans le
>      payload doit matcher le tenant du JWT) — D11 invariant.
>    * Threader le `principal` dans les appels au trait
>      (notamment `delete(key, force, principal)` per audit fix
>      D12 H-4).
>
> 4. **Convertir proto ↔ trait** : 6 helpers de conversion
>    (mirror de ceux du SDK Python) dans
>    `services/catalog/conversions.rs`. Validation à la traversée
>    pour rejeter les payloads malformés avant qu'ils n'atteignent
>    la trait.
>
> 5. **Émettre l'observabilité D13** : pour chaque RPC, ouvrir un
>    span (`spans::CATALOG_REGISTER` etc.), observer son
>    histogramme (`catalog_register_duration_seconds`),
>    incrémenter `audit_events_total{tenant, action, op_tag,
>    principal_present}` (avec la valeur `principal_present`
>    correctement set selon le contexte JWT), et émettre l'event
>    structuré via `events::d13_streams::AuditEvent` ou
>    `EvolutionEvent` selon le verbe.
>
> 6. **Mapper `CatalogError` → `tonic::Status`** : 6 variantes →
>    6 codes gRPC selon le tableau (§ 8). Sanitisation des
>    erreurs internes : ne JAMAIS leak un message interne au
>    client (log côté serveur avec full context, retourne
>    `Status::internal("internal server error")`).
>
> 7. **Wirer dans `runtime::Server`** : étendre
>    `ServerBuilder::build()` pour enregistrer le nouveau
>    service. Threader la config `catalog.backend` (default
>    `memory`, prépare D14 `postgres`). Étendre la file
>    descriptor set pour `tonic-reflection`.
>
> 8. **Tests d'intégration** : au moins 30 nouveaux tests Rust
>    couvrant : 6 RPCs × happy path + tenant mismatch + scope
>    missing + invalid argument + concurrency (100 concurrent
>    register on same `(tenant, name)` → distinct integers) + 7
>    SLO activations + observability emission verifiable.
>
> 9. **Helm extensions** : valeurs `catalog.backend`,
>    `catalog.policy.maxNamesPerTenant`, etc. — le tout préparant
>    D14 sans rien casser de D11/D12/D13.
>
> 10. **Activation des 7 SLOs `D12-CT-1..7`** dans la baseline
>     v0.1.13b ou v0.1.14 : ils passent de SKIPPED à `passed:
>     true/false ; measured_ns: <réel>` après le 1er run vert
>     post-merge de `bench-catalog.yml` étendu.

### 1.4 Hors-périmètre (D14 scope)

D12-bis NE livre PAS :

* **Persistence postgres** — déféré explicitement à D14. Le
  trait `CatalogRegistry` reste le contrat ; la swap impl est
  invisible au handler.
* **Audit log HMAC signing** — déféré à D14-bis (key management +
  rotation procedure).
* **Embed pool** — le champ `compiled` de `CanonicalCatalog`
  reste `None` dans les réponses ; D14 (ou parallèle) câble un
  embed pool.
* **OTel SDK réel** — déféré à D13-bis ; les spans utilisent
  `tracing::info_span!` qui est une no-op en l'absence du
  layer OTel mais reste corrélable via la console subscriber
  pour le dev.
* **Bench load D12 path** — `ghz` contre les 6 RPCs est livré
  en D12-bis (pour activer les SLOs) mais le bench distribué
  (1000 RPS sustained) attend D14 pour avoir une persistence
  qui supporte la charge.

---

## 2. État des lieux post-D13 — pré-conditions remplies

D12-bis hérite de **9 invariants stables** que la phase
implementation ne doit PAS retoucher :

### 2.1 Invariants stables (NE PAS modifier)

| # | Invariant | Source | Audit qui le pinne |
|---|---|---|---|
| 1 | `CatalogRegistry` trait signature : 6 méthodes async, signatures finales (delete prend `principal: &str`) | `registry.rs::CatalogRegistry` | D12 audit H-4 fix |
| 2 | `InMemoryRegistry` impl + 5 evolve ops + transitive cascade delete | `memory.rs` | D12 audit C-1 fix |
| 3 | `CatalogPolicy` quotas defaults : 100 names/tenant, 1000 versions/name, 256 classes/catalog, 64 sentences/class | `policy.rs::DEFAULT_*` constants | D12 architecture § 11 |
| 4 | `AuditEmitter` interface : `emit_register/_evolve/_delete(key, meta, [op], principal)` | `audit.rs` | D12 architecture § 13 |
| 5 | `efe_audit_events_total` labels : `[tenant, action, op_tag, principal_present]` | `d13_instruments.rs` | D13 audit C-1 fix + `test_d13_metric_label_allowlist.py` |
| 6 | 16 span constants : `efe.catalog.{register,evolve,get,list,list_versions,delete}` + `efe.auth.verify` + `efe.ratelimit.acquire` + … | `spans.rs` | `test_d13_phase2_event_emitters.py::test_six_catalog_spans_declared` |
| 7 | 5 typed event builders : `RequestEvent`, `AuditEvent`, `ClassifyEvent`, `EvolutionEvent`, `SecurityEvent`, each with `into_value() -> serde_json::Value` matching the JSON-Schema | `events::d13_streams` | `test_d13_phase2_event_emitters.py` |
| 8 | `register_all_d13()` is invoked at startup from `efe_telemetry::init` | `init.rs` (D13 audit M-1 fix) | `test_d13_audit_fixes.py::test_telemetry_init_registers_d13_metrics` |
| 9 | Helm chart Tier 2 wires the auth interceptor that populates `AuthContext` into `Request<T>::extensions()` | M9 admin auth + D11 ServiceMonitor | `test_d11_audit_regressions.py` |

### 2.2 Gaps à fermer par D12-bis

| Gap | Statut |
|---|---|
| Stubs Rust tonic pour `catalog.proto` | absents (build.rs ne liste pas) |
| `services/catalog.rs` handler | absent |
| Wire dans `runtime::Server::builder()` | absent |
| `EFE_CATALOG_BACKEND` env var + `ServerConfig.catalog_backend` | absent |
| Helm `observability.dashboards` est wired (D13) MAIS `catalog.backend` n'existe pas | partiel |
| Tests d'intégration tonic pour `CatalogService` | 0 (les 29 tests D12 sont unit-only sur le trait) |
| Bench `ghz` contre les 6 RPCs | absent |
| Activation SLOs `D12-CT-1..7` | SKIPPED |

---

## 3. Pré-condition #0 — régénération des stubs Rust tonic

### 3.1 État actuel

`crates/efe-server/build.rs` :

```rust
let proto_files = [
    efe_v1.join("types.proto"),
    efe_v1.join("efe.proto"),
    efe_v1.join("admin.proto"),
];
```

Les stubs générés par `tonic-build` aboutissent dans le module
`crate::proto::v1::{efe, admin}` (cf. `lib.rs::proto`). Mais
`catalog.proto` n'est jamais compilé côté Rust. Aucun
`crate::proto::v1::catalog::catalog_service_server::CatalogService`
n'existe.

### 3.2 Modification minimale

```rust
let proto_files = [
    efe_v1.join("types.proto"),
    efe_v1.join("efe.proto"),
    efe_v1.join("admin.proto"),
    efe_v1.join("catalog.proto"),   // D12-bis NEW
];
```

### 3.3 Module proto exposé dans `lib.rs`

```rust
pub mod proto {
    pub mod v1 {
        // ... (existing types, efe, admin) ...

        /// Sprint **D12-bis** — catalog admin service (6 RPCs).
        pub mod catalog {
            #![allow(clippy::all, clippy::pedantic, missing_docs, unreachable_pub)]
            tonic::include_proto!("efe.v1.catalog");
        }
    }
}
```

### 3.4 Vérification post-build

Après `cargo build -p efe-server`, le binaire doit exposer :

* `crate::proto::v1::catalog::CatalogService` — trait à implémenter
* `crate::proto::v1::catalog::catalog_service_server::CatalogServiceServer<T>`
  — la `tower::Service`-compatible wrapper
* Tous les messages : `RegisterCatalogRequest`, `CatalogHandle`,
  `EvolveCatalogRequest`, `CatalogEvolutionOp`, `AppendClassOp`,
  `RemoveClassOp`, `UpdateThresholdsOp`, `ReplaceClassAnchorsOp`,
  `RecompileOp`, `GetCatalogRequest`, `CanonicalCatalog`,
  `CatalogMetadata`, `CompiledClass`, `ListCatalogsRequest`,
  `ListCatalogsResponse`, `CatalogRefSummary`,
  `ListCatalogVersionsRequest`, `ListCatalogVersionsResponse`,
  `DeleteCatalogRequest`, `CatalogContent`, `EmbedderSpec`,
  `CatalogClass`, `ConePolicy`, `ThresholdPolicy`, `CatalogRef`.

### 3.5 Test de fumée pré-implementation

Avant tout autre code D12-bis, ce test doit passer :

```rust
#[test]
fn catalog_proto_module_exposes_service_trait() {
    use crate::proto::v1::catalog::catalog_service_server::CatalogService;
    // Compile-time : the trait is in scope.
    fn _assert_send_sync<T: Send + Sync + 'static>() {}
    _assert_send_sync::<dyn CatalogService>();
}
```

Side-effect : la file descriptor set embarquée dans
`tonic-reflection` inclut désormais `efe.v1.catalog`. Le client
`grpcurl` peut introspecter le service.

---

## 4. Architecture du translation layer

### 4.1 Topologie des modules

D12-bis introduit **un seul fichier neuf** (et un sous-module
optionnel pour les conversions) :

```
crates/efe-server/src/services/
├── admin.rs              ← M9 (existant)
├── catalog.rs            ← D12-bis NEW
├── catalog/
│   └── conversions.rs    ← D12-bis NEW (helpers proto ↔ trait)
├── metrics.rs            ← M11 (existant)
├── mod.rs                ← étendu : pub mod catalog;
└── swarm.rs              ← M9 (existant)
```

### 4.2 Le struct `CatalogServiceImpl`

```rust
// crates/efe-server/src/services/catalog.rs

use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::auth::AuthContext;
use crate::catalog::{
    AuditEmitter, CatalogPolicy, CatalogRegistry,
};
use crate::proto::v1::catalog as pb;
use crate::proto::v1::catalog::catalog_service_server::CatalogService;

/// Sprint **D12-bis** — gRPC handler bridging the proto layer to
/// the in-memory (or postgres, post-D14) `CatalogRegistry`.
///
/// Holds `Arc<dyn CatalogRegistry>` + `Arc<AuditEmitter>` +
/// `Arc<CatalogPolicy>`. The handler is **stateless** — every
/// request reads the auth context from `Request<T>::extensions()`
/// (populated by the M9/D11 auth interceptor), so multiple
/// `CatalogServiceImpl` instances can be cloned cheaply.
#[derive(Clone)]
pub struct CatalogServiceImpl {
    registry: Arc<dyn CatalogRegistry>,
    audit:    Arc<AuditEmitter>,
    policy:   Arc<CatalogPolicy>,
}

impl CatalogServiceImpl {
    pub fn new(
        registry: Arc<dyn CatalogRegistry>,
        audit:    Arc<AuditEmitter>,
        policy:   Arc<CatalogPolicy>,
    ) -> Self {
        Self { registry, audit, policy }
    }
}
```

### 4.3 La `tonic::Service` impl (squelette)

```rust
#[tonic::async_trait]
impl CatalogService for CatalogServiceImpl {
    async fn register_catalog(
        &self,
        request: Request<pb::RegisterCatalogRequest>,
    ) -> Result<Response<pb::CatalogHandle>, Status> {
        // 1. Extract auth context (from interceptor extensions).
        let auth = AuthContext::from_request(&request)?;
        // 2. RBAC : require admin:catalog:write.
        require_scope(auth, "admin:catalog:write")?;
        // 3. Tenant invariant : claimed tenant must equal JWT tenant.
        assert_tenant_match(auth, &request.get_ref().tenant)?;
        // 4. Open the span.
        let _span = tracing::info_span!(
            crate::spans::CATALOG_REGISTER,
            tenant = %auth.tenant,
            name = %request.get_ref().name,
        )
        .entered();
        // 5. Time the histogram.
        let start = std::time::Instant::now();
        let result_label = "ok";  // updated on error path
        // 6. Translate proto → trait.
        let req = request.into_inner();
        let content = conversions::proto_content_to_trait(req.content)?;
        let metadata = build_initial_metadata(&auth, &req.description, &req.labels);
        // 7. Call the trait.
        let registered = self.registry
            .register(&auth.tenant, &req.name, content, metadata)
            .await
            .map_err(catalog_error_to_status)?;
        // 8. Audit emit (typed event + counter + tracing log).
        emit_register_audit(&self.audit, &auth, &registered);
        // 9. Histogram observe.
        catalog_register_duration_seconds()
            .with_label_values(&[&auth.tenant, result_label])
            .observe(start.elapsed().as_secs_f64());
        // 10. Translate trait → proto + return.
        let handle = conversions::registered_to_handle(registered)?;
        Ok(Response::new(handle))
    }

    // ... 5 other RPCs follow the same pattern ...
}
```

### 4.4 Pattern unifié (template pour les 6 RPCs)

```
[1] auth = AuthContext::from_request(&request)?
[2] require_scope(auth, <scope>)?
[3] assert_tenant_match(auth, <tenant in payload>)?  // skip for some RPCs
[4] _span = info_span!(spans::<NAME>, ...).entered()
[5] start = Instant::now()
[6] req = request.into_inner()
    domain_input = conversions::proto_to_trait(...)?
[7] result = self.registry.<method>(...).await
                .map_err(catalog_error_to_status)?
[8] emit_<verb>_audit(&self.audit, &auth, &result)
[9] <metric_for_verb>().observe(start.elapsed())
[10] response = conversions::trait_to_proto(result)?
     Ok(Response::new(response))
```

### 4.5 Pourquoi un struct par-handler et pas un free function ?

* **Dependency injection** : le handler dépend de 3 traits/structs
  qui doivent être Arc-shared. Un `Clone` de `CatalogServiceImpl`
  ne fait que cloner les `Arc`, coût négligeable.
* **Testabilité** : un `CatalogServiceImpl::new(mock_registry,
  mock_audit, default_policy)` permet de tester chaque méthode
  unit-style sans tonic.
* **Évolutivité D14** : quand on swap `InMemoryRegistry` →
  `PostgresCatalogRegistry`, c'est juste un changement de type
  derrière le `Arc<dyn CatalogRegistry>`. Le handler ne change
  pas.

---

## 5. Auth context — extraction du JWT

### 5.1 Pattern existant (M9 + D11)

Le serveur tonic, derrière son `tower::ServiceBuilder`, a un
`auth_interceptor` (existant) qui :

1. Lit le header `authorization: Bearer <jwt>`.
2. Vérifie la signature contre la `JWKs` configurée.
3. Décode les claims (`sub`, `tenant_id`, `scope`, `aud`, `iss`,
   `exp`, `iat`).
4. Vérifie `aud == "efe.v1"` et `iss` est dans la liste autorisée.
5. Construit un `AuthContext` :

```rust
// crates/efe-server/src/auth/context.rs

#[derive(Debug, Clone)]
pub struct AuthContext {
    /// `sub` claim. Empty for anonymous (only allowed in dev
    /// when EFE_AUTH_ENABLED=false).
    pub principal: String,
    /// `tenant_id` claim. Empty for anonymous.
    pub tenant: String,
    /// Parsed `scope` claim (space-separated).
    pub scopes: Vec<String>,
    /// Token expiry (unix seconds), used by some quota gates.
    pub expires_at: i64,
}
```

6. Insère le `AuthContext` dans les extensions de la `Request<T>` :

```rust
async fn auth_interceptor(mut req: Request<()>) -> Result<Request<()>, Status> {
    let token = extract_bearer(&req)?;
    let claims = validate_jwt(token)?;
    let auth = AuthContext {
        principal: claims.sub,
        tenant:    claims.tenant_id,
        scopes:    claims.scope.split_whitespace().map(|s| s.to_owned()).collect(),
        expires_at: claims.exp,
    };
    req.extensions_mut().insert(auth);
    Ok(req)
}
```

### 5.2 Helper `from_request`

D12-bis ajoute un helper public :

```rust
// crates/efe-server/src/auth/context.rs

impl AuthContext {
    /// Extract from request extensions. Returns
    /// `Status::unauthenticated` if absent (means the auth
    /// interceptor was not configured, or auth was disabled but
    /// the handler is being called directly).
    pub fn from_request<T>(req: &Request<T>) -> Result<&Self, Status> {
        req.extensions()
            .get::<Self>()
            .ok_or_else(|| Status::unauthenticated(
                "auth context missing — auth interceptor must be \
                 configured before this service",
            ))
    }
}
```

### 5.3 Auth disabled mode (dev / smoke tests)

Quand `EFE_AUTH_ENABLED=false`, l'interceptor injecte un
`AuthContext` synthétique :

```rust
AuthContext {
    principal: "anonymous-dev".to_owned(),
    tenant:    std::env::var("EFE_DEV_TENANT").unwrap_or_else(|_| "tenant-dev".to_owned()),
    scopes:    vec!["admin:catalog:read".into(), "admin:catalog:write".into(), ...],
    expires_at: i64::MAX,
}
```

Cela permet aux tests d'intégration et au dev local de fonctionner
sans IdP. **Jamais activer en prod** — un test CI rejette cette
combinaison via `test_d12bis_auth_disabled_only_in_dev.py`.

### 5.4 Diagram du flow auth pour 1 RPC

```
client SDK
    │ gRPC + bearer JWT
    ▼
tonic::Server (M9 baseline)
    │
    ├── tower::Layer : auth_interceptor          ← M9/D11
    │     │ extract Bearer
    │     │ verify signature
    │     │ decode claims
    │     │ AuthContext { principal, tenant, scopes }
    │     │ req.extensions_mut().insert(auth)
    │     ▼
    ├── tower::Layer : MetricsLayer              ← M11/D9
    │     │ record_request_completion
    │     ▼
    ├── tower::Layer : RateLimitLayer            ← D11 Tier 2
    │     │ token bucket per tenant
    │     ▼
    └── service : CatalogServiceImpl              ← D12-bis NEW
          │
          │ AuthContext::from_request(&req)?
          │ require_scope(auth, "admin:catalog:write")?
          │ assert_tenant_match(auth, &req.tenant)?
          ▼
        → registry.register(...).await
```

---

## 6. RBAC : scope checks par RPC + tenant assertion

### 6.1 Matrice scope × RPC

Référence architecturale D12 § 6 + D12-bis :

| RPC | Scope requis |
|---|---|
| `RegisterCatalog` | `admin:catalog:write` |
| `EvolveCatalog`   | `admin:catalog:write` |
| `GetCatalog`      | `admin:catalog:read` |
| `ListCatalogs`    | `admin:catalog:read` |
| `ListCatalogVersions` | `admin:catalog:read` |
| `DeleteCatalog`   | `admin:catalog:write` |

`include_compiled=true` sur `GetCatalog` ne demande **pas** de
scope additionnel (cf. D12 architecture § 6 décision : déjà
gated par `admin:catalog:read`).

### 6.2 Helper `require_scope`

```rust
// crates/efe-server/src/auth/scopes.rs

pub fn require_scope(auth: &AuthContext, scope: &str) -> Result<(), Status> {
    if auth.scopes.iter().any(|s| s == scope) {
        return Ok(());
    }
    // Emit a security event for the scope-missing case (D13 § 6.7).
    crate::events::security_emit(SecurityEvent::scope_missing(
        auth, scope,
    ));
    Err(Status::permission_denied(format!(
        "missing scope: {scope}"
    )))
}
```

L'event security est émis **avant** de retourner l'erreur. Il
alimente le dashboard 05 et l'alerte `D13ObsAuthFailureSpike`.

### 6.3 Helper `assert_tenant_match`

Le D11 invariant : le tenant claimed par le client dans le payload
doit matcher le tenant_id du JWT validé. Sinon → `permission_denied`.

```rust
// crates/efe-server/src/auth/scopes.rs

pub fn assert_tenant_match(
    auth: &AuthContext,
    claimed_tenant: &str,
) -> Result<(), Status> {
    if auth.tenant == claimed_tenant {
        return Ok(());
    }
    // Emit a security event (tenant_mismatch is alert-worthy).
    crate::events::security_emit(SecurityEvent::tenant_mismatch(
        auth, claimed_tenant,
    ));
    Err(Status::permission_denied(format!(
        "tenant mismatch: JWT claims {jwt_tenant!r}, request claims {req_tenant!r}",
        jwt_tenant = &auth.tenant,
        req_tenant = claimed_tenant,
    )))
}
```

### 6.4 Quels RPCs ont besoin de `assert_tenant_match` ?

| RPC | Tenant dans le payload ? | Vérif requise |
|---|---|---|
| `RegisterCatalog` | `request.tenant` | OUI |
| `EvolveCatalog` | parsé depuis `from_key` | OUI (parse + compare) |
| `GetCatalog` | parsé depuis `key` | OUI |
| `ListCatalogs` | `request.tenant` | OUI |
| `ListCatalogVersions` | `request.tenant` | OUI |
| `DeleteCatalog` | parsé depuis `key` | OUI |

Le helper accepte un `&str` ; pour les RPCs avec key, on parse
d'abord (helper `parse_canonical_key`) et on extrait le tenant.

### 6.5 Cas dégénéré : `GetCatalog` avec tenant invalide dans la key

Scénario : `auth.tenant = "acme"`, `request.key = "globex/billing@v1"`.

Le handler :

1. Parse la key → `(globex, billing, 1)`.
2. `assert_tenant_match("acme", "globex")` → fail → `permission_denied`.

L'erreur est retournée AVANT que la registry ne soit interrogée.
Pas de fuite « catalog X exists ? oui/non » entre tenants — le
serveur répond identiquement que le catalog n'existe pas pour
acme, ou que le tenant ne correspond pas.

D11 architecture § 4 mandate "no information leak" entre tenants.
D12-bis hérite de cette garantie via `assert_tenant_match` avant
tout call à la registry.

### 6.6 Test dédié

```rust
#[tokio::test]
async fn cross_tenant_get_catalog_returns_permission_denied() {
    let (server, _addr) = test_server_with_registry().await;
    let mut client = catalog_client_for_tenant(server, "acme").await;
    let resp = client.get_catalog(GetCatalogRequest {
        key: "globex/billing@v1".to_owned(),
        include_compiled: false,
    }).await;
    assert_eq!(resp.unwrap_err().code(), Code::PermissionDenied);
}
```

---

## 7. Conversions proto ↔ trait

### 7.1 Structure du module

```
crates/efe-server/src/services/catalog/conversions.rs
├── content
│   ├── proto_content_to_trait : pb::CatalogContent → CatalogContent
│   └── trait_content_to_proto : CatalogContent → pb::CatalogContent
├── class
│   ├── proto_class_to_trait
│   └── trait_class_to_proto
├── metadata
│   ├── proto_metadata_to_trait
│   └── trait_metadata_to_proto
├── evolve_op
│   └── proto_op_to_trait : pb::CatalogEvolutionOp → EvolveOp
├── handle
│   └── registered_to_handle : RegisteredCatalog → pb::CatalogHandle
└── canonical_key
    ├── parse_canonical_key : &str → (tenant, name, version)
    └── format_canonical_key : (tenant, name, version) → String
```

### 7.2 Pattern de validation à la traversée

Chaque helper qui lit un proto retourne `Result<DomainType, Status>`
— jamais `panic`, jamais `silently fix`. Exemples :

```rust
pub fn proto_content_to_trait(
    p: Option<pb::CatalogContent>,
) -> Result<CatalogContent, Status> {
    let p = p.ok_or_else(|| Status::invalid_argument(
        "content is required"
    ))?;
    let embedder = p.embedder.ok_or_else(|| Status::invalid_argument(
        "content.embedder is required"
    ))?;
    if embedder.dim == 0 {
        return Err(Status::invalid_argument(
            "embedder.dim must be > 0",
        ));
    }
    if p.classes.len() < 2 {
        return Err(Status::invalid_argument(
            format!("content.classes must have ≥ 2 entries, got {}",
                    p.classes.len()),
        ));
    }
    Ok(CatalogContent {
        embedder: EmbedderSpec {
            family: embedder.family,
            model: embedder.model,
            dim: embedder.dim,
            normalize: embedder.normalize,
        },
        classes: p.classes
            .into_iter()
            .map(proto_class_to_trait)
            .collect::<Result<Vec<_>, _>>()?,
        cone_policy: p.cone_policy
            .map(|c| ConePolicy { max_projection_distance: c.max_projection_distance })
            .unwrap_or_default(),
        thresholds: p.thresholds
            .map(|t| ThresholdPolicy {
                confidence_high:   t.confidence_high,
                confidence_medium: t.confidence_medium,
            })
            .unwrap_or_default(),
    })
}
```

Les invariants quotas (≥ 2 classes, ≤ 64 sentences/class, etc.)
sont déjà validés par `CatalogPolicy::validate_content` à l'intérieur
de la trait. **Mais** la conversion fait un fail-fast pour les
violations grossières (proto champ absent, dim = 0) afin que :

* le call à la trait ne soit pas tenté inutilement ;
* l'observabilité enregistre un `result="invalid_argument"` plutôt
  qu'un `result="ok"` immédiatement suivi d'un `Err` interne.

### 7.3 Conversion `pb::CatalogEvolutionOp` → `EvolveOp`

Le proto a un oneof :

```proto
message CatalogEvolutionOp {
  oneof op {
    AppendClassOp        append_class = 1;
    RemoveClassOp        remove_class = 2;
    UpdateThresholdsOp   update_thresholds = 3;
    ReplaceClassAnchorsOp replace_class_anchors = 4;
    RecompileOp          recompile = 5;
  }
}
```

tonic-build génère un enum `pb::catalog_evolution_op::Op` avec 5
variants. La conversion :

```rust
pub fn proto_op_to_trait(
    p: pb::CatalogEvolutionOp,
) -> Result<EvolveOp, Status> {
    use pb::catalog_evolution_op::Op as OneOf;
    let inner = p.op.ok_or_else(|| Status::invalid_argument(
        "evolve op is required (one of append_class, remove_class, …)",
    ))?;
    Ok(match inner {
        OneOf::AppendClass(o) => {
            let class = proto_class_to_trait(o.cls.ok_or_else(|| {
                Status::invalid_argument("AppendClassOp.cls is required")
            })?)?;
            EvolveOp::AppendClass { class }
        }
        OneOf::RemoveClass(o) => EvolveOp::RemoveClass {
            class_name: o.class_name,
        },
        OneOf::UpdateThresholds(o) => {
            let thresholds = o.thresholds.ok_or_else(|| {
                Status::invalid_argument("UpdateThresholdsOp.thresholds is required")
            })?;
            EvolveOp::UpdateThresholds {
                thresholds: ThresholdPolicy {
                    confidence_high:   thresholds.confidence_high,
                    confidence_medium: thresholds.confidence_medium,
                },
            }
        }
        OneOf::ReplaceClassAnchors(o) => EvolveOp::ReplaceClassAnchors {
            class_name: o.class_name,
            canonical_sentences: o.canonical_sentences,
        },
        OneOf::Recompile(o) => {
            let new_embedder = o.new_embedder.ok_or_else(|| {
                Status::invalid_argument("RecompileOp.new_embedder is required")
            })?;
            EvolveOp::Recompile {
                new_embedder: EmbedderSpec {
                    family:    new_embedder.family,
                    model:     new_embedder.model,
                    dim:       new_embedder.dim,
                    normalize: new_embedder.normalize,
                },
            }
        }
    })
}
```

### 7.4 Construction du `CatalogMetadata` initial

À l'arrivée du `RegisterCatalogRequest`, le handler construit un
`CatalogMetadata` :

```rust
fn build_initial_metadata(
    auth: &AuthContext,
    description: &str,
    labels: &std::collections::HashMap<String, String>,
) -> CatalogMetadata {
    CatalogMetadata {
        created_at_unix_ms: now_unix_ms(),
        created_by:         auth.principal.clone(),
        description:        description.to_owned(),
        parent_key:         String::new(),       // overwritten by trait
        evolution_op:       String::new(),       // overwritten by trait
        labels:             labels.clone().into_iter().collect(),
        content_sha256:     ContentDigest(String::new()),  // computed by trait
    }
}
```

Le trait `register()` overwrite `evolution_op="register"`, vide
`parent_key`, et stamp `content_sha256`. Le handler n'a pas besoin
de toucher à ces champs.

### 7.5 Round-trip `CanonicalCatalog` → `pb::CatalogHandle`

```rust
pub fn registered_to_handle(
    r: RegisteredCatalog,
) -> Result<pb::CatalogHandle, Status> {
    Ok(pb::CatalogHandle {
        key: r.key.to_wire(),
        catalog: Some(registered_to_canonical_proto(r)?),
    })
}

pub fn registered_to_canonical_proto(
    r: RegisteredCatalog,
) -> Result<pb::CanonicalCatalog, Status> {
    Ok(pb::CanonicalCatalog {
        content:  Some(trait_content_to_proto(r.content)?),
        metadata: Some(trait_metadata_to_proto(r.metadata)),
        compiled: r.compiled
            .unwrap_or_default()
            .into_iter()
            .map(|c| pb::CompiledClass {
                class_name:           c.class_name,
                anchors_blob:         c.anchors_blob,
                num_anchors:          c.num_anchors,
                embedder_fingerprint: c.embedder_fingerprint,
            })
            .collect(),
    })
}
```

### 7.6 Tests de conversion

Au moins **20 tests** dans `services/catalog/conversions.rs::tests` :

* round-trip `proto_content_to_trait → trait_content_to_proto` =
  identity (modulo ordering).
* parsing `evolve op` × 5 variants.
* fail-fast sur proto invalides (dim=0, classes vides, oneof
  manquant).
* boundary : 64 sentences (max), 256 classes (max).
* unicode safety : class name avec caractères spéciaux.
* canonical_key parse : `acme/billing@v3`, `acme/billing@3`,
  `acme/billing@v0` (rejette).

---

## 8. Error mapping `CatalogError` → `tonic::Status`

### 8.1 Matrice canonique

| `CatalogError` variant | `tonic::Code` | Message client | Log côté serveur |
|---|---|---|---|
| `NotFound(msg)` | `NotFound` | `msg` (déjà sanitisé par le trait) | `info` |
| `InvalidArgument(msg)` | `InvalidArgument` | `msg` | `info` |
| `PermissionDenied(msg)` | `PermissionDenied` | `msg` | `warn` (security event) |
| `QuotaExceeded(msg)` | `ResourceExhausted` | `msg` | `warn` |
| `Conflict(msg)` | `FailedPrecondition` | `msg` | `info` |
| `Internal(msg)` | `Internal` | **`"internal server error"`** (pas le message) | `error` (avec full context, request_id, trace_id) |

### 8.2 La fonction `catalog_error_to_status`

```rust
fn catalog_error_to_status(err: CatalogError) -> Status {
    use CatalogError::*;
    let (code, client_msg, log_level) = match &err {
        NotFound(m)         => (Code::NotFound,           m.clone(),                 LogLevel::Info),
        InvalidArgument(m)  => (Code::InvalidArgument,    m.clone(),                 LogLevel::Info),
        PermissionDenied(m) => (Code::PermissionDenied,   m.clone(),                 LogLevel::Warn),
        QuotaExceeded(m)    => (Code::ResourceExhausted,  m.clone(),                 LogLevel::Warn),
        Conflict(m)         => (Code::FailedPrecondition, m.clone(),                 LogLevel::Info),
        Internal(_)         => (Code::Internal, "internal server error".to_owned(), LogLevel::Error),
    };
    log_at_level(log_level, "catalog error", &err);
    Status::new(code, client_msg)
}
```

### 8.3 Sanitisation des erreurs internes

**Threat model T-12bis-2** : un message d'erreur peut leak :

* la structure SQL interne (« relation `catalogs` does not exist »).
* le path interne (« failed to open /var/lib/efe/state.json »).
* l'IP d'un node interne (« connection refused 10.0.0.7:5432 »).

Ces fuites sont des indicateurs de surface d'attaque. Le mapping
`Internal(...) → "internal server error"` est strict : seul le
log côté serveur (avec `trace_id` corrélé) garde le message
original.

### 8.4 Tests d'error mapping

```rust
#[test]
fn internal_error_does_not_leak_to_client() {
    let err = CatalogError::Internal("connection to 10.0.0.7:5432 refused".into());
    let status = catalog_error_to_status(err);
    assert_eq!(status.code(), Code::Internal);
    assert_eq!(status.message(), "internal server error");
    // The original message is NOT in the public surface.
    assert!(!status.message().contains("10.0.0.7"));
}
```

---

## 9. Observability emission par RPC

### 9.1 Les 4 axes obligatoires

Chaque RPC émet :

| Axe | Source | Cardinalité supplémentaire |
|---|---|---|
| 1. Span tracing (OTel-ready) | `spans::CATALOG_<verb>` | aucune (1 span/req) |
| 2. Histogramme latency | `efe_catalog_<verb>_duration_seconds{tenant, [op,] [include_compiled,] result}` | per RPC : voir D13 § 5.2 |
| 3. Counter audit | `efe_audit_events_total{tenant, action, op_tag, principal_present}` | seulement pour register/evolve/delete |
| 4. Structured log | `events::d13_streams::<XxxEvent>::into_value()` via `tracing::info!(target = "...")` | per RPC : 1 ligne JSON |

### 9.2 Pattern unifié `emit_<verb>_audit`

```rust
fn emit_register_audit(
    audit:      &AuditEmitter,
    auth:       &AuthContext,
    registered: &RegisteredCatalog,
) {
    // 1. Tracing-based audit emit (D12 audit.rs).
    audit.emit_register(&registered.key, &registered.metadata);

    // 2. Counter increment with principal_present (D13 audit C-1 fix).
    crate::metrics::d13_instruments::audit_events_total()
        .with_label_values(&[
            &auth.tenant,
            "register",
            "register",
            if auth.principal.is_empty() { "false" } else { "true" },
        ])
        .inc();

    // 3. Structured JSON log via the typed builder.
    let event = events::d13_streams::AuditEvent {
        envelope: events::d13_streams::CommonEnvelope {
            timestamp:       events::d13_streams::format_rfc3339_now(),
            level:           events::d13_streams::Level::Info,
            request_id:      None,
            trace_id:        current_trace_id(),
            span_id:         current_span_id(),
            tenant_id:       Some(auth.tenant.clone()),
            principal:       Some(auth.principal.clone()),
            service_version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        action:         events::d13_streams::AuditAction::Register,
        resource_type:  "catalog".to_owned(),
        resource_key:   registered.key.to_wire(),
        parent_key:     None,
        op_tag:         "register".to_owned(),
        content_sha256: Some(registered.metadata.content_sha256.0.clone()),
        description:    Some(registered.metadata.description.clone()),
        labels:         registered.metadata.labels.clone(),
    };
    tracing::info!(
        target: "audit",
        json = %event.into_value(),
        "audit",
    );
}
```

### 9.3 Patterns par verbe

#### 9.3.1 Register

* Span : `spans::CATALOG_REGISTER`
* Attributes : `tenant`, `name`, `n_classes`
* Histogram : `efe_catalog_register_duration_seconds{tenant, result}`
* Counter : `efe_audit_events_total{tenant, action="register", op_tag="register", principal_present=...}`
* Log : `AuditEvent { action: Register, op_tag: "register", parent_key: None }`

#### 9.3.2 Evolve

* Span : `spans::CATALOG_EVOLVE`
* Attributes : `tenant`, `name`, `from_version`, `op` (audit_tag)
* Histogram : `efe_catalog_evolve_duration_seconds{tenant, op, result}`
* Counter : `efe_audit_events_total{tenant, action="evolve", op_tag=<op_tag>, principal_present=...}`
* Log : `EvolutionEvent { from_key, to_key, op_tag, diff_summary }`
  * Le `diff_summary` est calculé en comparant `before` (fetched
    avant l'evolve) et `after` (le retour de la trait).
  * Pour APPEND_CLASS, `diff_summary.classes_added = ["churn"]`.
  * Pour REMOVE_CLASS, `diff_summary.classes_removed = ["spam"]`.
  * Etc.

Implementation hint : la trait `evolve()` retourne le `RegisteredCatalog`
final. Le handler peut comparer son `content` à celui de la
version source (fetched via un `get()` interne avant le call).
**Mais** ça dérape la concurrence (race entre get + evolve). La
décision (D-12bis-7) : laisser le trait calculer le diff et
l'exposer dans une nouvelle field `diff_summary` du
`RegisteredCatalog`, OU calculer le diff dans le handler à partir
du `op` (qui contient déjà la info nécessaire pour les 4 ops
non-RECOMPILE).

Décision retenue : **calculer dans le handler à partir du `op`**.
Pas de nouveau field sur la trait. Pour RECOMPILE, le diff_summary
ne contient que `embedder_old_fingerprint` + `embedder_new_fingerprint`
(les classes ne changent pas, seul l'embedder change).

#### 9.3.3 Get

* Span : `spans::CATALOG_GET`
* Attributes : `key`, `include_compiled`
* Histogram : `efe_catalog_get_duration_seconds{tenant, include_compiled, result}`
  où `include_compiled` est `"true"`/`"false"` (label string)
* **Pas de counter audit** — les reads ne génèrent pas d'audit
  event (par décision D-12bis-3).
* **Pas de log audit** — seulement un `RequestEvent` standard
  émis par l'interceptor metrics (M11).

#### 9.3.4 List + ListVersions

* Span : `spans::CATALOG_LIST` ou `spans::CATALOG_LIST_VERSIONS`
* Attributes : `tenant`, `page_size` (pour List), `name` (pour ListVersions)
* Histogram : `efe_catalog_list_duration_seconds{tenant, result}`
  (les deux verbes share un histogramme — `name` n'est pas un
  label pour bound la cardinalité)
* Pas de counter, pas de log audit.

#### 9.3.5 Delete

* Span : `spans::CATALOG_DELETE`
* Attributes : `key`, `force`
* Histogram : pas dédié — les deletes sont rares ; le `MetricsLayer`
  M11 capture déjà la latency via `efe_grpc_request_duration_seconds`.
* Counter : `efe_audit_events_total{action="delete", op_tag="delete", principal_present=...}`
* Log : `AuditEvent { action: Delete, op_tag: "delete", description: if force { "force=true" } else { "" } }`

### 9.4 Cardinalité totale

Pour 100 tenants × 5 evolve ops × 2 result × 2 principal_present
= 2 000 séries pour `efe_catalog_evolve_duration_seconds`. Bien
sous le budget (D13 § 5.4 estime 18 000 séries totales `efe_*`).

`audit_events_total` : 100 × 3 actions × 7 op_tags × 2
principal_present = 4 200 séries. OK.

---

## 10. Concurrency model — le handler fait confiance au trait

### 10.1 Garanties du trait

Le trait `CatalogRegistry` (D12) garantit, pour
`InMemoryRegistry` :

* **Register monotone** : 100 register concurrents sur le même
  `(tenant, name)` produisent 100 versions distinctes,
  numérotées sequentiellement.
* **Cascade delete correcte** : le delete avec `force=true`
  supprime uniquement les descendants transitifs (audit fix
  C-1).
* **Read consistency** : un `get()` voit l'état post-write
  serializé (pas de partial visibility).

Tout cela via un `tokio::sync::RwLock<BTreeMap<...>>`. Une seule
écriture à la fois ; lectures parallèles tant qu'aucune écriture
en cours.

### 10.2 Le handler n'ajoute rien

D12-bis ne réimplémente pas la concurrence. Le handler :

* Appelle `self.registry.register(...).await` — bloqué tant que
  d'autres writes terminent (ordre arrival-order).
* Si le `RwLock` est saturé (tous les futures bloquent), le
  back-pressure remonte via `tokio` runtime.
* Pas de timeout côté handler — le timeout client gRPC (D9 SLO :
  30 s par défaut) suffit.

### 10.3 Cas limite : graceful shutdown pendant une write

Le `Shutdown` infrastructure (D11) émet un signal SIGTERM.
`runtime::Server` propage à tous les handlers via `select!{}`.
Pour les writes en cours :

1. Le future `registry.register(...).await` continue d'exécuter
   (pas de drop forcé) — le `RwLock` write est tenu.
2. Le serveur attend `terminationGracePeriodSeconds` (Helm :
   30s par défaut) avant de force-exit.
3. Si le write finit avant, ok ; sinon, kill SIGKILL et l'in-memory
   state est perdu (par essence).

D14 (postgres) résout définitivement ce problème : le write est
durably committed avant return.

### 10.4 Test de concurrence (preuve du SLO D12-CT-6)

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_register_assigns_distinct_versions() {
    let (server, addr) = start_test_server().await;
    let mut clients = (0..100)
        .map(|_| catalog_client_for_tenant(addr.clone(), "acme"))
        .collect::<futures::stream::FuturesUnordered<_>>();

    // Fire 100 concurrent register on the same (tenant=acme, name=billing).
    let mut handles = Vec::with_capacity(100);
    for mut c in clients {
        handles.push(tokio::spawn(async move {
            c.register_catalog(sample_register_req("billing")).await
        }));
    }
    let results = futures::future::join_all(handles).await;
    let versions = results
        .into_iter()
        .filter_map(|r| r.ok())
        .filter_map(|r| r.ok())
        .map(|r| r.into_inner().catalog.unwrap().metadata.unwrap())
        .map(|m| {
            // Parse version from key
            let key = m.parent_key; // ... actually use key from handle
            parse_version_from_key(&key)
        })
        .collect::<Vec<_>>();
    let unique: std::collections::HashSet<_> = versions.iter().copied().collect();
    assert_eq!(versions.len(), 100);
    assert_eq!(unique.len(), 100, "100 concurrent registers must produce 100 distinct versions");
}
```

Ce test alimente le SLO `D12-CT-6` (`catalog::serialization_pass_pct`).

---

## 11. Configuration + `ServerConfig` knobs

### 11.1 Nouvelle config bloc

```rust
// crates/efe-server/src/config.rs

#[derive(Debug, Clone)]
pub struct ServerConfig {
    // ... existing fields ...

    /// Sprint **D12-bis** — catalog backend selector.
    pub catalog: CatalogConfig,
}

#[derive(Debug, Clone)]
pub struct CatalogConfig {
    /// Backend impl. v0.1.13 supports `"memory"` only.
    /// v0.1.14 (D14) adds `"postgres"`.
    pub backend: CatalogBackend,
    /// Tunable policy. Defaults match `policy.rs::DEFAULT_*`.
    pub policy: CatalogPolicyConfig,
    /// (D14 only) Postgres connection settings. Ignored when
    /// `backend == Memory`.
    pub postgres: Option<PostgresConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogBackend {
    Memory,
    /// Reserved for D14 ; rejected at startup in v0.1.13.
    Postgres,
}

#[derive(Debug, Clone)]
pub struct CatalogPolicyConfig {
    pub max_names_per_tenant:    u32,
    pub max_versions_per_name:   u32,
    pub max_classes_per_catalog: u32,
    pub max_sentences_per_class: u32,
}

impl Default for CatalogPolicyConfig {
    fn default() -> Self {
        Self {
            max_names_per_tenant:    100,
            max_versions_per_name:   1000,
            max_classes_per_catalog: 256,
            max_sentences_per_class: 64,
        }
    }
}
```

### 11.2 Env var resolution

```rust
fn catalog_config_from_env() -> Result<CatalogConfig, ConfigError> {
    let backend = match std::env::var("EFE_CATALOG_BACKEND")
        .as_deref().unwrap_or("memory")
    {
        "memory"   => CatalogBackend::Memory,
        "postgres" => CatalogBackend::Postgres,  // accepted but rejected later
        other      => return Err(ConfigError::Invalid(format!(
            "EFE_CATALOG_BACKEND={other!r} ; expected memory or postgres"
        ))),
    };
    Ok(CatalogConfig {
        backend,
        policy: catalog_policy_from_env()?,
        postgres: None,  // D14
    })
}
```

### 11.3 Validation d'erreur D14-pending

```rust
impl ServerBuilder {
    pub fn build(self) -> Result<Server, ServerBuildError> {
        // ... existing code ...

        let registry: Arc<dyn CatalogRegistry> = match cfg.catalog.backend {
            CatalogBackend::Memory => Arc::new(InMemoryRegistry::new()),
            CatalogBackend::Postgres => {
                return Err(ServerBuildError::CatalogBackendNotImplemented(
                    "postgres backend is reserved for D14 ; \
                     set EFE_CATALOG_BACKEND=memory until then".to_owned()
                ));
            }
        };
        // ... rest of build ...
    }
}
```

### 11.4 Helm values — préparation D14

```yaml
# deploy/helm/efe/values.yaml — D12-bis additions

catalog:
  # Backend impl. "memory" (default) keeps state in-process — lost
  # on every restart. "postgres" lands in D14.
  backend: memory

  # Tunable policy (D12 quotas). Override per-deployment.
  policy:
    maxNamesPerTenant:    100
    maxVersionsPerName:   1000
    maxClassesPerCatalog: 256
    maxSentencesPerClass: 64

  # Postgres config. Ignored when backend=memory ; documented now
  # so D14 deployments don't need a chart upgrade.
  postgres:
    enabled: false
    # See `crates/efe-server/src/catalog/pg_config.rs` for full schema.
    url: ""
    poolSize: 10
    statementCacheSize: 100
    sslMode: prefer
    secretRef: ""        # name of an existing Secret with PG_PASSWORD
```

---

## 12. Wire dans `runtime::Server`

### 12.1 `ServerBuilder::build()` — diff

```rust
// crates/efe-server/src/runtime.rs (D12-bis additions)

impl ServerBuilder {
    pub fn build(self) -> Result<Server, ServerBuildError> {
        let cfg = self.config;

        // ... existing init : telemetry, auth, ratelimit, idempotency ...

        // D12-bis NEW : catalog backend.
        let registry: Arc<dyn CatalogRegistry> =
            build_catalog_registry(&cfg.catalog)?;
        let audit: Arc<AuditEmitter> = Arc::new(AuditEmitter::new());
        let policy: Arc<CatalogPolicy> = Arc::new(
            CatalogPolicy::from_config(&cfg.catalog.policy)
        );

        // D12-bis NEW : catalog service.
        let catalog_service = CatalogServiceImpl::new(
            registry,
            audit,
            policy,
        );

        // Build the tonic server.
        let mut router = tonic::transport::Server::builder()
            .layer(MetricsLayer::new())
            .layer(auth_layer.clone())          // M9
            .layer(idempotency_layer.clone())   // D11
            .layer(ratelimit_layer.clone())     // D11
            .add_service(SwarmServiceServer::new(swarm_impl))    // M9
            .add_service(AdminServiceServer::new(admin_impl))    // M9 (cone admin)
            .add_service(catalog_service_server::CatalogServiceServer::new(catalog_service))  // D12-bis
            .add_service(reflection_service);                    // M11

        Ok(Server { router, /* ... */ })
    }
}
```

### 12.2 Reflection descriptor extension

`build.rs` doit inclure `catalog.proto` dans le file descriptor
set utilisé par `tonic-reflection`. Une seule ligne :

```rust
tonic_build::configure()
    .build_server(true)
    .build_client(true)
    .file_descriptor_set_path(&descriptor_path)
    .compile_protos(&proto_files, &[protos_root.clone()])?;
```

`proto_files` étant étendu (cf. § 3.2), le descriptor inclut
automatiquement `efe.v1.catalog`.

Vérification post-build :

```bash
grpcurl -plaintext localhost:50051 list
# Expected output (sorted) :
#   efe.v1.AdminService
#   efe.v1.SwarmService
#   efe.v1.catalog.CatalogService     ← D12-bis NEW
#   grpc.reflection.v1alpha.ServerReflection
```

### 12.3 Layers ordering — pourquoi `auth` AVANT `metrics` ?

L'ordre des layers dans tonic est important :

1. `MetricsLayer` (M11) : observe la latency *brute*, y compris
   le temps passé dans les autres layers.
2. `auth_layer` (M9/D11) : reject les requests non-authentifiées
   *avant* qu'elles n'atteignent l'idempotency cache.
3. `idempotency_layer` (D11) : de-duplique sur `(tenant,
   X-Idempotency-Key)` — donc requiert que le tenant soit déjà
   extrait par auth.
4. `ratelimit_layer` (D11) : rejets après auth (per-tenant
   bucket).
5. Service handlers.

D12-bis ne change pas l'ordre. Le `CatalogServiceImpl` consomme
ce que les layers en amont ont posé dans `extensions()`.

---

## 13. Tests stratégie — unit + intégration + concurrence + isolation

### 13.1 Tests unit (mocked registry)

Cible : `services/catalog.rs::tests`. Couvre la logique du
handler isolée du trait via une `MockCatalogRegistry`.

```rust
#[derive(Clone, Default)]
struct MockCatalogRegistry {
    register_calls: Arc<Mutex<Vec<(String, String)>>>,
    register_response: Arc<Mutex<Option<RegisteredCatalog>>>,
    register_error: Arc<Mutex<Option<CatalogError>>>,
}

#[async_trait]
impl CatalogRegistry for MockCatalogRegistry {
    async fn register(&self, tenant: &str, name: &str, _content: CatalogContent, _meta: CatalogMetadata)
        -> Result<RegisteredCatalog, CatalogError>
    {
        self.register_calls.lock().unwrap().push((tenant.into(), name.into()));
        if let Some(err) = self.register_error.lock().unwrap().clone() {
            return Err(err);
        }
        Ok(self.register_response.lock().unwrap().clone().unwrap())
    }
    // ... 5 other methods ...
}
```

15 unit tests :

* register : happy path → registry called, audit emitted
* register : missing scope → permission_denied
* register : tenant mismatch → permission_denied
* register : invalid content (1 class) → invalid_argument
* register : registry returns Internal → status maps to internal, internal log emitted, message sanitized
* register : audit_events_total incremented with principal_present="true"
* register : audit_events_total incremented with principal_present="false" when JWT sub is empty
* evolve : per-op audit_tag in counter label
* evolve : diff_summary computed from op
* get : missing scope → permission_denied
* get : include_compiled=true threaded to trait
* list : page_size validated
* list_versions : tenant mismatch → permission_denied
* delete : principal threaded to trait
* delete : force=true threaded to trait

### 13.2 Tests d'intégration (in-process tonic server)

Cible : `crates/efe-server/tests/d12bis_catalog_integration.rs`.
Pattern : démarrer un `Server` complet sur un port aléatoire,
exécuter des appels via le client tonic généré.

```rust
async fn start_integration_server(
    auth_disabled_tenant: Option<&str>,
) -> (Arc<Notify>, SocketAddr) {
    let mut cfg = ServerConfig::default();
    cfg.catalog = CatalogConfig::default();  // memory backend
    cfg.auth.enabled = auth_disabled_tenant.is_none();
    if let Some(t) = auth_disabled_tenant {
        std::env::set_var("EFE_DEV_TENANT", t);
    }
    let server = Server::builder().config(cfg).build().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let shutdown = Arc::new(Notify::new());
    let s = Arc::clone(&shutdown);
    tokio::spawn(async move {
        server.serve_with_listener(listener, async move {
            s.notified().await
        }).await.unwrap();
    });
    (shutdown, addr)
}
```

20 tests d'intégration :

* Each of the 6 RPCs : happy path + error case (12 tests).
* Cross-tenant get returns permission_denied (1 test).
* Tenant mismatch on register returns permission_denied (1 test).
* Scope deny matrix (read endpoint with no scope, write endpoint
  with read-only scope) (3 tests).
* Reflection : `grpcurl` lists catalog.CatalogService (1 test, via
  tonic-reflection client API).
* Idempotency : same `X-Idempotency-Key` on register returns the
  same response (1 test).
* Rate limit : burst of 1000 register saturates the tenant bucket,
  some requests get `ResourceExhausted` (1 test).

### 13.3 Tests de concurrence (preuve SLO D12-CT-6)

`tests/d12bis_catalog_concurrency.rs` :

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_register_distinct_versions() { ... }

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_evolve_increments_monotonically() { ... }

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_delete_does_not_corrupt_chain() { ... }
```

Chaque test fire 100 ops parallèles, mesure la pass-rate, vérifie
les invariants. Ces tests tournent dans le job CI
`bench-catalog.yml` et alimentent `obs::dash::cardinality_bound_count`
(D13-OBS-5).

### 13.4 Tests d'isolation (preuve D11-MT-1 hold)

`tests/d12bis_catalog_isolation.rs` :

```rust
#[tokio::test]
async fn tenant_a_cannot_see_tenant_b_catalogs() { ... }

#[tokio::test]
async fn tenant_a_cannot_register_under_tenant_b() { ... }

#[tokio::test]
async fn tenant_a_cannot_delete_tenant_b_catalog() { ... }
```

Ces tests réutilisent le harness D11 isolation (`tests/d11_isolation_*.rs`)
avec les RPCs catalog.

### 13.5 Tests d'observability emission

`tests/d12bis_catalog_observability.rs` :

```rust
#[tokio::test]
async fn register_increments_audit_events_total() {
    let (server, addr) = start_integration_server(Some("acme")).await;
    let mut client = catalog_client(addr).await;
    let before = audit_events_total()
        .with_label_values(&["acme", "register", "register", "true"])
        .get();
    client.register_catalog(sample_register_req()).await.unwrap();
    let after = audit_events_total()
        .with_label_values(&["acme", "register", "register", "true"])
        .get();
    assert_eq!(after, before + 1);
}

#[tokio::test]
async fn register_observes_duration_histogram() { ... }

#[tokio::test]
async fn register_emits_audit_log_via_tracing_subscriber() { ... }

#[tokio::test]
async fn delete_with_empty_principal_increments_principal_present_false() { ... }
```

### 13.6 Synthèse — tests added en D12-bis

| Catégorie | Count |
|---|---|
| Unit (services/catalog.rs::tests) | ~ 15 |
| Conversions (services/catalog/conversions.rs::tests) | ~ 20 |
| Intégration (tests/d12bis_catalog_integration.rs) | ~ 20 |
| Concurrence (tests/d12bis_catalog_concurrency.rs) | ~ 5 |
| Isolation (tests/d12bis_catalog_isolation.rs) | ~ 5 |
| Observability (tests/d12bis_catalog_observability.rs) | ~ 8 |
| **Total Rust** | **~ 73** |

Plus les Python tests existants (~ 240 D13 + D12) qui
deviennent enfin verts end-to-end (les mocks ne sont plus
nécessaires pour la chaîne D12 → D13).

---

## 14. Extension du workflow `bench-catalog.yml`

Le workflow CI D12 (`bench-catalog.yml`) tourne actuellement :

* `cargo test -p efe-server --lib catalog::` (29 tests trait
  in-memory)
* `pytest scripts/tests/test_d12_*.py` (~ 70 tests SDK + CLI)

D12-bis étend :

```yaml
jobs:
  catalog-rust:
    steps:
      # ... existing ...
      - name: D12-bis integration tests
        run: |
          cargo test -p efe-server --test 'd12bis_catalog_*' \
              -- --test-threads 4
        timeout-minutes: 15

      - name: D12-bis concurrency tests (SLO D12-CT-6)
        run: |
          cargo test -p efe-server --test 'd12bis_catalog_concurrency' \
              -- --test-threads 1 --nocapture
        timeout-minutes: 10

  catalog-bench:
    name: ghz benchmark — D12 RPCs
    runs-on: ubuntu-22.04
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable
        with: { toolchain: "1.85.0" }

      - name: Build efe-server release
        run: cargo build -p efe-server --release

      - name: Start efe-server in background
        run: |
          export EFE_AUTH_ENABLED=false
          export EFE_DEV_TENANT=tenant-bench
          export EFE_CATALOG_BACKEND=memory
          ./target/release/efe-server > /tmp/server.log 2>&1 &
          sleep 3

      - name: Install ghz
        run: |
          curl -sSL https://github.com/bojand/ghz/releases/download/v0.118.0/ghz-linux-x86_64.tar.gz \
              | tar -xz -C /tmp
          sudo install -m 0755 /tmp/ghz /usr/local/bin/

      - name: Bench RegisterCatalog
        run: |
          ghz --insecure \
              --proto protos/efe/v1/catalog.proto \
              --import-paths protos \
              --call efe.v1.catalog.CatalogService.RegisterCatalog \
              --concurrency 50 --total 1000 \
              --data-file bench/data/register_catalog.json \
              --format json -o /tmp/bench-register.json \
              127.0.0.1:50051

      - name: Parse bench reports + emit SLO measurements
        run: |
          python bench/load/parse_ghz_report.py \
              --reports /tmp/bench-*.json \
              --bench-out /tmp/bench-results.json \
              --summary-out /tmp/bench-summary.md \
              --p99-budget-ms 200 \
              --error-budget-ratio 0.01 \
              --throughput-min-rps 50.0
```

Les SLO measurements remontent dans `bench_baselines/v0.1.13b/baseline.json`
(la baseline post-D12-bis), passant les 7 D12-CT de SKIPPED à
PASSED/FAILED.

---

## 15. Helm extensions — `catalog.backend` + `catalog.policy.*`

### 15.1 Templates additions

```yaml
# deploy/helm/efe/templates/configmap.yaml — D12-bis additions

data:
  EFE_CATALOG_BACKEND: {{ .Values.catalog.backend | quote }}
  EFE_CATALOG_MAX_NAMES_PER_TENANT:    {{ .Values.catalog.policy.maxNamesPerTenant | quote }}
  EFE_CATALOG_MAX_VERSIONS_PER_NAME:   {{ .Values.catalog.policy.maxVersionsPerName | quote }}
  EFE_CATALOG_MAX_CLASSES_PER_CATALOG: {{ .Values.catalog.policy.maxClassesPerCatalog | quote }}
  EFE_CATALOG_MAX_SENTENCES_PER_CLASS: {{ .Values.catalog.policy.maxSentencesPerClass | quote }}
{{- if and (eq .Values.catalog.backend "postgres") .Values.catalog.postgres.enabled }}
  # D14 — postgres backend env
  EFE_CATALOG_PG_URL:          {{ .Values.catalog.postgres.url | quote }}
  EFE_CATALOG_PG_POOL_SIZE:    {{ .Values.catalog.postgres.poolSize | quote }}
  # ... (D14)
{{- end }}
```

### 15.2 Helm test : RPC reachability

```yaml
# deploy/helm/efe/templates/tests/test-catalog-rpc.yaml

{{- if .Values.catalog.helmTestEnabled | default true }}
apiVersion: v1
kind: Pod
metadata:
  name: {{ include "efe.fullname" . }}-test-catalog-rpc
  annotations:
    "helm.sh/hook": test
spec:
  restartPolicy: Never
  containers:
    - name: test
      image: fullstorydev/grpcurl:v1.8.7
      command:
        - sh
        - -c
        - |
          set -e
          grpcurl -plaintext {{ include "efe.fullname" . }}:50051 list \
              | grep -q "efe.v1.catalog.CatalogService" \
              || (echo "FAIL : CatalogService not exposed" >&2; exit 1)
          echo "OK : CatalogService is reachable"
{{- end }}
```

---

## 16. Migration prep D14 — postgres-ready par construction

### 16.1 Pourquoi le trait protège déjà

Le trait `CatalogRegistry` (D12) a été designé en pensant
postgres-ready :

* Toutes les méthodes sont `async` (postgres = network IO).
* Les retours utilisent des structs domain-pure, pas de proto ou
  d'opaque DB rows.
* La concurrency est encapsulée derrière le trait — `InMemoryRegistry`
  utilise `RwLock`, `PostgresCatalogRegistry` utilisera SELECT
  FOR UPDATE.

D12-bis hérite de cette propriété : le handler ne touche jamais
à la storage layer directement.

### 16.2 Ce que D14 changera

| Composant | D12-bis (memory) | D14 (postgres) |
|---|---|---|
| Choix du backend | `match cfg.catalog.backend { Memory => InMemoryRegistry::new() }` | `match cfg.catalog.backend { Memory => ..., Postgres => PostgresCatalogRegistry::new(pg_pool).await? }` |
| Wire dans Server | identique | identique (handler ne change pas) |
| Tests intégration | run avec InMemoryRegistry | run avec PostgresCatalogRegistry contre testcontainer |
| SLOs D12-CT-1..7 | mesurés in-memory | mesurés en postgres avec connection pool |
| Recovery sur restart | perdu | persisté |

### 16.3 Changements migration-friendly inclus dans D12-bis

* `CatalogConfig.postgres` field (optional) **déjà déclaré** dans
  D12-bis, default `None`. D14 le rend obligatoire quand
  `backend=postgres`.
* Helm values `catalog.postgres.*` **déjà déclarées**, `enabled:
  false` par défaut. D14 active.
* Le hander construit son `CatalogPolicy::from_config(...)` à
  partir de la config (pas hardcodé). D14 hérite.

### 16.4 État volatile et garanties post-restart en D12-bis

Tant que D14 n'est pas livré, **chaque** redémarrage de
`efe-server` perd toutes les catalogs registered. Le helm chart
documente clairement :

```yaml
catalog:
  backend: memory     # WARNING: state is lost on every pod restart.
                      # For production, set backend=postgres (D14+).
```

---

## 17. Threat model — 10 vecteurs handler-spécifiques

10 vecteurs HAZOP propres au translation layer. Chacun avec
likelihood, impact, mitigation, test.

### T-12bis-1 — Cross-tenant data leak via mal-formed `from_key`

* **Likelihood** : élevé si le handler ne parse pas le tenant
  avant de valider.
* **Impact** : critique (un tenant peut introspecter les chaînes
  de catalog d'un autre).
* **Mitigation** : `parse_canonical_key` + `assert_tenant_match`
  AVANT tout call à la registry.
* **Test** : `cross_tenant_get_catalog_returns_permission_denied`
  (cf. § 6.6).

### T-12bis-2 — Internal error message leaks DB schema / IPs

* **Likelihood** : moyen (Rust `Display` des erreurs internes
  inclut souvent le contexte).
* **Impact** : moyen (surface d'attaque révélée).
* **Mitigation** : `Internal(...)` → `Status::internal("internal
  server error")` ; le message original reste côté serveur.
* **Test** : `internal_error_does_not_leak_to_client` (cf. § 8.4).

### T-12bis-3 — Audit log integrity gap (no HMAC)

* **Likelihood** : faible (logs append-only via tracing).
* **Impact** : élevé pour compliance (audit log répudiable).
* **Mitigation** : déféré à D14-bis (HMAC signing). En attendant,
  doc dit "compliance limited" pour les events <= v0.1.14.
* **Test** : N/A — couvert par D14-bis.

### T-12bis-4 — Span attribute leakage of PII

* **Likelihood** : moyen (un dev pourrait ajouter `text` ou
  `email` à un span attribute).
* **Impact** : critique (PII dans Tempo).
* **Mitigation** : `spans::FORBIDDEN_ATTR_PREFIXES` + OTel
  collector allowlist (D13).
* **Test** : `test_d13_phase2_event_emitters.py::test_forbidden_attr_includes_pii_terms`.

### T-12bis-5 — Auth interceptor not configured → handler called directly

* **Likelihood** : faible (build-time).
* **Impact** : critique (pas d'auth = bypass).
* **Mitigation** : `AuthContext::from_request` retourne
  `Status::unauthenticated` si extension absente.
* **Test** : `service_without_auth_interceptor_returns_unauthenticated`.

### T-12bis-6 — Quotas bypass via concurrent registers

* **Likelihood** : faible (le trait sérialise déjà).
* **Impact** : moyen (quotas dépassés).
* **Mitigation** : la trait check les quotas SOUS le write lock.
  Aucun race possible.
* **Test** : `concurrent_registers_respect_versions_quota`.

### T-12bis-7 — Idempotency cache key collision across tenants

* **Likelihood** : faible (D11 cache est keyed par `(tenant,
  X-Idempotency-Key)`).
* **Impact** : critique si broken (un tenant peut récupérer la
  réponse d'un autre).
* **Mitigation** : pin tested by D11 isolation suite.
* **Test** : `idempotency_cache_isolated_per_tenant` (D11
  existant).

### T-12bis-8 — Reflection exposes internal proto

* **Likelihood** : faible (reflection est on-by-default).
* **Impact** : moyen (info disclosure).
* **Mitigation** : reflection n'expose que des proto publics ;
  rien d'interne.
* **Test** : `reflection_lists_only_documented_services`.

### T-12bis-9 — `include_compiled=true` returns large payloads

* **Likelihood** : moyen (cardinality des compiled blobs).
* **Impact** : moyen (network saturation, slow client).
* **Mitigation** : pour D12-bis, `compiled` est toujours None
  (embed pool D14) ; max payload borné. D14 ajoute un cap +
  pagination.
* **Test** : `get_with_include_compiled_returns_empty_for_d12bis`.

### T-12bis-10 — Audit emit failure swallows the request

* **Likelihood** : faible (audit emit est tracing — fire-and-forget).
* **Impact** : faible (catalog state committed, audit miss).
* **Mitigation** : audit emit ne return pas `Result` ; les pertes
  sont métriques (`audit_emission_failures_total`, à ajouter en
  D14-bis).
* **Test** : `register_succeeds_even_when_audit_emit_fails`.

---

## 18. 12 décisions de conception clés

### D-12bis-1 — Single struct `CatalogServiceImpl` (vs free functions)

**Choix** : un struct cloneable derrière `Arc<>` deps.
**Rationale** : DI-friendly + mock-friendly + D14-ready (swap impl).

### D-12bis-2 — Conversions in a separate sub-module

**Choix** : `services/catalog/conversions.rs` séparé.
**Rationale** : 6 helpers × 20 lignes chacun = 120 lignes. Garder
le handler propre (~ 600 lignes) et tester les conversions
isolément (20 tests).

### D-12bis-3 — Read RPCs ne génèrent PAS d'audit event

**Choix** : `Get`, `List`, `ListVersions` n'émettent pas
`audit_events_total` ni `AuditEvent` log.
**Rationale** : (a) les reads sont fréquents (1000× plus que
writes) ; émettre des audits explose Loki. (b) le `RequestEvent`
de l'interceptor metrics suffit pour l'audit forensique. (c)
compliance ne demande l'audit que sur les MUTATIONS.
**Conséquence** : l'alerte `D13ObsAuditPrincipalUnknown` (D13)
ne couvre que les writes, pas les reads. C'est intentionnel.

### D-12bis-4 — `principal_present` label : bool string vs metric dédié

**Choix** : label `principal_present="true"|"false"` sur le
counter existant `efe_audit_events_total`.
**Rationale** : cardinalité × 2 (4 200 séries → 8 400 séries) =
acceptable. Alternative (metric dédié `audit_unknown_principal_total`)
double la maintenance pour aucun gain.
**Audit fix** : C-1 D13.

### D-12bis-5 — `diff_summary` calculé dans le handler (pas dans la trait)

**Choix** : le handler computes `diff_summary` à partir du
`EvolveOp` (c'est la source authoritative ; le trait n'a pas à
le ré-exposer).
**Rationale** : keep the trait minimal, pas de redundance, pas
de race entre fetch + diff.
**Conséquence** : pour RECOMPILE, `diff_summary` ne contient que
les fingerprints (pas le set des classes — qui n'a pas changé).

### D-12bis-6 — Internal errors strip the message before client return

**Choix** : `Status::internal("internal server error")` toujours.
**Rationale** : threat T-12bis-2 (info disclosure).
**Conséquence** : doc operator que pour debug il faut consulter
les logs serveur via `request_id` (corrélation Loki).

### D-12bis-7 — Backend selector via env var (pas Cargo feature)

**Choix** : `EFE_CATALOG_BACKEND=memory|postgres` runtime.
**Rationale** : un seul binaire pour les deux modes ; déploiement
plus simple.
**Conséquence** : v0.1.13 build pourrait théoriquement supporter
postgres, mais l'impl rejected au startup. v0.1.14 unlock.

### D-12bis-8 — Compiled anchors restent `None` en D12-bis

**Choix** : `include_compiled=true` retourne `compiled=[]` en
v0.1.13b.
**Rationale** : pas d'embed pool = pas de bytes à retourner.
Évite d'exposer un champ "compiled mais vide" qui pourrait
confuser les clients.
**D14 décision** : embed pool wirable, compiled blobs persistés
dans postgres (`bytea` ou S3 référencé).

### D-12bis-9 — `assert_tenant_match` sur TOUS les RPCs

**Choix** : pas d'exemption.
**Rationale** : zero trust. Un tenant ne peut pas appeler
ListCatalogs d'un autre tenant via un cross-tenant token.
**Conséquence** : un opérateur SaaS qui veut introspecter tous
les tenants utilise un endpoint séparé (admin:catalog:cross-tenant
scope, deferred D15+).

### D-12bis-10 — Helm test = grpcurl reachability

**Choix** : `helm test` utilise `grpcurl list` pour confirmer
que le service est exposé.
**Rationale** : preuve la plus simple ; pas besoin de monter un
JWT valide pour le test.
**Conséquence** : ne teste pas la chaîne complète register
→ list → get. Ce test-là est intégration tonic en CI, pas helm
test.

### D-12bis-11 — Layers ordering : auth avant idempotency

**Choix** : auth → idempotency → ratelimit → service.
**Rationale** : idempotency cache est keyed par `(tenant,
key)` — donc tenant doit être extrait en amont. Ratelimit
applique post-idempotency pour pas pénaliser les retries.
**Conséquence** : l'auth interceptor doit être idempotent
lui-même (ne pas modifier le request body) — déjà le cas.

### D-12bis-12 — Spans use `tracing::info_span!` (sans dep OTel)

**Choix** : utiliser le tracing macro standard.
**Rationale** : pas de nouvelle dep en D12-bis. Quand D13-bis
active OTel SDK, le `tracing-opentelemetry` layer convertit
automatiquement en OTel spans — zero code change.
**Conséquence** : en v0.1.13, les spans sont visibles dans le
console subscriber + JSON layer. Pas de Tempo correlation
jusqu'à D13-bis.

---

## 19. 10 risques identifiés et mitigations

### R-12bis-1 — Stub regen casse une cargo build qui marchait

* **Description** : `tonic-build` regenerate les stubs pour 4
  fichiers proto. Conflit de nom symbol entre admin.proto et
  catalog.proto ?
* **Mitigation** : namespace strict (chaque proto a son `package`).
* **Owner** : engineering.

### R-12bis-2 — Span macro ne compile pas sans tracing-opentelemetry

* **Description** : `info_span!` de `tracing` est OK, mais si on
  ajoute des span attributes que l'OTel layer rejette, panic en
  prod.
* **Mitigation** : utiliser uniquement les `ATTR_*` constants de
  `spans.rs`.
* **Owner** : engineering.

### R-12bis-3 — In-memory state lost on every test

* **Description** : `InMemoryRegistry::new()` reset chaque
  fixture. Tests qui assument state shared échouent.
* **Mitigation** : fixture pattern (`tests/d12bis_helpers.rs`)
  qui partage un `Arc<dyn CatalogRegistry>` quand needed.
* **Owner** : engineering.

### R-12bis-4 — Conversion errors not surfaced as InvalidArgument

* **Description** : si une conversion utilise `.unwrap()` au lieu
  de `.ok_or_else(...)`, le serveur panic et tonic retourne
  `Internal`.
* **Mitigation** : audit C-2 du fichier conversions ; clippy
  `unwrap_used` deny.
* **Owner** : engineering ; review systematic.

### R-12bis-5 — Reflection descriptor too large

* **Description** : 4 protos × N messages → file descriptor set
  pèse plusieurs MB.
* **Mitigation** : standard tonic supports it ; aucun seuil
  atteint.
* **Owner** : platform.

### R-12bis-6 — JWT principal claim is missing in production

* **Description** : un IdP non-EFE ne stamp pas `sub`. Le handler
  émet `principal_present="false"` à 100 %, alerte
  D13ObsAuditPrincipalUnknown fire en permanence.
* **Mitigation** : doc l'IdP requirement dans
  `docs/howto/onboarding-as-a-tenant.md` (manque actuellement —
  voir l'analyse MVP). Auth interceptor reject avec
  `Status::unauthenticated` si `sub` est empty et auth est on.
* **Owner** : SRE + tech lead.

### R-12bis-7 — Audit emission saturates tracing buffer

* **Description** : 1000 register/sec × tracing JSON layer
  serialization = ~ 30 KB/sec output. OK pour stdout mais peut
  bloquer si Loki ingest lag.
* **Mitigation** : tracing `non_blocking` writer (M11) absorbe
  spikes ; alerte D13ObsLogPipelineLag détecte.
* **Owner** : SRE.

### R-12bis-8 — D14 swap accidentally introduces semantic divergence

* **Description** : `PostgresCatalogRegistry` implémente le
  trait mais subtle divergence (e.g. cascade delete legèrement
  différent).
* **Mitigation** : suite de tests « contract » qui run identique
  contre les deux impls.
* **Owner** : engineering D14.

### R-12bis-9 — `assert_tenant_match` coût non-négligeable sous load

* **Description** : `parse_canonical_key` allocate à chaque call.
* **Mitigation** : profiling en CI (D9 suite) ; si > 5 µs,
  introduire `Cow<str>` ou borrow-friendly parser.
* **Owner** : performance.

### R-12bis-10 — Bench `ghz` saturate l'in-memory registry

* **Description** : 1000 RPS register sur la même `(tenant,
  name)` saturate le RwLock write throughput. SLO D12-CT-1
  failed.
* **Mitigation** : bench fait 50 RPS sustained (réaliste, conforme
  à l'usage register est ops); SLO D12-CT-1 ≤ 200 ms est calibré
  pour ça.
* **Owner** : engineering bench.

---

## 20. Plan jour-par-jour (5 j wallclock)

### J1 — Foundations + stubs

* Modifier `crates/efe-server/build.rs` : ajouter `catalog.proto`.
* Vérifier le build : `cargo build -p efe-server` succeed,
  `crate::proto::v1::catalog::CatalogService` accessible.
* Créer `services/catalog.rs` skeleton + `services/catalog/conversions.rs`.
* Ajouter `pub mod catalog;` dans `services/mod.rs`.
* Test fumée : `catalog_proto_module_exposes_service_trait`.

### J2 — Conversions + error mapping

* Implémenter les 6 helpers de conversion dans `conversions.rs`.
* Tests unit conversions : 20 tests round-trip / fail-fast.
* Implémenter `catalog_error_to_status`.
* Implémenter `internal_error_does_not_leak_to_client` test.

### J3 — Handler RPCs + auth + observability

* Implémenter les 6 méthodes du trait `CatalogService` dans
  `CatalogServiceImpl`.
* Câbler `AuthContext::from_request`, `require_scope`,
  `assert_tenant_match`.
* Émettre les 4 axes observability par RPC.
* Tests unit handler : 15 tests (mocked registry).

### J4 — Wire dans Server + intégration

* Étendre `ServerConfig` avec `CatalogConfig`.
* Étendre `ServerBuilder::build()` pour wirer le service.
* Helm values + configmap additions.
* Tests d'intégration (in-process tonic) : 20 tests.
* Tests isolation : 5 tests.
* Tests concurrence : 5 tests.

### J5 — Bench + CI + closure

* Étendre `bench-catalog.yml` avec le job `catalog-bench`.
* Run bench locally → activer les SLOs D12-CT-1..7 dans
  `bench_baselines/v0.1.13b/baseline.json`.
* Helm test `test-catalog-rpc`.
* `BENCHMARKS.md` § 14.7 update + CHANGELOG entry.
* Final cross-sprint regression : 800+ Python + 117+
  efe-telemetry + 250+ efe-server, all green.

---

## 21. SLOs activés post-D12-bis

D12-bis fait passer **les 7 SLOs `D12-CT-1..7` de SKIPPED à
measured**. Reproduit ici pour référence :

| ID | Bench key | Comparator | Cible | Status pre-D12-bis | Status post-D12-bis |
|---|---|---|---|---|---|
| `SLO-D12-CT-1` | `catalog::register_p99_small` | lte | 200 ms | SKIPPED | measured |
| `SLO-D12-CT-2` | `catalog::register_p99_large` | lte | 1500 ms | SKIPPED | measured |
| `SLO-D12-CT-3` | `catalog::evolve_append_p99` | lte | 250 ms | SKIPPED | measured |
| `SLO-D12-CT-4` | `catalog::get_p99` | lte | 50 ms | SKIPPED | measured |
| `SLO-D12-CT-5` | `catalog::list_p99` | lte | 75 ms | SKIPPED | measured |
| `SLO-D12-CT-6` | `catalog::serialization_pass_pct` | gte | 100 % | SKIPPED | measured |
| `SLO-D12-CT-7` | `catalog::sha256_witness_match_pct` | gte | 100 % | SKIPPED | measured |

**SLO D12-CT-7** mérite une note : `content_sha256` round-trip
est implementé dans `InMemoryRegistry::register` via la fonction
`compute_digest()` (D12). En v0.1.13, c'est un **placeholder**
non-cryptographique. D14-bis (HMAC) le remplace par SHA-256 réel.
Pour D12-bis, le SLO mesure que le placeholder round-trip à 100
%, ce qui est trivial mais pin l'invariant.

Conséquence baseline : le measured value de D12-CT-7 sera
`1.0 = 100 %` même avec le placeholder. Pas un faux positif —
le SLO mesure exactement ce qu'il dit (round-trip match du
digest, quel qu'il soit).

---

## 22. Anti-régression D12-bis

### 22.1 Tests Rust ajoutés (cumul)

| Suite | Pre-D12-bis | Post-D12-bis | Δ |
|---|---|---|---|
| `cargo test -p efe-server --lib catalog::` | 31 | 31 | 0 |
| `cargo test -p efe-server --lib services::catalog` | 0 | ~ 35 | +35 |
| `cargo test -p efe-server --test 'd12bis_*'` | 0 | ~ 38 | +38 |
| `cargo test -p efe-server --lib` (full) | 185 | ~ 258 | +73 |

### 22.2 Tests Python existants enfin verts end-to-end

Les ~ 240 tests `test_d12_*.py` + `test_d13_*.py` étaient verts
contre des MOCKS (proto stubs only). Post-D12-bis, les exemples
réels marchent :

```bash
EFE_TOKEN=... python examples/05_catalog_admin.py
# Step [1/8] register v1 → succeed
# Step [2/8] list_catalogs → succeed
# ...
# Step [8/8] delete v1 → succeed
```

### 22.3 Helm test enfin opérable

```bash
helm test efe-acme -n tenant-acme
# OK : CatalogService is reachable                    ← D12-bis
# OK : efe-d13-01-operator-overview                   ← D13
# OK : efe-d13-02-tenant-drill-down
# OK : efe-d13-03-catalog-version-distribution
# OK : efe-d13-04-cost-attribution
# OK : efe-d13-05-audit-stream
# OK : efe-d13-06-capacity-planning
```

### 22.4 Pin du surface-stable

`scripts/tests/test_d12bis_grpc_surface_pinned.py` (NEW) :

```python
def test_catalog_service_exposes_6_rpcs():
    """Pin the wire surface: 6 RPCs, no more, no less."""

def test_d12bis_helm_test_exists():
    """The grpcurl reachability test must be wired."""

def test_register_all_d13_called_at_startup():
    """Pin the M-1 fix doesn't regress."""

def test_audit_events_total_label_set_unchanged():
    """Pin C-1 fix : 4 labels [tenant, action, op_tag, principal_present]."""
```

---

## 23. Critères d'acceptation (DoD)

D12-bis est **mergeable en main** quand :

1. **Code livré** :
   * `crates/efe-server/build.rs` étendu (catalog.proto added).
   * `crates/efe-server/src/services/catalog.rs` (~ 600 lignes).
   * `crates/efe-server/src/services/catalog/conversions.rs`
     (~ 250 lignes).
   * `crates/efe-server/src/auth/scopes.rs` étendu (`require_scope`,
     `assert_tenant_match`).
   * `crates/efe-server/src/config.rs` étendu (`CatalogConfig`).
   * `crates/efe-server/src/runtime.rs` étendu (wire service).
   * Helm values + configmap + helm test additions.
2. **Tests** : ≥ 73 nouveaux tests Rust ; cross-sprint Python
   reste vert.
3. **CI** : `bench-catalog.yml` étendu avec le job
   `catalog-bench` ; SLOs D12-CT-1..7 mesurés.
4. **Helm** : `helm lint`, `helm template`, `helm test` ✅.
5. **Docs** :
   * `BENCHMARKS.md` § 14.7 (D12-bis section).
   * `CHANGELOG.md` entry sous `[Unreleased]`.
   * `bench_baselines/v0.1.13b/baseline.json` avec mesures.
6. **Audit interne livré** :
   [`docs/audits/d12bis_audit_report.md`](../audits/d12bis_audit_report.md)
   — 7 bugs trouvés (2 CRIT, 1 HIGH, 2 MED, 2 LOW), 6 fixés,
   1 reporté (L2 hors-périmètre). Cf. closure 2026-04-27.
7. **No regression** : 800+ Python + 250+ Rust efe-server tests
   verts.
8. **Smoke E2E** : `examples/05_catalog_admin.py` complète les
   8 steps contre un `efe-server` local + JWT auth=disabled
   dev mode.

---

## 24. Out of scope & roadmap D14+

D12-bis NE livre PAS :

* **Postgres backend** — D14 (2 sem).
* **HMAC audit signing** — D14-bis (0.5 sem).
* **Embed pool + compiled anchors** — parallèle à D14 ou D15.
* **OTel SDK réel** — D13-bis (0.5 sem).
* **Auto-scaling sur métriques catalog** — D15+.
* **Cross-tenant admin scope (`admin:catalog:cross-tenant`)** —
  D15+.

L'ordre sprint suggéré post-D12-bis (cf. analyse MVP) :

1. **D14a** (1 sem) : SQL schema + `PostgresCatalogRegistry` impl.
2. **D14b** (1 sem) : Multi-réplica + concurrence + bench D12 pg
   path + backup runbook.
3. **D13-bis** (0.5 sem) : OTel SDK réel + 6 catalog spans actifs.
4. **D14-bis** (0.5 sem) : HMAC audit signing + key rotation.

---

## 25. Annexes

### Annexe A — `services/catalog.rs` skeleton complet

```rust
//! Sprint **D12-bis** — gRPC handler bridging the proto layer to
//! the catalog domain trait.
//!
//! Cf. `docs/bench/d12bis_catalog_grpc_handler_architecture.md`.

use std::sync::Arc;
use std::time::Instant;

use tonic::{Request, Response, Status};

use crate::auth::{AuthContext, require_scope, assert_tenant_match};
use crate::catalog::{
    AuditEmitter, CatalogPolicy, CatalogRegistry,
    CatalogKey, EvolveOp,
};
use crate::events::d13_streams::{
    AuditEvent, AuditAction, EvolutionEvent, CommonEnvelope, Level,
    format_rfc3339_now,
};
use crate::proto::v1::catalog as pb;
use crate::proto::v1::catalog::catalog_service_server::CatalogService;
use crate::services::catalog::conversions;
use crate::spans;

use efe_telemetry::metrics::d13_instruments::{
    audit_events_total,
    catalog_register_duration_seconds,
    catalog_evolve_duration_seconds,
    catalog_get_duration_seconds,
    catalog_list_duration_seconds,
    catalog_versions_per_name,
};

#[derive(Clone)]
pub struct CatalogServiceImpl {
    registry: Arc<dyn CatalogRegistry>,
    audit:    Arc<AuditEmitter>,
    policy:   Arc<CatalogPolicy>,
}

impl CatalogServiceImpl {
    pub fn new(
        registry: Arc<dyn CatalogRegistry>,
        audit:    Arc<AuditEmitter>,
        policy:   Arc<CatalogPolicy>,
    ) -> Self {
        Self { registry, audit, policy }
    }
}

#[tonic::async_trait]
impl CatalogService for CatalogServiceImpl {
    async fn register_catalog(
        &self,
        request: Request<pb::RegisterCatalogRequest>,
    ) -> Result<Response<pb::CatalogHandle>, Status> {
        let auth = AuthContext::from_request(&request)?;
        require_scope(auth, "admin:catalog:write")?;
        let req = request.get_ref();
        assert_tenant_match(auth, &req.tenant)?;

        let _span = tracing::info_span!(
            spans::CATALOG_REGISTER,
            tenant = %auth.tenant,
            name = %req.name,
            n_classes = req.content.as_ref()
                .map(|c| c.classes.len()).unwrap_or(0),
        ).entered();

        let start = Instant::now();
        let req = request.into_inner();
        let content = conversions::proto_content_to_trait(req.content)?;
        let metadata = build_initial_metadata(auth, &req.description, &req.labels);

        let result = self.registry
            .register(&auth.tenant, &req.name, content, metadata)
            .await;

        let result_label = if result.is_ok() { "ok" } else { "error" };
        catalog_register_duration_seconds()
            .with_label_values(&[&auth.tenant, result_label])
            .observe(start.elapsed().as_secs_f64());

        let registered = result.map_err(catalog_error_to_status)?;

        // Update versions gauge for this (tenant, name).
        if let Ok(versions) = self.registry
            .list_versions(&auth.tenant, &req.name).await
        {
            catalog_versions_per_name()
                .with_label_values(&[&auth.tenant, &req.name])
                .set(versions.len() as i64);
        }

        emit_register_audit(&self.audit, auth, &registered);

        let handle = conversions::registered_to_handle(registered)?;
        Ok(Response::new(handle))
    }

    // ... 5 other RPCs (sketch follows the same pattern) ...
}

// ====================================================================
// Audit emission helpers
// ====================================================================

fn emit_register_audit(
    audit: &AuditEmitter,
    auth:  &AuthContext,
    r:     &crate::catalog::RegisteredCatalog,
) {
    audit.emit_register(&r.key, &r.metadata);
    audit_events_total()
        .with_label_values(&[
            &auth.tenant, "register", "register",
            principal_present_label(auth),
        ])
        .inc();
    let event = AuditEvent {
        envelope: CommonEnvelope {
            timestamp:       format_rfc3339_now(),
            level:           Level::Info,
            request_id:      None,  // populated by interceptor — TODO
            trace_id:        None,
            span_id:         None,
            tenant_id:       Some(auth.tenant.clone()),
            principal:       Some(auth.principal.clone()),
            service_version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        action:         AuditAction::Register,
        resource_type:  "catalog".to_owned(),
        resource_key:   r.key.to_wire(),
        parent_key:     None,
        op_tag:         "register".to_owned(),
        content_sha256: Some(r.metadata.content_sha256.0.clone()),
        description:    Some(r.metadata.description.clone()),
        labels:         r.metadata.labels.clone(),
    };
    tracing::info!(target: "audit", json = %event.into_value(), "audit.register");
}

fn principal_present_label(auth: &AuthContext) -> &'static str {
    if auth.principal.is_empty() { "false" } else { "true" }
}

fn build_initial_metadata(
    auth: &AuthContext,
    description: &str,
    labels: &std::collections::HashMap<String, String>,
) -> crate::catalog::CatalogMetadata {
    use std::collections::BTreeMap;
    use std::time::{SystemTime, UNIX_EPOCH};
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH).unwrap_or_default()
        .as_millis() as i64;
    crate::catalog::CatalogMetadata {
        created_at_unix_ms: now_ms,
        created_by:         auth.principal.clone(),
        description:        description.to_owned(),
        parent_key:         String::new(),
        evolution_op:       String::new(),
        labels:             labels.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<BTreeMap<_,_>>(),
        content_sha256:     crate::catalog::ContentDigest(String::new()),
    }
}

// ====================================================================
// Error mapping
// ====================================================================

fn catalog_error_to_status(err: crate::catalog::CatalogError) -> Status {
    use crate::catalog::CatalogError::*;
    match err {
        NotFound(m)         => Status::not_found(m),
        InvalidArgument(m)  => Status::invalid_argument(m),
        PermissionDenied(m) => Status::permission_denied(m),
        QuotaExceeded(m)    => Status::resource_exhausted(m),
        Conflict(m)         => Status::failed_precondition(m),
        Internal(m)         => {
            tracing::error!(internal_err = %m, "catalog handler — internal error");
            Status::internal("internal server error")
        }
    }
}
```

### Annexe B — Helm values complet (D12-bis additions)

```yaml
# Audit-pinned by `test_d12bis_helm_values_block.py`.
catalog:
  backend: memory
  helmTestEnabled: true

  policy:
    maxNamesPerTenant:    100
    maxVersionsPerName:   1000
    maxClassesPerCatalog: 256
    maxSentencesPerClass: 64

  postgres:
    enabled: false
    url: ""
    poolSize: 10
    statementCacheSize: 100
    sslMode: prefer
    secretRef: ""
```

### Annexe C — Tonic test client patterns

```rust
// tests/common/mod.rs

use tonic::transport::{Channel, Endpoint};
use crate::proto::v1::catalog::catalog_service_client::CatalogServiceClient;

pub async fn catalog_client(
    addr: SocketAddr,
) -> CatalogServiceClient<Channel> {
    let endpoint = Endpoint::from_shared(format!("http://{addr}"))
        .unwrap()
        .timeout(Duration::from_secs(5));
    CatalogServiceClient::connect(endpoint).await.unwrap()
}

pub async fn catalog_client_with_token(
    addr: SocketAddr,
    token: &str,
) -> CatalogServiceClient<InterceptedService<Channel, AuthInterceptor>> {
    let inner = catalog_client(addr).await.into_inner();
    let token = token.to_owned();
    CatalogServiceClient::with_interceptor(
        inner,
        move |mut req: Request<()>| {
            req.metadata_mut().insert(
                "authorization",
                MetadataValue::from_str(&format!("Bearer {token}")).unwrap(),
            );
            Ok(req)
        },
    )
}
```

### Annexe D — Bench data sample

```json
// bench/data/register_catalog.json
{
  "tenant":      "tenant-bench",
  "name":        "support_triage",
  "version":     "v1",
  "content": {
    "embedder": {
      "family":    "sentence-transformers",
      "model":     "paraphrase-multilingual-MiniLM-L12-v2",
      "dim":       384,
      "normalize": "l2"
    },
    "classes": [
      {
        "name":                 "billing",
        "description":          "Charge / refund issues.",
        "canonical_sentences":  ["I want a refund", "my card was charged twice"],
        "color":                "#ef4444",
        "deprecated":           false
      },
      {
        "name":                 "login",
        "description":          "Authentication problems.",
        "canonical_sentences":  ["I cannot log in", "password reset failed"],
        "color":                "#3b82f6",
        "deprecated":           false
      }
    ],
    "cone_policy":  { "max_projection_distance": 0.7 },
    "thresholds":   { "confidence_high": 0.8, "confidence_medium": 0.5 }
  },
  "created_by":  "bench@efe.example",
  "description": "ghz bench fixture",
  "labels":      { "env": "bench" }
}
```

### Annexe E — `bench-catalog.yml` extension complète

```yaml
  catalog-bench:
    name: ghz benchmark — D12 RPCs (D12-bis SLO activation)
    runs-on: ubuntu-22.04
    timeout-minutes: 30
    needs: [catalog-rust]
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable
        with: { toolchain: "1.85.0" }

      - name: Build efe-server release
        run: cargo build -p efe-server --release

      - name: Start efe-server
        run: |
          export EFE_AUTH_ENABLED=false
          export EFE_DEV_TENANT=tenant-bench
          export EFE_CATALOG_BACKEND=memory
          ./target/release/efe-server > /tmp/server.log 2>&1 &
          sleep 5
          # Health check
          curl -f http://127.0.0.1:9090/metrics > /dev/null

      - name: Install ghz
        run: |
          curl -sSL \
            https://github.com/bojand/ghz/releases/download/v0.118.0/ghz-linux-x86_64.tar.gz \
            | tar -xz -C /tmp
          sudo install -m 0755 /tmp/ghz /usr/local/bin/

      - name: Bench — RegisterCatalog small (≤ 5 classes)
        run: |
          ghz --insecure \
              --proto protos/efe/v1/catalog.proto \
              --import-paths protos \
              --call efe.v1.catalog.CatalogService.RegisterCatalog \
              --concurrency 50 --total 1000 \
              --data-file bench/data/register_catalog_small.json \
              --format json -o /tmp/bench-register-small.json \
              127.0.0.1:50051

      - name: Bench — RegisterCatalog large (100 classes)
        run: |
          ghz --insecure \
              --proto protos/efe/v1/catalog.proto \
              --import-paths protos \
              --call efe.v1.catalog.CatalogService.RegisterCatalog \
              --concurrency 10 --total 200 \
              --data-file bench/data/register_catalog_large.json \
              --format json -o /tmp/bench-register-large.json \
              127.0.0.1:50051

      - name: Bench — EvolveCatalog APPEND_CLASS
        run: |
          ghz --insecure \
              --proto protos/efe/v1/catalog.proto \
              --import-paths protos \
              --call efe.v1.catalog.CatalogService.EvolveCatalog \
              --concurrency 20 --total 500 \
              --data-file bench/data/evolve_append_class.json \
              --format json -o /tmp/bench-evolve.json \
              127.0.0.1:50051

      - name: Bench — GetCatalog
        run: |
          ghz --insecure \
              --proto protos/efe/v1/catalog.proto \
              --import-paths protos \
              --call efe.v1.catalog.CatalogService.GetCatalog \
              --concurrency 100 --total 5000 \
              --data-file bench/data/get_catalog.json \
              --format json -o /tmp/bench-get.json \
              127.0.0.1:50051

      - name: Bench — ListCatalogs
        run: |
          ghz --insecure \
              --proto protos/efe/v1/catalog.proto \
              --import-paths protos \
              --call efe.v1.catalog.CatalogService.ListCatalogs \
              --concurrency 50 --total 1000 \
              --data-file bench/data/list_catalogs.json \
              --format json -o /tmp/bench-list.json \
              127.0.0.1:50051

      - name: Aggregate + emit SLO measurements (catalog::*)
        run: |
          python bench/load/parse_ghz_report.py \
              --reports /tmp/bench-*.json \
              --bench-out bench_baselines/v0.1.13b/raw.json \
              --summary-out bench_baselines/v0.1.13b/summary.md

      - name: Activate D12-CT-* SLOs in baseline
        run: |
          python scripts/baseline/activate_slos.py \
              --raw bench_baselines/v0.1.13b/raw.json \
              --baseline bench_baselines/v0.1.13b/baseline.json \
              --family D12-CT
```

### Annexe F — Smoke E2E example (post-D12-bis)

```bash
# Assumes a local efe-server running on 127.0.0.1:50051 with auth disabled.

# Generate a dev JWT (or use auth disabled mode).
export EFE_TOKEN=$(python -c "
import base64, json
header = base64.urlsafe_b64encode(b'{\"alg\":\"HS256\"}').rstrip(b'=').decode()
payload = base64.urlsafe_b64encode(json.dumps({
    'sub': 'alice@acme.com',
    'tenant_id': 'tenant-bench',
    'scope': 'admin:catalog:read admin:catalog:write',
    'aud': 'efe.v1',
    'iss': 'dev',
    'exp': 99999999999,
}).encode()).rstrip(b'=').decode()
print(f'{header}.{payload}.devsignature')
")

# Run the lifecycle example.
EFE_HOST=127.0.0.1:50051 EFE_TLS=false \
    python examples/05_catalog_admin.py

# Expected output (8 steps, all succeed) :
# Sprint D12 catalog admin demo
#   endpoint:  127.0.0.1:50051
#   insecure:  True
#   catalog:   d12_demo_1714232400_a1b2c3
#
# [1/8] register v1
#   ✓ v1 registered
#     key:           tenant-bench/d12_demo_…@v1
#     ...
# [2/8] list_catalogs
#   1 catalog(s) visible to tenant 'tenant-bench'
# ...
# [8/8] delete_catalog v1 (force=True)
#   ✓ deleted tenant-bench/d12_demo_…@v1
#
#   Demo done. Catalog d12_demo_… v2/v3 still live.
```

---

**Fin de l'architecture D12-bis. Date de production :
2026-04-27. Version : 1.0. Auteur : équipe Engineering EFE.**
