# Lettre E — Service backend (sidecar) · Architecture détaillée

> **Statut** : Draft v1.0.0 — Document de référence pour l'adoption de niveau E du Mode 1 EFE.
> **Auteur** : rodrigue.pekar@icloud.com
> **Date** : 2026-04-19
> **Dépendances doc** : `LETTER_A_ARCHITECTURE.md`, `LETTER_B_ARCHITECTURE.md`, `LETTER_C_ARCHITECTURE.md`, `LETTER_D_ARCHITECTURE.md`, `MODE1_ARCHITECTURE.md`, `M9_ARCHITECTURE.md`, `M10_ARCHITECTURE.md`, `M12_ARCHITECTURE.md`.
> **Portée** : déploiement d'`efe-server` comme **service réseau managé** consommable par des clients hétérogènes (Go, TypeScript, Java, Python, …) via gRPC, avec l'ensemble des disciplines opérationnelles attendues d'un service de production multi-tenant : SLO publié, mTLS, rate-limiting, admin API, observabilité complète, CI/CD, runbook on-call.

---

## Table des matières

1. Positionnement et contrats
2. État actuel (AS-IS) — inventaire et gaps infra
3. Cas d'usage canoniques et anti-usages
4. Architecture générale — data plane et control plane
5. Surface gRPC publique — le contrat inter-équipes
6. Topologies de déploiement — sidecar vs central vs fédéré
7. Cycle de vie des pods et graceful shutdown
8. Authentification, autorisation et rotation de secrets
9. Multi-tenancy — isolation, quotas, chargeback
10. Observabilité — metrics, logs, tracing, alerting
11. Performance et scaling
12. SLO, error budget et politique de freeze
13. Sécurité d'infrastructure
14. Stratégie de tests en environnement E
15. CI/CD et release engineering
16. Versioning protocolaire
17. Documentation et onboarding des équipes consommatrices
18. Dépendances d'infrastructure
19. Plan d'implémentation
20. Matrice de risques
21. Critères Go / No-Go
22. Annexes

---

## §1. Positionnement et contrats

### §1.1 Définition opérationnelle de E

La **Lettre E — Service backend** est la forme d'intégration qui satisfait la propriété suivante :

> *EFE tourne comme un service réseau autonome, déployé et maintenu par une équipe plateforme distincte des équipes consommatrices. N équipes produits, N runtimes différents (Go, TypeScript, Java, Python, …), M versions de catalog, tous se parlent via un contrat gRPC figé, un SLO publié, et une discipline on-call 24 / 7 — sans que la moindre équipe cliente ne doive embarquer `sentence-transformers` ou monter un pod EFE chez elle.*

Cette propriété a **cinq corollaires structurants**, les plus forts de la trajectoire A → E :

- **C1 — EFE devient un service autonome.** Conteneur dédié, image signée, Deployment Kubernetes, SLA. L'équipe plateforme *possède* le service ; les équipes produits ne *consomment* que son endpoint.
- **C2 — Consommation multi-runtime.** Les clients ne sont plus forcément Python. Go, TypeScript, Java et Rust sont des cibles de première classe. Cela impose des **stubs gRPC régénérés par langue** publiés en SDK.
- **C3 — SLO publié et défendu.** Availability, p95, error budget — contractualisés dans un document public, mesurés en continu, violés = freeze.
- **C4 — Control plane séparé du data plane.** Enregistrement de catalogs, rotation de secrets, audit d'accès passent par une **AdminService** distincte de l'API Classify. Une panne du control plane n'affecte pas le data plane.
- **C5 — Déploiement découplé des consommateurs.** EFE se release à sa propre cadence, sans coordonner avec les releases clientes. Compatibilité ascendante protobuf stricte.

### §1.2 Place dans la trajectoire A → E

| Lettre | Infrastructure cible | Utilisateurs visés | Concurrence | Proprio du process |
|---|---|---|---|---|
| A | binaire CLI | un humain ponctuel | non | OS local |
| B | REPL shell | un humain en atelier | non | utilisateur |
| C | script Python | un scénario métier | non | équipe produit |
| D | lib embarquée | **une** app Python hôte | oui | équipe applicative |
| **E** | **service managé** | **N équipes × M runtimes** | **oui** | **équipe plateforme** |

E est la **seule lettre qui requiert une équipe plateforme** pour être viable. Sans équipe on-call, sans dashboards, sans runbook, sans budget cloud : pas de E. C'est le palier où EFE quitte le statut de bibliothèque et devient de l'*infrastructure partagée*.

### §1.3 Contrats externes

E expose **sept surfaces contractuelles**, versionnées indépendamment :

- **Surface gRPC data plane** : `efe.mode1.v1.Mode1Service` — figée, bumpable uniquement par création d'une nouvelle version (`v2` en parallèle).
- **Surface gRPC control plane** : `efe.admin.v1.AdminService` — idem, isolée du data plane.
- **Surface HTTP/REST (opt.)** : gateway `grpc-gateway` qui miroite les RPC gRPC en `POST /v1/classify`. Même discipline de versioning.
- **Surface SDK** : packages publiés par langue (`efe-tool` Python, `efe-client-go`, `@efe/client-ts`, `com.efe:client-java`). Chacun sa propre SemVer, mais figé à une version proto.
- **Surface SLO** : document public (`SLO.md`) — disponibilité, p95, error budget, politique de dégradation.
- **Surface Auth** : mTLS CA chain, schéma JWT claim. Toute rupture = migration coordonnée.
- **Surface Observability** : noms de metrics Prometheus, format JSON des logs, conventions OpenTelemetry — stables pour que les dashboards client restent fonctionnels.

### §1.4 Principes invariants de E

- **P1 — Stateless au niveau pod.** Aucune state locale qui doit survivre au restart d'un pod. Catalogues chargés en mémoire sont *caches*, pas source de vérité.
- **P2 — Catalogs en source de vérité externe.** Les catalogs YAML + `.compiled.json` vivent dans un bucket S3 / GCS / PVC, pas en read-only dans l'image.
- **P3 — Auth obligatoire.** Pas de RPC Classify acceptée sans mTLS ou JWT valide. Rejet au niveau ingress.
- **P4 — Quota par tenant.** Chaque consommateur a un `tenant_id` et un rate-limit distinct. Un tenant bruyant ne noie pas les autres.
- **P5 — SLO mesuré, pas promis.** Tout objectif annoncé est observable via un dashboard public. On arrête d'annoncer ce qu'on ne mesure pas.
- **P6 — Proto additif seulement.** On *ajoute* des champs en version mineure, on *ne supprime jamais* dans une version majeure donnée. Breaking = nouvelle version majeure.
- **P7 — Release progressive.** Toute nouvelle version passe par canary (5 % traffic), puis rolling. Un rollback prend moins de 5 minutes.
- **P8 — Shutdown gracieux.** SIGTERM → refuse nouveaux → drain in-flight avec timeout → close gRPC channels → exit 0. Pod termination windows calés.
- **P9 — Pas de secret dans l'image.** Tous les secrets (JWT signing key, mTLS client certs) montés au runtime via Kubernetes Secrets / Vault Agent / CSI driver.
- **P10 — Admin plane séparé.** L'AdminService écoute sur un port différent, protégé par une auth plus stricte (admin JWT ou mTLS RBAC). Un accès admin compromis ne donne pas accès au data plane, et inversement.
- **P11 — Observability par défaut.** Chaque pod expose `/metrics`, émet des logs JSON structurés, propage les spans OpenTelemetry. Désactivation opt-out, jamais opt-in.
- **P12 — Sandbox par défaut.** Containers non-root, `readOnlyRootFilesystem: true`, `seccompProfile: RuntimeDefault`, NetworkPolicies restrictives.

### §1.5 Différences contractuelles avec D

| | D (lib embarquée) | E (service) |
|---|---|---|
| Processus | partagé avec l'app hôte | **dédié** (container séparé) |
| Langage consommateur | Python uniquement | **multi-langage** |
| Cold start | amorti par le boot de l'app | **facturé au pod scale-up** |
| Observabilité | hôte | **propre stack + dashboards** |
| SLA | best effort de l'app | **SLO formel** |
| Multi-tenant | non | **oui (design central)** |
| Cycle de release | release hôte | **indépendant** |
| Auth | dictée par l'hôte | **obligatoire au niveau service** |
| Catalog lifecycle | boot de l'hôte | **API d'admin dédiée** |
| Coût | caché (RAM hôte) | **visible (pods + infra)** |

