# P2 Self-service — Architecture chirurgicale de l'**Activation opposable**

> **Statut** : architecture détaillée, prête à implémenter
> (jalons sur Φ-A → Φ-B du roadmap `PRODUCT_COMPLETION_ARCHITECTURE.md`).
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : ce document **étend** le sprint GLOSS
> (`docs/EFE_GO_LIVE_ONBOARDING_ARCHITECTURE.md`) en transformant le
> *self-service* d'un objectif logistique interne (« 30 min de
> SE-time ») en une **promesse commerciale opposable** mesurable
> par un tiers, sécurisée par SLO, et **défendable en SLA**. Il
> complète la doctrine "3 promesses" héritée de P1 :
> - **P1 Latence opposable** : un classify roundtrip ≤ 200 ms p99
>   (cf. `EFE_P1_SURFACE_ARCHITECTURE.md`).
> - **P2 Activation opposable (Self-service)** : un nouveau client
>   atteint le `first successful classify` en ≤ 30 min p90 **sans
>   intervention humaine côté Fournisseur** (ce document).
> - **P3 Auditabilité opposable** : voir
>   `docs/audits/` + chaîne HMAC D14-bis.
>
> **Pré-requis** : avoir lu
> [`PRODUCT_COMPLETION_ARCHITECTURE.md`](PRODUCT_COMPLETION_ARCHITECTURE.md)
> §1 (mission), §3 (contrat C-1 → C-20), §16 (SLO matrix),
> [`EFE_GO_LIVE_ONBOARDING_ARCHITECTURE.md`](EFE_GO_LIVE_ONBOARDING_ARCHITECTURE.md)
> (5 piliers GLOSS), et
> [`EFE_P1_SURFACE_ARCHITECTURE.md`](EFE_P1_SURFACE_ARCHITECTURE.md)
> (doctrine de surface contractuelle, exemple paradigmatique).

---

## Table des matières

