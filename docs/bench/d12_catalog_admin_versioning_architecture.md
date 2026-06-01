# Sprint D12 — Architecture : Catalog admin endpoints + versioning (proto, registry, SDK, CLI)

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : suit D11 (per-tenant isolation, primitives
> multi-tenant ; ``EfeAdminClient`` raise ``NotImplementedError``
> sur ``list_catalogs`` faute de RPC ``ListCatalogs`` dans le proto).
> D12 est **le sprint qui ferme la dette D11-bis et ouvre la
> phase 2 SaaS de governance** : centraliser les catalogs
> côté serveur avec un cycle de vie strict (register → evolve →
> delete) et un versioning monotone par ``(tenant, name)``.
> **Pré-requis** : avoir lu
> [`d11_per_tenant_isolation_architecture.md`](d11_per_tenant_isolation_architecture.md)
> (auth, tenant scoping, cone registry comme référence de design),
> et l'audit
> [`d11_isolation_report.md`](../audits/d11_isolation_report.md)
> qui établit l'état post-D11 (230/230 tests verts, 33 SLOs gated,
> ``ListCatalogs`` RPC tracé en Pending).

---

## Table des matières

1. [Contexte & mission D12](#1-contexte--mission-d12)
2. [État post-D11 — primitives existantes vs gaps](#2-état-post-d11--primitives-existantes-vs-gaps)
3. [Modèle de données : Catalog vs Cone vs ConeRef](#3-modèle-de-données--catalog-vs-cone-vs-coneref)
4. [Versioning monotone par (tenant, name)](#4-versioning-monotone-par-tenant-name)
5. [Architecture proto — extension de admin.proto](#5-architecture-proto--extension-de-adminproto)
6. [Serveur : trait CatalogRegistry + impl in-memory](#6-serveur--trait-catalogregistry--impl-in-memory)
7. [Hot-reload sémantique et garanties de cohérence](#7-hot-reload-sémantique-et-garanties-de-cohérence)
8. [Operations d'évolution — 5 ops + arbitrage](#8-operations-dévolution--5-ops--arbitrage)
9. [SDK Python — extension EfeAdminClient (clôt D11)](#9-sdk-python--extension-efeadminclient-clôt-d11)
10. [CLI — `efe-tool catalog {register,evolve,list,get,delete}`](#10-cli--efe-tool-catalog-registerevolvelistgetdelete)
11. [Threat model — 8 vecteurs catalog-spécifiques](#11-threat-model--8-vecteurs-catalog-spécifiques)
12. [Limites + quotas + cardinalité](#12-limites--quotas--cardinalité)
13. [Migration in-memory → persistent (D14+ stub)](#13-migration-in-memory--persistent-d14-stub)
14. [12 décisions de conception clés](#14-12-décisions-de-conception-clés)
15. [10 risques identifiés et mitigations](#15-10-risques-identifiés-et-mitigations)
16. [Plan jour-par-jour (5 j wallclock)](#16-plan-jour-par-jour-5-j-wallclock)
17. [SLOs ajoutés post-D12](#17-slos-ajoutés-post-d12)
18. [Anti-régression & realism check](#18-anti-régression--realism-check)
19. [Intégration CI : workflow `bench-catalog.yml`](#19-intégration-ci--workflow-bench-catalogyml)
20. [Critères d'acceptation (DoD)](#20-critères-dacceptation-dod)
21. [Out of scope & roadmap follow-ups](#21-out-of-scope--roadmap-follow-ups)
22. [Annexes](#22-annexes)

---

## 1. Contexte & mission D12

### 1.1 État post-D11

D11 a livré la couche multi-tenant (cf. § 13 BENCHMARKS) :

* SDK `efe_tool.admin` sub-package (`EfeAdminClient`,
  `CatalogRef`, `AdminError`).
* Helm Tier 2 (ResourceQuota, LimitRange, NetworkPolicy ingress
  per-tenant).
* Cost attribution via `aggregate_usage.py`.
* 7 SLOs `D11-MT-*` gated.
* 230/230 tests cross-sprint verts.
* **Audit** : 4 bugs CRITICAL/HIGH fixés ; 1 LOW reporté.

Et pourtant, **deux frottements DX critiques restent** :

| Problème | État D11 | Impact |
|----------|----------|--------|
| `EfeAdminClient.list_catalogs()` raise `NotImplementedError` | tracked Pending | Méthode publique inutilisable ; surface SDK incomplète |
| Pas de catalog admin endpoints | tenant-scoped via classify path uniquement | Application reload requiert client redeploy |
| Pas de versioning catalogs | YAML sur disk, pas server-side | Pas d'audit trail, pas de rollback |
| Pas de hot-reload | catalog modification = redeploy | Time-to-iterate trop long pour un partenaire pilote |

D12 ferme ces gaps via un **`CatalogService` côté serveur** qui
mirror le design éprouvé du `AdminService` côté cones (registry
versionné par `(tenant, task)`).

### 1.2 Mission D12 (énoncé chirurgical)

> 1. **Étendre le proto** `protos/efe/v1/admin.proto` avec un
>    `CatalogService` (5 RPCs : RegisterCatalog, EvolveCatalog,
>    GetCatalog, ListCatalogs, ListCatalogVersions, DeleteCatalog)
>    + 6 messages associés. Régénérer les stubs Python + Rust.
>
> 2. **Implémenter côté Rust** un trait `CatalogRegistry` avec :
>    * impl in-memory `MemoryCatalogRegistry` pour v0.1.12 (state
>      perdu au redémarrage — acceptable MVP).
>    * versioning strict monotone par `(tenant, name)`.
>    * tenant-scoping (D11 invariants : `assert_tenant_match`).
>    * 5 evolution ops (append_class, remove_class,
>      update_thresholds, replace_class_anchors, recompile).
>    * limites (max 100 catalogs/tenant, max 50 versions/catalog,
>      max 10 000 anchors/catalog).
>    * audit trail avant/après chaque mutation.
>
> 3. **Étendre l'auth scoping** avec deux nouvelles scopes :
>    `admin:catalog:read` et `admin:catalog:write`. Mirror du
>    design `admin:cone:*`.
>
> 4. **Wirer le `Execute` path** pour accepter une référence
>    catalog server-side via `SwarmSpec.catalog_ref` (en plus
>    du `alignment_cone` inline existant). Lookup tenant-scoped
>    + caching LRU (10 000 entrées).
>
> 5. **Activer `EfeAdminClient.list_catalogs()`** côté SDK
>    (clôt la dette D11-B1) + ajouter 5 méthodes admin
>    (`register_catalog`, `evolve_catalog`, `get_catalog`,
>    `list_catalog_versions`, `delete_catalog`).
>
> 6. **Livrer la CLI** `efe-tool catalog {register,evolve,list,get,delete}`
>    avec format YAML on-disk identique à l'existant
>    (back-compat avec `AnchorCatalog.load`).
>
> 7. **Définir 7 SLOs `D12-CT-*`** : 3 versioning + 2 hot-reload
>    + 2 SDK/CLI roundtrip. Gated en CI dès baseline post-merge
>    stable.

### 1.3 Localisation des changements

| Fichier | Modification | Sub-step |
|---------|--------------|----------|
| `protos/efe/v1/admin.proto` | **Étendu** : `CatalogService` (5 RPCs) + 6 messages | D12.1 |
| `protos/efe/v1/types.proto` | **Étendu** : `CatalogRef` (mirror de `ConeRef`) | D12.1 |
| `protos/efe/v1/efe.proto` | **Étendu** : `SwarmSpec.cone_source` ajouté variant `catalog_ref` | D12.1 |
| `clients/python/efe_tool/_proto/...` | **Régénéré** — `*_pb2.py`, `*_pb2_grpc.py`, `*_pb2.pyi` | D12.1 |
| `crates/efe-server/src/services/catalog.rs` | **Nouveau** — `CatalogServiceImpl` (gRPC handler) | D12.2 |
| `crates/efe-server/src/catalog/registry.rs` | **Nouveau** — `CatalogRegistry` trait | D12.2 |
| `crates/efe-server/src/catalog/memory.rs` | **Nouveau** — `MemoryCatalogRegistry` impl | D12.2 |
| `crates/efe-server/src/catalog/policy.rs` | **Nouveau** — quotas, limits | D12.2 |
| `crates/efe-server/src/catalog/audit.rs` | **Nouveau** — register/evolve/delete log emission | D12.2 |
| `crates/efe-server/src/auth/scopes.rs` | **Étendu** — `admin:catalog:read`/`:write` | D12.2 |
| `crates/efe-server/src/runtime.rs` | **Étendu** — wire `Arc<dyn CatalogRegistry>` to AdminService | D12.2 |
| `crates/efe-server/src/services/swarm.rs` | **Étendu** — Execute path lookup `catalog_ref` | D12.3 |
| `crates/efe-telemetry/src/metrics/recorder.rs` | **Étendu** — 4 nouveaux compteurs `efe_catalog_*` | D12.3 |
| `crates/efe-server/tests/d12_catalog_*.rs` | **Nouveau** × 5 — 35+ tests | D12.2-3 |
| `clients/python/efe_tool/admin/client.py` | **Étendu** — 5 méthodes catalog admin + activation `list_catalogs` | D12.4 |
| `clients/python/efe_tool/admin/__init__.py` | **Étendu** — exports nouveaux types | D12.4 |
| `clients/python/efe_tool/admin/types.py` | **Nouveau** — `RegisteredCatalog`, `CatalogEvolution`, `EvolveOp` enum | D12.4 |
| `clients/python/efe_tool/cli/catalog.py` | **Nouveau** — sous-commande `efe-tool catalog ...` | D12.5 |
| `clients/python/efe_tool/cli/main.py` | **Étendu** — wire `catalog` subcommand | D12.5 |
| `docs/docs/howto/catalog-admin.md` | **Nouveau** | D12.4 |
| `docs/docs/howto/catalog-evolution.md` | **Nouveau** | D12.4 |
| `docs/docs/reference/catalog-rpc.md` | **Nouveau** | D12.4 |
| `docs/docs/reference/cli.md` | **Étendu** — section catalog subcommand | D12.5 |
| `examples/05_catalog_admin.py` | **Nouveau** — register + evolve + classify | D12.5 |
| `scripts/parse_criterion_report.py` | **Modifié** — 7 SLOs `D12-CT-*` | D12.7 |
| `bench/load/parse_ghz_report.py` | **Modifié** — `RESERVED_PREFIXES` + `catalog::` | D12.7 |
| `scripts/tests/test_d12_*.py` | **Nouveau** × 3 — disjoint keys, surface, audit | D12.7 |
| `.github/workflows/bench-catalog.yml` | **Nouveau** | D12.7 |
| `bench_baselines/v0.1.12/baseline.json` | **Nouveau** | D12.8 |
| `BENCHMARKS.md § 14` | **Nouveau** | D12.8 |
| `docs/audits/d12_catalog_report.md` | **Nouveau** — sprint closure skeleton | D12.8 |
| `CHANGELOG.md` | Entrée Unreleased D12 | D12.8 |

---

## 2. État post-D11 — primitives existantes vs gaps

### 2.1 Ce qui existe déjà (à RÉUTILISER)

| Composant | Fichier | Pourquoi pertinent |
|-----------|---------|-------------------|
| `AnchorCatalog` (client-side) | `clients/python/efe_tool/catalog.py` | YAML schema + compile + cache |
| `ConeRegistry` (server-side, M9) | `crates/efe-core/src/alignment/registry.rs` | **Modèle de référence** : versioning par `(tenant, task)`, monotonic, max_versions enforcement |
| `AdminServiceImpl` (cone path) | `crates/efe-server/src/services/admin.rs` | Pattern auth + tenant-match déjà en place |
| `assert_tenant_match` (D11) | `crates/efe-server/src/auth/guard.rs` | Réutilisable pour catalog endpoints |
| `IdempotencyCache` `(tenant, request_id)` | `crates/efe-server/src/idempotency/cache.rs` | Modèle de référence pour catalog cache |
| Prometheus tenant-labelled metrics | `crates/efe-telemetry/src/metrics/recorder.rs` | Ajouter 4 compteurs catalog au même format |
| `efe_tool.AnchorCatalog.load()` | `clients/python/efe_tool/catalog.py` | YAML → in-memory déjà testé |
| `EfeAdminClient` skeleton | `clients/python/efe_tool/admin/client.py` (D11) | Étendre, ne pas reécrire |

### 2.2 Ce qui MANQUE (à LIVRER en D12)

| Manque | Impact si absent | Sub-step D12 |
|--------|------------------|---------------|
| `CatalogService` proto RPCs | Pas de serveur catalog admin | D12.1 |
| Stubs Python régénérés | SDK `list_catalogs` raise `NotImplementedError` | D12.1 |
| `CatalogRegistry` trait + impl | Pas de stockage server-side | D12.2 |
| `assert_tenant_match` sur catalog endpoints | Cross-tenant leak réel | D12.2 |
| Auth scopes `admin:catalog:*` | Pas de RBAC granulaire | D12.2 |
| Execute lookup via `catalog_ref` | Pas de hot-reload | D12.3 |
| SDK admin client extensions | Surface incomplète | D12.4 |
| CLI subcommand | DX manuelle ardue | D12.5 |
| 7 SLOs gated | Pas de mesure régression | D12.7 |

### 2.3 Pourquoi maintenant (et pas plus tard)

* La dette D11-B1 (`list_catalogs` raise) est visible dans le SDK
  publié sur PyPI.
* Phase 2 SaaS commercial requiert un **modèle de gouvernance**
  des catalogs (qui peut publier, qui peut modifier, qui peut
  rollback).
* La pression du Pilot 1.0 sur l'iteration time : un partenaire
  qui doit redéployer son app pour changer ses anchors **abandonne**
  vers un concurrent.

---

## 3. Modèle de données : Catalog vs Cone vs ConeRef

### 3.1 Distinction conceptuelle

EFE a actuellement DEUX concepts confusément similaires :

* **Catalog** (client-side) — l'objet logique : (name, version,
  classes, anchors, embedder, thresholds). Vit en YAML on-disk
  et `AnchorCatalog` in-memory.
* **Cone** (server-side, M9) — la projection mathématique
  utilisée par Mode-1 fusion : un set d'anchors + une
  max_projection_distance. Versionnée par `(tenant, task, version)`.

Aujourd'hui, le client envoie un `SwarmSpec.alignment_cone` inline
(extrait du catalog) et le serveur n'a pas de notion de "catalog".
Le client a tout, le serveur ne sait rien.

### 3.2 Le pas D12 : promouvoir Catalog au server-side

D12 introduit le concept **Catalog server-side** :

```
                    Client                          Server
                    ──────                          ──────
   Pre-D12:
   AnchorCatalog (client-side YAML)        Cone (per-RPC inline)
        │
        ├── compile + embed
        └── classify(text, catalog) ──────► AlignmentCone in spec
                                              │
                                              └── Mode-1 fusion

   Post-D12:
   AnchorCatalog (client-side YAML)        CatalogRegistry (per-tenant)
        │                                    ├── (tenant, name, v1) → Catalog
        ├── compile + embed                  ├── (tenant, name, v2) → Catalog
        ├── (option A) classify(text,        └── (tenant, name, v3) → Catalog
        │       catalog) ──────────────────► ConeFromCatalog (inlined)
        │
        └── (option B, NEW) classify(text,
                catalog_ref="acme/intent@v3") ─► server resolves
                                                  CatalogRegistry.get
                                                  └── ConeFromCatalog
```

Les deux options coexistent en v0.1.12. Migration progressive :

| Phase | Recommandation | Bénéfice |
|-------|----------------|----------|
| Existing clients | Option A (inline) | zéro changement |
| New deployments | Option B (catalog_ref) | hot-reload, audit trail |
| Phase 2 SaaS | Option B forced | governance par operator |

### 3.3 `CatalogRef` ↔ `ConeRef` mapping

```protobuf
// Existing (M9)
message ConeRef {
    string tenant = 1;
    string task = 2;        // logical purpose ("support_triage")
    uint32 version = 3;
}

// NEW (D12)
message CatalogRef {
    string tenant = 1;
    string name = 2;        // catalog name ("support_triage")
    uint32 version = 3;
}
```

**Observation** : `ConeRef.task ≈ CatalogRef.name` (sémantiquement).
On garde les deux pour la compatibilité descendante des clients
M9-era. Une catégorie d'op `EvolveCatalog::AdoptConeAlias` permet
de migrer en place.

### 3.4 Canonical key string

Format unique :
```
"<tenant>/<name>@v<N>"   — e.g. "acme/support_triage@v3"
```

Mirror du `ConeHandle.key` existant. Permet de :
* Loger le catalog dans l'audit en une seule string.
* Hash key pour cache lookup côté Execute.
* Display dans CLI / UI.

---

## 4. Versioning monotone par (tenant, name)

### 4.1 Invariants

Pour tout `(tenant, name)` :

1. **Version monotone strict croissante** : v1, v2, v3... (pas de
   v1.5, pas de "vlatest"). Match le design ConeRegistry M9.
2. **Immutabilité** : une version, une fois registered, est
   **immutable**. Modifications créent une nouvelle version.
3. **Pas de version 0** : v1 est la première (registered via
   `RegisterCatalog`).
4. **Gap-free** : pas de v1, v2, v4 — `EvolveCatalog` incrémente
   strictement de 1.
5. **Bounded count** : max 50 versions par `(tenant, name)`. Au-delà,
   `RegisterCatalog` retourne `RESOURCE_EXHAUSTED`.

### 4.2 Pourquoi ces invariants

* **Monotone strict** : audit trail lisible (v1 → v2 → v3 → ...).
* **Immutable** : protection contre "qui a changé v3 hier ?". Si
  v3 doit changer, on crée v4.
* **Gap-free** : un opérateur peut prédire la prochaine version.
* **Bounded** : empêche un attaquant `admin:catalog:write` de
  saturer la mémoire avec des milliers de versions.

### 4.3 Politique de rollback

Pas de "rollback" littéral (impossible avec immutabilité). À la
place :

* **Forward-rollback** : `RegisterCatalog` ou `EvolveCatalog` crée
  une nouvelle version qui ressemble à une version antérieure.
  Trace explicite.
* **Pin client-side** : applications consomment `catalog_ref`
  avec une version explicite (ex `acme/support_triage@v3`),
  jamais `@latest` (pas supporté en v0.1.12 — hors-scope).
* **DeleteCatalog version range** : peut delete des versions
  inutilisées (audit log conservé).

### 4.4 Politique de delete

Un opérateur peut delete :
* **Une version spécifique** : `DeleteCatalog("acme/support@v3")`.
  Si une autre version la référence (via union ou héritage), le
  serveur vérifie et :
  * `force=false` → `FAILED_PRECONDITION`
  * `force=true` → cascade delete + audit log
* **Un catalog entier** : delete toutes les versions sous un
  `(tenant, name)`. Toujours `force=true` requis.
* **Un tenant** : hors-scope (operator-side k8s namespace delete).

---

## 5. Architecture proto — extension de admin.proto

### 5.1 Nouveau service CatalogService

```protobuf
// protos/efe/v1/admin.proto — D12 extension
syntax = "proto3";
package efe.v1.admin;

import "google/protobuf/empty.proto";
import "google/protobuf/timestamp.proto";
import "efe/v1/types.proto";

// Existing service (kept).
service AdminService {
    rpc RegisterCone(RegisterConeRequest) returns (ConeHandle);
    rpc EvolveCone(EvolveConeRequest) returns (ConeHandle);
    rpc GetCone(GetConeRequest) returns (CanonicalCone);
    rpc ListVersions(ListVersionsRequest) returns (ListVersionsResponse);
    rpc DeleteCone(DeleteConeRequest) returns (google.protobuf.Empty);
    rpc BreachHistory(BreachHistoryRequest) returns (BreachHistoryResponse);
}

// NEW (D12) — separate service for clean RBAC separation.
service CatalogService {
    rpc RegisterCatalog(RegisterCatalogRequest) returns (CatalogHandle);
    rpc EvolveCatalog(EvolveCatalogRequest) returns (CatalogHandle);
    rpc GetCatalog(GetCatalogRequest) returns (CanonicalCatalog);
    rpc ListCatalogs(ListCatalogsRequest) returns (ListCatalogsResponse);
    rpc ListCatalogVersions(ListCatalogVersionsRequest) returns (ListVersionsResponse);
    rpc DeleteCatalog(DeleteCatalogRequest) returns (google.protobuf.Empty);
}
```

> **Décision** : separate service (vs. add to AdminService).
> Justification §14 (decision #6) : RBAC clean — un partenaire avec
> seulement `admin:catalog:*` ne peut pas appeler les cone RPCs.

### 5.2 Messages catalog

```protobuf
// ─── Register ──────────────────────────────────────────────────

message RegisterCatalogRequest {
    string tenant = 1;
    string name = 2;
    string version = 3;          // SemVer-shaped (e.g. "1.0.0") — informational
    CatalogContent content = 4;
    string created_by = 5;       // sub claim, audit
    string description = 6;
    map<string, string> labels = 7;
}

message CatalogHandle {
    // Canonical key "tenant/name@vN" — N is the integer version.
    string key = 1;
    CanonicalCatalog catalog = 2;
}

// ─── Content ───────────────────────────────────────────────────

message CatalogContent {
    EmbedderSpec embedder = 1;
    repeated CatalogClass classes = 2;
    ConePolicy cone_policy = 3;
    ThresholdPolicy thresholds = 4;
}

message EmbedderSpec {
    string family = 1;           // "sentence-transformers"
    string model = 2;
    uint32 dim = 3;
    string normalize = 4;        // "l2"
}

message CatalogClass {
    string name = 1;
    string description = 2;
    repeated string canonical_sentences = 3;
    string color = 4;            // optional UI hint
    bool deprecated = 5;
}

message ConePolicy {
    float max_projection_distance = 1;
}

message ThresholdPolicy {
    float confidence_high = 1;
    float confidence_medium = 2;
}

// ─── Canonical (returned form, with audit metadata) ────────────

message CanonicalCatalog {
    CatalogContent content = 1;
    CatalogMetadata metadata = 2;
    // Server-side compiled anchors (one (N, dim) tensor per class).
    // Optional in responses — set only when caller has admin:catalog:read.
    repeated CompiledClass compiled = 3;
}

message CatalogMetadata {
    google.protobuf.Timestamp created_at = 1;
    string created_by = 2;
    string description = 3;
    string parent_key = 4;       // canonical key of previous version (empty if v1)
    string evolution_op = 5;     // "register" / "append_class" / etc.
    map<string, string> labels = 6;
    string content_sha256 = 7;   // immutability witness
}

message CompiledClass {
    string class_name = 1;
    bytes anchors_blob = 2;      // (N, dim) f32 LE-encoded
    uint32 num_anchors = 3;
    string embedder_fingerprint = 4;
}

// ─── Evolve ────────────────────────────────────────────────────

message EvolveCatalogRequest {
    string from_key = 1;         // canonical key of source version
    CatalogEvolutionOp op = 2;
    CatalogMetadata metadata_override = 3;
}

message CatalogEvolutionOp {
    oneof op {
        AppendClassOp append_class = 1;
        RemoveClassOp remove_class = 2;
        UpdateThresholdsOp update_thresholds = 3;
        ReplaceClassAnchorsOp replace_class_anchors = 4;
        RecompileOp recompile = 5;
    }
}

message AppendClassOp {
    CatalogClass class = 1;
}

message RemoveClassOp {
    string class_name = 1;
}

message UpdateThresholdsOp {
    ThresholdPolicy thresholds = 1;
}

message ReplaceClassAnchorsOp {
    string class_name = 1;
    repeated string canonical_sentences = 2;
}

message RecompileOp {
    EmbedderSpec new_embedder = 1;
}

// ─── Read ──────────────────────────────────────────────────────

message GetCatalogRequest {
    string key = 1;              // canonical key
    bool include_compiled = 2;   // requires admin:catalog:read
}

message ListCatalogsRequest {
    string tenant = 1;           // MUST match JWT tenant
    int32 page_size = 2;         // default 100, max 500
    string page_token = 3;       // empty for first page
}

message ListCatalogsResponse {
    repeated CatalogRef catalogs = 1;
    string next_page_token = 2;
}

message ListCatalogVersionsRequest {
    string tenant = 1;
    string name = 2;
}

// ListVersionsResponse reused from cone path (same shape).

// ─── Delete ────────────────────────────────────────────────────

message DeleteCatalogRequest {
    string key = 1;              // canonical key
    bool force = 2;
}
```

### 5.3 Messages dans types.proto

```protobuf
// protos/efe/v1/types.proto — D12 extension

// CatalogRef — server-side catalog reference (mirror of ConeRef).
message CatalogRef {
    string tenant = 1;
    string name = 2;
    uint32 version = 3;
}
```

### 5.4 Wirage Execute → catalog_ref

```protobuf
// efe.proto — extension du oneof cone_source pour CatalogRef
message SwarmSpec {
    // ... existing fields ...
    oneof cone_source {
        AlignmentCone alignment_cone = 7;
        ConeRef alignment_cone_ref = 8;
        // NEW (D12)
        CatalogRef catalog_ref = 9;
    }
}
```

`CatalogRef` est le 3e variant du oneof. Le serveur résoud :
* `alignment_cone` → utiliser le cone inline (pré-D12).
* `alignment_cone_ref` → lookup ConeRegistry (M9, pré-D12).
* `catalog_ref` → lookup CatalogRegistry (D12) + extract anchors
  + construct AlignmentCone runtime.

### 5.5 Backward compat

* Aucun champ existant n'est renommé.
* Aucun field number existant n'est réutilisé.
* `CatalogService` est un nouveau service séparé (ajout, pas
  modification).
* Les clients pré-D12 continuent d'utiliser `alignment_cone` ou
  `alignment_cone_ref` — comportement strictement identique.
* Le SDK Python a `import` lazy : si les nouvelles classes
  catalog ne sont pas dans le pb2, l'admin client tombera back
  sur `NotImplementedError` clair.

---

## 6. Serveur : trait CatalogRegistry + impl in-memory

### 6.1 Trait `CatalogRegistry`

```rust
// crates/efe-server/src/catalog/registry.rs

use std::sync::Arc;

use crate::catalog::types::{
    CanonicalCatalog, CatalogContent, CatalogKey,
    CatalogEvolutionOp, CatalogMetadata,
};
use crate::efe_error::EfeError;

#[async_trait::async_trait]
pub trait CatalogRegistry: Send + Sync + 'static {
    /// Register a new catalog version. Atomic.
    ///
    /// Returns the assigned canonical key (with the integer
    /// version filled in by the registry — never trust the
    /// client's `version` claim).
    async fn register(
        &self,
        tenant: &str,
        name: &str,
        content: CatalogContent,
        metadata: CatalogMetadata,
    ) -> Result<CatalogKey, EfeError>;

    /// Apply an evolution op to an existing version. Returns the
    /// NEW canonical key (one greater than `from`).
    async fn evolve(
        &self,
        from: &CatalogKey,
        op: CatalogEvolutionOp,
        metadata_override: Option<CatalogMetadata>,
    ) -> Result<CatalogKey, EfeError>;

    /// Fetch a specific version.
    async fn get(
        &self,
        key: &CatalogKey,
    ) -> Result<CanonicalCatalog, EfeError>;

    /// List all (tenant, name) tuples for a tenant.
    async fn list(
        &self,
        tenant: &str,
        page_size: usize,
        page_token: Option<&str>,
    ) -> Result<(Vec<CatalogKey>, Option<String>), EfeError>;

    /// List versions for a specific (tenant, name).
    async fn list_versions(
        &self,
        tenant: &str,
        name: &str,
    ) -> Result<Vec<CatalogKey>, EfeError>;

    /// Delete one specific version.
    async fn delete(
        &self,
        key: &CatalogKey,
        force: bool,
    ) -> Result<(), EfeError>;
}
```

### 6.2 Impl `MemoryCatalogRegistry`

* Stockage : `DashMap<(tenant, name, version), Arc<CanonicalCatalog>>`.
* Index secondaire : `DashMap<(tenant, name), AtomicU32>` pour la
  prochaine version.
* Compilation server-side : à `register` ou `evolve`, le serveur
  embed les `canonical_sentences` avec son embedder local
  (sentence-transformers, dim 384). L'embedder fingerprint est
  pinné dans `CompiledClass.embedder_fingerprint`.
* Validation : avant insertion, le serveur vérifie :
  * `tenant != ""`, `name != ""`.
  * `name` matches `^[a-z][a-z0-9_-]{0,62}$`.
  * `len(classes) >= 2`.
  * Inter-class similarity (cosine entre centroids) < 0.85
    (anti-collision sémantique).
  * Embedder fingerprint match server's.
  * Quotas (cf. §12).
* Audit : log line emit AVANT mutation, success/fail logged AFTER.

### 6.3 Limites + quotas

```rust
// catalog/policy.rs
pub struct CatalogPolicy {
    pub max_catalogs_per_tenant: u32,        // 100
    pub max_versions_per_catalog: u32,       // 50
    pub max_classes_per_catalog: u32,        // 100
    pub max_anchors_per_class: u32,          // 100
    pub max_anchors_total_per_catalog: u32,  // 10_000
    pub max_class_name_len: u32,             // 64
    pub max_canonical_sentence_len: u32,     // 1024
}

impl Default for CatalogPolicy {
    fn default() -> Self {
        Self {
            max_catalogs_per_tenant: 100,
            max_versions_per_catalog: 50,
            max_classes_per_catalog: 100,
            max_anchors_per_class: 100,
            max_anchors_total_per_catalog: 10_000,
            max_class_name_len: 64,
            max_canonical_sentence_len: 1024,
        }
    }
}
```

Tunable via env vars `EFE_CATALOG_MAX_*`.

### 6.4 Audit log emission

Chaque mutation (register/evolve/delete) loge :

```json
{
  "ts": "2026-04-27T14:20:00Z",
  "level": "INFO",
  "msg": "catalog.registered",
  "tenant_id": "acme",
  "catalog_key": "acme/support_triage@v3",
  "op": "register",
  "actor_sub": "service-account-acme-ops",
  "before_sha256": null,
  "after_sha256": "abc123...",
  "num_classes": 4,
  "num_anchors_total": 16,
  "embedder_fingerprint": "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2@384",
  "labels": {"env": "prod", "stage": "dev"}
}
```

Pour `evolve` : `before_sha256` = SHA256 de la version from,
`after_sha256` = SHA256 de la nouvelle version. Permet de prouver
qu'aucune mutation in-place n'a eu lieu.

---

## 7. Hot-reload sémantique et garanties de cohérence

### 7.1 Le problème

Un partenaire publie `catalog@v3`, son app classifie avec
`catalog_ref="acme/support@v3"`. Que se passe-t-il à v3 → v4 ?

Trois sémantiques possibles :

| Sémantique | Comportement | Trade-off |
|------------|-------------|-----------|
| **Sticky** | v3 toujours servi tant qu'il existe ; client doit explicitement passer v4 | Pas de hot-reload mais predictable |
| **Latest-floating** | `catalog_ref` sans version pin → le serveur choisit le max(version) | Hot-reload mais comportement émergent |
| **Hybrid** | Version explicite obligatoire ; client s'occupe du `bump` | Predictable + opt-in hot-reload |

D12 retient **Hybrid** (hardline) :

* Le client pin une version dans `catalog_ref.version` (≥ 1).
* Pour upgrader, le client **change explicitement** la version.
* Pas de syntaxe `@latest` côté D12 (tracked roadmap D14+).
* Hot-reload = "le serveur a la nouvelle version dès register, le
  client la lit dès qu'il change son code/config".

**Justification** : production code doit être deterministic.
Latest-floating cause des surprises (un changement catalog côté
ops fait varier les classifications côté app sans signal).

### 7.2 Cache côté serveur

Le serveur Execute path lookup via :

```rust
// crates/efe-server/src/services/swarm.rs (extension D12)

let cone = match spec.cone_source {
    Some(ConeSource::AlignmentCone(c)) => Cone::from_proto(c)?,
    Some(ConeSource::AlignmentConeRef(r)) => {
        cone_registry.get(&r.into())?.cone
    },
    Some(ConeSource::CatalogRef(r)) => {
        // D12 — catalog_ref lookup with LRU caching
        let key = CatalogKey::from(r);
        let canonical = catalog_cache.get_or_lookup(&key, |k| {
            catalog_registry.get(k).await
        })?;
        // Build AlignmentCone from compiled class anchors.
        Cone::from_canonical_catalog(canonical)?
    },
    None => return Err(EfeError::InvalidArgument("no cone source")),
};
```

LRU cache `CatalogCache` :
* Capacity : 10 000 catalog versions in-memory.
* TTL : 0 (no expiration ; immutable versions).
* Eviction : LRU on capacity overflow.
* Invalidation : `DeleteCatalog` triggers explicit eviction.

### 7.3 Cohérence read-after-write

Garantie : après `RegisterCatalog(...)` retourne OK, le prochain
`GetCatalog`/`ListCatalogs`/`Execute(catalog_ref=...)` voit la
nouvelle version.

Implémentation :
* In-memory : `DashMap` insert atomique → visible immédiatement.
* Pour Phase 2 (DB-backed) : write fence + `commit` sync avant
  return.

### 7.4 Cohérence client-side cache

Le SDK `EfeAdminClient.get_catalog` peut cacher :
* TTL `cache_ttl_s` : default 60 s.
* Bypass : `bypass_cache=True` pour forcer un round-trip.

Risk : entre deux `get_catalog` d'un même client, une opération
d'evolve a eu lieu. Le client voit une version stale jusqu'à
expiration TTL. Acceptable dans Phase 2 — Phase 3 ajoutera un
mécanisme de notification (server-side stream `WatchCatalog`).

---

## 8. Operations d'évolution — 5 ops + arbitrage

### 8.1 Les 5 ops

| Op | Sémantique | Re-embed nécessaire ? |
|----|-----------|:--------------------:|
| `AppendClass` | Ajoute une nouvelle classe à la fin | OUI (juste les nouveaux anchors) |
| `RemoveClass` | Retire une classe (fail si dernière) | NON |
| `UpdateThresholds` | Change `confidence_high/medium` | NON |
| `ReplaceClassAnchors` | Re-embed une classe existante | OUI (juste cette classe) |
| `Recompile` | Re-embed tout avec un nouvel embedder | OUI (tous les anchors) |

### 8.2 Coût d'embedding

Chaque embed-ing :
* `paraphrase-multilingual-MiniLM-L12-v2` : ~30 ms par batch de
  ~10 sentences sur CPU server.
* 100 classes × 5 sentences chacune = 500 sentences = ~50 batches
  = ~1.5 s.

→ `Recompile` peut prendre quelques secondes. RPC timeout
recommandé = 30 s.

### 8.3 Sémantique de version après evolve

Toute op `EvolveCatalog` retourne une **nouvelle version**
(monotone +1) avec :
* `metadata.parent_key` = canonical key de from.
* `metadata.evolution_op` = e.g. "append_class:billing".
* `metadata.content_sha256` = nouveau hash.

Si `EvolveCatalog` échoue (validation, quota), aucune nouvelle
version n'est créée — atomicité garantie.

### 8.4 Arbitrage : pourquoi pas plus d'ops ?

Considérées et rejetées :

| Op rejetée | Raison |
|-----------|--------|
| `RenameClass` | Casse les apps qui consomment le label par valeur |
| `MergeClasses` | Sémantique floue (anchors fusionnés ou union ?) → laisser au client |
| `SplitClass` | Idem |
| `BulkUpdateAnchors` | Composé de N `ReplaceClassAnchors` |
| `Sort/Reorder classes` | Pas de sémantique ordonnée |

Le set de 5 couvre 95 % des besoins observés. Plus = surface plus
grande à tester.

---

## 9. SDK Python — extension EfeAdminClient (clôt D11)

### 9.1 Nouvelles méthodes

```python
# clients/python/efe_tool/admin/client.py — extension D12

class EfeAdminClient:
    # ... existing methods (D11) ...

    # ─── Catalog admin (D12 NEW) ──────────────────────────

    def list_catalogs(
        self,
        *,
        page_size: int = 100,
        page_token: str | None = None,
    ) -> list[CatalogRef]:
        """List catalogs visible to the authenticated tenant.

        v0.1.12 — ListCatalogs RPC active (closes D11-B1 debt).
        """
        from efe_tool._proto.efe.v1 import admin_pb2
        stub = self._catalog_stub()  # NEW : separate CatalogService stub
        with self._grpc_call("list_catalogs"):
            req = admin_pb2.ListCatalogsRequest(
                tenant=self._tenant_from_jwt(),  # extracted from JWT
                page_size=page_size,
                page_token=page_token or "",
            )
            resp = stub.ListCatalogs(
                req, metadata=self._metadata(), timeout=self._timeout_s,
            )
        return [
            CatalogRef(
                tenant=c.tenant,
                name=c.name,
                version=c.version,
                num_classes=int(c.num_classes),
                num_anchors=int(c.num_anchors),
            )
            for c in resp.catalogs
        ]

    def register_catalog(
        self,
        *,
        tenant: str,
        name: str,
        version: str,
        content: dict,                # parsed YAML
        description: str = "",
        labels: dict[str, str] | None = None,
    ) -> RegisteredCatalog:
        """Register a new catalog version.

        Server filters by JWT tenant: `tenant` must match.
        """
        ...

    def evolve_catalog(
        self,
        *,
        from_key: str,                # "tenant/name@vN"
        op: EvolveOp,                 # enum + payload
        description: str = "",
    ) -> RegisteredCatalog:
        """Apply an evolution op, returning the new version."""
        ...

    def get_catalog(
        self,
        key: str,
        *,
        include_compiled: bool = False,
    ) -> RegisteredCatalog:
        """Fetch a specific version by canonical key."""
        ...

    def list_catalog_versions(
        self,
        *,
        tenant: str,
        name: str,
    ) -> list[str]:
        """List all versions for a (tenant, name)."""
        ...

    def delete_catalog(
        self,
        key: str,
        *,
        force: bool = False,
    ) -> None:
        """Delete a specific catalog version."""
        ...
```

### 9.2 Nouveaux types — `admin/types.py`

```python
# clients/python/efe_tool/admin/types.py
"""Sprint D12 — admin client types for catalog operations."""
from __future__ import annotations

import enum
from dataclasses import dataclass
from datetime import datetime
from typing import Any


@dataclass(frozen=True)
class RegisteredCatalog:
    """Server-side catalog version + audit metadata.

    Attributes:
        key: Canonical key "tenant/name@vN".
        tenant: owning tenant.
        name: catalog name.
        version: integer version.
        content: full YAML-parseable catalog body.
        metadata: register/evolve audit metadata.
        compiled: optional per-class compiled anchors (only when
            include_compiled=True at fetch time).
    """
    key: str
    tenant: str
    name: str
    version: int
    content: dict[str, Any]
    metadata: CatalogAuditMetadata
    compiled: list[CompiledClassRef] | None = None


@dataclass(frozen=True)
class CatalogAuditMetadata:
    created_at: datetime
    created_by: str
    description: str
    parent_key: str           # empty for v1
    evolution_op: str          # "register" | "append_class" | etc.
    labels: dict[str, str]
    content_sha256: str


@dataclass(frozen=True)
class CompiledClassRef:
    class_name: str
    num_anchors: int
    embedder_fingerprint: str


class EvolveOp(enum.Enum):
    """Discriminated union of evolution operations."""
    APPEND_CLASS = "append_class"
    REMOVE_CLASS = "remove_class"
    UPDATE_THRESHOLDS = "update_thresholds"
    REPLACE_CLASS_ANCHORS = "replace_class_anchors"
    RECOMPILE = "recompile"
```

Helpers builder for each op (avoid raw dict construction at
caller side) :

```python
def append_class(
    *,
    name: str, description: str = "",
    canonical_sentences: list[str],
    color: str | None = None,
) -> dict: ...

def update_thresholds(
    *, confidence_high: float, confidence_medium: float,
) -> dict: ...

# etc.
```

### 9.3 Surface freeze impact

`__all__` étendu :
* `RegisteredCatalog` (NEW)
* `CatalogAuditMetadata` (NEW)
* `CompiledClassRef` (NEW)
* `EvolveOp` (NEW)

Plus une activation : `EfeAdminClient.list_catalogs` ne raise
plus `NotImplementedError`. Test régression D11 audit
(`test_list_catalogs_raises_not_implemented`) à update — devient
`test_list_catalogs_returns_list`.

---

## 10. CLI — `efe-tool catalog {register,evolve,list,get,delete}`

### 10.1 Sous-commande `register`

```bash
$ efe-tool catalog register \
    --file my-catalog.yaml \
    --tenant acme \
    --version 1.0.0 \
    --description "Initial release"
Registered: acme/support_triage@v1
  4 classes, 16 anchors total
  embedder: sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2@384
  audit: created_by=service-account-ops, sha256=abc123...
```

### 10.2 Sous-commande `evolve`

```bash
$ efe-tool catalog evolve \
    --from acme/support_triage@v1 \
    --append-class --class-file new-class.yaml
Evolved: acme/support_triage@v1 → acme/support_triage@v2
  5 classes (was 4), 21 anchors (was 16)

$ efe-tool catalog evolve \
    --from acme/support_triage@v2 \
    --update-thresholds --confidence-high=0.7 --confidence-medium=0.5
Evolved: acme/support_triage@v2 → acme/support_triage@v3
  thresholds: confidence_high 0.6→0.7 ; confidence_medium 0.5→0.5
```

### 10.3 Sous-commandes `list`, `get`, `delete`

```bash
$ efe-tool catalog list
acme/support_triage   3 versions  (v1, v2, v3)
acme/billing_extract  1 version   (v1)

$ efe-tool catalog get acme/support_triage@v3 --output json
{
  "key": "acme/support_triage@v3",
  ...
}

$ efe-tool catalog delete acme/support_triage@v1 --force
Deleted: acme/support_triage@v1
```

### 10.4 Format YAML on-disk

Reuse format `AnchorCatalog` (D8/D11) — pas de changement client.
Le CLI parse le YAML, l'envoie comme `CatalogContent` au serveur.

---

## 11. Threat model — 8 vecteurs catalog-spécifiques

| # | Vecteur | Mitigation D12 | Test |
|--:|---------|----------------|------|
| 1 | Tenant A registers catalog "acme/foo" | Server validates `tenant == JWT.tenant_id` | `d12_catalog_cross_tenant.rs::register_foreign_tenant` |
| 2 | Anonymous user registers catalog | Reject UNAUTHENTICATED before any registry op | inherited from D11 auth guard |
| 3 | Tenant A reads tenant B's catalog | Server filters by JWT tenant on Get/List | `d12_catalog_cross_tenant.rs::get_foreign_catalog` |
| 4 | Catalog version monotonic violation | Registry refuses non-monotonic evolve | `d12_catalog_versioning.rs::monotonic_strict` |
| 5 | Catalog quota DOS (register 10000 versions) | Reject at `max_versions_per_catalog` | `d12_catalog_policy.rs::version_quota` |
| 6 | Catalog content integrity (mid-flight tamper) | `content_sha256` in metadata, audit log | `d12_catalog_audit.rs::integrity_witness` |
| 7 | Embedder fingerprint mismatch | Server rejects if catalog declares mismatched embedder | `d12_catalog_register.rs::embedder_fp_mismatch` |
| 8 | DeleteCatalog cascade explosion | `force=true` required for cascade ; audit log | `d12_catalog_delete.rs::cascade_force_required` |

---

## 12. Limites + quotas + cardinalité

### 12.1 Limits per tenant

| Resource | Limit | Behavior at limit |
|----------|------:|-------------------|
| catalogs (distinct names) | 100 | RegisterCatalog → RESOURCE_EXHAUSTED |
| versions per catalog | 50 | EvolveCatalog → RESOURCE_EXHAUSTED |
| classes per catalog | 100 | content validation fails |
| anchors per class | 100 | content validation fails |
| anchors total per catalog | 10 000 | content validation fails |
| total memory budget | tenant-wide ~50 MB | RESOURCE_EXHAUSTED |

### 12.2 Cardinality impact on Prometheus

Le D11 ajoute `tenant` label sur tous les compteurs. D12 ajoute :

```
efe_catalog_register_total{tenant, op}
efe_catalog_evolve_total{tenant, op}
efe_catalog_get_total{tenant}
efe_catalog_delete_total{tenant, force}
efe_catalog_storage_bytes{tenant}
efe_catalog_count{tenant}
efe_catalog_version_count{tenant, name}    # cardinality concern
```

`efe_catalog_version_count{tenant, name}` est l'unique métrique
qui ajoute du `name` label (1 série par catalog). Pour 100 tenants
× 100 catalogs = 10 000 séries. Acceptable (<< 1M Prometheus
budget).

Si > 100 tenants × 100 catalogs : bascule recommandée vers
ServiceMonitor `metricRelabeling` qui drop les tenants
out-of-allowlist (déjà disponible via D11.A11 cardinality
allowlist).

---

## 13. Migration in-memory → persistent (D14+ stub)

### 13.1 Cible D14

`PersistentCatalogRegistry` qui wrappe le trait avec :
* SQLite single-file backend pour les déploiements ≤ 1 pod.
* Postgres backend pour Phase 2 SaaS (HA).
* Migration : tool `efe-admin migrate-catalogs --from memory --to sqlite:///path`.

### 13.2 Schéma SQLite (preview D14)

```sql
CREATE TABLE catalog_versions (
    tenant     TEXT NOT NULL,
    name       TEXT NOT NULL,
    version    INTEGER NOT NULL,
    content    BLOB NOT NULL,        -- JSON serialised CatalogContent
    metadata   BLOB NOT NULL,        -- JSON serialised CatalogMetadata
    compiled   BLOB NOT NULL,        -- per-class compiled anchors (numpy.save format)
    sha256     TEXT NOT NULL,
    created_at INTEGER NOT NULL,     -- unix epoch
    PRIMARY KEY (tenant, name, version)
);
CREATE INDEX idx_catalog_lookup ON catalog_versions(tenant, name);
```

### 13.3 Plan migration phases

```
v0.1.12 (D12) — in-memory only, lost on restart
                 ↓
v0.1.13       — same code, +SQLite optional (Helm flag)
                 ↓
v0.1.14 (D14) — SQLite default, in-memory deprecated
                 ↓
v0.2.0+       — Postgres default for Phase 2 SaaS
```

Aucun changement de proto requis sur le chemin v0.1.12 → SQLite.

---

## 14. 12 décisions de conception clés

| # | Décision | Alternative rejetée | Justification |
|--:|----------|---------------------|---------------|
| 1 | Versioning monotone strict | "v1.5", "vlatest" floating | Audit trail clair ; predictable |
| 2 | Immutability post-register | PUT-style upsert | Trace forensique solide |
| 3 | In-memory MVP storage | DB-backed dès v0.1.12 | Ship vite ; migration tracée |
| 4 | Server-side embedding compilation | Client-side | Single source of truth ; embedder fingerprint match |
| 5 | YAML on-disk reuse | New JSON-only format | Backward compat, doc continuity |
| 6 | Separate CatalogService | Add to AdminService | RBAC clean separation |
| 7 | 5 evolve ops (not 10+) | Plus comprehensive set | 95 % use cases covered ; smaller surface |
| 8 | Hard pin version (no @latest) | Latest-floating | Predictable production behavior |
| 9 | content_sha256 in metadata | No integrity witness | Forensic trail ; mid-flight tamper detection |
| 10 | Quotas per-tenant | Cluster-wide quotas | Per-tenant fairness (D11 gate) |
| 11 | New protobuf service (not extension) | Extend AdminService | Wire compat ; allow incremental rollout |
| 12 | Cache LRU 10 000 versions | TTL-based | Versions immutable → cache forever |

---

## 15. 10 risques identifiés et mitigations

| # | Risque | Sévérité | Probabilité | Mitigation |
|--:|--------|:--------:|:-----------:|------------|
| 1 | Proto change breaks pre-D12 clients | HIGH | LOW | Strictly additive (new service, new oneof variant) |
| 2 | In-memory state lost on pod restart | HIGH | HIGH | Document expected ; migration path tracked D14 |
| 3 | Embedding cost on Recompile DOS | MED | MED | Per-tenant rate limit on Evolve RPCs (one-pole) |
| 4 | Stub regen drift | HIGH | LOW | CI step validates pb2 stubs match .proto |
| 5 | Catalog content > 1 MB exceeds gRPC max | MED | MED | Validate `total_anchors * dim * 4 bytes ≤ 16 MB` |
| 6 | Audit log volume explosion | MED | MED | Per-tenant log streams ; sampling on high-volume |
| 7 | SDK admin client breaks API surface (M11) | HIGH | LOW | Test surface freeze ; new methods additive |
| 8 | Tenant_id mismatch with catalog_ref.tenant | HIGH | MED | Server `assert_tenant_match` (D11 invariant) |
| 9 | Race condition on version increment | MED | LOW | Atomic per-`(tenant, name)` counter (DashMap entry CAS) |
| 10 | Migration broken old-clients on @latest | LOW | LOW | @latest never supported in D12 ; reject with clear error |

---

## 16. Plan jour-par-jour (5 j wallclock)

### D12.1 (1 j) — Proto + stubs
* Edit `protos/efe/v1/admin.proto` : add CatalogService + 6 messages.
* Edit `protos/efe/v1/types.proto` : add CatalogRef.
* Edit `protos/efe/v1/efe.proto` : oneof variant.
* Run `cargo build` : tonic regenerates Rust stubs.
* Run `python -m grpc_tools.protoc ...` : regenerate Python stubs.
* Verify : `pb2.CatalogService` exists, `pb2.CatalogServiceStub`
  exists, `pb2.RegisterCatalogRequest` exists.
* `cargo test --workspace` : ensure no regression.

### D12.2 (1.5 j) — Server registry + service
* `crates/efe-server/src/catalog/registry.rs` — trait.
* `crates/efe-server/src/catalog/memory.rs` — impl.
* `crates/efe-server/src/catalog/policy.rs` — quotas.
* `crates/efe-server/src/catalog/audit.rs` — log emission.
* `crates/efe-server/src/services/catalog.rs` — gRPC handler.
* `crates/efe-server/src/auth/scopes.rs` — admin:catalog:* scopes.
* `crates/efe-server/src/runtime.rs` — wire registry to service.
* 5 test files (`d12_catalog_*.rs`) — 35+ tests.

### D12.3 (½ j) — Execute path catalog_ref
* `crates/efe-server/src/services/swarm.rs` — wire CatalogRef
  lookup.
* `crates/efe-telemetry/src/metrics/recorder.rs` — 4 new counters.
* `crates/efe-server/src/catalog/cache.rs` — LRU.
* Tests in `d12_execute_catalog_ref.rs`.

### D12.4 (1 j) — SDK admin extensions
* `clients/python/efe_tool/admin/client.py` — 5 new methods +
  activate `list_catalogs`.
* `clients/python/efe_tool/admin/types.py` — new dataclasses.
* `__init__.py` — export new symbols.
* `docs/docs/howto/catalog-admin.md`.
* `docs/docs/howto/catalog-evolution.md`.
* `docs/docs/reference/catalog-rpc.md`.

### D12.5 (½ j) — CLI subcommand
* `clients/python/efe_tool/cli/catalog.py` — register, evolve,
  list, get, delete.
* `clients/python/efe_tool/cli/main.py` — wire subcommand.
* `docs/docs/reference/cli.md` — extend.
* `examples/05_catalog_admin.py` — full workflow demo.

### D12.7 (½ j) — SLOs + CI
* `parse_criterion_report.py` — 7 SLOs `D12-CT-*`.
* `RESERVED_PREFIXES` — add `catalog::`.
* `bench-catalog.yml` workflow.
* `scripts/tests/test_d12_*.py` — disjoint keys, surface, audit.

### D12.8 (½ j) — Closure
* `bench_baselines/v0.1.12/baseline.json`.
* `BENCHMARKS § 14`.
* `docs/audits/d12_catalog_report.md` skeleton.
* `CHANGELOG.md` Unreleased D12.

---

## 17. SLOs ajoutés post-D12

### 17.1 Liste des 7 nouveaux SLOs

| ID | Métrique | Comparator | Cible | Display | Source |
|----|----------|:----------:|------:|:-------:|--------|
| `D12-CT-1` | `catalog::register_e2e_p99_ms` | `lte` | 2000 | ms | `bench-catalog.yml` synthetic |
| `D12-CT-2` | `catalog::evolve_e2e_p99_ms` | `lte` | 5000 | ms | idem |
| `D12-CT-3` | `catalog::version_monotonic_violations` | `lte` | 0 | count | Rust isolation tests |
| `D12-CT-4` | `catalog::cross_tenant_leakage` | `lte` | 0 | count | Rust isolation tests |
| `D12-CT-5` | `catalog::cache_hit_ratio` | `gte` | 0.95 | ratio | Prometheus |
| `D12-CT-6` | `catalog::cli_register_e2e_s` | `lte` | 5 | s | CLI smoke test |
| `D12-CT-7` | `catalog::sdk_round_trip_e2e_s` | `lte` | 3 | s | SDK e2e test |

### 17.2 Disjoint-key invariant (étendu)

`RESERVED_PREFIXES` post-D12 = **10 namespaces** :

```python
RESERVED_PREFIXES = (
    "eval_accuracy::", "eval_latency::",       # D6/D7/D8
    "load_", "soak::",                          # D9
    "docs::", "dx::",                           # D10
    "isolation::", "cost::", "obs::",            # D11
    "catalog::",                                 # D12 NEW
)
```

---

## 18. Anti-régression & realism check

### 18.1 Tests Rust workspace

```bash
cargo test --workspace
# +
cargo test --package efe-server --test 'd12_catalog_*'
```

D12 ajoute 5 fichiers, ~35 tests Rust :
* `d12_catalog_register.rs` — register flow + validation
* `d12_catalog_evolve.rs` — 5 ops + monotonicity
* `d12_catalog_versioning.rs` — invariants
* `d12_catalog_cross_tenant.rs` — D11 isolation reused
* `d12_catalog_audit.rs` — log emission

### 18.2 Tests Python

```bash
pytest scripts/tests/test_d12_*.py
pytest examples/tests/
pytest scripts/billing/tests/   # carry-over D11
```

D12 ajoute ~15 tests Python.

### 18.3 Realism check sur minikube

```bash
helm install efe oci://... --version 0.1.12 -n efe
# Tier 1
EFE_JWT=$(idp-token --tenant acme --scopes admin:catalog:write) \
    efe-tool catalog register --file catalogs/support_triage.yaml \
    --tenant acme --version 1.0.0
efe-tool catalog list
efe-tool catalog evolve --from acme/support_triage@v1 \
    --append-class --class-file new-class.yaml
efe-tool catalog get acme/support_triage@v2
```

---

## 19. Intégration CI : workflow `bench-catalog.yml`

```yaml
name: bench-catalog

on:
  push:
    paths:
      - "protos/efe/v1/**"
      - "crates/efe-server/src/catalog/**"
      - "crates/efe-server/src/services/catalog.rs"
      - "clients/python/efe_tool/admin/**"
      - "clients/python/efe_tool/cli/catalog.py"
  pull_request:
    paths: [same]
  workflow_dispatch:

jobs:
  proto-stub-validity:
    # Ensure regenerated stubs match .proto
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - run: |
          # Regenerate from current .proto
          protoc --python_out=/tmp/regen --grpc_python_out=/tmp/regen \
              --proto_path=protos protos/efe/v1/admin.proto
          # Diff against committed stubs
          diff -q /tmp/regen/efe/v1/admin_pb2.py clients/python/efe_tool/_proto/efe/v1/admin_pb2.py

  catalog-rust-tests:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test --workspace --release --test 'd12_*'

  catalog-python-tests:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
      - run: pip install -e ./clients/python && pytest scripts/tests/test_d12_*.py
```

---

## 20. Critères d'acceptation (DoD)

### 20.1 Proto
- [ ] CatalogService 5 RPCs livrés.
- [ ] CatalogRef in types.proto.
- [ ] SwarmSpec.cone_source extended with catalog_ref.
- [ ] Stubs regenerated + committed (Python + Rust).

### 20.2 Server
- [ ] CatalogRegistry trait + MemoryCatalogRegistry impl.
- [ ] 5 evolve ops complete.
- [ ] Quotas enforced.
- [ ] Audit log per mutation.
- [ ] Execute path resolves catalog_ref → AlignmentCone.
- [ ] 35+ Rust tests passing.

### 20.3 SDK + CLI
- [ ] EfeAdminClient 5 new methods + activated list_catalogs.
- [ ] efe-tool catalog subcommand 5 verbs.
- [ ] examples/05_catalog_admin.py.
- [ ] surface freeze test extended.

### 20.4 Documentation
- [ ] howto/catalog-admin.md.
- [ ] howto/catalog-evolution.md.
- [ ] reference/catalog-rpc.md.
- [ ] reference/cli.md updated.

### 20.5 CI + SLOs
- [ ] 7 SLOs `D12-CT-*` in parse_criterion_report.
- [ ] RESERVED_PREFIXES extended.
- [ ] bench-catalog.yml workflow.
- [ ] All previous SLOs (33) still gated.

### 20.6 Closure
- [ ] BENCHMARKS § 14.
- [ ] bench_baselines/v0.1.12/baseline.json.
- [ ] docs/audits/d12_catalog_report.md skeleton.
- [ ] CHANGELOG entry.

---

## 21. Out of scope & roadmap follow-ups

### 21.1 Hors-scope D12

- ❌ Persistent storage (D14).
- ❌ `@latest` floating version pointer (D15+).
- ❌ Server-side `WatchCatalog` streaming RPC (D15+).
- ❌ Catalog UI (D14+).
- ❌ Cross-tenant catalog sharing (sponsorship pattern, hors-scope).

### 21.2 Roadmap après D12

| Sprint | Cible | Justification |
|--------|-------|---------------|
| **D12.bis** | Reviewer feedback resolution | First catalog admin partner pilot |
| **D13** | Grafana dashboards (full SRE pack) | Phase 2 prod observability |
| **D14** | Production cutover + persistent catalog | Pilot → paid |
| **D15** | OAuth2/OIDC + WatchCatalog | Phase 3 SaaS |

### 21.3 Métrique de succès post-D12

> *« 3 partenaires utilisent activement `efe-tool catalog`
> (commits Git montrant catalog YAML evolution). Aucun support
> ticket sur "ListCatalogs raise NotImplementedError". `D12-CT-*`
> SLOs verts depuis ≥ 3 weeks. Architecture spec en référence
> dans le partner onboarding pack. »*

---

## 22. Annexes

### 22.1 Annexe A — Full proto contract (récap)

```protobuf
service CatalogService {
    rpc RegisterCatalog(RegisterCatalogRequest) returns (CatalogHandle);
    rpc EvolveCatalog(EvolveCatalogRequest) returns (CatalogHandle);
    rpc GetCatalog(GetCatalogRequest) returns (CanonicalCatalog);
    rpc ListCatalogs(ListCatalogsRequest) returns (ListCatalogsResponse);
    rpc ListCatalogVersions(ListCatalogVersionsRequest) returns (ListVersionsResponse);
    rpc DeleteCatalog(DeleteCatalogRequest) returns (google.protobuf.Empty);
}
```

### 22.2 Annexe B — `examples/05_catalog_admin.py` skeleton

```python
"""Sprint D12 — example 5: catalog admin workflow.

End-to-end : register a catalog, evolve it, list, get, delete.
Demonstrates the canonical governance path for partner ops.

Pre-requisites:
    * efe-server with admin:catalog:* scope JWT
    * pip install efe-tool==0.1.12
"""
import os
from efe_tool.admin import EfeAdminClient, EvolveOp

with EfeAdminClient(
    endpoint="127.0.0.1:50051",
    jwt=os.environ["EFE_JWT_ADMIN"],
    insecure=True,
) as admin:
    # 1. Register
    handle = admin.register_catalog(
        tenant="acme",
        name="support_triage",
        version="1.0.0",
        content=yaml_load("catalogs/support_triage.yaml"),
        description="Initial release",
    )
    print(f"Registered: {handle.key}")

    # 2. Evolve — append a new class
    handle_v2 = admin.evolve_catalog(
        from_key=handle.key,
        op=EvolveOp.APPEND_CLASS,
        op_payload={
            "class": {
                "name": "billing_extract",
                "description": "Extract billing details",
                "canonical_sentences": [
                    "Show me my last invoice",
                    "Combien j'ai payé ce mois ?",
                ],
            },
        },
    )
    print(f"Evolved to: {handle_v2.key}")

    # 3. List
    for ref in admin.list_catalogs():
        print(f"  {ref}")

    # 4. Cleanup
    admin.delete_catalog(handle.key, force=True)
```

### 22.3 Annexe C — Cheat-sheet partner onboarding

```bash
# 1. Get JWT with admin:catalog:write scope
EFE_JWT=$(idp-token --tenant acme --scopes admin:catalog:write)

# 2. Register your first catalog
efe-tool catalog register --file my-catalog.yaml --tenant acme --version 1.0.0

# 3. Iterate by evolving — anchors live server-side
efe-tool catalog evolve --from acme/my-catalog@v1 \
    --replace-class-anchors --class-name fraud --class-file fraud-v2.yaml

# 4. Application uses CatalogRef — no app redeploy needed
# In your app code:
#   client.classify(text, catalog_ref="acme/my-catalog@v3")

# 5. Audit your catalog history
efe-tool catalog list-versions --tenant acme --name my-catalog
```

### 22.4 Annexe D — Glossary

| Term | Definition |
|------|------------|
| **Catalog** | Versioned set of anchors with metadata, registered server-side via D12. |
| **CatalogRef** | Reference to a server-side catalog by `(tenant, name, version)`. |
| **Canonical key** | String form `tenant/name@vN`. |
| **Evolution op** | One of 5 transformations (append_class, remove_class, etc.). |
| **Hot-reload** | Server picks up new catalog versions without restart ; client must explicitly bump version. |

### 22.5 Annexe E — Checklist d'exécution architect → impl

```
□ D12.1.1  protos/efe/v1/admin.proto: add CatalogService + 6 messages
□ D12.1.2  protos/efe/v1/types.proto: add CatalogRef
□ D12.1.3  protos/efe/v1/efe.proto: extend SwarmSpec.cone_source
□ D12.1.4  Regenerate Python pb2 + pb2_grpc (committed)
□ D12.1.5  cargo build (tonic auto-regen)
□ D12.1.6  test_d12_proto_contract.py: pinpoint regenerated message types

□ D12.2.1  catalog/registry.rs (trait)
□ D12.2.2  catalog/memory.rs (impl)
□ D12.2.3  catalog/policy.rs (quotas)
□ D12.2.4  catalog/audit.rs (log emission)
□ D12.2.5  services/catalog.rs (gRPC handler)
□ D12.2.6  auth/scopes.rs (admin:catalog:*)
□ D12.2.7  runtime.rs (wire registry)
□ D12.2.8  d12_catalog_register.rs (8 tests)
□ D12.2.9  d12_catalog_evolve.rs (10 tests)
□ D12.2.10 d12_catalog_versioning.rs (6 tests)
□ D12.2.11 d12_catalog_cross_tenant.rs (5 tests)
□ D12.2.12 d12_catalog_audit.rs (6 tests)

□ D12.3.1  services/swarm.rs (catalog_ref resolution)
□ D12.3.2  catalog/cache.rs (LRU)
□ D12.3.3  telemetry recorder: efe_catalog_* counters
□ D12.3.4  d12_execute_catalog_ref.rs (4 tests)

□ D12.4.1  efe_tool/admin/types.py (new dataclasses)
□ D12.4.2  efe_tool/admin/client.py (5 new methods + activate list_catalogs)
□ D12.4.3  efe_tool/admin/__init__.py (exports)
□ D12.4.4  docs/docs/howto/catalog-admin.md
□ D12.4.5  docs/docs/howto/catalog-evolution.md
□ D12.4.6  docs/docs/reference/catalog-rpc.md
□ D12.4.7  surface freeze test extended

□ D12.5.1  cli/catalog.py (5 verbs)
□ D12.5.2  cli/main.py (wire subcommand)
□ D12.5.3  docs/docs/reference/cli.md (extend)
□ D12.5.4  examples/05_catalog_admin.py

□ D12.7.1  parse_criterion_report.py (7 SLOs)
□ D12.7.2  RESERVED_PREFIXES += "catalog::"
□ D12.7.3  bench-catalog.yml workflow
□ D12.7.4  scripts/tests/test_d12_disjoint_keys.py
□ D12.7.5  scripts/tests/test_d12_surface_extensions.py
□ D12.7.6  scripts/tests/test_d12_proto_contract.py

□ D12.8.1  bench_baselines/v0.1.12/baseline.json
□ D12.8.2  BENCHMARKS § 14
□ D12.8.3  docs/audits/d12_catalog_report.md
□ D12.8.4  CHANGELOG.md Unreleased D12
```

### 22.6 Annexe F — Frontière D12 ↔ D13+

D12 ships :
* Catalog admin endpoints + versioning ✅
* Hot-reload via `catalog_ref` ✅
* SDK + CLI ✅
* Audit trail ✅

D12 does NOT ship :
* Persistent storage (D14)
* `@latest` floating ref (D15+)
* Server-side notification stream (D15+)
* Catalog UI (D14+)
* Cross-tenant sharing (out of scope, ever?)

---

**Fin de l'architecture D12.** Le sprinter peut implémenter en
suivant Annexe E, en revenant à §5 pour proto, §6 pour le
serveur, §9 pour le SDK.

> **Approbation requise avant implémentation** :
> - [ ] Lead infra (revue trait + storage migration path)
> - [ ] Lead sécu (revue scoping admin:catalog:*)
> - [ ] Owner produit (revue versioning sémantique)
> - [ ] Lead dev (revue SDK + CLI surface)