Le pivot essentiel : **E a son propre ROI explicite**. Tant qu'un seul consommateur existe, D est plus simple et moins cher. Dès qu'apparaissent un second consommateur dans un runtime différent, ou un besoin de canary rollout cross-équipes, E devient économiquement défendable.

---

## §2. État actuel (AS-IS)

### §2.1 Ce qui existe

| Composant | Source | Maturité |
|---|---|---|
| `efe-server` binaire Rust | M10 | Fonctionnel, tests unitaires verts |
| Proto `efe.mode1.v1.Mode1Service` | M10 | Défini, versionné |
| Proto `efe.admin.v1.AdminService` | M9 | Partiel (RegisterCatalog implémenté) |
| Health endpoint gRPC | M10 | Standard `grpc.health.v1` |
| Cargo workspace | repo root | `cargo build --release -p efe-server` OK |
| Telemetry crate | `efe-telemetry` | Hooks basiques (tracing-subscriber) |
| Python client + stubs | M11 | Référence pour autres langages |

### §2.2 Ce qui manque — le gap E

Tous les éléments suivants sont *absents* ou *partiels* :

- **G1 — Dockerfile production**. Image existante (si elle existe) est pour dev. Pas multi-stage, pas distroless, pas signée.
- **G2 — Manifests Kubernetes**. Pas de Deployment / Service / HPA / NetworkPolicy / PodDisruptionBudget dans `deploy/k8s/`.
- **G3 — mTLS et JWT**. Le code server a un stub `--insecure=false` mais aucune chaîne de confiance documentée.
- **G4 — Rate-limiting multi-tenant**. Pas de quota par `tenant_id`. Un client peut saturer le swarm.
- **G5 — AdminService complet**. Manque `ListCatalogs`, `GetCatalog`, `DeleteCatalog`, audit log.
- **G6 — Catalog storage externe**. Actuellement les catalogs sont dans `catalogs/` du repo, montés en volume ; aucun pattern S3/GCS documenté.
- **G7 — Observability stack**. `/metrics` Prometheus existe (M12 partiel) mais pas de règles Alertmanager, pas de dashboards Grafana livrés.
- **G8 — SDK Go/TS/Java**. Aucun stub client hors Python.
- **G9 — CI/CD pipeline**. Pas de release automatisée de l'image container ni des SDK.
- **G10 — Runbook on-call**. Zéro doc "à 3 h du matin".
- **G11 — Load tests**. Aucun k6/locust script.
- **G12 — SLO publié**. Pas de `SLO.md`.

### §2.3 Ce qui est acquis côté langage de référence

M11 a livré un client Python propre, testé à 37+ tests, qui peut servir de *référence comportementale* pour tous les autres SDK. Cela garantit qu'un consommateur TS/Go/Java peut recrypto le comportement attendu (retry, error mapping, idempotency) en s'inspirant de Python.

### §2.4 Synthèse

E est **90 % infrastructure, 10 % code applicatif**. Les deux crates Rust (`efe-server`, `efe-telemetry`) sont le substrat ; tout le reste est de la *discipline opérationnelle* : images, manifests, dashboards, alertes, runbooks, SDK multi-langage. Le chantier est plus *ops* que *dev*.

---

## §3. Cas d'usage canoniques et anti-usages

### §3.1 UC-E1 — Équipe Go consomme EFE

**Contexte.** Une équipe backend travaille en Go/gRPC. Elle veut router les tickets support sans dépendre de Python.

```go
// consumer (Go)
import efev1 "github.com/your-org/efe-client-go/mode1/v1"

cli := efev1.NewMode1Client(conn)
resp, err := cli.Classify(ctx, &efev1.ClassifyRequest{
    Text:     "my card was charged",
    Catalog:  "support_triage",
    TenantId: "team-payments",
})
if err != nil { /* map errors */ }
switch {
    case resp.Reject: /* ask clarification */
    case resp.Confidence >= 0.82: /* act on resp.Label */
    default: /* escalate to human */
}
```

**Attentes.**
- Stub Go publié dans un repo ou registry propre (GitHub, BuildBuddy, Artifactory).
- mTLS déjà câblé via `cert-manager` côté cluster, transparente pour l'équipe Go.
- Quota par tenant géré en amont — latence stable indépendamment des autres équipes.

### §3.2 UC-E2 — Plateforme multi-team

**Contexte.** Cinq équipes produits utilisent EFE : support, fraud, billing, legal, churn. Chaque équipe a son catalog, ses seuils de confidence, son propre dashboard.

```
team-support    → catalog "support_triage"     (QPS 50)
team-fraud      → catalog "fraud_alert"        (QPS 10, p95 stricte)
team-billing    → catalog "billing_escalation" (QPS 20)
team-legal      → catalog "legal_triage"       (QPS 5, forte confidence)
team-churn      → catalog "churn_predictor"    (batch, off-hours)
```

**Attentes.**
- AdminService permet à l'équipe plateforme (seule) de register les catalogs.
- Dashboard segmenté par tenant : chaque équipe voit *ses* RED metrics.
- Chargeback mensuel basé sur `efe_classify_total{tenant="team-xyz"}`.

### §3.3 UC-E3 — Batch nocturne

**Contexte.** Un pipeline nocturne classifie 500k messages en 4 h. Sans throttling, il sature EFE pour les équipes temps réel.

**Attentes.**
- Tenant `team-churn` a un rate-limit de 100 QPS (vs 1000 pour support temps réel).
- Job batch respecte son quota via backoff client.
- Dashboard montre le pic nocturne sans alertes SLO-breach.

### §3.4 UC-E4 — Migration D → E

**Contexte.** Une app FastAPI utilise actuellement EFE en Lettre D (embarqué). L'équipe décide de déporter vers E pour partager avec une autre équipe.

**Migration.**

1. Phase 1 : déployer `efe-server` central (E), pointe ancienne app vers `EFE_ENDPOINT=efe-service:50051` (feature flag).
2. Phase 2 : 10 % du traffic passe par E, 90 % reste en D. Compare verdicts via shadow log.
3. Phase 3 : 100 % E, retirer les dépendances D de l'app.
4. Phase 4 : la seconde équipe consomme E.

**Attentes.** Retour en arrière trivial (flip feature flag) pendant les phases 1-2.

### §3.5 UC-E5 — A/B de versions de catalog

**Contexte.** Le catalog `support_triage` a une v1.0 en prod. L'équipe tune et sort une v1.1 avec trois nouvelles canonical sentences.

**Flow.**

1. Upload v1.1 via `AdminService.RegisterCatalog` avec tag `canary`.
2. `efe-server` configure traffic split : 5 % `@v1.1`, 95 % `@v1.0`.
3. Metrics comparées (accuracy estimée via reject rate, drift de labels).
4. Promotion de v1.1 → `primary`, v1.0 → `legacy`.
5. v1.0 supprimée après 2 semaines sans rollback.

**Attentes.** Aucun consommateur ne change de code. Le routing est côté serveur.

### §3.6 Anti-usages explicites

- **Single-consumer, Python-only.** D est bien meilleur : moins d'infra, moins de latence, moins de coût. E est overkill.
- **Prototype / PoC.** Mettre en place un cluster K8s + mTLS pour valider une hypothèse métier = erreur d'allocation de temps. C ou D suffisent.
- **Latence ultra-basse (< 100 ms p95).** Le réseau ajoute 5-30 ms ; l'embedder ajoute 50-100 ms. Si votre SLA est 100 ms, seul D embarqué satisfait.
- **Absence d'équipe on-call.** Un service E sans on-call est une dette technique active. Mieux vaut rester sur D.

---

## §4. Architecture générale

### §4.1 Diagramme — data plane + control plane

