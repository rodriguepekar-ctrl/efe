# EFE — Architecture **Φ-D-OPS** — Application opérationnelle de production end-to-end

> **Sprint** : Φ-D-OPS (D95 → D99)
>
> **Mission** : transformer les **6 sprints de completion** (Mode-2, Mode-3,
> AL, Streaming, Compliance, P1/P2-LATACT) en un **système unifié,
> déployable, vendable, exploitable** — du signup d'un tenant jusqu'à
> la facturation, l'audit annuel, et l'incident response 24/7.
>
> **Doctrine cardinale** : *un système n'est pas opérationnel parce
> qu'il compile — il l'est quand un on-call débutant à 3h du matin
> peut résoudre un incident sans réveiller un senior*. Tout le reste
> est de la documentation.
>
> Φ-D-OPS livre la **couche d'orchestration** qui rend les 4 sprints
> précédents (200+ pin tests, 105 SLOs, 32 workflows CI, 28 runbooks)
> exploitables en production multi-tenant multi-cloud.

---

## Table des matières

- **§ 0** — Méta : doctrines D-OPS-*, glossaire, conventions
- **§ 1** — Mission Φ-D-OPS & contrat opposable (= 18 SLOs maîtres)
- **§ 2** — État AS-IS : 6 sprints complétés, ce qui manque encore
- **§ 3** — Gap analysis : du « ça compile » au « ça tourne en prod 24/7 »
- **§ 4** — Topologie système : composants × régions × couches
- **§ 5** — Data flow end-to-end : signup → classify → audit → facturation
- **§ 6** — Multi-cloud deployment : K8s topology × 3 régions
- **§ 7** — Tenant lifecycle : signup → sandbox → POC → production → erasure
- **§ 8** — Master SLO consolidation : 105 SLOs → 1 dashboard
- **§ 9** — Observability stack : Prometheus + Grafana + OTel + audit chain
- **§ 10** — Incident response + on-call rotation
- **§ 11** — Release pipeline : canary unifié 4 surfaces
- **§ 12** — Capacity planning + cost model + chargeback
- **§ 13** — Compliance posture continue : conformity assessment automatisé
- **§ 14** — Customer surfaces : portal + API + docs + status page + trust center
- **§ 15** — Internal tools : admin CLI + alert routing + runbook automation
- **§ 16** — Hand-off downstream : M25 agent runtime, M27 commercial edge
- **§ 17** — Matrice complète des artefacts Φ-D-OPS (= 40)
- **§ 18** — Plan d'implémentation D95 → D99
- **§ 19** — Risques + mitigations (24)
- **§ 20** — Décisions clés (22)
- **§ 21** — Anti-régression (= 36 pin tests + 8 realism)
- **§ 22** — Critères d'acceptation (DoD à 100 %)
- **§ 23** — Annexes (A → I)
- **§ 24** — Defer Tickets `DT-OPS-*` (= 14 vrais SKIPs opposables)

---

## § 0. Méta, conventions, glossaire chirurgical

### 0.1 Doctrines D-OPS-* (= 10 contraintes opposables)

| Doctrine | Énoncé |
|---|---|
| **D-OPS-1 — Single source of truth par surface** | 1 SLO file par sprint, 1 runbook par incident type, 1 dashboard Grafana par surface. **Aucune duplication**. Tout SLO référencé doit exister dans **exactement un** `bench_baselines/v*/*.yaml`. |
| **D-OPS-2 — 105 SLOs consolidés en 1 gate** | Le release tag GA gate sur **les 6 fichiers SLO réunis**. Un FAIL anywhere bloque le tag. Master dashboard montre les 6 surfaces en un seul écran. |
| **D-OPS-3 — Multi-cloud parité isomorphique** | AWS / GCP / Azure ont les **mêmes services managés mappés**, les mêmes Terraform modules structurés identiquement, le même bench `cloud_parity` ≤ 20 % variance. |
| **D-OPS-4 — Audit chain continu HMAC** | Tous les events compliance-relevant (signup, sandbox_created, classify, retune, breach, erasure, SLO bascule) écrivent dans `audit_events` avec HMAC-chained. Replay daily. |
| **D-OPS-5 — Canary obligatoire 4 surfaces** | Toute promotion (bundle AL, model retune, code release, schema migration, sub-processor change) passe par : 10 % traffic × 24h × auto-rollback. |
| **D-OPS-6 — Tenant isolation strict du portail au LLM** | Tenant `data_region=EU` ⇒ portal session → API JWT → backend → Postgres → Redis → LLM provider TOUT en EU. 0 cross-region leak. |
| **D-OPS-7 — GitOps continu** | Tout config en YAML versionné (Terraform, Helm, K8s manifests, SLO baselines, alert rules, dashboards). Pas de `kubectl edit` manuel en prod. Audit trail via git. |
| **D-OPS-8 — On-call 24/7 documenté** | 1 SRE primary + 1 secondary + escalation Slack/PagerDuty. Toute alerte critical déclenche page. Runbook obligatoire pour chaque alerte. |
| **D-OPS-9 — Postmortem ≤ 48h pour Sev1/Sev2** | Chaque incident severity ≥ 2 → postmortem Markdown signé dans `docs/postmortems/YYYYMMDD_*.md` ≤ 48h. Action items linkés à Linear. |
| **D-OPS-10 — Cost transparency par tenant** | Chargeback model : chaque tenant a un compteur `tenant_cost_usd_per_month` calculé temps réel (= GPU + LLM API + storage + bande passante). Budget alerts à 80 % / 100 % / 120 %. |

### 0.2 Glossaire chirurgical

| Terme | Définition |
|---|---|
| **Surface** | Une zone fonctionnelle EFE : Mode-1, Mode-2, Mode-3, AL, Streaming, Compliance, P1 latence, P2 activation. |
| **Master gate** | Workflow CI qui exécute les 6 SLO suites et bloque le release tag si une seule FAIL. |
| **Tenant tier** | `sandbox` (free) / `poc` (90 jours, 1 catalog) / `production` (multi-catalog, SLA) / `enterprise` (sovereign deploy). |
| **Compliance posture** | État courant des 256 controls × 6 frameworks avec evidence runtime. Doit être PASS-able à tout moment pour un audit ad-hoc. |
| **Canary** | Déploiement 10 % du trafic vers une nouvelle version pendant 24h minimum avec auto-rollback si SLO regression. |
| **Chargeback** | Compteur `tenant_cost_usd` granulaire qui décompose GPU/LLM/storage/network par tenant. |
| **Master dashboard** | 1 page Grafana qui montre les 105 SLOs sur les 6 surfaces avec heat-map. |
| **Runbook** | Markdown opérationnel `docs/runbook/<event>.md` avec : détection, causes, triage, mitigation, cross-refs. |
| **Postmortem** | Markdown `docs/postmortems/YYYYMMDD_<slug>.md` post-Sev1/Sev2 : timeline, root cause, contributing factors, action items, prevention. |

### 0.3 Conventions de référence

- SLO master : `SLO-OPS-<KEY>`
- Sprint Φ-D-OPS = 5 jours D95 → D99
- Defer Ticket : `DT-OPS-<NAME>`
- Cross-refs : EFE_AL_*, EFE_STREAM_*, EFE_COMP_*, EFE_LATACT_*, EFE_MODE2_*, EFE_MODE3_*, M29
- Severity : Sev1 = total outage / data loss ; Sev2 = degraded SLO breach ; Sev3 = single tenant impact ; Sev4 = no customer impact

### 0.4 Anti-mission (= ce que Φ-D-OPS **ne fait pas**)

