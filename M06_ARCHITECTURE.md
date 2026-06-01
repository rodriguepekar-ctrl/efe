# M06 — Model Registry & Serving
## Architecture complète, ultra-détaillée
### (cosign signé bout-à-bout + rollback garanti < 60 s)

---

**Document** : spécification d'architecture
**Module Python** : `efe_registry` (paquet) + `efe_registry.serving`, `efe_registry.router`, `efe_registry.signing`, `efe_registry.canary`, `efe_registry.rollback`
**Sprint ciblé** : Sprint 6 (semaines 11-12) du plan MVP
**Dépend de** : M03 (Training Orchestrator), M04 (Training Workers), M05 (Evaluation Gate)
**Consommé par** : M07 (Observability), M08 (Rollback/Safety), M10 (Compliance), M11 (API Gateway)
**Statut** : spec gelée pour implémentation — aucune ambiguïté subsistante
**SLO durs** : vérification signature pré-chargement < 800 ms p95 — rollback bout-à-bout < 60 s p99 — contrat API rétro-compatible 90 jours

---

## Table des matières

1. [Mission et positionnement](#1-mission-et-positionnement)
2. [Scope MVP — ce qui est IN et OUT](#2-scope-mvp--ce-qui-est-in-et-out)
3. [Théorie : immutabilité cryptographique + bascule atomique](#3-théorie--immutabilité-cryptographique--bascule-atomique)
4. [Vue d'ensemble architecturale — 8 plans](#4-vue-densemble-architecturale--8-plans)
5. [Cycle de vie d'un artefact modèle](#5-cycle-de-vie-dun-artefact-modèle)
6. [Modèle de données](#6-modèle-de-données)
7. [Plan Signing — cosign key-based + keyless](#7-plan-signing--cosign-key-based--keyless)
8. [Plan Serving — vLLM adapter + multi-LoRA](#8-plan-serving--vllm-adapter--multi-lora)
9. [Plan Routing — TenantRouter + Redis Lua atomique](#9-plan-routing--tenantrouter--redis-lua-atomique)
10. [Plan Déploiement — Shadow + Canary](#10-plan-déploiement--shadow--canary)
11. [Plan Rollback — le chronogramme 60 s](#11-plan-rollback--le-chronogramme-60-s)
12. [Plan Compatibilité — contract_hash](#12-plan-compatibilité--contract_hash)
13. [Plan Admission — Kubernetes policy-controller](#13-plan-admission--kubernetes-policy-controller)
14. [SBOM + SLSA provenance](#14-sbom--slsa-provenance)
15. [NATS topics et flows d'événements](#15-nats-topics-et-flows-dévénements)
16. [Interfaces publiques](#16-interfaces-publiques)
17. [Stratégie de tests](#17-stratégie-de-tests)
18. [Observabilité](#18-observabilité)
19. [Plan de sprint M06](#19-plan-de-sprint-m06)
20. [Critères d'acceptation](#20-critères-dacceptation)
21. [Risques et mitigations](#21-risques-et-mitigations)
22. [Annexes](#22-annexes)

---

## 1. Mission et positionnement

### 1.1 Mission fonctionnelle

M06 est le **magasin canonique des modèles promus** et le **runtime de service**
de la plateforme EFE. Il répond à quatre questions fondamentales :

1. **Quel artefact modèle est actuellement servi pour `(tenant, task, variant)` ?**
   → `TenantRouter.route(req) → Endpoint` en < 2 ms p99.
2. **Cet artefact est-il authentique et non-altéré ?**
   → Chaîne de signatures cosign vérifiée à 4 niveaux (CI, admission K8s,
   pré-chargement, runtime périodique).
3. **Comment déployer un challenger sans dégrader les utilisateurs ?**
   → Shadow tap + Canary geometric 1 % → 5 % → 25 % → 100 %, halt sur SLO breach.
4. **Comment revenir au champion précédent quand ça dérape ?**
   → Rollback garanti **< 60 s** p99, adossé à des instances *pre-warmed standby*
   et une table de routage Redis avec Lua CAS atomique.

En une phrase : **M06 est le garant de la promesse "zéro-régression" de
l'EFE au plan du serving** — tout changement observable par l'utilisateur
est signé, pilotable, et réversible en moins d'une minute.

### 1.2 Contraintes non-négociables

| Contrainte | Valeur | Preuve |
|---|---|---|
| Immutabilité artefacts | S3 Object Lock COMPLIANCE ≥ 7 j | impossible de muter via API AWS |
| Signature cosign | toute image OCI et tout blob modèle | vérif CI + admission + load + runtime |
| Transparency log | Rekor public entry obligatoire pour keyless | UUID Rekor stocké DB |
| Rollback latence | < 60 s p99 | histogram Prometheus + alerte si > 45 s |
| Multi-tenant | aucune route cross-tenant | test dédié + audit trimestriel |
| Compat API | un `contract_hash` vit ≥ 90 jours après retrait | sweeper + alerte anticipée |
| Pas de perte requête | drain gracieux 30 s max | counter `dropped_requests` SLO 0 |

### 1.3 Position dans le pipeline EFE

```
                                    ┌─────────────────┐
                                    │  M11 Gateway    │
                                    │  (HTTP / gRPC)  │
                                    └────────┬────────┘
                                             │  /v1/completions
                                             │  (tenant_id, task_id, …)
                                             ▼
          ┌──────────────────────────────────────────────────────────────┐
          │                      M06 TenantRouter                        │
          │  (stateless, réplicas N, Redis routing + NATS invalidation)  │
          └───┬──────────────┬──────────────┬──────────────┬─────────────┘
              │              │              │              │
         champion        candidate        shadow       previous_champion
         (stable)        (canary x %)     (0 % rép.)   (pre-warmed 0 %)
              │              │              │              │
              ▼              ▼              ▼              ▼
          ┌───────┐     ┌───────┐     ┌───────┐     ┌───────┐
          │ vLLM  │     │ vLLM  │     │ vLLM  │     │ vLLM  │
          │ pod   │     │ pod   │     │ pod   │     │ pod   │
          └───┬───┘     └───┬───┘     └───┬───┘     └───┬───┘
              │             │             │             │
              └─────────────┴─────┬───────┴─────────────┘
                                  ▼
              ┌────────────────────────────────────────────┐
              │  S3 Object Lock (artefacts immutables)     │
              │  + Postgres artifact_index (metadata signé)│
              │  + Rekor transparency log (keyless)        │
              └────────────────────────────────────────────┘

  Amont : M05.ChampionRegistry.promote() émet sur NATS ─┐
                                                        │
  Aval  : M08.KillSwitch / M05.rollback_daemon ────────►│  ArtifactPromoter
          appellent M06.Rollback gRPC                   │  / RollbackExecutor
                                                        ▼
                                                   M06 control plane
```

### 1.4 Relation avec M05 : contrat de handoff

M05 **décide** qu'un run devient champion. M06 **exécute** le basculement vers
le serving. Le handoff passe par :

- NATS topic `efe.gate.decisions.promote` publié par M05 au COMMIT DB.
- Payload : `{tenant_id, task_id, run_id, artifact_ref, content_hash, signature_key_id, decision_id}`.
- M06 souscrit, vérifie la signature d'abord sur l'évènement (HMAC du payload
  avec clé M05→M06 partagée) puis sur l'artefact (cosign).
- M06 répond par `efe.registry.artifact.accepted` ou `.rejected` avec cause.
- Idempotence : `decision_id` est la clé de dédoublonnage (inséré dans
  `artifact_promotions` avec UNIQUE).

### 1.5 Relation avec M08 : rollback via gRPC

M08 est le **détenteur humain** du kill-switch et l'exécuteur du circuit-breaker
automatique. Quand M08 décide qu'il faut revenir en arrière, il appelle :

```proto
service RegistryControl {
    rpc Rollback(RollbackRequest) returns (RollbackReply);
}

message RollbackRequest {
    string tenant_id = 1;
    string task_id = 2;
    string actor = 3;                // humain ou "circuit-breaker"
    string reason = 4;
    string trigger_kind = 5;         // manual | auto_slo | auto_regression
    google.protobuf.Timestamp requested_at = 6;
    bytes idempotency_key = 7;       // SHA-256 ; dédoublonnage
}

message RollbackReply {
    string rollback_id = 1;
    uint32 latency_ms = 2;
    string final_state = 3;          // completed | partial | failed
    bytes from_artifact_hash = 4;
    bytes to_artifact_hash = 5;
}
```

Le SLO `latency_ms < 60_000` est **scellé contractuellement** : toute
violation est un incident P1.

---

## 2. Scope MVP — ce qui est IN et OUT

### 2.1 IN au MVP (sprint 6, ~2 semaines)

- Registre Postgres `artifacts` + index (tenant, task, semver, content_hash).
- Stockage S3 multi-bucket (`efe-artifacts-<tenant>`, Object Lock COMPLIANCE 7 j).
- Signature cosign **key-based** (KMS AWS) pour tous les artefacts promus.
- Signature cosign **keyless** (Fulcio + Rekor) pour artefacts produits en CI.
- Admission Kubernetes via `policy-controller` de Sigstore — mode ENFORCE.
- Runtime vLLM 0.5+ (`vllm.AsyncLLMEngine`) avec tensor-parallel 1/2/4/8.
- Multi-LoRA serving (1 base modèle, N adapter LoRA) — scaler les petits tenants.
- TenantRouter : Redis Cluster avec Lua CAS atomique + broadcast NATS.
- Quota enforcer : token-bucket Redis par `(tenant, api_key)`.
- Shadow Tapper : fork lecture, fire-and-forget, diff scoré offline.
- CanaryController : schedule `geometric` (1 %, 5 %, 25 %, 100 %) + halt SLO.
- RollbackExecutor : bascule atomique sur Redis, drain 30 s, < 60 s bout-à-bout.
- Pre-warmed standby : 1 pod dédié par `(tenant, task)` actif, 0 % trafic.
- Contract hash : SHA-256 sur `(input_schema, output_schema)` canonicalisés.
- SBOM CycloneDX 1.5 généré à la publication et attesté par cosign.
- SLSA provenance L1 minimum (L3 visé v1.1) via `cosign attest --predicate`.
- SDK Python client léger (`efe_registry.client.Client`) pour ops/debug.

### 2.2 OUT au MVP

| Fonctionnalité | Raison | Jalon |
|---|---|---|
| Multi-région failover actif-actif | coût + complexité | v1.2 |
| Compilation graph-level (TensorRT-LLM) | stabilité vLLM suffisante | v1.1 |
| Speculative decoding généralisé | gains moyens vs complexité | v1.1 |
| A/B testing orienté métrique business | hors périmètre EFE core | v1.1 |
| Déploiement serverless (cold-start) | incompatible SLO rollback 60 s | non planifié |
| SLSA L3 (hermetic builds) | dépend de M12 mature | v1.1 |
| Cross-cloud artefacts portable | S3 suffisant MVP | v1.2 |
| Confidential computing (TEE) | hors-scope MVP | v2.0 |

### 2.3 Roadmap post-MVP

- **v1.1 (T+3 mois)** : SLSA L3, TensorRT-LLM, multi-région actif-passif,
  évaluation shadow auto-pondérée.
- **v1.2 (T+6 mois)** : multi-cloud (GCP, Azure), compression adapter (QAT).
- **v2.0 (T+9 mois)** : TEE/confidential computing pour artefacts sensibles.

---

## 3. Théorie : immutabilité cryptographique + bascule atomique

### 3.1 Pourquoi deux propriétés distinctes

Un Model Registry doit résoudre **deux problèmes orthogonaux** :

1. **Problème de confiance** : "ce que je charge correspond-il à ce qui a été
   validé par le Gate ?" → résolu par **immutabilité cryptographique** :
   content-hash SHA-256 + signatures cosign multi-parties + transparency log.

2. **Problème de disponibilité** : "puis-je basculer d'un modèle à un autre
   sans dégrader les utilisateurs en cours ?" → résolu par **bascule atomique**
   sur une table de routage dont toutes les instances du router sont cohérentes
   en < 1 s, avec instances cibles déjà pré-chauffées.

Les deux sont **nécessaires ensemble**. Une signature vérifiée qui autorise à
charger un modèle ne règle pas la question du basculement. Un basculement
atomique vers un artefact non-vérifié est un désastre.

### 3.2 Invariants fondamentaux

**I1 — Content-hash stable**
> Pour tout artefact `A`, `content_hash(A) = SHA-256(canonical_bytes(A))` où
> `canonical_bytes` tri les fichiers par nom et concatène avec séparateur
> `\x00`. Deux répertoires identiques produisent le même hash.

**I2 — Signature vérifiable indépendamment**
> Pour tout artefact signé, un tiers avec la clé publique (ou la CA Fulcio)
> peut vérifier la signature sans accès au système EFE.

**I3 — Routage cohérent**
> Pour tout `(tenant_id, task_id, variant)`, tous les routeurs convergent vers
> la même décision de routage en ≤ 1 s après un changement, **sans** avoir à
> redémarrer.

**I4 — Drain gracieux**
> Aucune requête acceptée n'est perdue lors d'un basculement. Soit elle est
> servie par l'endpoint d'origine (in-flight), soit elle est re-routée vers
> le nouveau.

**I5 — Auditabilité bout-à-bout**
> Toute publication, promotion, canary step, rollback est un évènement dans la
> chaîne d'audit M05 (via NATS). Rien ne se passe dans M06 sans laisser de
> trace signée.

**I6 — Préwarm coûteux mais honnête**
> Le coût du pre-warmed standby est explicite et documenté : `N_actifs × 1
> GPU`. Pas de cold-start caché qui tromperait le SLO.

### 3.3 Modèle de cohérence

Le routage est **éventuellement cohérent** dans Redis Cluster (via PUBLISH /
SUBSCRIBE) mais **strictement cohérent localement** : chaque routeur lit
exclusivement sa copie locale, invalidée sur notification.

- Fenêtre d'incohérence post-écriture : < 500 ms en p99 (latence PubSub + TTL
  cache).
- Lecture locale : 0 round-trip, uniquement `dict` in-process.
- CAS en écriture : garantit qu'aucun update concurrent ne fait perdre une
  décision précédente (protection contre *lost update*).

Formellement, on a **eventual consistency + monotonic read** : une fois qu'un
routeur a observé une version `v`, il ne retournera jamais une version < `v`.

### 3.4 Budget temporel d'un rollback

Le SLO 60 s p99 n'est tenable qu'avec un budget temporel strict :

| Phase | Budget | Cumul | Actions |
|---|---|---|---|
| T0 | 0 s | 0 s | trigger reçu, `rollback_events.state=requested` |
| Lookup | ≤ 0.5 s | 0.5 s | lecture `champion_history` pour N-1 |
| Prepare | ≤ 1 s | 1.5 s | vérif standby ready, préparation Lua args |
| Broadcast | ≤ 0.5 s | 2 s | PUBLISH `routing.updated` — signal à tous routeurs |
| Swap atomic | ≤ 1 s | 3 s | Lua MULTI/EXEC : SET + HSET + PUBLISH |
| Cache invalidate | ≤ 1 s | 4 s | routeurs mettent à jour leur cache local |
| Verify | ≤ 1 s | 5 s | `efe_router_inflight{artifact=new}` > 99 % |
| Drain current | ≤ 30 s | 35 s | sync timeout des in-flight sur ancien pod |
| Safety margin | ≤ 25 s | 60 s | marge pour incidents (réseau, GC, …) |

Hors budget → P1 + post-mortem obligatoire.

---

## 4. Vue d'ensemble architecturale — 8 plans

Le module se lit comme un empilement de **8 plans orthogonaux** :

### 4.1 Storage Plane (persistance)

- S3 : buckets versionnés + Object Lock COMPLIANCE + tagging par tenant.
- Postgres : `artifacts`, `artifact_signatures`, `sboms`, `slsa_provenance`,
  `routing_table`, `endpoints`, `canary_runs`, `shadow_taps`, `rollback_events`,
  `contracts`, `contract_usage`, `artifact_promotions`.
- Redis Cluster : routing hot-path + cache vérif signature (court TTL).

### 4.2 Signing Plane (cryptographie)

- `CosignKeyBased` : wrapper autour de la CLI cosign + KMS AWS (ou Vault).
- `CosignKeyless` : Fulcio + Rekor + OIDC (GitHub Actions au MVP).
- `SignatureVerifier` : 4 niveaux de vérification (voir §7.3).
- `SBOMAttester`, `SLSAAttester` : attestations auxiliaires.

### 4.3 Serving Plane (runtime)

- `VllmAdapter` : wrap `AsyncLLMEngine`, expose `/v1/completions` OpenAI-compat.
- `MultiLoraServer` : base modèle + jusqu'à 16 adapters LoRA actifs.
- `ModelLoader` : mmap depuis disque local (après téléchargement S3 vérifié).
- `HealthCheck` : readiness probe qui bloque tant que le modèle ne répond pas.

### 4.4 Routing Plane (dispatch)

- `TenantRouter` : résout `(tenant, task, variant) → endpoint` en p99 < 2 ms.
- `RoutingTable` : Redis + cache in-memory + invalidation NATS.
- `QuotaEnforcer` : token-bucket Redis atomique (Lua).
- `WeightedDispatcher` : distribue selon `weight_percent` par endpoint.

### 4.5 Deployment Controller Plane (progression)

- `ShadowTapper` : fork lecture en parallèle, jette réponse, scoré offline.
- `CanaryController` : pilote la progression selon schedule + SLO.
- `SloGateway` : interface vers M07 (`LiveMetricsProvider`-like pour SLO).

### 4.6 Rollback Plane (réversibilité)

- `RollbackExecutor` : orchestration bout-à-bout sous SLO 60 s.
- `PreWarmedStandby` : maintient 1 pod "standby warm" prêt à recevoir trafic.
- `AtomicSwap` : Lua script CAS sur routing table.

### 4.7 Compatibility Plane (contrats)

- `ContractRegistry` : catalogue des `contract_hash` actifs et dépréciés.
- `SchemaCanonicalizer` : normalise JSON Schema (tri clés, repr stable).
- `DeprecationSweeper` : garantit 90 j de survie post-retrait + alertes.

### 4.8 Verification Plane (admission)

- `AdmissionWebhook` : intercepte pods K8s, refuse images non-signées.
- `PolicyManager` : CRDs Sigstore `ClusterImagePolicy` pilotées par Helm.
- `RuntimeVerifier` : re-vérifie signatures toutes les heures sur les pods live.

---

## 5. Cycle de vie d'un artefact modèle

### 5.1 Machine à états (9 états, 13 transitions)

```
       ┌───────┐
       │ DRAFT │  (créé par worker M04, pas encore uploadé)
       └───┬───┘
           │ upload complete
           ▼
     ┌──────────┐      verify_sig_fail      ┌──────────┐
     │ UPLOADED ├──────────────────────────▶│ QUARANTINED │
     └────┬─────┘                           └──────────┘
          │ sign+sbom+slsa
          ▼
       ┌────────┐       admitted by K8s        ┌────────┐
       │ SIGNED ├──────────────────────────────▶│ ADMITTED │
       └────┬───┘                              └────┬─────┘
            │                                       │ promoted by M05
            │                                       ▼
            │                                  ┌─────────┐
            │                                  │ SERVING │◀────┐
            │                                  └────┬────┘      │
            │                                       │  canary   │  rollback
            │                                       ▼  done     │  triggered
            │                                  ┌─────────┐      │
            │                                  │ CHAMPION├──────┘
            │                                  └────┬────┘
            │                                       │ superseded
            │                                       ▼
            │                                  ┌──────────┐
            │                                  │ DEPRECATED│
            │                                  └────┬─────┘
            │                                       │ retention expires
            │                                       ▼
            │                                  ┌───────┐
            │                                  │ PURGED │ (S3 Object Lock expiration)
            │                                  └───────┘
            │
            │  rejected (never promoted)
            ▼
     ┌──────────┐
     │ REJECTED │
     └──────────┘
```

Transitions autorisées (whitelist stricte) :

| From | To | Garde | Acteur |
|---|---|---|---|
| DRAFT | UPLOADED | checksum upload OK | M04 worker |
| UPLOADED | SIGNED | cosign sign + SBOM attesté | CI ou ops |
| UPLOADED | QUARANTINED | signature invalide ou vérif échouée | SignatureVerifier |
| SIGNED | ADMITTED | policy-controller K8s admission OK | admission webhook |
| SIGNED | REJECTED | M05 a rejeté le challenger | M05 ChampionRegistry |
| ADMITTED | SERVING | pod ready + health-check 3× | CanaryController |
| SERVING | CHAMPION | canary atteint 100 % sans breach | CanaryController |
| CHAMPION | SERVING | rollback déclenché (redevient un des pods servants) | RollbackExecutor |
| CHAMPION | DEPRECATED | nouveau champion promu | ArtifactPromoter |
| SERVING | DEPRECATED | canary aborted / rollback définitif | RollbackExecutor |
| DEPRECATED | PURGED | retention passée (7 j minimum) | RetentionSweeper |
| QUARANTINED | DEPRECATED | décision humaine | Ops |
| REJECTED | DEPRECATED | TTL 30 j écoulé | RetentionSweeper |

Interdit : `PURGED → *` (terminal), `CHAMPION → CHAMPION` (au plus un actif
par `(tenant, task)`), `DRAFT → SIGNED` direct (doit passer par UPLOADED).

### 5.2 Transitions et effets de bord

Chaque transition est :

- **Atomique en DB** (UPDATE … WHERE state=<current> RETURNING *), refus si
  état inattendu (CAS).
- **Auditée** : publie sur NATS `efe.registry.artifact.<new_state>` avec payload
  signé HMAC (clé partagée chaîne audit M05).
- **Journalisée** : ligne ajoutée dans `artifact_state_transitions` avec
  `from_state, to_state, actor, reason, ts, old_version, new_version`.

---

## 6. Modèle de données

### 6.1 Tables Postgres (DDL complet)

```sql
-- ============================================================
-- Artefacts (source de vérité content-hash)
-- ============================================================
CREATE TABLE artifacts (
    artifact_id       UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id         TEXT NOT NULL,
    task_id           TEXT NOT NULL,
    semver            TEXT NOT NULL,           -- ex. 1.4.2+acme
    content_hash      BYTEA NOT NULL,          -- SHA-256 sur canonical_bytes
    oci_ref           TEXT NOT NULL,           -- efe-registry.io/models/<t>/<ta>:<s>@sha256:<h>
    s3_url            TEXT NOT NULL,           -- s3://efe-artifacts-<t>/...
    parent_run_id     UUID NOT NULL,           -- M03 run_id
    parent_decision_id UUID,                   -- M05 decision_id (NULL si challenger non-promu)
    size_bytes        BIGINT NOT NULL CHECK (size_bytes > 0),
    framework         TEXT NOT NULL CHECK (framework IN ('vllm','peft','full')),
    state             TEXT NOT NULL CHECK (state IN (
                           'draft','uploaded','signed','admitted',
                           'serving','champion','deprecated','purged',
                           'quarantined','rejected')),
    state_version     BIGINT NOT NULL DEFAULT 0,   -- CAS
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    uploaded_at       TIMESTAMPTZ,
    signed_at         TIMESTAMPTZ,
    admitted_at       TIMESTAMPTZ,
    served_at         TIMESTAMPTZ,
    promoted_at       TIMESTAMPTZ,
    deprecated_at     TIMESTAMPTZ,
    purged_at         TIMESTAMPTZ,
    UNIQUE (tenant_id, task_id, semver),
    UNIQUE (content_hash)
);

CREATE INDEX ix_artifacts_tenant_task_state
    ON artifacts (tenant_id, task_id, state);

CREATE INDEX ix_artifacts_state_promoted
    ON artifacts (state, promoted_at DESC)
    WHERE state IN ('serving', 'champion');

-- ============================================================
-- Signatures cosign (peut en avoir plusieurs par artefact :
-- key-based + keyless + attestation)
-- ============================================================
CREATE TABLE artifact_signatures (
    signature_id      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    artifact_id       UUID NOT NULL REFERENCES artifacts(artifact_id) ON DELETE CASCADE,
    signing_method    TEXT NOT NULL CHECK (signing_method IN
                          ('key_kms', 'keyless_oidc', 'attestation')),
    key_id            TEXT,                      -- pour key_kms
    fulcio_cert_pem   TEXT,                      -- pour keyless
    rekor_entry_uuid  TEXT,                      -- Rekor transparency log
    rekor_inclusion_proof_json JSONB,
    signature_b64     TEXT NOT NULL,             -- base64 DSSE envelope
    payload_sha256    BYTEA NOT NULL,            -- hash du payload signé
    signing_identity  TEXT,                      -- ex. https://github.com/efe/efe/.github/workflows/release.yml@refs/heads/main
    signing_issuer    TEXT,                      -- ex. https://token.actions.githubusercontent.com
    verified_at       TIMESTAMPTZ,
    verified_ok       BOOLEAN NOT NULL DEFAULT false,
    verification_details JSONB,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX ix_sig_artifact ON artifact_signatures (artifact_id);
CREATE UNIQUE INDEX uq_sig_rekor_uuid
    ON artifact_signatures (rekor_entry_uuid)
    WHERE rekor_entry_uuid IS NOT NULL;

-- ============================================================
-- SBOM (CycloneDX 1.5 JSON, attesté cosign)
-- ============================================================
CREATE TABLE sboms (
    sbom_id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    artifact_id       UUID NOT NULL REFERENCES artifacts(artifact_id) ON DELETE CASCADE,
    format            TEXT NOT NULL DEFAULT 'cyclonedx-1.5',
    content_jsonb     JSONB NOT NULL,
    content_sha256    BYTEA NOT NULL,
    attestation_rekor_uuid TEXT,                 -- cosign attest sign signature
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (artifact_id, format)
);

-- ============================================================
-- SLSA provenance (in-toto v1.0)
-- ============================================================
CREATE TABLE slsa_provenance (
    provenance_id     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    artifact_id       UUID NOT NULL REFERENCES artifacts(artifact_id) ON DELETE CASCADE,
    slsa_level        SMALLINT NOT NULL CHECK (slsa_level BETWEEN 1 AND 4),
    builder_id        TEXT NOT NULL,             -- ex. https://github.com/efe/builder@v1
    build_type        TEXT NOT NULL,             -- ex. https://efe.io/Build/v1
    invocation_jsonb  JSONB NOT NULL,
    metadata_jsonb    JSONB NOT NULL,
    materials_jsonb   JSONB NOT NULL,
    dsse_envelope_b64 TEXT NOT NULL,
    rekor_entry_uuid  TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ============================================================
-- Endpoints (pods vLLM vivants servant un artefact)
-- ============================================================
CREATE TABLE endpoints (
    endpoint_id       UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    pool_id           TEXT NOT NULL,             -- gpu_a100_shared, gpu_h100_acme, ...
    pod_name          TEXT NOT NULL UNIQUE,
    namespace         TEXT NOT NULL,
    url               TEXT NOT NULL,             -- http://efe-pod-abc:8000
    artifact_id       UUID REFERENCES artifacts(artifact_id),
    role              TEXT NOT NULL CHECK (role IN (
                          'champion','candidate','shadow','standby_warm')),
    state             TEXT NOT NULL CHECK (state IN (
                          'pending','loading','ready','draining','drained','failed')),
    readiness_since   TIMESTAMPTZ,
    last_heartbeat_at TIMESTAMPTZ,
    tensor_parallel   SMALLINT NOT NULL CHECK (tensor_parallel IN (1,2,4,8)),
    lora_capacity     SMALLINT NOT NULL DEFAULT 0,  -- 0 = full-fine-tune
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX ix_endpoints_state ON endpoints (state);
CREATE INDEX ix_endpoints_artifact ON endpoints (artifact_id);

-- ============================================================
-- Routing table (copie durable ; Redis est la hot path)
-- ============================================================
CREATE TABLE routing_table (
    tenant_id         TEXT NOT NULL,
    task_id           TEXT NOT NULL,
    variant           TEXT NOT NULL DEFAULT 'default',
    endpoint_id       UUID NOT NULL REFERENCES endpoints(endpoint_id),
    weight_percent    SMALLINT NOT NULL CHECK (weight_percent BETWEEN 0 AND 100),
    priority          SMALLINT NOT NULL DEFAULT 0,
    version           BIGINT NOT NULL DEFAULT 0,
    role              TEXT NOT NULL CHECK (role IN (
                          'champion','candidate','shadow','standby_warm')),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by        TEXT NOT NULL,
    PRIMARY KEY (tenant_id, task_id, variant, endpoint_id)
);

CREATE INDEX ix_routing_tenant_task ON routing_table (tenant_id, task_id);

-- Invariant : Σ(weight_percent WHERE role IN ('champion','candidate'))
-- par (tenant, task, variant) = 100.  Vérifié par CHECK trigger.

-- ============================================================
-- Canary runs (un actif par tenant+task)
-- ============================================================
CREATE TABLE canary_runs (
    canary_id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id         TEXT NOT NULL,
    task_id           TEXT NOT NULL,
    candidate_artifact UUID NOT NULL REFERENCES artifacts(artifact_id),
    baseline_artifact UUID NOT NULL REFERENCES artifacts(artifact_id),
    current_percent   SMALLINT NOT NULL DEFAULT 1
                         CHECK (current_percent BETWEEN 0 AND 100),
    target_percent    SMALLINT NOT NULL DEFAULT 100
                         CHECK (target_percent BETWEEN 0 AND 100),
    schedule_kind     TEXT NOT NULL DEFAULT 'geometric',   -- geometric | linear | manual
    step_duration_s   INTEGER NOT NULL DEFAULT 600,        -- 10 min
    state             TEXT NOT NULL CHECK (state IN (
                          'ramping','paused','completed','rolled_back','aborted')),
    started_at        TIMESTAMPTZ NOT NULL,
    last_step_at      TIMESTAMPTZ,
    ended_at          TIMESTAMPTZ,
    slo_breach_count  SMALLINT NOT NULL DEFAULT 0,
    auto_rollback     BOOLEAN NOT NULL DEFAULT true,
    auto_advance      BOOLEAN NOT NULL DEFAULT true
);

CREATE UNIQUE INDEX uq_canary_active
    ON canary_runs (tenant_id, task_id)
    WHERE state IN ('ramping','paused');

-- ============================================================
-- Shadow taps
-- ============================================================
CREATE TABLE shadow_taps (
    shadow_id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id         TEXT NOT NULL,
    task_id           TEXT NOT NULL,
    candidate_artifact UUID NOT NULL REFERENCES artifacts(artifact_id),
    baseline_artifact UUID NOT NULL REFERENCES artifacts(artifact_id),
    tap_rate_percent  SMALLINT NOT NULL CHECK (tap_rate_percent BETWEEN 0 AND 100),
    started_at        TIMESTAMPTZ NOT NULL,
    ended_at          TIMESTAMPTZ,
    n_samples         BIGINT NOT NULL DEFAULT 0,
    cosine_mean       DOUBLE PRECISION,
    cosine_p05        DOUBLE PRECISION,
    latency_delta_ms_mean DOUBLE PRECISION
);

-- ============================================================
-- Rollback events (chronologie opérationnelle — SLO <60s)
-- ============================================================
CREATE TABLE rollback_events (
    rollback_id       UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id         TEXT NOT NULL,
    task_id           TEXT NOT NULL,
    from_artifact     UUID NOT NULL REFERENCES artifacts(artifact_id),
    to_artifact       UUID NOT NULL REFERENCES artifacts(artifact_id),
    trigger_kind      TEXT NOT NULL CHECK (trigger_kind IN (
                          'manual','auto_slo','auto_regression','admin')),
    actor             TEXT NOT NULL,
    reason            TEXT NOT NULL,
    idempotency_key   BYTEA NOT NULL,
    state             TEXT NOT NULL CHECK (state IN (
                          'requested','in_progress','completed','failed')),
    requested_at      TIMESTAMPTZ NOT NULL,
    started_at        TIMESTAMPTZ,
    completed_at      TIMESTAMPTZ,
    latency_ms        INTEGER,
    phases_jsonb      JSONB NOT NULL DEFAULT '{}',  -- instrumentation par phase
    UNIQUE (idempotency_key)
);

CREATE INDEX ix_rollback_tenant_task ON rollback_events (tenant_id, task_id);

-- ============================================================
-- Contracts (schéma I/O de l'API publique du modèle)
-- ============================================================
CREATE TABLE contracts (
    contract_hash           BYTEA PRIMARY KEY,          -- SHA-256
    input_schema_canonical  JSONB NOT NULL,
    output_schema_canonical JSONB NOT NULL,
    registered_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    deprecated_at           TIMESTAMPTZ,
    sunset_at               TIMESTAMPTZ,                 -- retrait effectif (≥ deprecated_at + 90 j)
    description             TEXT
);

CREATE TABLE artifact_contracts (
    artifact_id       UUID NOT NULL REFERENCES artifacts(artifact_id) ON DELETE CASCADE,
    contract_hash     BYTEA NOT NULL REFERENCES contracts(contract_hash),
    PRIMARY KEY (artifact_id, contract_hash)
);

-- ============================================================
-- Idempotence des promotions reçues de M05
-- ============================================================
CREATE TABLE artifact_promotions (
    decision_id       UUID PRIMARY KEY,          -- M05.decision_id
    artifact_id       UUID NOT NULL REFERENCES artifacts(artifact_id),
    tenant_id         TEXT NOT NULL,
    task_id           TEXT NOT NULL,
    received_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    accepted          BOOLEAN NOT NULL,
    rejection_reason  TEXT
);

-- ============================================================
-- Journal des transitions d'état
-- ============================================================
CREATE TABLE artifact_state_transitions (
    transition_id     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    artifact_id       UUID NOT NULL REFERENCES artifacts(artifact_id) ON DELETE CASCADE,
    from_state        TEXT NOT NULL,
    to_state          TEXT NOT NULL,
    old_version       BIGINT NOT NULL,
    new_version       BIGINT NOT NULL,
    actor             TEXT NOT NULL,
    reason            TEXT,
    nats_msg_id       TEXT,
    ts                TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX ix_transitions_artifact_ts
    ON artifact_state_transitions (artifact_id, ts DESC);
```

### 6.2 Schéma Redis

```
# Routing hot path
routing:{tenant}:{task}:v                (STRING, version CAS counter)
routing:{tenant}:{task}                  (HASH)
    endpoint_id_1 -> weight_percent + ":" + role  (ex. "70:champion")
    endpoint_id_2 -> "30:candidate"

# Channel de notifications
channel routing.updated                   (PubSub, payload = "tenant:task")

# Cache de vérif signatures (court TTL 60s)
sigcache:{artifact_id}                   (STRING, b"ok"|b"fail:<reason>", EX 60)

# Quota (token-bucket)
quota:{tenant}:{api_key}                 (HASH)
    tokens   -> float
    last_ts  -> epoch_ms

# Leader election pour canary controller
lock:canary:{tenant}:{task}              (STRING, EX 30, NX)
```

### 6.3 Layout S3

```
s3://efe-artifacts-<tenant>/
  <task_id>/
    <semver>/
      model.safetensors           (weights — protégé Object Lock COMPLIANCE 7j)
      tokenizer.json
      tokenizer_config.json
      config.json
      generation_config.json
      manifest.json                (content-hash + liste fichiers)
      manifest.sig                 (cosign signature du manifest)
      sbom.cdx.json                (CycloneDX 1.5)
      sbom.cdx.json.sig            (cosign attestation)
      provenance.intoto.jsonl     (SLSA provenance in-toto)
      provenance.intoto.jsonl.sig  (cosign attestation)
```

Object Lock **COMPLIANCE** mode (pas GOVERNANCE) → même root IAM ne peut pas
supprimer avant expiration. Force majeure → écrire dans nouveau bucket et
basculer via DNS.

---

## 7. Plan Signing — cosign key-based + keyless

### 7.1 Les deux modes supportés

**Mode A — Key-based (par défaut pour artefacts promus)**

- Clé ECDSA P-256 hébergée dans AWS KMS (alias `alias/efe-signing-<env>`).
- Rotation annuelle planifiée (nouvelle clé créée 30 j avant, période de
  chevauchement pendant laquelle les deux clés sont valides).
- Signature référence la clé via son ARN : `awskms:///alias/efe-signing-prod`.

**Mode B — Keyless (pour artefacts produits en CI)**

- Identité fournie par GitHub OIDC (`id-token: write` dans workflow YAML).
- Certificat éphémère (< 10 min) délivré par Fulcio.
- Signature et certificat déposés dans Rekor (transparency log public).

### 7.2 Commandes réelles

```bash
# --------- Mode A : Key-based (production) ---------

# Sign image OCI + manifest
cosign sign \
  --key awskms:///alias/efe-signing-prod \
  --output-signature dist/model.safetensors.sig \
  --output-certificate dist/model.safetensors.cert \
  efe-registry.io/models/acme/summarize:1.4.2@sha256:abc123...

# Sign attestation SBOM
cosign attest \
  --key awskms:///alias/efe-signing-prod \
  --predicate dist/sbom.cdx.json \
  --type cyclonedx \
  efe-registry.io/models/acme/summarize:1.4.2@sha256:abc123...

# Sign attestation SLSA
cosign attest \
  --key awskms:///alias/efe-signing-prod \
  --predicate dist/provenance.intoto.jsonl \
  --type slsaprovenance \
  efe-registry.io/models/acme/summarize:1.4.2@sha256:abc123...

# Verify (côté consommateur)
cosign verify \
  --key awskms:///alias/efe-signing-prod \
  --output-file verification.json \
  efe-registry.io/models/acme/summarize:1.4.2@sha256:abc123...

# --------- Mode B : Keyless (CI builds intermédiaires) ---------

cosign sign \
  --identity-token "$GITHUB_ID_TOKEN" \
  efe-registry.io/models/acme/summarize:dev-abc123

cosign verify \
  --certificate-identity \
    "https://github.com/efe-project/efe/.github/workflows/build.yml@refs/heads/main" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
  efe-registry.io/models/acme/summarize:dev-abc123
```

### 7.3 Quatre niveaux de vérification

| Niveau | Quand | Qui | Bloque quoi |
|---|---|---|---|
| **L1** Pre-publish | après `cosign sign` en CI | CI worker | push vers registry |
| **L2** Admission | création pod K8s | `policy-controller` | pod scheduling |
| **L3** Pre-load | avant `mmap` dans vLLM | `VllmAdapter._verify_or_die()` | démarrage pod |
| **L4** Runtime | toutes les heures en tâche de fond | `RuntimeVerifier` | continue à servir |

L3 est crucial : même si l'image OCI (qui héberge uniquement le tokenizer, la
config, et un entrypoint) est signée et admise, les poids `model.safetensors`
sont téléchargés à partir de S3 au premier démarrage. Le pod doit re-vérifier
la signature du blob avant de l'ouvrir en mémoire.

### 7.4 Interface Python — `SignatureVerifier`

```python
# src/efe_registry/signing/verifier.py
from __future__ import annotations

import asyncio
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

from efe_registry.errors import SignatureError, SignatureExpired


@dataclass(frozen=True, slots=True)
class VerificationResult:
    ok: bool
    method: Literal["key_kms", "keyless_oidc", "attestation"]
    key_id: str | None
    rekor_uuid: str | None
    verified_at_ns: int
    details: dict[str, str]


class SignatureVerifier:
    """Wraps cosign CLI — cached, async, structured errors."""

    def __init__(
        self,
        *,
        kms_key_arn: str | None,
        keyless_cert_identity: str | None,
        keyless_oidc_issuer: str | None,
        rekor_url: str = "https://rekor.sigstore.dev",
        timeout_s: float = 15.0,
    ) -> None:
        self._kms = kms_key_arn
        self._ident = keyless_cert_identity
        self._issuer = keyless_oidc_issuer
        self._rekor = rekor_url
        self._timeout = timeout_s

    async def verify_image(
        self, oci_ref: str, *, require_attestations: list[str] | None = None,
    ) -> VerificationResult:
        if self._kms:
            return await self._verify_key_based(oci_ref, require_attestations)
        if self._ident and self._issuer:
            return await self._verify_keyless(oci_ref, require_attestations)
        raise SignatureError("no signing method configured")

    async def verify_blob(
        self, blob_path: Path, sig_path: Path, cert_path: Path | None = None,
    ) -> VerificationResult:
        """L3 — verify model.safetensors avant mmap."""
        cmd = ["cosign", "verify-blob"]
        if self._kms:
            cmd += ["--key", self._kms]
        else:
            cmd += ["--certificate", str(cert_path),
                     "--certificate-identity", self._ident,
                     "--certificate-oidc-issuer", self._issuer]
        cmd += ["--signature", str(sig_path), str(blob_path)]
        return await self._run(cmd, method="key_kms" if self._kms else "keyless_oidc")

    async def _verify_key_based(self, ref, require_attestations):
        cmd = ["cosign", "verify", "--key", self._kms, "--output", "json", ref]
        res = await self._run(cmd, method="key_kms")
        if require_attestations:
            for pred_type in require_attestations:
                att_cmd = ["cosign", "verify-attestation", "--key", self._kms,
                            "--type", pred_type, "--output", "json", ref]
                await self._run(att_cmd, method="attestation")
        return res

    async def _verify_keyless(self, ref, require_attestations):
        cmd = ["cosign", "verify",
               "--certificate-identity", self._ident,
               "--certificate-oidc-issuer", self._issuer,
               "--rekor-url", self._rekor,
               "--output", "json", ref]
        return await self._run(cmd, method="keyless_oidc")

    async def _run(self, cmd, *, method) -> VerificationResult:
        proc = await asyncio.wait_for(
            asyncio.create_subprocess_exec(
                *cmd, stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
            ),
            timeout=self._timeout,
        )
        stdout, stderr = await proc.communicate()
        if proc.returncode != 0:
            raise SignatureError(
                f"cosign failed: {stderr.decode(errors='replace')}",
            )
        return _parse_cosign_output(stdout, method=method)
```

### 7.5 Cache de vérification

Chaque vérif coûte ~300-700 ms (appel KMS + éventuellement Rekor). Pour ne pas
pénaliser les démarrages de pods :

- Cache Redis `sigcache:{artifact_id}` avec TTL 60 s (le code de démarrage
  pod s'exécute 1× puis ne refait pas la vérif L3 avant expiration).
- Runtime verifier (L4) utilise lui aussi le cache pour éviter la tempête de
  re-vérifs.
- Cache invalidé sur rotation de clé (patterns `DEL sigcache:*` dans le
  runbook KeyRotation).

### 7.6 Rotation de clé

Procédure :

1. J-30 : créer nouvelle clé KMS `efe-signing-prod-2027`.
2. J-30 → J-0 : double-sign (anciennes et nouvelle clé) tous les artefacts
   publiés.
3. J-0 : passer la nouvelle clé comme primaire dans Helm values.
4. J+7 : désactiver signature avec l'ancienne (mais laisser vérif active).
5. J+90 : passer l'ancienne clé en "archive" (verif historique seulement,
   jamais signer).
6. J+365 : suppression KMS (après export dans cold storage archival).

Côté vérification : le `KeyResolver` (hérité de M05) est utilisé pour mapper
`signing_key_id` → clé concrète ; un artefact signé par l'ancienne clé reste
vérifiable.

---

## 8. Plan Serving — vLLM adapter + multi-LoRA

### 8.1 Runtime vLLM

Choix : **vLLM 0.5+** pour serving inference. Raisons :

- `AsyncLLMEngine` supporte un throughput continu (continuous batching).
- Support natif de PagedAttention (économie KV-cache).
- Support natif de **Multi-LoRA** (16 adapters actifs en parallèle, O(1)
  switch entre eux par requête).
- API OpenAI-compatible (`/v1/completions`, `/v1/chat/completions`) → SDK
  existants fonctionnent.
- Supporte tensor-parallel 1 / 2 / 4 / 8 → unique exécutable pour toutes
  tailles de modèles du MVP.

### 8.2 Adapter Python

```python
# src/efe_registry/serving/vllm_adapter.py
from __future__ import annotations

import asyncio
import logging
from dataclasses import dataclass
from pathlib import Path
from typing import AsyncIterator

from vllm import AsyncEngineArgs, AsyncLLMEngine, SamplingParams
from vllm.lora.request import LoRARequest

from efe_registry.errors import ArtifactVerificationFailed
from efe_registry.models import Artifact
from efe_registry.signing.verifier import SignatureVerifier

_log = logging.getLogger(__name__)


@dataclass(slots=True)
class VllmRuntimeConfig:
    tensor_parallel_size: int
    max_model_len: int
    dtype: str = "bfloat16"
    gpu_memory_utilization: float = 0.9
    enable_lora: bool = False
    max_loras: int = 0
    max_lora_rank: int = 64


class VllmAdapter:
    """Wraps vLLM AsyncLLMEngine with signature verification and tenancy."""

    def __init__(
        self,
        artifact: Artifact,
        runtime: VllmRuntimeConfig,
        verifier: SignatureVerifier,
        local_root: Path,
    ) -> None:
        self._artifact = artifact
        self._runtime = runtime
        self._verifier = verifier
        self._root = local_root / str(artifact.artifact_id)
        self._engine: AsyncLLMEngine | None = None
        self._lora_registry: dict[str, LoRARequest] = {}

    async def load(self) -> None:
        """Download (if needed), verify, then load into vLLM."""
        await self._download_if_missing()
        await self._verify_blob_or_die()
        self._engine = AsyncLLMEngine.from_engine_args(
            AsyncEngineArgs(
                model=str(self._root),
                tensor_parallel_size=self._runtime.tensor_parallel_size,
                max_model_len=self._runtime.max_model_len,
                dtype=self._runtime.dtype,
                gpu_memory_utilization=self._runtime.gpu_memory_utilization,
                enable_lora=self._runtime.enable_lora,
                max_loras=self._runtime.max_loras,
                max_lora_rank=self._runtime.max_lora_rank,
            ),
        )

    async def _download_if_missing(self) -> None:
        if (self._root / "model.safetensors").exists():
            return
        self._root.mkdir(parents=True, exist_ok=True)
        # … mc / aws s3 cp vers self._root (pseudo)
        raise NotImplementedError("s3 download to local_root")

    async def _verify_blob_or_die(self) -> None:
        blob = self._root / "model.safetensors"
        sig = self._root / "model.safetensors.sig"
        cert = self._root / "model.safetensors.cert"
        cert = cert if cert.exists() else None
        result = await self._verifier.verify_blob(blob, sig, cert)
        if not result.ok:
            raise ArtifactVerificationFailed(
                f"blob verification failed for {self._artifact.artifact_id}: "
                f"{result.details}",
            )
        _log.info("artifact_blob_verified",
                  extra={"artifact_id": str(self._artifact.artifact_id),
                         "method": result.method,
                         "rekor": result.rekor_uuid})

    async def register_lora(self, lora_id: str, lora_path: Path) -> None:
        assert self._engine is not None
        req = LoRARequest(lora_name=lora_id, lora_int_id=hash(lora_id) & 0xFFFF,
                           lora_path=str(lora_path))
        self._lora_registry[lora_id] = req

    async def generate(
        self, prompt: str, sampling: SamplingParams,
        *, lora_id: str | None = None, request_id: str,
    ) -> AsyncIterator[str]:
        assert self._engine is not None
        lora = self._lora_registry.get(lora_id) if lora_id else None
        async for out in self._engine.generate(
            prompt=prompt, sampling_params=sampling,
            request_id=request_id, lora_request=lora,
        ):
            yield out.outputs[0].text

    async def close(self) -> None:
        if self._engine is not None:
            await self._engine.shutdown()
```

### 8.3 Multi-LoRA serving

Pour les tenants de petite taille (< 50 req/s) — plutôt que d'avoir un pod
dédié par tenant — on mutualise une **base frozen** (7B / 13B / 70B) avec N
adapters LoRA chargés en mémoire. vLLM permet de router chaque requête vers
un adapter par `lora_id` passé dans la requête.

Le routeur M06 ajoute le `lora_id` au champ `LoRARequest` lors du dispatch :

```
req: {tenant: acme, task: summarize, prompt: ...}
     → lookup routing: (acme, summarize) → endpoint E1 (base 7B) + lora_id "acme/sum/v3"
     → E1 génère avec l'adapter chargé
```

Gain : économie de facteur 5–20× sur GPU pour les petits tenants, sans
compromettre l'isolation logique (les adapters sont séparés en mémoire).

Limite : si un tenant pousse > 10 req/s soutenus, on le migre vers un pod
dédié pour éviter le head-of-line blocking entre tenants sur le même vLLM.

### 8.4 Ressources GPU par pool

| Pool | GPU | TP | Taille modèle max | Throughput indicatif |
|---|---|---|---|---|
| `gpu_a100_shared` | A100 40 GB | 1 | 7B bf16 | 800 tok/s |
| `gpu_a100_med` | 2× A100 80 GB | 2 | 13B bf16 | 1200 tok/s |
| `gpu_h100_large` | 4× H100 80 GB | 4 | 70B bf16 | 3500 tok/s |
| `gpu_h100_xl` | 8× H100 80 GB | 8 | 180B bf16 | 6000 tok/s |

Le choix du pool est **dicté par l'artefact** (config M04), pas par le tenant.
L'isolation tenant se fait logiquement (routing + quota + namespace) plutôt
que physiquement (sauf demande explicite premium).

---

## 9. Plan Routing — TenantRouter + Redis Lua atomique

### 9.1 Architecture du router

Le router est un **composant stateless** répliqué (3 pods min par cluster)
lisant uniquement Redis pour la hot path. Chaque requête suit :

```
req entrant
  │
  ▼
┌─────────────────────────────────────────┐
│  1. AuthN/AuthZ (OIDC / API key)         │  ← M11 déjà vérifié, on re-check
│  2. QuotaEnforcer.check(tenant, api_key) │  ← token-bucket Redis Lua
│  3. RoutingTable.resolve(tenant, task)   │  ← cache local 1 s + Redis
│  4. WeightedDispatcher.pick(entries)     │  ← tirage pondéré
│  5. forward(endpoint.url, req)           │  ← HTTP/2 keep-alive pool
└─────────────────────────────────────────┘
```

### 9.2 Lua atomic swap

```lua
-- KEYS[1] = routing:{tenant}:{task}:v
-- KEYS[2] = routing:{tenant}:{task}
-- ARGV[1] = expected_version
-- ARGV[2] = new_version
-- ARGV[3..N] = endpoint_id, "weight:role", ...   (pairs HSET)

local cur = tonumber(redis.call('GET', KEYS[1]) or '0')
if cur ~= tonumber(ARGV[1]) then
    return {err = 'version_mismatch', current = cur}
end

local new_ver = tonumber(ARGV[2])
if new_ver ~= cur + 1 then
    return {err = 'version_not_sequential'}
end

redis.call('DEL', KEYS[2])
for i = 3, #ARGV, 2 do
    redis.call('HSET', KEYS[2], ARGV[i], ARGV[i+1])
end

redis.call('SET', KEYS[1], ARGV[2])
redis.call('PUBLISH', 'routing.updated',
            cjson.encode({tenant_key = KEYS[2], version = new_ver}))
return 'ok'
```

Propriétés :

- **Atomique** (Redis exécute le script en single-thread).
- **CAS** via la version (pas de lost update).
- **PubSub** synchrone avec l'écriture (pas de fenêtre où Redis a la nouvelle
  valeur mais pas encore notifié).

Erreurs possibles : `version_mismatch` (retry côté caller après relecture),
`version_not_sequential` (bug applicatif — incident).

### 9.3 Cache local et invalidation

Chaque routeur garde en mémoire :

```python
class RoutingCache:
    _entries: dict[tuple[str, str, str], RoutingEntry]
    _versions: dict[tuple[str, str], int]
    _ttl_s: float = 1.0
    _last_fetch: dict[tuple[str, str], float]
```

Lecture :

1. Si `(now - _last_fetch) < _ttl_s` → retour cache.
2. Sinon : `HGETALL routing:{t}:{ta}` + `GET routing:{t}:{ta}:v` dans un
   pipeline ; refresh cache.

Invalidation : thread dédié abonné à `routing.updated` ; sur message, enlève
l'entrée concernée du cache (force prochaine lecture à fetch).

Fenêtre d'incohérence théorique :
- TTL expiré côté client qui n'a pas encore reçu la notification → au pire 1 s.
- Notification reçue → < 100 ms (latence PubSub + dispatcher thread).

### 9.4 Weighted dispatch

```python
class WeightedDispatcher:
    def pick(self, entries: list[RoutingEntry], *, seed: int) -> RoutingEntry:
        # Stratified : on ne fait pas de true random, on utilise un
        # compteur atomique modulo 100 pour garantir qu'un flux de 1000
        # requêtes donne bien ~1 % au canary 1 %.
        bucket = self._counter.increment() % 100
        acc = 0
        for e in entries:
            acc += e.weight_percent
            if bucket < acc:
                return e
        return entries[-1]  # sécurité arithmétique
```

Ce "stratified" est plus équitable qu'un random uniform pour les faibles % :
à 1 %, on est sûr d'avoir **exactement** 1 requête sur 100 qui atteint le
candidate, pas une variance poissonienne.

### 9.5 QuotaEnforcer Lua

```lua
-- KEYS[1] = quota:{tenant}:{api_key}
-- ARGV[1] = now_ms
-- ARGV[2] = capacity
-- ARGV[3] = refill_per_sec
-- ARGV[4] = cost

local tokens   = tonumber(redis.call('HGET', KEYS[1], 'tokens') or ARGV[2])
local last_ts  = tonumber(redis.call('HGET', KEYS[1], 'last_ts') or ARGV[1])

local elapsed = (tonumber(ARGV[1]) - last_ts) / 1000.0
local new_tokens = math.min(tonumber(ARGV[2]),
                             tokens + elapsed * tonumber(ARGV[3]))

if new_tokens < tonumber(ARGV[4]) then
    redis.call('HMSET', KEYS[1], 'tokens', new_tokens, 'last_ts', ARGV[1])
    redis.call('EXPIRE', KEYS[1], 3600)
    return 0  -- refuse
end

redis.call('HMSET', KEYS[1],
            'tokens', new_tokens - tonumber(ARGV[4]),
            'last_ts', ARGV[1])
redis.call('EXPIRE', KEYS[1], 3600)
return 1  -- accepte
```

p95 < 2 ms (mesuré en staging sur Redis Cluster 3-shards).

---

## 10. Plan Déploiement — Shadow + Canary

### 10.1 Shadow tap

**But** : évaluer un challenger en production sans exposer les utilisateurs.

**Mécanisme** :

- Le router forke, pour un échantillon `tap_rate_percent`, la requête :
  - Appel **principal** vers baseline → réponse retournée au client.
  - Appel **shadow** vers candidate → réponse jetée, score calculé.
- Les deux appels sont lancés en parallèle (`asyncio.gather`), le shadow a un
  timeout court (1.5 s) ; son échec n'affecte jamais l'utilisateur.

**Scoring** :

- `cosine_logits` : similarité cosine des top-k logits (k=50).
- `latency_delta_ms` : diff temps de réponse candidate - baseline.
- `output_bleu` : BLEU-4 sur le texte généré (si LLM text).

Stockés dans `shadow_taps` (agrégats) + Prometheus `efe_registry_shadow_*`.

**Usage** : un shadow de 30 min avec >10k samples est un pré-requis
recommandé avant de démarrer un canary. Le Gate M05 peut exiger un shadow
préalable via policy.

### 10.2 Canary controller

**Schedule `geometric`** (par défaut) :

| Step | % | Durée min au palier |
|---|---|---|
| 0 | 1 % | 10 min |
| 1 | 5 % | 10 min |
| 2 | 25 % | 20 min |
| 3 | 100 % | — |

Pour passer à l'étape suivante, **tous** les SLO doivent être verts pendant
au moins `step_duration_s / 2` minutes consécutives :

- Latence p95 candidate ≤ 1.5× baseline
- Error rate 5xx candidate ≤ 2× baseline + 0.5 %
- Qualité implicite (thumbs_down, retry rate) pas pire que baseline + Δ
- Aucune alerte critique active sur l'endpoint

Si SLO breach pendant `step_duration_s` :
- `auto_rollback=true` → déclenche RollbackExecutor avec `trigger=auto_slo`.
- `auto_rollback=false` → passe `state=paused`, alerte humaine, attend
  décision.

### 10.3 Contrôleur — implémentation

```python
# src/efe_registry/canary/controller.py
from __future__ import annotations

import asyncio
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone

from sqlalchemy.ext.asyncio import AsyncSession

from efe_registry.db.models import CanaryRun, RollbackEvent
from efe_registry.observability.slo import SloGateway
from efe_registry.rollback.executor import RollbackExecutor
from efe_registry.router.updater import RoutingUpdater

SCHEDULE_GEOMETRIC = [1, 5, 25, 100]


@dataclass(frozen=True, slots=True)
class Step:
    index: int
    percent: int
    min_duration_s: int


class CanaryController:
    def __init__(
        self, session: AsyncSession,
        *, slo: SloGateway, routing: RoutingUpdater,
        rollback: RollbackExecutor,
    ) -> None:
        self._s = session
        self._slo = slo
        self._routing = routing
        self._rollback = rollback

    async def tick(self, canary_id: str) -> None:
        run = await self._s.get(CanaryRun, canary_id)
        if run is None or run.state != "ramping":
            return

        schedule = _schedule_for(run.schedule_kind)
        current_idx = _find_step_index(schedule, run.current_percent)

        # 1. Check SLO
        breach = await self._slo.has_breached(
            tenant_id=run.tenant_id, task_id=run.task_id,
            candidate_artifact=run.candidate_artifact,
            baseline_artifact=run.baseline_artifact,
            window_seconds=60,
        )
        if breach:
            run.slo_breach_count += 1
            if run.slo_breach_count >= 3:
                await self._abort_or_rollback(run)
                return

        # 2. Check time elapsed at current step
        elapsed = (datetime.now(timezone.utc)
                    - (run.last_step_at or run.started_at)).total_seconds()
        step = schedule[current_idx]
        if elapsed < step.min_duration_s:
            return
        # 3. Advance
        if current_idx + 1 >= len(schedule):
            await self._complete(run)
            return
        next_step = schedule[current_idx + 1]
        await self._routing.set_canary_percent(
            tenant_id=run.tenant_id, task_id=run.task_id,
            candidate_artifact=run.candidate_artifact,
            baseline_artifact=run.baseline_artifact,
            percent=next_step.percent,
        )
        run.current_percent = next_step.percent
        run.last_step_at = datetime.now(timezone.utc)
        run.slo_breach_count = 0
        await self._s.flush()

    async def _complete(self, run: CanaryRun) -> None:
        run.state = "completed"
        run.ended_at = datetime.now(timezone.utc)
        await self._s.flush()

    async def _abort_or_rollback(self, run: CanaryRun) -> None:
        if run.auto_rollback:
            await self._rollback.execute(
                tenant_id=run.tenant_id, task_id=run.task_id,
                actor="canary-controller",
                reason=f"SLO breach x{run.slo_breach_count}",
                trigger_kind="auto_slo",
            )
            run.state = "rolled_back"
        else:
            run.state = "paused"
        run.ended_at = datetime.now(timezone.utc)
        await self._s.flush()
```

### 10.4 Leader election du canary controller

Plusieurs réplicas du control plane tournent. Pour éviter des ticks en double :

- Chaque tenant+task a un lock Redis : `lock:canary:{t}:{ta}` avec `SET NX EX 30`.
- Le leader tient le lock 30 s, re-acquiert à mi-chemin ; si crash, un autre
  prend le relais dans les 30 s.
- Les opérations idempotentes (SLO check, advance step) survivent même sans
  lock (au pire, un tick redondant).

---

## 11. Plan Rollback — le chronogramme 60 s

### 11.1 Principe du pre-warmed standby

**Problème à résoudre** : un rollback implique de faire monter en charge un
pod qui était idle. Si ce pod doit télécharger le modèle (5-30 GB), charger
les poids (20-60 s), puis chauffer son KV cache, on est à 2-5 minutes —
**impossible de tenir 60 s**.

**Solution** : toujours maintenir **1 instance supplémentaire "standby warm"**
par `(tenant, task)` actif :

- Pod `role=standby_warm` déployé avec replica=1.
- Image et poids téléchargés une seule fois au démarrage.
- Engine vLLM **instancié**, KV cache pré-alloué, chargé en VRAM.
- Reçoit **0 % du trafic** utilisateur.
- Reçoit **1 requête synthétique toutes les 60 s** pour maintenir le cache CUDA
  chaud (compilation JIT, memory allocator stable).
- Health-check K8s toutes les 10 s.
- Coût : 1 GPU supplémentaire par pool actif ; accepté au MVP.

Alternative économique (v1.1) : standby partagé entre plusieurs tenants sur
base d'un même modèle (multi-LoRA), mais plus complexe.

### 11.2 Chronogramme T0 → T0+60s

```
T0     ── Trigger reçu (gRPC Rollback / NATS rollback.requested)
       ── INSERT rollback_events (state='requested', idempotency_key=...)
       ── Réponse gRPC immédiate { rollback_id, accepted=true }

T0+0.1s── Lookup previous champion via champion_history M05 (ou CHAMPION→SERVING
          transition journal) → previous_artifact_id

T0+0.5s── SELECT * FROM endpoints
          WHERE artifact_id=previous_artifact_id AND role='standby_warm'
          → vérifier state=ready ET last_heartbeat_at < 15s

T0+1s  ── Préparer args Lua :
          - clés : routing:{t}:{ta}:v, routing:{t}:{ta}
          - expected_version, new_version
          - pairs : endpoint_id_standby_warm → "100:champion"
                    endpoint_id_old_champion → "0:draining"
                    endpoint_id_candidate     → (supprimé si encore là)

T0+1.5s── EVALSHA (script Lua atomic_swap)
          → renvoie 'ok' ou 'version_mismatch' (retry)

T0+2s  ── PUBLISH routing.updated consumé par tous les routeurs
          → cache invalidation en < 500 ms

T0+3s  ── Routeurs dirigent le nouveau trafic vers endpoint_standby_warm
          (désormais role=champion dans routing)

T0+3s  ── Envoi signal 'drain' aux endpoints old_champion :
          POST http://{pod}/admin/drain (graceful shutdown)
          → pod cesse d'accepter new req, termine in-flight

T0+10s ── UPDATE endpoints SET role='draining' WHERE artifact_id=<old>
          UPDATE artifacts SET state='serving' WHERE artifact_id=<new>
          UPDATE rollback_events SET state='in_progress',
                                     started_at=T0+10s

T0+15s ── Poll : efe_router_inflight{artifact=<new>} >= 99 %
          (vérification que le basculement trafic a pris)

T0+20s ── Poll : efe_router_inflight{artifact=<old>} drops vers 0

T0+30s ── Deadline drain des in-flight : si > 0 encore, log warning
          et force-kill pod SIGKILL (accepter qq 5xx sur queue)

T0+45s ── Nouvelle instance standby_warm créée (K8s Job) pour
          préparer prochain rollback potentiel
          (this doesn't bloque la complétion du current rollback)

T0+50s ── UPDATE rollback_events SET state='completed',
                                     completed_at=T0+50s,
                                     latency_ms=50_000
          NATS PUBLISH efe.registry.rollback.completed
          AuditChain (via M05) AuditKind.ROLLBACK avec payload signé

T0+60s ── Marge — si on est ici et pas à completed, P1
```

### 11.3 RollbackExecutor — implémentation

```python
# src/efe_registry/rollback/executor.py
from __future__ import annotations

import asyncio
import uuid
from dataclasses import dataclass
from datetime import datetime, timezone
from hashlib import sha256
from typing import Literal

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_registry.clock import Clock, SystemClock
from efe_registry.db.models import (
    Artifact, Endpoint, RollbackEvent, RoutingTableRow,
)
from efe_registry.errors import (
    NoStandbyAvailable, RollbackTimeout, RollbackVersionConflict,
)
from efe_registry.observability.audit import AuditChannel
from efe_registry.observability.metrics import rollback_latency_seconds
from efe_registry.router.lua import RoutingLuaRunner


@dataclass(frozen=True, slots=True)
class RollbackReceipt:
    rollback_id: str
    from_artifact_id: str
    to_artifact_id: str
    latency_ms: int
    final_state: Literal["completed", "partial", "failed"]


class RollbackExecutor:
    """SLO < 60 s p99. Phase instrumentation in rollback_events.phases_jsonb."""

    HARD_DEADLINE_S = 60.0
    DRAIN_DEADLINE_S = 30.0

    def __init__(
        self,
        session: AsyncSession,
        *,
        lua: RoutingLuaRunner,
        audit: AuditChannel,
        clock: Clock | None = None,
    ) -> None:
        self._s = session
        self._lua = lua
        self._audit = audit
        self._clock: Clock = clock or SystemClock()

    async def execute(
        self,
        *,
        tenant_id: str, task_id: str,
        actor: str, reason: str,
        trigger_kind: Literal[
            "manual", "auto_slo", "auto_regression", "admin",
        ],
        idempotency_key: bytes | None = None,
    ) -> RollbackReceipt:
        t0 = self._clock.now_monotonic_s()
        idem = idempotency_key or self._derive_idem(tenant_id, task_id, actor)

        # Idempotence : if we already completed, return same receipt
        existing = await self._s.execute(
            select(RollbackEvent).where(
                RollbackEvent.idempotency_key == idem,
            ),
        )
        prev = existing.scalar_one_or_none()
        if prev is not None and prev.state == "completed":
            return _receipt_from_row(prev)

        row = await self._insert_requested(
            tenant_id=tenant_id, task_id=task_id,
            actor=actor, reason=reason,
            trigger_kind=trigger_kind, idempotency_key=idem,
        )

        try:
            phases: dict[str, float] = {}
            # Phase 1 : lookup
            phases["lookup_s"] = await self._phase_lookup(row, tenant_id, task_id)
            # Phase 2 : standby ready check
            standby = await self._phase_check_standby(row)
            # Phase 3 : atomic swap
            phases["swap_s"] = await self._phase_swap(row, standby)
            # Phase 4 : drain old
            phases["drain_s"] = await self._phase_drain(row)
            # Phase 5 : verify traffic shifted
            phases["verify_s"] = await self._phase_verify(row, standby)
            # Phase 6 : audit + replace standby
            phases["audit_s"] = await self._phase_audit(
                row, tenant_id=tenant_id, task_id=task_id,
            )
            phases["total_s"] = self._clock.now_monotonic_s() - t0

            row.state = "completed"
            row.completed_at = self._clock.now()
            row.latency_ms = int(phases["total_s"] * 1000)
            row.phases_jsonb = phases
            await self._s.flush()
            rollback_latency_seconds.labels(
                tenant=tenant_id, task=task_id, trigger=trigger_kind,
            ).observe(phases["total_s"])
            return _receipt_from_row(row)

        except RollbackTimeout as exc:
            row.state = "failed"
            row.phases_jsonb = {"error": str(exc), **phases}
            await self._s.flush()
            raise
```

(Les phases sont détaillées dans §11.4.)

### 11.4 Phases détaillées

Chaque phase a un timeout strict et instrumente sa durée.

```python
    async def _phase_swap(self, row: RollbackEvent, standby: Endpoint) -> float:
        t = self._clock.now_monotonic_s()
        # Lookup current version
        routing_key = f"routing:{row.tenant_id}:{row.task_id}"
        ver_key = f"{routing_key}:v"
        try:
            current_version = await self._lua.get_version(ver_key)
            new_entries = await self._build_new_entries(
                tenant_id=row.tenant_id, task_id=row.task_id,
                new_champion=standby, old_champion_artifact=row.from_artifact,
            )
            await asyncio.wait_for(
                self._lua.atomic_swap(
                    ver_key=ver_key, routing_key=routing_key,
                    expected_version=current_version,
                    new_version=current_version + 1,
                    entries=new_entries,
                ),
                timeout=2.0,
            )
        except asyncio.TimeoutError:
            raise RollbackTimeout("atomic_swap phase >2s")
        return self._clock.now_monotonic_s() - t

    async def _phase_drain(self, row: RollbackEvent) -> float:
        t = self._clock.now_monotonic_s()
        endpoints = await self._find_endpoints_by_artifact(row.from_artifact)
        tasks = [self._drain_pod(ep) for ep in endpoints]
        try:
            await asyncio.wait_for(asyncio.gather(*tasks, return_exceptions=True),
                                    timeout=self.DRAIN_DEADLINE_S)
        except asyncio.TimeoutError:
            # Force kill remaining pods; accept small % 5xx on in-flight
            await self._force_kill(endpoints)
        return self._clock.now_monotonic_s() - t

    async def _phase_verify(
        self, row: RollbackEvent, standby: Endpoint,
    ) -> float:
        t = self._clock.now_monotonic_s()
        # Poll Prometheus (M07) for inflight metric
        deadline = t + 5.0
        while self._clock.now_monotonic_s() < deadline:
            share = await self._metrics.inflight_share(
                tenant=row.tenant_id, task=row.task_id,
                artifact_id=standby.artifact_id,
            )
            if share >= 0.99:
                return self._clock.now_monotonic_s() - t
            await asyncio.sleep(0.5)
        raise RollbackTimeout("traffic not shifted to new champion within 5s")
```

### 11.5 Garanties formelles

**G1 — Pas de perte de requête** : toute requête acceptée avant T0 est, soit
terminée par l'ancien pod (dans les 30 s), soit re-routée vers le standby
warm (si elle était en file). La métrique `efe_router_dropped_total` doit
rester à 0 pendant un rollback.

**G2 — SLO 60 s p99** : deadline hardé dans `_phase_drain` à 30 s ; chaque
phase bornée ; sum ≤ 45 s + marge. Si on dépasse, `RollbackTimeout` émis,
`state=failed`, alerte P1, post-mortem obligatoire.

**G3 — Idempotence** : `idempotency_key` SHA-256 sur `(tenant, task, actor,
reason, rounded_ts_min)`. Un rollback redéclenché dans la minute renvoie le
même `rollback_id`.

**G4 — Traçabilité** : `rollback_events.phases_jsonb` contient toutes les
mesures de chaque phase. Post-mortem rapide possible.

**G5 — Audit cryptographique** : un évènement `AuditKind.ROLLBACK` est ajouté
à la chaîne M05 par `AuditChannel.record()` en fin d'exécution, avec
`payload = {from, to, latency_ms, trigger, actor}` signé HMAC.

---

## 12. Plan Compatibilité — contract_hash

### 12.1 Définition

Un `contract_hash` est un SHA-256 sur le JSON canonique d'un couple
`(input_schema, output_schema)` où les schémas sont en JSON Schema Draft
2020-12 et la canonicalisation suit :

- Clés objets triées alphabétiquement (récursivement).
- Séparateurs `","` et `":"` (pas d'espaces).
- Floats en `repr()` Python stable (pas de `1.0` vs `1` ambigu).
- Pas de `null` dans tableaux (exclusion explicite).
- Pas de commentaires (`$comment` ignoré).
- Encoding UTF-8 sans BOM.

```python
def contract_hash(input_schema: dict, output_schema: dict) -> bytes:
    payload = json.dumps(
        {"in": input_schema, "out": output_schema},
        sort_keys=True, separators=(",", ":"),
        ensure_ascii=False,
    ).encode("utf-8")
    return hashlib.sha256(payload).digest()
```

### 12.2 Utilisation côté client

Les clients soucieux de compatibilité ajoutent un header :

```http
POST /v1/completions HTTP/1.1
Host: api.efe.io
X-EFE-Contract-Hash: sha256:<64hex>
Authorization: Bearer <token>
```

Le TenantRouter résout :

1. Si ce hash est actuellement servi → route vers endpoint matching.
2. Sinon → route vers champion courant (hash différent) et renvoie header
   de warning : `Warning: 299 efe.io "Requested contract_hash deprecated"`.
3. Si le hash n'a jamais existé → 400 `Invalid-Contract-Hash`.

### 12.3 Deprecation Sweeper

Après retrait d'un contract (passage à DEPRECATED), le Sweeper :

- Démarre un timer `sunset_at = deprecated_at + 90 j`.
- Émet une alerte à J-30, J-7, J-1 via `efe.registry.contract.deprecating`.
- À `sunset_at`, passe à `state=sunset` → requêtes avec ce hash renvoyées
  410 Gone avec corps descriptif.
- Les clients utilisateurs sont listés dans `contract_usage` (dernier 30 j).

### 12.4 Garantie à 90 jours

Une fois un contract promu au moins une fois, il reste résolvable (au moins
via `X-EFE-Contract-Hash`) pendant **90 jours à partir de sa dépréciation**,
même après que ses artefacts associés soient passés à DEPRECATED puis PURGED.

Mécanisme : le router garde une route "archive" vers un artefact de fallback
(le plus récent artefact ayant satisfait ce contract_hash) tant que
`sunset_at > now`. Cet artefact reste en S3 (Object Lock empêche purge)
jusqu'à ce que `sunset_at` + retention soit dépassé.

---

## 13. Plan Admission — Kubernetes policy-controller

### 13.1 Installation

On déploie le Sigstore `policy-controller` via Helm :

```yaml
# charts/efe/values.yaml
policyController:
  enabled: true
  webhook:
    name: efe-policy-webhook
    failurePolicy: Fail
    namespaceSelector:
      matchLabels:
        efe.io/policy: enforce
```

Les namespaces `efe-prod`, `efe-staging` ont le label `efe.io/policy: enforce`.
Les namespaces de dev peuvent opter pour `warn` plutôt qu'`enforce`.

### 13.2 Policy CRD

```yaml
apiVersion: policy.sigstore.dev/v1beta1
kind: ClusterImagePolicy
metadata:
  name: efe-model-artifacts
spec:
  images:
    - glob: "efe-registry.io/models/**"
  authorities:
    - name: efe-production-kms
      key:
        kms: awskms:///alias/efe-signing-prod
      ctlog:
        url: https://rekor.sigstore.dev
    - name: efe-ci-keyless
      keyless:
        identities:
          - issuer: "https://token.actions.githubusercontent.com"
            subject: "https://github.com/efe-project/efe/.github/workflows/release.yml@refs/heads/main"
  policy:
    fetchConfigFile: true
    # Require attestations (SBOM + SLSA)
    includeTypedSignatures: true
    mode: enforce
```

### 13.3 Cycle d'admission

```
kubectl apply -f pod.yaml   (pod référence image efe-registry.io/models/...)
     │
     ▼
Kubernetes API server
     │
     ▼ (via admission webhook)
policy-controller
     │
     ▼
1. Fetch image manifest depuis registry
2. Lookup signatures cosign associées à l'image
3. Verify contre les authorities (KMS et/ou keyless)
4. Si require attestations : verify SBOM + SLSA attestations
5. Decision :
    - PASS → pod scheduled
    - FAIL → pod rejected avec message clair
     │
     ▼
Pod scheduled OR rejected
```

### 13.4 Images en test (non-signées)

Pour les images de tests CI, on utilise un namespace dédié `efe-tests` sans
label `efe.io/policy: enforce`. Aucune image non-signée ne peut entrer dans
`efe-prod` ou `efe-staging`.

### 13.5 Emergency override (break-glass)

Dans un incident extrême, un SRE peut temporairement retirer le label
`efe.io/policy: enforce` d'un namespace, déployer, puis re-l'activer. Cette
opération :

- **N'est jamais silencieuse** : le webhook d'audit K8s enregistre.
- **Est auditée** dans la chaîne M05 (`AuditKind.EMERGENCY_OVERRIDE`).
- **Déclenche un incident P1 automatique** via runbook on-call.
- **Est révoquée automatiquement** après 30 minutes (CronJob K8s).

---

## 14. SBOM + SLSA provenance

### 14.1 SBOM CycloneDX 1.5

Généré au moment du build via `syft` :

```bash
syft dist/model.safetensors \
  -o cyclonedx-json=dist/sbom.cdx.json \
  --source-name "efe-model-acme-summarize" \
  --source-version "1.4.2"

cosign attest \
  --key awskms:///alias/efe-signing-prod \
  --predicate dist/sbom.cdx.json \
  --type cyclonedx \
  efe-registry.io/models/acme/summarize:1.4.2
```

Contenu typique :

```json
{
  "bomFormat": "CycloneDX",
  "specVersion": "1.5",
  "version": 1,
  "metadata": {
    "timestamp": "2026-04-20T10:00:00Z",
    "tools": [{ "name": "syft", "version": "1.18.0" }],
    "component": {
      "type": "machine-learning-model",
      "name": "efe-model-acme-summarize",
      "version": "1.4.2",
      "hashes": [
        { "alg": "SHA-256", "content": "abc123..." }
      ]
    }
  },
  "components": [
    { "type": "library", "name": "pytorch", "version": "2.3.0",
      "purl": "pkg:generic/pytorch@2.3.0", "licenses": [...] },
    { "type": "library", "name": "transformers", "version": "4.40.0", ... },
    { "type": "data", "name": "training-dataset", "version": "sha256:def456..." }
  ]
}
```

### 14.2 SLSA provenance (in-toto v1.0)

Généré par le builder M04 (Training Worker) :

```json
{
  "_type": "https://in-toto.io/Statement/v1",
  "predicateType": "https://slsa.dev/provenance/v1",
  "subject": [{
    "name": "efe-model-acme-summarize",
    "digest": { "sha256": "abc123..." }
  }],
  "predicate": {
    "buildDefinition": {
      "buildType": "https://efe.io/Build/v1",
      "externalParameters": {
        "run_id": "018e5f3a-...",
        "method": "lora",
        "base_model": "meta-llama/Llama-3-8B"
      },
      "internalParameters": {
        "gpu_pool": "gpu_h100_large",
        "checkpoint_steps": 2000
      },
      "resolvedDependencies": [
        { "name": "training-dataset",
          "digest": { "sha256": "def456..." },
          "uri": "s3://efe-golden/acme/summarize/v3/" }
      ]
    },
    "runDetails": {
      "builder": { "id": "https://github.com/efe/builder@v1" },
      "metadata": {
        "invocationId": "018e5f3a-...",
        "startedOn": "2026-04-19T14:00:00Z",
        "finishedOn": "2026-04-19T17:34:21Z"
      }
    }
  }
}
```

### 14.3 SLSA level ciblé

- **MVP** : SLSA **L1** (provenance présente, pas hermétique).
- **v1.1** : SLSA **L3** (hermetic builds avec Tekton + provenance non-forgeable).

L'objectif L3 nécessite un builder isolé (pas de réseau pendant la compilation,
dépendances épinglées par hash). Au MVP, on accepte L1 (juste la preuve
d'origine).

---

## 15. NATS topics et flows d'événements

### 15.1 Topics catalogue

| Topic | Direction | Producteur | Consommateur | Payload |
|---|---|---|---|---|
| `efe.gate.decisions.promote` | in | M05 | M06 | `{decision_id, artifact, tenant, task}` |
| `efe.gate.decisions.reject` | in | M05 | M06 | idem |
| `efe.worker.artifact.ready` | in | M04 | M06 | `{run_id, s3_url, content_hash, tenant}` |
| `efe.registry.artifact.uploaded` | out | M06 | M07 | `{artifact_id}` |
| `efe.registry.artifact.signed` | out | M06 | M07, M10 | idem + `{rekor_uuid}` |
| `efe.registry.artifact.accepted` | out | M06 | M05 | `{decision_id, artifact_id}` |
| `efe.registry.artifact.rejected` | out | M06 | M05 | idem + `{reason}` |
| `efe.registry.routing.updated` | out | M06 | M06 (routers) | `{tenant, task, version}` |
| `efe.registry.canary.advanced` | out | M06 | M07 | `{canary_id, old_percent, new_percent}` |
| `efe.registry.canary.halted` | out | M06 | M07, M08 | `{canary_id, reason}` |
| `efe.registry.rollback.triggered` | in | M08 | M06 | `{tenant, task, reason, actor}` |
| `efe.registry.rollback.completed` | out | M06 | M05, M07, M08 | `{rollback_id, latency_ms}` |
| `efe.registry.contract.deprecating` | out | M06 | M07, M11 | `{contract_hash, sunset_at}` |

### 15.2 Format message

Tous les messages suivent ce format, version-aware :

```json
{
  "schema_version": "1.0",
  "event_type": "efe.registry.artifact.signed",
  "event_id": "018e5f3a-2e8a-7f4a-9b21-...",
  "occurred_at": "2026-04-20T10:12:34.567Z",
  "tenant_id": "acme",
  "payload": { ... },
  "hmac": "b64(HMAC-SHA256(canonical(event) || key))"
}
```

HMAC authentifie l'émetteur ; tous les consommateurs le vérifient.

### 15.3 Flow "promotion complète"

```
M04 worker → NATS efe.worker.artifact.ready
                        │
                        ▼
                   M06 UploadHandler
                   1. Validate manifest
                   2. Insert artifacts (DRAFT → UPLOADED)
                   3. Trigger sign pipeline (CI or local cosign)
                        │
                        ▼
                   NATS efe.registry.artifact.signed
                        │
                        ▼
M05 observes ... time later
M05 emits NATS efe.gate.decisions.promote
                        │
                        ▼
                   M06 ArtifactPromoter
                   1. Idempotence check (artifact_promotions)
                   2. Verify signature (cached)
                   3. Ensure endpoint pool ready
                   4. Create CanaryRun (1%)
                   5. RoutingUpdater.set_canary_percent(1)
                        │
                        ▼
                   NATS efe.registry.canary.advanced(0→1%)
                        │
                        ▼
                   CanaryController.tick() loop (every 30s)
                   → SLO checks → advance → 5%, 25%, 100%
                        │
                        ▼
                   NATS efe.registry.canary.completed
                   + transition artifact SERVING → CHAMPION
                   + transition old_champion CHAMPION → DEPRECATED
```

---

## 16. Interfaces publiques

### 16.1 Python API (in-process)

```python
# Top-level façade
from efe_registry import Registry

reg = Registry.from_settings(settings)

# Publish (typically called by M04 worker)
artifact = await reg.publish(
    tenant_id="acme", task_id="summarize", semver="1.4.2",
    s3_url="s3://...", content_hash=b"...", parent_run_id=run_id,
    framework="vllm",
)

# Promote (typically called by M05 via NATS)
await reg.promote(
    decision_id=uuid4(), artifact_id=artifact.artifact_id,
    actor="gate-automation", reason="M05 promote",
)

# Rollback
receipt = await reg.rollback(
    tenant_id="acme", task_id="summarize",
    actor="ops:jdoe", reason="canary breach p95",
    trigger_kind="manual",
)
assert receipt.latency_ms < 60_000
```

### 16.2 gRPC (efe.registry.v1)

```proto
syntax = "proto3";
package efe.registry.v1;

import "google/protobuf/timestamp.proto";

service Registry {
    rpc Publish(PublishRequest) returns (Artifact);
    rpc Promote(PromoteRequest) returns (PromoteReply);
    rpc Rollback(RollbackRequest) returns (RollbackReply);
    rpc GetArtifact(GetArtifactRequest) returns (Artifact);
    rpc ListArtifacts(ListArtifactsRequest) returns (ArtifactList);
    rpc StreamRollbacks(StreamRollbacksRequest) returns (stream RollbackEvent);
    rpc StreamCanary(StreamCanaryRequest) returns (stream CanaryUpdate);
}

message PublishRequest {
    string tenant_id = 1;
    string task_id = 2;
    string semver = 3;
    string s3_url = 4;
    bytes  content_hash = 5;
    string parent_run_id = 6;
    string framework = 7;
    google.protobuf.Timestamp created_at = 8;
}

message Artifact {
    string artifact_id = 1;
    string tenant_id = 2;
    string task_id = 3;
    string semver = 4;
    bytes  content_hash = 5;
    string state = 6;
    google.protobuf.Timestamp created_at = 7;
    google.protobuf.Timestamp promoted_at = 8;
}

message RollbackRequest {
    string tenant_id = 1;
    string task_id = 2;
    string actor = 3;
    string reason = 4;
    string trigger_kind = 5;
    bytes  idempotency_key = 6;
}

message RollbackReply {
    string rollback_id = 1;
    uint32 latency_ms = 2;
    string final_state = 3;
    bytes  from_artifact_hash = 4;
    bytes  to_artifact_hash = 5;
}
```

### 16.3 HTTP REST (FastAPI)

Base path : `/v1/registry`.

| Méthode | Chemin | Usage |
|---|---|---|
| `POST` | `/v1/registry/artifacts` | publish |
| `GET` | `/v1/registry/artifacts/{artifact_id}` | détails |
| `GET` | `/v1/registry/artifacts?tenant=&task=&state=` | liste filtrée |
| `POST` | `/v1/registry/promotions` | promote (interne, gated HMAC) |
| `POST` | `/v1/registry/rollbacks` | rollback (MFA obligatoire) |
| `GET` | `/v1/registry/canaries/{canary_id}` | statut canary |
| `POST` | `/v1/registry/canaries/{canary_id}/advance` | advance manuel |
| `POST` | `/v1/registry/canaries/{canary_id}/pause` | pause |
| `GET` | `/v1/registry/contracts` | liste contrats actifs |
| `GET` | `/v1/registry/rollbacks?tenant=&task=` | historique rollbacks |
| `GET` | `/v1/registry/endpoints?tenant=&task=` | liste endpoints vivants |
| `GET` | `/healthz` | santé interne |

### 16.4 Router HTTP (inférence)

Base path : `/v1/completions` (OpenAI-compat) — exposé par les pods vLLM via
le router.

Headers additionnels :

- `X-EFE-Tenant-Id` : obligatoire (forcé par M11).
- `X-EFE-Task-Id` : obligatoire.
- `X-EFE-Variant` : optionnel (défaut `default`).
- `X-EFE-Contract-Hash` : optionnel (sinon : champion courant).
- `X-EFE-Request-Id` : propagé en trace OTel.

Réponse — headers :

- `X-EFE-Artifact-Id` : artefact qui a servi la requête.
- `X-EFE-Content-Hash` : 64 hex du SHA-256 du modèle servant (premiers 16).
- `X-EFE-Variant-Served` : ex. `canary` / `champion`.
- `X-EFE-Canary-Percent` : % actuel du canary si candidate a servi.

---

## 17. Stratégie de tests

### 17.1 Tests unitaires (≥ 200 cas)

```
tests_registry/
  test_artifact_state_fsm.py          # 13 transitions valides, le reste refusé
  test_content_hash_canonical.py      # stabilité sous permutations fichiers
  test_routing_atomic_swap.py          # Lua CAS version_mismatch
  test_routing_cache_invalidation.py   # PubSub update → local cache refresh
  test_quota_token_bucket_lua.py       # refill, block, concurrent
  test_weighted_dispatch_stratified.py # distribution exacte à 1%
  test_canary_schedule_geometric.py    # progression 1→5→25→100
  test_canary_slo_breach_x3.py         # abort après 3 breach
  test_canary_paused_no_auto_rollback.py
  test_rollback_idempotency.py         # même idem key → même receipt
  test_rollback_version_conflict_retry.py
  test_rollback_drain_hard_deadline.py
  test_rollback_phases_instrumented.py # phases_jsonb complet
  test_shadow_tap_no_impact_on_latency.py
  test_shadow_tap_scoring.py           # cosine, bleu, latency_delta
  test_contract_hash_stable.py         # repr stable sur perm clés
  test_contract_deprecation_sunset.py  # timer 90j
  test_sig_verifier_key_based.py       # wraps cosign OK
  test_sig_verifier_keyless.py          # wraps cosign keyless OK
  test_sig_verifier_rejects_tampered.py
  test_sig_cache_ttl.py                # 60s expire
  test_multi_lora_routing.py           # lora_id propagé à vllm
  test_endpoint_state_fsm.py
  test_pre_warmed_standby_stays_ready.py  # KV cache + health OK
  test_nats_message_hmac_verified.py
  test_admission_policy_fails_unsigned.py
  test_sbom_parseable_cyclonedx_1_5.py
  test_slsa_provenance_schema_valid.py
```

### 17.2 Tests d'intégration (≥ 40 cas)

```
tests_registry_integration/
  test_e2e_publish_sign_admit_promote_rollback.py
       # Scénario complet sur K8s local (kind + Sigstore stack)
  test_e2e_rollback_under_60s.py
       # Mesure réelle SLO, 50 runs consécutifs
  test_e2e_canary_auto_rollback_on_slo.py
       # Fake M07 qui retourne breach, vérifie rollback déclenché
  test_e2e_multi_tenant_isolation.py
       # 2 tenants servent en parallèle, aucune cross-pollination
  test_e2e_contract_deprecation_returns_410.py
       # Après sunset, 410 Gone
  test_e2e_key_rotation_overlap.py
       # Pendant 30 j overlap, artefacts signés par les 2 clés vérifient
  test_e2e_policy_controller_blocks_unsigned.py
       # kubectl apply pod unsigned → rejected
  test_e2e_shadow_tap_produces_diff_scores.py
  test_e2e_routing_redis_failover.py
       # Redis primary tombe, failover sentinel OK, routage continue
  test_e2e_nats_message_replay_idempotent.py
       # Replay d'un message promote ne crée pas double artefact
```

### 17.3 Tests de résilience / chaos

```
tests_registry_chaos/
  test_chaos_redis_primary_kill.py
  test_chaos_pod_oom_during_canary.py
  test_chaos_network_partition_router_redis.py
  test_chaos_clock_skew_50ms.py
  test_chaos_gpu_node_evict_during_serve.py
  test_chaos_standby_warm_pod_dies_mid_rollback.py
       # doit fallback vers nouvelle créée, mais SLO peut glisser 30s
```

### 17.4 Tests de charge

- **Router** : 10 000 req/s p99 < 2 ms (sans backend, mock endpoint).
- **Rollback** : 50 rollbacks consécutifs, aucun dépassement 60 s.
- **Canary tick** : 1000 canary runs actifs simultanés, tick loop < 5 s.

### 17.5 Couverture cible

- `efe_registry.signing` : **95 %** (critique sécu).
- `efe_registry.rollback` : **95 %** (critique SLO).
- `efe_registry.router` : **90 %**.
- `efe_registry.canary` : **90 %**.
- `efe_registry.serving` : **80 %** (wrap vLLM hard à mocker).
- Global : **≥ 85 %**.

---

## 18. Observabilité

### 18.1 Metrics Prometheus

```
# ---- Publication / signature ----
efe_registry_artifact_publish_total{tenant, task, outcome="ok|fail"}
efe_registry_artifact_bytes{tenant, task}              (histogram)
efe_registry_signature_verify_latency_seconds{method, layer="L1|L2|L3|L4"}
                                                       (histogram)
efe_registry_signature_verify_failures_total{method, reason}

# ---- Routing ----
efe_router_requests_total{tenant, task, variant, outcome="2xx|4xx|5xx"}
efe_router_request_latency_seconds{tenant, task}       (histogram)
efe_router_routing_table_version{tenant, task}          (gauge)
efe_router_cache_hits_total{tenant, task}
efe_router_cache_misses_total{tenant, task}
efe_router_quota_rejected_total{tenant, api_key}
efe_router_inflight_requests{tenant, task, artifact_id}  (gauge)
efe_router_dropped_requests_total{tenant, task}         (counter, SLO == 0)

# ---- Canary ----
efe_canary_active{tenant, task, canary_id}             (gauge, 0|1)
efe_canary_percent{tenant, task, canary_id}            (gauge, 0-100)
efe_canary_slo_breaches_total{tenant, task, metric}
efe_canary_advance_total{tenant, task, from_percent, to_percent}
efe_canary_aborted_total{tenant, task, reason}

# ---- Rollback ----
efe_rollback_requested_total{tenant, task, trigger}
efe_rollback_completed_total{tenant, task, trigger}
efe_rollback_failed_total{tenant, task, phase}
efe_rollback_latency_seconds{tenant, task, trigger}    (histogram; SLO 60s p99)
efe_rollback_phase_latency_seconds{tenant, task, phase} (histogram)

# ---- Serving ----
efe_serving_loaded_artifacts{pool}                     (gauge)
efe_serving_vram_utilization_bytes{pool, pod}
efe_serving_tokens_generated_total{tenant, task, artifact_id}
efe_serving_ttft_seconds{tenant, task}                 (histogram)
efe_serving_inter_token_latency_seconds{tenant, task}  (histogram)

# ---- Shadow ----
efe_shadow_samples_total{tenant, task}
efe_shadow_cosine_mean{tenant, task}                   (gauge)
efe_shadow_latency_delta_ms_mean{tenant, task}         (gauge)

# ---- Admission ----
efe_admission_reject_total{policy, reason}
efe_admission_pass_total{policy}
```

### 18.2 Alerting (Alertmanager rules)

```yaml
groups:
- name: efe.registry.slo
  interval: 30s
  rules:
  - alert: EFERollbackSloBreach
    expr: histogram_quantile(0.99,
              rate(efe_rollback_latency_seconds_bucket[5m])) > 60
    for: 2m
    labels: { severity: critical, module: M06 }
    annotations:
      summary: "Rollback p99 > 60s"

  - alert: EFERouterDroppedRequests
    expr: rate(efe_router_dropped_requests_total[5m]) > 0
    for: 1m
    labels: { severity: critical, module: M06 }

  - alert: EFESignatureVerifyFailing
    expr: rate(efe_registry_signature_verify_failures_total[10m]) > 0.1
    for: 5m
    labels: { severity: warning, module: M06 }

  - alert: EFECanaryStuckAtPercent
    expr: efe_canary_percent > 0 and efe_canary_percent < 100
          and time() - efe_canary_last_step_ts > 3600
    for: 30m
    labels: { severity: warning, module: M06 }

  - alert: EFEContractDeprecationNear
    expr: efe_registry_contract_sunset_seconds_remaining < 86400 * 7
    for: 1h
    labels: { severity: info, module: M06 }
```

### 18.3 Dashboards Grafana

**Dashboard "M06 — Model Registry"** (12 panels) :

1. Request rate + error rate par (tenant, task) — stacked area.
2. p50 / p95 / p99 latence routing.
3. Inflight par artefact (timeline empilé).
4. Canary progression (timeline + annotations des steps).
5. Canary active gauge + SLO breach count.
6. Rollback latency histogram + SLO line à 60s.
7. Rollback phases breakdown (stacked bar).
8. Signature verify latency par layer (L1-L4).
9. Routing table version (monotonic counter par tenant).
10. Quota rejections par API key.
11. Shadow cosine mean timeline (candidate vs baseline).
12. Endpoints state (table : ready/loading/draining/failed).

### 18.4 Traces OpenTelemetry

Spans racine :

- `registry.publish` — enfants : `s3.upload`, `db.insert`, `sign.invoke`,
  `nats.publish`.
- `registry.promote` — enfants : `verify_sig`, `canary.create`,
  `routing.update`, `nats.publish`.
- `registry.rollback.execute` — enfants : `lookup_previous`,
  `check_standby`, `atomic_swap`, `drain`, `verify_shift`, `audit.record`.
- `router.route` — enfants : `auth.check`, `quota.enforce`, `cache.lookup`,
  `dispatch.forward`.
- `serving.generate` — enfants : `vllm.prefill`, `vllm.decode`,
  `lora.load`.

Sampling : 10 % global, 100 % pour erreurs + rollbacks.

### 18.5 Logs structurés

Format JSON via `structlog`. Champs standard :

```json
{
  "ts": "2026-04-20T10:12:34.567Z",
  "level": "info",
  "logger": "efe_registry.rollback.executor",
  "event": "rollback_completed",
  "tenant_id": "acme",
  "task_id": "summarize",
  "rollback_id": "...",
  "from_artifact": "...",
  "to_artifact": "...",
  "trigger": "auto_slo",
  "actor": "canary-controller",
  "latency_ms": 42_150,
  "phases": {"lookup_s": 0.08, "swap_s": 0.45, "drain_s": 28.2, ...},
  "trace_id": "...",
  "span_id": "..."
}
```

Logs critiques (rollback completed/failed, admission rejected, signature
fail) : **toujours** émettre un évènement explicite, indépendamment du niveau
log.

---

## 19. Plan de sprint M06

### 19.1 Découpage en 5 lots (10 jours ouvrés, 2 devs)

| Lot | Durée | Contenu | DRI | Livrables |
|---|---|---|---|---|
| **L1 — Registry core** | 2 j | tables DDL, publish, verify L1/L3, S3 adapter | Dev A | `efe_registry.core` + tests unitaires |
| **L2 — vLLM serving** | 2 j | `VllmAdapter`, multi-LoRA, health | Dev B | pods vLLM bootable, API OpenAI-compat |
| **L3 — Router + Lua** | 2 j | `TenantRouter`, Redis Lua swap, quota, cache + PubSub | Dev A | 10k rps staging |
| **L4 — Canary + Shadow** | 2 j | `CanaryController`, `ShadowTapper`, SLO gateway vers M07 | Dev B | canary 1→100% scripté |
| **L5 — Rollback + admission** | 2 j | `RollbackExecutor`, pre-warmed standby, K8s policy-controller, SLSA+SBOM | Dev A+B | E2E < 60s SLO 50/50 |

### 19.2 Dépendances amont bloquantes

- M03 : manifest API stable (L1 bloqué sinon).
- M04 : production d'artefacts + NATS `artifact.ready` fonctionnel.
- M05 : `efe.gate.decisions.promote` publié + `ChampionRegistry.rollback`
  accessible (pour audit chain access).
- M07 : `LiveMetricsProvider` shim minimal (pour SLO gateway L4).
- M12 : Helm chart umbrella + `policy-controller` déployable.

### 19.3 Definition of Done — sprint M06

- [ ] Tous les tests unitaires verts (CI).
- [ ] Tests d'intégration E2E verts sur kind + Sigstore stack en CI.
- [ ] Mesure SLO rollback < 60s p99 sur 50 runs consécutifs staging.
- [ ] Couverture ≥ 85 % (Codecov PR decorator vert).
- [ ] `mypy --strict` vert sur `src/efe_registry/`.
- [ ] `ruff check` zéro erreur.
- [ ] Dashboard Grafana provisionné via Terraform.
- [ ] 5 alertes Alertmanager listées et testées.
- [ ] Document d'architecture à jour (ce doc + ADR si changements).
- [ ] Runbook ops "Rollback en < 60 s" rédigé + drill fait.

---

## 20. Critères d'acceptation

### 20.1 Fonctionnel

1. Un artefact produit par M04 peut être publié, signé (cosign key-based),
   admissioné (K8s policy-controller), promu par M05, canariaé (1→100 %) et
   servi en production **sans intervention manuelle** — chemin happy-path de
   bout-en-bout.
2. Un rollback déclenché par M08 via gRPC `Rollback` revient au champion
   précédent en < 60 s p99 (mesuré sur 50 runs consécutifs).
3. Un canary en breach SLO 3 fois de suite déclenche un rollback automatique
   si `auto_rollback=true`, sinon passe à `paused`.
4. Une requête HTTP avec `X-EFE-Contract-Hash` référant un contrat sunset
   reçoit 410 Gone + corps descriptif.
5. Un pod dont l'image n'est pas signée cosign est **rejeté** par
   policy-controller (mode ENFORCE).
6. Un rollback idempotent (même `idempotency_key`) retourne le même
   `rollback_id` que le premier appel.
7. Deux tenants servis en parallèle (acme + globex) n'interfèrent pas : aucune
   requête acme n'atteint un endpoint globex.

### 20.2 Non-fonctionnel

1. Router p99 latence < 2 ms (hors backend vLLM) à 10 000 rps sur 3 réplicas.
2. Signature L3 verify < 800 ms p95 (cache chaud).
3. Rollback SLO 60 s p99 — alerte P1 si dépassé.
4. Disponibilité contrôle M06 ≥ 99.9 %/mois.
5. Aucune perte de requête (`efe_router_dropped_requests_total` == 0 sauf
   chaos test explicite).

### 20.3 Sécurité & conformité

1. Aucun pod déployé en prod sans signature cosign (vérif sur 100 % des pods
   `efe-prod` namespace via audit automatique hebdomadaire).
2. Tous les artefacts promus ont SBOM + SLSA provenance attachés et vérifiables.
3. Rotation de clé KMS testée sur staging (overlap 30 j validé).
4. Tout rollback est inscrit dans la chaîne d'audit M05 avec signature HMAC.
5. `AuditKind.EMERGENCY_OVERRIDE` détecté → incident P1 automatique.

### 20.4 Qualité d'ingénierie

1. Tests ≥ 200 unitaires + 40 intégration + 6 chaos.
2. Couverture ≥ 85 % global, ≥ 95 % sur signing + rollback.
3. `mypy --strict` vert sur `src/efe_registry/`.
4. `ruff check` zéro erreur.
5. Tous les endpoints API documentés dans OpenAPI 3.1 ; contract_hash
   utilisable par les SDK.
6. Runbook ops rollback < 60 s + runbook rotation de clé rédigés et drillés.

---

## 21. Risques et mitigations

### 21.1 Matrice des risques

| Risque | Impact | Probabilité | Mitigation |
|---|---|---|---|
| Pre-warmed standby coûteux (1 GPU/tenant·task) | financier | Certain | Facturer surcoût aux tenants premium ; multi-LoRA pour petits |
| Redis primary down pendant rollback | P0 indispo | Faible | Sentinel quorum ; routeurs ont cache TTL 5 s comme secours |
| Cosign KMS indisponible | blocage promotions | Faible | Sigstore keyless en backup + cache vérif Redis TTL 60 s |
| Rekor transparency log down | keyless refusé | Faible | Fallback vers mode key-based ; dégradation logée |
| Policy-controller webhook fail | pod rejeté à tort | Moyen | `failurePolicy: Fail` en prod mais override break-glass |
| Drain dépasse 30 s (requêtes longues) | 5xx mineures | Moyen | Accepter petite perte ; documenter dans SLO |
| Canary SLO faux positif | rollback inutile | Moyen | Seuils conservateurs ; 3 breach consécutifs avant abort |
| Contract hash collision | nulle (SHA-256) | Négligeable | N/A |
| Artefact MITM pendant pull S3 | corruption | Faible | L3 verify avant mmap ; TLS 1.3 obligatoire |
| Clé KMS compromise | breach majeur | Très faible | Rotation mensuelle possible ; Rekor immutable |
| NATS message replay | duplicate actions | Faible | Idempotence par `decision_id` / `idempotency_key` |
| Split-brain routing table | incohérence | Très faible | CAS sur version + Redis Cluster quorum |
| Standby pod KV-cache dérive | latence pic rollback | Moyen | Requête synthétique /60 s ; métrique `ttft_standby` |

### 21.2 Runbooks

Runbooks à rédiger (liste obligatoire pour GA) :

- `runbook-rollback-60s.md` — procédure manuelle si auto-rollback échoue.
- `runbook-cosign-kms-down.md` — bascule keyless + alerte legal.
- `runbook-policy-override.md` — émergence break-glass + audit.
- `runbook-key-rotation.md` — rotation planifiée avec overlap.
- `runbook-redis-failover.md` — primary down → sentinel promote.
- `runbook-canary-stuck.md` — pause manuel, diagnostic, resume ou abort.
- `runbook-contract-sunset.md` — retrait définitif d'un contract_hash.

---

## 22. Annexes

### A. Helm values (extrait)

```yaml
# charts/efe-registry/values.yaml
registry:
  replicaCount: 3
  image:
    repository: efe-registry.io/core/registry
    tag: "1.0.0"
    pullPolicy: IfNotPresent
  resources:
    requests: { cpu: 500m, memory: 1Gi }
    limits:   { cpu: 2,    memory: 4Gi }

  signing:
    kmsKeyArn: "arn:aws:kms:eu-west-1:123456789012:alias/efe-signing-prod"
    rekorUrl: "https://rekor.sigstore.dev"
    keylessIssuer: "https://token.actions.githubusercontent.com"
    keylessIdentity: "https://github.com/efe-project/efe/.github/workflows/release.yml@refs/heads/main"
    verifyCache:
      ttlSeconds: 60

  routing:
    redisUrl: "redis://efe-redis-cluster:6379"
    cacheTtlSeconds: 1.0

  canary:
    scheduleKind: "geometric"
    stepDurationSeconds: 600
    autoRollback: true
    autoAdvance: true
    sloBreachAbortThreshold: 3

  rollback:
    hardDeadlineSeconds: 60
    drainDeadlineSeconds: 30
    preWarmedStandby: true
    syntheticPingIntervalSeconds: 60

  s3:
    bucketPattern: "efe-artifacts-{tenant}"
    objectLockDays: 7

serving:
  vllm:
    imageRepository: "efe-registry.io/serving/vllm"
    imageTag: "0.5.3"
    gpuPools:
      - name: gpu_a100_shared
        nodeSelector: { "efe.io/gpu": "a100" }
        tensorParallel: 1
        maxModelLen: 8192
      - name: gpu_h100_large
        nodeSelector: { "efe.io/gpu": "h100" }
        tensorParallel: 4
        maxModelLen: 32768

  standbyWarm:
    enabled: true
    replicasPerTenantTask: 1
    maxUnreadySeconds: 120

admission:
  policyController:
    enabled: true
    mode: enforce
    breakGlassTtlSeconds: 1800
```

### B. Kubernetes `ClusterImagePolicy` complet

```yaml
apiVersion: policy.sigstore.dev/v1beta1
kind: ClusterImagePolicy
metadata:
  name: efe-models-strict
spec:
  images:
    - glob: "efe-registry.io/models/**"
    - glob: "efe-registry.io/serving/**"
  authorities:
    - name: efe-prod-kms
      key:
        kms: awskms:///alias/efe-signing-prod
      ctlog:
        url: https://rekor.sigstore.dev
      attestations:
        - name: must-have-sbom
          predicateType: https://cyclonedx.org/bom
        - name: must-have-slsa
          predicateType: https://slsa.dev/provenance/v1
    - name: efe-prod-kms-2027  # overlap pour rotation
      key:
        kms: awskms:///alias/efe-signing-prod-2027
      ctlog:
        url: https://rekor.sigstore.dev
  mode: enforce
  match:
    - resource: pods
      version: v1
```

### C. Script Lua `atomic_swap.lua` (complet)

```lua
-- atomic_swap.lua
-- KEYS[1] = "routing:{tenant}:{task}:v"
-- KEYS[2] = "routing:{tenant}:{task}"
-- ARGV[1] = expected_version (str int)
-- ARGV[2] = new_version      (str int)
-- ARGV[3..N] = endpoint_id, "<weight_percent>:<role>", ...

if #ARGV < 2 or (#ARGV - 2) % 2 ~= 0 then
    return {err = 'invalid_argc'}
end

local cur = tonumber(redis.call('GET', KEYS[1]) or '0')
local exp = tonumber(ARGV[1])
local nxt = tonumber(ARGV[2])

if cur ~= exp then
    return {err = 'version_mismatch', current = cur}
end
if nxt ~= cur + 1 then
    return {err = 'version_not_sequential'}
end

-- Validate sum of weights = 100 for champion+candidate roles
local weight_sum = 0
for i = 3, #ARGV, 2 do
    local value = ARGV[i + 1]
    local sep = string.find(value, ':')
    if sep == nil then
        return {err = 'invalid_value_format', at = ARGV[i]}
    end
    local w = tonumber(string.sub(value, 1, sep - 1))
    local role = string.sub(value, sep + 1)
    if role == 'champion' or role == 'candidate' then
        weight_sum = weight_sum + w
    end
end
if weight_sum ~= 100 then
    return {err = 'weights_not_100', sum = weight_sum}
end

-- Atomic replace
redis.call('DEL', KEYS[2])
for i = 3, #ARGV, 2 do
    redis.call('HSET', KEYS[2], ARGV[i], ARGV[i + 1])
end
redis.call('SET', KEYS[1], ARGV[2])

-- Notify subscribers
local msg = string.format('{"tenant_key":"%s","version":%d}', KEYS[2], nxt)
redis.call('PUBLISH', 'routing.updated', msg)

return 'ok'
```

### D. `VllmConfig` production (70B)

```python
VllmRuntimeConfig(
    tensor_parallel_size=4,
    max_model_len=32_768,
    dtype="bfloat16",
    gpu_memory_utilization=0.92,
    enable_lora=False,
    max_loras=0,
    max_lora_rank=0,
)
```

Pour multi-LoRA (base 7B + 16 adapters) :

```python
VllmRuntimeConfig(
    tensor_parallel_size=1,
    max_model_len=8_192,
    dtype="bfloat16",
    gpu_memory_utilization=0.85,
    enable_lora=True,
    max_loras=16,
    max_lora_rank=64,
)
```

### E. Invariants SQL (triggers)

```sql
-- Somme des weights champion+candidate = 100 par (tenant, task, variant)
CREATE OR REPLACE FUNCTION check_routing_weights_sum()
RETURNS TRIGGER AS $$
DECLARE
    total INTEGER;
BEGIN
    SELECT COALESCE(SUM(weight_percent), 0) INTO total
    FROM routing_table
    WHERE tenant_id = NEW.tenant_id
      AND task_id = NEW.task_id
      AND variant = NEW.variant
      AND role IN ('champion', 'candidate');
    IF total <> 100 THEN
        RAISE EXCEPTION
            'routing weights for (%, %, %) sum to % (expected 100)',
            NEW.tenant_id, NEW.task_id, NEW.variant, total;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER trg_routing_weights_sum
    AFTER INSERT OR UPDATE OR DELETE ON routing_table
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION check_routing_weights_sum();

-- Un seul champion par (tenant, task, variant)
CREATE UNIQUE INDEX uq_one_champion_per_route
    ON routing_table (tenant_id, task_id, variant)
    WHERE role = 'champion';
```

### F. Commandes opérationnelles de référence

```bash
# Déclencher un rollback manuel (MFA requise par M11)
efe-tool registry rollback \
  --tenant acme --task summarize \
  --actor "$USER" --reason "latency regression"

# Lister les canaries actifs
efe-tool registry canaries list --state ramping

# Pauser un canary
efe-tool registry canaries pause CANARY_ID

# Forcer l'avance d'un canary (override humain)
efe-tool registry canaries advance CANARY_ID --percent 25

# Vérifier la signature d'un artefact en local
efe-tool registry verify ARTIFACT_ID

# Rapport SLO rollback dernier 7j
efe-tool registry slo rollback --since 7d
```

### G. Glossaire M06

| Terme | Définition |
|---|---|
| **Artefact** | Bundle immutable (weights + config + tokenizer + manifest) stocké en S3 Object Lock. |
| **Champion** | Artefact actuellement servant ≥ 1 % du trafic pour un `(tenant, task)` — état FSM. |
| **Candidate** | Artefact en canary (< 100 %). |
| **Standby warm** | Pod pré-chauffé servant 0 % mais prêt à recevoir trafic en < 3 s. |
| **Canary** | Déploiement progressif par paliers de % avec gate SLO. |
| **Shadow** | Tap sur un candidate, réponse jetée, scoré offline. |
| **Rollback** | Bascule atomique du trafic vers le champion N-1, SLO 60 s. |
| **Content-hash** | SHA-256 sur bytes canoniques de l'artefact. |
| **Contract-hash** | SHA-256 sur `(input_schema, output_schema)` canonique. |
| **Cosign** | CLI/SDK Sigstore pour signer/vérifier OCI + blobs. |
| **Fulcio** | CA éphémère Sigstore pour keyless signing (OIDC). |
| **Rekor** | Transparency log public Sigstore. |
| **SBOM** | Software Bill of Materials (CycloneDX). |
| **SLSA** | Supply-chain Levels for Software Artifacts. |
| **Policy-controller** | Admission webhook Sigstore pour K8s. |
| **TenantRouter** | Composant stateless dispatchant `(tenant, task, variant) → endpoint`. |
| **Pre-warmed** | Engine vLLM instancié + KV cache alloué + compilation CUDA faite. |
| **Atomic swap** | Lua Redis MULTI-key HSET + SET + PUBLISH en un seul bloc single-thread. |

---

**Fin du document M06.**
*Version 1.0 — spec gelée pour implémentation — dernière revue 2026-04-20.*
*Tout changement ultérieur passe par une ADR référençant ce document.*