```
                    ┌──────────────────────────────────────┐
                    │        Clients (N runtimes)          │
                    │   Go · TS · Java · Python · Rust     │
                    └─────────┬────────────────┬───────────┘
                              │                │
                       (data plane)     (control plane)
                          gRPC :443        gRPC :8443
                              │                │
                              ▼                ▼
                    ┌──────────────────────────────────────┐
                    │       Ingress (Istio / Envoy)        │
                    │  TLS termination · JWT validation    │
                    │  rate limiting · audit log           │
                    └─────────┬────────────────┬───────────┘
                              │                │
            ┌─────────────────┼────────────────┼──────────┐
            │                 │                │          │
            ▼                 ▼                ▼          ▼
     ┌────────────┐    ┌────────────┐   ┌────────────┐   ...
     │ efe-server │    │ efe-server │   │ efe-server │
     │   pod 1    │    │   pod 2    │   │   pod N    │
     │            │    │            │   │            │
     │ ┌────────┐ │    │ ┌────────┐ │   │ ┌────────┐ │
     │ │ Data   │ │    │ │ Data   │ │   │ │ Data   │ │
     │ │ plane  │ │    │ │ plane  │ │   │ │ plane  │ │
     │ │:50051  │ │    │ │:50051  │ │   │ │:50051  │ │
     │ └────────┘ │    │ └────────┘ │   │ └────────┘ │
     │ ┌────────┐ │    │ ┌────────┐ │   │ ┌────────┐ │
     │ │ Admin  │ │    │ │ Admin  │ │   │ │ Admin  │ │
     │ │:50052  │ │    │ │:50052  │ │   │ │:50052  │ │
     │ └────────┘ │    │ └────────┘ │   │ └────────┘ │
     │ ┌────────┐ │    │ ┌────────┐ │   │ ┌────────┐ │
     │ │/metrics│ │    │ │/metrics│ │   │ │/metrics│ │
     │ │:9090   │ │    │ │:9090   │ │   │ │:9090   │ │
     │ └────────┘ │    │ └────────┘ │   │ └────────┘ │
     └──────┬─────┘    └──────┬─────┘   └──────┬─────┘
            │                 │                │
            └────── shared ───┴────────────────┘
                              │
                              ▼
                    ┌──────────────────────┐
                    │ Catalog Store        │
                    │ S3 / GCS / NFS PVC   │
                    │ (read-mostly, mTLS)  │
                    └──────────────────────┘
```

### §4.2 Plans et ports

| Plan | Port | Protocole | Auth |
|---|---|---|---|
| Data | 50051 | gRPC | mTLS + JWT (tenant) |
| Admin | 50052 | gRPC | mTLS + JWT (admin role) |
| Metrics | 9090 | HTTP | NetworkPolicy (Prometheus only) |
| Health | 50053 | gRPC | ouvert (aucune info sensible) |

La séparation ports Data/Admin permet :
- Différencier les NetworkPolicies (Admin accessible seulement depuis un namespace `efe-admin`).
- Limiter l'impact d'une fuite de credential.
- Opérer un rate-limit plus strict sur Admin (faible QPS nominal).

### §4.3 Boundaries

- **B1 — Ingress ↔ Pod.** L'ingress termine TLS et vérifie le JWT ; le pod accepte uniquement depuis l'ingress via mTLS interne.
- **B2 — Data plane ↔ Control plane.** Jamais de handler data qui consulte l'admin plane en hot path. Les catalogs sont chargés en mémoire et servis depuis cache local.
- **B3 — Pod ↔ Catalog Store.** Read-mostly. Admin service *écrit*, data plane *lit* via un cache avec TTL (1h) ou un poll signal.
- **B4 — Pod ↔ Observability.** Logs JSON sur stdout, metrics sur `/metrics`, spans via OTLP exporter. Aucun push depuis le pod vers une infra externe (pull model).

### §4.4 Invariants architecturaux

- **I1.** Aucun pod n'écrit dans le catalog store sauf via l'AdminService explicite.
- **I2.** Aucune mutation de catalog en mémoire n'est possible hors de l'admin plane.
- **I3.** L'AdminService valide strictement tous les inputs (nom de catalog, fingerprint embedder, schema) avant d'écrire.
- **I4.** Un pod qui n'arrive pas à lire un catalog continue à servir le catalog *précédent en mémoire*.
- **I5.** Un pod en crash-loop n'accepte pas de traffic (readinessProbe retourne 503 avant que le loop commence).

---

## §5. Surface gRPC publique

### §5.1 Proto `efe.mode1.v1.Mode1Service`

```proto
syntax = "proto3";
package efe.mode1.v1;

service Mode1Service {
  rpc Classify(ClassifyRequest) returns (ClassifyResponse);
  rpc DescribeSpec(DescribeSpecRequest) returns (DescribeSpecResponse);
}

message ClassifyRequest {
  string text = 1;
  string catalog = 2;
  string tenant_id = 3;          // rate-limit key
  string request_id = 4;         // idempotency + tracing
  int32  max_rounds = 5;         // override; default = catalog config
  int32  budget_wall_clock_ms = 6;  // override
  bool   return_sub_scores = 7;
  map<string, string> metadata = 8; // extension
}

message ClassifyResponse {
  string label = 1;
  float  confidence = 2;
  float  alignment_delta = 3;
  int32  rounds = 4;
  bool   reject = 5;
  string zone = 6;               // "trusted" | "warn" | "reject"
  repeated SubScore sub_scores = 7;
  string catalog_version = 8;    // echoed for tracing
  int64  server_elapsed_ms = 9;
  string swarm_id = 10;
  string served_by_pod = 11;     // debug
}

message SubScore {
  string label = 1;
  float  cosine = 2;
}

message DescribeSpecRequest { SwarmSpec spec = 1; }
message DescribeSpecResponse {
  bool ok = 1;
  string reason = 2;
  int32 estimated_cost_ms = 3;
}
```

### §5.2 Proto `efe.admin.v1.AdminService`

```proto
syntax = "proto3";
package efe.admin.v1;

service AdminService {
  rpc RegisterCatalog(RegisterCatalogRequest) returns (RegisterCatalogResponse);
  rpc ListCatalogs(ListCatalogsRequest) returns (ListCatalogsResponse);
  rpc GetCatalog(GetCatalogRequest) returns (GetCatalogResponse);
  rpc DeleteCatalog(DeleteCatalogRequest) returns (DeleteCatalogResponse);
  rpc ReloadCatalogs(ReloadCatalogsRequest) returns (ReloadCatalogsResponse);
  rpc RotateJwtKey(RotateJwtKeyRequest) returns (RotateJwtKeyResponse);
  rpc GetAuditLog(GetAuditLogRequest) returns (stream AuditLogEntry);
}

message RegisterCatalogRequest {
  bytes yaml_content = 1;        // catalog YAML raw bytes
  string name = 2;               // must match metadata.name
  string version = 3;            // must match metadata.version
  string tag = 4;                // "primary" | "canary" | "legacy"
  string embedder_fingerprint = 5;
  string registered_by = 6;      // actor (from JWT sub)
}

message CatalogInfo {
  string name = 1;
  string version = 2;
  string tag = 3;
  string fingerprint = 4;
  repeated string classes = 5;
  int64 registered_at = 6;
  string registered_by = 7;
  int64 size_bytes = 8;
}

message AuditLogEntry {
  int64 at = 1;
  string actor = 2;
  string action = 3;             // "register" | "delete" | "rotate" ...
  string target = 4;             // catalog name@version or key id
  bool success = 5;
  string detail = 6;
}
```

### §5.3 HTTP/REST gateway (optionnel)

Un `grpc-gateway` bridge expose :

```
POST /v1/classify
POST /v1/describe-spec
```

Même authentification (JWT dans header `Authorization: Bearer ...`). mTLS terminé à l'ingress.

L'intérêt : consommateurs TypeScript en environnement contraint (serverless, edge) qui n'ont pas de stub gRPC.

### §5.4 Règles de versioning

- **Ajout de champ optionnel** : V1.x mineur, non-breaking.
- **Retrait de champ** : requiert `v2` séparé, déprécation sur 2 versions mineures avant retrait.
- **Renommage** : considéré comme retrait + ajout. Jamais à la volée.
- **Nouvelle RPC** : V1.x mineur.
- **Retrait de RPC** : `v2`.

Le namespace `v2` cohabite avec `v1` pendant 6 mois minimum.

### §5.5 Error codes gRPC

| Code gRPC | Signification côté EFE | Client doit |
|---|---|---|
| `OK` | verdict émis | consommer la response |
| `UNAVAILABLE` | pod en drain, ingress down | retry avec backoff |
| `DEADLINE_EXCEEDED` | budget dépassé côté serveur | clarifier si retry ne converge pas |
| `RESOURCE_EXHAUSTED` | quota tenant atteint | backoff respectant `retry_after` metadata |
| `INVALID_ARGUMENT` | input mal formé (catalog inconnu, texte vide) | corriger, **pas** retry |
| `PERMISSION_DENIED` | JWT invalide ou tenant non autorisé | renouveler token |
| `UNAUTHENTICATED` | mTLS échoue / JWT absent | renouveler auth |
| `FAILED_PRECONDITION` | catalog en cours de reload | retry |
| `INTERNAL` | bug côté serveur | escalate ; ne pas retry plus de 3× |
| `NOT_FOUND` | catalog inexistant | corriger, pas retry |

### §5.6 Idempotency

