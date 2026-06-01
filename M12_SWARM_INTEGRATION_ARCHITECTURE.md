# M12 — Integration with EFE Swarm — Architecture détaillée

> **Statut** : Draft v1.0.0 — Document normatif pour le module de fusion
> Python ↔ Rust : la couche d'intégration entre la plateforme EFE
> (M01→M11) et le cœur ``efe-server`` / ``efe-core`` Rust qui exécute
> les essaims vectoriels.
> **Auteur** : rodrigue.pekar@icloud.com
> **Date** : 2026-04-23
> **Dépendances doc** : `M01..M11` architectures, `MODE1_ARCHITECTURE.md`,
> `M10_SCHEDULING_ARCHITECTURE.md`, `M11_SECURITY_ARCHITECTURE.md`,
> `protos/efe/v1/efe.proto`, `protos/efe/v1/types.proto`,
> `protos/efe/v1/admin.proto`.
> **Périmètre** : la couche de fusion qui porte les décisions de la
> plateforme (dérive, retraining, canary, rollout) jusqu'à l'essaim
> Rust, et qui reflète en sens inverse les verdicts et la télémétrie
> du swarm vers la plateforme pour piloter l'apprentissage continu.
> **Précision** : chirurgicale, RPC-par-RPC, champ-par-champ de
> protobuf, tag-par-tag de télémétrie.

---

## Sommaire

