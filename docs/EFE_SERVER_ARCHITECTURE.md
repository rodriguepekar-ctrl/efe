# `efe-server` — Architecture détaillée du module gRPC + modes (M4)

> **Statut** : Draft v1.0.0 — Document de référence pour l'achèvement
> du module **M4 = `efe-server`** entre l'AS-IS (post-D18, Mode-1 +
> multi_replica + OIDC + embedder bundle livrés ✅) et la cible
> **GA 1.0** (AdminService 8 RPCs + mTLS + 3 scopes JWT + live
> bundle reload + streaming + Mode-2 + Mode-3).
> **Auteur** : rodrigue.pekar@icloud.com
> **Date** : 2026-04-28.
> **Dépendances doc** : `docs/PRODUCT_COMPLETION_ARCHITECTURE.md`,
> `docs/EFE_KERNELS_ARCHITECTURE.md`, `docs/EFE_FFI_ARCHITECTURE.md`,
> `M11_SECURITY_ARCHITECTURE.md`, `M12_ARCHITECTURE.md`,
> `LETTER_E_ARCHITECTURE.md`,
> `docs/bench/d12bis_catalog_grpc_handler_architecture.md`,
> `docs/bench/d14bis_hmac_audit_chain_architecture.md`,
> `docs/bench/d15_onboarding_idp_architecture.md`,
> `docs/bench/d16_embedder_bundling_architecture.md`,
> `docs/bench/d18_multi_replica_bench_d12_architecture.md`.
> **Portée** : 6 sprints non-contigus s'étalant sur **4 phases**
> (Φ-B / Φ-C / Φ-D / Φ-E / Φ-F). Pas un sprint mais une **épine
> dorsale serveur** consolidée :
>
>   * **D21** (Φ-B) — `AdminService` 8 RPCs (List/Get/Delete/Reload/Pin/Evolve/ListAuditEvents).
>   * **D22** (Φ-B) — mTLS obligatoire + 3 scopes JWT (data/admin/audit).
>   * **D27** (Φ-C) — Live bundle reload via `AdminService.ReloadBundle`.
>   * **D32** (Φ-D) — `Mode1Service.ExecuteStreaming` activé.
>   * **D34** (Φ-E) — `Mode2Service` proto + handler.
>   * **D37** (Φ-F) — `Mode3Service` proto + handler.

---

## Table des matières

§0. Méta, glossaire, conventions
§1. Mission de `efe-server` (extension Φ-B → Φ-F)
§2. Position dans la topologie EFE
§3. État AS-IS au 2026-04-28 (post-D18)
§4. Définition opérationnelle de `efe-server` v1.0 GA
§5. Principes directeurs
§6. Invariants structurels
§7. Vue d'ensemble — 6 sprints, 6 livrables, dependency graph
§8. Surface gRPC publique cible
§9. **Sprint D21 — AdminService 8 RPCs**
§10. **Sprint D22 — mTLS + 3 scopes JWT**
§11. **Sprint D27 — Live bundle reload**
§12. **Sprint D32 — Streaming RPC**
§13. **Sprint D34 — Mode-2 (anchors dynamiques + LLM router)**
§14. **Sprint D37 — Mode-3 (multi-turn parallel)**
§15. Authorization matrix (RPC × scope)
§16. Audit chain integration
§17. Rate limiting per-tenant — extension streaming
§18. Observabilité — nouvelles métriques (~15 instruments)
§19. Dashboards Grafana — Mode-1/2/3 panels
§20. SLO produit final
§21. Migration plan (D18 → D37)
§22. Coexistence avec clients legacy
§23. Plans d'implémentation détaillés sprint-par-sprint
§24. Risques transverses + atténuations
§25. Critères Go/No-Go par sprint
§26. Annexes — proto definitions complètes
§27. Annexes — Helm template additions
§28. Annexes — RACI + budget
§29. Annexes — Gantt prévisionnel
§30. Mises à jour du document

---

## §0. Méta, glossaire, conventions

### §0.1 Conventions

* **AS-IS** = état du code à fin D18 (post-audit), v0.1.19.
* **TO-BE** = état cible défini par chaque sprint.
* Les sprints D21, D22, D27, D32, D34, D37 sont **non-contigus** :
  des sprints intercalaires (D23-D26 SDKs Go/TS/Java, D28-D31 GPU,
  D33 templates, D35-D36 LLM routers) ne touchent pas `efe-server`
  directement.
* Les défauts résiduels sont **DT-S.x** (Defect Trace, Server).
* Toute mention "le moteur" sans qualification = `efe-core::SwarmController`.
* `[ref:fichier:ligne]` = pointeur dans le code source.

### §0.2 Glossaire

| Terme | Définition |
|-------|------------|
| **`efe-server`** | Le crate Rust `crates/efe-server/` qui expose les services gRPC + le runtime multi-réplica. |
| **`Mode1Service`** | Le service gRPC actuel (D4-D18) qui expose `Execute`, `DescribeSpec`, et `ExecuteStreaming` (skeleton). Mode-1 = anchors compilés via catalog. |
| **`Mode2Service`** | Le service gRPC nouveau (D34) qui ajoute le routage par LLM avant le swarm. Anchors générées dynamiquement par requête. |
| **`Mode3Service`** | Le service gRPC nouveau (D37) qui exécute k swarms parallèles + agrège les centroïdes. |
| **`AdminService`** | Le service gRPC d'administration. AS-IS : 6 RPCs cone-scoped. TO-BE : +8 RPCs catalog-scoped (D21). |
| **`CatalogService`** | Le service gRPC de gestion des catalogs (D12-bis ✅). 4 RPCs : `Register`, `Evolve`, `Resolve`, `Pin`. |
| **mTLS** | Mutual TLS — le serveur ET le client s'authentifient via certificat X.509. cert-manager provisioning automatique en K8s. |
| **Scope JWT** | Claim `scope` (RFC 6749) ou `scopes` array. Trois valeurs : `data` (Mode1/2/3 RPCs), `admin` (AdminService), `audit` (read-only ListAuditEvents). |
| **`ReloadBundle`** | RPC nouveau D27 qui swap atomiquement le `Arc<Bundle>` actif. Drain l'ancien bundle quand RPCs en flight terminent. |
| **`ExecuteStreaming`** | RPC server-side streaming D32. Émet 1 chunk par fusion round + 1 chunk final avec verdict. |
| **`LlmRouter`** | Trait Rust D34 qui génère anchors dynamiques pour Mode-2. Implems : OpenAI, Anthropic, Mock. |
| **`Mode3Aggregator`** | Trait Rust D37 qui combine k centroïdes en un verdict final. Stratégies : Mean, Weighted, Median. |
| **`AlignmentChain`** | Le `efe-core::alignment::registry::InMemoryRegistry` qui gère les cônes (D9). RPCs admin "RegisterCone", "GetCone", etc. |

### §0.3 Cible matérielle

`efe-server` tourne sur K8s avec :

* CPU 8 vCPU minimum, RAM 8 GiB minimum (D18 § 14.4 res block)
* Optionnel : 1 NVIDIA T4 (Tier-1) ou A10 (Tier-3 prod) — feature `cuda`
* TLS termination via Envoy/Istio (mTLS ingress) OU directement via tonic-tls

### §0.4 Cibles fonctionnelles GA

```
GA v1.0 efe-server :
├── Mode1Service   ─ Execute + DescribeSpec + ExecuteStreaming   (D4 + D32)
├── Mode2Service   ─ Execute + ExecuteStreaming + RouterHealth   (D34)
├── Mode3Service   ─ Execute + SubscribeVerdict + Cancel          (D37)
├── AdminService   ─ 14 RPCs (6 cone + 8 catalog)                (D9 + D21)
├── CatalogService ─ 4 RPCs (Register / Evolve / Resolve / Pin)  (D12-bis ✅)
├── HealthService  ─ Standard gRPC health probe                  (D9 ✅)
└── Reflection v1  ─ Standard gRPC reflection                    (D4 ✅)
```

---

## §1. Mission de `efe-server` (extension Φ-B → Φ-F)

### §1.1 Énoncé opérationnel

> *Étendre `efe-server` post-D18 pour exposer une surface gRPC GA
> 1.0 multi-tenant avec mTLS obligatoire, 3 scopes JWT, AdminService
> complet, live bundle reload, streaming RPC, et les 3 modes
> d'exécution (Mode-1, Mode-2, Mode-3) — sans casser un seul client
> existant Mode-1 D4-D18.*

### §1.2 Périmètre stricte du document

Ce document couvre **uniquement** les 6 sprints :

* **D21** — AdminService 8 RPCs catalog-scoped.
* **D22** — mTLS + 3 scopes JWT.
* **D27** — Live bundle reload.
* **D32** — Streaming Mode-1.
* **D34** — Mode-2 service.
* **D37** — Mode-3 service.

**Hors-portée** :

* CUDA kernels, GPU runtime, memory allocator → cf. `EFE_KERNELS_ARCHITECTURE.md`.
* PyO3 bindings → cf. `EFE_FFI_ARCHITECTURE.md`.
* Backup + DR → cf. `docs/bench/d17_backup_dr_architecture.md`.
* Multi-replica + leader election → cf. `docs/bench/d18_multi_replica_bench_d12_architecture.md` (livré).
* Embedder bundle pipeline → cf. `docs/bench/d16_embedder_bundling_architecture.md` (livré).
* OIDC tenant onboarding → cf. `docs/bench/d15_onboarding_idp_architecture.md` (livré).

### §1.3 Mission complémentaire

`efe-server` est aussi le **canari de toute la stack runtime** :
chaque sprint doit s'intégrer end-to-end (gRPC → handler →
controller → backend → response) sans drift par rapport aux
modules amont. Une régression D21 sur AdminService doit casser
les pin tests AVANT D22, etc.

---

## §2. Position dans la topologie EFE

### §2.1 Graph de dépendances post-D37