Chaque `ClassifyRequest` peut porter un `request_id`. Le serveur maintient une fenêtre de déduplication de 60 s : deux requêtes avec le même `request_id` dans cette fenêtre reçoivent le même `ClassifyResponse` (sans re-calcul). Cela protège les clients face aux retries sur network blip.

---

## §6. Topologies de déploiement

### §6.1 Topologie A — Single-tenant central (MVP)

Un seul cluster Kubernetes, un seul namespace `efe`, un seul Deployment de 3-10 pods. Tous les consommateurs appellent via l'ingress.

```
consumers ─┐
           ├─→ efe-ingress ─→ efe-service ─→ efe-pod-[1..N]
consumers ─┘
```

**Quand.** Moins de 5 équipes, ≤ 500 QPS, un seul catalog ou catalogs homogènes.

**Avantages.** Simple à opérer, facturation mutualisée, dashboards unifiés.

**Limites.** Noisy neighbor : un tenant mal bornée impacte les autres.

### §6.2 Topologie B — Multi-tenant isolé

Un cluster, un namespace `efe`, mais plusieurs Deployments : un par tenant *à fort SLA*, un mutualisé pour le reste.

```
team-fraud ─→ efe-fraud-deployment (3 pods)    — SLA strict p99 < 2s
team-legal ─→ efe-legal-deployment (2 pods)    — isolation dédiée
others     ─→ efe-shared-deployment (5 pods)   — mutualisé
```

**Quand.** Un ou deux tenants critiques, le reste best-effort.

**Avantages.** Isolation forte pour les critiques. Autres bénéficient du pool mutualisé.

**Limites.** Coût plus élevé (plus de pods), complexité de routing côté ingress.

### §6.3 Topologie C — Sidecar par app

Chaque app déploie son propre pod `efe-server` à côté du container applicatif (sidecar dans le même pod).

```
pod app-A: [ app-A | efe-server ]   ← communication via 127.0.0.1
pod app-B: [ app-B | efe-server ]
```

**Quand.** Latence ultra-critique (localhost), isolation totale par application.

**Avantages.** Pas de hop réseau, pas de noisy neighbor, blast radius minimal.

**Limites.** Coût élevé (modèle chargé N fois en RAM), pas de mutualisation.

### §6.4 Topologie D — Multi-région (V1.2)

Réplication inter-régions avec routing géo-proche côté ingress.

```
US traffic ─→ ingress-us ─→ efe-deployment-us
EU traffic ─→ ingress-eu ─→ efe-deployment-eu
```

Catalog store réplique via S3 cross-region ou object store global.

### §6.5 Matrice de choix

| Propriété | Topo A | Topo B | Topo C | Topo D |
|---|---|---|---|---|
| Coût infra | $ | $$ | $$$ | $$$$ |
| Latence | moyenne | moyenne | basse | variable |
| Isolation | faible | moyenne | totale | totale |
| Opérabilité | simple | moyenne | complexe | complexe |
| MVP | **✓** | non | non | non |

### §6.6 Manifests K8s MVP (Topologie A)

Annexe §22.B livre les YAML complets : `Deployment`, `Service`, `HorizontalPodAutoscaler`, `PodDisruptionBudget`, `NetworkPolicy`, `ServiceMonitor`, `PrometheusRule`.

Points clés :
- `replicas: 3` minimum (pour PDB `minAvailable: 2`).
- Resources : `requests: cpu=500m memory=768Mi`, `limits: cpu=2 memory=1.5Gi`.
- `readinessProbe` : `grpc-health-probe` sur `:50053`, `initialDelaySeconds: 20`.
- `livenessProbe` : idem, `initialDelaySeconds: 60` (laisse le model load).
- `terminationGracePeriodSeconds: 45` (draining window).

---

## §7. Cycle de vie des pods

### §7.1 Boot

```
T=0      container start
T=5 ms   argparse + lire config (env + mounted secrets)
T=10 ms  ouvrir port 50053 (health) — retourne NOT_SERVING
T=50 ms  init telemetry (tracing, metrics registry)
T=100 ms charger catalogs from store (cache priming)
T=200 ms load embedder weights from disk cache
T=1.5 s  embedder ready
T=1.5 s  health retourne SERVING
T=1.5 s  ports 50051 / 50052 écoutent
```

### §7.2 Steady state

- Traitement des RPC Classify → swarm → verdict.
- AdminService reçoit des opérations rares.
- Metrics incrémentent, logs s'écrivent, spans s'exportent.

### §7.3 Shutdown gracieux

```
T=0      SIGTERM reçu (K8s ou operator)
T=1 ms   health retourne NOT_SERVING (loadbalancer bascule vers autres pods)
T=50 ms  refuse new RPC (renvoie UNAVAILABLE avec retry-after)
T=50ms+  drain : attend in-flight RPC jusqu'à 30s max
T≤30s    si in-flight > 0 : cancel channels, log warning
T=30s+   exit 0
```

PodDisruptionBudget `minAvailable: 2` empêche K8s de drain plus de 1 pod à la fois parmi 3.

### §7.4 Hot-reload de catalog

`AdminService.RegisterCatalog` :

1. Valide le YAML (schema, fingerprint, non-empty classes).
2. Compile → `.compiled.json`.
3. Écrit dans store externe avec version + tag.
4. Broadcast via pub/sub (Redis / NATS) ou poll 60s : tous les pods rechargent.
5. Copy-on-write en mémoire (cf. D §8.3) : requêtes en cours continuent sur ancienne version, nouvelles voient la nouvelle.
6. Audit log écrit (qui, quand, quoi).

### §7.5 Rolling deployment

- `maxSurge: 1, maxUnavailable: 0` : toujours au moins N pods en service.
- Canary phase (§14.4) avant rolling complet.
- Automated rollback si error rate > 2× baseline sur 2 minutes.

---

## §8. Authentification et autorisation

### §8.1 Couche 1 — mTLS

**Inter-service.** Tout appel au port 50051 vient d'une charge de travail avec un certificat client signé par la CA interne (gérée par `cert-manager`).

```
CA root → intermediate CA (EFE) → cert-manager → Pod client cert
```

Rotation automatique des certs : durée de vie 24h, renouvelés à 16h.

### §8.2 Couche 2 — JWT (fine-grained)

Après mTLS, le client doit présenter un JWT dans `Authorization: Bearer ...`. Le JWT contient :

```json
{
  "iss": "https://auth.internal",
  "sub": "service-account-team-support",
  "tenant_id": "team-support",
  "scopes": ["efe:classify"],
  "catalogs": ["support_triage", "fraud_alert"],
  "exp": 1750000000,
  "iat": 1749950000,
  "jti": "abc-unique-id"
}
```

Le serveur valide :
- Signature (rotation de clé via AdminService).
- `exp` / `iat`.
- Scope `efe:classify` pour data plane, `efe:admin` pour control plane.
- `tenant_id` → utilisé pour rate-limit et chargeback.
- `catalogs` : si le client demande un catalog non listé → `PERMISSION_DENIED`.

### §8.3 Couche 3 — Rate limiting

Par tenant_id + par catalog. Implémentation via `tower-governor` (Rust) ou Envoy rate-limit service.

```
team-support : 1000 QPS (data), 10 QPS (admin)
team-fraud   : 500 QPS  (data), 10 QPS (admin)
team-legal   : 100 QPS  (data), 5  QPS (admin)
team-churn   : 100 QPS  (data), 0  QPS (admin) — batch only
```

Dépassement = `RESOURCE_EXHAUSTED` avec header `retry-after: 5s`.

### §8.4 Rotation de secrets

- **mTLS certs** : cert-manager rotate 24h. Zero downtime (server accepte deux CAs pendant 10 min).
- **JWT signing key** : rotation manuelle via `AdminService.RotateJwtKey`. Keep old key 7 jours (re-signe les tokens).
- **Admin JWT keys** : séparées des tenant keys. Rotation 30 jours.

### §8.5 Ingress policy

Pour le data plane : Istio `AuthorizationPolicy` qui exige mTLS + JWT scope `efe:classify`.

Pour admin : même chose + restriction namespace (only from `efe-admin` namespace).

---

## §9. Multi-tenancy

### §9.1 Modèle d'isolation

E est **multi-tenant logique** par défaut :
- Même pods servent tous les tenants.
- Isolation par JWT `tenant_id` + quota + metrics labelisés.

Pour isolation physique (cas rare), topologie B (§6.2).

### §9.2 Catalog par tenant

Un catalog appartient à un tenant (champ `owner` dans metadata) ou est `shared` (accessible à tous).