1. [Mission de M12](#1-mission-de-m12)
2. [Positionnement dans l'architecture EFE](#2-positionnement-dans-larchitecture-efe)
3. [Principes directeurs](#3-principes-directeurs)
4. [Invariants structurels](#4-invariants-structurels)
5. [Modèle de menace et hypothèses](#5-modèle-de-menace-et-hypothèses)
6. [Acteurs et topologie](#6-acteurs-et-topologie)
7. [Vue d'ensemble architecturale](#7-vue-densemble-architecturale)
8. [Sous-système 12.1 — **Transport gRPC**](#8-sous-système-121--transport-grpc)
9. [Sous-système 12.2 — **Contract Adapter (Protobuf bridge)**](#9-sous-système-122--contract-adapter-protobuf-bridge)
10. [Sous-système 12.3 — **Anchor Catalog Bridge**](#10-sous-système-123--anchor-catalog-bridge)
11. [Sous-système 12.4 — **Artefact Sync**](#11-sous-système-124--artefact-sync)
12. [Sous-système 12.5 — **SwarmClient (Python API)**](#12-sous-système-125--swarmclient-python-api)
13. [Sous-système 12.6 — **Verdict Reconciler**](#13-sous-système-126--verdict-reconciler)
14. [Sous-système 12.7 — **HBM / Resource Budgeting**](#14-sous-système-127--hbm--resource-budgeting)
15. [Sous-système 12.8 — **Backpressure & Circuit Breaker**](#15-sous-système-128--backpressure--circuit-breaker)
16. [Sous-système 12.9 — **Streaming Interface**](#16-sous-système-129--streaming-interface)
17. [Sous-système 12.10 — **Admin Bridge (Cone Registry)**](#17-sous-système-1210--admin-bridge-cone-registry)
18. [Sous-système 12.11 — **Telemetry Bridge**](#18-sous-système-1211--telemetry-bridge)
19. [Budget de performance](#19-budget-de-performance)
20. [Taxonomie d'erreurs](#20-taxonomie-derreurs)
21. [Stratégie de tests](#21-stratégie-de-tests)
22. [Plan de sprints](#22-plan-de-sprints)
23. [Critères Go / No-Go spécifiques à M12](#23-critères-go--no-go-spécifiques-à-m12)
24. **Annexes**
    - A. [Mapping protobuf ↔ Python ↔ Rust](#annexe-a--mapping-protobuf--python--rust)
    - B. [Matrice d'erreurs gRPC](#annexe-b--matrice-derreurs-grpc)
    - C. [DDL Postgres — tables M12](#annexe-c--ddl-postgres--tables-m12)
    - D. [Schéma YAML d'un catalogue d'ancres](#annexe-d--schéma-yaml-dun-catalogue-dancres)
    - E. [Matrice de risques M12](#annexe-e--matrice-de-risques-m12)
    - F. [Matrice RACI](#annexe-f--matrice-raci)
    - G. [Glossaire d'intégration](#annexe-g--glossaire-dintégration)

---

## §1. Mission de M12

### §1.1 Énoncé opérationnel

M12 est **le seul pont autorisé entre la plateforme Python et le cœur
Rust**. Son rôle unique et opposable est :

> *Étant donné une décision métier produite par la plateforme (un
> ``retrain_decision`` de M10, une promotion d'artefact de M07, une
> requête de classification de Mode 1), la traduire en un appel gRPC
> conforme à l'API ``efe.v1.SwarmService`` / ``efe.v1.admin.AdminService``
> du serveur Rust, consommer la réponse ou le flux streaming, reconcilier
> le verdict avec les modèles métier de la plateforme, et propager la
> télémétrie dans le sens inverse — tout cela sans que les couches
> métier aient jamais à connaître le protobuf, le HBM, les templates
> ``NaSem/NaLog/NaCrit`` ou les cônes d'alignement.*

Ce mandat se décompose en onze responsabilités non-chevauchantes :

1. **Encapsuler** le transport gRPC (TLS mutuel, retry, deadlines).
2. **Adapter** les contrats protobuf ↔ les dataclasses Python.
3. **Pont-des-ancres** — publier dans le registre de cônes les ancres
   matérialisées depuis les FeatureSpec / baselines de M04 / M09.
4. **Synchroniser** les artefacts ML (M05) vers les templates swarm
   consommés par ``efe-core::genesis``.
5. **Offrir** une API Python idiomatique (``SwarmClient``) aux modules
   consommateurs (M06 inférence, Mode 1 classifier, M10 scheduler).
6. **Reconcilier** le verdict du swarm (``DecodedOutput``, ``Verdict``)
   avec les modèles métier de la plateforme (label, confidence,
   alignment delta).
7. **Budgétiser** les ressources par requête et par tenant (HBM,
   wall-clock, watt-heures, recursion depth).
8. **Protéger** la plateforme des incidents côté swarm
   (backpressure, circuit breaker, bulkheads).
9. **Exposer** un mode streaming (SSE / bidi) pour l'inférence
   long-running et l'évaluation d'ensemble.
10. **Administrer** le cycle de vie des cônes : register, evolve,
    list, delete, breach history — via ``AdminService``.
11. **Relier** la télémétrie : chaque appel gRPC produit des spans
    OTel, des métriques Prometheus et des événements d'audit M11.

### §1.2 Ce que M12 **n'est pas**

- **Pas un moteur d'essaim** : M12 ne calcule aucun vecteur, aucun
  consensus, aucune projection. Tout cela vit dans Rust (``efe-core``).
- **Pas un embedder** : la transformation texte → vecteur est déléguée
  à Mode 1 (``clients/python/efe_tool``) ou à un service
  sentence-transformers dédié. M12 reçoit déjà des vecteurs.
- **Pas un orchestrateur** : M12 exécute des RPCs ponctuels, il ne
  coordonne pas des workflows multi-étapes. L'orchestration reste à
  M10 (scheduling) / M06 (inférence) / Mode 1 bridge.
- **Pas un IdP** : l'authentification mTLS et les JWT sont fournis par
  M11 ; M12 ne fait que transporter les credentials.
- **Pas un cache** : les résultats d'essaim ne sont pas cachés au
  niveau M12 (idempotence ⊂ M10, cache HTTP ⊂ gateway). M12 est un
  proxy *fidèle*.

### §1.3 Promesses externes

| Promesse | Valeur opposable |
|---|---|
| Toute requête Python → Rust transite par une unique surface `SwarmClient` | 1 seul point d'entrée, 1 seul Circuit Breaker |
| Toute erreur Rust est mappée vers la taxonomie Python | 1:1 avec `protos/efe/v1` + `grpc.StatusCode` |
| Aucun appel synchrone ne dépasse son deadline | `grpc.deadline` honoré, 499 côté client, 4 Deadline Exceeded côté serveur |
| Toute réponse reflète un seul artefact Rust connu | `artifact_digest` retourné par le serveur vérifié côté Python |
| Aucun champ protobuf n'est deviné | Mapping explicite maintenu dans `contract_adapter.py` + test de round-trip |
| Toute requête produit au moins 3 métriques | `rpc_latency`, `rpc_status`, `hbm_bytes_estimate` |
| Reconnexion transparente sur broker gRPC redémarré | Backoff exponentiel avec jitter, 5 tentatives max |
| Pas de backpressure leak | File bornée par tenant, 503 au-delà |

---

## §2. Positionnement dans l'architecture EFE

### §2.1 Vue globale de flux

```
 ┌───────────────────────────────────────────────────────────────┐
 │                   Python Platform (M01 → M11)                 │
 │                                                               │
 │  ┌─────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐           │
 │  │  M06    │ │  M07     │ │  M09     │ │  M10     │           │
 │  │Inference│ │Rollout   │ │Drift     │ │Scheduler │           │
 │  └────┬────┘ └────┬─────┘ └────┬─────┘ └────┬─────┘           │
 │       │           │            │            │                  │
 │       └───────────┴────────────┴────────────┘                  │
 │                         │                                      │
 │                         ▼                                      │
 │  ┌──────────────────────────────────────────────────────┐      │
 │  │              M12 — Swarm Integration                  │      │
 │  │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐  │      │
 │  │  │ Client   │ │ Adapter  │ │ Reconci- │ │Telemetry │  │      │
 │  │  │ (gRPC)   │ │ (proto)  │ │ liator   │ │ Bridge   │  │      │
 │  │  └─────┬────┘ └──────────┘ └──────────┘ └──────────┘  │      │
 │  └────────┼──────────────────────────────────────────────┘      │
 │           │                                                    │
 │           │ mTLS (SPIFFE) + JWT (service account)              │
 │           │ + M11 TenantContext injection                      │
 └───────────┼────────────────────────────────────────────────────┘
             │
             ▼ gRPC/HTTP2
 ┌───────────────────────────────────────────────────────────────┐
 │                    Rust Swarm Core (efe-server)               │
 │                                                               │
 │   M10 gRPC → Auth → Interceptors → Rate limit → Idempotency    │
 │       │                                                       │
 │       ▼                                                       │
 │   SwarmController (M8)                                        │
 │     ├─ Genesis  (M3)  — spawn nano-agents                      │
 │     ├─ Fusion   (M5)  — round-by-round updates in ELS (M4)     │
 │     ├─ Consensus(M6)  — convergence detector                   │
 │     └─ Dissolution(M7)— centroid + decoder + projection        │
 │     └─ Alignment(M9) — cone registry + breach reports          │
 └───────────────────────────────────────────────────────────────┘
```

### §2.2 Positionnement par couche

| Couche | Présence de M12 |
|--------|-----------------|
| **L1 Fondations** | Imports de `efe_platform.clock`, `efe_platform.hashing`, `efe_security.crypto` |
| **L2 Persistance** | Table `swarm_cone_bindings` (Feature ↔ ConeRef), table `swarm_call_log` (audit RPC) |
| **L3 Services** | `SwarmClient`, `ContractAdapter`, `CatalogBridge`, `VerdictReconciler`, `HbmBudgeter` |
| **L4 Orchestration** | `SwarmRpcInterceptor` (OTel + audit), `CircuitBreakerDaemon` |
| **L5 API / UI** | Pas de routes propres — M12 est invoqué par M06 inference, Mode 1 tool, M10 scheduler |

### §2.3 Dépendances

```
    M12 ──────┬──── M01 Foundations (clock, hashing, ids)
              │
              ├──── M11 Security (mTLS, JWT, audit, PDP)
              │
              ├──── M03 Registry (artifact blobs)
              │
              ├──── M04 Catalog (feature specs)
              │
              └──── M09 Drift (baselines → anchors)

 consumed by : M06 Inference, M07 Rollout (canary),
               M10 Scheduling (retrain → cone re-register),
               Mode 1 Bridge (classifier)

 talks to    : efe-server (gRPC) — tonic/rust, protobuf efe.v1
```

---

## §3. Principes directeurs

### §3.1 Principes opposables en revue PR

1. **P1 — Wire-format honesty.** Le code métier ne manipule jamais du
   protobuf brut. Le contract adapter est le seul endroit qui
   construit ou décode un message `efe.v1.*`.
2. **P2 — Single client, single pool.** Une seule instance de
   `SwarmClient` par process/pod. Toute création ad-hoc est interdite
   (lint CI sur `grpc.aio.insecure_channel`).
3. **P3 — Fail fast on contract drift.** Les tests de contrat
   comparent les `FieldDescriptor.number` protobuf à un snapshot JSON
   versionné ; tout écart bloque la CI avant même les tests
   d'intégration.
4. **P4 — No secrets in SwarmSpec.** L'embedding envoyé ne contient
   aucune PII résiduelle ; la vérification se fait via le linter M11
   avant la sérialisation protobuf.
5. **P5 — Deadlines obligatoires.** Aucun appel gRPC sans un
   `deadline` explicite ≤ `max_wall_clock_ms` du SwarmSpec + marge.
6. **P6 — Idempotence cross-layer.** L'`idempotency_key` fourni côté
   Python est passé tel quel comme metadata gRPC et honoré par
   l'interceptor du serveur Rust (M10 coreside).
7. **P7 — Tenant propagation.** Le tenant_id sort du SecurityContext
   M11, traverse M12 sous forme de metadata `x-efe-tenant-id` et
   devient le `tenant` dans les `ConeRef` côté serveur.
8. **P8 — Schema versioning.** Chaque message porte un `schema_version`
   stable. L'adapter accepte `N` et `N-1` simultanément pendant une
   fenêtre de grâce de 6 mois (rollback-safe).
9. **P9 — Zero silent truncation.** Si un champ dépasse sa taille
   max (ex. `problem_embedding.len() > MAX_DIM`), M12 lève
   `SchemaViolation` avant d'atteindre le wire.
10. **P10 — Observable by default.** Chaque RPC produit ≥ 1 span OTel,
    ≥ 1 métrique histogramme latence, ≥ 1 événement d'audit M11.
11. **P11 — Backpressure, not queuing.** En saturation, on répond 503
    au plus vite ; on n'accumule pas de requêtes en mémoire
    au-dessus d'une limite dure `max_inflight_per_tenant`.
12. **P12 — Bulkheads.** Les tenants ne partagent pas leur slot : un
    tenant saturé ne peut pas priver un autre de capacité (semaphore
    par tenant).

### §3.2 Ce que M12 refuse d'implémenter

- **Fallback local en pur-Python** — M12 ne simule pas l'essaim.
  Si ``efe-server`` est down, l'appel échoue en ``SERVICE_UNAVAILABLE``.
- **Cache transparent de vecteurs** — un cache biaiserait l'audit.
  L'idempotence est optionnelle et explicite.
- **Transformation sémantique** — pas de reshape, pas de downcast
  ``f32 → f16``. Ce qui arrive sort conforme au champ protobuf.
- **Auth propre** — M12 consomme les credentials M11. Toute tentative
  de définir une auth ad-hoc est refusée en revue.
- **Routing multi-backend** — une seule URL gRPC par tenant. Le
  sharding inter-régions relève de M11 (edge routing).

---

## §4. Invariants structurels

| Code | Nom | Énoncé formel |
|------|-----|---------------|
| **I1** | Contract adapter unique | Le serveur Rust ne reçoit que des messages construits par `contract_adapter.build_*`. Une recherche CI sur `efe_pb2.SwarmSpec(` hors de `contract_adapter.py` doit retourner 0 occurrences. |
| **I2** | SwarmSpec validé | Chaque `SwarmSpec` publié satisfait : `problem_embedding.len() == dim_latent`, `population.total() > 0`, `alignment_cone.anchors.len() > 0`. |
| **I3** | Tenant binding end-to-end | Toute requête porte `x-efe-tenant-id` ; toute `ConeRef` retournée a son `tenant` vérifié contre ce metadata. |
| **I4** | Deadline honorée | `deadline.total_seconds() * 1000 ≥ spec.budget.max_wall_clock_ms + network_margin_ms`. |
| **I5** | Audit par RPC | Chaque appel produit une ligne `efe-sec/audit-chain/v1` avec `action = "swarm.execute" \| "swarm.admin.*"`. |
| **I6** | Circuit breaker global | À chaque instant, le breaker a un état unique `{closed, open, half_open}` observable via `/metrics`. |
| **I7** | Backpressure borné | `inflight_requests(tenant) ≤ max_inflight_per_tenant`. |
| **I8** | Artefact intègre | `artifact_digest` fourni par le serveur est comparé à la valeur attendue ; mismatch → `ArtifactDigestMismatch` et fail-closed. |
| **I9** | Réentrance impossible | Un `SwarmClient.execute(...)` n'appelle jamais lui-même `execute(...)` (pas de récursion de swarm au niveau M12). |
| **I10** | Schema version monotone | Le `schema_version` d'un `SwarmSpec` est ≥ la version du contract adapter local ; rollback requiert une PR d'adapter. |
| **I11** | Binding immutable après commit | Une fois enregistrée dans `swarm_cone_bindings`, la mapping `(tenant, feature) ↔ ConeRef` ne peut changer qu'à travers `CatalogBridge.evolve_cone`, jamais en écriture directe. |
| **I12** | mTLS obligatoire en prod | En `config.transport.mtls_required = true`, toute tentative d'utiliser un canal insecure lève au démarrage. |

---

## §5. Modèle de menace et hypothèses

### §5.1 Hypothèses de sécurité

- Le **canal gRPC** entre M12 et ``efe-server`` est sur mTLS 1.3,
  signé par une AC interne (cert-manager + SPIFFE).
- Les **credentials de service** (JWT interne) sont émis par l'IdP
  de M11 et rotationnés toutes les 15 min.
- La **CA** de ``efe-server`` est pinned dans la config M12.
- Le **serveur Rust** applique déjà son propre rate-limit et
  idempotence (voir `M10_ARCHITECTURE.md` coreside, côté Rust).

### §5.2 Vecteurs d'attaque modélisés

| Vecteur | Attaque | Mitigation M12 |
|---------|---------|----------------|
| Client compromis | Envoie des embeddings malveillants (NaN, taille massive) | Validation stricte dans `contract_adapter.validate_spec` |
| Spoofing tenant | Un tenant forge un `x-efe-tenant-id` | Le metadata n'est jamais lu côté M12 ; il vient uniquement de `SecurityContext.principal.tenant_id` (M11) |
| Replay de response | Attaquant rejoue un réponse du serveur | Chaque `ExecuteResponse` porte un `swarm_id` + `request_id` vérifiés |
| Amplification HBM | Attaquant demande `population.total = 10^9` | Pré-validation : `population.total ≤ MAX_POPULATION = 50_000` |
| Starvation | Un tenant monopolise les slots | Bulkheads par tenant, semaphores Asyncio bornés |
| MITM (pas de mTLS) | Interception du trafic | mTLS 1.3 obligatoire, CA pinning |
| Denial via long deadline | Client envoie deadline=1h | `max_client_deadline_s = 60` borné côté M12 |

### §5.3 Bounds durs

```
MAX_DIM               = 4096
MAX_POPULATION        = 50_000
MAX_ROUNDS            = 256
MAX_ANCHORS           = 10_000
MAX_HBM_BYTES         = 6 * 1024**3    # 6 GiB
MAX_CLIENT_DEADLINE_S = 60
MAX_INFLIGHT_PER_TENANT = 32
MAX_MSG_BYTES_CLIENT  = 32 * 1024**2   # 32 MiB
MAX_MSG_BYTES_SERVER  = 64 * 1024**2   # 64 MiB
```

---

## §6. Acteurs et topologie

### §6.1 Acteurs logiques

| Acteur | Rôle | Déploiement |
|--------|------|-------------|
| **Consumer plateforme** | Module M06/M07/M09/M10/Mode 1 | Pod Python |
| **SwarmClient** | Point d'entrée unique | Singleton dans le pod |
| **gRPC channel pool** | Ensemble de canaux keep-alive | 1 canal par destination, HTTP/2 multiplexé |
| **Contract adapter** | Traducteur wire ↔ Python | Composant pur |
| **Verdict reconciler** | Post-traitement du verdict | Composant pur |
| **Catalog bridge** | Gestion des ConeRef | Daemon + API |
| **Artefact syncer** | Copie les poids ML vers le registry Rust | Job périodique (M10) |
| **efe-server** | Serveur Rust gRPC | Pod Rust (3 à N replicas) |

### §6.2 Topologie physique

```
 ┌─────────────────────────────────────────────────────────────┐
 │           Kubernetes Namespace : efe-prod                   │
 │                                                             │
 │   ┌──────────────┐            ┌───────────────────┐         │
 │   │ Platform Pod │            │ Swarm Service     │         │
 │   │ (Python)     │            │ Pod (Rust)        │         │
 │   │              │            │                   │         │
 │   │   M01..M11   │            │  efe-server       │         │
 │   │   +  M12     │─ mTLS ────▶│  + efe-core       │         │
 │   │              │            │  + efe-kernels    │         │
 │   │              │            │  (CUDA)           │         │
 │   └──────┬───────┘            └─────────┬─────────┘         │
 │          │                              │                   │
 │          ▼                              ▼                   │
 │   ┌──────────────┐            ┌───────────────────┐         │
 │   │ Prometheus + │            │ Postgres (cones,  │         │
 │   │ OTel         │            │ audit trails)     │         │
 │   └──────────────┘            └───────────────────┘         │
 │                                                             │
 │   ┌───────────────────────────────────────────────────────┐ │
 │   │ Service Mesh (Linkerd/Istio) — mTLS auto, policy      │ │
 │   └───────────────────────────────────────────────────────┘ │
 └─────────────────────────────────────────────────────────────┘
```

### §6.3 Modèle de concurrence

| Composant | Concurrence | Isolation |
|-----------|-------------|-----------|
| `SwarmClient.execute` | N appels en parallèle via `asyncio.gather` | Semaphore par tenant |
| `grpc.aio.Channel` | Multiplexe automatiquement sur HTTP/2 | 1 canal partagé |
| `ContractAdapter` | Pur, sans état | Thread-safe par construction |
| `VerdictReconciler` | Pur | idem |
| `CatalogBridge` | Transactions SQL série | `SELECT FOR UPDATE` sur ConeRef |
| `ArtefactSyncer` | Daemon séquentiel | Un job global, pas de parallélisme |

---

## §7. Vue d'ensemble architecturale

### §7.1 Arborescence des sous-modules

```
efe_swarm_integration/                  -- M12
├── __init__.py                          -- public exports
├── errors.py                            -- taxonomy
├── config.py                            -- dataclass frozen
├── version.py
│
├── transport/                           -- §8
│   ├── __init__.py
│   ├── channel.py                       -- grpc.aio.Channel factory + mTLS
│   ├── credentials.py                   -- cert + JWT loading
│   ├── interceptors/
│   │   ├── __init__.py
│   │   ├── tenant.py                    -- inject x-efe-tenant-id
│   │   ├── auth.py                      -- Bearer JWT + mTLS peer check
│   │   ├── deadline.py                  -- clamp to MAX_CLIENT_DEADLINE_S
│   │   ├── retry.py                     -- retry on UNAVAILABLE/RESOURCE_EXHAUSTED
│   │   ├── audit.py                     -- M11 audit emission
│   │   ├── telemetry.py                 -- OTel span + Prometheus histograms
│   │   └── circuit_breaker.py           -- per-tenant CB
│   └── pool.py                          -- single process-wide pool
│
├── contract/                            -- §9
│   ├── __init__.py
│   ├── snapshot.py                      -- frozen snapshot of proto schema
│   ├── adapter_spec.py                  -- Python SwarmSpec ↔ pb2.SwarmSpec
│   ├── adapter_result.py                -- pb2.ExecuteResponse ↔ Python
│   ├── adapter_admin.py                 -- AdminService messages
│   ├── adapter_cone.py                  -- AlignmentCone / ConeRef
│   ├── validator.py                     -- pre-serialisation bounds checks
│   └── generated/                       -- protoc-py output (git-committed)
│       ├── efe/v1/efe_pb2.py
│       ├── efe/v1/efe_pb2_grpc.py
│       ├── efe/v1/types_pb2.py
│       ├── efe/v1/admin_pb2.py
│       └── efe/v1/admin_pb2_grpc.py
│
├── catalog/                             -- §10
│   ├── __init__.py
│   ├── bridge.py                        -- feature_id ↔ cone_key
│   ├── yaml_loader.py                   -- parse anchors.yaml
│   ├── compiler.py                      -- run embedder → cone anchors
│   ├── store.py                         -- swarm_cone_bindings table
│   └── validator.py                     -- anchor sanity (no PII, variance)
│
├── artefact/                            -- §11
│   ├── __init__.py
│   ├── syncer.py                        -- M05 artefact → Rust registry
│   ├── manifest.py                      -- SBOM of published models
│   └── digest.py                        -- sha256 + HMAC signature
│
├── client.py                            -- §12  SwarmClient public API
│
├── reconciler/                          -- §13
│   ├── __init__.py
│   ├── decoder.py                       -- DecodedOutput → business label
│   ├── confidence.py                    -- alignment_delta → confidence
│   └── nearest_anchor.py                -- M7.5 decoder post-hook
│
├── budget/                              -- §14
│   ├── __init__.py
│   ├── hbm.py                           -- HbmBudgeter
│   ├── estimator.py                     -- ex-ante HBM estimation
│   └── wallclock.py                     -- deadline computation
│
├── flow_control/                        -- §15
│   ├── __init__.py
│   ├── bulkhead.py                      -- per-tenant semaphore
│   ├── circuit_breaker.py               -- 3-state CB
│   └── retry.py                         -- jittered retry policy
│
├── streaming/                           -- §16
│   ├── __init__.py
│   ├── server_stream.py                 -- ExecuteStreaming consumer
│   └── bidi.py                          -- bidi future hook
│
├── admin/                               -- §17
│   ├── __init__.py
│   ├── client.py                        -- AdminClient
│   ├── cone_ops.py                      -- register / evolve / delete
│   └── breach_tailer.py                 -- subscribe to BreachHistory
│
└── telemetry/                           -- §18
    ├── __init__.py
    ├── metrics.py                       -- Prometheus registry
    ├── tracing.py                       -- OTel spans
    └── audit_hook.py                    -- M11 audit adapter
```

### §7.2 Flux d'une requête Execute

```
 (A) Consumer (ex: M06 inference)
        ↓ SwarmExecuteRequest(
              text, catalog_key, tenant_id, task_id,
              budget_ms=500, idempotency_key=<opt>)
        ↓
 (B) SwarmClient.execute(request)
        ├── 1. acquire bulkhead for tenant
        ├── 2. resolve catalog (CatalogBridge) → ConeRef
        ├── 3. embed(text) → Vec[float] (délégué à Mode 1 bridge)
        ├── 4. ContractAdapter.build_execute_request(
        │          text, cone_ref, budget, deadline, tenant)
        ├── 5. Interceptors chain :
        │        tenant → auth → deadline → retry
        │        → audit → telemetry → circuit_breaker
        ├── 6. grpc stub.Execute(req, metadata, deadline)
        │   ┌─────────────────┐
        │   │ Rust efe-server │
        │   │ M10 interceptors│
        │   │ M8 Controller   │
        │   │ M3→M5→M6→M7     │
        │   └─────┬───────────┘
        │         │ ExecuteResponse
        │         ▼
        ├── 7. ContractAdapter.parse_execute_response → VerdictBundle
        ├── 8. VerdictReconciler.reconcile → BusinessResult(label,
        │          confidence, alignment_delta, swarm_id)
        ├── 9. AuditHook.emit → M11 audit line
        ├── 10. release bulkhead
        └── return BusinessResult
```

### §7.3 Flux d'une souscription Streaming

```
 (A) Consumer calls stream = client.execute_streaming(request)
 (B) SwarmClient opens a server-streaming RPC
 (C) Server emits StreamChunk[ text_delta | event | final ]
 (D) stream yields typed events :
        TextDelta(token) | ServerEvent(kind, labels) | FinalResult
 (E) On disconnect : grpc.aio.AioRpcError → reconciliation path
        or retry up to N times with backoff
 (F) Close : release bulkhead + audit
```

### §7.4 Flux d'enregistrement de cône

```
 (A) M09 publishes new baseline for feature F
 (B) CatalogBridge.register_or_evolve(feature=F, anchors=[...], tenant, task)
 (C) AdminService.RegisterCone / EvolveCone
 (D) Server persists in M9 (alignment registry) + returns ConeHandle
 (E) CatalogBridge.store(feature, ConeHandle.key, version)
 (F) M11 Audit : "swarm.cone.registered"
```

---

## §8. Sous-système 12.1 — **Transport gRPC**

### §8.1 Channel factory

Un singleton `SwarmChannelFactory` produit **un** `grpc.aio.Channel`
par URL cible.

```
Protocol SwarmChannelFactory :
    def channel(self, target: str, *, tenant: str | None) -> grpc.aio.Channel
    def close_all(self) -> Awaitable[None]
```

Paramètres non négociables :

| Paramètre gRPC | Valeur |
|----------------|--------|
| `grpc.keepalive_time_ms` | 20000 |
| `grpc.keepalive_timeout_ms` | 5000 |
| `grpc.keepalive_permit_without_calls` | 1 |
| `grpc.max_send_message_length` | `MAX_MSG_BYTES_CLIENT` |
| `grpc.max_receive_message_length` | `MAX_MSG_BYTES_SERVER` |
| `grpc.enable_retries` | 0 (on gère côté client) |
| `grpc.service_config` | JSON strict avec retries désactivés |

### §8.2 Credentials

- **mTLS** : `grpc.ssl_channel_credentials(
    root_certificates=CA_PEM,
    private_key=CLIENT_KEY, certificate_chain=CLIENT_CERT)`.
- **Bearer JWT** : fourni via `grpc.composite_channel_credentials(
    ssl_creds, call_creds)` où `call_creds` est un `AuthMetadataPlugin`
  qui rafraîchit le JWT via `TokenService` (M11).

### §8.3 Interceptors

Chaîne exécutée dans cet ordre strict (chaque maillon est un
`grpc.aio.UnaryUnaryClientInterceptor` ou équivalent streaming) :

1. `TenantInterceptor` — injecte `x-efe-tenant-id`, `x-efe-task-id`,
   `x-efe-request-id`.
2. `AuthInterceptor` — s'assure que le `call_creds` a rafraîchi le JWT.
3. `DeadlineInterceptor` — clamp à `MAX_CLIENT_DEADLINE_S`.
4. `RetryInterceptor` — retry uniquement pour
   `UNAVAILABLE`, `RESOURCE_EXHAUSTED`, `DEADLINE_EXCEEDED` (si
   `idempotency_key` présent), avec backoff exponentiel + jitter.
5. `AuditInterceptor` — émet `swarm.rpc.invoked` avant, et
   `swarm.rpc.completed` après (via M11 AuditEmitter).
6. `TelemetryInterceptor` — span OTel `grpc.client` + compteur
   `efe_m12_rpc_total{status=...}` + histogramme
   `efe_m12_rpc_latency_seconds{method=...}`.
7. `CircuitBreakerInterceptor` — consulte/alimente le breaker.

### §8.4 Retry policy

```
def should_retry(status, attempt, idempotent) -> bool:
    if status == UNAVAILABLE:           return attempt < 5
    if status == RESOURCE_EXHAUSTED:    return attempt < 3
    if status == DEADLINE_EXCEEDED:     return idempotent and attempt < 2
    return False

def backoff(attempt) -> timedelta:
    base = 200ms * (2 ** (attempt - 1))
    return base + random.uniform(0, base/2)
```

---

## §9. Sous-système 12.2 — **Contract Adapter (Protobuf bridge)**

### §9.1 Surface publique

```
ContractAdapter :
    def build_execute_request(
        *,
        embedding: Sequence[float],
        dim_latent: int,
        cone_source: ConeRef | AlignmentCone,
        population: PopulationMix,
        max_rounds: int,
        convergence: ConvergenceThresholds,
        budget: ResourceBudget,
        decode_mode: DecodeMode,
        deadline: datetime,
        raw_prompt: str | None,
    ) -> pb.ExecuteRequest

    def parse_execute_response(resp: pb.ExecuteResponse) -> VerdictBundle
    def build_register_cone_request(spec: ConeSpec) -> pb.RegisterConeRequest
    def parse_cone_handle(handle: pb.ConeHandle) -> ConeHandle
    def parse_breach_report(rep: pb.BreachReport) -> BreachReport
```

### §9.2 Validation à la frontière

Chaque `build_*` appelle `validator.validate_*` avant
sérialisation :

- `embedding.len() == dim_latent` (I2).
- `1 ≤ dim_latent ≤ MAX_DIM`.
- `population.total ≤ MAX_POPULATION`.
- `max_rounds ≤ MAX_ROUNDS`.
- `budget.max_hbm_bytes ≤ MAX_HBM_BYTES`.
- `budget.max_wall_clock_ms ∈ (0, MAX_CLIENT_DEADLINE_S * 1000]`.
- Si `AlignmentCone` inline : `anchors.len() ≤ MAX_ANCHORS`.
- `raw_prompt` : si présent, M11 PII linter invoqué ; toute PII
  détectée fait raise `PiiDetectedInPrompt`.

### §9.3 Snapshot contractuel

Un fichier `contract/snapshot.json` est committé en dépôt :

```json
{
  "schema_version": "efe.v1",
  "messages": {
    "SwarmSpec": {
      "fields": [
        {"name": "swarm_id", "number": 1, "type": "TYPE_STRING"},
        {"name": "problem_embedding", "number": 2, "type": "TYPE_FLOAT",
         "label": "LABEL_REPEATED"},
        ...
      ]
    },
    ...
  }
}
```

Un test de contrat CI compare ce snapshot à l'image runtime
`efe.v1.SwarmSpec.DESCRIPTOR.fields` ; tout écart de `field.number`
fait échouer la CI (**P3**).

### §9.4 Schema version & migrations

- Un enum `SchemaVersion = {v1, v2, …}`.
- `ContractAdapter` fournit `build_execute_request(version=v1)` et
  `build_execute_request(version=v2)`.
- Pendant 6 mois, les deux sont supportés (grace period).
- Après, un `DeprecationWarning` côté client + erreur explicite.

---

## §10. Sous-système 12.3 — **Anchor Catalog Bridge**

### §10.1 Rôle

Traduire la notion Python "**feature** vue par M04/M09" en
**cone** vu par `efe-core::alignment`. Une feature peut produire 1 à N
cones selon le mode de décodage (un cone par `task_id`, par exemple).

### §10.2 Modèle de données

Table `swarm_cone_bindings` :

| Colonne | Type |
|---------|------|
| `binding_id` | UUID PK |
| `tenant_id` | TEXT NOT NULL |
| `feature_id` | TEXT NOT NULL |
| `task_id` | TEXT NULL |
| `cone_tenant` | TEXT NOT NULL — tenant côté server |
| `cone_task` | TEXT NOT NULL — task côté server |
| `cone_version` | INTEGER NOT NULL |
| `cone_key` | TEXT — `tenant/task@vN` |
| `embedder_family` | TEXT |
| `embedder_version` | TEXT |
| `anchor_count` | INTEGER |
| `created_at` | TIMESTAMPTZ |
| `disabled_at` | TIMESTAMPTZ NULL |
| `state_version` | INTEGER |

RLS identique à M11.

### §10.3 Chargement depuis YAML

Un opérateur publie un `support_triage.yaml` :

```yaml
catalog_id: support_triage
tenant: acme
task: triage-v2
embedder:
  family: sentence-transformers
  model: all-MiniLM-L6-v2
  version: 2.7
dim: 384
labels:
  - label: fraud
    anchors:
      - "J'ai été débité sans consentement"
      - "Un achat non autorisé apparaît"
  - label: billing
    anchors:
      - "Ma facture ne correspond pas"
max_projection_distance: 0.35
```

`CatalogBridge.compile(path)` :

1. Valide la structure (Pydantic stricte).
2. Normalise NFKC + trim.
3. Passe M11 PII linter sur chaque ancre.
4. Embed chaque ancre via `EmbedderService` (Mode 1 bridge).
5. Produit un `ConeSpec` serveur.
6. Appelle `AdminClient.register_cone`.
7. Persiste le binding.

### §10.4 Evolution

Une evolve ne remplace pas le cone ; elle crée `tenant/task@v(N+1)`
via `AdminService.EvolveCone(op=...)`. Le binding local est
incrémenté.

---

## §11. Sous-système 12.4 — **Artefact Sync**

### §11.1 Contexte

M05 produit des artefacts fine-tunés (LoRA adapters) au format
safetensors. Côté Rust, ces artefacts ne sont pas nécessaires pour
le swarm *vectoriel* (qui opère sur embeddings), **mais** ils sont
utilisés par Mode 1 quand un embedder local est préféré.

M12 les synchronise :

- Copie les blobs M03 → bucket dédié au serveur Rust.
- Enregistre le `ArtefactManifest` (digest, dim, tokeniser) dans la
  base partagée.
- Émet `artefact.synced` (audit M11).

### §11.2 Pipeline

```
 M05.artefact.published (NATS) ──▶ ArtefactSyncer.on_event
   1. validate signature (HMAC from M05 DOMAIN_TAG)
   2. download blob from M03 object store
   3. recompute sha256; must match manifest.digest
   4. upload to swarm-facing bucket
   5. write swarm_artefact_manifests row
   6. publish "swarm.artefact.synced" event
```

### §11.3 Intégrité

Double check :

- Digest SHA-256 de bout-en-bout (M05 → M12 → bucket).
- Signature HMAC Ed25519 (M05 DOMAIN_TAG_ARTIFACT).
- Entry in `swarm_artefact_manifests` sealed via M11
  `DOMAIN_TAG_ARTEFACT_SYNC` (nouveau tag, voir §A).

---

## §12. Sous-système 12.5 — **SwarmClient (Python API)**

### §12.1 Surface publique

```
class SwarmClient :
    def __init__(
        self,
        *,
        config: SwarmClientConfig,
        embedder: EmbedderService,
        catalog: CatalogBridge,
        pdp: PolicyDecisionPoint,   # M11
        audit: AuditEmitter,        # M11
        tenant_bulkhead: BulkheadRegistry,
    ): ...

    async def classify(
        self, req: ClassifyRequest, *, ctx: SecurityContext,
    ) -> ClassifyResponse

    async def execute(
        self, spec: SwarmSpec, *, ctx: SecurityContext,
        deadline: datetime | None = None,
    ) -> VerdictBundle

    async def execute_streaming(
        self, spec: SwarmSpec, *, ctx: SecurityContext,
    ) -> AsyncIterator[StreamEvent]

    async def describe_spec(self, spec: SwarmSpec) -> DescribeSpecReply
    async def close(self) -> None
```

### §12.2 `classify` — Mode 1 fast path

```
async def classify(req, *, ctx):
    # 1. authz check (PDP)
    await require(ctx, action="swarm.classify", resource={kind:"catalog"})
    # 2. resolve cone
    cone_ref = await self._catalog.resolve(
        ctx.tenant_id, req.catalog_key)
    # 3. embed
    embedding = await self._embedder.embed(req.text,
        embedder=cone_ref.embedder_family)
    # 4. build spec
    spec = SwarmSpec.builder()
              .problem_embedding(embedding)
              .dim_latent(cone_ref.dim)
              .population(default_mix_for_classify())
              .max_rounds(req.max_rounds or 8)
              .alignment_cone_ref(cone_ref)
              .budget(ResourceBudget(wall_clock_ms=req.budget_ms))
              .build()
    # 5. execute
    verdict = await self.execute(spec, ctx=ctx)
    # 6. decode to label
    return self._reconciler.to_classify_response(verdict, cone_ref)
```

### §12.3 Idempotence

Si `req.idempotency_key` est fourni, il est propagé :

- Comme metadata gRPC `x-efe-idempotency-key`.
- L'interceptor côté serveur Rust (M10 Rust) garantit l'unicité.
- Côté client, on maintient une petite LRU `{key → result}` pour
  court-circuiter les appels réellement identiques.

### §12.4 Erreurs remontées au consumer

| Exception Python | Origine |
|-----------------|---------|
| `SchemaViolation` | `ContractAdapter.validator` |
| `PiiDetectedInPrompt` | M11 linter avant wire |
| `Unauthorised` | M11 PDP |
| `QuotaExceeded` | Bulkhead / breaker |
| `SwarmUnavailable` | gRPC UNAVAILABLE |
| `SwarmTimeout` | DEADLINE_EXCEEDED |
| `SwarmDivergence` | Verdict == DIVERGENCE |
| `SwarmModeCollapse` | Verdict == MODE_COLLAPSE |
| `AlignmentBreach` | alignment_delta > cone.radius |
| `ArtefactDigestMismatch` | §11.3 |

---

## §13. Sous-système 12.6 — **Verdict Reconciler**

### §13.1 Types

```
VerdictBundle (frozen) :
    swarm_id: str
    request_id: str
    verdict: VerdictEnum   # CONVERGED | DIVERGENCE | MODE_COLLAPSE | CONTINUE
    decoded: DecodedOutput  # text | raw_centroid | anchor_label
    confidence: float       # in [0,1]
    alignment_delta: float
    rounds_executed: int
    total_ms: float
    breach: BreachReport | None

ClassifyResponse (frozen) :
    label: str | None
    label_idx: int | None
    confidence: float
    alignment_delta: float
    swarm_id: str
    request_id: str
    verdict: VerdictEnum
    reasons: tuple[str, ...]
```

### §13.2 Règles de réconciliation

1. Si `verdict == CONVERGED` et `decoded.kind == anchor_label` :
   `label = cone.labels[decoded.idx]`.
2. Si `verdict == CONVERGED` mais `alignment_delta > threshold` :
   `label = None`, `reason = "uncertain_alignment"`.
3. Si `verdict == DIVERGENCE` ou `MODE_COLLAPSE` : `label = None`,
   `reason = "verdict:<name>"`, log warning.
4. Si `breach is not None` : propager à M09 comme
   `AlignmentBreachSignal` (audit + possible trigger).
5. Confidence : computed as `max(0, 1 - alignment_delta /
   cone.max_projection_distance)` clamp [0,1].

---

## §14. Sous-système 12.7 — **HBM / Resource Budgeting**

### §14.1 Objectif

Avant d'envoyer une requête, estimer le HBM que Rust va allouer.
Si l'estimation dépasse `max_hbm_bytes` ou la capacité du cluster,
refuser avant le RPC (fail fast).

### §14.2 Formule (par tenant, par spec)

```
hbm_est(bytes) ≈
     4 * dim_latent * (population.total + num_rounds * buffer_factor)
   + 4 * dim_latent * num_anchors * 2          # anchors + projections
   + 2 * MiB * overhead_static
```

Cette estimation est conservative (upper bound), affinée par
mesures côté serveur via le RPC `DescribeSpec`.

### §14.3 Interaction avec `DescribeSpec`

Pour les specs peu standards, M12 appelle d'abord
`SwarmService.DescribeSpec` :

```
resp = stub.DescribeSpec(DescribeSpecRequest(spec=spec))
# resp.valid, resp.errors, resp.hbm_bytes_estimate, resp.rounds_expected
if not resp.valid:
    raise SchemaViolation(errors=resp.errors)
if resp.hbm_bytes_estimate > MAX_HBM_BYTES:
    raise HbmBudgetExceeded(...)
```

### §14.4 Budget wall-clock

`deadline_grpc = now + spec.budget.max_wall_clock_ms +
 network_margin_ms` où `network_margin_ms = 200`.

---

## §15. Sous-système 12.8 — **Backpressure & Circuit Breaker**

### §15.1 Bulkhead per-tenant

Une instance `asyncio.Semaphore(max_inflight_per_tenant)` par tenant.

Si le `acquire()` n'est pas obtenu en `max_queue_wait_ms` (défaut 50),
`BackpressureRejected` (mappé 503).

### §15.2 Circuit breaker

Trois états `{closed, open, half_open}` :

| Depuis | Condition | Cible |
|--------|-----------|-------|
| closed | `error_rate > 25 %` sur 30 s et `n ≥ 20` | open |
| open | `open_since > 10 s` | half_open |
| half_open | 1 succès | closed |
| half_open | 1 échec | open |

Quand `open`, tout call lève `CircuitOpen` immédiatement sans
toucher le wire. Un métrique `efe_m12_cb_state{state, tenant}` est
exposé.

### §15.3 Bulkheads × Circuit Breaker

Ils se composent ainsi :

```
CB ──(open)──> fail fast
   ──(closed)──> Bulkhead ──(acquire)──> send RPC
                         ──(rejected)──> 503
```

---

## §16. Sous-système 12.9 — **Streaming Interface**

### §16.1 Serveur-streaming `ExecuteStreaming`

Le serveur Rust émet des `StreamChunk` avec un oneof :

- `text_delta` — un token/texte partiel (décodage texte).
- `event` — un `ServerEvent(kind, message, labels)`
  (ex. `"round.completed"`, `"breach.detected"`).
- `final` — un `ExecuteResponse` équivalent au unary.

### §16.2 Client Python

```
class StreamEvent :
    kind: Literal["text_delta", "server_event", "final"]
    data: Any

async for event in client.execute_streaming(spec, ctx=ctx):
    if event.kind == "final":
        verdict = event.data  # VerdictBundle
```

### §16.3 Déconnexions

Reconnexion automatique sur `UNAVAILABLE` pendant le flux :

- Si `final` non reçu et `idempotent=True` : re-subscribe depuis
  le dernier `server_event.round` reçu (si le serveur supporte
  la reprise ; sinon, full retry).
- Sinon : lève `StreamAborted`.

### §16.4 Bidi future hook

Un marker `BidiStreamHandle` est exposé pour un futur mode
bidi (submit incremental observations, receive updates). Non
implémenté MVP — le stub existe pour figer l'API.

---

## §17. Sous-système 12.10 — **Admin Bridge (Cone Registry)**

### §17.1 Opérations exposées

Toutes passent par `AdminClient` qui wrappe `AdminService` gRPC :

| Méthode | Action | Scope M11 requis |
|---------|--------|------------------|
| `register_cone(spec)` | `AdminService.RegisterCone` | `cone:write` |
| `evolve_cone(from_key, op)` | `AdminService.EvolveCone` | `cone:write` |
| `get_cone(key)` | `AdminService.GetCone` | `cone:read` |
| `list_versions(tenant, task)` | `AdminService.ListVersions` | `cone:read` |
| `delete_cone(key, force)` | `AdminService.DeleteCone` | `cone:admin` (MFA req.) |
| `breach_history(filters)` | `AdminService.BreachHistory` | `cone:read` |

### §17.2 BreachHistory tailer

Un daemon `BreachTailer` pull régulièrement `BreachHistory` et
publie sur NATS `efe.m12.breach.detected` pour M09.

```
async def run(self):
    while not self._stopping:
        reports = await self._admin.breach_history(limit=100, since=self._cursor)
        for r in reports:
            await self._nats.publish("efe.m12.breach.detected", r.to_dict())
            self._cursor = max(self._cursor, r.timestamp)
        await asyncio.sleep(self.poll_interval_s)
```

### §17.3 Evolution ops

Supportées (reflet du proto) :

- `append_anchor(anchor)`.
- `remove_anchor_idx(idx)`.
- `replace_anchor(idx, anchor)`.
- `update_radius(new_radius)`.
- `scale_anchors(factor)`.
- `union_with_key(other)`.

Chaque op est auditée.

---

## §18. Sous-système 12.11 — **Telemetry Bridge**

### §18.1 Métriques Prometheus

| Métrique | Type | Labels |
|----------|------|--------|
| `efe_m12_rpc_total` | Counter | tenant, method, status |
| `efe_m12_rpc_latency_seconds` | Histogram | method |
| `efe_m12_rpc_inflight` | Gauge | tenant |
| `efe_m12_cb_state` | Gauge | tenant, state |
| `efe_m12_cb_transitions_total` | Counter | tenant, from_state, to_state |
| `efe_m12_bulkhead_rejected_total` | Counter | tenant |
| `efe_m12_hbm_estimate_bytes` | Histogram | tenant |
| `efe_m12_verdict_total` | Counter | verdict (CONVERGED/…) |
| `efe_m12_alignment_delta` | Histogram | |
| `efe_m12_streaming_chunks_total` | Counter | kind |
| `efe_m12_cone_binding_version` | Gauge | feature_id |
| `efe_m12_artefact_sync_total` | Counter | status |

### §18.2 Spans OTel

- Root span `m12.classify` / `m12.execute` / `m12.execute_streaming`.
- Enfant `grpc.client.<Method>` produit par l'interceptor.
- Attributs : `efe.tenant`, `efe.task`, `efe.swarm_id`,
  `efe.verdict`, `efe.rounds_executed`, `efe.alignment_delta`.

### §18.3 Audit

Chaque RPC produit 1 à 2 lignes M11 (cf. §I5) :

- `swarm.execute` (outcome=success/failure).
- `swarm.admin.<op>`.
- `swarm.cone.registered`, `swarm.cone.evolved`.
- `swarm.artefact.synced`.
- `swarm.breach.detected`.

---

## §19. Budget de performance

| Axe | Cible | Conditions |
|-----|-------|------------|
| Latence client-side overhead (hors wire) | p99 < 5 ms | mTLS établi, pool chaud |
| Classify end-to-end | p99 < 350 ms | embedding 100 ms, wire 200 ms, parse 50 ms |
| Reconnect after broker restart | < 30 s, transparent | retry backoff |
| Streaming first-chunk latency | p99 < 150 ms | grpc keep-alive actif |
| Bulkhead acquire time | p99 < 10 ms | tenant non-saturé |
| Cone register | p99 < 500 ms | admin path |
| BreachTailer lag | < 5 s | 100 reports/poll |
| Memory overhead per pod | ≤ 128 MiB | 1000 specs/min |

---

## §20. Taxonomie d'erreurs

```
SwarmIntegrationError (root)
├── TransportError
│   ├── ChannelUnavailable
│   ├── MutualTlsFailed
│   └── TimeoutExceeded
├── ContractError
│   ├── SchemaViolation
│   ├── SchemaVersionMismatch
│   └── ContractSnapshotMismatch
├── CatalogError
│   ├── CatalogNotFound
│   ├── CatalogCompilationFailed
│   └── ConeRefNotBound
├── ArtefactError
│   ├── ArtefactDigestMismatch
│   └── ArtefactSyncFailed
├── VerdictError
│   ├── SwarmDivergence
│   ├── SwarmModeCollapse
│   ├── AlignmentBreach
│   └── UncertainAlignment
├── FlowControlError
│   ├── BackpressureRejected
│   ├── CircuitOpen
│   └── QuotaExceeded
├── SecurityError
│   ├── PiiDetectedInPrompt
│   ├── TenantMismatch
│   └── Unauthorised
└── StreamingError
    ├── StreamAborted
    └── UnexpectedChunk
```

Tous mappés en `ProblemDetails` RFC 7807 par le caller (M06/M10).

---

## §21. Stratégie de tests

### §21.1 Matrice

| Couche | Type | Outil |
|--------|------|-------|
| `contract_adapter` | Round-trip property-based | Hypothesis |
| `contract_snapshot` | Golden-file diff | pytest + JSON snapshot |
| `transport.interceptors` | Unit | pytest-asyncio |
| `client.classify` | Integration avec serveur stub | `grpc-testing` |
| `catalog.compile` | Integration avec embedder stub | pytest |
| `streaming` | Long-running | pytest + test server |
| `circuit_breaker` | State machine | property-based |
| `reconciler` | Table de vérité | pytest parametrize |

### §21.2 Test server

Un `FakeSwarmServer` (en pur Python grpc.aio) est fourni pour les
tests d'intégration :

- Répond à `Execute` avec un verdict configurable.
- Simule `UNAVAILABLE` / `RESOURCE_EXHAUSTED` à la demande.
- Expose le `DescribeSpec` avec une estimation HBM paramétrable.

### §21.3 Gates CI

- Couverture ≥ 92 % sur tous les sous-modules M12.
- `mypy --strict` zero erreur.
- `ruff ALL` sans dérogation.
- Snapshot contractuel figé (fail sur diff).
- Bench : régression p99 ≤ 10 %.

---

## §22. Plan de sprints

| Sprint | Livrable | DoD |
|--------|----------|-----|
| **S1** | `transport/channel.py` + `credentials.py` + interceptors base | Tests interceptor green |
| **S2** | `contract/generated/` + `adapter_spec.py` + `validator.py` | round-trip Hypothesis |
| **S3** | `client.execute` + `reconciler/` | FakeSwarmServer passes |
| **S4** | `catalog/bridge.py` + YAML + compile | register cone integration |
| **S5** | `artefact/syncer.py` | sha256 + HMAC integration |
| **S6** | `flow_control/` (bulkhead + CB) | state machine tests |
| **S7** | `streaming/` | first-chunk latency bench |
| **S8** | `admin/` + `breach_tailer` | e2e with AdminService |
| **S9** | `telemetry/` + dashboards Grafana | dashboard import OK |
| **S10** | `budget/` (HBM estimator, DescribeSpec) | bench OK |
| **S11** | Chaos suite (broker down, mTLS rotation, slow server) | 0 leak |
| **S12** | Release GA | Go/No-Go §23 |

---

## §23. Critères Go / No-Go spécifiques à M12

### §23.1 Go si

- Tous les 12 invariants testés en CI.
- Round-trip protobuf sans perte sur 10 000 cas Hypothesis.
- Circuit breaker + bulkhead vérifiés en chaos test (service down 5 min).
- mTLS rotation (24 h) sans dropped connections.
- Streaming 10 clients × 1 h sans leak.
- Bench p99 < 350 ms end-to-end classify sur golden set.
- Breach tailer lag < 5 s.

### §23.2 No-Go si

- Un client peut bypasser le contract adapter et envoyer du
  protobuf brut (I1 cassé).
- Un tenant peut dépasser son bulkhead (I7).
- Une réponse non-auditée existe (I5).
- mTLS échoue silencieusement (fallback insecure = échec immédiat).
- Un `artifact_digest` mismatch ne lève pas.

---

## Annexe A — Mapping protobuf ↔ Python ↔ Rust

| Protobuf message | Python class (M12) | Rust struct (efe-core) |
|-----------------|-------------------|----------------------|
| `efe.v1.SwarmSpec` | `types.SwarmSpec` | `efe_core::types::SwarmSpec` |
| `efe.v1.PopulationEntry` | `types.PopulationEntry` | `(TemplateCode, u32)` |
| `efe.v1.ConvergenceThresholds` | `types.ConvergenceThresholds` | `efe_core::types::ConvergenceThresholds` |
| `efe.v1.AlignmentCone` | `types.AlignmentCone` | `efe_core::types::AlignmentCone` |
| `efe.v1.Anchor` | `types.Anchor` | `Vec<f32>` |
| `efe.v1.ConeRef` | `types.ConeRef` | `efe_core::alignment::ConeKey` |
| `efe.v1.ResourceBudget` | `types.ResourceBudget` | `efe_core::types::ResourceBudget` |
| `efe.v1.DecodeMode` | `types.DecodeMode` (sum) | `efe_core::swarm::DecodeMode` |
| `efe.v1.Verdict` | `VerdictEnum` | `efe_core::swarm::Verdict` |
| `efe.v1.ExecuteRequest` | `contract.ExecuteRequest` | — |
| `efe.v1.ExecuteResponse` | `VerdictBundle` | — |
| `efe.v1.admin.ConeHandle` | `admin.ConeHandle` | `efe_core::alignment::ConeHandle` |

HMAC domain tags ajoutés par M12 (mis dans `efe_security/crypto/primitives.py`
via une PR dédiée) :

- `DOMAIN_TAG_SWARM_IDEMPOTENCY` — `b"efe-m12/idempotency/v1\x00"`
- `DOMAIN_TAG_ARTEFACT_SYNC` — `b"efe-m12/artefact-sync/v1\x00"`
- `DOMAIN_TAG_CONE_BINDING` — `b"efe-m12/cone-binding/v1\x00"`

---

## Annexe B — Matrice d'erreurs gRPC

| grpc.StatusCode | Python exception | retryable | http mapping |
|-----------------|-----------------|-----------|-------------|
| OK | — | — | 2xx |
| CANCELLED | `OperationCancelled` | no | 499 |
| UNKNOWN | `SwarmUnknownError` | no | 500 |
| INVALID_ARGUMENT | `SchemaViolation` | no | 400 |
| DEADLINE_EXCEEDED | `SwarmTimeout` | yes* | 504 |
| NOT_FOUND | `CatalogNotFound` / `ConeRefNotBound` | no | 404 |
| ALREADY_EXISTS | `ConeAlreadyExists` | no | 409 |
| PERMISSION_DENIED | `Unauthorised` | no | 403 |
| RESOURCE_EXHAUSTED | `QuotaExceeded` | yes | 429 |
| FAILED_PRECONDITION | `InvalidConeOp` | no | 409 |
| ABORTED | `OptimisticLockConflict` | yes | 409 |
| OUT_OF_RANGE | `SchemaViolation` | no | 400 |
| UNIMPLEMENTED | `UnsupportedRpc` | no | 501 |
| INTERNAL | `SwarmInternalError` | no | 500 |
| UNAVAILABLE | `SwarmUnavailable` | yes | 503 |
| DATA_LOSS | `SwarmDataLoss` | no | 500 |
| UNAUTHENTICATED | `AuthNError` | no | 401 |

`* retryable` uniquement si `idempotency_key` présent.

---

## Annexe C — DDL Postgres — tables M12

```sql
CREATE TABLE IF NOT EXISTS swarm_cone_bindings (
    binding_id         UUID PRIMARY KEY,
    tenant_id          TEXT NOT NULL,
    feature_id         TEXT NOT NULL,
    task_id            TEXT,
    cone_tenant        TEXT NOT NULL,
    cone_task          TEXT NOT NULL,
    cone_version       INTEGER NOT NULL,
    cone_key           TEXT NOT NULL,
    embedder_family    TEXT NOT NULL,
    embedder_version   TEXT NOT NULL,
    anchor_count       INTEGER NOT NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    disabled_at        TIMESTAMPTZ,
    state_version      INTEGER NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_cone_bindings_unique
    ON swarm_cone_bindings (tenant_id, feature_id, task_id)
    WHERE disabled_at IS NULL;

ALTER TABLE swarm_cone_bindings ENABLE ROW LEVEL SECURITY;
CREATE POLICY cone_bindings_tenant_isolation ON swarm_cone_bindings
    USING (tenant_id = current_setting('efe.tenant_id', true));

CREATE TABLE IF NOT EXISTS swarm_artefact_manifests (
    manifest_id        UUID PRIMARY KEY,
    tenant_id          TEXT NOT NULL,
    artefact_id        TEXT NOT NULL,
    digest             BYTEA NOT NULL,
    seal               BYTEA NOT NULL,
    synced_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    source_bucket      TEXT NOT NULL,
    target_bucket      TEXT NOT NULL,
    dim                INTEGER NOT NULL,
    tokenizer_family   TEXT,
    labels             JSONB NOT NULL DEFAULT '{}'::jsonb
);
CREATE INDEX IF NOT EXISTS idx_artefact_tenant ON swarm_artefact_manifests(tenant_id);
ALTER TABLE swarm_artefact_manifests ENABLE ROW LEVEL SECURITY;
CREATE POLICY artefact_manifest_tenant_isolation ON swarm_artefact_manifests
    USING (tenant_id = current_setting('efe.tenant_id', true));

CREATE TABLE IF NOT EXISTS swarm_call_log (
    call_id            UUID PRIMARY KEY,
    occurred_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    tenant_id          TEXT NOT NULL,
    actor_id           TEXT NOT NULL,
    method             TEXT NOT NULL,
    status_code        TEXT NOT NULL,
    latency_ms         REAL NOT NULL,
    swarm_id           TEXT,
    rounds_executed    INTEGER,
    verdict            TEXT,
    error_code         TEXT,
    attributes         JSONB NOT NULL DEFAULT '{}'::jsonb
);
CREATE INDEX IF NOT EXISTS idx_call_log_tenant_time
    ON swarm_call_log(tenant_id, occurred_at);
ALTER TABLE swarm_call_log ENABLE ROW LEVEL SECURITY;
CREATE POLICY call_log_tenant_isolation ON swarm_call_log
    USING (tenant_id = current_setting('efe.tenant_id', true));
```

---

## Annexe D — Schéma YAML d'un catalogue d'ancres

```yaml
catalog_id: support_triage
version: 3
tenant: acme
task: triage-v2
embedder:
  family: sentence-transformers
  model: all-MiniLM-L6-v2
  version: 2.7
dim: 384
locale: fr-FR
defaults:
  max_projection_distance: 0.35
labels:
  - label: fraud
    description: "Débit non autorisé, carte volée"
    anchors:
      - "J'ai été débité sans consentement"
      - "Une transaction non autorisée figure sur mon compte"
  - label: billing
    description: "Facture incorrecte, abonnement"
    anchors:
      - "Ma facture ne correspond pas"
  - label: technical
    description: "Bug, erreur applicative"
    anchors:
      - "L'application plante quand je clique sur Valider"
notes: |
  Toute évolution du catalogue impose une revue PR + tests golden set.
```

---

## Annexe E — Matrice de risques M12

| Id | Risque | Probabilité | Impact | Mitigation |
|----|--------|-------------|--------|------------|
| R1 | Drift protobuf entre Python et Rust | Moyenne | Haut | snapshot CI + test roundtrip |
| R2 | Saturation HBM serveur | Moyenne | Critique | HbmBudgeter + DescribeSpec |
| R3 | mTLS expiration silencieuse | Faible | Haut | cert-manager + liveness |
| R4 | Circuit breaker flap | Faible | Moyen | seuil 20 samples + 30s window |
| R5 | Fuite de PII via raw_prompt | Faible | Critique | M11 PII linter pré-wire |
| R6 | Tenant bypass via metadata forgé | Très faible | Critique | metadata issu de SecurityContext seulement |
| R7 | Leak streaming après disconnect | Faible | Moyen | finally block + bulkhead release |
| R8 | Artefact corrompu | Faible | Haut | digest SHA-256 + HMAC double-check |
| R9 | Cone evolve non-transactionnel | Faible | Haut | AdminService garantit atomicité côté Rust |
| R10 | BreachTailer cursor perdu | Faible | Moyen | cursor persisté en SQL |

---

## Annexe F — Matrice RACI

R = Responsible · A = Accountable · C = Consulted · I = Informed.
Rôles : **ARCH** architecte plateforme, **BE** backend Python,
**RUST** core team, **SRE** ops, **QA** qualité, **SEC** sécurité.

| Sous-système | ARCH | BE | RUST | SRE | QA | SEC |
|--------------|------|-----|------|-----|-----|-----|
| Transport | A | R | C | R | C | R |
| Contract adapter | A | R | R | I | R | C |
| Catalog bridge | A | R | C | C | C | C |
| Artefact sync | C | R | C | R | C | R |
| Client | A | R | I | I | R | I |
| Reconciler | C | R | I | I | R | I |
| Budget | A | R | C | R | C | I |
| Flow control | A | R | I | R | R | I |
| Streaming | C | R | C | R | C | I |
| Admin | A | R | R | I | C | R |
| Telemetry | A | R | C | R | R | I |

---

## Annexe G — Glossaire d'intégration

| Terme | Définition |
|-------|------------|
| **Adapter** | Composant qui traduit deux formats (proto ↔ Python) |
| **Bulkhead** | Isolation d'un groupe de ressources contre l'épuisement mutuel |
| **Circuit breaker** | État machine qui coupe les appels vers un backend en panne |
| **Cone (alignment)** | Ensemble d'ancres + radius utilisé par l'essaim pour projeter |
| **Contract snapshot** | Image figée du schéma protobuf utilisée par la CI |
| **DEK/KEK** | Hiérarchie de clés (hérité de M11) |
| **Idempotency key** | Identifiant d'action rejouable sans effet second |
| **mTLS** | Mutual TLS — authentification par certificat des deux côtés |
| **Nano-agent** | Agent minimal du swarm, spécialisé par template |
| **NaSem / NaLog / NaCrit** | Templates de nano-agents (sémantique, logique, critique) |
| **Reconciler** | Composant qui traduit le verdict en résultat métier |
| **SPIFFE** | Standard d'identité service-to-service |
| **Swarm verdict** | Issue du consensus : converged, divergence, mode-collapse |
| **Template code** | Enum Rust pour les types de nano-agents |

---

*Fin du document. Relecture trimestrielle obligatoire par l'équipe
plateforme + team Rust + SRE. Mise à jour après chaque upgrade
protobuf, chaque rupture d'invariant, chaque modification d'enveloppe
de performance. Toute mutation du snapshot contractuel requiert une
ADR signée.*