```
                      ┌─────────────────────────────┐
                      │  External clients            │
                      │  (Python SDK gRPC, Go SDK,   │
                      │   TS SDK, Java SDK)          │
                      └────────┬────────────────────┘
                               │
                               │ TLS 1.3 + mTLS + JWT (3 scopes)
                               ▼
                      ┌─────────────────────────────┐
                      │       efe-server             │
                      │                              │
                      │  ┌────────────────────────┐  │
                      │  │  Mode1Service (D4+D32) │  │
                      │  │  Mode2Service (D34)    │  │
                      │  │  Mode3Service (D37)    │  │
                      │  │  AdminService (D9+D21) │  │
                      │  │  CatalogService (D12)  │  │
                      │  │  HealthService          │  │
                      │  │  Reflection             │  │
                      │  └────────┬───────────────┘  │
                      │           │                   │
                      │  ┌────────▼───────────────┐  │
                      │  │  AuthGuard (D15+D22)    │  │
                      │  │  ScopeGuard (D22 NEW)   │  │
                      │  │  AuditEmitter (D14b)    │  │
                      │  │  RateLimiter (D18)      │  │
                      │  │  IdempotencyStore (D18) │  │
                      │  │  EmbedderRegistry (D16) │  │
                      │  │  CatalogRegistry (D14)  │  │
                      │  │  AlignmentRegistry (D9) │  │
                      │  │  LlmRouter (D34 NEW)    │  │
                      │  │  Mode3Aggregator (D37)  │  │
                      │  └────────┬───────────────┘  │
                      │           │                   │
                      └───────────┼───────────────────┘
                                  │
                                  ▼
                  ┌──────────────────────────────────┐
                  │          efe-core                 │
                  │   (SwarmController + ComputeBackend │
                  │    trait + types + alignment)     │
                  └────────────┬─────────────────────┘
                               │
                               ▼ (optional)
                       ┌──────────────┐
                       │  efe-kernels │
                       │   (GPU)      │
                       └──────────────┘
```

### §2.2 Direction stricte

`efe-server → efe-core` (transitivement → `efe-kernels`).
Jamais l'inverse.

### §2.3 Position vs `efe-ffi` (M3 PyO3)

`efe-ffi` (M3) et `efe-server` (M4) sont **PARALLÈLES** :

| Aspect | `efe-server` (gRPC) | `efe-ffi` (PyO3) |
|--------|--------------------|--------------------|
| **Trafic** | Réseau TCP | In-process |
| **Multi-tenant** | ✅ (per-tenant rate-limit, audit) | ❌ |
| **Audit chain** | ✅ HMAC server-side | ❌ |
| **Cible Lettre** | E (service backend) | A/B/C/D |
| **Latence p99** | 200 ms (charge 200 RPS) | 6 ms |

Les deux co-existent. `efe-ffi` consume `efe-core` directement.
`efe-server` aussi.

---

## §3. État AS-IS au 2026-04-28 (post-D18)

### §3.1 Inventaire physique

```
crates/efe-server/src/
├── auth/
│   ├── guard.rs            ✅ AuthGuard (D15)
│   ├── issuers.rs          ✅ OIDC issuers (D15)
│   ├── jwks.rs             ✅ JWKS cache (D15)
│   ├── jwt.rs              ✅ JWT validator (D15)
│   ├── mod.rs              ✅
│   ├── oidc.rs             ✅ OidcValidator (D15)
│   ├── scopes.rs           ✅ scope parsing (D15 stub)
│   ├── tenants.rs          ✅ TenantStatusGate (D15)
│   ├── token_store.rs      ✅ HS256 store (legacy)
│   └── test_keys/          ✅ test fixtures
├── catalog/                ✅ CatalogRegistry (D12-bis + D14)
├── embedder/               ✅ Bundle loader (D16)
├── multi_replica/          ✅ Leader election + idempotency + ratelimit (D18)
├── services/
│   ├── admin.rs            ⚠️ 6 RPCs cone-scoped (D9), needs D21 extension
│   ├── catalog/            ✅ CatalogService 4 RPCs (D12-bis)
│   ├── metrics.rs          ✅
│   ├── mod.rs              ✅
│   └── swarm.rs            ✅ Mode1Service Execute / DescribeSpec ; ExecuteStreaming = stub
├── config.rs               ✅ ServerConfig + RuntimeConfig + MultiReplicaConfig
├── runtime.rs              ✅ Server::build + serve (post-D18 audit GREEN)
├── shutdown.rs             ✅
└── lib.rs / main.rs / bin/ ✅
```

### §3.2 Surface gRPC AS-IS

| Service | RPCs | Status | Sprint |
|---------|------|:------:|:------:|
| `Mode1Service.Execute` | unary | ✅ | D4 |
| `Mode1Service.DescribeSpec` | unary | ✅ | D4 |
| `Mode1Service.ExecuteStreaming` | server-stream | ⚠️ stub | D32 |
| `AdminService.RegisterCone` | unary | ✅ | D9 |
| `AdminService.EvolveCone` | unary | ✅ | D9 |
| `AdminService.GetCone` | unary | ✅ | D9 |
| `AdminService.ListVersions` | unary | ✅ | D9 |
| `AdminService.DeleteCone` | unary | ✅ | D9 |
| `AdminService.BreachHistory` | unary | ✅ | D9 |
| `CatalogService.Register` | unary | ✅ | D12-bis |
| `CatalogService.Evolve` | unary | ✅ | D12-bis |
| `CatalogService.Resolve` | unary | ✅ | D12-bis |
| `CatalogService.Pin` | unary | ✅ | D12-bis |
| `HealthService.Check` | unary | ✅ | D9 |
| `HealthService.Watch` | server-stream | ✅ | D9 |
| `ServerReflection` | unary + bidi | ✅ | D4 |

**Total AS-IS** : 16 RPCs distincts (12 unary + 2 stream + 2 reflection).

### §3.3 Auth AS-IS (D15)

* JWT validation : HS256 (legacy) + OIDC RS256/ES256 multi-issuer.
* `AuthGuard::enabled_oidc(validator)` ou `enabled_hs256(store)` ou `disabled()`.
* `TenantStatusGate` : suspended tenants refusés.
* **Pas de scope check** côté handler — tout JWT valide passe.
* **Pas de mTLS obligatoire** — TLS optionnel, pas de cert client validé.

### §3.4 Audit chain AS-IS (D14-bis)

* `HmacChainedAuditSink` ✅ — chaque RPC admin/catalog write émet un event signé.
* `audit_events` table Postgres avec chain head gauge.
* `periodic_self_verify` background task (D14-bis audit C1).

### §3.5 Embedder bundle AS-IS (D16)

* `EmbedderRegistry::ensure_loaded_with_timeout(...)` — eager load au boot.
* `Arc<Bundle>` partagé.
* **Pas de hot-reload** — restart pod requis pour swap bundle.

### §3.6 Synthèse AS-IS

| Capacité | Status |
|----------|:------:|
| Mode-1 Execute | ✅ |
| Mode-1 ExecuteStreaming | ⚠️ stub |
| Mode-2 / Mode-3 | ❌ |
| AdminService cone (6 RPCs) | ✅ |
| AdminService catalog (8 RPCs) | ❌ |
| mTLS | ❌ |
| 3 scopes JWT | ❌ |
| Live bundle reload | ❌ |
| Audit chain HMAC | ✅ |
| Multi-replica + leader | ✅ |
| OIDC tenant onboarding | ✅ |
| Rate-limit per-tenant | ✅ |

**Verdict** : 6 capacités majeures à livrer sur 6 sprints.

---

## §4. Définition opérationnelle de `efe-server` v1.0 GA

### §4.1 Le contrat GA

`efe-server` v1.0 satisfait **20 propriétés contractuelles** :

| # | Propriété | Métrique opposable |
|--:|-----------|--------------------|
| S-1 | Mode-1 Execute p99 | ≤ 200 ms (GPU, 200 RPS) |
| S-2 | Mode-1 ExecuteStreaming p99 first-chunk | ≤ 50 ms |
| S-3 | Mode-2 Execute p99 | ≤ 1500 ms (incluant 1 LLM call) |
| S-4 | Mode-3 Execute p99 | ≤ 3 × Mode-1 p99 |
| S-5 | mTLS obligatoire en E | TLS 1.3 + cert client validé |
| S-6 | 3 scopes JWT | data / admin / audit avec deny-by-default |
| S-7 | AdminService 8+6 RPCs | tous GREEN avec audit chain |
| S-8 | Live bundle reload | ≤ 30s p99, no request dropped |
| S-9 | Audit chain integrity | 100 % HMAC verifiable |
| S-10 | Streaming back-pressure | client disconnect → server cleanup ≤ 1s |
| S-11 | Mode-2 LLM cache | hit rate ≥ 60 % par hash(text, top_k) |
| S-12 | Mode-3 k_max | k ≤ 5 hard cap |
| S-13 | Multi-tenant rate-limit | per-tenant leaky bucket (D18 ✅) |
| S-14 | Idempotency cross-replica | shared PG cache (D18 ✅) |
| S-15 | Health probe | liveness + readiness selon préconditions |
| S-16 | Graceful shutdown | 3-phase D18 ≤ 60s grace period |
| S-17 | Observabilité | 31 + 15 nouveaux instruments |
| S-18 | Determinism | seed fixé → outputs bit-identiques |
| S-19 | Backward compat | clients D4-D18 marchent inchangés |
| S-20 | Cargo workspace tests | 100 % GREEN sur tags release |

### §4.2 Versioning produit

| Version | Sprint | Livrable |
|---------|:------:|----------|
| v0.1.19 | D18 (livré) | Multi-réplica + audit |
| v0.3.0 | D24 (Φ-B) | Lettre E + Go SDK + TS SDK + AdminService 14 RPCs + mTLS |
| v0.4.x | D27 (Φ-C) | Live bundle reload |
| v0.6.x | D32 (Φ-D) | Streaming Mode-1 + GPU runtime |
| v0.7.0 | D36 (Φ-E) | Mode-2 + LLM routers |
| v0.8.0 | D39 (Φ-F) | Mode-3 multi-turn |
| **v1.0.0 GA** | **D42** | **toutes S-1..S-20** |