```yaml
# support_triage/owner: team-support → seul team-support peut classify
# fraud_alert/owner: shared → tous les tenants peuvent consommer
```

Le JWT du tenant `team-support` doit lister le catalog dans `catalogs: [...]` pour y accéder.

### §9.3 Quota et rate limiting

Configuration YAML versionnée dans le store :

```yaml
# tenants.yaml
tenants:
  team-support:
    classify_qps: 1000
    admin_qps: 10
    priority: high
  team-fraud:
    classify_qps: 500
    admin_qps: 10
    priority: high
  team-legal:
    classify_qps: 100
    admin_qps: 5
    priority: medium
  team-churn:
    classify_qps: 100
    admin_qps: 0
    priority: low                 # premier sacrifié sur overload
```

### §9.4 Chargeback

Prometheus exporte `efe_classify_total{tenant,catalog}` à l'heure. Un job mensuel agrège → facture via le système de billing interne (ou Excel pour MVP).

### §9.5 Canary par tenant

Un nouveau déploiement peut rester invisible à la plupart des tenants :

```
deployment-v1.5 (canary) : traffic = team-support uniquement
deployment-v1.4 (stable) : traffic = tous les autres
```

Après 48h sans incident, v1.5 devient stable pour tous.

---

## §10. Observabilité

### §10.1 Metrics Prometheus — RED model

```
# Rate
efe_classify_requests_total{tenant,catalog,outcome}

# Errors
efe_classify_errors_total{tenant,catalog,grpc_code}

# Duration
efe_classify_duration_seconds_bucket{tenant,catalog,le="…"}
efe_classify_duration_seconds_sum{tenant,catalog}
efe_classify_duration_seconds_count{tenant,catalog}

# Saturation
efe_inflight_requests{catalog}
efe_pod_memory_bytes
efe_pod_cpu_seconds_total

# Business
efe_classify_reject_total{tenant,catalog}
efe_catalog_version_info{catalog,version,fingerprint}  # info-type
```

### §10.2 Logs JSON structurés

Chaque ligne est une entrée JSON :

```json
{
  "ts": "2026-04-19T14:58:01.234Z",
  "level": "info",
  "msg": "classify_done",
  "tenant": "team-support",
  "catalog": "support_triage@1.0.0",
  "request_id": "abc-123",
  "trace_id": "0af…",
  "span_id": "bc3…",
  "pod": "efe-server-abc-xyz",
  "elapsed_ms": 842,
  "label": "fraud",
  "confidence": 0.861,
  "rounds": 4
}
```

Jamais le texte utilisateur en clair ; seulement `text_length` et `text_hash` (SHA-256 16 hex chars).

### §10.3 Tracing OpenTelemetry

Chaque span `efe.classify` :
- `efe.tenant`
- `efe.catalog`
- `efe.catalog_version`
- `efe.rounds`
- `efe.elapsed_ms`
- Parent : trace propagée depuis le consumer via W3C Trace Context.

Export OTLP vers collecteur (Jaeger / Tempo).

### §10.4 Dashboards Grafana

Trois dashboards livrés :

1. **EFE Overview** : RED agrégé, tous tenants. Cible SRE.
2. **EFE Per-Tenant** : RED segmenté. Cible équipe plateforme + tenant.
3. **EFE Fleet** : pod-level metrics (CPU, memory, restart count). Cible ops.

Dashboards versionnés en tant que JSON dans `deploy/grafana/`.

### §10.5 Alerting Alertmanager

Règles critiques :

```yaml
# Availability SLO breach
- alert: EfeErrorBudgetBurn
  expr: |
    sum(rate(efe_classify_errors_total[5m]))
    / sum(rate(efe_classify_requests_total[5m]))
    > 0.01
  for: 5m
  severity: critical

# Latency SLO breach
- alert: EfeLatencyP95Breach
  expr: |
    histogram_quantile(0.95,
      rate(efe_classify_duration_seconds_bucket[5m])
    ) > 2.0
  for: 10m
  severity: warning

# Pod flapping
- alert: EfePodRestartingFrequently
  expr: rate(kube_pod_container_status_restarts_total{pod=~"efe-server.+"}[10m]) > 0.3
  severity: warning
```

### §10.6 Health endpoints

- `grpc.health.v1.Health/Check` : SERVING / NOT_SERVING (pour readiness).
- HTTP `/healthz` : 200 si alive, 503 si shutting down (pour liveness simple).
- HTTP `/readyz` : 200 si ready (embedder chargé, catalogs loaded), 503 sinon.

---

## §11. Performance et scaling

### §11.1 Budget de performance par pod

| Phase | Budget p95 |
|---|---|
| Reception RPC + auth | < 20 ms |
| Input validation | < 1 ms |
| LRU cache lookup | < 1 ms |
| Embedder encode | 30-80 ms (CPU) |
| Swarm consensus | 300-1800 ms |
| Output serialization | < 5 ms |
| **Total p95** | **< 2 s** |

### §11.2 Capacity planning

Sur machine de référence (M3 Pro ou AWS c7i.large) :

| Ressource | Par pod | Par QPS |
|---|---|---|
| CPU | 500m-2000m | ~50m / QPS |
| RAM | 768Mi-1.5Gi | ~0 (fixe : modèle = 500MB) |
| Network I/O | négligeable | ~2 KB / classify |

Cible pour 300 QPS : 6-10 pods suivant workload mix.

### §11.3 HPA configuration

```yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
spec:
  minReplicas: 3
  maxReplicas: 20
  metrics:
    - type: Resource
      resource: { name: cpu, target: { type: Utilization, averageUtilization: 70 } }
    - type: Pods
      pods:
        metric: { name: efe_inflight_requests }
        target: { type: AverageValue, averageValue: 8 }
  behavior:
    scaleUp:   { stabilizationWindowSeconds: 30 }
    scaleDown: { stabilizationWindowSeconds: 300 }
```

Le scale-down lent (5 min) évite de yo-yo en présence de spikes courts.

### §11.4 Cold-start mitigation

Scale-up 0 → 1 prend 5-10 s (pull image + load model). Trop long pour un spike soudain.

**Mitigations** :
- `minReplicas: 3` garantit une base stable.
- `readinessProbe` avec `failureThreshold: 3` laisse le pod atteindre READY avant de recevoir du traffic.
- Pre-warm : un init container pré-télécharge le modèle pour accélérer.

### §11.5 Caching

- **LRU local par pod** : 10 000 entrées, TTL 1 h (cf. EfeClient cache).
- **Shared Redis cache** (V1.1) : pour des textes fréquents, hit rate 20-30 % espéré.

### §11.6 Batching

Non-MVP. gRPC streaming bidirectionnel permet de batcher N requêtes → une réponse agrégée, utile pour batch jobs. Reporté V1.2.

---

## §12. SLO, error budget et freeze

### §12.1 SLO publié (document séparé `SLO.md`)

| Objectif | Valeur | Mesure |
|---|---|---|
| Availability | 99.5 % (MVP) → 99.9 % (V1.1) | `efe_classify_requests_total - efe_classify_errors_total` sur 30j |
| Latency p95 | < 2 s | `histogram_quantile(0.95, efe_classify_duration_seconds)` |
| Latency p99 | < 3 s | idem |
| Error rate | < 0.5 % (MVP) → < 0.1 % (V1.1) | `efe_classify_errors_total / efe_classify_requests_total` |

### §12.2 Error budget

Availability 99.5 % sur 30 jours = 3.6 heures d'indisponibilité permise.

Budget tracking : dashboard "EFE Error Budget" avec burn rate 5 min et 1 h.

### §12.3 Politique de freeze

- **Burn rate > 14.4×/h** (consommerait 100 % du budget en < 3 h) : auto-rollback canary, incident P1.
- **Burn cumulé > 50 %** : freeze des release non-critiques.
- **Burn cumulé > 100 %** : freeze total hors security patches ; postmortem obligatoire.

### §12.4 SLO par tenant

Optionnel (V1.1). Chaque tenant premium a son propre SLO contractuel :

```
team-fraud : 99.9 % availability, p99 < 2 s
team-support : 99.5 %, p95 < 2 s
team-churn : best effort (batch, no SLA)
```

---

## §13. Sécurité d'infrastructure

### §13.1 Container sécurisé

```yaml
securityContext:
  runAsNonRoot: true
  runAsUser: 1000
  readOnlyRootFilesystem: true
  allowPrivilegeEscalation: false
  capabilities: { drop: ["ALL"] }
  seccompProfile: { type: RuntimeDefault }
```

### §13.2 NetworkPolicies