1. [Mission & contrat opposable P2](#1-mission--contrat-opposable-p2)
2. [Décomposition canonique du parcours « time-to-X »](#2-décomposition-canonique-du-parcours--time-to-x-)
3. [Surface 1 — Code (CLI + Helm + Terraform + Portal + API)](#3-surface-1--code-cli--helm--terraform--portal--api)
4. [Surface 2 — Mesure : funnel d'activation + télémétrie](#4-surface-2--mesure--funnel-dactivation--télémétrie)
5. [Surface 3 — SLO & gating (CI + prod)](#5-surface-3--slo--gating-ci--prod)
6. [Surface 4 — Tests (8 niveaux d'assurance)](#6-surface-4--tests-8-niveaux-dassurance)
7. [Surface 5 — Défensive (10 garde-fous d'idempotence + rollback)](#7-surface-5--défensive-10-garde-fous-didempotence--rollback)
8. [Surface 6 — Documentation & expérience développeur (DX)](#8-surface-6--documentation--expérience-développeur-dx)
9. [Surface 7 — Contractuelle & opposabilité juridique](#9-surface-7--contractuelle--opposabilité-juridique)
10. [Surface 8 — Économique (TCO du self-service)](#10-surface-8--économique-tco-du-self-service)
11. [Matrice complète des artefacts P2](#11-matrice-complète-des-artefacts-p2)
12. [Plan d'implémentation (mapping sprints D19 → D26)](#12-plan-dimplémentation-mapping-sprints-d19--d26)
13. [Risques identifiés & mitigations (18 risques)](#13-risques-identifiés--mitigations-18-risques)
14. [Décisions de conception clés (17)](#14-décisions-de-conception-clés-17)
15. [Anti-régression & realism check P2](#15-anti-régression--realism-check-p2)
16. [Critères d'acceptation (DoD P2-Surface)](#16-critères-dacceptation-dod-p2-surface)
17. [Hand-off & dépendances downstream](#17-hand-off--dépendances-downstream)
18. [Annexes (A → G)](#18-annexes-a--g)

---

## 1. Mission & contrat opposable P2

### 1.1 Rappel et extension de la promesse

> **P2 — Activation opposable (Self-service)** : un **nouveau
> client**, partant d'une URL marketing et d'une carte de paiement
> (ou d'un compte SSO), atteint son **premier classify réussi
> en environnement de production** en ≤ 30 minutes p90, **sans
> intervention humaine** côté Fournisseur (= aucun ticket support,
> aucun appel SE, aucun ops manuel).

Cette promesse **étend** le sprint GLOSS (cf.
`EFE_GO_LIVE_ONBOARDING_ARCHITECTURE.md` §0.1) sur **trois axes** :

1. **Mesure** : on quantifie chaque étape du parcours (« funnel
   d'activation ») et on signe la mesure dans la chaîne d'audit
   D14-bis (= opposable).
2. **Contrat** : on transforme l'objectif logistique « 30 min
   SE-time » en clause SLA avec pénalité (= juridique).
3. **Échelle** : la promesse couvre **3 cibles d'opérateur** :
   - **trial sandbox** (5 min de signup à 1er classify, gratuit) ;
   - **POC tenant** (30 min de signup à 1er classify prod-shaped) ;
   - **production tenant** (24 h de signup à load 100 RPS sustained).

### 1.2 Le mot « opposable » appliqué au self-service

Comme pour P1, **opposable** signifie ici :

1. **Mesurable** par un tiers — l'opérateur enregistre chaque
   transition d'état (`signup`, `provisioned`, `seeded`,
   `first_classify_ok`, `first_classify_correct`) avec un
   timestamp signé.
2. **Reproductible** par un tiers — l'opérateur peut, sur sa
   propre infra, rejouer le parcours en ≤ 30 min depuis un
   environnement neuf (= laptop avec Docker + un fournisseur
   cloud accepté).
3. **Attesté** — chaque transition est append-only dans la chaîne
   D14-bis ; les rapports mensuels d'activation sont signés HMAC.
4. **Borne supérieure stricte** — tout dépassement déclenche
   `ACTIVATION_BURN_ALERT` (cf. § 5.3.3) + (sur les pénalités
   contractuelles, cf. § 9.3) un *trial extension* gratuit
   pour le client lésé.

### 1.3 Trois cibles temporelles et leurs justifications

| Cible | Temps p90 | Population | SLA pénalité |
|-------|----------:|------------|--------------|
| **Trial Sandbox** (T-T-Sandbox) | ≤ 5 min | URL marketing → 1er classify sur dataset jouet | crédit 7 j de trial gratuits |
| **POC Tenant** (T-T-POC) | ≤ 30 min | signup payé → 1er classify sur catalog client | crédit 100 € sur facture suivante |
| **Production Tenant** (T-T-Prod) | ≤ 24 h | signup payé → classify 100 RPS sustained 1 h | crédit 500 € sur facture suivante |

#### 1.3.1 Pourquoi 5 / 30 / 24h (et pas 1 / 60 / 72 h)

- **5 min trial** : c'est le seuil de patience documenté pour les
  produits PLG (cf. corpus *Stripe Atlas Activation Funnel
  Benchmarks 2024*). Au-delà, ~60 % des prospects abandonnent.
  Faisable car le sandbox n'a *pas* de phase d'infra
  provisioning (image Docker pré-cuite + DB partagée sandbox).
- **30 min POC** : choisi par GLOSS sur la base d'une mesure
  empirique du chemin manuel actuel (3h15 → 30 min = ÷ 6.5).
  Compatible avec le délai d'un meeting de 1 h (15 min slides,
  30 min POC live, 15 min Q&A).
- **24 h production** : couvre la migration DNS, l'attestation
  AI Act § 14, les KYC bancaires obligatoires sur certains
  pays (RGPD strict), et les approvals internes côté client.

### 1.4 Périmètre exact de la mesure

**T = 0** est défini selon la cible :

| Cible | Définition de T = 0 |
|-------|---------------------|
| Trial Sandbox | Premier `GET /trial/signup` du client sur le site marketing (= timestamp ingress NGINX) |
| POC Tenant | `POST /v1/tenants` avec carte validée (= card-authorized webhook Stripe) |
| Production Tenant | `POST /v1/tenants` avec plan ≥ Pro (= billing webhook) |

**T = fin** est :
- Sandbox / POC : premier `200 OK` retourné par `efe.v1.SwarmService.Execute`
  avec `verdict.label != null` et `verdict.confidence > 0.5` (= classify
  techniquement réussi).
- Production : 60 min cumulatif de trafic ≥ 100 RPS soutenu sans
  alerte SLO C-1 active.

**Sont inclus** :
- L'inscription, la validation email/SSO,
- Le KYC bancaire si applicable,
- La provision Cloud (Terraform apply + Helm install),
- La création du catalog initial (template ou seed JSONL),
- La génération des API keys + JWT,
- Le premier appel gRPC avec validation côté serveur.

**Sont exclus** (par convention contractuelle) :
- Délais d'approval interne côté client (DPO, sécurité, juridique),
- Délais de propagation DNS hors-DC (cap à 30 min, ajouté en
  hors-budget),
- Outages tiers (Stripe, AWS IAM, Auth0, …) — couverts par
  C-12 (availability), pas par P2.

### 1.5 Conditions « de référence » pour l'opposabilité

| Paramètre | Valeur contractuelle | Tolérance |
|-----------|----------------------|-----------|
| **Client device** | Mac/Linux/Windows + navigateur récent (Chrome 122+, Firefox 123+, Safari 17+) | 1 navigateur récent |
| **Connectivité** | ≥ 5 Mbps download, ≥ 1 Mbps upload | DC-local p99 < 100 ms RTT |
| **Cloud cible** | AWS us-east-1 / eu-west-1 / GCP europe-west1 / Azure francecentral | parmi les 4 |
| **Carte de paiement** | Visa / Mastercard / Amex émise dans pays SEPA + UK + US + CA | 30 pays principaux |
| **Plan demandé** | Free / Trial / POC / Pro / Enterprise (5 plans) | les 5 |
| **Catalog seed** | Soit template officiel `support_triage` / `fraud_alert` / `billing_escalation`, soit JSONL ≤ 1 000 cas | parmi les 3 ou JSONL ≤ 1 000 |

Toute déviation **suspend** l'opposabilité pour ce parcours (= la
session n'est pas comptée dans le burn).

---

## 2. Décomposition canonique du parcours « time-to-X »

### 2.1 Tableau de référence — POC Tenant 30 min (14 étapes)

> Ce tableau est la **source de vérité unique** de la
> décomposition. Chaque optimisation P2 doit citer la ligne du
> tableau qu'elle touche.

| # | Étape | p90 cible | p50 attendu | Surface code | Métrique |
|--:|-------|----------:|-----------:|-------------|----------|
| 1 | Landing → click "Start POC" | 30 s | 10 s | site marketing | `funnel_landing_to_click_s` |
| 2 | Signup form fill (email + entreprise + cas d'usage) | 90 s | 45 s | `portal/signup` Next.js | `funnel_signup_form_s` |
| 3 | Email verify (lien magique) | 60 s | 20 s | `auth-service::magic_link` | `funnel_email_verify_s` |
| 4 | Card / SSO authorization | 60 s | 30 s | Stripe Elements ou OIDC IdP | `funnel_payment_s` |
| 5 | Plan select + cloud region pick | 30 s | 15 s | `portal/plan` | `funnel_plan_select_s` |
| 6 | Cloud provisioning (Terraform apply) | 480 s (8 min) | 300 s | `terraform/aws|gcp|azure` | `funnel_terraform_apply_s` |
| 7 | Helm install (efe-stack umbrella) | 240 s (4 min) | 180 s | `charts/efe-stack` | `funnel_helm_install_s` |
| 8 | Smoke (`efe-admin tenant healthcheck`) | 30 s | 10 s | `efe-admin` | `funnel_healthcheck_s` |
| 9 | Catalog seed (template ou JSONL upload) | 120 s | 60 s | `efe-admin catalog seed` | `funnel_catalog_seed_s` |
| 10 | API key gen + JWT issuer setup | 30 s | 10 s | `efe-admin auth init` | `funnel_auth_init_s` |
| 11 | Premier classify quickstart (curl ou SDK Python ou portail) | 90 s | 30 s | `quickstart.sh` ou portal | `funnel_first_classify_s` |
| 12 | Verdict reçu + UI render | 5 s | 2 s | portal/result | `funnel_render_verdict_s` |
| 13 | Tutorial nudge (« voir audit chain », « inviter collaborateur ») | 60 s | 20 s | portail post-success | `funnel_post_success_s` |
| 14 | First "Aha moment" (verdict different from random) | 30 s | 10 s | portal/aha_panel | `funnel_aha_s` |
| — | **TOTAL budget p90** | **1 800 s = 30 min** | ~ 740 s (12 min) | — | `funnel_t_to_first_classify_s` |
| — | **Marge anti-tail (cap inclus)** | n/a (déjà absorbé dans cibles) | | | |

### 2.2 Variantes par cible

#### 2.2.1 Trial Sandbox 5 min

- Étapes 6 + 7 + 10 sont **absentes** (image Docker pré-cuite +
  DB partagée + API key auto-générée).
- Le sandbox tourne sur infra Fournisseur ; le client ne provisionne
  rien.
- T-T-Sandbox p90 cible = **5 min** = 300 s. Reste à payer :
  étapes 1+2+3+5+9+11+12 ≈ 250 s p90 + 50 s marge.

#### 2.2.2 Production Tenant 24 h

- Étapes 1-14 (= POC) + extension :
  - 15. Migration DNS personnalisé (TLS + CNAME) : 30 min cap.
  - 16. KYC bancaire si pays sensible : ≤ 4 h.
  - 17. Approval client interne (DPO, sécu) : ≤ 16 h.
  - 18. Premier `100 RPS sustained 60 min` validé : ≤ 90 min.

### 2.3 Profil « Provisioning is the long pole »

```
1. Landing→click               ▏  30 s
2. Signup form                 ▎  90 s
3. Email verify                ▏  60 s
4. Card / SSO                  ▏  60 s
5. Plan select                 ▏  30 s
6. Cloud provisioning          ████████████████████  480 s (27 %)
7. Helm install                ██████████  240 s (13 %)
8. Healthcheck                 ▏  30 s
9. Catalog seed                ███  120 s
10. Auth init                  ▏  30 s
11. First classify             ▎  90 s
12. Render                     ▏   5 s
13. Tutorial nudge             ▏  60 s
14. Aha moment                 ▏  30 s
                              ─────────────────────────
TOTAL                                                  1 800 s
```

→ **40 % du budget est consommé par 2 étapes : Cloud provisioning
(27 %) + Helm install (13 %).** Tout effort d'optimisation P2 doit
prioritairement cibler ces deux étapes.

---

## 3. Surface 1 — Code (CLI + Helm + Terraform + Portal + API)

### 3.1 Arborescence des modules sur le chemin self-service

```
crates/
├── efe-admin/                     ← Pilier 1 GLOSS (CLI Rust)
│   └── src/cmd/
│       ├── tenant.rs               18 sous-commandes : create / list / show / delete / suspend / ...
│       ├── catalog.rs              seed / list / promote / freeze
│       ├── auth.rs                 init / rotate / list
│       └── healthcheck.rs          smoke E2E
├── efe-server/src/v1/
│   └── trial/                     ← NOUVEAU sprint P2-Foundation D19+
│       ├── signup.rs              POST /v1/trial/signup (anonymous user)
│       ├── sandbox.rs             POST /v1/trial/sandbox (provisioning shared infra)
│       └── upgrade.rs             POST /v1/trial/upgrade (trial → paid)
└── efe-server/src/v1/onboarding/
    └── funnel_events.rs           ← NOUVEAU : emission OTel events à chaque transition

charts/
├── efe-stack/                     ← Pilier 2 GLOSS (Helm umbrella)
│   ├── Chart.yaml
│   └── templates/
│       ├── _helpers.tpl
│       ├── deployment.yaml
│       ├── service.yaml
│       └── values.yaml.sample
└── (efe-server, efe-redis, efe-postgres, …)

terraform/                         ← Pilier 3 GLOSS
├── modules/
│   ├── aws/                       VPC + EKS + RDS + ElastiCache + ALB
│   ├── gcp/                       VPC + GKE + Cloud SQL + Memorystore + GLB
│   └── azure/                     VNet + AKS + PostgreSQL Flexible + Cache + AGW
└── examples/
    └── trial_30min/               profile prêt-à-l'emploi pour le POC 30 min

portal/                            ← Pilier 5 GLOSS (Next.js 14 SPA)
├── app/
│   ├── (auth)/signup/page.tsx     Étape 2
│   ├── (auth)/verify/page.tsx     Étape 3
│   ├── (auth)/plan/page.tsx       Étape 5
│   ├── (onboarding)/provision/page.tsx   Étape 6 + 7 (avec progress bar live)
│   ├── (onboarding)/catalog/page.tsx     Étape 9
│   ├── (onboarding)/first-call/page.tsx  Étape 11 + 12
│   └── (onboarding)/aha/page.tsx         Étape 14
└── lib/funnel_tracker.ts          ← NOUVEAU : émet les events funnel via WebSocket

docs/                              ← Pilier 4 GLOSS (Runbooks)
├── runbook/                       12 procédures
│   ├── tenant_provision.md
│   ├── tenant_suspend.md
│   ├── tenant_delete_gdpr_art17.md
│   └── ...
└── quickstart/                    ← NOUVEAU sprint P2-DX
    ├── 5_minute_sandbox.md
    ├── 30_minute_poc.md
    └── 24_hour_production.md
```

### 3.2 Le sentier critique de 30 min — déduplication des `apply`

Le sprint P2 introduit la notion de **layered apply** :

```
┌───────────────────────────────────────────────────────────────┐
│  Layer 0 : Cloud Account-shared (Fournisseur)                │
│    - 1× Cosign trust root, ECR/GAR/ACR organism, IAM roles   │
│    Provisionned UNE FOIS par cloud par mois (pas par tenant) │
└───────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌───────────────────────────────────────────────────────────────┐
│  Layer 1 : Tenant-VPC                                         │
│    VPC + 3 AZ + NAT GW + endpoints VPC                        │
│    Provisionné UNE FOIS par tenant, à la création            │
│    Wall-clock : ~ 3 min sur AWS, ~ 5 min sur GCP, ~ 4 min Az  │
└───────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌───────────────────────────────────────────────────────────────┐
│  Layer 2 : Tenant-Compute                                     │
│    EKS/GKE/AKS managed control plane + 2 node groups         │
│    Provisionné UNE FOIS par tenant. AWS : 8 min, GCP : 4 min  │
│    Optimisation : node group `efe-warm-pool` pré-démarré      │
│      (= 0 nodes scaled, mais le pool est défini)              │
└───────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌───────────────────────────────────────────────────────────────┐
│  Layer 3 : Tenant-Data                                        │
│    RDS Postgres + ElastiCache Redis                          │
│    Provisionné UNE FOIS par tenant. AWS : 7-10 min            │
│    Optimisation : Aurora Serverless v2 → cold start ≤ 60 s   │
└───────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌───────────────────────────────────────────────────────────────┐
│  Layer 4 : Tenant-EFE-stack (Helm)                            │
│    efe-server (3 réplicas), mcp-server, dashboards Grafana   │
│    Installé via `helm install`. Wall-clock ≤ 4 min.           │
└───────────────────────────────────────────────────────────────┘
```

**Optimisation critique** : Layer 2 et Layer 3 sont **parallélisés**
en Terraform (pas de dépendance entre eux). Gain ~ 5 min sur AWS.

**Optimisation v2** (sprint D25) : Layer 1 + Layer 2 sont
**pré-provisionnés** dans des "warm pools" (10 VPC+EKS pré-créés en
permanence par le Fournisseur). Le tenant en récupère un, lui change
les tags, et **élimine ~13 min** du parcours. T-T-POC tombe alors
à **~12 min** (vs 30). Cible v0.6.0.

### 3.3 Le portail Next.js — wireframe minimal

```
/                              landing (statique)
/signup                        formulaire 90 s
/verify                        email magic link
/plan                          plan + cloud region picker
/provision                     ← KEY PAGE :
                                 - SSE/WebSocket connexion vers backend
                                 - progress bar live des 4 layers Terraform
                                 - ETA mise à jour toutes les 5 s
                                 - bouton "abandon" qui trigger rollback
                                 - logs en streaming (Loki-style) si user clique "voir détails"
/onboarding/catalog            wizard 3 options : template / JSONL / upload
/onboarding/first-call         tabs : curl / Python / TS / Java
                                 Code samples copient-collables
                                 Live result panel à droite
/onboarding/aha                ← KEY PAGE :
                                 - montre verdict du 1er classify
                                 - compare au "random baseline"
                                 - dit explicitement "EFE > random by X %"
                                 - 1 CTA : "voir audit chain" / "inviter co-équipier"

/dashboard                     post-onboarding : navigate the tenant
/dashboard/usage               cost meter (lien avec P1 SLO billing)
/dashboard/audit               audit chain viewer
/dashboard/catalogs            CRUD catalog
/dashboard/api-keys            rotate / suspend
```

### 3.4 API REST publique P2 (15 endpoints)

```
# Trial
POST   /v1/trial/signup                    anonymous : crée user + send magic link
POST   /v1/trial/verify-email              {token}     : confirme email
POST   /v1/trial/sandbox                   :          : provision shared sandbox
POST   /v1/trial/sandbox/{id}/classify     :          : classify dans le sandbox
POST   /v1/trial/upgrade                   {plan}      : trial → POC/Pro
DELETE /v1/trial/sandbox/{id}              :          : destroy sandbox (GDPR art.17)

# POC / Production tenant
POST   /v1/tenants                         {plan, cloud, region, catalog_seed}
GET    /v1/tenants/{id}/onboarding-status  :          : full funnel state
GET    /v1/tenants/{id}/provisioning       :          : Terraform plan status (SSE)
POST   /v1/tenants/{id}/seed-catalog       {kind, payload}
POST   /v1/tenants/{id}/auth/api-keys      :          : generate key
GET    /v1/tenants/{id}/quickstart         :          : returns code samples
DELETE /v1/tenants/{id}                    :          : GDPR art.17 cascade delete
PUT    /v1/tenants/{id}/plan               {plan}      : upgrade plan
GET    /v1/tenants/{id}/usage              :          : current usage / quota
```

Conventions :
- Toutes les requêtes idempotentes (POST/PUT/DELETE) acceptent
  `Idempotency-Key` header (D18 idempotency table).
- Toutes les requêtes retournent `Operation-Id` header pour
  trace OTel cross-call.
- Erreurs structurées RFC 7807 (`application/problem+json`).

### 3.5 Doctrine API : « no silent fail »

Pour chaque endpoint P2, l'implémentation doit :

1. **Valider l'input avant tout side-effect** — pas de DB write
   suivi d'un rollback ; valider d'abord.
2. **Émettre l'événement funnel APRÈS commit** — sinon une session
   « provision started » mais non-commitée se compterait dans le
   burn de la mesure.
3. **Retourner un message d'erreur user-friendly** avec
   `remediation_hint` + `retry_after_ms` quand applicable.
4. **Tagger chaque erreur avec une `error_class`** parmi 14 codes
   fixes (cf. § 5.4) pour permettre l'analyse de cohorte.

---

## 4. Surface 2 — Mesure : funnel d'activation + télémétrie

### 4.1 Modèle de données du funnel

Chaque session d'activation crée une row dans la table
`onboarding_sessions` :

```sql
CREATE TABLE onboarding_sessions (
    session_id              UUID PRIMARY KEY,
    user_id                 UUID NOT NULL,
    tenant_id               UUID,                          -- NULL avant /v1/tenants
    plan_target             TEXT NOT NULL,                 -- 'sandbox' / 'poc' / 'pro' / 'enterprise'
    cloud_target            TEXT,                          -- 'aws' / 'gcp' / 'azure'
    region                  TEXT,

    -- Timestamps par étape (NULL = pas encore atteinte)
    started_at              TIMESTAMPTZ NOT NULL,          -- T = 0
    email_verified_at       TIMESTAMPTZ,
    payment_authorized_at   TIMESTAMPTZ,
    provisioning_started_at TIMESTAMPTZ,
    provisioning_done_at    TIMESTAMPTZ,
    helm_done_at            TIMESTAMPTZ,
    catalog_seeded_at       TIMESTAMPTZ,
    auth_initialized_at     TIMESTAMPTZ,
    first_classify_at       TIMESTAMPTZ,
    first_classify_correct_at TIMESTAMPTZ,                 -- = "aha moment"
    abandoned_at            TIMESTAMPTZ,                   -- si user dropoff
    abandoned_reason        TEXT,                          -- code structuré ; cf. § 5.4
    completed_at            TIMESTAMPTZ,                   -- = first_classify_correct_at OU abandoned_at

    -- Métriques agrégées (matérialisées pour reporting)
    duration_to_first_classify_seconds  INTEGER,
    duration_to_aha_seconds             INTEGER,
    rollbacks_count                     INTEGER NOT NULL DEFAULT 0,
    errors_count                        INTEGER NOT NULL DEFAULT 0,

    -- Audit chain
    audit_chain_head_at_start  BYTEA,                      -- snapshot
    audit_chain_head_at_end    BYTEA,
    audit_signature            BYTEA,                      -- HMAC sur le row entier
);

CREATE INDEX idx_onboarding_sessions_started_at ON onboarding_sessions(started_at);
CREATE INDEX idx_onboarding_sessions_plan_target ON onboarding_sessions(plan_target);
CREATE INDEX idx_onboarding_sessions_cloud_target ON onboarding_sessions(cloud_target);
```

Chaque étape est aussi tracée dans `onboarding_events` (insert-only
append log, table partitionnée par jour) :

```sql
CREATE TABLE onboarding_events (
    event_id            UUID PRIMARY KEY,
    session_id          UUID NOT NULL REFERENCES onboarding_sessions(session_id),
    event_kind          TEXT NOT NULL,    -- 'step.email_verified', 'step.helm_started', …
    occurred_at         TIMESTAMPTZ NOT NULL,
    metadata            JSONB,            -- payload spécifique étape
    correlation_id      UUID NOT NULL,    -- = OTel trace_id du request
    actor               TEXT NOT NULL,    -- 'user' / 'system' / 'terraform' / 'helm' / 'auth'
    error_class         TEXT,             -- cf. § 5.4 (= NULL si succès)
);
```

### 4.2 Métriques Prometheus minimales (10 instruments P2)

| # | Métrique | Type | Labels | Sens |
|--:|----------|------|--------|------|
| 1 | `efe_p2_funnel_started_total` | Counter | `plan_target,cloud` | sessions initiées (T=0) |
| 2 | `efe_p2_funnel_completed_total` | Counter | `plan_target,cloud,outcome` | sessions terminées (`outcome` ∈ `success` / `abandoned` / `error`) |
| 3 | `efe_p2_funnel_step_duration_seconds` | Histogram | `plan_target,step` | durée de chaque étape (14 valeurs `step`) |
| 4 | `efe_p2_funnel_t_to_first_classify_seconds` | Histogram | `plan_target,cloud` | T-T-Classify |
| 5 | `efe_p2_funnel_t_to_aha_seconds` | Histogram | `plan_target` | T-T-Aha (= au-delà du 1er classify, le 1er **bon** classify) |
| 6 | `efe_p2_funnel_drop_total` | Counter | `plan_target,step,reason` | abandons par étape × raison |
| 7 | `efe_p2_funnel_provisioning_seconds` | Histogram | `cloud,layer` | provisioning par cloud × layer (1..4) |
| 8 | `efe_p2_funnel_quickstart_choice_total` | Counter | `language` | popularité curl / Python / TS / Java |
| 9 | `efe_p2_funnel_rollback_total` | Counter | `step,reason` | rollbacks déclenchés |
| 10 | `efe_p2_funnel_active_sessions` | Gauge | `plan_target` | sessions en cours (= started, pas yet completed) |

Buckets canoniques du `_seconds` histogram pour P2 (latence
ordre de la minute) :

```
[1, 5, 10, 30, 60, 120, 300, 600, 1200, 1800, 3600, 7200, 14400]
= [1s, 5s, 10s, 30s, 1min, 2min, 5min, 10min, 20min, 30min, 1h, 2h, 4h]
```

### 4.3 Traces OTel — span layout d'une session

```
Span tree (1 trace = 1 onboarding session entière) :

efe.p2.onboarding              span racine, attrs={ session_id, plan, cloud, region }
├── efe.p2.signup              90 s
├── efe.p2.verify_email        60 s
├── efe.p2.authorize_payment   60 s
├── efe.p2.select_plan         30 s
├── efe.p2.provision           480 s
│   ├── efe.p2.tf_init         5 s
│   ├── efe.p2.tf_plan         15 s
│   ├── efe.p2.tf_apply        460 s     attrs={ layer ∈ {1,2,3} }
│   │   ├── efe.p2.tf_apply.layer_1   90 s
│   │   ├── efe.p2.tf_apply.layer_2   380 s   (long pole)
│   │   └── efe.p2.tf_apply.layer_3   330 s
│   └── efe.p2.helm_install    240 s
├── efe.p2.healthcheck         30 s
├── efe.p2.catalog_seed        120 s
├── efe.p2.auth_init           30 s
└── efe.p2.first_classify      95 s
    └── efe.p2.aha             30 s
```

Cette structure permet le « waterfall view » dans Tempo/Jaeger, qui
est extrêmement utile pour identifier les outliers d'une session
particulière (cf. § 7 du runbook `p2_burn_fast.md` à livrer en D20).

### 4.4 Le « funnel_tracker.ts » côté portail (Next.js)

Le portail émet chaque transition d'état au backend via une
connexion WebSocket dédiée (NE PAS utiliser de HTTP POST classique :
trop de latence pour les transitions sub-seconde) :

```typescript
// portal/lib/funnel_tracker.ts
import { useEffect } from 'react';

type FunnelStep =
  | 'landing_view'
  | 'signup_form_open'
  | 'signup_form_submit'
  | 'verify_email_open'
  | 'verify_email_clicked'
  | 'payment_form_open'
  | 'payment_authorized'
  | 'plan_selected'
  | 'provisioning_view'
  | 'provisioning_done'
  | 'catalog_view'
  | 'catalog_seeded'
  | 'first_call_view'
  | 'first_call_submit'
  | 'verdict_rendered'
  | 'aha_panel_view'
  | 'aha_clicked'
  | 'session_abandoned';

export function useFunnelTracker(sessionId: string) {
  const ws = useWebSocket(`/api/v1/funnel/${sessionId}`);
  return {
    track: (step: FunnelStep, metadata?: object) => {
      ws.send(JSON.stringify({
        step,
        occurred_at: new Date().toISOString(),
        metadata: metadata ?? {},
      }));
    },
  };
}
```

Le backend (`onboarding/funnel_events.rs`) reçoit, valide,
persiste dans `onboarding_events`, et met à jour la row de
`onboarding_sessions` (transaction unique).

### 4.5 Signature HMAC de la session (opposabilité)

Quand `onboarding_sessions.completed_at` est setté, le serveur :

1. Sérialise la row + l'array de `onboarding_events` correspondants
   en JSON canonique (clés triées, pas de whitespace).
2. Calcule un HMAC-SHA-256 avec la clé `EFE_AUDIT_HMAC_KEY` (la
   même que D14-bis).
3. Stocke le HMAC dans `audit_signature`.
4. Append un event dans la chaîne D14-bis avec
   `kind = "onboarding.session.completed"`.

→ Le rapport mensuel d'activation (cf. § 9.2) peut alors être
vérifié indépendamment par un tiers (cf. § 9.3).

---

## 5. Surface 3 — SLO & gating (CI + prod)

### 5.1 Inventaire des SLO P2

| ID | Nom | Comparator | Seuil | Source | État cible v1.0 |
|----|-----|:----------:|------:|--------|:--------------:|
| **SLO-P2-TTFC-1** | T-T-First-Classify p90 (POC) | lte | 1 800 s | `onboarding_sessions` query | **PASS** |
| **SLO-P2-TTFC-2** | T-T-First-Classify p90 (Sandbox) | lte | 300 s | idem | PASS |
| SLO-P2-TTFC-3 | T-T-First-Classify p90 (Production) | lte | 86 400 s | idem | PASS |
| **SLO-P2-AHA-1** | T-T-Aha p90 (POC) | lte | 2 100 s | idem | PASS |
| SLO-P2-AHA-2 | T-T-Aha p90 (Sandbox) | lte | 360 s | idem | PASS |
| SLO-P2-CONV-1 | Conversion rate Sandbox → POC | gte | 0.10 (10 %) | funnel agg | PASS |
| SLO-P2-CONV-2 | Conversion rate signup → completion | gte | 0.50 (50 %) | idem | PASS |
| SLO-P2-ABANDON-1 | Abandon rate à l'étape provision (étape 6) | lte | 0.05 (5 %) | idem | PASS |
| SLO-P2-ABANDON-2 | Abandon rate à l'étape catalog seed (étape 9) | lte | 0.10 (10 %) | idem | PASS |
| SLO-P2-CLOUD-PARITY | T-T-First-Classify écart max entre 3 clouds | lte | 1.5× | dérivé | PASS |
| SLO-P2-ROLLBACK | Rollback success rate sur DELETE tenant | gte | 1.0 (100 %) | telemetry | PASS |
| SLO-P2-DELETE-GDPR | T-T-Delete (art.17) p99 | lte | 900 s (15 min) | telemetry | PASS |
| SLO-P2-DOCS-DRIFT | Doc → code drift (snippets) | eq | 0 | nightly diff | PASS |
| **SLO-P2-BURN** | Burn rate composite | lte | 14.4× / 6× / 1× | dérivé multi-window | PASS |
| **SLO-P2-HUMAN-INTERVENTION** | % sessions ayant requis un appel SE ou ticket support | lte | 0.05 (5 %) | corrélation tickets ↔ sessions | PASS |

Total : **15 SLO P2**. C'est moins que P1 (17) car le funnel a moins
d'étages que la latence (12 → 14 mais SLOs racines suffisent).

### 5.2 Définitions opérationnelles clés

#### 5.2.1 SLO-P2-TTFC-1 (racine)

```yaml
slo_id: SLO-P2-TTFC-1
metric: |
  SELECT percentile_cont(0.90) WITHIN GROUP (
    ORDER BY duration_to_first_classify_seconds
  )
  FROM onboarding_sessions
  WHERE plan_target = 'poc'
    AND completed_at >= NOW() - INTERVAL '30 days'
    AND completed_at IS NOT NULL
    AND abandoned_at IS NULL
comparator: lte
threshold_seconds: 1800
window: 30d
error_budget: 0.10           # 90 % des POC ≤ 30 min ; 10 % toléré
contract_clause: "SLA §3.2"
```

#### 5.2.2 SLO-P2-CONV-1 (conversion)

Conversion = `count(plan_target='poc' WHERE coming_from='sandbox')
/ count(plan_target='sandbox' WHERE completed_at IS NOT NULL)`.
Mesuré sur fenêtre 90 jours glissants (volume nécessaire).

#### 5.2.3 SLO-P2-HUMAN-INTERVENTION (clé pour le « self-service »)

Joint chaque session avec la table `support_tickets` sur `tenant_id`
ou `user_email`. Si un ticket existe avec `created_at <=
session.completed_at + 7 days` et `category IN ('onboarding',
'provisioning', 'first_call')`, la session est marquée comme
ayant requis intervention.

Cible : ≤ 5 % des sessions. Au-dessus, on parle plus de « self-service »
mais de « assisted onboarding ».

#### 5.2.4 SLO-P2-DELETE-GDPR

Spécifique au droit à l'effacement (RGPD Art. 17). Quand un user
émet `DELETE /v1/tenants/{id}`, la suppression doit être complète
(DB + Helm uninstall + Terraform destroy + audit chain note)
en ≤ 15 min p99. Le SLO est gated par un test E2E qui mesure le
chemin complet.

### 5.3 Gating CI

#### 5.3.1 Pipeline CI E2E onboarding

Le workflow `.github/workflows/bench-p2-surface.yml` (à livrer
sprint D19) exécute :

```yaml
jobs:
  p2-e2e-sandbox-5min:
    runs-on: ubuntu-latest
    timeout-minutes: 15
    steps:
      - uses: actions/checkout@v4
      - name: Spin up k3d (1-node)
        run: k3d cluster create p2-test --wait
      - name: Helm install efe-stack (sandbox profile)
        run: helm install efe ./charts/efe-stack -f .ci/p2/sandbox.yaml
      - name: Run Playwright E2E (5-min sandbox flow)
        run: |
          cd portal
          npm ci
          npx playwright test e2e/sandbox_5min.spec.ts
      - name: Assert T-T-Sandbox ≤ 5 min
        run: |
          duration=$(jq '.t_to_first_classify_seconds' playwright-out.json)
          test "$duration" -le 300

  p2-e2e-poc-30min:
    # Plus lourd : provisionne sur AWS via Terraform (avec un compte CI
    # dédié + budget cap)
    if: vars.P2_E2E_AWS_ENABLED == 'true'
    runs-on: ubuntu-latest
    timeout-minutes: 45
    steps:
      - name: Apply terraform/examples/trial_30min
        run: |
          cd terraform/examples/trial_30min
          terraform init && terraform apply -auto-approve
      - name: Run Playwright E2E
        run: npx playwright test e2e/poc_30min.spec.ts
      - name: Teardown
        if: always()
        run: cd terraform/examples/trial_30min && terraform destroy -auto-approve
```

#### 5.3.2 Tests "doc → code drift"

`SLO-P2-DOCS-DRIFT` impose que **chaque snippet de code dans la
documentation quickstart soit testé**. Implémentation :

```python
# scripts/test_doc_snippets.py
def test_all_doc_snippets_run():
    for snippet in extract_code_snippets(["docs/quickstart/*.md"]):
        result = run_snippet_in_sandbox(snippet)
        assert result.exit_code == 0, f"Drift: {snippet.path}:{snippet.line}"
```

### 5.4 Codes d'erreur structurés (14 codes)

Chaque erreur du funnel a un `error_class` parmi 14 codes fixes :

| Code | Étape source | Description | Action user |
|------|--------------|-------------|-------------|
| `EMAIL_INVALID` | signup | Email mal formé | corriger |
| `EMAIL_DOMAIN_BLOCKED` | signup | Domaine bloqué (e.g. gmail consumer en plan enterprise) | utiliser email pro |
| `PAYMENT_DECLINED` | payment | Carte refusée | autre carte / contacter banque |
| `PAYMENT_3DS_REQUIRED` | payment | 3-D Secure requis | compléter |
| `CLOUD_QUOTA_EXCEEDED` | provision | Quota AWS/GCP/Azure dépassé (ex. EIP) | augmenter quota cloud |
| `CLOUD_REGION_UNAVAILABLE` | provision | Région down ou non-supportée | autre région |
| `CLOUD_AUTH_FAILED` | provision | OIDC trust mal configuré | reconfigurer SSO |
| `TF_APPLY_TIMEOUT` | provision | Terraform timeout (~ 15 min) | retry ; alerte ops |
| `HELM_INSTALL_FAILED` | helm | Image pull failed / template error | check logs ; runbook |
| `CATALOG_SEED_INVALID` | catalog | JSONL malformé ou trop gros (> 1 000 cas) | nettoyer JSONL |
| `CATALOG_TEMPLATE_MISSING` | catalog | Template demandé n'existe pas (changement nom) | autre template |
| `API_KEY_GEN_FAILED` | auth | KMS / Vault unreachable | retry |
| `FIRST_CALL_TIMEOUT` | first_call | gRPC call > 30 s | retry ; runbook |
| `INTERNAL` | * | Tout le reste | ticket support |

Cardinalité du counter `efe_p2_funnel_drop_total{reason}` = 14
valeurs distinctes. **Pas d'unbounded cardinality** comme avec un
free-form error message.

---

## 6. Surface 4 — Tests (8 niveaux d'assurance)

### 6.1 Pyramide P2

```
        ┌──────────────────────────┐
       8│  Chaos engineering tests │  1× / quarter
        ├──────────────────────────┤
       7│  Multi-cloud parity test │  1× / nightly
        ├──────────────────────────┤
       6│  E2E Playwright          │  1× / nightly
        ├──────────────────────────┤
       5│  Doc-snippet drift tests │  1× / commit
        ├──────────────────────────┤
       4│  Integration: tenant CRUD│  1× / PR
        ├──────────────────────────┤
       3│  Unit: funnel emitters   │  1× / save (LSP)
        ├──────────────────────────┤
       2│  Schema: API response    │  1× / save (LSP)
        ├──────────────────────────┤
       1│  Schema: OpenAPI vs code │  1× / commit
        └──────────────────────────┘
```

### 6.2 Tests E2E Playwright (niveau 6)

```typescript
// portal/e2e/poc_30min.spec.ts
import { test, expect } from '@playwright/test';

test('POC 30-min activation flow', async ({ page }) => {
  const startedAt = Date.now();

  // 1. Landing → signup
  await page.goto('https://app.efe.ai/');
  await page.click('text=Start POC');
  await page.fill('[name=email]', `test+${Date.now()}@example.com`);
  await page.fill('[name=company]', 'CI Acme');
  await page.click('text=Continue');

  // 2. Email verify (auto-loop via test mailbox)
  const verifyUrl = await fetchVerifyUrlFromMailbox();
  await page.goto(verifyUrl);

  // 3. Card (test card)
  await page.fill('[name=card_number]', '4242424242424242');
  await page.fill('[name=card_exp]', '12/34');
  await page.fill('[name=card_cvc]', '123');
  await page.click('text=Authorize');

  // 4. Plan
  await page.click('text=POC');
  await page.click('text=AWS us-east-1');
  await page.click('text=Provision');

  // 5. Wait for provisioning (with progress)
  await expect(page.locator('text=Provisioning complete')).toBeVisible({
    timeout: 15 * 60 * 1000,  // 15 min hard cap
  });

  // 6. Catalog seed
  await page.click('text=Use template');
  await page.click('text=support_triage');
  await page.click('text=Seed');

  // 7. First classify
  await page.click('text=Try first classify');
  await page.fill('textarea', 'Comment annuler mon abonnement ?');
  await page.click('text=Classify');
  await expect(page.locator('text=verdict.label')).toBeVisible({ timeout: 30000 });

  // 8. Aha
  await expect(page.locator('text=better than random')).toBeVisible();

  const elapsedMs = Date.now() - startedAt;
  await page.screenshot({ path: 'e2e-out.png', fullPage: true });
  expect(elapsedMs).toBeLessThan(30 * 60 * 1000);   // 30 min cap

  // Bonus : record the duration in funnel for SLO check
  await fetch('/api/v1/test/funnel-record', {
    method: 'POST',
    body: JSON.stringify({ duration_s: elapsedMs / 1000 }),
  });
});
```

### 6.3 Tests de parité multi-cloud (niveau 7)

```python
# scripts/tests/test_p2_multi_cloud_parity.py
import pytest

CLOUDS = ['aws', 'gcp', 'azure']

@pytest.mark.parametrize('cloud', CLOUDS)
def test_provisioning_completes_within_budget(cloud):
    duration = provision_poc_tenant(cloud, region='primary')
    assert duration <= 900     # 15 min cap

def test_t_to_first_classify_parity():
    """SLO-P2-CLOUD-PARITY : écart entre clouds ≤ 1.5×"""
    durations = {c: provision_poc_tenant(c) for c in CLOUDS}
    min_d, max_d = min(durations.values()), max(durations.values())
    assert max_d / min_d <= 1.5, f"{durations}"
```

### 6.4 Tests doc-snippet drift (niveau 5)

À chaque commit qui touche `docs/quickstart/**.md` ou
`docs/api/**.md`, le CI extrait tous les blocs `​```bash`,
`​```python`, `​```typescript`, `​```rust` et les exécute dans un
sandbox propre. Échec si :
- exit code ≠ 0,
- ou la sortie ne matche pas le bloc `​```text` qui suit (= expected output)
  si présent.

Ce gate empêche les régressions silencieuses où la doc montre un
API obsolète.

### 6.5 Tests d'idempotence (niveau 4)

Chaque endpoint mutant a un test d'idempotence :

```rust
#[tokio::test]
async fn create_tenant_is_idempotent() {
    let server = test_server().await;
    let payload = json!({ "plan": "poc", "cloud": "aws", "region": "us-east-1" });
    let idem = uuid::Uuid::new_v4();

    let r1 = server.post("/v1/tenants")
        .header("Idempotency-Key", idem.to_string())
        .json(&payload).send().await.unwrap();
    let r2 = server.post("/v1/tenants")
        .header("Idempotency-Key", idem.to_string())
        .json(&payload).send().await.unwrap();

    assert_eq!(r1.status(), 200);
    assert_eq!(r2.status(), 200);

    // Même tenant_id retourné — pas de duplication
    assert_eq!(r1.json::<Tenant>().await.unwrap().id,
               r2.json::<Tenant>().await.unwrap().id);
}
```

### 6.6 Chaos engineering (niveau 8)

| Scénario | Méthode | Critère PASS |
|----------|---------|--------------|
| Stripe down (webhook timeout) | mock interception | session marquée pending, retry auto, success ≤ 5 min |
| AWS quota EIP exceeded | mock IAM response | `EMAIL_NOTIFY` envoyé au ops + UI affiche `CLOUD_QUOTA_EXCEEDED` |
| Helm install échoue à 50 % | fail post-install hook | rollback complet (terraform destroy + DB cleanup) en ≤ 10 min |
| User abandonne entre étape 6 et 7 | ferme onglet | session marquée `abandoned`, ressources auto-destroy après 60 min |
| 100 signups en parallèle | load injection | tous complete sans deadlock, audit chain reste cohérente |
| Catalog seed JSONL 10 MB | upload | timeout user-friendly ≤ 30 s, message clair |
| OIDC token expiré pendant le flow | mock token | refresh transparent, pas de re-signup |
| Postal mailbox down (verify email) | mock SES | retry 3× puis fall-back SMS |

---

## 7. Surface 5 — Défensive (10 garde-fous d'idempotence + rollback)

### 7.1 Inventaire

| # | Nom | Localisation | Effet |
|--:|-----|--------------|-------|
| 1 | **Idempotency-Key** sur toutes mutations | tonic middleware (D18) | replay = same response |
| 2 | **`saga` pattern pour provisioning** | `v1/onboarding/saga.rs` | chaque step compensable |
| 3 | **Terraform `state lock` per-tenant** | `terraform/modules/*` | pas de race condition lors d'updates |
| 4 | **Helm `--atomic --wait`** | `efe-admin` helper | rollback auto si install fail |
| 5 | **Audit chain `pending` state** | D14-bis extension | provisioning entry n'est commit qu'au succès |
| 6 | **Session timeout 60 min** | `v1/onboarding/expire.rs` | force-cleanup des sessions abandonnées |
| 7 | **DB rows `soft-delete` 30j** | `tenant.deleted_at` | art.17 + recovery accidentel |
| 8 | **Quota check pré-flight** | `efe-admin tenant create` | early-fail avec `remediation_hint` |
| 9 | **Dry-run mode** | `efe-admin --dry-run` | preview Terraform plan sans apply |
| 10 | **`--from-scratch` reset** | `efe-admin tenant reset` | sub-second teardown + restart depuis étape N |

### 7.2 Détail 2 — Saga compensable

Le provisioning est décomposé en steps avec compensation explicite :

```rust
pub async fn run_provisioning_saga(req: ProvisionRequest)
    -> Result<TenantId, ProvisionError>
{
    let mut saga = SagaBuilder::new(req.session_id);

    // Step 1 : create DB row
    saga.add(
        "create_tenant_row",
        |ctx| async { tenant_repo.create(ctx).await },
        |ctx, tid| async { tenant_repo.delete_hard(tid).await }, // compensation
    );

    // Step 2 : terraform apply layer_1 (VPC)
    saga.add(
        "tf_apply_layer_1",
        |ctx| async { tf_runner.apply("layer_1", ctx).await },
        |ctx| async { tf_runner.destroy("layer_1", ctx).await },
    );

    // Step 3 : terraform apply layer_2 (compute)
    saga.add(
        "tf_apply_layer_2",
        |ctx| async { tf_runner.apply("layer_2", ctx).await },
        |ctx| async { tf_runner.destroy("layer_2", ctx).await },
    );

    // ... etc.

    // Step 9 : first_classify health check
    saga.add(
        "healthcheck",
        |ctx| async { health.smoke(ctx).await },
        |_, _| async { Ok(()) }, // pas de comp si already passed
    );

    saga.execute().await
}
```

Si **n'importe quelle** step échoue, les compensations des steps
précédentes sont exécutées en ordre inverse. Le user ne voit JAMAIS
de tenant à moitié provisionné. Le coût d'un rollback complet est
1.5× plus lent qu'un apply (les destroy sont souvent rapides),
soit ≤ 13 min sur AWS.

### 7.3 Détail 7 — Soft-delete 30 j

Quand un user fait `DELETE /v1/tenants/{id}` :

1. **t=0** : `tenant.deleted_at` set ; `tenant.status = 'deleting'`.
2. **t=0** : append audit chain `kind = "tenant.delete.requested"`.
3. **t=0..15min** : exécution du teardown saga (Helm uninstall →
   Terraform destroy → DB rows hard-delete sauf tenant lui-même).
4. **t=15min** : `tenant.status = 'soft_deleted'`. Audit chain
   `kind = "tenant.delete.softdone"`.
5. **t=30j** : daily cron supprime hard la row tenant + GDPR Art.
   17 satisfait. Audit chain `kind = "tenant.delete.harddone"`.

Pendant le délai 30j, l'admin Fournisseur peut **restaurer** le
tenant via `efe-admin tenant restore <id>` (rare mais utile contre
les suppressions accidentelles). Au-delà de 30j, restauration
**impossible** par contrat (clause Art. 17 du SLA).

---

## 8. Surface 6 — Documentation & expérience développeur (DX)

### 8.1 Le `docs/quickstart/` (3 documents)

#### 8.1.1 `5_minute_sandbox.md`

Public cible : développeur curieux qui veut tester en ≤ 5 min sans
carte. Structure :

1. **Open the sandbox** (lien direct, no login)
2. **Try one classify** (curl + Python + TS, copy-paste)
3. **See the verdict** (link vers UI)
4. **Compare to random** (le aha)
5. **Next step** : "extend with your data" OR "POC me up"

Cible LOC : 200 lignes max. Chaque snippet testé par
`test_doc_snippets.py`.

#### 8.1.2 `30_minute_poc.md`

Public cible : ingénieur d'un client qui prépare un POC. Structure :

1. **What you'll need** : carte, cloud account (1 region), 30 min.
2. **Signup** (lien + screenshot)
3. **Provisioning** (explication des layers Terraform, ETA)
4. **Catalog seed** (3 options : template / JSONL / upload)
5. **First call** (curl/Python/TS/Java avec output attendu)
6. **Verification** : "did my call appear in the audit chain ?"
7. **Cost meter** : "how much did I spend ? (P1 SLO)"
8. **What's next** : extend / invite collaborator / upgrade.

Cible LOC : 800 lignes max. Doit inclure 6+ screenshots.

#### 8.1.3 `24_hour_production.md`

Public cible : équipe ops d'un client qui ship en prod. Structure :

1. **Prerequisites** : DNS / TLS cert / KYC / DPO sign-off / SSO.
2. **Plan upgrade** (POC → Pro/Enterprise)
3. **Custom DNS** (CNAME + TLS provisioning)
4. **Production checklist** (15 items à valider).
5. **Load test guide** (`efe-bench --workload production-smoke`)
6. **Monitoring setup** (Grafana dashboards + Slack/PagerDuty webhooks)
7. **Rollback plan** : "comment revenir au POC si la prod casse"
8. **First incident** : how to RCA (lien runbooks).

Cible LOC : 1 500 lignes max. Inclut 10+ screenshots et un
diagramme architecture.

### 8.2 Métriques de qualité doc (gated en CI)

- `links_dead` = 0 (broken link checker en CI)
- `code_snippet_drift` = 0 (test_doc_snippets.py)
- `screenshot_drift` ≤ 3 (Playwright génère les screenshots dans
  les tests E2E, diff visuel via Percy ou Loki visual)
- `reading_time` ≤ cible (calculé par `mdReader` : 5min/30min/24h
  docs ne doivent pas dépasser respectivement 4 / 25 / 75 min de
  reading time selon `wpm = 200`).

### 8.3 SDK Quickstart auto-généré

Chaque SDK (Python, TS, Java, Go) embarque un sample :

```
clients/python/efe_tool/examples/quickstart.py
clients/typescript/efe-sdk/examples/quickstart.ts
clients/java/efe-sdk/examples/Quickstart.java
clients/go/efe-sdk/examples/quickstart.go
```

**Chacun** est testé dans CI (= `test_p2_quickstart_<lang>_runs`).
Chacun affiche le **même verdict** sur le **même input** (= contrat
de cohérence cross-SDK).

### 8.4 Erreurs UI — message + remediation

Chaque erreur (les 14 codes § 5.4) a dans le portail :

- **Titre** clair (e.g. "Quota AWS dépassé")
- **Description** factuelle (1-2 phrases)
- **Remediation_hint** : 1 action concrète (`Augmentez votre quota EIP
  AWS → support.aws.amazon.com/case/new`)
- **Self-service shortcut** : bouton qui essaie l'auto-remediation
  si possible (e.g. essayer une autre région, sauter une étape
  optionnelle).
- **Escalation** : "Si le problème persiste, ouvrir un ticket
  support" + lien `mailto:` pré-rempli avec `session_id` +
  `error_class` (= ticket triagé d'avance côté Fournisseur).

---

## 9. Surface 7 — Contractuelle & opposabilité juridique

### 9.1 Modèle de clause SLA P2 (extrait pour client commercial)

> ## §3.2 — Engagement d'activation libre-service (« P2 »)
>
> Le Fournisseur garantit que, lorsqu'un nouveau Client procède à
> l'inscription d'un Plan POC dans les Conditions de Référence
> définies en Annexe B (« Conditions d'activation »), le **90e
> centile du temps écoulé** entre l'autorisation de paiement et le
> **premier appel `Execute` retournant un verdict** sera inférieur
> ou égal à **trente (30) minutes**, sans intervention d'un
> représentant humain du Fournisseur.
>
> La mesure est consolidée sur **fenêtres mensuelles calendaires**.
> Le Fournisseur produit chaque mois un Rapport d'Activation signé
> HMAC-SHA-256 contenant : (i) le p50/p90/p99 mesuré par cible
> (Sandbox/POC/Production), (ii) le taux de conversion
> Sandbox → POC, (iii) le taux d'intervention humaine constaté,
> (iv) la liste des sessions ayant dépassé 30 min avec leur
> `error_class`.
>
> ### Pénalités
>
> En cas de **non-respect mensuel constaté** (p90 > 30 min sur le
> Mois calendaire) :
>
> | Dépassement mensuel | Pénalité |
> |---------------------|:--------:|
> | < 10 % au-dessus (i.e. p90 ∈ ]30, 33] min) | extension trial 7 j |
> | ∈ [10, 25] % au-dessus (= p90 ∈ ]33, 37.5] min) | crédit 100 € |
> | > 25 % au-dessus | crédit 500 € + revue jointe |
>
> Le crédit est appliqué automatiquement sur la facture du mois
> suivant ; il n'est jamais remboursé en espèces.

### 9.2 Annexe B type (conditions d'activation)

Liste des conditions matérielles + de configuration sous lesquelles
P2 est opposable (cf. § 1.5).

### 9.3 Procédure de constatation par tiers

Identique à la procédure P1 (cf. `EFE_P1_SURFACE_ARCHITECTURE.md`
§ 9.3) mais sur **les tables `onboarding_sessions` +
`onboarding_events`** au lieu de Prometheus histograms.

### 9.4 Outil `efe-attest activation`

Extension du binaire `efe-attest` (cf. P1 § 9.4) avec sub-command :

```bash
efe-attest activation verify \
    --baseline bench_baselines/v1.0.0/p2_slos.yaml \
    --reports activation-reports/2026-06.json \
    --audit-chain s3://efe-audit/2026-q3.chain \
    --human-tickets-export support_tickets.csv

[PASS] 2026-06 : p90_poc = 28.5 min, conversion = 0.32, intervention = 0.02
[PASS] cloud_parity = 1.12× (AWS 26m / GCP 29m / Azure 26m)
[PASS] delete_gdpr_p99 = 11.3 min
[FAIL] 2026-07 : p90_poc = 35.2 min (> 33 min), pénalité due : 100 €
```

---

## 10. Surface 8 — Économique (TCO du self-service)

### 10.1 Coût offensif (investissement P2)

| Composant | Description | Coût annuel ordre |
|-----------|-------------|------------------:|
| Trial sandbox infra | 5 nodes K8s partagés Fournisseur, ALB, RDS managed | ≈ 12 000 $ |
| CI E2E AWS | budget mensuel CI `vars.P2_E2E_AWS_ENABLED` (~30 runs/jour × $0.50) | ≈ 5 500 $ |
| Multi-cloud CI parity | idem GCP + Azure | ≈ 6 600 $ |
| Portal hosting (Next.js Vercel) | 1× pre-prod + 1× prod | ≈ 3 000 $ |
| Mailer (signup magic link) | Postmark/SES | ≈ 1 200 $ |
| Auth0 / Clerk tier (SSO) | 5 000 MAU | ≈ 6 000 $ |
| Engineering time | 1 FTE 6 mois sur P2 surface | ≈ 100 000 $ |
| Sous-total offensif | | **≈ 134 000 $** |

### 10.2 Coût défensif (sans P2)

Sans self-service mesurable, le Fournisseur paie :

| Coût | Description | Montant annuel |
|------|-------------|----------------|
| Onboarding manuel | 2 ing × 6 h × 50 tenants/an × 200 €/h | ≈ 120 000 $ |
| Churn dû à friction | ~ 15 % des trials abandonnés après 30 min | "perdu" — ≈ 300 k$ pipeline |
| Support tickets onboarding | 30 % des tickets entrants concernent l'onboarding | ≈ 40 000 $ |
| Sous-total défensif | | **≈ 460 000 $** |

### 10.3 ROI

**Économie nette annuelle** : 460 000 $ - 134 000 $ = **326 000 $**.
Le ROI sur le sprint P2 est ~2.4×, payback ≤ 5 mois.

---

## 11. Matrice complète des artefacts P2

> Liste exhaustive des artefacts requis pour que la promesse P2
> soit opposable.

| # | Artefact | État cible | Sprint livreur |
|--:|----------|:--:|:--:|
| 1 | `docs/EFE_P2_SELF_SERVICE_ARCHITECTURE.md` (ce doc) | ✅ | D19 |
| 2 | `crates/efe-server/src/v1/trial/{signup,sandbox,upgrade}.rs` | ✅ | D19 |
| 3 | `crates/efe-server/src/v1/onboarding/{funnel_events,saga,expire}.rs` | ✅ | D19 |
| 4 | `crates/efe-telemetry/src/metrics/p2_instruments.rs` (10 instruments) | ✅ | D19 |
| 5 | Migration SQL : `onboarding_sessions` + `onboarding_events` | ✅ | D19 |
| 6 | `efe-admin` 4 sous-commandes nouvelles : `tenant restore`, `tenant reset`, `dry-run`, `quickstart-bundle` | ✅ | D20 |
| 7 | Saga runtime + 9 compensations testées | ✅ | D20 |
| 8 | `portal/app/(onboarding)/**` pages Next.js | ✅ | D20-D21 |
| 9 | `portal/lib/funnel_tracker.ts` WebSocket | ✅ | D20 |
| 10 | `terraform/modules/{aws,gcp,azure}` layered (4 layers) | ✅ | D21-D22 |
| 11 | `terraform/examples/trial_30min/` profile | ✅ | D22 |
| 12 | `charts/efe-stack/values-{sandbox,poc,production}.yaml` | ✅ | D21 |
| 13 | `docs/quickstart/{5,30,24}_*.md` (3 quickstart docs) | ✅ | D22 |
| 14 | `clients/{python,typescript,java,go}/examples/quickstart.{py,ts,java,go}` (4 SDK samples) | ✅ | D22-D26 |
| 15 | Playwright E2E suite : `portal/e2e/{sandbox_5min,poc_30min,prod_24h}.spec.ts` | ✅ | D20 |
| 16 | `scripts/tests/test_doc_snippets.py` | ✅ | D19 |
| 17 | `scripts/tests/test_p2_multi_cloud_parity.py` | ✅ | D22 |
| 18 | `bench_baselines/v0.2.0..v1.0.0/p2_slos.yaml` | ✅ | D19-D42 |
| 19 | `eval/scripts/check_slo.py` étendu pour P2 SLOs YAML | ✅ | D20 |
| 20 | `.github/workflows/bench-p2-surface.yml` | ✅ | D19 |
| 21 | `deploy/observability/prometheus/rules.p2/{alerts,recording}.yaml` | ✅ | D19 |
| 22 | `deploy/observability/grafana/efe-p2-funnel.json` | ✅ | D19 |
| 23 | `docs/runbook/p2_burn_fast.md` | ✅ | D19 |
| 24 | `docs/runbook/p2_provision_failed.md` + 4 autres runbooks par étage | ✅ | D20 |
| 25 | `docs/runbook/p2_delete_gdpr_art17.md` | ✅ | D20 |
| 26 | `docs/sla/sla_template.md` §3.2 P2 clause | ✅ | D40 |
| 27 | `efe-attest activation` sub-command | ✅ | D40 (Φ-G) |
| 28 | 14 error_class codes documented + i18n FR/EN | ✅ | D21 |
| 29 | Warm pool de VPC+EKS (sprint optimisation) | ✅ | D26 (post-MVP) |
| 30 | Signage HMAC des sessions dans D14-bis audit chain | ✅ | D19 |

→ **30 artefacts**, alignement intentionnel sur P1 (= mêmes 8 surfaces,
même rigueur).

---

## 12. Plan d'implémentation (mapping sprints D19 → D26)

### 12.1 Vue d'ensemble

```
D19  ───── Sprint P2-Foundation ─────  Φ-A
           - Doc P2 surface (ce doc)
           - Migration DB onboarding_sessions + onboarding_events
           - efe-telemetry p2_instruments (10 histograms/counters)
           - efe-server::v1::trial + v1::onboarding skeleton
           - Prom alerts + Grafana dashboard
           - Runbook p2_burn_fast.md
           - First baseline v0.2.0/p2_slos.yaml
           - Workflow CI bench-p2-surface (sandbox E2E)

D20  ───── Sprint P2-Saga ─────  Φ-A
           - Saga runtime + 9 steps + compensations
           - efe-admin nouvelles sous-commandes
           - Portal pages (onboarding) Next.js
           - Tests d'idempotence
           - test_doc_snippets.py
           
D21  ───── Sprint P2-Cloud-Parity-1 ─────  Φ-B
           - Terraform AWS 4 layers
           - Helm values-{sandbox,poc,production}.yaml
           - 14 error_class codes i18n
           - E2E AWS POC 30-min passing

D22  ───── Sprint P2-Cloud-Parity-2 ─────  Φ-B
           - Terraform GCP 4 layers
           - test_p2_multi_cloud_parity.py
           - Quickstart docs 5/30/24
           - Cloud parity SLO PASS
           
D23-D25 ─── Φ-B suite : SDK + bundle réel ───
           - 4 quickstart SDK samples (Python, TS, Java, Go)
           - Bundle MiniLM réel
           
D26  ───── Sprint P2-WarmPool ─────  optimisation
           - Pré-provisionnés VPC+EKS warm pool
           - T-T-POC tombe ~30 min → ~12 min p90
           - SLO ratchet down

D40  ───── Sprint P2-Attestation ─────  Φ-G
           - efe-attest activation
           - Clause SLA §3.2
           - Premier audit tiers

D42  ───── GA v1.0.0 ─────
           - 15 SLO P2 PASS attestés
           - 30 artefacts P2 livrés
```

### 12.2 Détail D19 — Sprint P2-Foundation (9 j wallclock)

| Jour | Tâche |
|----:|-------|
| J1 | Écrire `docs/EFE_P2_SELF_SERVICE_ARCHITECTURE.md` (ce doc) |
| J2 | Migration SQL `onboarding_sessions` + `onboarding_events` + indices |
| J3 | Écrire `efe-telemetry::metrics::p2_instruments` (10 metrics + tests) |
| J4 | Écrire `efe-server::v1::onboarding::funnel_events` (endpoint WebSocket + persistance) |
| J5 | Écrire `efe-server::v1::trial::signup` + `sandbox` + `upgrade` (REST) |
| J6 | Écrire les alerts Prometheus + dashboard Grafana funnel |
| J7 | Écrire `runbook/p2_burn_fast.md` + `bench_baselines/v0.2.0/p2_slos.yaml` |
| J8 | Workflow CI `bench-p2-surface.yml` + `scripts/tests/test_doc_snippets.py` |
| J9 | Audit interne sprint + tag `v0.2.1-rc1` + freeze baseline initiale |

---

## 13. Risques identifiés & mitigations (18 risques)

| # | Risque | Sévérité | Probabilité | Mitigation |
|--:|--------|:--------:|:-----------:|------------|
| 1 | Terraform AWS apply lent à cause d'EKS managed | HIGH | HIGH | Warm pool D26 + parallèle layer 2/3 |
| 2 | Stripe webhook lag | MED | MED | retry exponentiel 5×, fall-back manual |
| 3 | Auth0 / Clerk lock-in | MED | LOW | abstraction `AuthProvider` trait |
| 4 | Email deliverability faible | HIGH | MED | dual-provider Postmark + SES |
| 5 | Quotas AWS/GCP/Azure cassent l'auto-provisioning | HIGH | HIGH | pre-flight quota check (gardefou #8) |
| 6 | Sandbox shared infra surchargée | MED | MED | rate-limit 100 sandboxes simultanés + queue |
| 7 | Drift doc-snippet vs code | MED | HIGH | `test_doc_snippets.py` en CI |
| 8 | Multi-cloud parity dérive | MED | HIGH | nightly parity test + SLO-P2-CLOUD-PARITY |
| 9 | Saga rollback foireux (compensation incomplete) | HIGH | MED | tests d'idempotence + 9 chaos scenarios |
| 10 | Coût trial sandbox explose | MED | MED | quota strict + auto-delete 60 min |
| 11 | Onboarding requiert humain (= P2 cassé) | HIGH | MED | SLO-P2-HUMAN-INTERVENTION + corrélation tickets |
| 12 | Portal Next.js break sur browser ancien | LOW | LOW | Playwright multi-browser dans CI |
| 13 | GDPR Art. 17 incomplet sur cascade delete | HIGH | LOW | E2E test "delete_then_verify_gone" |
| 14 | Audit chain HMAC drift | HIGH | LOW | hérité D14-bis qui est solide |
| 15 | Localisation FR/EN incomplète pour erreurs | LOW | HIGH | linter de complétude i18n en CI |
| 16 | Pré-provisionnés warm pool coûte plus que sauvegardé | LOW | LOW | métrique cost / saved-time, recalibrer |
| 17 | Onboarding sessions DB grossit indéfiniment | LOW | LOW | partition par jour + retention 365 j |
| 18 | Multi-tenant noisy neighbor dans sandbox | MED | MED | tenants isolés dans namespaces k8s distincts |

---

## 14. Décisions de conception clés (17)

| # | Décision | Alternative rejetée | Justification |
|--:|----------|---------------------|---------------|
| 1 | Cible **30 min POC** (vs 60 min) | 60 min plus safe | 30 min est le seuil "1 meeting" ; au-delà, prospect dropoff |
| 2 | **Funnel signé HMAC** (vs tracking marketing libre) | Mixpanel-style | Opposabilité juridique |
| 3 | **Saga compensable** (vs transaction DB unique) | DB transaction | Provisioning multi-cloud non-transactionnel |
| 4 | **WebSocket pour funnel events** (vs HTTP POST) | HTTP POST | Latence sub-seconde nécessaire pour UI live |
| 5 | **14 error_class codes structurés** (vs free-form) | free-form | Cardinality bounded + analytics |
| 6 | **Soft-delete 30 j** (vs hard-delete immédiat) | hard-delete | Recovery accidentel + audit retention |
| 7 | **Idempotency-Key obligatoire** sur mutations | facultatif | Retry safety |
| 8 | **Warm pool** (sprint D26) | provisioning à la demande | Coupe 50 % du temps d'activation |
| 9 | **3 quickstart docs** (sandbox/POC/prod) | doc unique | Charge cognitive adaptée à chaque cible |
| 10 | **Multi-cloud SLO de parité** (1.5×) | unique cloud | Évite biais cloud sur la mesure |
| 11 | **`SLO-P2-HUMAN-INTERVENTION` ≤ 5 %** | sans gating | Sans ça, "self-service" devient slogan vide |
| 12 | **`dry-run` flag obligatoire sur efe-admin write** | flag optionnel | Réduit incidents user |
| 13 | **Sandbox sur infra Fournisseur** | sandbox cloud user | Coupe étape provisioning (5 min cible) |
| 14 | **Quickstart code samples cross-SDK testés** | doc-only | Drift impossible |
| 15 | **Audit chain note pour chaque session** | log only | Opposabilité |
| 16 | **Portal Next.js** (vs server-rendered classic) | server-side | SSE/WebSocket live progress |
| 17 | **Test E2E nightly multi-cloud** (vs PR-time) | PR-time | Coût CI ~$25/day raisonnable |

---

## 15. Anti-régression & realism check P2

### 15.1 Anti-régression

Tous les SLO P2 sont gatés dans CI ; aucune régression admise en
main. Le baseline `bench_baselines/v0.2.0..v1.0.0/p2_slos.yaml`
historique frozen ; chaque version doit montrer un delta ≥ 0.

### 15.2 Realism check

- **Mesure** : doit être faite contre un cloud réel (pas un mock)
  pour `SLO-P2-TTFC-1` (POC), `SLO-P2-CLOUD-PARITY`.
- **Catalog seed** : doit être un template officiel ou un JSONL
  ≥ 100 cas (pas un dataset 1-case trivial).
- **Test E2E Playwright** : doit run en CI sur runner GitHub-hosted,
  pas en local seulement.
- **Test multi-cloud** : doit toucher les 3 clouds chaque nuit ;
  refus de baseline si 1 cloud manqué 7 jours d'affilée.

### 15.3 Sanity checks au démarrage

`efe-server` au démarrage :
1. Vérifie que la migration SQL `onboarding_sessions` a été
   appliquée.
2. Vérifie que la trial sandbox K8s est accessible (`kubectl
   get pods -n trial-sandbox`).
3. Vérifie que les 3 clouds Terraform modules ont un `version >=
   v1.0.0` taggé.
4. Émet `P2_BOOT_READINESS` log + gauge `efe_p2_boot_ready` = 1
   uniquement après ces 3 checks.

---

## 16. Critères d'acceptation (DoD P2-Surface)

### 16.1 Code

- [ ] 30 artefacts P2 livrés (§ 11).
- [ ] Migration SQL `onboarding_sessions` + `onboarding_events` appliquée.
- [ ] 10 métriques Prometheus P2 émises.
- [ ] Saga compensable avec 9 steps + tests d'idempotence GREEN.
- [ ] 14 error_class codes documentés + traduit FR/EN.
- [ ] Soft-delete 30 j fonctionnel + cron de hard-delete.

### 16.2 CI

- [ ] Job `bench-p2-surface` nightly avec Playwright + multi-cloud.
- [ ] 15 SLO P2 gatés dans `check_slo.py` (extension P1).
- [ ] Failure d'un SLO bloque la release tag.
- [ ] Artifact `playwright-out.json` uploadé chaque run.
- [ ] `test_doc_snippets.py` GREEN.

### 16.3 Mesure live

- [ ] p90 POC ≤ 1 800 s (SLO-P2-TTFC-1) mesuré sur baseline v1.0.0.
- [ ] Cloud parity ≤ 1.5× (SLO-P2-CLOUD-PARITY).
- [ ] Conversion sandbox → POC ≥ 10 % sur 90 j.
- [ ] Intervention humaine ≤ 5 % (SLO-P2-HUMAN-INTERVENTION).

### 16.4 Documentation

- [ ] `docs/EFE_P2_SELF_SERVICE_ARCHITECTURE.md` (ce doc).
- [ ] 3 quickstart `docs/quickstart/*.md`.
- [ ] `docs/runbook/p2_burn_fast.md` + 5 runbooks par étage.
- [ ] `docs/sla/sla_template.md` §3.2.

### 16.5 Contractuel

- [ ] Modèle clause SLA §3.2 validé juridiquement.
- [ ] Annexe B « conditions d'activation » signée.
- [ ] `efe-attest activation` publié + signé cosign.

### 16.6 Économique

- [ ] Coût annuel infra P2 ≤ 134 k$ documenté.
- [ ] ROI break-even ≤ 5 mois validé.

---

## 17. Hand-off & dépendances downstream

### 17.1 Dépendances entrantes

| Sprint | Livré | Ce que P2 réutilise |
|--------|-------|--------------------|
| D9 | Docker + Helm + load test | base déploy K8s |
| D14 | Postgres catalog | DB schema patterns |
| D14-bis | Audit chain HMAC | signature des sessions |
| D15 | OIDC tenant | SSO du portal |
| D16 | Embedder bundle | sandbox image pré-cuite |
| D17 | Backup + DR | restore après soft-delete |
| D18 | Idempotency table | Idempotency-Key middleware |
| GLOSS (D-onboarding) | CLI + Helm + Terraform + Portal | 5 piliers de base |

### 17.2 Dépendances sortantes

| Sprint downstream | Ce qu'il prend de P2 |
|-------------------|----------------------|
| Φ-C (D22-D26) | quickstart SDK avec bundle réel |
| Φ-G (D40-D42) | `efe-attest activation` + clause SLA |
| GA v1.0.0 | SLO P2 PASS attestés |

### 17.3 Risque ordre des sprints

P2-Cloud-Parity (D21+) dépend de l'existence des modules Terraform
GCP + Azure ; si l'effort glisse, on rétrograde P2 à AWS-only et
on suspend `SLO-P2-CLOUD-PARITY` jusqu'à parité complète.

---

## 18. Annexes (A → G)

### Annexe A — Liste exhaustive des 15 SLO P2

```yaml
# bench_baselines/v1.0.0/p2_slos.yaml
slos:
  - id: SLO-P2-TTFC-1
    description: "T-T-First-Classify p90 (POC)"
    quantile: 0.90
    comparator: lte
    threshold_seconds: 1800
    source: onboarding_sessions
  - id: SLO-P2-TTFC-2
    description: "T-T-First-Classify p90 (Sandbox)"
    quantile: 0.90
    comparator: lte
    threshold_seconds: 300
    source: onboarding_sessions
  # ... (les 13 autres dans le même style)
```

### Annexe B — Conditions d'activation (clause SLA)

Reproduit § 1.5 du doc.

### Annexe C — Glossaire P2

| Terme | Définition |
|-------|------------|
| **P2** | Promesse n°2 du contrat EFE : Activation opposable / Self-service |
| **TTFC** | Time-to-First-Classify |
| **T-T-Aha** | Time-to-Aha (= 1er bon classify) |
| **POC** | Proof-of-Concept tenant (cible 30 min) |
| **Sandbox** | Trial gratuit infra partagée (cible 5 min) |
| **Production** | Tenant payant prod (cible 24 h) |
| **Saga** | Pattern de transaction distribuée avec compensation |
| **Warm pool** | VPC+EKS pré-provisionnés pour réduire le temps d'activation |
| **Funnel event** | Transition d'état tracée dans `onboarding_events` |
| **error_class** | Code structuré parmi 14 valeurs (cf. § 5.4) |
| **Soft-delete** | Marquage `deleted_at` ; hard-delete différé 30 j |

### Annexe D — Calcul du burn rate

```
error_budget_monthly = 0.10  # 10 % des POCs peuvent dépasser 30 min
total_sessions_monthly ≈ 200 POC / mois (cible business)
allowed_above_30min = 200 × 0.10 = 20 / mois

burn_rate_1h_threshold = 14.4 ×
  → sessions_above_30min_in_1h ≥ 20 × (1 / (30*24)) × 14.4 = 0.4
  i.e. **dès 1 dépassement dans la dernière heure** sur un volume
  faible, on fire P2_BURN_FAST. Adapté à la cadence des activations.
```

### Annexe E — Mapping P2 → contrats C-1..C-20 + lettres A..E

| Contrat | Description | Couvert par |
|---------|-------------|-------------|
| C-14 | OIDC tenant onboarding | step 4 du funnel (SSO) |
| C-15 | Rate-limit per-tenant | gardefou #8 |
| C-20 | Documentation Quickstart < 5 min | étapes 1-5 du funnel + `5_minute_sandbox.md` |
| Lettre A (CLI one-shot) | Hardening dans GLOSS (60 % → 100 %) | `efe-admin` nouvelles commandes |
| Lettre E (Service backend) | Multi-tenant production-ready | toute la surface P2 |

### Annexe F — Procédure de constatation par tiers (§ 9.3 ext)

1. Le Client demande au Fournisseur l'accès au rapport mensuel
   d'activation + l'export de `onboarding_events` filtré sur ses
   sessions.
2. Le tiers exécute `efe-attest activation verify` (ou
   équivalent indépendant).
3. Le tiers re-calcule les p90/p99/conversion à partir de l'export.
4. Le tiers vérifie le HMAC chaîne D14-bis.
5. Si tout matche → atteste conformité. Sinon → la pénalité §3.2
   s'applique.

### Annexe G — Commandes de vérification rapide

```bash
# Métriques Prometheus actives ?
curl -s http://localhost:50051/metrics | grep '^efe_p2_'

# Funnel sessions du dernier mois
psql -c "SELECT plan_target,
                percentile_cont(0.90) WITHIN GROUP (ORDER BY duration_to_first_classify_seconds) as p90
         FROM onboarding_sessions
         WHERE completed_at >= NOW() - INTERVAL '30 days'
         GROUP BY plan_target;"

# E2E sandbox 5-min
cd portal && npx playwright test e2e/sandbox_5min.spec.ts

# Multi-cloud parity (nightly)
pytest scripts/tests/test_p2_multi_cloud_parity.py -v

# Doc-snippet drift
pytest scripts/tests/test_doc_snippets.py

# SLO P2 mensuel
python eval/scripts/check_slo.py \
    --baseline bench_baselines/v1.0.0/p2_slos.yaml \
    --measure target/activation_report_2026-06.json \
    --strict-skip

# Attestation tiers
efe-attest activation verify \
    --baseline bench_baselines/v1.0.0/p2_slos.yaml \
    --reports activation-reports/2026-06.json \
    --audit-chain s3://efe-audit/2026-q3.chain
```

---

— **fin du document** —