- ❌ Ne réécrit pas les 6 sprints précédents — orchestre uniquement.
- ❌ Ne livre pas le billing self-service (= sortie de scope, M28-M30).
- ❌ Ne livre pas le Customer Success runbook (= P2 sales + post-sales out of scope).
- ❌ Ne livre pas le SOC 2 Type II audit (= 12 mois période d'observation externe).
- ❌ Ne livre pas l'IPv6 / geographic load balancer (= follow-up M27).

---

## § 1. Mission Φ-D-OPS & contrat opposable

### 1.1 Mission

> Transformer 4 sprints de completion (= 200 pin tests, 105 SLOs, 32
> workflows, 28 runbooks) en un **système opérationnel** où :
>
> 1. Un **nouvel utilisateur** s'inscrit, crée un sandbox, lance sa
>    première classification, signe POC, en **≤ 30 min p90** (= P2).
> 2. Un **client production** envoie 100k requêtes/jour avec p99 ≤ 200
>    ms GPU, p99.99 ≤ 800 ms (= P1).
> 3. Un **auditeur** demande l'evidence de 256 controls — récupérable
>    en 2 min via portail (= Compliance).
> 4. Un **incident Sev1** (= leak data, breach) est notifié au DPO ≤
>    72h via la chaîne automatisée (= Compliance + Streaming).
> 5. Un **SRE on-call** débutant résout 80 % des alertes via runbooks
>    sans escalade.
> 6. Un **CTO acheteur** reçoit le pitch + démo + LOI en 45 min, intégration
>    en 5 jours (= GTM).

### 1.2 Contrat opposable (= 18 master SLOs)

Ces 18 SLO sont des **agrégats** au-dessus des 105 SLO surface. Ils
sont la lentille « business » que le CTO / CFO / DPO comprennent.

| Surface | Master SLO | Cible | Source |
|---|---|---|---|
| **Disponibilité** | SLO-OPS-AVAILABILITY | ≥ 99.95 % uptime mensuel | tous les surfaces |
| **Latence end-to-end** | SLO-OPS-E2E-P99 | ≤ 250 ms p99 (gRPC client → final chunk) | streaming + Mode-1/2/3 |
| **Compliance posture** | SLO-OPS-COMPLIANCE | 18/18 framework SLOs PASS | comp_slos.yaml |
| **Tenant onboarding** | SLO-OPS-ONBOARDING | TTFC sandbox p90 ≤ 5 min | latact_slos.yaml |
| **Cloud parity** | SLO-OPS-CLOUD-PARITY | variance < 20 % AWS/GCP/Azure | latact + benchmarks |
| **Cost per tenant** | SLO-OPS-COST-CONTROL | aucun tenant > 120 % budget | chargeback bench |
| **Release safety** | SLO-OPS-RELEASE-SAFETY | 100 % releases passent canary 24h | release pipeline |
| **Audit chain integrity** | SLO-OPS-AUDIT-INTEGRITY | daily HMAC replay = 0 broken | comp + AL + stream |
| **Incident response** | SLO-OPS-INCIDENT-TTACK | Sev1 ack ≤ 5 min, Sev2 ≤ 15 min | PagerDuty stats |
| **Postmortem velocity** | SLO-OPS-POSTMORTEM-48H | 100 % Sev1/Sev2 PM ≤ 48h | docs/postmortems/ |
| **On-call quality** | SLO-OPS-PAGE-RUNBOOK | 100 % alerts have runbook | static lint |
| **GitOps coverage** | SLO-OPS-GITOPS | 0 manual `kubectl edit` (audit log) | k8s admission |
| **Erasure SLA** | SLO-OPS-ERASURE-30D | 100 % requests done ≤ 30j | comp_erasure |
| **Breach SLA** | SLO-OPS-BREACH-72H | 100 % notifications ≤ 72h | comp_breach |
| **Pin tests health** | SLO-OPS-PINTESTS | 100 % green dans CI nightly | 4 test suites |
| **Documentation drift** | SLO-OPS-DOC-DRIFT | archis ↔ code ↔ tests cohérents | static lint cross-ref |
| **Trust center freshness** | SLO-OPS-TRUST-CENTER | publié ≤ 7j post-changement | static lint trust center |
| **Multi-region failover** | SLO-OPS-FAILOVER | RTO ≤ 1h, RPO ≤ 15 min | DR drill quarterly |

**Total** : 18 master SLOs au-dessus de **105 SLOs surface**. Pyramide cohérente.

### 1.3 Anti-mission

- ❌ Ne pas re-créer de SLOs surface (= déjà 105).
- ❌ Ne pas modifier les SLOs surface existants (= rigueur cross-sprint).
- ❌ Ne pas livrer de nouveaux modules métier (= orchestration only).

---

## § 2. État AS-IS : 6 sprints complétés

### 2.1 Inventaire chirurgical des sprints livrés

| Sprint | Surface | Pin tests | SLOs | Workflows | Runbooks | Status |
|---|---|---|---|---|---|---|
| **Φ-D-MODE2** | LLM-routed anchors (8 providers) | 18 | 18 | 3 | 4 | ✓ archi + code |
| **Φ-D-MODE3** | Multi-turn K-swarms | 18 | 18 | 4 | 5 | ✓ archi + code |
| **Φ-D-AL** | Active Learning closed-loop | 36 | 18 | 4 | 5 | ✓ complet |
| **Φ-D-STREAM** | 4 streaming RPCs | 45 | 18 | 3 | 4 | ✓ complet |
| **Φ-D-COMPLIANCE** | 6 frameworks + 256 controls | 45 | 18 | 3 | 5 | ✓ complet |
| **Φ-D-LATACT** | P1 GPU + P2 activation | 50 | 18 | 3 | 5 | ✓ complet |
| **TOTAL livré** | | **212 tests** | **108 SLOs** | **20 workflows** | **28 runbooks** | |
| **P1 + P2 surfaces v0.2.0** | (héritées) | (incluses) | 33 | 2 | (héritées) | inclus dans LATACT |

### 2.2 Ce qui manque pour passer en production (= gaps Φ-D-OPS)

| Gap | Sévérité | Surface concernée |
|---|---|---|
| Master dashboard 105 SLOs en 1 page | HIGH | Observability |
| Master gate CI (= 6 SLO suites en une exécution) | CRIT | Release |
| Tenant lifecycle automation signup → erasure | CRIT | Tenant ops |
| PagerDuty integration + on-call rotation | CRIT | Incident response |
| Helm charts production-grade | CRIT | K8s deploy |
| Chargeback metering temps-réel | HIGH | Cost/billing |
| Trust center customer-facing | HIGH | GTM compliance |
| Runbook automation (= alert → suggested runbook link) | MED | Ops experience |
| Postmortem template + tooling | MED | Quality |
| Capacity planning model (= prediction GPU/PG/Redis) | MED | Cost |
| DR drill quarterly playbook | HIGH | BCP |
| Admin CLI for support team | MED | Internal tools |
| Status page (= statuspage.io equivalent) | HIGH | Customer trust |
| Multi-region failover (active-passive) | HIGH | Resilience |

**Total** : 14 gaps. Φ-D-OPS les résout en 5 jours D95-D99 par 4 contributors.

---

## § 3. Gap analysis : « ça compile » → « ça tourne 24/7 »

### 3.1 Les 5 dimensions d'opérationnalité

```
                       Operational readiness
                              │
            ┌─────────────────┼─────────────────┐
            │                 │                 │
       1. Deploy           2. Run             3. Heal
       (= can ship)        (= stays up)       (= recovers)
            │                 │                 │
       ┌────┴────┐       ┌────┴────┐       ┌────┴────┐
       │ Helm    │       │ Prom    │       │ Runbook │
       │ Terraf  │       │ Grafana │       │ PagerD  │
       │ CI/CD   │       │ OTel    │       │ PM 48h  │
       │ Canary  │       │ Logs    │       │ Failover│
       └─────────┘       └─────────┘       └─────────┘

       4. Audit           5. Sell
       (= prove)          (= scale)
            │                 │
       ┌────┴────┐       ┌────┴────┐
       │ Compl   │       │ Portal  │
       │ Trust   │       │ Pricing │
       │ HMAC    │       │ Charge  │
       │ DPIA    │       │ Status  │
       └─────────┘       └─────────┘
```

Nos 4 sprints précédents couvrent partiellement chaque dimension.
**Φ-D-OPS** unifie + complète :
- **Deploy** : Helm + Terraform 3-cloud + master canary.
- **Run** : master Grafana + Prometheus federation cross-cluster.
- **Heal** : PagerDuty integration + runbook automation + DR drill.
- **Audit** : trust center auto-publié + DPIA auto-régénéré.
- **Sell** : portal v1 + status page + chargeback metering.

### 3.2 Le test ultime : « New on-call à 3h du matin »

> **Doctrine** : un système est opérationnel quand un SRE débutant à 3h
> du matin peut résoudre 80 % des alertes sans escalade. Si l'alerte
> nécessite un domain expert pour être triée, c'est un échec de
> conception, pas un échec de l'on-call.

Les 4 sprints précédents passent ce test au niveau **surface** (= chaque
runbook a un triage step). Φ-D-OPS l'étend au niveau **système** :
- Alert PagerDuty → lien direct vers le runbook
- Runbook → commandes copy-pasteable (= 1 clic / 1 ligne)
- Runbook → escalade path explicit (= qui à réveiller si triage échoue)
- Postmortem template auto-pré-rempli depuis l'incident timeline

---

## § 4. Topologie système : composants × régions × couches

### 4.1 Diagramme d'architecture (= 4 layers × 3 régions)

```
                            ┌─────────────────────────┐
                            │   USER (browser / SDK)  │
                            └────────────┬────────────┘
                                         │ HTTPS / mTLS
                                         ▼
        ┌────────────────────────────────┴────────────────────────────────┐
        │                          EDGE LAYER                              │
        │  Cloudflare (CDN, WAF, DDoS, Bot fight)                          │
        │  - TLS 1.3                                                        │
        │  - Geographic routing (= tenant.data_region pre-flight)          │
        │  - Rate limiting per tenant per IP                               │
        └────────────────────────────┬─────────────────────────────────────┘
                                     │
        ┌────────────────────────────┴─────────────────────────────────────┐
        │                    REGIONAL LAYER (EU / US / APAC)              │
        │                                                                  │
        │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐          │
        │  │ Portal       │  │ gRPC API     │  │ Admin API    │          │
        │  │ (Next.js 14) │  │ (efe-server) │  │ (efe-admin)  │          │
        │  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘          │
        │         │ /api/*          │ Mode-1/2/3       │                  │
        │         └────────┬────────┴────────┬─────────┘                  │
        │                  │                 │                            │
        │            ┌─────┴─────┐    ┌─────┴─────┐                      │
        │            │ AuthZ +   │    │ Routing + │                      │
        │            │ tenant ID │    │ saga      │                      │
        │            └─────┬─────┘    └─────┬─────┘                      │
        │                  │                 │                            │
        │  ┌───────────────┴─────────────────┴────────────────────┐      │
        │  │              CORE INFERENCE LAYER                     │      │
        │  │                                                       │      │
        │  │  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐  │      │
        │  │  │ Mode-1  │  │ Mode-2  │  │ Mode-3  │  │ AL pipe │  │      │
        │  │  │ swarm   │  │ LLM     │  │ K-swarms│  │ closed  │  │      │
        │  │  │ (Rust)  │  │ router  │  │ + agg   │  │ loop    │  │      │
        │  │  └────┬────┘  └────┬────┘  └────┬────┘  └────┬────┘  │      │
        │  │       └────────────┴────────────┴────────────┘       │      │
        │  └──────────────────────┬───────────────────────────────┘      │
        │                         │                                       │
        │  ┌──────────────────────┴───────────────────────────────┐      │
        │  │               STREAMING + AUDIT LAYER                 │      │
        │  │                                                       │      │
        │  │  ┌─────────────┐   ┌─────────────┐   ┌─────────────┐ │      │
        │  │  │RoundEmitter │   │ Saga        │   │ AuditStream │ │      │
        │  │  │tonic mpsc   │   │ orchestr.   │   │ HMAC chain  │ │      │
        │  │  └─────────────┘   └─────────────┘   └─────────────┘ │      │
        │  └──────────────────────┬───────────────────────────────┘      │
        │                         │                                       │
        │  ┌──────────────────────┴───────────────────────────────┐      │
        │  │              STORAGE LAYER (tenant-isolated)          │      │
        │  │                                                       │      │
        │  │  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐  │      │
        │  │  │ PG      │  │ Redis   │  │ S3-like │  │ Vault   │  │      │
        │  │  │ tenant  │  │ cache   │  │ backups │  │ secrets │  │      │
        │  │  │ -shard  │  │ + rate  │  │ + audit │  │ + HMAC  │  │      │
        │  │  │         │  │  limits │  │  cold   │  │  keys   │  │      │
        │  │  └─────────┘  └─────────┘  └─────────┘  └─────────┘  │      │
        │  └───────────────────────────────────────────────────────┘      │
        └──────────────────────────────────────────────────────────────────┘
                                     │
        ┌────────────────────────────┴─────────────────────────────────────┐
        │                       EXTERNAL LAYER                             │
        │                                                                  │
        │  LLM providers (residency-aware) :                              │
        │    EU-only : Mistral, Azure-EU, Vertex-EU, Bedrock-EU, Llama    │
        │    US-only : OpenAI, Anthropic, Cohere                          │
        │    Multi    : (= per-tenant policy)                              │
        │                                                                  │
        │  Observability : Prometheus + Grafana Cloud / Datadog (EU/US)   │
        │  Email/SMS    : SendGrid / Twilio (DPA signed)                  │
        │  PagerDuty    : multi-region alerting                            │
        └──────────────────────────────────────────────────────────────────┘
```

### 4.2 Détail couche par couche

#### 4.2.1 Edge layer
- **Cloudflare** : CDN + WAF + DDoS + Bot Fight + TLS 1.3 termination
- **Routing** : Cloudflare Workers inspecte `Authorization: Bearer <jwt>` →
  extrait `tenant.data_region` claim → route vers EU/US/APAC backend.
- **Rate limit** : per-tenant + per-IP token bucket.

#### 4.2.2 Regional layer
- 3 régions actives : **EU-West-1** (Paris), **US-East-1** (Virginia),
  **AP-Northeast-1** (Tokyo).
- Chaque région = K8s cluster (EKS / GKE / AKS selon le cloud secondaire).
- Multi-cloud disponible pour les enterprises (= sovereign deploy).

#### 4.2.3 Core inference layer
- 4 sub-systems Rust : Mode-1, Mode-2, Mode-3, AL closed-loop.
- Stateless ; horizontal pods auto-scaler (HPA) CPU/GPU.
- GPU pool dédié pour Mode-1/3 (= NVIDIA T4 ou L4, spot-friendly).

#### 4.2.4 Streaming + audit layer
- gRPC streaming via `tonic` 0.11.
- `RoundEmitter` trait (= sprint Φ-D-STREAM).
- `Saga` orchestrator (= sprint Φ-D-LATACT) pour tenant deploy + GDPR delete.
- `AuditStream` reader (= PG LISTEN/NOTIFY pour replay temps-réel).

#### 4.2.5 Storage layer
- **PostgreSQL 16** : RDS / Cloud SQL / Azure DB Flexible. Per-tenant
  shard (= 1 schema par tenant, isolation strict).
- **Redis 7** : ElastiCache / MemoryStore / Azure Cache for Redis.
- **Object store** : S3 / GCS / Azure Blob — backups + cold audit
  retention 7-10 ans.
- **Key Vault** : AWS KMS / GCP KMS / Azure Key Vault — clés HMAC
  per-tenant, rotation 90j.

#### 4.2.6 External layer
- 8 LLM providers (cf. Mode-2 archi).
- Observability : Prometheus self-hosted + Grafana Cloud (EU+US tier).
- PagerDuty : on-call rotation 24/7.

### 4.3 Surface map (= où chaque sprint vit dans la topologie)

| Sprint | Layer | Composant principal |
|---|---|---|
| Mode-1 | Core inference | `efe-server::services::swarm` |
| Mode-2 | Core + External | `efe-server::services::mode2` + LLM router |
| Mode-3 | Core | `efe-server::services::mode3` (K-swarms) |
| AL | Core + Storage | AL pipeline + PG gold queue |
| Streaming | Streaming + audit | `RoundEmitter` + `AuditStream` |
| Compliance | Cross-layer | HMAC chain + DPIA + ROPA generators |
| LATACT (P1) | Core inference (GPU) | sustained bench + tail multi-regime |
| LATACT (P2) | Regional + Portal | Next.js + saga + Terraform 3-cloud |

---

## § 5. Data flow end-to-end : signup → classify → audit → facturation

### 5.1 Le parcours complet (= 13 étapes)

```
[1] User signup via portal/signup
        │ POST /api/signup {email, plan_target}
        ▼
[2] efe-server::auth assigns tenant_id (UUID v7)
        │ writes onboarding_sessions row + audit_events.signup
        ▼
[3] Saga orchestrator provisions tenant
        │ - Provision PG schema (= per-tenant migration)
        │ - Migrate base tables
        │ - Seed demo catalog
        │ - Notify tenant email
        │ - Mark active
        │ Each step idempotent + compensable (saga)
        ▼
[4] Portal redirects to /sandbox
        │ Client emits activation.sandbox_created → audit
        │ time_since_signup_ms recorded (= TTFC timer)
        ▼
[5] User clicks "Run sample classify"
        │ portal → POST /api/sandbox/classify {tenant_id, query}
        ▼
[6] efe-server::services::swarm receives Mode-1 request
        │ - JWT check (tenant_id match)
        │ - Rate limit (= token bucket per tenant)
        │ - Broad scope check (= D22 mTLS)
        │ - Backend.execute()
        ▼
[7] SwarmController runs deterministic inference
        │ N fusion rounds emit RoundUpdate to mpsc(64)
        │ Streaming layer pushes to tonic stream
        │ Final chunk emitted
        ▼
[8] activation.first_classify emitted → audit chain
        │ HMAC-chained signature stored
        │ web_vitals pushed by client to Prometheus
        ▼
[9] User confirms result → activation.first_correct = AHA moment
        │ Nudge UI shows "Upgrade to POC"
        ▼
[10] User clicks "Request POC" → activation.poc_requested
        │ Saga starts: provision multi-cloud + DPIA + tenant config
        │ EU tenant + high-risk catalog → AI Act forces Mode-3
        ▼
[11] Production traffic: Mode-3 fan-out K=3 sub-swarms
        │ Each sub-swarm runs Mode-1 (Rust) or Mode-2 (LLM-routed)
        │ Mode-3 aggregator emits Final = aggregated verdict
        │ Every inference: audit chain entry + chargeback metering
        ▼
[12] Daily compliance jobs:
        │ - audit_chain_replay: HMAC chain integrity
        │ - dpia_freshness: per high-risk catalog ≤ 6 months
        │ - ropa_freshness: per tenant ≤ 1 month
        │ - sub-processor inventory ≤ 7 days
        │ - data residency scan: 0 cross-region leak
        ▼
[13] Monthly billing cron:
        │ Sum tenant_cost_usd from chargeback metering
        │ Generate invoice (Stripe billing or manual)
        │ Update audit chain with billing event
```

### 5.2 Latence cumulée (= budget E2E p99)

```
[Edge Cloudflare] ─────────────────── ~ 20 ms (TLS + WAF)
[Regional routing] ───────────────── ~ 5 ms
[Auth + JWT verify] ──────────────── ~ 8 ms
[Rate limit Redis lookup] ────────── ~ 3 ms
[Mode-1 SwarmController] ─────────── ~ 140 ms (GPU)
[Audit chain write PG] ───────────── ~ 12 ms
[Streaming serialize + emit] ─────── ~ 4 ms
[Network return] ─────────────────── ~ 8 ms
─────────────────────────────────────────────
SUB-TOTAL                              ~ 200 ms (= SLO-OPS-E2E-P99 target)

Mode-3 K=3 sub-swarms parallel : max(3× Mode-1) + agg ~ 350 ms
```

### 5.3 Audit chain entries pour 1 request typical

```
1. activation.classify_request_received
2. authz.scope_verified
3. ratelimit.token_consumed
4. swarm.execute_started
5. swarm.round_completed × N
6. swarm.execute_finished
7. activation.classify_response_sent
8. chargeback.tenant_cost_incremented

= 5 + N events HMAC-chained
```

---

## § 6. Multi-cloud deployment : K8s topology × 3 régions

### 6.1 K8s cluster layout (= identical across 3 clouds)

```yaml
# helm/values-prod-eu.yaml
namespace: efe-prod
region: eu-west-1
cloud: aws  # | gcp | azure
replicas:
  edge:        3   # Cloudflare-fronted
  api:         6   # gRPC server pool
  worker:      4   # AL + saga workers
  gpu:         2   # GPU pool T4 (Mode-1/3)
  scheduler:   1   # cron jobs
postgres:
  size: db.r6g.xlarge        # AWS RDS / GCP db-custom-4-16 / Azure GP_Standard_D4ds_v4
  ha: true
  backup_retention_days: 30
redis:
  size: cache.r6g.large
  replicas: 2
storage:
  cold_retention_years: 7
  region: eu-west-1
hmac_key_kms_arn: <region-specific>
```

### 6.2 Helm chart structure

```
helm/efe/
├── Chart.yaml
├── values.yaml                     # defaults
├── values-prod-eu.yaml             # EU prod override
├── values-prod-us.yaml             # US prod override
├── values-prod-ap.yaml             # APAC prod override
├── values-staging.yaml             # single-region staging
├── templates/
│   ├── _helpers.tpl
│   ├── api-deployment.yaml         # efe-server pods
│   ├── api-service.yaml
│   ├── api-hpa.yaml                # Horizontal Pod Autoscaler
│   ├── worker-deployment.yaml      # AL workers + saga
│   ├── gpu-deployment.yaml         # GPU pool (toleration nvidia.com/gpu)
│   ├── portal-deployment.yaml      # Next.js
│   ├── cron-jobs.yaml              # daily audit_replay, DPIA, ROPA
│   ├── postgres-pvc.yaml           # if self-managed PG
│   ├── secrets.yaml                # references KMS-mounted secrets
│   ├── configmaps.yaml             # tenant_config, llm_providers
│   ├── network-policy.yaml         # tenant isolation + egress rules
│   ├── pod-disruption-budget.yaml  # minAvailable for HA
│   ├── service-account.yaml
│   ├── role-binding.yaml
│   ├── ingress.yaml                # Cloudflare-friendly
│   └── monitoring/
│       ├── service-monitor.yaml    # Prometheus scrape
│       └── prometheus-rules.yaml   # 12 burn-rate cells from LATACT
└── crds/
    ├── tenant.yaml                  # CRD : Tenant resource
    └── catalog.yaml                 # CRD : Catalog resource
```

### 6.3 Tenant CRD example

```yaml
apiVersion: efe.ai/v1
kind: Tenant
metadata:
  name: tenant-acme
spec:
  plan: production
  data_region: EU
  ai_act_audience: high_risk
  consent:
    analytics: true
    model_training: false
    marketing: false
    third_party_sharing: false
  catalogs:
    - id: credit-decision-v1
      ai_act_risk: high
      forced_mode: mode3
      dpo_signoff_expires: "2027-04-15"
  budget_usd_monthly: 5000
status:
  provisioned_at: "2026-04-15T10:00:00Z"
  last_dpia_at: "2026-04-15T10:00:00Z"
  last_ropa_at: "2026-04-15T10:00:00Z"
  total_inferences_30d: 1450000
  cost_usd_mtd: 1234.56
```

Le K8s **CRD Tenant** est l'API déclarative. Le `efe-admin` CLI le manipule. Les sagas (provision, erasure) sont déclenchées par les events sur ces CRDs.

### 6.4 Multi-region failover (= active-passive)

```
                        ┌───────────────────────┐
                        │  Cloudflare GeoDNS    │
                        │  Anycast IP           │
                        └──────────┬────────────┘
                                   │ healthcheck every 10s
                ┌──────────────────┼──────────────────┐
                ▼                  ▼                  ▼
        ┌──────────────┐   ┌──────────────┐   ┌──────────────┐
        │ EU primary   │   │ US primary   │   │ AP primary   │
        │ (active)     │   │ (active)     │   │ (active)     │
        └──────┬───────┘   └──────┬───────┘   └──────┬───────┘
               │                  │                  │
        ┌──────┴────┐      ┌─────┴─────┐      ┌─────┴─────┐
        │ EU passive│      │ US passive│      │ AP passive│
        │ (= US-W2) │      │ (= US-E1+ │      │ (= AP-S1) │
        │           │      │  US-W1)   │      │           │
        └───────────┘      └───────────┘      └───────────┘

  RTO target : ≤ 1h failover (SLO-OPS-FAILOVER)
  RPO target : ≤ 15 min data loss (= PG streaming replication)
  Quarterly DR drill : actual failover test, postmortem.
```

---

## § 7. Tenant lifecycle : signup → erasure (état-machine complète)

### 7.1 États du tenant (= 12 states)

```
                  ┌───────────────────┐
                  │   SIGNED_UP       │ (= post-/api/signup)
                  └─────────┬─────────┘
                            │ saga.provision_sandbox
                            ▼
                  ┌───────────────────┐
                  │ SANDBOX_PROVISIONING │
                  └─────────┬─────────┘
                            │ all steps OK
                            ▼
                  ┌───────────────────┐
                  │   SANDBOX_ACTIVE  │ (= can call /sandbox/classify)
                  └─────────┬─────────┘
                            │ user clicks /poc/request
                            ▼
                  ┌───────────────────┐
                  │  POC_REQUESTED    │
                  └─────────┬─────────┘
                            │ ops approval + saga.provision_poc
                            ▼
                  ┌───────────────────┐
                  │  POC_PROVISIONING │
                  └─────────┬─────────┘
                            │ multi-cloud provision complete
                            ▼
                  ┌───────────────────┐
                  │   POC_ACTIVE      │ (= 90-day pilot)
                  └─────────┬─────────┘
                            │ DPA + commercial contract signed
                            ▼
                  ┌───────────────────┐
                  │ PRODUCTION_ACTIVE │ (= multi-catalog, SLA, billing)
                  └────┬──────┬───────┘
                       │      │
                       │      │ trigger : Art. 17 erasure request
                       │      ▼
                       │   ┌──────────────────┐
                       │   │ ERASURE_PENDING  │
                       │   └────────┬─────────┘
                       │            │ saga.gdpr_erasure_cascade
                       │            ▼
                       │   ┌──────────────────┐
                       │   │ ERASURE_DONE     │ (= tombstoned, no PII)
                       │   └──────────────────┘
                       │
                       │ trigger : downgrade / abandon
                       ▼
                  ┌───────────────────┐
                  │   SUSPENDED       │ (= 30-day grace before erasure)
                  └─────────┬─────────┘
                            │ 30 days
                            ▼
                  ┌───────────────────┐
                  │ AUTO_ERASURE      │ (= cf. ERASURE_PENDING flow)
                  └───────────────────┘
```

### 7.2 PostgreSQL state machine table

```sql
CREATE TABLE tenant_lifecycle_states (
  tenant_id           TEXT PRIMARY KEY,
  current_state       TEXT NOT NULL CHECK (current_state IN (
    'SIGNED_UP', 'SANDBOX_PROVISIONING', 'SANDBOX_ACTIVE',
    'POC_REQUESTED', 'POC_PROVISIONING', 'POC_ACTIVE',
    'PRODUCTION_ACTIVE', 'SUSPENDED', 'ERASURE_PENDING',
    'ERASURE_DONE', 'AUTO_ERASURE'
  )),
  entered_state_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  state_history       JSONB NOT NULL DEFAULT '[]'::jsonb,
  saga_run_id         UUID,
  last_audit_event_id TEXT,
  updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_tenant_lifecycle_state ON tenant_lifecycle_states (current_state);
```

### 7.3 Saga par transition d'état

| Transition | Saga | Steps | Compensation |
|---|---|---|---|
| `SIGNED_UP` → `SANDBOX_ACTIVE` | `saga_provision_sandbox` | 5 steps (PG schema, seed, notify) | drop schema, cancel email |
| `POC_REQUESTED` → `POC_ACTIVE` | `saga_provision_poc` | 8 steps (Terraform, DPIA, ROPA, sub-proc notify) | terraform destroy, revert ROPA |
| `POC_ACTIVE` → `PRODUCTION_ACTIVE` | `saga_promote_to_prod` | 6 steps (billing setup, SLA tier, dedicated runbook) | revert billing |
| any → `ERASURE_PENDING` | `saga_gdpr_erasure` | 6 steps (audit start, PG drop, Redis purge, S3 purge, Glacier purge, audit complete) | rare, only audit |

### 7.4 SLA par tenant tier

| Tier | TTFC | Latency p99 | Audit SLA | Support | Pricing |
|---|---|---|---|---|---|
| **Sandbox** | ≤ 5 min | best-effort | daily | community | Free |
| **POC** | ≤ 30 min | ≤ 500 ms | daily | email 24h | $2k/mo flat |
| **Production** | ≤ 24h | ≤ 250 ms p99 | continuous | Slack + email 4h | $10-50k/mo |
| **Enterprise** | white-glove | dedicated cluster | continuous + audit portal | dedicated CSM + PagerDuty bridge | $200k+/yr |

---

## § 8. Master SLO consolidation : 105 SLOs → 1 dashboard

### 8.1 Hiérarchie

```
                                Master SLO (18 OPS)
                                       │
                ┌─────────┬─────────┬─┴───────┬─────────┬─────────┐
                │         │         │         │         │         │
            Mode-1     Mode-2    Mode-3      AL       Streaming  Compliance  LATACT
            (P1)       (M21)     (M22)    (M24)      (Φ-D-S)   (Φ-D-C)    (Φ-D-LA)
              │         │         │         │         │         │         │
            17 SLO   18 SLO    18 SLO    18 SLO    18 SLO    18 SLO    18 SLO

  Total : 18 + 17 + 18 + 18 + 18 + 18 + 18 = 125 (mais 17 P1 héritées partagées avec Mode-1)
  Effective unique : 105 SLOs
```

### 8.2 Master gate workflow

```yaml
# .github/workflows/ops-master-gate.yml
name: ops-master-gate

on:
  pull_request:
  push:
    tags: ['v*.*.*']
  schedule:
    - cron: "0 6 * * *"   # daily nightly
  workflow_dispatch:

jobs:
  consolidate:
    name: Run all 6 SLO suites + consolidate
    runs-on: ubuntu-22.04
    timeout-minutes: 20
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with: {python-version: "3.12"}
      - run: pip install pytest pyyaml cryptography

      # ── 6 SLO suites in parallel ──
      - name: AL master gate
        run: bash scripts/ops/run_suite.sh al

      - name: Streaming master gate
        run: bash scripts/ops/run_suite.sh stream

      - name: Compliance master gate
        run: bash scripts/ops/run_suite.sh compliance

      - name: LATACT master gate
        run: bash scripts/ops/run_suite.sh latact

      - name: Mode-2 + Mode-3 master gate (from prior sprints)
        run: bash scripts/ops/run_suite.sh modes

      - name: Consolidate 105 SLOs
        run: |
          python3 scripts/ops/consolidate_all_slos.py \
            --baselines bench_baselines/v0.6.0/al_slos.yaml \
            --baselines bench_baselines/v0.7.0/stream_slos.yaml \
            --baselines bench_baselines/v0.8.0/comp_slos.yaml \
            --baselines bench_baselines/v0.8.0/latact_slos.yaml \
            --out target/ops/master_report.md
          cat target/ops/master_report.md

      - name: Hard gate
        run: |
          python3 scripts/ops/master_gate_check.py \
            --report target/ops/master_report.md \
            --tolerate-skip-with-dt \
            --fail-on-fail
```

### 8.3 Master Grafana dashboard layout

```text
┌─────────────────────────────────────────────────────────────────────┐
│ EFE Operational Dashboard                       v0.8.0 | last 24h   │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐  ┌────────────┐    │
│  │ 99.97 %    │  │ 192 ms     │  │ 0 / 18     │  │ 1,247      │    │
│  │ Uptime 30d │  │ p99 E2E    │  │ Compliance │  │ Active     │    │
│  │            │  │ Mode-1 GPU │  │ FAIL       │  │ tenants    │    │
│  └────────────┘  └────────────┘  └────────────┘  └────────────┘    │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │ 105 SLO heat-map (by surface × status)                      │    │
│  │                                                              │    │
│  │  Mode-1   ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇                                │    │
│  │  Mode-2   ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇                               │    │
│  │  Mode-3   ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇                               │    │
│  │  AL       ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇  ← 1 FAIL (cost-per-label)    │    │
│  │  Stream   ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇                               │    │
│  │  Compl    ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇                               │    │
│  │  LATACT   ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇                               │    │
│  │           green=PASS  yellow=SKIP-DT  red=FAIL              │    │
│  └─────────────────────────────────────────────────────────────┘    │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │ Burn-rate matrix 4×3 (P1)                                   │    │
│  │                                                              │    │
│  │              1h       6h      24h     72h                   │    │
│  │  Critical   ███      ░░░     ░░░     ░░░                    │    │
│  │  Major      ░░░      ███     ░░░     ░░░                    │    │
│  │  Watch      ░░░      ░░░     ███     ███                    │    │
│  └─────────────────────────────────────────────────────────────┘    │
│                                                                     │
│  ┌──────────────────────────┐  ┌──────────────────────────┐         │
│  │ Active incidents         │  │ Last 5 releases          │         │
│  │  • None ✓                │  │  v0.8.0 — canary 100%    │         │
│  │                          │  │  v0.7.4 — canary 100%    │         │
│  └──────────────────────────┘  └──────────────────────────┘         │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

---

## § 9. Observability stack : Prom + Grafana + OTel + audit chain

### 9.1 Stack components

| Layer | Component | Purpose | Retention |
|---|---|---|---|
| **Metrics** | Prometheus (regional) | Scrape /metrics from pods | 15 days local |
| | Prometheus federation | Cross-region aggregation | 90 days central |
| | Grafana Cloud / self-hosted | Dashboards + alerts | — |
| **Traces** | OpenTelemetry SDK | Distributed tracing | — |
| | Tempo / Jaeger | Trace storage + UI | 7 days |
| **Logs** | structured tracing via `tracing` (Rust) | Application logs | — |
| | Loki / Datadog Logs | Log aggregation | 30 days |
| **Audit** | `audit_events` table + S3 cold | Tamper-evident | 7-10 years |

### 9.2 Métriques cardinales (= 12 instruments cross-sprint)

```
# Latence
efe_request_duration_seconds{mode, tenant, status}
efe_streaming_chunks_total{rpc, tenant, payload_kind}
efe_streaming_inter_chunk_seconds{rpc, tenant}

# Active learning
efe_al_gold_queue_size{tenant}
efe_al_retune_duration_seconds{catalog}
efe_al_drift_wasserstein_distance{tenant, catalog}

# Compliance
efe_audit_chain_breaks_total{tenant}
efe_compliance_evidence_pass_total{framework, control}
efe_breach_notification_seconds_p99{tenant}

# Saga + lifecycle
efe_saga_duration_seconds{saga_name, outcome}
efe_tenant_lifecycle_transitions_total{from, to}
efe_chargeback_tenant_cost_usd_total{tenant, resource_type}
```

### 9.3 Burn-rate 4×3 alerts (hérité Φ-D-LATACT)

Voir `alerts/p1_burn_4x3.yaml`. 12 cellules opérationnelles.

### 9.4 OTel traces strategy

Chaque request crée un **span racine** `efe.request.{mode}` avec 8-12 spans enfants :
```
efe.request.mode1
├── authz.verify
├── ratelimit.check
├── swarm.execute
│   ├── swarm.round.1
│   ├── swarm.round.2
│   ├── swarm.round.N
│   └── swarm.finalize
├── audit.write
├── chargeback.increment
└── stream.emit
```

**Échantillonnage** : 1 % traces par défaut ; 100 % traces si latency > p95 (= adaptive sampling via OTel processors).

---

## § 10. Incident response + on-call rotation

### 10.1 Sévérité matrix

| Sev | Définition | Response time | Page | Postmortem |
|---|---|---|---|---|
| **Sev1** | Total outage, data loss, security breach | ≤ 5 min ack | All on-call paged | ≤ 24h required |
| **Sev2** | SLO breach affecting > 10 % users | ≤ 15 min ack | Primary on-call | ≤ 48h required |
| **Sev3** | Single tenant impact, no SLO breach | ≤ 1h ack | Slack only | Optional |
| **Sev4** | Internal, no customer impact | next business day | None | Not required |

### 10.2 On-call rotation

```
Week 1 : Alice (primary)   + Bob (secondary)
Week 2 : Bob (primary)     + Carol (secondary)
Week 3 : Carol (primary)   + David (secondary)
Week 4 : David (primary)   + Alice (secondary)

Handoff : Friday 17:00 CET, sync 15 min on open incidents + risks.

Compensation : on-call premium = 10 % salary during rotation week.
```

### 10.3 PagerDuty integration

```yaml
# scripts/ops/pagerduty_services.yaml
services:
  - name: efe-prod-eu-critical
    integration: prometheus-alertmanager
    severity_mapping:
      critical: sev1
      major:    sev2
      watch:    sev3
    escalation_policy:
      - delay_minutes: 0
        targets: [on_call_primary]
      - delay_minutes: 15
        targets: [on_call_secondary]
      - delay_minutes: 30
        targets: [engineering_lead]
      - delay_minutes: 60
        targets: [cto]

  - name: efe-prod-eu-compliance
    integration: prometheus-alertmanager
    severity_mapping:
      critical: sev1
    escalation_policy:
      - delay_minutes: 0
        targets: [dpo, on_call_primary]
      - delay_minutes: 5
        targets: [legal_counsel]
```

### 10.4 Incident playbook (= 5 phases)

```
[Detection]
  Alert → PagerDuty page → primary on-call ack within 5 min

[Triage] (= phase critique pour novice)
  1. Open the alert link → runbook auto-loaded
  2. Run the triage commands listed at the top of the runbook
  3. Determine : is it Sev1/2/3 ? Affected tenants ?
  4. If unclear : declare Sev2 + page secondary, continue

[Containment]
  Follow the runbook's "Mitigation immédiate" section
  Common actions :
    - block_tenant : K8s NetworkPolicy
    - rotate_keys  : Key Vault rotation cmd
    - rollback     : helm rollback efe N-1
    - failover     : Cloudflare GeoDNS toggle

[Communication]
  Sev1 : status page update + customer email within 30 min
  Sev2 : status page update within 1h
  Internal Slack #efe-incidents (= auto-archived in audit chain)

[Resolution + Postmortem]
  When SLO recovers AND no further alerts for 1h :
    - declare resolved in PagerDuty
    - status page : "Resolved"
    - schedule postmortem within 48h
    - postmortem signed by primary on-call + engineering lead
```

### 10.5 Postmortem template

```markdown
# Postmortem — YYYYMMDD - <slug>

**Severity** : Sev1 / Sev2
**Duration** : start → recovery (= total minutes)
**Customer impact** : N tenants × M minutes degraded
**Author** : <primary_on_call>
**Reviewers** : <engineering_lead> + <dpo if compliance event>

## 1. Summary (= 3-4 sentences)

## 2. Timeline (= UTC timestamps)
- T+0    : alert fired
- T+3m   : primary ack
- T+8m   : root cause hypothesis
- T+22m  : mitigation applied
- T+25m  : SLO recovered

## 3. Root cause
(= no blame, factual)

## 4. Contributing factors
- (= ce qui a aggravé le problème)
- (= ce qui aurait pu réduire le blast radius)

## 5. What went well
- (= toujours mentionner pour l'apprentissage)

## 6. What went wrong
- (= sans blâme individuel)

## 7. Action items (= Linear-tracked)
| # | Item | Owner | Linear ticket | Due |

## 8. Prevention (= what we change permanently)
```

---

## § 11. Release pipeline : canary unifié 4 surfaces

### 11.1 Release flow

```
[Developer pushes PR]
  │
  ▼
[CI gate 1 : pin tests + linters]
  │ (= 200+ tests, 6 SLO suites, doc drift check)
  ▼
[CI gate 2 : master SLO consolidation]
  │ python3 consolidate_all_slos.py → 105 SLOs PASS or FAIL
  ▼
[Reviewer approves PR]
  │
  ▼
[Merge to main]
  │
  ▼
[Build artifacts] (= Rust binary, Docker image, Helm chart, Next.js build)
  │
  ▼
[Deploy to staging]
  │ helm upgrade --install efe-staging
  ▼
[Smoke tests staging] (= 8 realism tests)
  │
  ▼
[Deploy canary 10 % traffic to prod EU]
  │ helm upgrade --install efe-prod -f values-prod-eu.yaml
  │ Argo Rollouts or Flagger traffic split
  ▼
[Canary monitor 24h]
  │ Watch SLO burn-rate, compare to baseline
  │ If 4×3 burn-rate fires "critical" → auto-rollback
  ▼
[Promote canary to 100 %]
  │
  ▼
[Repeat for US then APAC (= staggered)]
  │
  ▼
[Audit chain entry : release.promoted]
```

### 11.2 Canary auto-rollback rules (= D-OPS-5)

```yaml
# flagger/canary-rules.yaml
analysis:
  interval: 1m
  threshold: 5     # fail count before rollback
  iterations: 1440 # 24h × 60 min
  metrics:
    - name: efe_request_error_rate_5m
      thresholdRange:
        max: 1.0
    - name: efe_request_duration_seconds_p99
      thresholdRange:
        max: 0.250
    - name: efe_streaming_close_total_cancelled_ratio_5m
      thresholdRange:
        max: 0.05
    - name: efe_compliance_audit_chain_breaks_total_1h
      thresholdRange:
        max: 0
  webhooks:
    - name: rollback-notify-slack
      url: https://hooks.slack.com/...
    - name: rollback-audit-chain
      url: http://audit-svc/v1/release-rolled-back
```

### 11.3 Release cadence

| Cadence | Cible | Trigger |
|---|---|---|
| **Daily** | Patch releases (= bugfix) | merged PR → auto-deploy staging |
| **Weekly** | Minor releases | Friday morning, all SLO green |
| **Monthly** | Major releases | strategic milestones, board demo |
| **Hotfix** | Sev1/Sev2 fix | < 4h after postmortem |

---

## § 12. Capacity planning + cost model + chargeback

### 12.1 Cost model par tenant (= chargeback granulaire)

```python
# scripts/ops/chargeback_compute.py
def compute_tenant_cost(tenant_id: str, period: str) -> dict:
    return {
        "gpu_seconds":      gpu_pool_used[tenant_id],      # $ per sec
        "llm_api_tokens":   llm_provider_tokens[tenant_id], # $ per token
        "pg_storage_gb":    pg_used_gb[tenant_id],         # $ per GB
        "redis_ops":        redis_ops[tenant_id],          # $ per K ops
        "s3_storage_gb":    s3_used_gb[tenant_id],         # $ per GB
        "bandwidth_gb":     bandwidth_egress[tenant_id],   # $ per GB
        "audit_chain_ops":  audit_writes[tenant_id],       # $ per K ops
    }

# Pricing rates (= per-region adjusted)
RATES = {
    "gpu_seconds":     0.00015,
    "llm_api_tokens":  0.00003,   # weighted avg across providers
    "pg_storage_gb":   0.10,
    "redis_ops":       0.00001,
    "s3_storage_gb":   0.025,
    "bandwidth_gb":    0.09,
    "audit_chain_ops": 0.000001,
}
```

### 12.2 Capacity model

| Resource | 1k tenants × small | 100 tenants × prod | Notes |
|---|---|---|---|
| GPU T4 | 2 nodes | 8 nodes | Spot OK for staging, on-demand prod |
| API pods | 6 replicas | 18 replicas | HPA based on req/sec |
| PG | 1 instance db.r6g.large | 1 db.r6g.4xlarge + read replicas | Per-tenant schema |
| Redis | 1 cache.r6g.large | 1 cache.r6g.2xlarge | Per-tenant key prefix |
| S3 storage | 100 GB | 5 TB | 7-year retention cold tier |
| Bandwidth | 100 GB/month | 5 TB/month | CDN-fronted reduces by 60 % |

### 12.3 Budget alerts (= SLO-OPS-COST-CONTROL)

```yaml
budget_alerts:
  per_tenant:
    - threshold_pct: 80
      action: email_tenant_admin
    - threshold_pct: 100
      action: email_tenant_admin + email_efe_csm
    - threshold_pct: 120
      action: throttle_tenant_to_50pct + email_tenant_admin + sev3_page
  global_efe:
    - threshold_usd_monthly: 50000
      action: page_cto
    - threshold_usd_monthly: 100000
      action: page_cto + cfo_email
```

---

## § 13. Compliance posture continue : conformity assessment automatisé

### 13.1 État continu de la posture (= dashboard live)

```
                              Compliance posture (live)
                              ┌─────────────────────┐
                              │  256 / 256 controls │
                              │  18 / 18 SLOs PASS  │
                              │  6 / 6 frameworks   │
                              └──────────┬──────────┘
                                         │
              ┌──────────┬───────────────┼───────────────┬──────────┐
              │          │               │               │          │
           GDPR        SOC 2          ISO 27001       AI Act     HIPAA      CCPA
          (47/47)     (58/58)         (87/87)         (38/38)   (18/18)    (8/8)
              │          │               │               │          │          │
           Daily       Daily           Daily           Daily      Daily      Daily
           evidence    evidence        evidence        evidence   evidence   evidence
           replay      replay          replay          replay     replay     replay
```

### 13.2 Annual conformity assessment automatisé

Daily job `conformity_assessment_check.py` :
1. Pour chaque control, vérifier que l'evidence script a passé hier.
2. Calculer le score posture global = `# PASS / # applicable`.
3. Si < 100 %, page DPO + log Sev3.
4. Mensuel : générer rapport PDF auto-signé HMAC pour audit externe.
5. Annuel : `efe-admin run-conformity-assessment` produit un dossier complet.

### 13.3 Trust center customer-facing

```
trust.efe-ai.com/
├── /                 (= overview, last_updated, n controls passing)
├── /gdpr             (= 47 controls + evidence access)
├── /soc2             (= 64 controls)
├── /iso27001         (= 93 controls)
├── /ai-act           (= 38 controls + DPIA samples)
├── /hipaa            (= 18 controls)
├── /ccpa             (= 8 controls)
├── /subprocessors    (= live list + 30-day notification archive)
├── /dpia/<tenant>    (= per-tenant DPIA, auth required)
├── /ropa/<tenant>    (= per-tenant ROPA, auth required)
├── /audit/incidents  (= public Sev1 incidents, 5-year archive)
└── /api              (= JSON endpoints for automated due diligence)
```

---

## § 14. Customer surfaces : portal + API + docs + status + trust

### 14.1 Customer-facing surfaces inventaire

| Surface | URL | Purpose | Tier |
|---|---|---|---|
| **Landing** | efe-ai.com | Marketing + waitlist | Public |
| **Portal** | app.efe-ai.com | Sandbox + POC + dashboard | All |
| **API gRPC** | api.efe-ai.com:443 | Mode-1/2/3 + admin | Paid |
| **Docs** | docs.efe-ai.com | API ref + tutorials + how-to | Public |
| **Status page** | status.efe-ai.com | Uptime + incidents | Public |
| **Trust center** | trust.efe-ai.com | Compliance + evidence | Public + auth gated |
| **Admin console** | admin.efe-ai.com | Internal ops dashboard | Internal only |

### 14.2 Status page architecture

```
Components on status.efe-ai.com :
  ✓ API EU             ← Operational
  ✓ API US             ← Operational
  ✓ API APAC           ← Operational
  ✓ Portal             ← Operational
  ✓ Audit chain        ← Operational
  ⚠ Mode-3 EU          ← Degraded performance (latency p99 elevated)
  ✓ Erasure cascade    ← Operational
  ✓ LLM providers      ← Operational

History (= last 90 days) :
  2026-05-14 14:23 UTC — Mode-3 EU latency elevated (resolved in 45 min)
  ...
```

Status page driven by Prometheus alerts + manual updates by on-call. Integrates with Statuspage.io or self-hosted Cachet.

---

## § 15. Internal tools : admin CLI + alert routing + runbook automation

### 15.1 efe-admin CLI (= déjà existant, étendu)

```bash
# Tenant ops
efe-admin tenant list
efe-admin tenant create --plan poc --region EU
efe-admin tenant get <tenant_id>
efe-admin tenant promote <tenant_id> --to production
efe-admin tenant suspend <tenant_id>
efe-admin tenant erase <tenant_id> --confirm

# Compliance
efe-admin compliance run-conformity-assessment
efe-admin compliance regenerate-dpia --tenant <tenant_id>
efe-admin compliance regenerate-ropa --tenant <tenant_id>
efe-admin compliance audit-chain-replay --days 30

# Incident
efe-admin incident create --sev 1 --description "..."
efe-admin incident postmortem --id <incident_id>

# Release
efe-admin release canary-status
efe-admin release rollback --version v0.7.4

# Capacity
efe-admin capacity report
efe-admin capacity scale-gpu --target 12

# Audit
efe-admin audit dump --tenant <tenant_id> --since 7d
efe-admin audit verify-chain
```

### 15.2 Alert routing logic

```python
# scripts/ops/alert_router.py
def route_alert(alert: PrometheusAlert) -> PagerDutyService:
    # Compliance alerts → DPO + on-call
    if alert.labels.get("category") == "compliance":
        return "efe-prod-eu-compliance"

    # Mode-3 high-risk catalog alerts → escalate to AI/ML lead
    if alert.labels.get("ai_act_risk") == "high":
        return "efe-prod-ml-lead"

    # GPU pool alerts → infra team
    if alert.labels.get("component") == "gpu_pool":
        return "efe-prod-infra"

    # Default
    return "efe-prod-eu-critical"
```

### 15.3 Runbook automation

Chaque alert PagerDuty inclut un lien direct vers le runbook + un bouton "Run triage commands" qui exécute les commandes du runbook dans une session SSH brokered :

```
PagerDuty alert "P1BurnCritical1h" :
  → opens https://runbooks.efe-ai.com/latact_pending_overdue
  → button "🚀 Run triage" exec :
      kubectl get pods -l app=efe-server --field-selector=status.phase!=Running
      kubectl logs -l app=efe-server --since=10m | grep ERROR | tail -50
      python3 scripts/ops/check_burn_rate.py --window 1h
  → outputs streamed to PagerDuty incident timeline
  → on-call reads + decides next action
```

---

## § 16. Hand-off downstream

### 16.1 M25 Agent Runtime (= next sprint)

Φ-D-OPS livre l'inférence + compliance + lifecycle. **M25** ajoute :
- Agent SDK Python + TypeScript (= consommer EFE depuis app cliente)
- Tool calling primitives (= MCP-like protocol)
- Memory store per-agent (= long-term tenant memory)
- Multi-agent orchestration (= K agents coopératifs)

Φ-D-OPS prerequisites : OK (saga + audit chain + tenant lifecycle ready).

### 16.2 M27 Commercial edge

- Billing self-service (Stripe / Adyen)
- Pricing optimization per tenant
- Quote builder for enterprise
- Sales pipeline automation (HubSpot / Pipedrive)

### 16.3 M28 Events/Streaming external

- Kafka egress for tenant data lakes
- Webhooks per tenant (= activation.*, breach.*, compliance.*)
- Public event schema registry

---

## § 17. Matrice complète des artefacts Φ-D-OPS (= 40)

| # | Artefact | Type | Sprint |
|---|---|---|---|
| 01 | Archi (ce document) | doc | D95 |
| 02 | Master SLO YAML `ops_master_slos.yaml` (= 18 SLOs) | config | D95 |
| 03 | `scripts/ops/consolidate_all_slos.py` | tool | D95 |
| 04 | `scripts/ops/master_gate_check.py` | tool | D95 |
| 05 | `scripts/ops/run_suite.sh` (= unified runner) | tool | D95 |
| 06 | Helm chart `helm/efe/` complete | infra | D95 |
| 07 | K8s Tenant CRD | infra | D96 |
| 08 | K8s Catalog CRD | infra | D96 |
| 09 | `migrations/00Y_tenant_lifecycle_states.sql` | sql | D96 |
| 10 | `scripts/ops/saga_provision_sandbox.py` (= bench) | bench | D96 |
| 11 | `scripts/ops/saga_provision_poc.py` (= bench) | bench | D96 |
| 12 | `scripts/ops/tenant_state_machine_audit.py` | bench | D96 |
| 13 | Master Grafana dashboard JSON | obs | D97 |
| 14 | Prometheus federation config | obs | D97 |
| 15 | `alerts/ops_master_burn.yaml` (= cross-surface) | obs | D97 |
| 16 | PagerDuty services config | obs | D97 |
| 17 | `scripts/ops/alert_router.py` | tool | D97 |
| 18 | `scripts/ops/runbook_auto_link.py` | tool | D97 |
| 19 | Postmortem template `docs/postmortems/_template.md` | doc | D97 |
| 20 | `scripts/ops/postmortem_velocity_audit.py` | bench | D97 |
| 21 | `scripts/ops/chargeback_compute.py` | tool | D98 |
| 22 | `scripts/ops/budget_alert_check.py` | bench | D98 |
| 23 | `scripts/ops/capacity_planner.py` | tool | D98 |
| 24 | `scripts/ops/cost_per_tenant_bench.py` | bench | D98 |
| 25 | Trust center scaffold `trust/index.html` | doc | D98 |
| 26 | `scripts/ops/trust_center_freshness_audit.py` | bench | D98 |
| 27 | Status page integration scaffold | doc | D98 |
| 28 | Conformity assessment automated job | tool | D98 |
| 29 | DR drill playbook + simulator | doc + tool | D99 |
| 30 | Multi-region failover bench | bench | D99 |
| 31 | `efe-admin` CLI extension (tenant + compliance + audit) | code | D99 |
| 32 | Master pin tests `test_ops_completion.py` | test | D99 |
| 33 | Workflow `ops-master-gate.yml` | ci | D99 |
| 34 | Workflow `ops-nightly.yml` | ci | D99 |
| 35 | Workflow `ops-dr-drill.yml` | ci | D99 |
| 36 | Runbook `ops_master_gate_fail.md` | doc | D99 |
| 37 | Runbook `ops_chargeback_overrun.md` | doc | D99 |
| 38 | Runbook `ops_postmortem_overdue.md` | doc | D99 |
| 39 | Runbook `ops_dr_drill_failure.md` | doc | D99 |
| 40 | DT-OPS-* table dans § 24 | doc | D99 |

---

## § 18. Plan d'implémentation D95 → D99

| Jour | Tâche | Artefacts | Parallel ? |
|---|---|---|---|
| **D95** | Master SLO YAML + consolidate + run_suite + Helm chart | 01-06 | Team A |
| **D96** | K8s CRDs + tenant lifecycle SQL + saga benches | 07-12 | Team A+B |
| **D97** | Observability (Grafana, Prometheus federation, PagerDuty) | 13-20 | Team C |
| **D98** | Chargeback + capacity + trust center + conformity | 21-28 | Team B+D |
| **D99** | DR drill + admin CLI + pin tests + workflows + runbooks | 29-40 | All teams |

### 18.1 Critical path

```
D95 (foundation) → D96 (lifecycle) → D97 (observability) → D98 (cost+trust) → D99 (resilience+release)
```

### 18.2 Team allocation

- **Team A (= 2 SREs)** : infra Helm + K8s + tenant lifecycle (D95-D96).
- **Team B (= 2 dev)** : sagas + chargeback + admin CLI (D96-D98).
- **Team C (= 1 SRE)** : observability stack (D97).
- **Team D (= 1 SRE + 1 dev)** : trust center + status + DR drill (D98-D99).

Sprint 5j × 6 contributors = 30 person-days.

---

## § 19. Risques + mitigations (24)

| # | Risque | Sévérité | Mitigation |
|---|---|---|---|
| R1 | Master gate trop strict (= false positive blocks releases) | HIGH | `--tolerate-skip-with-dt` flag + manual override doc |
| R2 | Helm chart drift entre régions | HIGH | values-prod-{eu,us,ap}.yaml validated by `helm lint` in CI |
| R3 | PagerDuty alert storm (= 100 alerts/min) | HIGH | Alertmanager grouping + inhibition rules |
| R4 | Saga state machine deadlock | CRIT | Timeout 30 min + force-kill via admin CLI |
| R5 | Chargeback computation drift vs reality | HIGH | Daily reconciliation + Stripe Audit Trail cross-check |
| R6 | Multi-cloud Terraform module fork | MED | Single common module + per-cloud overrides |
| R7 | DR drill cause real outage | CRIT | Run in pre-prod cluster monthly + prod quarterly |
| R8 | Trust center reveals sensitive info | HIGH | DPO review every page before publish |
| R9 | Status page becomes outdated | MED | Auto-update from Prometheus alerts + manual confirm |
| R10 | Audit chain HMAC key leak | CRIT | KMS-managed + 90-day rotation + tenant-isolated |
| R11 | Tenant CRD admission webhook flake | MED | Failure mode = open (deploy proceeds) + alert |
| R12 | Postmortem culture erodes | MED | Engineering lead reviews each PM + retros |
| R13 | On-call burnout | MED | Cap 2 weeks/month + compensate + handoffs |
| R14 | New SLO added without runbook | MED | Pin test : every SLO has linked runbook |
| R15 | Conformity assessment evidence stale | HIGH | Daily replay + alert if any control unset > 30 days |
| R16 | Budget alert spam | LOW | Tunable thresholds per tenant size |
| R17 | DR drill skipped quarter | HIGH | Calendar block + audit log |
| R18 | Vendor lock-in (= one cloud) | MED | Always Multi-cloud verified by `cloud_parity_bench` |
| R19 | OTel sampling biased toward fast traces | MED | Adaptive sampling per-quantile |
| R20 | K8s CRD schema drift | MED | OpenAPI schema versioned + migration playbook |
| R21 | Master dashboard cardinality explosion | MED | Federated Prometheus + recording rules |
| R22 | Customer success can't access trust center | LOW | Internal admin endpoint mirrors public + +1 auth |
| R23 | Helm rollback fails mid-release | HIGH | Argo Rollouts auto-rollback + manual `helm history` |
| R24 | Saga compensation step itself fails | CRIT | Compensation timeout + admin force-resolve runbook |

---

## § 20. Décisions clés (22)

| # | Décision | Rationale |
|---|---|---|
| D1 | Helm + Argo Rollouts for canary | Industry standard, GitOps-native |
| D2 | 3 regions active-active (EU/US/APAC) | Latency + residency + DR |
| D3 | Prometheus federation cross-cluster | Master dashboard requires aggregation |
| D4 | PagerDuty primary (not Opsgenie/VictorOps) | Mature, multi-channel, runbook links |
| D5 | Postmortem ≤ 48h (vs industry 1 week) | Faster learning loop |
| D6 | Chargeback metering live (not monthly) | Customer trust + real-time alerts |
| D7 | Trust center customer-facing (vs internal-only) | Sales accelerator |
| D8 | Master SLO 18 (= aligned with sprint count) | Easy mental model |
| D9 | DR drill quarterly (not annual) | Real-readiness, not paper |
| D10 | K8s CRDs (vs config files) | Declarative + GitOps |
| D11 | Tenant per-schema PG (vs per-row) | Strong isolation, easier erasure |
| D12 | Audit chain in PG (vs separate store) | One backup, one disaster recovery |
| D13 | Saga compensation idempotent | Retry safety |
| D14 | On-call rotation 1 week (vs 2 weeks) | Reduce burnout, more handoffs |
| D15 | Sev1 ack 5 min (vs 15 min industry) | Customer-promised |
| D16 | Master gate hard-fail (vs warn-only) | Tag releases must be clean |
| D17 | Runbook auto-link from PagerDuty | New SRE accelerator |
| D18 | OTel adaptive sampling | Quality over volume |
| D19 | Conformity assessment automated | Manual = compliance theater |
| D20 | Status page driven by Prometheus | Single source of truth |
| D21 | efe-admin Rust binary (not Python script) | Performance + reliability |
| D22 | 36 pin tests Φ-D-OPS | Aligned with cross-sprint pattern |

---

## § 21. Anti-régression (= 36 pin tests + 8 realism)

### 21.1 — Pin tests structurels (= 18)

| # | Test |
|---|---|
| 1 | `test_ops_master_slo_yaml_has_18_entries` |
| 2 | `test_ops_consolidate_script_exists_and_invokable` |
| 3 | `test_ops_master_gate_script_exists` |
| 4 | `test_ops_helm_chart_structure_valid` (= templates + values + Chart.yaml) |
| 5 | `test_ops_helm_values_3_regions_present` (eu, us, ap) |
| 6 | `test_ops_k8s_tenant_crd_present` |
| 7 | `test_ops_k8s_catalog_crd_present` |
| 8 | `test_ops_tenant_lifecycle_migration_present` |
| 9 | `test_ops_master_grafana_dashboard_json_valid` |
| 10 | `test_ops_alerts_master_burn_yaml_valid` |
| 11 | `test_ops_pagerduty_services_yaml_valid` |
| 12 | `test_ops_postmortem_template_present` |
| 13 | `test_ops_chargeback_compute_script_present` |
| 14 | `test_ops_trust_center_scaffold_present` |
| 15 | `test_ops_master_workflows_exist` (= 3) |
| 16 | `test_ops_master_runbooks_exist` (= 4) |
| 17 | `test_ops_dt_tickets_documented` (= 14) |
| 18 | `test_ops_efe_admin_extensions_present` |

### 21.2 — Pin tests sémantiques (= 18)

| # | Test |
|---|---|
| 19 | `test_ops_consolidate_aggregates_all_6_sprint_slos` |
| 20 | `test_ops_master_gate_fails_on_any_surface_fail` |
| 21 | `test_ops_master_gate_passes_when_all_skip_with_dt` |
| 22 | `test_ops_tenant_state_machine_12_states_valid` |
| 23 | `test_ops_saga_provision_sandbox_5_steps` |
| 24 | `test_ops_saga_provision_poc_8_steps` |
| 25 | `test_ops_chargeback_includes_all_resource_types` |
| 26 | `test_ops_capacity_planner_produces_per_tier_estimates` |
| 27 | `test_ops_alert_router_returns_compliance_service_for_compliance_alerts` |
| 28 | `test_ops_runbook_auto_link_resolves_alert_to_runbook` |
| 29 | `test_ops_postmortem_velocity_under_48h` |
| 30 | `test_ops_trust_center_freshness_under_7d` |
| 31 | `test_ops_conformity_assessment_emits_pdf` |
| 32 | `test_ops_dr_drill_simulator_failover_under_1h` |
| 33 | `test_ops_master_dashboard_includes_all_105_slos` |
| 34 | `test_ops_helm_lint_passes_all_regions` |
| 35 | `test_ops_budget_alert_fires_at_120pct` |
| 36 | `test_ops_efe_admin_tenant_erase_invokable` |

### 21.3 — Realism tests (= 8 sentinels)

| # | Test | Backend |
|---|---|---|
| R1 | `realism_ops_full_release_canary_24h` | Argo Rollouts staging |
| R2 | `realism_ops_dr_failover_eu_to_passive` | Live K8s + Cloudflare GeoDNS |
| R3 | `realism_ops_master_gate_against_real_prod_metrics` | Live Prometheus |
| R4 | `realism_ops_tenant_full_lifecycle_signup_to_erasure` | Live PG + saga |
| R5 | `realism_ops_pagerduty_alert_fires_under_load` | Real Prometheus alert |
| R6 | `realism_ops_chargeback_matches_aws_billing` | Real AWS Cost Explorer |
| R7 | `realism_ops_audit_chain_replay_full_year` | Real audit_events table |
| R8 | `realism_ops_conformity_assessment_against_auditor_pack` | Auditor partner |

---

## § 22. Critères d'acceptation (DoD à 100 %)

```text
Φ-D-OPS DoD =
   (18/18 master SLOs ACTIVE en CI sandbox + 16 PASS en prod)
 ∧ (40/40 artefacts livrés)
 ∧ (36/36 pin tests verts en < 10 s)
 ∧ (8/8 realism sentinels documentés, ≥ 3 réellement exécutés en pre-prod)
 ∧ (Master gate workflow `ops-master-gate.yml` green sur 3 runs consécutifs)
 ∧ (Helm chart deploys identically across AWS / GCP / Azure)
 ∧ (Tenant lifecycle 12 states fully implemented with sagas)
 ∧ (Master Grafana dashboard shows 105 SLOs in 1 page)
 ∧ (PagerDuty integration + on-call rotation documented)
 ∧ (Postmortem template + 1 sample postmortem in repo)
 ∧ (Chargeback compute + budget alerts wired)
 ∧ (Trust center scaffold live with 6 frameworks)
 ∧ (Status page live with 8+ components)
 ∧ (efe-admin CLI extended with 15+ subcommands)
 ∧ (1 successful DR drill simulator run with RTO ≤ 1h)
 ∧ (Aucune régression sur les 4 sprints précédents)
```

---

## § 23. Hand-off & dépendances downstream

- **M25 Agent Runtime** : consomme l'API stable + lifecycle + audit chain.
- **M27 Commercial edge** : ajoute Stripe billing + pricing optimization.
- **M28 Events/Streaming** : ajoute Kafka egress + webhooks per tenant.
- **M29 Data Rights** : intégré dans Compliance + Saga GDPR.
- **M30 Sales Enablement** : trust center + DPIA + ROPA exposés.

---

## § 24. Annexes (A → I)

- A — Mapping master SLO ↔ surface SLOs (= 18 → 105 traçabilité)
- B — Helm values reference complète (= 3 régions)
- C — K8s Tenant + Catalog CRD JSON schema
- D — PagerDuty escalation policies par service
- E — Chargeback rates v1 (= per-resource $/unit)
- F — Status page component model
- G — Trust center page templates
- H — Postmortem 3 examples (good, mediocre, exemplary)
- I — Liens : `EFE_AL_*`, `EFE_STREAM_*`, `EFE_COMP_*`, `EFE_LATACT_*`, `EFE_MODE2_*`, `EFE_MODE3_*`

---

## § 25. Defer Tickets `DT-OPS-*` (= 14 vrais SKIPs opposables)

Anti-Φ-D.A1 : tout SLO non testable en CI sandbox doit pointer vers un
défer-ticket nommé avec un blocker explicite.

| ID | SLO bloqué | Blocker | Workflow | Sortie |
|---|---|---|---|---|
| `DT-OPS-LIVE-MULTI-REGION` | SLO-OPS-FAILOVER | 3 régions K8s provisionnées | `ops-dr-drill.yml` | simulator PASS |
| `DT-OPS-LIVE-PAGERDUTY` | SLO-OPS-INCIDENT-TTACK | PagerDuty account + on-call rotation | N/A | mock PASS |
| `DT-OPS-LIVE-CHARGEBACK-AWS` | SLO-OPS-COST-CONTROL | AWS Cost Explorer integration | `ops-nightly.yml` | synthetic PASS |
| `DT-OPS-LIVE-TRUST-CENTER-CDN` | SLO-OPS-TRUST-CENTER | CDN-fronted trust.efe-ai.com | N/A | scaffold present |
| `DT-OPS-LIVE-STATUS-PAGE` | SLO-OPS-AVAILABILITY | Statuspage.io or Cachet deploy | N/A | manual config doc |
| `DT-OPS-LIVE-CONFORMITY-PDF` | SLO-OPS-COMPLIANCE | PDF generator wired to evidence | `ops-nightly.yml` | Markdown PASS |
| `DT-OPS-LIVE-HELM-3-CLOUD` | SLO-OPS-CLOUD-PARITY | Real K8s clusters on 3 clouds | N/A | helm lint PASS |
| `DT-OPS-LIVE-OTEL-COLLECTORS` | SLO-OPS-AVAILABILITY (= traces) | OTel collectors deployed | N/A | local config PASS |
| `DT-OPS-LIVE-FAILOVER-DRILL` | SLO-OPS-FAILOVER | Real production drill | `ops-dr-drill.yml` | simulator PASS |
| `DT-OPS-LIVE-PROMETHEUS-FED` | Master dashboard | Prom federation cross-cluster | N/A | local fed config PASS |
| `DT-OPS-LIVE-RUNBOOK-AUTO` | SLO-OPS-PAGE-RUNBOOK | PagerDuty webhook runbook injector | N/A | static link PASS |
| `DT-OPS-LIVE-BILLING-STRIPE` | Chargeback to invoice | Stripe Billing integration | N/A | scaffold doc |
| `DT-OPS-LIVE-DPO-AUDITOR-PACK` | Conformity assessment | External auditor onboarded | N/A | self-attestation PASS |
| `DT-OPS-LIVE-K8S-ADMISSION-AUDIT` | SLO-OPS-GITOPS | K8s admission webhook for kubectl edit | N/A | static audit log PASS |

### 25.1 Mapping inverse (= SLO → mode CI/prod)

| Master SLO | CI sandbox | Production |
|---|---|---|
| SLO-OPS-AVAILABILITY | mock 99.95 % | DT-OPS-LIVE-STATUS-PAGE |
| SLO-OPS-E2E-P99 | local bench | live Prometheus |
| SLO-OPS-COMPLIANCE | 18/18 SLOs PASS | DT-OPS-LIVE-CONFORMITY-PDF |
| SLO-OPS-ONBOARDING | synth bench | live portal traffic |
| SLO-OPS-CLOUD-PARITY | helm lint PASS | DT-OPS-LIVE-HELM-3-CLOUD |
| SLO-OPS-COST-CONTROL | bench mock | DT-OPS-LIVE-CHARGEBACK-AWS |
| SLO-OPS-RELEASE-SAFETY | flagger config valid | live canary 3 runs |
| SLO-OPS-AUDIT-INTEGRITY | self-test 100 events | live audit chain replay |
| SLO-OPS-INCIDENT-TTACK | mock alert | DT-OPS-LIVE-PAGERDUTY |
| SLO-OPS-POSTMORTEM-48H | static lint | live PM velocity |
| SLO-OPS-PAGE-RUNBOOK | static lint | DT-OPS-LIVE-RUNBOOK-AUTO |
| SLO-OPS-GITOPS | static lint | DT-OPS-LIVE-K8S-ADMISSION-AUDIT |
| SLO-OPS-ERASURE-30D | comp_erasure xref | live cascade |
| SLO-OPS-BREACH-72H | comp_breach xref | live drill |
| SLO-OPS-PINTESTS | 4 test suites green | live nightly green |
| SLO-OPS-DOC-DRIFT | static lint | live audit |
| SLO-OPS-TRUST-CENTER | scaffold present | DT-OPS-LIVE-TRUST-CENTER-CDN |
| SLO-OPS-FAILOVER | simulator PASS | DT-OPS-LIVE-FAILOVER-DRILL |

**Bilan attendu en CI** : 14 PASS + 4 SKIP-with-DT (= 0 faussement-PASS).
**Bilan attendu en production** : 18 PASS (= tous DT-OPS-* résolus).

---

## § 26. Synthèse opérationnelle

### 26.1 Ce que Φ-D-OPS livre, en 1 ligne

> *Le sprint Φ-D-OPS unifie 4 sprints, 105 SLOs, 200+ pin tests, 32
> workflows, 28 runbooks en un système où un SRE débutant à 3h du
> matin peut résoudre 80 % des alertes via runbook, où un auditeur
> peut sortir l'evidence des 256 controls en 2 minutes, où un nouveau
> tenant signe en 5 minutes, et où le release tag GA passe ou
> échoue de manière déterministe via un master gate.*

### 26.2 Ce qui change pour le client final

**Avant Φ-D-OPS** :
- Pas de portail
- Pas de status page
- Pas de trust center
- Pas de chargeback transparent
- Pas de DR drill
- Pas de master gate unifié

**Après Φ-D-OPS** :
- Portail Next.js → signup → first classify en 5 min
- Status page live avec history 90 jours
- Trust center exposant 256 controls en 6 frameworks
- Chargeback live + budget alerts 80 % / 100 % / 120 %
- DR drill quarterly avec RTO ≤ 1h
- Master gate 105 SLOs sur 1 dashboard

### 26.3 Ce qui change pour l'équipe interne

- **CTO / engineering** : peut taguer une release en confiance (= master gate)
- **DPO / compliance** : audit-ready à tout moment (= conformity assessment continu)
- **SRE / on-call** : runbook auto-linked depuis chaque alerte
- **Sales / CSM** : trust center + status page = arguments de vente
- **Finance** : chargeback temps réel + budget controls
- **Founders** : 1 dashboard maître = 1 vision en 30 secondes

---

**Fin du document Φ-D-OPS Operational Production Architecture.**