- Data plane pod accepte ingress seulement depuis l'Istio gateway pod namespace.
- Admin plane pod accepte seulement depuis `efe-admin` namespace.
- Pods n'ont pas d'egress sauf vers :
  - Catalog store (S3 / PVC)
  - OTLP collector
  - Prometheus (scrape in, so egress pull-mode déja couvert)
  - DNS

### §13.3 Image signing

- `cosign sign` sur chaque release.
- `cosign verify` au niveau Kyverno / Gatekeeper avant pull.

### §13.4 SBOM

`syft` génère SBOM (SPDX) à chaque build. Stocké en attachment OCI à côté de l'image.

### §13.5 Vulnerability scanning

- `trivy` scanne l'image à chaque build.
- Critical / High bloquent le merge.
- Scheduled re-scan quotidien sur images en prod.

### §13.6 Audit logs

Toute opération AdminService → entrée dans `AuditLogEntry` table (Postgres ou fichier rotatif).

Rétention 1 an minimum.

---

## §14. Stratégie de tests

### §14.1 Pyramide

```
         ▲  game day / chaos      ~4 / quarter
        ╱ ╲
       ╱   ╲ canary in prod      ~1 per release
      ╱     ╲
     ╱       ╲ load test         ~1 per release + nightly
    ╱─────────╲
   ╱           ╲ integration     ~50 tests (docker-compose)
  ╱─────────────╲
 ╱               ╲ unit           ~500 tests (Rust + Python)
╱─────────────────╲
```

### §14.2 Unit

Hérité de M5-M10 Rust + Python. Doit rester vert.

### §14.3 Intégration

`docker-compose` lance efe-server + client Python + Prometheus + Grafana. Tests végétent en CI à chaque PR.

### §14.4 Load test

`k6` script :

```javascript
export const options = {
  stages: [
    { duration: '1m',  target: 100  },
    { duration: '5m',  target: 500  },
    { duration: '1m',  target: 0    },
  ],
  thresholds: {
    'http_req_duration': ['p(95)<2000'],
    'http_req_failed':   ['rate<0.01'],
  },
};
```

Exécuté chaque nuit en staging. Résultats archivés pour regression tracking.

### §14.5 Canary in prod

Release v_next → 5 % du traffic pendant 30 min. Metrics watch :

- `efe_classify_errors_total` < 2× baseline
- p95 < 1.5× baseline

Si OK → 25 % → 50 % → 100 % par paliers 15 min. Sinon → auto-rollback.

### §14.6 Game day / chaos

Trimestriel. Scénarios :
- Kill random pod.
- Network partition entre pods et catalog store.
- Expiration du cert mTLS.
- Corruption d'un catalog en cours de load.
- Spike 10× traffic nominal.

Chaque scenario → runbook validé, temps de résolution mesuré.

---

## §15. CI/CD et release engineering

### §15.1 Pipeline

```
push → CI (unit + integration) → build image → scan → sign
     → push to registry → canary deploy → metrics watch
     → promote or rollback → update SDK versions → release notes
```

### §15.2 Artifacts par release

- Docker image `efe/server:X.Y.Z` (signée).
- Helm chart `efe-server-X.Y.Z.tgz`.
- SDK Python `efe-tool==X.Y.Z` sur PyPI interne.
- SDK Go `github.com/your-org/efe-client-go@vX.Y.Z`.
- SDK TS `@your-org/efe-client@X.Y.Z` sur npm registry.
- SDK Java `com.your-org:efe-client:X.Y.Z` sur Maven.
- Release notes (Markdown) + changelog.

### §15.3 Feature flags

Toutes les features non-triviales passent derrière un flag géré par un système externe (LaunchDarkly, ConfigCat, ou un simple CM dans K8s).

### §15.4 Rollback

Automation :
- Dashboard detection burn-rate spike → trigger → rollout undo (< 5 min).
- Manual via `kubectl rollout undo deployment/efe-server`.
- Last-known-good image tag toujours dispo en local registry.

### §15.5 Coordination SDK

Un bump de proto (non-breaking) :
1. PR sur `efe/protos/mode1/v1.proto`.
2. CI régénère les 4 SDK (Python, Go, TS, Java).
3. Tests dans chaque SDK via docker-compose avec nouvelle image.
4. Bump mineur dans chaque SDK, publication parallèle.

Total : ~1 h automated.

---

## §16. Versioning protocolaire

### §16.1 Règles strictes

- **Additif** : champ `proto3` optionnel → mineur, non-breaking.
- **Renaming** : jamais.
- **Removal** : requiert V2.
- **Type change** : jamais.

### §16.2 Namespaces parallèles

```
efe.mode1.v1.Mode1Service         # stable, support continu
efe.mode1.v2.Mode1Service         # next, en dev
efe.mode1.v1alpha1.Mode1Service   # experimental
```

Un pod peut servir les 3 simultanément.

### §16.3 Déprécation

Annonce → warning → freeze → retrait sur 6 mois minimum.

### §16.4 Communication

Release notes, e-mail aux tenants, dashboard de deprecation tracking ("X % of traffic still on v1 after announcement").

---

## §17. Documentation et onboarding

### §17.1 Docs par langue

```
docs/
  getting-started/
    python.md        → quickstart avec efe-tool client
    go.md            → quickstart avec efe-client-go
    ts.md
    java.md
  concepts/
    catalogs.md
    verdicts.md
    tenancy.md
  operations/
    slo.md
    runbook.md
    incident-response.md
  versioning/
    proto-v1.md
    migration-v1-to-v2.md
```

### §17.2 Runbook on-call

Structure `docs/operations/runbook.md` :

- **Symptôme** : ce que le pager dit.
- **Hypothèses** : causes possibles, ordre de probabilité.
- **Diagnostique** : commandes `kubectl`, requêtes Prometheus, logs à inspecter.
- **Remédiation** : étapes par hypothèse.
- **Escalation** : qui appeler si bloquant.

### §17.3 Onboarding d'une nouvelle équipe consommatrice

Process standard :
1. L'équipe plateforme crée un JWT + tenant_id pour la nouvelle équipe.
2. L'équipe plateforme ajoute le tenant dans `tenants.yaml`.
3. L'équipe consommatrice clone le quickstart de son langage.
4. Smoke test contre staging → contre prod.
5. L'équipe plateforme monitor les premières heures.

Temps cible : 2 heures.

---

## §18. Dépendances d'infrastructure

### §18.1 Hard deps

- Kubernetes 1.28+
- cert-manager 1.13+
- Istio 1.20+ (ou Linkerd)
- Prometheus + Alertmanager
- Grafana
- Loki / ELK pour logs
- OpenTelemetry Collector
- Registry OCI (Harbor, ECR)
- Catalog store (S3 / GCS / PVC)

### §18.2 Soft deps

- Vault (secret management)
- LaunchDarkly (feature flags)
- PagerDuty (on-call rotation)
- Slack (notifications)
- GitHub / GitLab (source control + CI)

### §18.3 Coût estimé (mensuel, MVP)

| Composant | Coût approximatif |
|---|---|
| 3 × pods 1 CPU / 1 Gi (AWS m7i.large-ish) | ~$150 |
| Ingress Istio | $30 |
| Observability stack (modest) | $50 |
| Registry OCI storage | $5 |
| Catalog store S3 (10 GB) | $0.30 |
| **Total MVP** | **~$235 / mois** |

À 10 équipes consommatrices → $23 / équipe / mois. Ordre de grandeur d'un SaaS basique.

---

## §19. Plan d'implémentation

### §19.1 Sprint E-1 (2 j) — Dockerfile production

- Multi-stage build Rust.
- Image non-root.
- Health probes.
- Tests : l'image boot, répond `/healthz`.

### §19.2 Sprint E-2 (2 j) — Manifests K8s

- Deployment, Service, HPA, PDB, NetworkPolicy, ServiceMonitor.
- Tests : `kind` cluster local.

### §19.3 Sprint E-3 (2 j) — mTLS via cert-manager

- Intégration cert-manager.
- Certificat + Issuer + Certificate resources.
- Tests : mTLS obligatoire, rejet sans cert.

### §19.4 Sprint E-4 (3 j) — JWT validation

- Crate `jsonwebtoken`.
- Middleware tonic qui valide signature, claims, scopes.
- Tests : JWT valide OK, expiré rejeté, scope insuffisant rejeté.

### §19.5 Sprint E-5 (3 j) — Rate limiting multi-tenant

- `tower-governor` avec keyer sur `tenant_id`.
- Config YAML monté en ConfigMap.
- Tests : 1000 RPS team-support OK, team-churn > 100 → RESOURCE_EXHAUSTED.