### §4.3 Gating entre versions

Aucun tag release n'est posé tant que :

* Cargo workspace tests + Python pin tests GREEN.
* Pin tests audit institutionnels GREEN (test_audit_<sprint>_*).
* `helm install efe ./deploy/helm/efe --set runtime.gpuEnabled=true` succeed sur kind.
* Backward-compat : tous les tests Mode-1 D4-D18 GREEN.

---

## §5. Principes directeurs

### §5.1 Principes opposables en revue PR

1. **Backward compat sacrée**. Aucun client Mode-1 D4-D18
   ne casse. Si un test D4 RED, le PR est bloqué.
2. **Wiring eager registration**. Tout service nouveau doit appeler
   `register_all_<sprint>()` dans `Server::build` AVANT le service
   correspondant — sinon dashboards Grafana montrent "no data".
   Recurrence D17-AC2 / D18-A3 / K-A1 / F-A1 — pin test pour
   chaque nouveau sprint.
3. **Scope deny-by-default**. Un nouveau RPC sans `#[scope("...")]`
   refuse implicitement (compile-time error via macro).
4. **Audit chain pour tous les writes**. Tout RPC qui modifie
   l'état (Reload, Pin, Delete, Mode-2 anchors) émet un event
   signé HMAC avant le response.
5. **Streaming back-pressure**. Tout server-stream doit observer
   `client_disconnect` via `tonic::Status::Cancelled` et clean up
   ressources ≤ 1s. Pin test obligatoire.
6. **No silent fallback**. Si Mode-2 LLM router échoue, refuse
   avec `Status::Unavailable` ; ne fallback PAS sur Mode-1
   silently. Operator config explicite.
7. **Determinism preservation**. Mode-2 / Mode-3 ne brisent pas
   la determinism Mode-1 (cache anchors par hash, seed propagé
   à tous les sub-swarms).
8. **mTLS strict en prod**. Refus de connexion non-mTLS au boot
   quand `EFE_RUNTIME_MTLS_REQUIRED=true`. Permissive en dev opt-in.
9. **gRPC error code mapping**. Toutes les erreurs Rust mappent
   à un code gRPC standard (cf. §15).
10. **Proto-driven evolution**. Toute extension proto MUST passer
    `buf breaking` check en CI (no breaking changes sans bump
    major version proto).

### §5.2 Ce que `efe-server` refuse d'implémenter

* **Authn HTTP basic / API keys**. JWT-only en GA ; API keys
  hors-scope.
* **Plugins runtime** (WASM, dynamic load). Tout doit être
  compile-time link.