### §19.6 Sprint E-6 (3 j) — AdminService complet

- RegisterCatalog, ListCatalogs, GetCatalog, DeleteCatalog, ReloadCatalogs.
- Audit log.
- Tests : enregistrement + listing + suppression, audit écrit.

### §19.7 Sprint E-7 (2 j) — Catalog store externe

- S3 / PVC support via crate `object_store`.
- Poll every 60s + notify via Redis pub/sub.
- Tests : upload → pods reload.

### §19.8 Sprint E-8 (3 j) — Observability stack

- ServiceMonitor + PrometheusRule pour alertes SLO.
- 3 dashboards Grafana.
- OTLP exporter.
- Tests : dashboards rendus, alertes firing sur scenario `kill pod`.

### §19.9 Sprint E-9 (2 j) — SDK Go

- Génération stubs via `buf` ou `protoc`.
- Package `github.com/your-org/efe-client-go`.
- Quickstart + tests.

### §19.10 Sprint E-10 (2 j) — SDK TypeScript

- Idem, `@your-org/efe-client`.

### §19.11 Sprint E-11 (2 j) — SDK Java

- Idem, `com.your-org:efe-client`.

### §19.12 Sprint E-12 (2 j) — Release engineering

- Pipeline CI → canary → promote.
- Image signing cosign.
- SBOM avec syft.

### §19.13 Sprint E-13 (1.5 j) — Load tests k6

- Scripts pour RED baseline.
- Nightly run.

### §19.14 Sprint E-14 (1.5 j) — Runbook + SLO

- `SLO.md`, `runbook.md`.
- Game day #1.

### §19.15 Sprint E-15 (2 j) — Documentation

- Quickstart 4 langues.
- Architecture concepts.
- Migration D → E guide.

### §19.16 Total

**~33 jours-homme** pour atteindre Go sur E. C'est le sprint le plus lourd de la trajectoire ; justifié uniquement si ≥ 2 équipes consommatrices.

---

## §20. Matrice de risques

| ID | Risque | Proba | Impact | Mitigation |
|---|---|---|---|---|
| R-E1 | Cold start pod > 5 s viole SLO | Haute | Haut | `minReplicas: 3`, warmup init container |
| R-E2 | Fuite de JWT compromet tenant | Moyenne | Critique | Scope strict, rotation 30j, audit logs |
| R-E3 | Noisy neighbor consomme tout le pool | Certaine sans RL | Haut | Rate limiting par tenant obligatoire |
| R-E4 | Cert mTLS expire sans rotation | Faible si cert-manager | Critique | Alerting expiry < 48h |
| R-E5 | Fleet mémoire explose (embedder × N pods) | Certaine | Moyen | Documenté, budget cloud prévu |
| R-E6 | Proto change casse SDK sans warning | Faible si discipline | Haut | Gates CI sur breaking changes |
| R-E7 | Catalog corrompu propagé à tous les pods | Faible | Haut | Validation AdminService + canary reload |
| R-E8 | Load test oublie un scenario critique | Moyenne | Moyen | Revue quarterly, game day |
| R-E9 | Burn error budget rapide sans alerte | Moyenne | Haut | Multi-window alerts (5m + 1h) |
| R-E10 | K8s upgrade casse Istio | Moyenne | Haut | Staging env mirror + smoke avant prod |
| R-E11 | Dépendance cloud (S3) indispo | Faible | Moyen | Cache local de catalogs avec TTL |
| R-E12 | Équipe plateforme insuffisante (solo on-call) | Haute au départ | Haut | Rotation >= 3 personnes |
| R-E13 | SDK TS non maintenu | Moyenne | Moyen | Ownership clair, tests CI |
| R-E14 | Coût scalant imprévu | Moyenne | Moyen | Budget alert + HPA maxReplicas |

---

## §21. Critères Go / No-Go

### §21.1 Bloquants techniques

- **G-E1** Image Docker signée, multi-stage, non-root, CIS benchmark passing.
- **G-E2** Manifests K8s complets, déployables par `helm install`.
- **G-E3** mTLS entre clients et EFE fonctionnel, tested.
- **G-E4** JWT validation avec 3 scopes minimum.
- **G-E5** Rate limiting par tenant, vérifié en load test.
- **G-E6** AdminService avec Register / List / Get / Delete / Reload + audit.
- **G-E7** Catalog store externe (S3 minimum) avec reload propagation.
- **G-E8** 3 dashboards Grafana + 5 règles Alertmanager.
- **G-E9** SDK Python + Go minimum publiés, tests e2e.
- **G-E10** Pipeline CI → canary → promote → rollback automation.
- **G-E11** Load test 500 QPS avec p95 < 2 s, error rate < 0.5 %.
- **G-E12** Runbook 10 scénarios minimum testé via game day.
- **G-E13** SLO publié, error budget dashboard en place.

### §21.2 Bloquants organisationnels

- **G-E14** Équipe on-call ≥ 3 personnes, rotation définie.
- **G-E15** ≥ 2 équipes consommatrices onboarded.
- **G-E16** Process Incident Response documenté et testé.
- **G-E17** Budget cloud mensuel approuvé pour 12 mois.
- **G-E18** RACI entre équipe plateforme et équipes produits clair.

### §21.3 Non-bloquants

- SDK Java, TypeScript (peuvent arriver par paliers).
- Multi-région.
- Feature flags avancés.
- Cache Redis partagé.
- Batching RPC streaming.

### §21.4 Procédure Go

1. CI verte 10 runs consécutifs incluant load test.
2. Game day #1 réussi.
3. 2 tenants pilotes onboarded en staging depuis ≥ 2 semaines sans incident.
4. Équipe on-call formée.
5. Annonce générale interne.

---

## §22. Annexes

### §22.A Proto complet `efe.mode1.v1`

Fichier `protos/efe/mode1/v1/service.proto` — voir §5.1 pour le contenu.

### §22.B Manifests K8s de référence

```yaml
# deploy/k8s/deployment.yaml
apiVersion: apps/v1
kind: Deployment
metadata: { name: efe-server, namespace: efe }
spec:
  replicas: 3
  strategy: { type: RollingUpdate, rollingUpdate: { maxSurge: 1, maxUnavailable: 0 } }
  selector: { matchLabels: { app: efe-server } }
  template:
    metadata:
      labels: { app: efe-server }
      annotations:
        prometheus.io/scrape: "true"
        prometheus.io/port: "9090"
    spec:
      serviceAccountName: efe-server
      terminationGracePeriodSeconds: 45
      containers:
      - name: efe-server
        image: efe/server:1.0.0
        imagePullPolicy: IfNotPresent
        ports:
        - { name: data,    containerPort: 50051, protocol: TCP }
        - { name: admin,   containerPort: 50052, protocol: TCP }
        - { name: health,  containerPort: 50053, protocol: TCP }
        - { name: metrics, containerPort: 9090,  protocol: TCP }
        env:
        - { name: EFE_BACKEND, value: mvp }
        - { name: EFE_CATALOG_STORE, value: s3://efe-catalogs/ }
        - { name: RUST_LOG, value: info }
        resources:
          requests: { cpu: 500m, memory: 768Mi }
          limits:   { cpu: 2,    memory: 1.5Gi }
        readinessProbe:
          exec: { command: [ "/bin/grpc_health_probe", "-addr=:50053" ] }
          initialDelaySeconds: 20
          periodSeconds: 5
        livenessProbe:
          exec: { command: [ "/bin/grpc_health_probe", "-addr=:50053" ] }
          initialDelaySeconds: 60
          periodSeconds: 15
          failureThreshold: 3
        securityContext:
          runAsNonRoot: true
          runAsUser: 1000
          readOnlyRootFilesystem: true
          allowPrivilegeEscalation: false
          capabilities: { drop: ["ALL"] }
          seccompProfile: { type: RuntimeDefault }
        volumeMounts:
        - { name: tmp,   mountPath: /tmp }
        - { name: certs, mountPath: /etc/certs, readOnly: true }
      volumes:
      - name: tmp
        emptyDir: { sizeLimit: 256Mi }
      - name: certs
        csi:
          driver: secrets-store.csi.k8s.io
          readOnly: true
          volumeAttributes: { secretProviderClass: efe-tls }
---
apiVersion: policy/v1
kind: PodDisruptionBudget
metadata: { name: efe-server, namespace: efe }
spec:
  minAvailable: 2
  selector: { matchLabels: { app: efe-server } }
---
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata: { name: efe-server, namespace: efe }
spec:
  scaleTargetRef: { apiVersion: apps/v1, kind: Deployment, name: efe-server }
  minReplicas: 3
  maxReplicas: 20
  metrics:
  - { type: Resource, resource: { name: cpu, target: { type: Utilization, averageUtilization: 70 } } }
---
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata: { name: efe-server, namespace: efe }
spec:
  podSelector: { matchLabels: { app: efe-server } }
  policyTypes: [Ingress, Egress]
  ingress:
  - from:
    - namespaceSelector: { matchLabels: { name: istio-system } }
    ports:
    - { protocol: TCP, port: 50051 }
    - { protocol: TCP, port: 50053 }
  - from:
    - namespaceSelector: { matchLabels: { name: efe-admin } }
    ports:
    - { protocol: TCP, port: 50052 }
  - from:
    - namespaceSelector: { matchLabels: { name: monitoring } }
    ports:
    - { protocol: TCP, port: 9090 }
  egress:
  - to:
    - namespaceSelector: { matchLabels: { name: kube-system } }
    ports:
    - { protocol: UDP, port: 53 }
  - to:
    ports:
    - { protocol: TCP, port: 443 }     # S3 / registry
```

### §22.C Alertmanager rules

```yaml
groups:
- name: efe-slo
  rules:
  - alert: EfeErrorRateHigh
    expr: |
      sum(rate(efe_classify_errors_total[5m]))
      / sum(rate(efe_classify_requests_total[5m]))
      > 0.01
    for: 5m
    labels: { severity: critical, service: efe }
    annotations:
      summary: "EFE error rate > 1% for 5 minutes"
      runbook: "https://wiki/runbook/efe#error-rate"

  - alert: EfeLatencyP95High
    expr: |
      histogram_quantile(0.95,
        sum(rate(efe_classify_duration_seconds_bucket[5m])) by (le)
      ) > 2.0
    for: 10m
    labels: { severity: warning, service: efe }

  - alert: EfePodRestartLoop
    expr: rate(kube_pod_container_status_restarts_total{namespace="efe"}[10m]) > 0.3
    for: 5m
    labels: { severity: warning, service: efe }

  - alert: EfeTenantQuotaBreach
    expr: sum by (tenant) (rate(efe_classify_errors_total{grpc_code="RESOURCE_EXHAUSTED"}[5m])) > 10
    for: 5m
    labels: { severity: info, service: efe }
```

### §22.D Runbook template

Pour chaque alerte :

```markdown
## Alert: EfeErrorRateHigh

**Severity**: critical
**SLO**: availability

### Symptom
Error rate > 1% for 5 min.

### First checks (5 min)
1. `kubectl -n efe get pods -o wide` — pods restarting?
2. `kubectl -n efe logs -l app=efe-server --tail=200 | grep -i error`
3. Grafana dashboard "EFE Overview" → which tenant / catalog is failing?
4. Check catalog store: `aws s3 ls s3://efe-catalogs/`

### Hypothesis & remediation

H1: One bad catalog was deployed
  → `AdminService.ListCatalogs` → find last registered
  → Rollback via `AdminService.DeleteCatalog` + register previous

H2: Pod OOM
  → `kubectl top pods -n efe`
  → Scale up by 2; investigate memory leak

H3: Upstream (catalog store S3) down
  → `aws s3api head-bucket --bucket efe-catalogs`
  → Pods serve from in-memory cache; coordinate with cloud team

### Escalation
@efe-oncall in #efe-incidents (Slack) → on-call engineer
@efe-lead if not resolved in 30 min

### Postmortem
File postmortem within 48h if user-visible impact.
```

### §22.E ADR — décisions clés

- **ADR-E-001** — Topologie MVP = single-tenant logique avec routage par JWT. Raison : simplicité, moins d'infra, isolation via quotas suffisante au début.
- **ADR-E-002** — mTLS + JWT obligatoires ensemble. Raison : defense in depth.
- **ADR-E-003** — AdminService sur port séparé. Raison : blast radius réduit.
- **ADR-E-004** — Catalog store externe (S3/PVC). Raison : stateless pods, rolling deploys non-destructifs.
- **ADR-E-005** — Ports 50051 / 50052 / 50053 / 9090. Raison : séparation plans + standards.
- **ADR-E-006** — Rate limiting via `tower-governor`, clé = tenant_id. Raison : éprouvé, Rust-native.
- **ADR-E-007** — Proto additif uniquement. Raison : compat cross-SDK.
- **ADR-E-008** — Image signée cosign + SBOM syft. Raison : supply chain integrity.
- **ADR-E-009** — `minReplicas: 3` + PDB `minAvailable: 2`. Raison : zero-downtime rolling deploys.
- **ADR-E-010** — Canary 5 % → 25 % → 50 % → 100 %. Raison : détection précoce de régression.
- **ADR-E-011** — SLO publié en V1.5 d'abord, V1.1 vise 99.9 %. Raison : on promet ce qu'on peut défendre.
- **ADR-E-012** — Multi-tenancy logique au MVP, physique (topologie B) en post-MVP. Raison : évolution progressive selon demande.
- **ADR-E-013** — SDK Python + Go obligatoires pour Go. TS / Java nice-to-have. Raison : couverture 80 % des consommateurs typiques.
- **ADR-E-014** — Pas de batching streaming au MVP. Raison : complexifie ; report V1.2 si besoin.

### §22.F Glossaire

- **Data plane** — chemin des RPC Classify ; impact direct sur les utilisateurs finals.
- **Control plane** — chemin de configuration (AdminService) ; rarement exercé.
- **Tenant** — client logique identifié par JWT claim `tenant_id`, séparé par quota + metrics.
- **SLO** — Service Level Objective, valeur cible mesurable (availability, latency).
- **Error budget** — marge de non-respect du SLO tolérée sur une fenêtre (30j typique).
- **Burn rate** — vitesse à laquelle l'error budget est consommé ; multiples (1×, 14.4×) servent de déclencheurs.
- **Canary** — déploiement progressif sur un petit pourcentage de traffic avant rollout complet.
- **PDB** — PodDisruptionBudget, limite K8s sur le nombre de pods pouvant être drain simultanément.
- **HPA** — HorizontalPodAutoscaler, auto-scaling horizontal basé sur metrics.
- **mTLS** — Mutual TLS, authentification bidirectionnelle par certificats.
- **JWT** — JSON Web Token, porteur de claims signé.
- **OTLP** — OpenTelemetry Protocol, format d'export des spans/metrics/logs.
- **cert-manager** — opérateur K8s qui renouvelle automatiquement les certificats.
- **Istio** — service mesh fournissant TLS, load balancing, policies.
- **Cosign** — signature cryptographique d'images container (projet Sigstore).
- **SBOM** — Software Bill of Materials, inventaire des dépendances.
- **Runbook** — document décrivant les étapes de diagnostic/remédiation d'incidents.
- **Game day** — exercice chaos ingénierie contrôlé, trimestriel.

### §22.G Comparaison avec services AI/ML de référence

| Propriété | NVIDIA Triton | TorchServe | Kserve | **EFE server (E)** |
|---|---|---|---|---|
| Focus | inférence GPU/CPU | PyTorch models | K8s-native ML | Ephemeral swarm classifier |
| Protocole | HTTP/gRPC | HTTP | HTTP/gRPC | **gRPC + optional HTTP** |
| Multi-tenancy | basique | non | via Knative | **first-class** |
| Catalog abstraction | model repository | model archive | InferenceService | **anchor catalog (YAML)** |
| Hot-reload | oui | oui | oui | **oui via AdminService** |
| Auth | via ingress | via ingress | via ingress | **mTLS + JWT natif** |
| Observability | metrics natifs | metrics natifs | metrics natifs | **RED Prometheus + OTel** |
| Package size | gros (GPU deps) | moyen | variable | **petit (500 MB) — CPU uniquement** |
| SLO framework | non | non | non | **intégré (doc SLO + dashboards)** |

**Positionnement** : EFE server est **spécialisé, léger, opinioné**. Pour de la classification texte CPU avec catalogues versionnés, il est plus direct qu'un Triton ou Kserve. Pour de la GPU inference massive, ce n'est pas son terrain.

---

**Fin du document.**

*Ce document est la source de vérité pour la Lettre E — le palier le plus lourd et le plus complexe de la trajectoire A → E. Toute évolution passe par PR sur ce fichier avec revue d'un mainteneur plateforme + un SRE. Les ADR (§22.E) sont append-only. Chaque manifest, script de déploiement et runbook a sa propre version dans `deploy/`.*