* **Cross-tenant routing** (un tenant talks à l'autre). Strict
  isolation.
* **Synchronous LLM call from sync handler**. Mode-2 LLM call
  est async via tokio.

---

## §6. Invariants structurels

### §6.1 Invariants de structure de fichiers

* `crates/efe-server/src/services/` accueille **un fichier par
  service** :
  * `swarm.rs` — Mode1Service (D4 + D32)
  * `mode2.rs` — Mode2Service (D34 NEW)
  * `mode3.rs` — Mode3Service (D37 NEW)
  * `admin.rs` — AdminService (D9 + D21)
  * `catalog/` — CatalogService (D12-bis ✅)
* `protos/efe/v1/` accueille **un fichier proto par service** :
  * `mode1.proto`, `mode2.proto` (NEW), `mode3.proto` (NEW),
    `admin.proto`, `catalog.proto`.
* `crates/efe-server/src/auth/` :
  * `mtls.rs` (NEW D22) — cert chain verification.
  * `scope.rs` (NEW D22) — scope guard macro + middleware.

### §6.2 Invariants de runtime

* Aucun handler ne tient le GIL Python (n/a, pas de PyO3 ici).
* Aucun handler ne hold un mutex pendant un await sur réseau.
* Toute écriture DB passe par audit chain (D14-bis pattern).
* Toute requête traverse 4 couches dans l'ordre : mTLS → JWT →
  scope → handler.
* Multi-replica : tout RPC est idempotent ou guarde via
  `IdempotencyStore` (D18).

### §6.3 Invariants de versioning

* Proto schema_version : major bump = breaking ; minor = additif.
* Tous les SDKs (Go, TS, Java, Python) re-compilent sur le même
  proto file via `buf generate`.
* `efe-server` minor version aligne avec le crate workspace.

---

## §7. Vue d'ensemble — 6 sprints, 6 livrables, dependency graph

### §7.1 Tableau d'agrégation

| Sprint | Phase | Wallclock | Livrable | Dépendances entrantes |
|:------:|:-----:|:---------:|----------|------------------------|
| **D21** | Φ-B | 2 sem | AdminService 8 RPCs catalog | D14 PG, D14b audit, D15 OIDC |
| **D22** | Φ-B | 2 sem | mTLS + 3 scopes JWT | D21 (AdminService consomme scope=admin) |
| **D27** | Φ-C | 2 sem | Live bundle reload | D16 EmbedderRegistry, D21 AdminService.ReloadBundle |
| **D32** | Φ-D | 2 sem | Streaming Mode-1 | D29 GPU runtime (optional) |
| **D34** | Φ-E | 2 sem | Mode-2 service | D33 templates, D34 LlmRouter trait |
| **D37** | Φ-F | 2 sem | Mode-3 service | D36 Mode-2 stable |

**Total** : 12 semaines wallclock, ~60 jours-h.

### §7.2 Dépendances inter-sprints

```
   D14 PG ──┐
   D14b audit ┼──> D21 AdminService catalog (Φ-B)
   D15 OIDC ──┘                │
                                ▼
                          D22 mTLS + scopes (Φ-B)
                                │
              ┌─────────────────┼──────────────────┐
              ▼                 ▼                  ▼
        D27 reload        D32 streaming       (D33-D36 LLM
        (Φ-C)             (Φ-D)               routers ext)
                                                   │
                                                   ▼
                                            D34 Mode-2 (Φ-E)
                                                   │
                                                   ▼
                                            D37 Mode-3 (Φ-F)
```

### §7.3 Volumétrie code attendue

| Sprint | Lignes Rust | Lignes Helm | Lignes proto | Lignes tests |
|:------:|:-----------:|:-----------:|:------------:|:------------:|
| D21 | ~600 | +50 | +120 | +250 |
| D22 | ~450 | +80 | 0 | +200 |
| D27 | ~350 | +30 | +20 | +150 |
| D32 | ~500 | 0 | +60 | +180 |
| D34 | ~800 | +40 | +200 | +300 |
| D37 | ~600 | 0 | +80 | +200 |
| **Total** | **~3 300** | **~200** | **~480** | **~1 280** |

≈ **5 200 lignes nettes** sur 6 sprints.

---

## §8. Surface gRPC publique cible

### §8.1 Inventaire post-D37 (GA)

| # | Service | RPC | Type | Sprint | Scope |
|--:|---------|-----|:----:|:------:|:-----:|
| 1 | Mode1Service | Execute | unary | D4 ✅ | data |
| 2 | Mode1Service | DescribeSpec | unary | D4 ✅ | data |
| 3 | Mode1Service | ExecuteStreaming | server-stream | D32 | data |
| 4 | Mode2Service | Execute | unary | D34 | data |
| 5 | Mode2Service | ExecuteStreaming | server-stream | D34 | data |
| 6 | Mode2Service | RouterHealth | unary | D34 | data |
| 7 | Mode3Service | Execute | unary | D37 | data |
| 8 | Mode3Service | SubscribeVerdict | server-stream | D37 | data |
| 9 | Mode3Service | Cancel | unary | D37 | data |
| 10 | AdminService | RegisterCone | unary | D9 ✅ | admin |
| 11 | AdminService | EvolveCone | unary | D9 ✅ | admin |
| 12 | AdminService | GetCone | unary | D9 ✅ | admin |
| 13 | AdminService | ListVersions | unary | D9 ✅ | admin |
| 14 | AdminService | DeleteCone | unary | D9 ✅ | admin |
| 15 | AdminService | BreachHistory | unary | D9 ✅ | audit |
| 16 | AdminService | ListCatalogs | unary | D21 | admin |
| 17 | AdminService | GetCatalog | unary | D21 | admin |
| 18 | AdminService | DeleteCatalog | unary | D21 | admin |
| 19 | AdminService | ReloadBundle | unary | D21+D27 | admin |
| 20 | AdminService | PinCatalog | unary | D21 | admin |
| 21 | AdminService | EvolveCatalog | unary | D21 | admin |
| 22 | AdminService | ListAuditEvents | server-stream | D21 | audit |
| 23 | AdminService | ReloadCatalog | unary | D21 | admin |
| 24 | CatalogService | Register | unary | D12-bis ✅ | data |
| 25 | CatalogService | Evolve | unary | D12-bis ✅ | data |
| 26 | CatalogService | Resolve | unary | D12-bis ✅ | data |
| 27 | CatalogService | Pin | unary | D12-bis ✅ | data |
| 28 | HealthService | Check | unary | D9 ✅ | (no auth) |
| 29 | HealthService | Watch | server-stream | D9 ✅ | (no auth) |
| 30 | ServerReflection | (multiple) | bidi | D4 ✅ | (no auth) |

**Total GA** : **30 RPCs distincts**.

### §8.2 Mapping scope → service

| Scope | Services accessibles | RPCs |
|-------|----------------------|:----:|
| `data` | Mode1Service, Mode2Service, Mode3Service, CatalogService | 13 |
| `admin` | AdminService (write RPCs) | 13 |
| `audit` | AdminService (read-only) | 2 (BreachHistory, ListAuditEvents) |
| (no auth) | HealthService, ServerReflection | 3 |

---

## §9. Sprint **D21** — AdminService 8 RPCs

### §9.1 Objectif

Étendre l'`AdminServiceImpl` post-D9 (6 RPCs cone-scoped) avec
**8 nouveaux RPCs catalog-scoped** pour gérer le cycle de vie des
catalogs : list, get, delete, reload, pin, evolve, listauditevents,
reloadcatalog.

### §9.2 Architecture du proto

`protos/efe/v1/admin.proto` étendu :

```protobuf
service AdminService {
  // Existants D9 — cone management
  rpc RegisterCone(RegisterConeRequest) returns (RegisterConeResponse);
  rpc EvolveCone(EvolveConeRequest) returns (EvolveConeResponse);
  rpc GetCone(GetConeRequest) returns (GetConeResponse);
  rpc ListVersions(ListVersionsRequest) returns (ListVersionsResponse);
  rpc DeleteCone(DeleteConeRequest) returns (DeleteConeResponse);
  rpc BreachHistory(BreachHistoryRequest) returns (BreachHistoryResponse);

  // NEW D21 — catalog management
  rpc ListCatalogs(ListCatalogsRequest) returns (ListCatalogsResponse);
  rpc GetCatalog(GetCatalogRequest) returns (GetCatalogResponse);
  rpc DeleteCatalog(DeleteCatalogRequest) returns (DeleteCatalogResponse);
  rpc ReloadBundle(ReloadBundleRequest) returns (ReloadBundleResponse);
  rpc PinCatalog(PinCatalogRequest) returns (PinCatalogResponse);
  rpc EvolveCatalog(EvolveCatalogRequest) returns (EvolveCatalogResponse);
  rpc ReloadCatalog(ReloadCatalogRequest) returns (ReloadCatalogResponse);

  // NEW D21 — audit access (server-stream, scope=audit)
  rpc ListAuditEvents(ListAuditEventsRequest)
    returns (stream AuditEvent);
}
```

### §9.3 Détail des 8 nouveaux RPCs

#### `ListCatalogs`
* **Input** : `ListCatalogsRequest { tenant_id, page_size, page_token }`
* **Output** : `ListCatalogsResponse { catalogs: repeated CatalogSummary, next_page_token }`
* **Backend** : `CatalogRegistry::list_for_tenant(tenant_id, paginated)`.
* **Audit** : `audit_emitter.emit(action="catalog.list", tenant_id)`.

#### `GetCatalog`
* **Input** : `{ tenant_id, catalog_id, version }` (version Optional ; latest si absent)
* **Output** : `GetCatalogResponse { catalog: Catalog, fingerprint }`
* **Backend** : `CatalogRegistry::get(catalog_id, version)`.

#### `DeleteCatalog`
* **Input** : `{ tenant_id, catalog_id, version, soft_delete: bool }`
* **Output** : `{ deleted: bool, audit_id }`
* **Audit** : event obligatoire ; `soft_delete=true` recommandé prod (mark deleted in PG, keep row).

#### `ReloadBundle` (D21 : RPC déclaré ; D27 wiring complet)
* **Input** : `{ bundle_oci_ref OR bundle_path, tenant_id }`
* **Output** : `{ reload_id, status: enum (Pending/Loading/SmokeOk/Active/Failed) }`
* **D21 status** : RPC declared, returns `Status::Unimplemented("D27 pending")`.
* **D27 status** : full wiring (cf. §11).

#### `PinCatalog`
* **Input** : `{ tenant_id, catalog_id, version, pin_until_unix_ms }`
* **Output** : `{ pinned: bool, expires_at }`
* **Backend** : `CatalogRegistry::pin(catalog_id, version, expires_at)`.
* **Use case** : pin a known-good version while testing a new one.

#### `EvolveCatalog`
* **Input** : `{ tenant_id, base_catalog_id, base_version, evolution: CatalogEvolution }`
* **Output** : `{ new_catalog_id, new_version, audit_id }`
* **Backend** : `CatalogRegistry::evolve(base, evolution)`.
* **Audit** : event with full evolution payload.

#### `ReloadCatalog`
* **Input** : `{ tenant_id, catalog_id }`
* **Output** : `{ reloaded: bool, version }`
* **Backend** : refresh in-memory cache from PG.

#### `ListAuditEvents` (server-stream)
* **Input** : `{ tenant_id, since_unix_ms, until_unix_ms, action_filter? }`
* **Output** : stream of `AuditEvent { id, ts, action, tenant_id, signed_payload, signature }`
* **Backend** : `audit_events` table query streamed in chunks of 100.
* **Scope** : `audit` (read-only).

### §9.4 Handler implementation

```rust
// crates/efe-server/src/services/admin.rs (extension)

use efe_core::AdminCatalogPort;
use crate::auth::scope::ScopeGuard;
use crate::audit::AuditEmitter;

pub struct AdminServiceImpl {
    // Existing D9 fields
    cone_registry: Arc<dyn AlignmentRegistry>,
    auth: Arc<AuthGuard>,
    rate_limiter: Option<Arc<BucketRegistry>>,
    
    // NEW D21 fields
    catalog_registry: Arc<dyn CatalogRegistry>,
    audit_emitter: Arc<AuditEmitter>,
    embedder_registry: Arc<EmbedderRegistry>,  // D27 wiring
}

#[tonic::async_trait]
impl admin_service_server::AdminService for AdminServiceImpl {
    // Existing 6 RPCs preserved unchanged.

    async fn list_catalogs(
        &self,
        request: Request<ListCatalogsRequest>,
    ) -> Result<Response<ListCatalogsResponse>, Status> {
        // 1. Auth + scope check
        let claims = self.auth.verify_request(&request)?;
        ScopeGuard::require(&claims, "admin")?;
        
        // 2. Rate limit
        if let Some(rl) = &self.rate_limiter {
            rl.check_or_status(&claims.tenant_id, "admin.list_catalogs", 1.0)
                .await?;
        }
        
        // 3. Backend call
        let req = request.into_inner();
        let catalogs = self.catalog_registry
            .list_for_tenant(&req.tenant_id, req.page_size, req.page_token.as_deref())
            .await
            .map_err(map_catalog_error_to_status)?;
        
        // 4. Audit emit (read action)
        self.audit_emitter.emit_async(audit::AuditEvent {
            action: "catalog.list".into(),
            tenant_id: req.tenant_id,
            payload: serde_json::json!({"page_size": req.page_size}),
            ..Default::default()
        }).await;
        
        // 5. Build response
        Ok(Response::new(ListCatalogsResponse { catalogs, next_page_token: ... }))
    }

    // ... 7 other catalog RPCs follow same pattern
}
```

### §9.5 Tests + pin tests D21

* **Unit tests** : 8 RPCs × 3 scenarios (happy / unauthorized / not-found) = 24 tests.
* **Integration tests** : `tests/d21_admin_catalog_integration.rs` end-to-end avec PG embedded.
* **Pin tests Python** : `scripts/tests/test_d21_admin_8_rpcs.py` :
  * Each RPC declared in proto.
  * Each RPC has handler in admin.rs.
  * Each RPC checks scope=admin (or audit for ListAuditEvents).
  * Each RPC emits audit event.

### §9.6 Critères Go/No-Go D21

* GG-D21.1 : 8 RPCs GREEN (24 unit tests + 8 integration tests).
* GG-D21.2 : Audit chain shows entries pour chaque RPC tested.
* GG-D21.3 : Pin tests Python `test_d21_admin_8_rpcs.py` GREEN.
* GG-D21.4 : Backward compat — 6 cone RPCs préservés intacts.
* GG-D21.5 : v0.3.0-rc1 tag prêt.

---

## §10. Sprint **D22** — mTLS + 3 scopes JWT

### §10.1 Objectif

Imposer **mTLS obligatoire en production** + introduire la
**hiérarchie de scopes JWT** (`data` / `admin` / `audit`) avec
deny-by-default et per-RPC scope guard.

### §10.2 mTLS architecture

#### Stratégie globale

* **Production (Tier-2/3)** : `EFE_RUNTIME_MTLS_REQUIRED=true`. Boot
  refuse si pas de cert client validé pour chaque connexion.
* **Dev (Tier-1)** : `EFE_RUNTIME_MTLS_REQUIRED=false` (default).
  TLS optionnel ; sans cert client, JWT seul suffit.

#### Cert chain

```
Trust anchor : Helm-installed CA (cert-manager Issuer)
   │
   └── Root CA (efe-ca)
       │
       ├── Server cert (efe-server)        — used by tonic server TLS
       │
       └── Client certs (per consumer)      — one per tenant or service
           ├── tenant-acme-client.crt
           ├── tenant-bigco-client.crt
           └── ...
```

#### Implementation

```rust
// crates/efe-server/src/auth/mtls.rs (NEW D22)

use rustls::server::WebPkiClientVerifier;
use rustls::RootCertStore;
use std::sync::Arc;

/// **Audit fix S-A1** — mTLS configuration is built at server boot
/// from the cert files mounted by Helm. The `client_ca_file` is
/// the trust root for client certs ; without it, mTLS is impossible
/// to enforce.
pub struct MtlsConfig {
    pub server_cert_path: PathBuf,
    pub server_key_path: PathBuf,
    pub client_ca_path: Option<PathBuf>,  // None = no mTLS
    pub required: bool,
}

impl MtlsConfig {
    /// Build the rustls server config. When `required = true` and
    /// `client_ca_path = None`, refuses construction (config error).
    pub fn build_server_config(&self) -> Result<rustls::ServerConfig, MtlsError> {
        let server_cert = load_cert(&self.server_cert_path)?;
        let server_key = load_private_key(&self.server_key_path)?;
        
        let client_verifier = if let Some(ca_path) = &self.client_ca_path {
            let mut roots = RootCertStore::empty();
            roots.add(load_cert(ca_path)?[0].clone())?;
            WebPkiClientVerifier::builder(Arc::new(roots)).build()?
        } else if self.required {
            return Err(MtlsError::MissingClientCaButRequired);
        } else {
            // No mTLS verification — TLS only.
            return rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(server_cert, server_key)
                .map_err(MtlsError::from);
        };
        
        rustls::ServerConfig::builder()
            .with_client_cert_verifier(client_verifier)
            .with_single_cert(server_cert, server_key)
            .map_err(MtlsError::from)
    }
}
```

### §10.3 Scope JWT hierarchy

#### Claim format

JWT claim `scope` est un **string space-separated** (RFC 6749) :

```json
{
  "iss": "https://auth.efe.ai",
  "aud": "efe.v1",
  "sub": "user-12345",
  "tenant_id": "acme-corp",
  "scope": "data admin",    ← ICI : space-separated tokens
  "exp": 1735689600
}
```

#### Three canonical scopes

| Scope | Permits | Refuses |
|-------|---------|---------|
| `data` | Mode1/2/3 RPCs + CatalogService 4 RPCs | AdminService |
| `admin` | AdminService write RPCs (12) | (cap 13 RPCs total) |
| `audit` | AdminService read-only (BreachHistory + ListAuditEvents) | data + admin writes |

Un JWT peut avoir **plusieurs scopes** : `"scope": "data admin audit"`.

#### ScopeGuard implementation

```rust
// crates/efe-server/src/auth/scope.rs (NEW D22)

/// **Audit fix S-A2** — pre-fix the existing `auth/scopes.rs`
/// parsed scopes but no per-RPC guard enforced them. Every
/// `AdminService` RPC fell through with no scope check beyond
/// generic JWT validation. This module introduces the
/// `ScopeGuard::require` enforcement that EVERY handler MUST
/// call as the first auth step.
pub struct ScopeGuard;

impl ScopeGuard {
    /// Require that `claims.scope` contains the given token.
    /// Returns `tonic::Status::permission_denied` otherwise.
    ///
    /// Panic-free : if `claims.scope` is empty, returns Err.
    pub fn require(claims: &Claims, scope: &str) -> Result<(), Status> {
        let granted = claims.scope.split_whitespace().any(|s| s == scope);
        if granted {
            Ok(())
        } else {
            tracing::warn!(
                target: "efe.auth.scope",
                tenant_id = %claims.tenant_id,
                requested = scope,
                granted = %claims.scope,
                "scope check failed"
            );
            Err(Status::permission_denied(format!(
                "missing scope: {scope}"
            )))
        }
    }
}
```

### §10.4 Per-RPC scope guard pattern

Every handler starts with :

```rust
async fn list_catalogs(...) -> Result<Response<...>, Status> {
    let claims = self.auth.verify_request(&request)?;
    ScopeGuard::require(&claims, "admin")?;  // MANDATORY first line
    // ... rest of handler
}
```

For read-only audit RPCs :

```rust
async fn list_audit_events(...) -> ... {
    let claims = self.auth.verify_request(&request)?;
    ScopeGuard::require(&claims, "audit")?;  // audit-scope only
    // ...
}
```

### §10.5 Helm cert-manager wiring

```yaml
# deploy/helm/efe/templates/d22-mtls-cert.yaml
{{- if and .Values.mtls .Values.mtls.enabled }}
apiVersion: cert-manager.io/v1
kind: Issuer
metadata:
  name: {{ include "efe.fullname" . }}-mtls-issuer
spec:
  selfSigned: {}
---
apiVersion: cert-manager.io/v1
kind: Certificate
metadata:
  name: {{ include "efe.fullname" . }}-mtls-server
spec:
  secretName: {{ include "efe.fullname" . }}-mtls-server-tls
  issuerRef:
    name: {{ include "efe.fullname" . }}-mtls-issuer
    kind: Issuer
  commonName: {{ include "efe.fullname" . }}.{{ .Release.Namespace }}.svc
  dnsNames:
    - {{ include "efe.fullname" . }}.{{ .Release.Namespace }}.svc
    - {{ include "efe.fullname" . }}.{{ .Release.Namespace }}.svc.cluster.local
  duration: 720h        # 30 days
  renewBefore: 240h     # 10 days before expiry
  privateKey:
    algorithm: ECDSA
    size: 256
{{- end }}
```

### §10.6 Tests D22

* **Unit tests** : 5 mTLS edge cases (cert valid / expired / wrong CA / missing client cert / required=false).
* **Unit tests scope** : 9 tests (each scope × 3 outcomes).
* **Integration test** : `tests/d22_mtls_integration.rs` — boot server with mTLS required, connect with valid + invalid certs.
* **Pin tests** : `scripts/tests/test_d22_mtls_3_scopes.py` :
  * Each Mode1/2/3 RPC has `ScopeGuard::require("data")?`.
  * Each AdminService write RPC has `ScopeGuard::require("admin")?`.
  * Audit RPCs have `ScopeGuard::require("audit")?`.

### §10.7 Critères Go/No-Go D22

* GG-D22.1 : `EFE_RUNTIME_MTLS_REQUIRED=true` boot refuses missing CA.
* GG-D22.2 : 9 scope unit tests + 5 mTLS unit tests GREEN.
* GG-D22.3 : Pin tests Python verify scope guard at every handler.
* GG-D22.4 : Helm `cert-manager.io/v1 Issuer + Certificate` template ships.
* GG-D22.5 : v0.3.0 RC tag.

---

## §11. Sprint **D27** — Live bundle reload

### §11.1 Objectif

Permettre le swap atomique d'un embedder bundle SANS restart pod
ni request dropping. RPC `AdminService.ReloadBundle` (déclaré en
D21) reçoit son wiring complet.

### §11.2 Sequence reload

```
Operator → AdminService.ReloadBundle({bundle_oci_ref})
                        │
                        ▼
   Phase 1 (≤ 5s) : verify
       - Pull OCI image to staging dir
       - Verify Ed25519 signature (D16)
       - Verify SHA-256 of every component
       - Smoke test : encode 10 fixed sentences ; assert L2-norm = 1
   
   Phase 2 (≤ 1s) : atomic swap
       - acquire WriteGuard on EmbedderRegistry.active
       - new_bundle = Arc::new(new_bundle)
       - old_bundle = swap(active, new_bundle)  ← atomic
       - release WriteGuard
   
   Phase 3 (≤ 30s drain) : drain
       - Spawn task that holds old Arc<Bundle>
       - Wait until Arc::strong_count(&old) == 1 (no in-flight RPC)
       - Drop old_bundle ; trigger Bundle::drop side effects (close ONNX session)
   
   Response : ReloadBundleResponse { reload_id, status: "Active", duration_ms }
```

### §11.3 EmbedderRegistry extension

```rust
// crates/efe-server/src/embedder/registry.rs (extension D27)

pub struct EmbedderRegistry {
    // Existing D16
    bundles: DashMap<String, Arc<Bundle>>,
    
    // NEW D27 — slot for the active bundle, atomically swapped.
    active: parking_lot::RwLock<Option<Arc<Bundle>>>,
    
    // NEW D27 — telemetry
    last_reload_ts: AtomicU64,
}

impl EmbedderRegistry {
    /// Atomic bundle swap. Returns the old `Arc<Bundle>` so the
    /// caller can spawn the drain task.
    pub async fn reload(
        &self,
        new_bundle_path: &Path,
        policy: LoadPolicy,
        trust: Arc<TrustStore>,
    ) -> Result<Arc<Bundle>, ReloadError> {
        // Phase 1 : load + verify
        let new_bundle = self.load_with_verify(new_bundle_path, policy, trust).await?;
        let new_arc = Arc::new(new_bundle);
        
        // Smoke test
        self.smoke_test(&new_arc).await?;
        
        // Phase 2 : atomic swap
        let mut guard = self.active.write();
        let old = guard.replace(Arc::clone(&new_arc))
            .ok_or(ReloadError::NoActiveBundle)?;
        drop(guard);
        
        self.last_reload_ts.store(now_unix_ms() as u64, Ordering::Relaxed);
        efe_telemetry::metrics::gpu_instruments::embedder_active_bundles()
            .set(1);  // still 1 active bundle ; old is draining
        
        Ok(old)  // caller spawns drain task
    }
    
    async fn smoke_test(&self, bundle: &Arc<Bundle>) -> Result<(), ReloadError> {
        let fixtures = ["hello", "world", "test 1", "test 2", "..."];
        for text in &fixtures {
            let emb = bundle.encode(text).map_err(ReloadError::SmokeEncode)?;
            let norm: f32 = emb.iter().map(|v| v * v).sum::<f32>().sqrt();
            if (norm - 1.0).abs() > 1e-3 {
                return Err(ReloadError::SmokeL2NormFail(norm));
            }
        }
        Ok(())
    }
}
```

### §11.4 Failure rollback

If Phase 1 (verify) or smoke test fails :

* New bundle dropped ; old bundle stays active.
* RPC returns `ReloadBundleResponse { status: "Failed", error: "..." }`.
* Audit event emitted with full error.
* `efe_embedder_reload_failures_total{reason}` counter bumped.

### §11.5 Multi-replica coordination

In multi-replica mode (D18), each replica receives the
`ReloadBundle` request **independently** via gRPC LB :

* Either : operator sends to one replica, who replicates via Postgres
  via a `bundle_reload_events` table (post-GA — D27 stub).
* Or : operator runs `for r in $(replicas); do grpcurl $r ReloadBundle; done`.

For D27, **per-replica reload** is acceptable. The
`bundle_reload_events` propagation is DT-S-D27-COORD-1 (post-GA).

### §11.6 Tests D27

* **Unit tests** : 6 scenarios (happy / sig fail / hash fail / smoke fail / no active / drain timeout).
* **Integration test** : `tests/d27_bundle_reload_integration.rs` boot server with one bundle, reload to a different one, verify in-flight RPCs complete on old bundle.
* **Pin tests** : `scripts/tests/test_d27_bundle_reload.py` :
  * `EmbedderRegistry::reload()` accessible from `AdminService.ReloadBundle` handler.
  * Smoke test fixtures hardcoded.
  * Drain task spawned post-swap.

### §11.7 Critères Go/No-Go D27

* GG-D27.1 : Reload p99 ≤ 30s on T4 + paraphrase-multilingual-MiniLM bundle.
* GG-D27.2 : 0 RPC dropped during reload (in-flight on old bundle complete).
* GG-D27.3 : Audit event for each reload (success or failure).
* GG-D27.4 : `efe_embedder_reload_total{result}` metric exposed.
* GG-D27.5 : v0.4.x tag.

---

## §12. Sprint **D32** — Streaming RPC

### §12.1 Objectif

Activer `Mode1Service.ExecuteStreaming` (skeleton existant) pour
émettre des chunks server-side au fur et à mesure des fusion rounds,
permettant aux clients d'afficher progress + early-abort sur
confidence threshold.

### §12.2 Proto extension

```protobuf
// protos/efe/v1/mode1.proto

service Mode1Service {
  rpc Execute(ExecuteRequest) returns (ExecuteResponse);
  rpc DescribeSpec(DescribeSpecRequest) returns (DescribeSpecResponse);
  rpc ExecuteStreaming(ExecuteRequest) returns (stream StreamChunk);
}

message StreamChunk {
  oneof payload {
    RoundUpdate round_update = 1;
    PartialVerdict partial = 2;
    FinalVerdict final = 3;
    Error error = 4;
  }
}

message RoundUpdate {
  uint32 round = 1;
  float variance = 2;
  float confidence_so_far = 3;
  uint32 active_agents = 4;
}

message PartialVerdict {
  string label = 1;
  float confidence = 2;
  bytes partial_centroid_quantized = 3;  // int8 quantized
}

message FinalVerdict {
  ExecuteResponse response = 1;  // same shape as Execute unary
}
```

### §12.3 Server-side streaming implementation

```rust
// crates/efe-server/src/services/swarm.rs (D32 activation)

pub type ExecuteStreamingStream =
    ReceiverStream<Result<StreamChunk, Status>>;

#[tonic::async_trait]
impl swarm_service_server::SwarmService for SwarmServiceImpl {
    type ExecuteStreamingStream = ExecuteStreamingStream;

    async fn execute_streaming(
        &self,
        request: Request<ExecuteRequest>,
    ) -> Result<Response<Self::ExecuteStreamingStream>, Status> {
        // 1. Auth + scope
        let claims = self.auth.verify_request(&request)?;
        ScopeGuard::require(&claims, "data")?;
        
        // 2. Rate limit (count as 1 RPS, no special weight)
        if let Some(rl) = &self.rate_limiter {
            rl.check_or_status(&claims.tenant_id, "mode1.streaming", 1.0).await?;
        }
        
        // 3. Build channel
        let (tx, rx) = mpsc::channel::<Result<StreamChunk, Status>>(64);
        
        // 4. Spawn task that drives the swarm + emits chunks
        let backend = self.backend.clone();
        let req = request.into_inner();
        tokio::spawn(async move {
            // Build SwarmController exec, but instead of awaiting
            // the full result, hook a callback that emits each
            // round update.
            let result = backend.execute_with_round_callback(
                req,
                |round_stats| {
                    // Send round update ; if client dropped tx,
                    // bail.
                    let _ = tx.try_send(Ok(StreamChunk {
                        payload: Some(stream_chunk::Payload::RoundUpdate(
                            RoundUpdate {
                                round: round_stats.round,
                                variance: round_stats.variance,
                                confidence_so_far: round_stats.confidence,
                                active_agents: round_stats.n_active,
                            }
                        )),
                    }));
                },
            ).await;
            
            // Emit final
            match result {
                Ok(response) => {
                    let _ = tx.send(Ok(StreamChunk {
                        payload: Some(stream_chunk::Payload::Final(
                            FinalVerdict { response: Some(response) }
                        )),
                    })).await;
                }
                Err(e) => {
                    let _ = tx.send(Err(map_efe_error_to_status(e))).await;
                }
            }
        });
        
        Ok(Response::new(ReceiverStream::new(rx)))
    }
}
```

### §12.4 Client cancellation

When client disconnects (gRPC `CANCELLED`), the `tx.try_send`
fails ; the spawned task should detect via `tx.is_closed()` after
each round and abort. Cleanup ≤ 1s (SLO S-10).

```rust
loop {
    backend.run_round(...);
    if tx.is_closed() {
        // Client cancelled
        backend.abort();
        return;
    }
    // ...
}
```

### §12.5 Backpressure

`mpsc::channel(64)` provides a 64-chunk buffer. If the client is
slow, `try_send` returns `Full` ; the server can either :

* (a) block (back-pressure) — risk of slowing down the swarm.
* (b) drop the chunk — risk of missing a round update.

For Mode-1 (typically 4-20 rounds, < 50 chunks total), the buffer
is rarely full. Default : (a) `tx.send(...).await` for ordering
guarantee.

### §12.6 Tests D32

* **Unit tests** : 4 scenarios (happy / client cancel / backend error / timeout).
* **Integration test** : `tests/d32_streaming_integration.rs` — full client→server stream with assertion on chunk count.
* **Pin tests** : `scripts/tests/test_d32_streaming.py` :
  * `ExecuteStreaming` no longer returns `Status::Unimplemented`.
  * Each chunk type carries a payload.
  * Final chunk always emitted (success or error).

### §12.7 Critères Go/No-Go D32

* GG-D32.1 : Streaming p99 first-chunk ≤ 50 ms (SLO S-2).
* GG-D32.2 : Client cancel triggers cleanup ≤ 1s.
* GG-D32.3 : Final chunk always emitted (no premature stream close).
* GG-D32.4 : v0.6.x tag.

---

## §13. Sprint **D34** — Mode-2 (anchors dynamiques + LLM router)

### §13.1 Objectif

Livrer le **Mode-2** : anchors générées dynamiquement par un LLM
de routage à chaque requête, permettant au swarm de gérer des
domaines hors-catalog. Latence cible : p99 ≤ 1500 ms (incluant
1 LLM call).

### §13.2 Proto

```protobuf
// protos/efe/v1/mode2.proto

service Mode2Service {
  rpc Execute(Mode2ExecuteRequest) returns (Mode2ExecuteResponse);
  rpc ExecuteStreaming(Mode2ExecuteRequest) returns (stream Mode2StreamChunk);
  rpc RouterHealth(google.protobuf.Empty) returns (RouterHealthResponse);
}

message Mode2ExecuteRequest {
  string query_text = 1;
  string llm_router_id = 2;  // "openai" | "anthropic" | "mock"
  uint32 top_k_anchors = 3;  // default 5
  string tenant_id = 4;
  // ... idem Mode1 (request_id, max_rounds, seed, …)
}

message Mode2ExecuteResponse {
  Verdict verdict = 1;
  RouterMetadata router = 2;
}

message RouterMetadata {
  string router_id = 1;
  uint32 anchors_generated = 2;
  uint32 latency_ms = 3;
  bool from_cache = 4;
  string llm_call_id = 5;
}
```

### §13.3 LlmRouter trait

```rust
// crates/efe-core/src/swarm/mode2/router.rs (NEW D34)

#[async_trait]
pub trait LlmRouter: Send + Sync + std::fmt::Debug {
    async fn generate_anchors(
        &self,
        query_text: &str,
        top_k: u32,
    ) -> Result<RouterOutput, LlmRouterError>;
}

#[derive(Debug, Clone)]
pub struct RouterOutput {
    pub anchors: Vec<DynamicAnchor>,
    pub llm_call_id: String,
    pub from_cache: bool,
    pub latency_ms: u32,
}

pub struct DynamicAnchor {
    pub label: String,
    pub centroid: Vec<f32>,
    pub confidence: f32,
}
```

### §13.4 Implementations (D35-D36 follow-up)

D34 ships :
* `MockRouter` (deterministic anchors via SHA-256 of query) — tests.
* `LlmRouter` trait + RouterError enum.
* Cache `Arc<DashMap<u64, RouterOutput>>` keyed on
  `hash(query_text, top_k, router_id)`. TTL 5 min default.

D35-D36 ship :
* `OpenAIRouter` (gpt-4o-mini default).
* `AnthropicRouter` (claude-haiku default).

### §13.5 Mode2ServiceImpl

```rust
// crates/efe-server/src/services/mode2.rs (NEW D34)

pub struct Mode2ServiceImpl {
    backend: DynExecuteBackend,         // shared with Mode1
    routers: HashMap<String, Arc<dyn LlmRouter>>,
    auth: Arc<AuthGuard>,
    rate_limiter: Option<Arc<BucketRegistry>>,
    audit_emitter: Arc<AuditEmitter>,
    anchor_cache: Arc<DashMap<u64, RouterOutput>>,
}

#[tonic::async_trait]
impl mode2_service_server::Mode2Service for Mode2ServiceImpl {
    async fn execute(
        &self,
        request: Request<Mode2ExecuteRequest>,
    ) -> Result<Response<Mode2ExecuteResponse>, Status> {
        let claims = self.auth.verify_request(&request)?;
        ScopeGuard::require(&claims, "data")?;
        if let Some(rl) = &self.rate_limiter {
            rl.check_or_status(&claims.tenant_id, "mode2.execute", 2.0).await?;  // 2× weight
        }
        
        let req = request.into_inner();
        let router = self.routers.get(&req.llm_router_id)
            .ok_or_else(|| Status::not_found(format!(
                "unknown router: {}", req.llm_router_id
            )))?;
        
        // Cache lookup
        let cache_key = hash_request(&req);
        let router_output = if let Some(cached) = self.anchor_cache.get(&cache_key) {
            cached.clone()
        } else {
            let out = router.generate_anchors(&req.query_text, req.top_k_anchors)
                .await
                .map_err(map_router_error_to_status)?;
            self.anchor_cache.insert(cache_key, out.clone());
            out
        };
        
        // Build SwarmSpec with dynamic anchors
        let spec = build_spec_from_dynamic_anchors(&req, &router_output)?;
        
        // Execute swarm
        let response = self.backend.execute(spec).await
            .map_err(map_efe_error_to_status)?;
        
        // Audit
        self.audit_emitter.emit_async(AuditEvent {
            action: "mode2.execute".into(),
            tenant_id: req.tenant_id,
            payload: serde_json::json!({
                "router_id": req.llm_router_id,
                "from_cache": router_output.from_cache,
                "anchors": router_output.anchors.len(),
            }),
            ..Default::default()
        }).await;
        
        Ok(Response::new(Mode2ExecuteResponse {
            verdict: Some(response.verdict),
            router: Some(RouterMetadata {
                router_id: req.llm_router_id,
                anchors_generated: router_output.anchors.len() as u32,
                latency_ms: router_output.latency_ms,
                from_cache: router_output.from_cache,
                llm_call_id: router_output.llm_call_id,
            }),
        }))
    }
    
    // execute_streaming + router_health follow same pattern
}
```

### §13.6 Tests D34

* **Unit tests** : 8 scenarios (each router × {happy, error, cache hit/miss}).
* **Integration test** : `tests/d34_mode2_integration.rs` with MockRouter.
* **Pin tests** : `scripts/tests/test_d34_mode2.py` :
  * Mode2Service registered in proto + handler.
  * Cache hit rate ≥ 50 % on synthetic load.
  * Audit event with router metadata.

### §13.7 Critères Go/No-Go D34

* GG-D34.1 : Mode-2 p99 ≤ 1500 ms (SLO S-3) avec MockRouter.
* GG-D34.2 : Cache hit rate ≥ 60 % sur fixtures (SLO S-11).
* GG-D34.3 : 3 routers (Mock + 2 stubs OpenAI/Anthropic) wired.
* GG-D34.4 : v0.7.0 RC tag.

---

## §14. Sprint **D37** — Mode-3 (multi-turn parallel)

### §14.1 Objectif

Livrer le **Mode-3** : k swarms parallèles + aggregator pour
self-consistency-style decisions critiques. p99 ≤ 3 × Mode-1.

### §14.2 Proto

```protobuf
// protos/efe/v1/mode3.proto

service Mode3Service {
  rpc Execute(Mode3ExecuteRequest) returns (Mode3ExecuteResponse);
  rpc SubscribeVerdict(SubscribeVerdictRequest)
    returns (stream Mode3StreamChunk);
  rpc Cancel(CancelRequest) returns (google.protobuf.Empty);
}

message Mode3ExecuteRequest {
  string query_text = 1;
  uint32 k = 2;  // number of parallel swarms ; max 5 (SLO S-12)
  string mode_per_call = 3;  // "mode1" | "mode2"
  string llm_router_id = 4;  // if mode_per_call = mode2
  string aggregator = 5;  // "mean" | "weighted" | "median"
  string tenant_id = 6;
  string request_id = 7;
  uint64 seed = 8;
}

message Mode3ExecuteResponse {
  Verdict final_verdict = 1;
  repeated SubVerdict sub_verdicts = 2;
  AggregationMetadata aggregation = 3;
}

message SubVerdict {
  uint32 swarm_idx = 1;
  Verdict verdict = 2;
  uint32 latency_ms = 3;
}

message AggregationMetadata {
  string strategy = 1;
  float total_confidence = 2;
  float dispersion = 3;  // std dev across sub-verdicts
}
```

### §14.3 Mode3Aggregator trait

```rust
// crates/efe-core/src/swarm/mode3/aggregator.rs (NEW D37)

pub trait Mode3Aggregator: Send + Sync + std::fmt::Debug {
    fn aggregate(
        &self,
        sub_verdicts: &[Verdict],
    ) -> Result<AggregationOutput, AggregationError>;
}

pub struct MeanAggregator;
pub struct WeightedAggregator { /* by confidence */ }
pub struct MedianAggregator;
```

### §14.4 Mode3ServiceImpl

```rust
// crates/efe-server/src/services/mode3.rs (NEW D37)

pub struct Mode3ServiceImpl {
    mode1_backend: Arc<Mode1ServiceImpl>,
    mode2_backend: Arc<Mode2ServiceImpl>,
    aggregators: HashMap<String, Box<dyn Mode3Aggregator>>,
    auth: Arc<AuthGuard>,
    rate_limiter: Option<Arc<BucketRegistry>>,
    audit_emitter: Arc<AuditEmitter>,
    inflight: Arc<DashMap<String, JoinHandle<()>>>,  // for Cancel
}

#[tonic::async_trait]
impl mode3_service_server::Mode3Service for Mode3ServiceImpl {
    async fn execute(
        &self,
        request: Request<Mode3ExecuteRequest>,
    ) -> Result<Response<Mode3ExecuteResponse>, Status> {
        let claims = self.auth.verify_request(&request)?;
        ScopeGuard::require(&claims, "data")?;
        let req = request.into_inner();
        
        // Hard cap k ≤ 5 (SLO S-12)
        let k = req.k.min(5).max(1);
        
        // Rate limit : k × cost (each sub-swarm counts).
        if let Some(rl) = &self.rate_limiter {
            rl.check_or_status(&claims.tenant_id, "mode3.execute", k as f32).await?;
        }
        
        // Spawn k parallel swarms via tokio::JoinSet
        let mut set = tokio::task::JoinSet::new();
        for i in 0..k {
            let mode = req.mode_per_call.clone();
            let m1 = self.mode1_backend.clone();
            let m2 = self.mode2_backend.clone();
            let r = req.clone();
            set.spawn(async move {
                match mode.as_str() {
                    "mode1" => execute_mode1_inner(&m1, &r, i).await,
                    "mode2" => execute_mode2_inner(&m2, &r, i).await,
                    _ => Err(Status::invalid_argument("unknown mode_per_call")),
                }
            });
        }
        
        // Collect with deadline
        let mut sub_verdicts = Vec::with_capacity(k as usize);
        while let Some(res) = set.join_next().await {
            match res {
                Ok(Ok(verdict)) => sub_verdicts.push(verdict),
                Ok(Err(status)) => return Err(status),
                Err(join_err) => return Err(Status::internal(format!("{join_err}"))),
            }
        }
        
        // Aggregate
        let aggregator = self.aggregators.get(&req.aggregator)
            .ok_or_else(|| Status::not_found(format!(
                "unknown aggregator: {}", req.aggregator
            )))?;
        let agg_output = aggregator.aggregate(&sub_verdicts)
            .map_err(map_aggregation_error_to_status)?;
        
        // Audit
        self.audit_emitter.emit_async(AuditEvent {
            action: "mode3.execute".into(),
            tenant_id: req.tenant_id,
            payload: serde_json::json!({
                "k": k,
                "mode_per_call": req.mode_per_call,
                "aggregator": req.aggregator,
                "dispersion": agg_output.dispersion,
            }),
            ..Default::default()
        }).await;
        
        Ok(Response::new(Mode3ExecuteResponse {
            final_verdict: Some(agg_output.final_verdict),
            sub_verdicts,
            aggregation: Some(AggregationMetadata { ... }),
        }))
    }
}
```

### §14.5 Tests D37

* **Unit tests** : 12 scenarios (3 aggregators × 4 cases).
* **Integration** : `tests/d37_mode3_integration.rs` with k=3 mock backend.
* **Pin tests** : `test_d37_mode3.py` validates k_max enforcement, aggregator selection, audit emit.

### §14.6 Critères Go/No-Go D37

* GG-D37.1 : Mode-3 p99 ≤ 3 × Mode-1 p99 (SLO S-4).
* GG-D37.2 : k_max=5 enforced.
* GG-D37.3 : Aggregator dispersion correctly reported.
* GG-D37.4 : v0.8.0 tag.

---

## §15. Authorization matrix

### §15.1 Inventaire complet

| RPC | Required scope | Auth |
|-----|:--------------:|:----:|
| Mode1.Execute | data | JWT + mTLS |
| Mode1.DescribeSpec | data | JWT + mTLS |
| Mode1.ExecuteStreaming | data | JWT + mTLS |
| Mode2.Execute | data | JWT + mTLS |
| Mode2.ExecuteStreaming | data | JWT + mTLS |
| Mode2.RouterHealth | data | JWT + mTLS |
| Mode3.Execute | data | JWT + mTLS |
| Mode3.SubscribeVerdict | data | JWT + mTLS |
| Mode3.Cancel | data | JWT + mTLS |
| Catalog.Register | data | JWT + mTLS |
| Catalog.Evolve | data | JWT + mTLS |
| Catalog.Resolve | data | JWT + mTLS |
| Catalog.Pin | data | JWT + mTLS |
| Admin.RegisterCone | admin | JWT + mTLS |
| Admin.EvolveCone | admin | JWT + mTLS |
| Admin.GetCone | admin | JWT + mTLS |
| Admin.ListVersions | admin | JWT + mTLS |
| Admin.DeleteCone | admin | JWT + mTLS |
| Admin.BreachHistory | audit | JWT + mTLS |
| Admin.ListCatalogs | admin | JWT + mTLS |
| Admin.GetCatalog | admin | JWT + mTLS |
| Admin.DeleteCatalog | admin | JWT + mTLS |
| Admin.ReloadBundle | admin | JWT + mTLS |
| Admin.PinCatalog | admin | JWT + mTLS |
| Admin.EvolveCatalog | admin | JWT + mTLS |
| Admin.ReloadCatalog | admin | JWT + mTLS |
| Admin.ListAuditEvents | audit | JWT + mTLS |
| Health.Check | (none) | none (probe-friendly) |
| Health.Watch | (none) | none |
| Reflection.* | (none) | none (proto introspection) |

---

## §16. Audit chain integration

Every state-modifying RPC emits an audit event :

```
Modifying RPCs (write actions) :
   Mode2/3.Execute       : action="mode<N>.execute"
   Catalog.{Register,Evolve,Pin}  : action="catalog.<op>"
   Admin.RegisterCone    : action="cone.register"
   Admin.EvolveCone      : action="cone.evolve"
   Admin.DeleteCone      : action="cone.delete"
   Admin.{List,Get}Catalog        : action="catalog.<op>"  (read but tracked)
   Admin.DeleteCatalog   : action="catalog.delete"
   Admin.ReloadBundle    : action="bundle.reload"
   Admin.PinCatalog      : action="catalog.pin"
   Admin.EvolveCatalog   : action="catalog.evolve"
   Admin.ReloadCatalog   : action="catalog.reload"

Read-only RPCs (no audit) :
   Mode1.Execute / Mode1.DescribeSpec / Mode1.ExecuteStreaming
   Mode2.RouterHealth
   Catalog.Resolve
   Admin.GetCone / Admin.ListVersions
   Admin.BreachHistory / Admin.ListAuditEvents
   Health.* / Reflection.*
```

The chain head gauge is updated synchronously via D14-bis logic.

---

## §17. Rate limiting per-tenant — extension streaming

The D18 leaky bucket (`PgRateLimitStore` / `MockRateLimitStore`)
applies to all unary RPCs. For streaming, two strategies :

* **Stream-level** : 1 RPS = 1 call (counted on stream open).
* **Chunk-level** : N RPS where N = number of chunks emitted.

D32 / D34 / D37 use **stream-level** (default) — consistent with
gRPC convention and avoids penalising long streams.

The cost per RPC is per §13.5 : Mode-2 = 2× cost (LLM call) ;
Mode-3 = k × cost.

---

## §18. Observabilité — nouvelles métriques

### §18.1 Inventaire post-D37

15 nouveaux instruments (cumulative D21-D37) :

| # | Métrique | Type | Sprint |
|---|----------|:----:|:------:|
| 1 | `efe_admin_catalog_calls_total{rpc, outcome}` | Counter | D21 |
| 2 | `efe_admin_catalog_duration_seconds{rpc}` | Histogram | D21 |
| 3 | `efe_mtls_handshakes_total{outcome}` | Counter | D22 |
| 4 | `efe_jwt_scope_denials_total{scope_required}` | Counter | D22 |
| 5 | `efe_embedder_reload_total{result}` | Counter | D27 |
| 6 | `efe_embedder_reload_duration_seconds` | Histogram | D27 |
| 7 | `efe_embedder_active_bundles{tenant}` | Gauge | D27 |
| 8 | `efe_streaming_active{service}` | Gauge | D32 |
| 9 | `efe_streaming_chunks_emitted_total{service}` | Counter | D32 |
| 10 | `efe_streaming_first_chunk_seconds{service}` | Histogram | D32 |
| 11 | `efe_mode2_router_calls_total{router_id, outcome}` | Counter | D34 |
| 12 | `efe_mode2_anchor_cache_hit_ratio` | Gauge | D34 |
| 13 | `efe_mode2_router_latency_seconds{router_id}` | Histogram | D34 |
| 14 | `efe_mode3_dispersion` | Histogram | D37 |
| 15 | `efe_mode3_k_distribution` | Histogram | D37 |

Total cardinality post-D37 : ≈ 1 500 séries (sous budget D13 50 000).

### §18.2 Dashboards Grafana

3 nouveaux dashboards (D34/D37) :

* **EFE Admin** (D21+D27) : RPC counters + reload events + audit chain.
* **EFE Mode-2** (D34) : router latency + cache hit + LLM call cost.
* **EFE Mode-3** (D37) : k distribution + dispersion + parallelism.

---

## §19. Dashboards Grafana — Mode-1/2/3 panels

(Détail exhaustif des panels par dashboard — élidé pour concision ;
cf. `deploy/helm/efe/dashboards/` post-D37.)

---

## §20. SLO produit final

Cf. §4.1 contrat GA — 20 propriétés S-1..S-20.

---

## §21. Migration plan (D18 → D37)

* **D18 → D21** : aucun breaking change ; AdminService extended.
* **D21 → D22** : OPT-IN mTLS ; existing JWT-only clients still work.
* **D22 → D27** : ReloadBundle wired ; existing clients unchanged.
* **D27 → D32** : ExecuteStreaming activated ; existing Execute unary unchanged.
* **D32 → D34** : Mode2Service NEW ; existing Mode1 clients unchanged.
* **D34 → D37** : Mode3Service NEW ; existing Mode1/Mode2 unchanged.

Backward compat held through every sprint.

---

## §22. Coexistence avec clients legacy

* Python `EfeClient` (gRPC, D10) : Mode-1 only. Continues to work
  through all sprints.
* Python `EfeNativeClient` (PyO3, D25) : Mode-1 only via in-process.
  Mode-2/3 NOT exposed via PyO3 (data flow incompatible — Mode-2
  needs LLM call which ne convient pas in-process).
* Go SDK (D23) : Mode-1 only at v0.3.0. Adds Mode-2/3 in v0.7+.
* TS SDK (D24) : same.
* Java SDK (D26) : same.

---

## §23. Plans d'implémentation détaillés

### §23.1 D21 (10 jours)

| Jour | Livrable |
|:----:|----------|
| J1 | Proto extension admin.proto + buf generate |
| J2 | AdminServiceImpl extended + ListCatalogs handler |
| J3 | GetCatalog + DeleteCatalog handlers |
| J4 | ReloadBundle (Unimplemented stub) + PinCatalog |
| J5 | EvolveCatalog + ReloadCatalog |
| J6 | ListAuditEvents server-stream |
| J7 | Unit tests (24) |
| J8 | Integration test PG embedded |
| J9 | Pin tests Python + audit chain validation |
| J10 | Helm template update + RC tag |

### §23.2 D22 (10 jours)

| Jour | Livrable |
|:----:|----------|
| J1 | rustls config + auth/mtls.rs skeleton |
| J2 | Helm cert-manager Issuer + Certificate template |
| J3 | tonic transport server with mTLS |
| J4 | auth/scope.rs ScopeGuard + parsing |
| J5 | Wire ScopeGuard into all handlers (28 RPCs) |
| J6 | Mode1Service + Mode2Service stub + Mode3Service stub scope check |
| J7 | Unit tests (5 mTLS + 9 scope = 14 tests) |
| J8 | Integration test mTLS happy + sad paths |
| J9 | Pin tests + Helm validation |
| J10 | v0.3.0 tag |

### §23.3 D27, D32, D34, D37 detailed plans

(Similar 10-day plans — details elided for concision ; each follows
the pattern : proto → handler → tests → audit fixes → release.)

---

## §24. Risques transverses + atténuations

| # | Risque | Probabilité | Impact | Mitigation |
|--:|--------|:----------:|:------:|------------|
| S-R-1 | mTLS deployment breaks existing clients | Médium | Critique | Opt-in en D22 ; mandatory only en GA |
| S-R-2 | Live bundle reload corrupts in-flight requests | Faible | Critique | Atomic Arc swap + drain ≤ 30s + smoke test |
| S-R-3 | LLM router cost out of budget (Mode-2) | Médium | Hard | Cache 60 % hit + per-tenant rate limit |
| S-R-4 | Mode-3 k_max abuse | Médium | Hard | Hard cap k ≤ 5 ; per-tenant rate limit × k |
| S-R-5 | Streaming chunk back-pressure | Médium | Médium | Buffer 64 + back-pressure documented |
| S-R-6 | Scope guard forgotten on new RPC | Médium | Critique | Pin test institutionnel pour chaque sprint |
| S-R-7 | Audit chain growth unbounded | Médium | Hard | TTL 90j + S3 archival (post-GA) |
| S-R-8 | gRPC proto breaking change | Faible | Critique | `buf breaking` CI gate |
| S-R-9 | Multi-replica reload coordination | Médium | Hard | Per-replica reload + bundle_reload_events table (post-GA) |
| S-R-10 | Mode-2/3 amplifies error class | Médium | Médium | Cap aggregation dispersion ; refuse > threshold |

---

## §25. Critères Go/No-Go par sprint

Cf. §9.6, §10.7, §11.7, §12.7, §13.7, §14.6.

---

## §26. Annexes — proto definitions complètes

(Trois nouveaux proto files : `admin.proto` étendu, `mode2.proto`, `mode3.proto`.
Détail exhaustif des messages élidé ici ; cf. § 9.2, 13.2, 14.2 pour les contracts.)

---

## §27. Annexes — Helm template additions

(D22 mTLS + D27 reload — déjà détaillés en § 10.5 et § 11.)

---

## §28. Annexes — RACI + budget

| Activité | Core | Ops | SDK | Sec | PM |
|----------|:----:|:---:|:---:|:---:|:--:|
| D21 AdminService 8 RPCs | **R** | C | I | I | A |
| D22 mTLS + scopes | C | **R** | I | C | A |
| D27 Live bundle reload | **R** | I | I | I | A |
| D32 Streaming Mode-1 | **R** | I | C | I | A |
| D34 Mode-2 service | **R** | I | C | C | A |
| D37 Mode-3 service | **R** | I | C | I | A |

**Budget marginal** : 0 € (no new cloud services). LLM API costs
covered separately (D34).

---

## §29. Annexes — Gantt prévisionnel D21 → D37

```
   D21 D22  D23 D24  D25 D26 D27  D28 D29 D30 D31 D32  D33 D34 D35 D36  D37
M4 ███ ███   .   .    .   .  ███   .   .   .   .  ███   .  ███  .   .  ███
   ▲   ▲    (other module sprints — efe-ffi, efe-kernels)
   │   │
   AdminService    mTLS+scopes
   8 RPCs
```

Sprint cadence M4 : every 5 sprints in average over 17 sprints
(D21 → D37). M4 work density : 6 sprints out of 17 = 35 % of
the program calendar.

---

## §30. Mises à jour du document

### §30.1 Versioning

* **v1.0.0 (2026-04-28)** — première rédaction post-D18.
* Toute découverte audit-time / défaut résiduel / changement scope
  déclenche une révision avec :
  * Nouvelle entrée CHANGELOG.md.
  * Bump du Statut (en haut).
  * Pin test garde dans
    `scripts/tests/test_efe_server_doc_drift.py`.

### §30.2 Liens à maintenir

* `docs/PRODUCT_COMPLETION_ARCHITECTURE.md` §6 (Φ-B), §7 (Φ-C),
  §8 (Φ-D), §9 (Φ-E), §10 (Φ-F) — cohérence multi-phase.
* `crates/efe-server/Cargo.toml` — version sync.
* `protos/efe/v1/*.proto` — proto contract sync.
* `deploy/helm/efe/templates/*.yaml` — Helm template sync.

### §30.3 Critères d'archive

Ce document devient **historique** une fois GA 1.0 livré.
Les modifications post-GA partent dans
`EFE_SERVER_v1.1+_ARCHITECTURE.md`.

**Fin du document.**
