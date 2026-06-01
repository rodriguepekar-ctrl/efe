# EFE GLOSS Sprint C — Architecture Complète (Portail Next.js + Terraform GCP/Azure + RB-07 à RB-12)

**Codename** : « GLOSS-C »
**Status** : Architecture v1.0 — design freeze
**Auteur** : Architecture EFE Core
**Date** : 2026-05-09
**Pré-requis** : Sprint A (D8 GA) + Sprint B (Helm + Terraform AWS + RB-01 à RB-06 GA) ; cumul ~3 100 tests GREEN, 90 fichiers Sprint B
**Successeur** : Release GA EFE 1.0 (M22 + GLOSS A+B+C complet)

---

## §0. Executive Summary

### 0.1 Mission

Sprint C clôture GLOSS — la suite « Go-Live & Onboarding Self-Service » — en livrant les **trois derniers piliers** opérationnels qui rendent le produit utilisable côté tenant, déployable sur les 3 hyperscalers, et résilient face aux 12 incidents-types les plus probables sur la première année :

1. **Portail Admin Next.js 14** : UI web pour le DPO et l'admin tenant, scope minimal mais complet (8 pages : dashboard, audit, reviews, compliance, keys, quotas, settings, login)
2. **Module Terraform GCP** : équivalent fonctionnel du module AWS (Sprint B) sur Google Cloud (GKE + Cloud SQL + Memorystore + KMS + GCS + Workload Identity)
3. **Module Terraform Azure** : équivalent fonctionnel sur Azure (AKS + Azure Database for PostgreSQL Flexible Server + Azure Cache for Redis + Key Vault + Blob + Workload Identity)
4. **6 Runbooks RB-07 à RB-12** : latency, migration failure, review queue saturée, TLS cert expiry, LLM quota, Postgres failover

Sprint C **ne livre PAS** :
- ❌ Multi-region active-active (réservé M25)
- ❌ DPO export / fine-tuning (réservé M23)
- ❌ Eval harness LLM (réservé M24)
- ❌ Sub-chart `portal-admin` Helm code complet : seul le **template skeleton** est dans Sprint B (sub-chart `portal-admin/` avec `enabled: false` par défaut). Sprint C **active** ce sub-chart en livrant l'image Docker + le code Next.js.

### 0.2 Quatre piliers et leurs interactions

```
┌──────────────────────────────────────────────────────────────────────┐
│                    APRÈS GLOSS-C (Release v1.0 GA)                   │
│                                                                       │
│  Tenant DPO / admin                                                  │
│       │                                                               │
│       │ HTTPS (TLS 1.3)                                              │
│       ▼                                                               │
│  ┌──────────────────────┐                                            │
│  │  Portail Next.js 14  │  ← Pilier 1                                │
│  │  - SSO OIDC          │                                            │
│  │  - 8 pages           │                                            │
│  │  - tRPC + Server     │                                            │
│  │    Components        │                                            │
│  └─────────┬────────────┘                                            │
│            │                                                          │
│            │ tRPC (server-side) + gRPC-web (server-side)             │
│            ▼                                                          │
│  ┌──────────────────────────┐    ┌──────────────────────────┐       │
│  │  efe-server gRPC :50051  │←───┤  mcp-server HTTP :8080   │       │
│  └──────────────────────────┘    └──────────────────────────┘       │
│       (Sprint A+B unchanged)         (Sprint A+B unchanged)          │
│                                                                       │
│  ─── Provisionning infrastructure (Pilier 2-3) ────────────────────  │
│                                                                       │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐               │
│  │ Terraform   │    │ Terraform   │    │ Terraform   │               │
│  │ AWS (S-B)   │    │ GCP (S-C)   │    │ Azure (S-C) │               │
│  │ ──────────  │    │ ──────────  │    │ ──────────  │               │
│  │ EKS+RDS+    │    │ GKE+CloudSQL│    │ AKS+Postgres│               │
│  │ ElastiCache │    │ +Memorystore│    │ Flexible+   │               │
│  │ +KMS+S3     │    │ +KMS+GCS    │    │ Cache+KV+   │               │
│  │ +IRSA       │    │ +WI         │    │ Blob+WI     │               │
│  └─────────────┘    └─────────────┘    └─────────────┘               │
│                                                                       │
│  ─── Ops resilience (Pilier 4) ────────────────────────────────────  │
│                                                                       │
│  Runbooks 12 incidents-types : RB-01..RB-12                          │
│  - RB-01..06 : Sprint B (DB full, breaker, audit, BYOK, budget,      │
│                          crash-loop)                                  │
│  - RB-07..12 : Sprint C (latency, migration, review queue, TLS,      │
│                          LLM quota, PG failover)                     │
└──────────────────────────────────────────────────────────────────────┘
```

### 0.3 Scope synthétique

| Pilier | Output | Effort | LOC |
|--------|--------|--------|-----|
| Portail Next.js 14 (8 pages + tRPC + auth + tests) | TS / TSX | 5 j | ~6 200 |
| Module Terraform GCP | HCL + tests Go | 3 j | ~1 700 |
| Module Terraform Azure | HCL + tests Go | 3 j | ~1 800 |
| 6 Runbooks RB-07 à RB-12 | Markdown + scripts validation | 1.5 j | ~2 500 |
| Helm portal-admin sub-chart activation (image + values) | Update + ConfigMap | 0.5 j | ~150 |
| Tests Playwright + Vitest + terratest GCP/Azure | Tests | 2 j | ~3 200 |
| **Total Sprint C** | | **15 j** | **~15 550** |

### 0.4 Critères d'acceptation Sprint C (DoD synthèse)

1. ✅ Portail Next.js déployé sur k3d local + cluster cloud, SSO OIDC fonctionnel
2. ✅ DPO peut se connecter, voir le dashboard, attest compliance, créer une API key, voir un audit row + son concept_stack Mode-3
3. ✅ DPO peut approve/reject un review pending Mode-3 ; action audit-loggée
4. ✅ `terraform apply` GCP réussit en ≤ 15 min sur projet sandbox GCP
5. ✅ `terraform apply` Azure réussit en ≤ 18 min sur subscription sandbox Azure
6. ✅ 145 tests E2E Playwright (3 navigateurs : Chromium, Firefox, WebKit)
7. ✅ 110 tests unit Vitest
8. ✅ 8 tests terratest GCP + 8 tests terratest Azure
9. ✅ 6 runbooks RB-07 à RB-12 publiés avec procédures de validation reproductibles
10. ✅ 0 régression Sprint A+B (helm chart, tests Python, tests Rust restent GREEN)
11. ✅ Compliance preserved (CSP strict, audit_ui log, RBAC tenant_id enforcement, no cross-tenant leak)

### 0.5 Garanties cardinales transversales

- **Default OFF** : Sprint C livre le code mais le déploiement portal-admin reste **opt-in** via `helm install --set portal-admin.enabled=true`
- **0 modification M16-M22 ni Sprint A+B** : portail consomme uniquement les outils mcp-server existants (M16-M22 + 16 outils D8) ; aucune nouvelle migration DB
- **Multi-tenant strict** : RBAC enforcé au niveau tRPC (claim `tenants` du JWT IdP) ; pas d'accès cross-tenant possible
- **GDPR Art. 22 right-to-explanation** : page `/audit/[id]` pour Mode-3 expose le `concept_stack_jsonb` complet avec turn-by-turn breakdown

---

## §1. Contexte & justification

### 1.1 Où Sprint C s'inscrit dans la roadmap

| Sprint | Output | Statut | Tests cumul |
|--------|--------|--------|-------------|
| Sprint A (D8) | CLI `efe-admin` + 16 outils mcp-server + migration 0013 | ✅ GA | 3 040 → 3 100 |
| Sprint B (Helm/TF AWS/Runbooks) | Helm umbrella 6 sub-charts + Terraform AWS + RB-01..06 | ✅ GA | 3 100 → 3 100 + 90 fichiers infra |
| **Sprint C (Portail/TF GCP+Azure/Runbooks)** | Portail Next.js + Terraform GCP/Azure + RB-07..12 | 🚧 **en cours** | → 3 100 + ~290 nouveaux tests |
| Release v1.0 GA | Tag git, signature Sigstore, distribution | ⏳ post Sprint C | — |

### 1.2 Pourquoi Next.js 14 plutôt que React SPA + Vite

**Comparaison choix UI** :

| Choix | Pour | Contre | Verdict |
|-------|------|--------|---------|
| React SPA (Vite) | Build rapide, simple | Pas de SSR / RSC ; bundle JS plus gros ; SEO-unfriendly (peu pertinent pour un portail interne mais reste un + pour les liens internes) | ❌ |
| Next.js 14 App Router | RSC (Server Components) → moins de JS client ; cache HTTP natif ; tRPC + Server Actions intégrés ; Middleware Edge | Courbe d'apprentissage légèrement plus longue | ✅ |
| Remix | Bonne UX dev, full-stack | Communauté plus petite, écosystème tRPC moins mature | ❌ |
| Astro + Solid | Très performant | Moins commun en SaaS B2B, peu d'écosystème enterprise | ❌ |

**Décision** : Next.js 14 App Router avec Server Components. Justifications :
1. **RSC streaming** : panel `/dashboard` charge les KPI server-side (cache 30s) avec progressive HTML ; UX perçue ≤ 1.5 s TTI même sur connexions lentes
2. **tRPC v11** intégré natif → typage end-to-end TS depuis client jusqu'à backend Postgres (via Pydantic mcp-server)
3. **Middleware Edge** : auth check sub-millisecond avant le rendering, évite hydration costs
4. **Supply chain** : équipe EFE majoritairement TS, courbe = 0
5. **Production-ready** : déjà utilisé par Vercel, Netflix, TikTok pour des scaling 10⁹+ requests/jour

### 1.3 Pourquoi modules Terraform séparés GCP/Azure (vs unifié multi-cloud)

Sprint B a déjà tranché le débat avec AWS (cf. § 1.4 doc Sprint B). Sprint C reproduit la même décision pour GCP/Azure :

- **Module séparé** = 1 fichier `main.tf` HCL idiomatique, pas d'abstraction fuyante
- **Multi-cloud unifié** (Crossplane, Pulumi component) = abstraction sur les hyperscalers, mais leak des particularités (e.g. GKE Workload Identity ≠ EKS IRSA ≠ AKS Pod Identity v2)
- **Maintenance** : 3 modules indépendants × bug n cloud-spécifique = O(n), versus 1 module × bugs croisés = O(n²)
- **SE familiarity** : un SE qui maîtrise GCP appelle directement le module `terraform/modules/gcp/`, sans surcouche

### 1.4 Pourquoi Workload Identity (GCP) + Pod Identity v2 (Azure) plutôt que static credentials

**AWS** = IRSA (IAM Roles for Service Accounts), implémentée Sprint B
**GCP** = Workload Identity Federation (WI), Sprint C
**Azure** = Workload Identity (anciennement « Pod Identity v2 »), Sprint C

**Patterns équivalents** :

| Concept | AWS | GCP | Azure |
|---------|-----|-----|-------|
| Trust source | IAM OIDC provider EKS | GCP Workload Identity Pool | Azure AD federated identity credential |
| K8s side | ServiceAccount annotated `eks.amazonaws.com/role-arn=...` | ServiceAccount annotated `iam.gke.io/gcp-service-account=...` | ServiceAccount annotated `azure.workload.identity/client-id=...` + label `azure.workload.identity/use=true` |
| Pod side | AWS SDK auto-detects via env vars | GCP SDK auto-detects via metadata server | Azure SDK auto-detects via `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, `AZURE_FEDERATED_TOKEN_FILE` env vars |

**Anti-pattern à éviter** : credentials statiques montés en Secret K8s (exposable, rotation manuelle, audit déficient). Tous les modules GLOSS rejettent ce pattern.

### 1.5 Anti-patterns à éviter Sprint C

#### 1.5.1 Portail = « lourd dashboard analytics »

❌ **Mauvais** : portail avec 50 widgets, dashboards customisables drag-and-drop, etc. Volume = compliance pain (chaque widget = un audit).

✅ **Bon** : 8 pages **fonctionnelles** scope-limit. Pour visualisation analytics, le DPO utilise Grafana directement (déjà dans Helm chart Sprint B). Le portail est l'**interface compliance** + **interface action** (approve / reject reviews, attest, rotate keys) — pas un BI tool.

#### 1.5.2 Terraform GCP/Azure = copies AWS

❌ **Mauvais** : porter naïvement le module AWS en remplaçant `aws_db_instance` par `google_sql_database_instance`. Résultat : configuration sub-optimale qui n'exploite pas les particularités GCP (e.g. Cloud SQL Insights, AlloyDB...).

✅ **Bon** : module GCP idiomatique GCP (Cloud SQL HA Regional + Cloud SQL Insights + private IP via VPC peering) ; module Azure idiomatique Azure (Flexible Server + Geo-redundant backup + private endpoint via VNet integration).

#### 1.5.3 Runbooks « copier la procédure RB-01 »

❌ **Mauvais** : 12 runbooks identiques mais pour cas différents.

✅ **Bon** : chaque runbook adapte au cas spécifique. RB-08 (migration failure) parle de `sqlx migrate revert`, pas de `pg_resize`. RB-12 (PG failover) parle de Patroni / Cloud SQL HA / Azure auto-failover, pas de pgBackRest.

### 1.6 Glossaire Sprint C

| Terme | Définition |
|-------|------------|
| **App Router** | Router Next.js 14 avec layouts, RSC (Server Components), nested routing |
| **RSC** | React Server Components — rendu côté serveur, pas dans bundle client |
| **tRPC v11** | Type-safe RPC framework end-to-end (client TS ↔ server TS) |
| **NextAuth v5** | Auth.js — bibliothèque OIDC pour Next.js |
| **Server Action** | Fonction async exposée comme un endpoint HTTP par Next.js (form submission natif) |
| **GKE Autopilot** | GKE mode managé (Google gère les nodes) |
| **Cloud SQL Insights** | Performance Insights GCP équivalent à RDS Performance Insights |
| **AKS** | Azure Kubernetes Service |
| **Flexible Server** | Tier moderne d'Azure Database for PostgreSQL (vs deprecated Single Server) |
| **WI** | Workload Identity (GCP / Azure) |
| **Patroni** | HA Postgres orchestrator (utilisé par CrunchyData PGO) |

---

## §2. Architecture cible globale

### 2.1 Layout du repo après Sprint C

```
efe/
├── apps/
│   └── portal-admin/                    # NEW — Next.js 14 app
│       ├── app/                          # App Router (RSC)
│       ├── components/
│       ├── lib/
│       ├── tests/
│       ├── package.json
│       ├── pnpm-lock.yaml
│       ├── tsconfig.json
│       ├── tailwind.config.ts
│       ├── next.config.js
│       ├── Dockerfile                    # Production multi-stage build
│       └── README.md
├── charts/efe-stack/charts/portal-admin/  # Sprint B skeleton — Sprint C activates
├── terraform/modules/
│   ├── aws/                              # Sprint B
│   ├── gcp/                              # NEW Sprint C
│   └── azure/                            # NEW Sprint C
├── docs/runbooks/
│   ├── RUNBOOK_RB_01..06_*.md            # Sprint B
│   ├── RUNBOOK_RB_07_LATENCY_P99.md      # NEW
│   ├── RUNBOOK_RB_08_MIGRATION_FAILED.md # NEW
│   ├── RUNBOOK_RB_09_REVIEW_QUEUE_SATURATED.md  # NEW
│   ├── RUNBOOK_RB_10_TLS_CERT_EXPIRY.md  # NEW
│   ├── RUNBOOK_RB_11_LLM_QUOTA_NEAR_FULL.md  # NEW
│   └── RUNBOOK_RB_12_PG_FAILOVER.md      # NEW
├── scripts/
│   └── runbook-validate.sh               # Sprint B — extended Sprint C
└── .github/workflows/
    ├── helm-stack-test.yml               # Sprint B
    ├── terraform-aws.yml                 # Sprint B
    ├── terraform-gcp.yml                 # NEW Sprint C
    ├── terraform-azure.yml               # NEW Sprint C
    └── portal-admin-test.yml             # NEW Sprint C
```

### 2.2 Diagramme dépendances Sprint C ↔ Sprint A/B

```
┌──────────────────────────────────────────────────────────────────────┐
│                    Sprint C composants                                │
│                                                                       │
│  ┌──────────────────┐    ┌──────────────────┐    ┌────────────────┐ │
│  │ Portal Next.js   │    │ Terraform GCP    │    │ Terraform Azure│ │
│  │ - 8 pages        │    │ Module           │    │ Module         │ │
│  │ - tRPC routers   │    │                  │    │                │ │
│  └────────┬─────────┘    └────────┬─────────┘    └───────┬────────┘ │
│           │ depends on            │ depends on           │ depends on│
│           ▼                       ▼                      ▼           │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │ mcp-server tools D8 (Sprint A)                                 │ │
│  │   16 tools : tenants, keys, compliance, catalogs, audit, etc.  │ │
│  └────────────────────────────────────────────────────────────────┘ │
│           ▲                                                          │
│           │ Helm chart                                               │
│           ▼                                                          │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │ Sprint B umbrella + Postgres + Redis + Helm hooks              │ │
│  │ Default-deny NetworkPolicy + PSS restricted                    │ │
│  └────────────────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────────────┘
```

### 2.3 Boundaries Sprint C

| Composant | Owner | Responsabilité | Déployé sur |
|-----------|-------|----------------|-------------|
| `portal-admin` | EFE Core | UI tenant DPO/admin | Pod K8s (Helm sub-chart Sprint B) |
| `terraform/modules/gcp` | EFE Core | Provisioning infra GCP | SE laptop |
| `terraform/modules/azure` | EFE Core | Provisioning infra Azure | SE laptop |
| Runbooks RB-07 à RB-12 | EFE Ops | Procédures incident formalisées | docs (consultés via on-call) |

### 2.4 Versioning

| Pilier | Versioning | Tag git |
|--------|-----------|---------|
| Portal Next.js | semver | `portal-admin/vX.Y.Z` |
| Terraform GCP | semver indépendant | `tf-gcp/vX.Y.Z` |
| Terraform Azure | semver indépendant | `tf-azure/vX.Y.Z` |
| Runbooks | versionnés par révision Markdown | git history |

Premier release Sprint C :
- `portal-admin/v1.0.0`
- `tf-gcp/v1.0.0`
- `tf-azure/v1.0.0`

---

## §3. Portail Admin Next.js 14 — détails complets

### 3.1 Vue d'ensemble

Le portail admin est une **SPA isomorphe** Next.js 14, destinée au :
- **DPO tenant** (Data Protection Officer) — utilisateur principal pour compliance
- **Admin tenant** (CTO / IT lead) — utilisateur pour quotas + keys
- **Auditeur tenant** (lecture seule) — utilisateur pour audit logs

Personas et droits :

| Persona | Pages accessibles | Actions write |
|---------|-------------------|---------------|
| DPO | toutes (8 pages) | attest compliance, approve/reject reviews |
| Admin | toutes sauf compliance | rotate keys, set quotas |
| Auditeur | dashboard, audit, reviews (read-only) | aucune |

Granularity RBAC enforcée via :
1. Claims OIDC (`scope`, `roles`) du JWT IdP
2. tRPC procedure middleware vérifiant `ctx.user.role`
3. UI conditional rendering (boutons greyed-out si pas droit)

### 3.2 Stack technique

| Couche | Technologie | Version |
|--------|-------------|---------|
| Framework | Next.js | 14.2 (App Router) |
| Language | TypeScript | 5.4 strict |
| Auth | NextAuth.js (Auth.js) | v5 (beta stabilisé fin 2025) |
| API layer | tRPC | v11 |
| State (server) | TanStack Query | v5 |
| State (UI) | Zustand | v4 (minimal usage) |
| UI primitives | shadcn/ui | latest (Radix-based) |
| Styling | Tailwind CSS | v3.4 |
| Forms | react-hook-form | v7 |
| Validation | zod | v3 |
| Charts | Recharts | v2 |
| Tests unit | Vitest | v1.6 |
| Tests E2E | Playwright | v1.45 |
| Linting | Biome | v1.8 |
| Bundler | Webpack 5 (Next.js prod) / Turbopack (dev) | bundled |
| Package mgr | pnpm | v9 |
| Container | Distroless Node.js | gcr.io/distroless/nodejs22-debian12:nonroot |

### 3.3 Layout du repo `apps/portal-admin/`

```
apps/portal-admin/
├── package.json
├── pnpm-lock.yaml
├── tsconfig.json
├── tsconfig.test.json
├── tailwind.config.ts
├── postcss.config.js
├── next.config.js
├── biome.json
├── playwright.config.ts
├── vitest.config.ts
├── Dockerfile
├── .dockerignore
├── README.md
│
├── app/                                # Next.js App Router
│   ├── layout.tsx                       # Root layout
│   ├── globals.css
│   ├── error.tsx                        # Global error boundary
│   ├── not-found.tsx
│   │
│   ├── (public)/                        # Unauthenticated routes
│   │   ├── login/
│   │   │   └── page.tsx
│   │   └── error/
│   │       └── page.tsx
│   │
│   ├── (authenticated)/                 # Protected routes
│   │   ├── layout.tsx                   # Auth check + nav + tenant switcher
│   │   ├── dashboard/
│   │   │   ├── page.tsx                 # Overview KPIs (RSC)
│   │   │   └── loading.tsx
│   │   ├── audit/
│   │   │   ├── page.tsx                 # Search + paginate
│   │   │   ├── loading.tsx
│   │   │   └── [auditId]/
│   │   │       └── page.tsx             # Single audit detail + concept_stack
│   │   ├── reviews/
│   │   │   ├── page.tsx                 # Pending Mode-3 reviews
│   │   │   └── [token]/
│   │   │       └── page.tsx             # Review detail + approve/reject
│   │   ├── compliance/
│   │   │   └── page.tsx                 # AI Act + GDPR attestations
│   │   ├── keys/
│   │   │   └── page.tsx                 # API keys CRUD
│   │   ├── quotas/
│   │   │   └── page.tsx                 # Quotas view + edit
│   │   └── settings/
│   │       └── page.tsx                 # Tenant settings (data residency, BYOK)
│   │
│   └── api/
│       ├── auth/[...nextauth]/route.ts  # NextAuth handlers
│       ├── trpc/[trpc]/route.ts         # tRPC fetch handler
│       ├── healthz/route.ts             # Liveness
│       └── readyz/route.ts              # Readiness
│
├── components/
│   ├── ui/                              # shadcn/ui primitives (Button, Input, etc.)
│   ├── nav/
│   │   ├── header.tsx
│   │   ├── tenant-switcher.tsx
│   │   └── user-menu.tsx
│   ├── audit/
│   │   ├── audit-table.tsx
│   │   ├── audit-filters.tsx
│   │   └── concept-stack-viewer.tsx     # Tree view JSONB Mode-3
│   ├── reviews/
│   │   ├── review-card.tsx
│   │   └── review-action-modal.tsx
│   ├── compliance/
│   │   └── attestation-form.tsx
│   ├── keys/
│   │   ├── key-create-modal.tsx
│   │   └── key-list-table.tsx
│   ├── dashboard/
│   │   ├── kpi-card.tsx
│   │   ├── rps-chart.tsx
│   │   └── latency-chart.tsx
│   └── shared/
│       ├── confirm-dialog.tsx
│       ├── error-banner.tsx
│       └── loading-spinner.tsx
│
├── lib/
│   ├── auth/
│   │   ├── config.ts                    # NextAuth config
│   │   ├── session.ts                   # Server-side session helpers
│   │   └── rbac.ts                      # Role / permission checks
│   ├── trpc/
│   │   ├── server.ts                    # Server tRPC instance
│   │   ├── client.ts                    # Client tRPC instance (TanStack Query)
│   │   ├── context.ts                   # tRPC ctx (user, tenant, mcp client)
│   │   └── routers/
│   │       ├── _index.ts                # appRouter
│   │       ├── tenant.ts
│   │       ├── audit.ts
│   │       ├── reviews.ts
│   │       ├── compliance.ts
│   │       ├── keys.ts
│   │       └── quotas.ts
│   ├── grpc/
│   │   └── client.ts                    # gRPC-web → efe-server (server-side only)
│   ├── mcp/
│   │   └── client.ts                    # HTTP → mcp-server (server-side only)
│   ├── audit/
│   │   └── ui-audit-log.ts              # Records UI actions to ui_audit_log
│   └── utils/
│       ├── format.ts                    # Date / number formatters
│       ├── tenant.ts                    # Tenant slug validation
│       └── pdf.ts                       # PDF export (right-to-explanation)
│
├── tests/
│   ├── unit/
│   │   ├── components/
│   │   │   ├── audit-table.test.tsx
│   │   │   ├── concept-stack-viewer.test.tsx
│   │   │   └── confirm-dialog.test.tsx
│   │   └── lib/
│   │       ├── rbac.test.ts
│   │       └── tenant.test.ts
│   └── e2e/
│       ├── login.spec.ts
│       ├── dashboard.spec.ts
│       ├── audit-search.spec.ts
│       ├── audit-detail.spec.ts
│       ├── review-approve.spec.ts
│       ├── review-reject.spec.ts
│       ├── compliance-attest.spec.ts
│       ├── keys-create-revoke.spec.ts
│       ├── quotas-set.spec.ts
│       ├── rbac-enforcement.spec.ts
│       └── full-flow.spec.ts
│
└── public/
    ├── favicon.ico
    └── logo.svg
```

### 3.4 Pages détaillées

#### 3.4.1 `/login` (public)

Single SSO button → redirige vers IdP OIDC (Keycloak / Okta / Azure AD / Auth0).

```tsx
// app/(public)/login/page.tsx
import { signIn } from "next-auth/react";
import { Button } from "@/components/ui/button";

export default function LoginPage() {
  return (
    <main className="flex h-screen items-center justify-center">
      <div className="w-full max-w-sm space-y-6 p-8">
        <h1 className="text-2xl font-bold">EFE Portal Admin</h1>
        <Button onClick={() => signIn("oidc", { callbackUrl: "/dashboard" })}>
          Sign in with SSO
        </Button>
      </div>
    </main>
  );
}
```

Validation tests E2E :
- `tests/e2e/login.spec.ts` : redirect vers IdP, callback réussi → arrive à `/dashboard`
- Login avec mauvais credentials → reste sur IdP avec erreur (out of scope portal — IdP problème)

#### 3.4.2 `/dashboard` (DPO + Admin + Auditeur)

KPIs principaux affichés en RSC (cache 30s). Données depuis `mcp-server.efe_get_tenant_health` :

```tsx
// app/(authenticated)/dashboard/page.tsx
import { trpc } from "@/lib/trpc/server";
import { KpiCard } from "@/components/dashboard/kpi-card";
import { RpsChart } from "@/components/dashboard/rps-chart";
import { LatencyChart } from "@/components/dashboard/latency-chart";
import { Suspense } from "react";

export const revalidate = 30; // cache 30 sec

export default async function DashboardPage() {
  const health = await trpc.tenant.health();

  return (
    <div className="space-y-6 p-6">
      <h1 className="text-2xl font-bold">Dashboard</h1>

      <div className="grid grid-cols-1 gap-4 md:grid-cols-3 lg:grid-cols-6">
        <KpiCard label="RPS" value={health.metrics.rps_last_5m ?? "—"} />
        <KpiCard label="p99 latency" value={`${health.metrics.p99_ms_last_5m}ms`} />
        <KpiCard label="Error rate" value={`${(health.metrics.error_rate_last_5m * 100).toFixed(2)}%`} />
        <KpiCard label="Cost today" value={`$${health.cost_today_usd?.toFixed(2)}`} />
        <KpiCard label="Pending reviews" value={health.pending_reviews} />
        <KpiCard label="Mode-3 convergence" value="—" /> {/* fed by Prometheus */}
      </div>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <Suspense fallback={<div>Loading…</div>}>
          <RpsChart />
        </Suspense>
        <Suspense fallback={<div>Loading…</div>}>
          <LatencyChart />
        </Suspense>
      </div>

      <div className="rounded-md bg-yellow-50 p-4 dark:bg-yellow-900">
        <p>Status : {health.status === "healthy" ? "✅" : "⚠️"} {health.status}</p>
      </div>
    </div>
  );
}
```

Les charts (`RpsChart` + `LatencyChart`) sont des Server Components qui fetch Prometheus directement (server-side, pas via mcp-server) :

```tsx
// components/dashboard/rps-chart.tsx
import { LineChart, Line, XAxis, YAxis, ResponsiveContainer } from "recharts";

async function fetchPromQL(query: string) {
  const url = new URL(process.env.PROMETHEUS_URL ?? "http://prometheus:9090");
  url.pathname = "/api/v1/query_range";
  url.searchParams.set("query", query);
  url.searchParams.set("start", `${Math.floor(Date.now() / 1000) - 3600}`);
  url.searchParams.set("end", `${Math.floor(Date.now() / 1000)}`);
  url.searchParams.set("step", "60");
  const res = await fetch(url, { next: { revalidate: 60 } });
  if (!res.ok) throw new Error(`prometheus: ${res.status}`);
  const data = await res.json();
  return data.data?.result?.[0]?.values ?? [];
}

export async function RpsChart() {
  const points = await fetchPromQL("sum(rate(efe_classify_total[1m]))");
  const data = points.map(([t, v]: [number, string]) => ({
    t: new Date(t * 1000).toLocaleTimeString(),
    v: parseFloat(v),
  }));

  return (
    <ResponsiveContainer width="100%" height={300}>
      <LineChart data={data}>
        <XAxis dataKey="t" />
        <YAxis />
        <Line dataKey="v" />
      </LineChart>
    </ResponsiveContainer>
  );
}
```

#### 3.4.3 `/audit` (search + paginate)

```tsx
// app/(authenticated)/audit/page.tsx
import { trpc } from "@/lib/trpc/server";
import { AuditTable } from "@/components/audit/audit-table";
import { AuditFilters } from "@/components/audit/audit-filters";

interface Props {
  searchParams: {
    cursor?: string;
    mode?: "MODE_2" | "MODE_AUTO_ESC" | "MODE_3";
    since?: string;
  };
}

export default async function AuditPage({ searchParams }: Props) {
  const { rows, nextCursor } = await trpc.audit.query({
    cursor: searchParams.cursor,
    mode: searchParams.mode,
    since: searchParams.since,
    limit: 50,
  });

  return (
    <div className="space-y-4 p-6">
      <h1 className="text-2xl font-bold">Audit log</h1>
      <AuditFilters />
      <AuditTable rows={rows} />
      {nextCursor && (
        <a
          href={`/audit?cursor=${nextCursor}&mode=${searchParams.mode ?? ""}`}
          className="text-blue-600 hover:underline"
        >
          Next page →
        </a>
      )}
    </div>
  );
}
```

Filter component : tenant (locked to user's tenant), date range (since/until), mode (M2/M3/AUTO), confidence range, status.

#### 3.4.4 `/audit/[auditId]` (Mode-3 detail + concept_stack)

C'est la **page la plus critique** du portail — elle expose la traçabilité Mode-3 pour right-to-explanation GDPR Art. 22.

```tsx
// app/(authenticated)/audit/[auditId]/page.tsx
import { trpc } from "@/lib/trpc/server";
import { ConceptStackViewer } from "@/components/audit/concept-stack-viewer";
import { Button } from "@/components/ui/button";
import { Suspense } from "react";

interface Props {
  params: { auditId: string };
}

export default async function AuditDetailPage({ params }: Props) {
  const auditId = parseInt(params.auditId, 10);
  if (isNaN(auditId)) {
    return <div>Invalid audit ID</div>;
  }

  const audit = await trpc.audit.get({ auditId });

  return (
    <div className="space-y-6 p-6">
      <h1 className="text-2xl font-bold">Audit detail #{audit.id}</h1>

      <div className="grid grid-cols-2 gap-4">
        <div>
          <label className="text-sm font-medium">Tenant</label>
          <p>{audit.tenant}</p>
        </div>
        <div>
          <label className="text-sm font-medium">Mode</label>
          <p>{audit.mode}</p>
        </div>
        <div>
          <label className="text-sm font-medium">Sentence hash</label>
          <p className="font-mono text-xs">{audit.sentence_hash}</p>
        </div>
        <div>
          <label className="text-sm font-medium">Label</label>
          <p>{audit.label} ({(audit.confidence ?? 0).toFixed(3)})</p>
        </div>
        <div>
          <label className="text-sm font-medium">Timestamp</label>
          <p>{new Date(audit.ts).toISOString()}</p>
        </div>
        <div>
          <label className="text-sm font-medium">HMAC chain</label>
          <p>{audit.chain_intact ? "✅ intact" : "❌ broken"}</p>
        </div>
      </div>

      {audit.mode === "MODE_3" && audit.concept_stack && (
        <div>
          <h2 className="text-xl font-semibold">Concept stack (Mode-3 reasoning trace)</h2>
          <p className="text-sm text-gray-600">
            GDPR Art. 22 right-to-explanation : the full multi-turn reasoning
            trace that led to the final classification.
          </p>
          <Suspense fallback={<div>Rendering trace…</div>}>
            <ConceptStackViewer stack={audit.concept_stack} />
          </Suspense>
        </div>
      )}

      <div className="flex gap-2">
        <Button asChild>
          <a href={`/api/audit/${auditId}/export.pdf`}>
            Export PDF (right-to-explanation)
          </a>
        </Button>
      </div>
    </div>
  );
}
```

Le `ConceptStackViewer` est un tree-view RSC qui affiche turn par turn :

```tsx
// components/audit/concept-stack-viewer.tsx
import type { ConceptStack } from "@/lib/types/concept-stack";

export function ConceptStackViewer({ stack }: { stack: ConceptStack }) {
  return (
    <div className="space-y-4">
      {stack.entries.map((entry, idx) => (
        <details
          key={idx}
          className="rounded-md border border-gray-200 p-4"
          open={idx === stack.entries.length - 1}
        >
          <summary className="cursor-pointer font-semibold">
            Turn {idx + 1} — confidence {entry.swarm_confidence.toFixed(3)}
            {entry.converged && " ✅ converged"}
          </summary>
          <div className="mt-3 space-y-2">
            <div>
              <strong>Anchors top-{entry.top_anchors.length}</strong> :
              <ul className="list-disc pl-6 text-sm">
                {entry.top_anchors.map((a) => (
                  <li key={a.anchor_id}>
                    {a.text} → label={a.label}, score={a.score.toFixed(3)}
                  </li>
                ))}
              </ul>
            </div>
            <div>
              <strong>Provider used</strong> : {entry.provider_used} / {entry.template_id}
            </div>
            <div>
              <strong>LLM reasoning</strong> :
              <pre className="overflow-x-auto rounded bg-gray-50 p-2 text-xs">
                {entry.llm_reasoning}
              </pre>
            </div>
            <div className="text-xs text-gray-500">
              {entry.tokens_used} tokens · ${entry.cost_usd.toFixed(5)}
              · {entry.duration_ms}ms
            </div>
          </div>
        </details>
      ))}
    </div>
  );
}
```

Right-to-explanation PDF export :

```ts
// app/api/audit/[auditId]/export.pdf/route.ts
import { auth } from "@/lib/auth/config";
import { trpc } from "@/lib/trpc/server";
import { generateExplanationPdf } from "@/lib/utils/pdf";

export async function GET(request: Request, { params }: { params: { auditId: string } }) {
  const session = await auth();
  if (!session) return new Response("Unauthorized", { status: 401 });

  const audit = await trpc.audit.get({ auditId: parseInt(params.auditId, 10) });
  // Audit log this export (GDPR Art. 33 trail).
  await trpc.audit.recordUiAction({
    action: "audit.export_explanation",
    target_id: params.auditId,
  });

  const pdfBytes = await generateExplanationPdf(audit);
  return new Response(pdfBytes, {
    headers: {
      "Content-Type": "application/pdf",
      "Content-Disposition": `attachment; filename="audit-${params.auditId}-explanation.pdf"`,
    },
  });
}
```

#### 3.4.5 `/reviews` (Mode-3 pending reviews)

```tsx
// app/(authenticated)/reviews/page.tsx
import { trpc } from "@/lib/trpc/server";
import { ReviewCard } from "@/components/reviews/review-card";

export const revalidate = 10;

export default async function ReviewsPage() {
  const { reviews } = await trpc.reviews.listPending();

  if (reviews.length === 0) {
    return (
      <div className="p-6">
        <h1 className="text-2xl font-bold">Mode-3 pending reviews</h1>
        <p className="mt-4 text-gray-600">No reviews pending. ✨</p>
      </div>
    );
  }

  return (
    <div className="space-y-4 p-6">
      <h1 className="text-2xl font-bold">
        Mode-3 pending reviews ({reviews.length})
      </h1>
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        {reviews.map((r) => (
          <ReviewCard key={r.review_token} review={r} />
        ))}
      </div>
    </div>
  );
}
```

#### 3.4.6 `/reviews/[token]` (approve/reject)

```tsx
// app/(authenticated)/reviews/[token]/page.tsx
import { trpc } from "@/lib/trpc/server";
import { ReviewActionModal } from "@/components/reviews/review-action-modal";
import { ConceptStackViewer } from "@/components/audit/concept-stack-viewer";

export default async function ReviewDetailPage({
  params,
}: { params: { token: string } }) {
  const review = await trpc.reviews.get({ token: params.token });

  return (
    <div className="space-y-6 p-6">
      <h1 className="text-2xl font-bold">Review {params.token}</h1>

      <div className="grid grid-cols-2 gap-4">
        <div>
          <label className="text-sm font-medium">Catalog</label>
          <p>{review.catalog} ({review.bundle_version})</p>
        </div>
        <div>
          <label className="text-sm font-medium">Proposed label</label>
          <p>{review.proposed_label} ({review.proposed_confidence.toFixed(3)})</p>
        </div>
        <div>
          <label className="text-sm font-medium">Sentence hash</label>
          <p className="font-mono text-xs">{review.sentence_hash}</p>
        </div>
        <div>
          <label className="text-sm font-medium">Expires</label>
          <p>{new Date(review.expires_at).toLocaleString()}</p>
        </div>
      </div>

      <div>
        <h2 className="text-xl font-semibold">Concept stack</h2>
        <ConceptStackViewer stack={review.concept_stack} />
      </div>

      <div className="flex gap-2">
        <ReviewActionModal token={params.token} action="approve" />
        <ReviewActionModal token={params.token} action="reject" />
      </div>
    </div>
  );
}
```

`ReviewActionModal` est un Client Component avec form :

```tsx
"use client";
import { useState } from "react";
import { trpc } from "@/lib/trpc/client";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";

export function ReviewActionModal({
  token,
  action,
}: {
  token: string;
  action: "approve" | "reject";
}) {
  const [open, setOpen] = useState(false);
  const [reason, setReason] = useState("");
  const [finalLabel, setFinalLabel] = useState("");

  const approve = trpc.reviews.approve.useMutation();
  const reject = trpc.reviews.reject.useMutation();

  const handle = async () => {
    if (action === "approve") {
      await approve.mutateAsync({ token, finalLabel: finalLabel || undefined });
    } else {
      if (reason.trim().length < 10) {
        alert("Rejection reason must be at least 10 characters.");
        return;
      }
      await reject.mutateAsync({ token, reason, finalLabel: finalLabel || undefined });
    }
    setOpen(false);
    location.reload();
  };

  return (
    <>
      <Button
        variant={action === "approve" ? "default" : "destructive"}
        onClick={() => setOpen(true)}
      >
        {action === "approve" ? "Approve" : "Reject"}
      </Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{action === "approve" ? "Approve" : "Reject"} review</DialogTitle>
          </DialogHeader>
          <div className="space-y-4">
            <input
              placeholder="Optional final label override"
              value={finalLabel}
              onChange={(e) => setFinalLabel(e.target.value)}
              className="w-full rounded border p-2"
            />
            {action === "reject" && (
              <textarea
                placeholder="Rejection reason (>= 10 chars)"
                value={reason}
                onChange={(e) => setReason(e.target.value)}
                className="w-full rounded border p-2"
                rows={3}
                required
              />
            )}
            <Button onClick={handle}>Confirm {action}</Button>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}
```

#### 3.4.7 `/compliance`

Page d'attestation AI Act Art. 14 + GDPR Art. 22 :

```tsx
// app/(authenticated)/compliance/page.tsx
import { trpc } from "@/lib/trpc/server";
import { AttestationForm } from "@/components/compliance/attestation-form";

export default async function CompliancePage() {
  const status = await trpc.compliance.verify();

  return (
    <div className="space-y-6 p-6">
      <h1 className="text-2xl font-bold">Compliance</h1>

      <div className="grid grid-cols-2 gap-4">
        <div className="rounded-md border p-4">
          <h2 className="font-semibold">EU AI Act Art. 14 (high-risk human oversight)</h2>
          {status.ai_act_art14.acknowledged ? (
            <p>✅ Acknowledged on {status.ai_act_art14.acknowledged_at} by {status.ai_act_art14.signed_by}</p>
          ) : (
            <p>❌ Not acknowledged</p>
          )}
        </div>
        <div className="rounded-md border p-4">
          <h2 className="font-semibold">GDPR Art. 22 (right not to be subject)</h2>
          {status.gdpr_art22.acknowledged ? (
            <p>✅ Acknowledged on {status.gdpr_art22.acknowledged_at} by {status.gdpr_art22.signed_by}</p>
          ) : (
            <p>❌ Not acknowledged</p>
          )}
        </div>
      </div>

      <AttestationForm currentStatus={status} />
    </div>
  );
}
```

`AttestationForm` est un Client Component avec validation :

```tsx
"use client";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import { trpc } from "@/lib/trpc/client";

const schema = z.object({
  ai_act_art14: z.boolean(),
  gdpr_art22: z.boolean(),
  signed_by: z.string().email(),
  dpo_contact: z.string().email().optional(),
}).refine((d) => d.ai_act_art14 || d.gdpr_art22, {
  message: "At least one attestation must be selected",
}).refine((d) => !d.gdpr_art22 || !!d.dpo_contact, {
  message: "DPO contact is required when GDPR Art. 22 is acknowledged",
});

export function AttestationForm({ currentStatus }: { currentStatus: any }) {
  const { register, handleSubmit, formState: { errors } } = useForm({
    resolver: zodResolver(schema),
  });
  const attest = trpc.compliance.attest.useMutation();

  return (
    <form onSubmit={handleSubmit((d) => attest.mutate(d))} className="space-y-4">
      <label className="flex items-center gap-2">
        <input type="checkbox" {...register("ai_act_art14")} />
        I acknowledge EU AI Act Art. 14 (high-risk human oversight)
      </label>
      <label className="flex items-center gap-2">
        <input type="checkbox" {...register("gdpr_art22")} />
        I acknowledge GDPR Art. 22 (right not to be subject to solely automated decisions)
      </label>
      <input
        placeholder="Your email (signer)"
        {...register("signed_by")}
        className="block w-full rounded border p-2"
      />
      <input
        placeholder="DPO contact email (required for GDPR)"
        {...register("dpo_contact")}
        className="block w-full rounded border p-2"
      />
      {errors.root && <p className="text-red-600">{errors.root.message}</p>}
      <button type="submit" className="rounded bg-blue-600 px-4 py-2 text-white">
        Attest
      </button>
    </form>
  );
}
```

#### 3.4.8 `/keys`

Liste des API keys avec actions create / rotate / revoke. Cf. doc Sprint A § 3.7.

#### 3.4.9 `/quotas`

Vue + édition des quotas (max_rps, max_tokens_per_day, monthly_cap_usd).

#### 3.4.10 `/settings`

Vue read-only data residency + BYOK key + DPO contact + suspend tenant.

### 3.5 Auth flow OIDC

```
[User]
   │
   │ 1. Click "Sign in with SSO"
   ▼
[Portal /login]
   │
   │ 2. GET /api/auth/signin/oidc → 302
   ▼
[IdP (Keycloak / Okta / Azure AD / Auth0)]
   │
   │ 3. User authenticates (password / MFA)
   │
   │ 4. GET /api/auth/callback/oidc?code=XXX&state=YYY
   ▼
[Portal NextAuth.js]
   │
   │ 5. Exchange code for tokens (POST /token to IdP)
   │
   │ 6. Verify ID token (signature, exp, iss, aud)
   │
   │ 7. Extract claims : sub, email, tenants, roles
   │
   │ 8. Create encrypted session cookie (JWT signed with secret)
   │
   ▼
[Portal /dashboard?tenant=acme]
```

NextAuth v5 config :

```ts
// lib/auth/config.ts
import NextAuth from "next-auth";
import OIDCProvider from "next-auth/providers/oidc";

export const { auth, handlers, signIn, signOut } = NextAuth({
  providers: [
    OIDCProvider({
      id: "oidc",
      issuer: process.env.OIDC_ISSUER!,
      clientId: process.env.OIDC_CLIENT_ID!,
      clientSecret: process.env.OIDC_CLIENT_SECRET!,
      authorization: {
        params: { scope: "openid email profile efe:tenants" },
      },
    }),
  ],
  session: {
    strategy: "jwt",
    maxAge: 60 * 60 * 8, // 8h
  },
  callbacks: {
    async jwt({ token, profile }) {
      if (profile) {
        // Extract tenant claims (custom IdP claim).
        token.tenants = (profile as any).tenants ?? [];
        token.roles = (profile as any).roles ?? [];
      }
      return token;
    },
    async session({ session, token }) {
      session.user.tenants = token.tenants as string[];
      session.user.roles = token.roles as string[];
      return session;
    },
  },
  pages: {
    signIn: "/login",
    error: "/error",
  },
  cookies: {
    sessionToken: {
      name: "__Secure-portal-session",
      options: {
        httpOnly: true,
        sameSite: "strict",
        secure: true,
        path: "/",
      },
    },
  },
});
```

Middleware Edge pour vérification auth avant tout rendering :

```ts
// middleware.ts
import { auth } from "@/lib/auth/config";
import { NextResponse } from "next/server";

export default auth((req) => {
  const { nextUrl } = req;
  const isAuthed = !!req.auth;
  const isPublic = nextUrl.pathname.startsWith("/login")
                   || nextUrl.pathname.startsWith("/api/auth")
                   || nextUrl.pathname.startsWith("/api/healthz")
                   || nextUrl.pathname.startsWith("/api/readyz");

  if (!isAuthed && !isPublic) {
    return NextResponse.redirect(new URL("/login", nextUrl));
  }
  return NextResponse.next();
});

export const config = {
  matcher: ["/((?!_next/static|_next/image|favicon.ico).*)"],
};
```

### 3.6 RBAC + multi-tenant

#### 3.6.1 Tenant scoping côté tRPC

Chaque tRPC procedure middleware vérifie que `ctx.tenant ∈ session.user.tenants` :

```ts
// lib/trpc/context.ts
import { auth } from "@/lib/auth/config";
import { McpClient } from "@/lib/mcp/client";

export async function createContext(req: Request) {
  const session = await auth();
  // Tenant is set via header `x-efe-tenant` (set by tenant-switcher in nav).
  const tenant = req.headers.get("x-efe-tenant") || session?.user.tenants?.[0];
  if (!tenant) throw new Error("No tenant in context");
  // Enforce tenant ∈ user's tenants.
  if (session && !session.user.tenants.includes(tenant)) {
    throw new Error(`Tenant ${tenant} not in user's tenants`);
  }
  return {
    user: session?.user,
    tenant,
    mcp: new McpClient({
      baseUrl: process.env.MCP_HTTP_URL!,
      tenant,
      bearerToken: session?.accessToken,
    }),
  };
}

export type Context = Awaited<ReturnType<typeof createContext>>;
```

#### 3.6.2 RBAC procedure middleware

```ts
// lib/trpc/server.ts
import { initTRPC, TRPCError } from "@trpc/server";
import { Context } from "./context";

const t = initTRPC.context<Context>().create();

export const router = t.router;

const isAuthenticated = t.middleware(({ ctx, next }) => {
  if (!ctx.user) {
    throw new TRPCError({ code: "UNAUTHORIZED" });
  }
  return next({ ctx: { ...ctx, user: ctx.user } });
});

const requireRole = (role: "dpo" | "admin" | "auditor") =>
  t.middleware(({ ctx, next }) => {
    if (!ctx.user) throw new TRPCError({ code: "UNAUTHORIZED" });
    if (!ctx.user.roles.includes(role)) {
      throw new TRPCError({ code: "FORBIDDEN", message: `requires role ${role}` });
    }
    return next({ ctx });
  });

export const publicProcedure = t.procedure;
export const authedProcedure = t.procedure.use(isAuthenticated);
export const dpoProcedure = t.procedure.use(isAuthenticated).use(requireRole("dpo"));
export const adminProcedure = t.procedure.use(isAuthenticated).use(requireRole("admin"));
```

#### 3.6.3 Mapping rôles → procedures

| Procedure | Réservé à |
|-----------|-----------|
| `compliance.attest` | `dpo` |
| `reviews.approve / reject` | `dpo` |
| `keys.create / rotate / revoke` | `admin` |
| `quotas.set` | `admin` |
| `audit.query / get` | tous (auth) |
| `reviews.listPending / get` | tous (auth) |

### 3.7 Sécurité

#### 3.7.1 CSP strict

```ts
// next.config.js
const csp = `
  default-src 'self';
  script-src 'self' 'nonce-{NONCE}' 'strict-dynamic';
  style-src 'self' 'unsafe-inline';
  img-src 'self' data:;
  connect-src 'self' https://idp.example.com;
  frame-ancestors 'none';
  base-uri 'self';
  form-action 'self';
`.replace(/\s+/g, " ").trim();

module.exports = {
  output: "standalone",
  reactStrictMode: true,
  poweredByHeader: false,
  async headers() {
    return [
      {
        source: "/:path*",
        headers: [
          { key: "Content-Security-Policy", value: csp },
          { key: "X-Frame-Options", value: "DENY" },
          { key: "X-Content-Type-Options", value: "nosniff" },
          { key: "Referrer-Policy", value: "strict-origin-when-cross-origin" },
          { key: "Permissions-Policy", value: "camera=(), microphone=(), geolocation=()" },
          { key: "Strict-Transport-Security", value: "max-age=63072000; includeSubDomains; preload" },
        ],
      },
    ];
  },
};
```

#### 3.7.2 CSRF

Next.js 14 Server Actions ont un CSRF check natif (via origin header check). tRPC mutations utilisent `Origin` + `Referer` validation.

#### 3.7.3 UI audit log

Toute action `write` est log via tRPC middleware :

```ts
// lib/trpc/middleware/audit-log.ts
import { trpc } from "@/lib/trpc/server";

export const auditLogged = (action: string) =>
  t.middleware(async ({ ctx, next, input }) => {
    const result = await next({ ctx });
    // Best-effort log (non-blocking).
    ctx.mcp.callTool("efe_record_ui_audit", {
      tenant_id: ctx.tenant,
      user_email: ctx.user!.email,
      action,
      payload_jsonb: input as object,
      // Confirm not needed — read-only audit recording.
    }).catch((e) => console.error("ui audit log failed:", e));
    return result;
  });
```

NB : le tool `efe_record_ui_audit` n'est pas dans le scope D8 actuel ; il sera ajouté en Sprint D ou via D8.1 patch. Pour Sprint C, le portail loggue best-effort en interne (file-based) en attendant le tool backend.

#### 3.7.4 Cross-tenant isolation tests

Test E2E critique :

```ts
// tests/e2e/rbac-enforcement.spec.ts
test("DPO of tenant A cannot access tenant B audit", async ({ page }) => {
  await loginAsDPO(page, "tenantA");
  // Try to navigate to /audit with tenant=tenantB header.
  const response = await page.request.get("/api/trpc/audit.query", {
    headers: { "x-efe-tenant": "tenantB" },
  });
  expect(response.status()).toBe(403);
});
```

### 3.8 tRPC routers

`lib/trpc/routers/_index.ts` :

```ts
import { router } from "../server";
import { auditRouter } from "./audit";
import { complianceRouter } from "./compliance";
import { keysRouter } from "./keys";
import { quotasRouter } from "./quotas";
import { reviewsRouter } from "./reviews";
import { tenantRouter } from "./tenant";

export const appRouter = router({
  audit: auditRouter,
  compliance: complianceRouter,
  keys: keysRouter,
  quotas: quotasRouter,
  reviews: reviewsRouter,
  tenant: tenantRouter,
});

export type AppRouter = typeof appRouter;
```

Sample router :

```ts
// lib/trpc/routers/compliance.ts
import { z } from "zod";
import { router, dpoProcedure, authedProcedure } from "../server";

export const complianceRouter = router({
  verify: authedProcedure.query(async ({ ctx }) => {
    return ctx.mcp.callTool("efe_verify_compliance", {
      tenant_id: ctx.tenant,
    });
  }),
  attest: dpoProcedure
    .input(z.object({
      ai_act_art14: z.boolean(),
      gdpr_art22: z.boolean(),
      signed_by: z.string().email(),
      dpo_contact: z.string().email().optional(),
    }))
    .mutation(async ({ ctx, input }) => {
      return ctx.mcp.callTool("efe_attest_compliance", {
        tenant_id: ctx.tenant,
        ...input,
        confirm: true,
      });
    }),
});
```

### 3.9 Tests Vitest + Playwright

#### 3.9.1 Vitest unit tests (~110 tests)

| Module | Tests | Coverage |
|--------|-------|----------|
| `components/audit/audit-table.tsx` | 8 | rendering, filters, sort |
| `components/audit/concept-stack-viewer.tsx` | 12 | turn rendering, expand/collapse, edge cases (empty, broken stack) |
| `components/reviews/review-action-modal.tsx` | 10 | approve/reject flows, validation reason |
| `components/compliance/attestation-form.tsx` | 14 | zod validation, both attestations, DPO required |
| `components/keys/key-create-modal.tsx` | 8 | scope dropdown, expiration parsing, plaintext display once |
| `lib/trpc/routers/*` | 30 | each procedure (mocked context) |
| `lib/auth/rbac.ts` | 15 | tenant ∈ tenants, role check, anonymous reject |
| `lib/utils/tenant.ts` | 8 | slug validation |
| `lib/utils/format.ts` | 5 | timestamps, latencies, costs |
| **Total** | **110** | ≥ 80 % lines |

#### 3.9.2 Playwright E2E (~145 specs)

| Spec | Tests | Browsers |
|------|-------|----------|
| login.spec.ts | 8 | chrome, firefox, webkit |
| dashboard.spec.ts | 12 | × 3 = 36 specs |
| audit-search.spec.ts | 10 | × 3 = 30 specs |
| audit-detail.spec.ts | 8 (Mode-3 concept_stack rendering) | × 3 |
| review-approve.spec.ts | 6 | × 3 |
| review-reject.spec.ts | 6 | × 3 |
| compliance-attest.spec.ts | 8 | × 3 |
| keys-create-revoke.spec.ts | 6 | × 3 |
| quotas-set.spec.ts | 4 | × 3 |
| rbac-enforcement.spec.ts | 8 (cross-tenant blocked, role-mismatch) | × 3 |
| full-flow.spec.ts | 4 (full DPO journey) | × 3 |
| **Total** | **~145 specs** | 3 browsers |

CI runs all 3 browsers in parallel. Local dev runs Chromium only by default.

### 3.10 Build + Docker

#### 3.10.1 Multi-stage Dockerfile

```dockerfile
# apps/portal-admin/Dockerfile

# Stage 1 : install deps + build
FROM node:22-alpine AS builder
WORKDIR /app

# Copy lockfile + manifests
COPY package.json pnpm-lock.yaml ./
RUN corepack enable && pnpm install --frozen-lockfile

# Copy source
COPY . .

# Build (output: standalone for distroless runtime)
ARG NEXT_PUBLIC_VERSION=dev
ENV NEXT_TELEMETRY_DISABLED=1
RUN pnpm build

# Stage 2 : distroless runtime (no shell, no apt)
FROM gcr.io/distroless/nodejs22-debian12:nonroot AS runtime
WORKDIR /app

ENV NODE_ENV=production
ENV PORT=3000
ENV NEXT_TELEMETRY_DISABLED=1

# Copy standalone build (Next.js ships its own minimal node_modules)
COPY --from=builder --chown=nonroot:nonroot /app/.next/standalone ./
COPY --from=builder --chown=nonroot:nonroot /app/.next/static ./.next/static
COPY --from=builder --chown=nonroot:nonroot /app/public ./public

USER nonroot
EXPOSE 3000

ENTRYPOINT ["/nodejs/bin/node", "server.js"]
```

Image size cible : **< 200 MB** (distroless + standalone).

#### 3.10.2 Build pipeline CI

```yaml
# .github/workflows/portal-admin-test.yml
name: portal-admin-test

on:
  pull_request:
    paths:
      - "apps/portal-admin/**"

jobs:
  unit-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
        with: { version: 9 }
      - uses: actions/setup-node@v4
        with: { node-version: 22, cache: "pnpm", cache-dependency-path: "apps/portal-admin/pnpm-lock.yaml" }
      - run: cd apps/portal-admin && pnpm install --frozen-lockfile
      - run: cd apps/portal-admin && pnpm test:unit
      - run: cd apps/portal-admin && pnpm build

  e2e-tests:
    runs-on: ubuntu-latest
    needs: unit-tests
    strategy:
      matrix:
        browser: [chromium, firefox, webkit]
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
        with: { version: 9 }
      - uses: actions/setup-node@v4
        with: { node-version: 22 }
      - run: cd apps/portal-admin && pnpm install --frozen-lockfile
      - run: cd apps/portal-admin && pnpm playwright install --with-deps ${{ matrix.browser }}
      - run: cd apps/portal-admin && pnpm test:e2e --project=${{ matrix.browser }}

  docker-build:
    runs-on: ubuntu-latest
    needs: unit-tests
    steps:
      - uses: actions/checkout@v4
      - uses: docker/setup-buildx-action@v3
      - run: |
          cd apps/portal-admin
          docker build -t portal-admin:ci .
          # Verify image size budget.
          SIZE=$(docker image inspect portal-admin:ci --format='{{.Size}}')
          if [ "$SIZE" -gt 209715200 ]; then
            echo "Image too large: $SIZE bytes (budget 200 MB)"
            exit 1
          fi
```

---

## §4. Module Terraform GCP — détails complets

### 4.1 Layout

```
terraform/modules/gcp/
├── main.tf                            # composition + locals
├── variables.tf                       # inputs
├── outputs.tf                         # outputs
├── versions.tf                        # provider pins
├── vpc.tf                             # VPC + subnets + NAT
├── gke.tf                             # GKE cluster + node pools
├── cloudsql.tf                        # Cloud SQL Postgres HA Regional
├── memorystore.tf                     # Memorystore Redis
├── kms.tf                             # KMS keyring + BYOK key
├── iam.tf                             # Workload Identity + service accounts
├── gcs.tf                             # GCS audit bucket
├── secretmanager.tf                   # Secret Manager entries
├── README.md
└── examples/
    └── basic/
        └── main.tf
```

### 4.2 `versions.tf`

```hcl
terraform {
  required_version = ">= 1.6, < 2.0"

  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "~> 5.30"
    }
    google-beta = {
      source  = "hashicorp/google-beta"
      version = "~> 5.30"
    }
    random = {
      source  = "hashicorp/random"
      version = "~> 3.6"
    }
  }
}
```

### 4.3 `variables.tf` (extraits clés)

```hcl
variable "project_id" {
  description = "GCP project ID."
  type        = string

  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{4,28}[a-z0-9]$", var.project_id))
    error_message = "Project ID must match GCP naming rules."
  }
}

variable "region" {
  description = "GCP region (e.g. europe-west1, us-central1)."
  type        = string
}

variable "zones" {
  description = "GCP zones for multi-zone deployment."
  type        = list(string)
  default     = []
}

variable "tenant" {
  description = "Tenant slug, used as resource prefix."
  type        = string
  validation {
    condition     = can(regex("^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$", var.tenant))
    error_message = "Tenant slug must match [a-z0-9-]{3,32}."
  }
}

variable "data_residency" {
  description = "Data residency requirement."
  type        = string
  default     = "any"
}

variable "gke_version" {
  description = "GKE Kubernetes version."
  type        = string
  default     = "1.30"
}

variable "gke_release_channel" {
  description = "GKE release channel (RAPID / REGULAR / STABLE)."
  type        = string
  default     = "REGULAR"
}

variable "gke_use_autopilot" {
  description = "Use GKE Autopilot (managed nodes) instead of standard."
  type        = bool
  default     = false
}

variable "cloudsql_tier" {
  description = "Cloud SQL machine tier (e.g. db-custom-4-16384)."
  type        = string
  default     = "db-custom-4-16384"
}

variable "cloudsql_disk_size_gb" {
  description = "Cloud SQL disk size in GB."
  type        = number
  default     = 100
}

variable "redis_tier" {
  description = "Memorystore tier (BASIC / STANDARD_HA)."
  type        = string
  default     = "STANDARD_HA"
}

variable "redis_memory_size_gb" {
  description = "Memorystore memory size in GB."
  type        = number
  default     = 4
}

variable "enable_kms_byok" {
  description = "Provision a customer-managed KMS key for BYOK."
  type        = bool
  default     = true
}

variable "audit_bucket_retention_days" {
  description = "GCS audit bucket retention in days (>= 365)."
  type        = number
  default     = 2555
}
```

### 4.4 `vpc.tf`

```hcl
resource "google_compute_network" "vpc" {
  project                 = var.project_id
  name                    = "${local.prefix}-vpc"
  auto_create_subnetworks = false
  routing_mode            = "REGIONAL"
}

resource "google_compute_subnetwork" "private" {
  project       = var.project_id
  name          = "${local.prefix}-private"
  region        = var.region
  network       = google_compute_network.vpc.id
  ip_cidr_range = "10.42.0.0/20"

  private_ip_google_access = true

  secondary_ip_range {
    range_name    = "pods"
    ip_cidr_range = "10.43.0.0/16"
  }

  secondary_ip_range {
    range_name    = "services"
    ip_cidr_range = "10.44.0.0/20"
  }

  log_config {
    aggregation_interval = "INTERVAL_5_SEC"
    flow_sampling        = 0.5
    metadata             = "INCLUDE_ALL_METADATA"
  }
}

resource "google_compute_router" "router" {
  project = var.project_id
  name    = "${local.prefix}-router"
  region  = var.region
  network = google_compute_network.vpc.id
}

resource "google_compute_router_nat" "nat" {
  project                            = var.project_id
  name                               = "${local.prefix}-nat"
  router                             = google_compute_router.router.name
  region                             = var.region
  nat_ip_allocate_option             = "AUTO_ONLY"
  source_subnetwork_ip_ranges_to_nat = "ALL_SUBNETWORKS_ALL_IP_RANGES"

  log_config {
    enable = true
    filter = "ERRORS_ONLY"
  }
}

# Private Service Access for Cloud SQL + Memorystore.
resource "google_compute_global_address" "private_service_range" {
  project       = var.project_id
  name          = "${local.prefix}-private-svc"
  purpose       = "VPC_PEERING"
  address_type  = "INTERNAL"
  prefix_length = 16
  network       = google_compute_network.vpc.id
}

resource "google_service_networking_connection" "private_vpc_connection" {
  network                 = google_compute_network.vpc.id
  service                 = "servicenetworking.googleapis.com"
  reserved_peering_ranges = [google_compute_global_address.private_service_range.name]
}
```

### 4.5 `gke.tf`

```hcl
resource "google_container_cluster" "primary" {
  project  = var.project_id
  name     = "${local.prefix}-gke"
  location = var.region

  release_channel {
    channel = var.gke_release_channel
  }

  network    = google_compute_network.vpc.id
  subnetwork = google_compute_subnetwork.private.id

  ip_allocation_policy {
    cluster_secondary_range_name  = "pods"
    services_secondary_range_name = "services"
  }

  private_cluster_config {
    enable_private_nodes    = true
    enable_private_endpoint = false
    master_ipv4_cidr_block  = "172.16.0.0/28"
  }

  master_authorized_networks_config {
    dynamic "cidr_blocks" {
      for_each = var.gke_master_authorized_networks
      content {
        cidr_block   = cidr_blocks.value
        display_name = "authorized-${index(var.gke_master_authorized_networks, cidr_blocks.value)}"
      }
    }
  }

  workload_identity_config {
    workload_pool = "${var.project_id}.svc.id.goog"
  }

  addons_config {
    horizontal_pod_autoscaling { disabled = false }
    http_load_balancing { disabled = false }
    network_policy_config { disabled = false }
  }

  database_encryption {
    state    = var.enable_kms_byok ? "ENCRYPTED" : "DECRYPTED"
    key_name = var.enable_kms_byok ? google_kms_crypto_key.gke_etcd[0].id : null
  }

  logging_service    = "logging.googleapis.com/kubernetes"
  monitoring_service = "monitoring.googleapis.com/kubernetes"

  # Remove default node pool — we manage node pools separately.
  remove_default_node_pool = true
  initial_node_count       = 1

  resource_labels = local.common_labels

  deletion_protection = var.environment == "prod"
}

resource "google_container_node_pool" "primary" {
  project    = var.project_id
  name       = "${local.prefix}-primary"
  location   = var.region
  cluster    = google_container_cluster.primary.name
  node_count = 1            # initial ; autoscaling takes over

  autoscaling {
    min_node_count = 3
    max_node_count = 12
    location_policy = "BALANCED"
  }

  management {
    auto_repair  = true
    auto_upgrade = true
  }

  node_config {
    machine_type = "e2-standard-4"
    disk_size_gb = 100
    disk_type    = "pd-balanced"

    service_account = google_service_account.gke_node.email
    oauth_scopes = ["https://www.googleapis.com/auth/cloud-platform"]

    workload_metadata_config {
      mode = "GKE_METADATA"
    }

    shielded_instance_config {
      enable_secure_boot          = true
      enable_integrity_monitoring = true
    }

    labels = local.common_labels
  }
}
```

### 4.6 `cloudsql.tf`

```hcl
resource "random_password" "cloudsql" {
  length      = 48
  special     = true
}

resource "google_sql_database_instance" "efe" {
  project          = var.project_id
  name             = "${local.prefix}-postgres"
  database_version = "POSTGRES_16"
  region           = var.region

  depends_on = [google_service_networking_connection.private_vpc_connection]

  settings {
    tier              = var.cloudsql_tier
    availability_type = "REGIONAL"          # HA — primary + standby in different zones
    disk_size         = var.cloudsql_disk_size_gb
    disk_type         = "PD_SSD"
    disk_autoresize   = true

    backup_configuration {
      enabled                        = true
      start_time                     = "02:00"
      point_in_time_recovery_enabled = true
      transaction_log_retention_days = 7
      backup_retention_settings {
        retained_backups = 14
        retention_unit   = "COUNT"
      }
    }

    ip_configuration {
      ipv4_enabled                                  = false
      private_network                               = google_compute_network.vpc.id
      enable_private_path_for_google_cloud_services = true
    }

    insights_config {
      query_insights_enabled  = true
      query_string_length     = 1024
      record_application_tags = true
      record_client_address   = false
    }

    database_flags {
      name  = "log_min_duration_statement"
      value = "1000"
    }
    database_flags {
      name  = "log_statement"
      value = "ddl"
    }

    user_labels = local.common_labels
  }

  encryption_key_name = var.enable_kms_byok ? google_kms_crypto_key.cloudsql[0].id : null

  deletion_protection = var.environment == "prod"
}

resource "google_sql_database" "efe" {
  project  = var.project_id
  name     = "efe"
  instance = google_sql_database_instance.efe.name
}

resource "google_sql_user" "admin" {
  project  = var.project_id
  name     = "efe_admin"
  instance = google_sql_database_instance.efe.name
  password = random_password.cloudsql.result
}
```

### 4.7 `memorystore.tf`

```hcl
resource "google_redis_instance" "efe" {
  project        = var.project_id
  name           = "${local.prefix}-redis"
  region         = var.region
  tier           = var.redis_tier
  memory_size_gb = var.redis_memory_size_gb

  authorized_network      = google_compute_network.vpc.id
  connect_mode            = "PRIVATE_SERVICE_ACCESS"
  redis_version           = "REDIS_7_0"

  auth_enabled            = true
  transit_encryption_mode = "SERVER_AUTHENTICATION"

  persistence_config {
    persistence_mode    = "RDB"
    rdb_snapshot_period = "ONE_HOUR"
  }

  redis_configs = {
    maxmemory-policy = "noeviction"
  }

  labels = local.common_labels
}

# Note : Memorystore does NOT support Redis Stack modules (HNSW vector search).
# For tenants needing Redis Stack, externalRedis.enabled=false in Helm chart
# (uses in-cluster Redis Stack StatefulSet from Sprint B).
```

> **⚠️ Limitation GCP** : Memorystore Redis Standard does NOT support Redis Stack modules (vector search). Tenants enabling Mode-3 vector search MUST use the in-cluster `redis-stack` Helm sub-chart instead of Memorystore. Documented in README.md.

### 4.8 `kms.tf`

```hcl
resource "google_kms_key_ring" "main" {
  project  = var.project_id
  name     = "${local.prefix}-keyring"
  location = var.region
}

resource "google_kms_crypto_key" "byok" {
  count = var.enable_kms_byok ? 1 : 0

  name            = "${local.prefix}-byok"
  key_ring        = google_kms_key_ring.main.id
  rotation_period = "7776000s" # 90 days

  version_template {
    algorithm        = "GOOGLE_SYMMETRIC_ENCRYPTION"
    protection_level = "SOFTWARE"
  }

  labels = merge(local.common_labels, { purpose = "byok" })
}

resource "google_kms_crypto_key" "cloudsql" {
  count = var.enable_kms_byok ? 1 : 0

  name            = "${local.prefix}-cloudsql"
  key_ring        = google_kms_key_ring.main.id
  rotation_period = "7776000s"
}

resource "google_kms_crypto_key" "gke_etcd" {
  count = var.enable_kms_byok ? 1 : 0

  name            = "${local.prefix}-gke-etcd"
  key_ring        = google_kms_key_ring.main.id
  rotation_period = "7776000s"
}

resource "google_kms_crypto_key" "gcs_audit" {
  count = var.enable_kms_byok ? 1 : 0

  name            = "${local.prefix}-gcs-audit"
  key_ring        = google_kms_key_ring.main.id
  rotation_period = "7776000s"
}

# Grant Cloud SQL service account decrypt access on cloudsql key.
resource "google_kms_crypto_key_iam_member" "cloudsql_decrypter" {
  count = var.enable_kms_byok ? 1 : 0

  crypto_key_id = google_kms_crypto_key.cloudsql[0].id
  role          = "roles/cloudkms.cryptoKeyEncrypterDecrypter"
  member        = "serviceAccount:service-${data.google_project.current.number}@gcp-sa-cloud-sql.iam.gserviceaccount.com"
}

data "google_project" "current" {
  project_id = var.project_id
}
```

### 4.9 `iam.tf` — Workload Identity

```hcl
# GKE node default service account (least privilege).
resource "google_service_account" "gke_node" {
  project      = var.project_id
  account_id   = "${local.prefix}-gke-node"
  display_name = "GKE node SA for ${local.prefix}"
}

resource "google_project_iam_member" "gke_node_logging" {
  project = var.project_id
  role    = "roles/logging.logWriter"
  member  = "serviceAccount:${google_service_account.gke_node.email}"
}

resource "google_project_iam_member" "gke_node_metrics" {
  project = var.project_id
  role    = "roles/monitoring.metricWriter"
  member  = "serviceAccount:${google_service_account.gke_node.email}"
}

# efe-server Workload Identity SA (mapped to K8s SA "efe:efe-server").
resource "google_service_account" "efe_server" {
  project      = var.project_id
  account_id   = "${local.prefix}-efe-server"
  display_name = "efe-server WI SA for ${local.prefix}"
}

resource "google_service_account_iam_member" "efe_server_wi" {
  service_account_id = google_service_account.efe_server.id
  role               = "roles/iam.workloadIdentityUser"
  member             = "serviceAccount:${var.project_id}.svc.id.goog[efe/efe-server]"
}

# efe-server can use BYOK key.
resource "google_kms_crypto_key_iam_member" "efe_server_byok" {
  count = var.enable_kms_byok ? 1 : 0

  crypto_key_id = google_kms_crypto_key.byok[0].id
  role          = "roles/cloudkms.cryptoKeyEncrypterDecrypter"
  member        = "serviceAccount:${google_service_account.efe_server.email}"
}

# efe-server can write to GCS audit bucket.
resource "google_storage_bucket_iam_member" "efe_server_audit_writer" {
  bucket = google_storage_bucket.audit.name
  role   = "roles/storage.objectCreator"
  member = "serviceAccount:${google_service_account.efe_server.email}"
}
```

### 4.10 `gcs.tf`

```hcl
resource "google_storage_bucket" "audit" {
  project  = var.project_id
  name     = "${local.prefix}-audit"
  location = var.region

  storage_class               = "STANDARD"
  uniform_bucket_level_access = true
  public_access_prevention    = "enforced"

  versioning {
    enabled = true
  }

  encryption {
    default_kms_key_name = var.enable_kms_byok ? google_kms_crypto_key.byok[0].id : null
  }

  lifecycle_rule {
    condition {
      age = 90
    }
    action {
      type          = "SetStorageClass"
      storage_class = "NEARLINE"
    }
  }

  lifecycle_rule {
    condition {
      age = 365
    }
    action {
      type          = "SetStorageClass"
      storage_class = "COLDLINE"
    }
  }

  lifecycle_rule {
    condition {
      age = var.audit_bucket_retention_days
    }
    action {
      type = "Delete"
    }
  }

  retention_policy {
    retention_period = var.audit_bucket_retention_days * 86400
    is_locked        = false
  }

  labels = merge(local.common_labels, { purpose = "audit-archive" })
}
```

### 4.11 `secretmanager.tf`

```hcl
resource "google_secret_manager_secret" "rds_credentials" {
  project   = var.project_id
  secret_id = "${local.prefix}-cloudsql-credentials"

  replication {
    auto {
      customer_managed_encryption {
        kms_key_name = var.enable_kms_byok ? google_kms_crypto_key.byok[0].id : null
      }
    }
  }

  labels = local.common_labels
}

resource "google_secret_manager_secret_version" "rds_credentials" {
  secret = google_secret_manager_secret.rds_credentials.id
  secret_data = jsonencode({
    username           = google_sql_user.admin.name
    password           = random_password.cloudsql.result
    host               = google_sql_database_instance.efe.private_ip_address
    port               = 5432
    dbname             = "efe"
    connection_string  = "postgresql://${google_sql_user.admin.name}:${urlencode(random_password.cloudsql.result)}@${google_sql_database_instance.efe.private_ip_address}:5432/efe?sslmode=require"
  })
}

resource "random_password" "audit_hmac_seed" {
  length  = 64
  special = false
}

resource "google_secret_manager_secret" "audit_hmac_seed" {
  project   = var.project_id
  secret_id = "${local.prefix}-audit-hmac-seed"

  replication {
    auto {}
  }

  labels = merge(local.common_labels, {
    purpose      = "audit-chain-hmac"
    criticality  = "p0"
  })
}

resource "google_secret_manager_secret_version" "audit_hmac_seed" {
  secret      = google_secret_manager_secret.audit_hmac_seed.id
  secret_data = random_password.audit_hmac_seed.result

  lifecycle {
    ignore_changes = [secret_data]
  }
}
```

### 4.12 `outputs.tf`

```hcl
output "kubeconfig_command" {
  value = "gcloud container clusters get-credentials ${google_container_cluster.primary.name} --region ${var.region} --project ${var.project_id}"
}

output "gke_cluster_name" {
  value = google_container_cluster.primary.name
}

output "cloudsql_secret_id" {
  value = google_secret_manager_secret.rds_credentials.secret_id
}

output "redis_endpoint" {
  value = google_redis_instance.efe.host
}

output "redis_auth_string" {
  value     = google_redis_instance.efe.auth_string
  sensitive = true
}

output "byok_kms_key" {
  value = var.enable_kms_byok ? google_kms_crypto_key.byok[0].id : null
}

output "audit_bucket_name" {
  value = google_storage_bucket.audit.name
}

output "efe_server_workload_identity_sa" {
  value = google_service_account.efe_server.email
}

output "audit_hmac_secret_id" {
  value = google_secret_manager_secret.audit_hmac_seed.secret_id
}
```

---

## §5. Module Terraform Azure

### 5.1 Layout

```
terraform/modules/azure/
├── main.tf
├── variables.tf
├── outputs.tf
├── versions.tf
├── vnet.tf
├── aks.tf
├── postgres.tf
├── redis.tf
├── keyvault.tf
├── identity.tf                        # User-Assigned Managed Identity + WI
├── blob.tf
├── README.md
└── examples/
    └── basic/
        └── main.tf
```

### 5.2 `versions.tf`

```hcl
terraform {
  required_version = ">= 1.6, < 2.0"

  required_providers {
    azurerm = {
      source  = "hashicorp/azurerm"
      version = "~> 3.108"
    }
    azuread = {
      source  = "hashicorp/azuread"
      version = "~> 2.50"
    }
    random = {
      source  = "hashicorp/random"
      version = "~> 3.6"
    }
  }
}
```

### 5.3 `aks.tf`

```hcl
resource "azurerm_kubernetes_cluster" "efe" {
  name                = "${local.prefix}-aks"
  location            = azurerm_resource_group.efe.location
  resource_group_name = azurerm_resource_group.efe.name
  dns_prefix          = "${local.prefix}-dns"
  kubernetes_version  = var.aks_version

  oidc_issuer_enabled       = true            # required for Workload Identity
  workload_identity_enabled = true

  default_node_pool {
    name           = "system"
    node_count     = 3
    vm_size        = "Standard_D4ds_v5"
    vnet_subnet_id = azurerm_subnet.aks.id
    type           = "VirtualMachineScaleSets"
    auto_scaling_enabled = true
    min_count      = 3
    max_count      = 6

    only_critical_addons_enabled = true

    upgrade_settings {
      max_surge = "10%"
    }
  }

  identity {
    type = "SystemAssigned"
  }

  network_profile {
    network_plugin    = "azure"
    network_plugin_mode = "overlay"
    network_policy    = "calico"
    service_cidr      = "10.45.0.0/16"
    dns_service_ip    = "10.45.0.10"
  }

  api_server_access_profile {
    authorized_ip_ranges = var.aks_authorized_ip_ranges
  }

  azure_active_directory_role_based_access_control {
    azure_rbac_enabled = true
  }

  oms_agent {
    log_analytics_workspace_id = azurerm_log_analytics_workspace.efe.id
  }

  microsoft_defender {
    log_analytics_workspace_id = azurerm_log_analytics_workspace.efe.id
  }

  tags = local.common_tags
}

resource "azurerm_kubernetes_cluster_node_pool" "user" {
  name                  = "user"
  kubernetes_cluster_id = azurerm_kubernetes_cluster.efe.id
  vm_size               = "Standard_D4ds_v5"
  vnet_subnet_id        = azurerm_subnet.aks.id

  auto_scaling_enabled = true
  min_count            = 3
  max_count            = 12

  upgrade_settings {
    max_surge = "10%"
  }

  tags = local.common_tags
}
```

### 5.4 `postgres.tf`

```hcl
resource "random_password" "postgres" {
  length      = 48
  special     = true
}

resource "azurerm_postgresql_flexible_server" "efe" {
  name                          = "${local.prefix}-postgres"
  resource_group_name           = azurerm_resource_group.efe.name
  location                      = azurerm_resource_group.efe.location
  version                       = "16"
  delegated_subnet_id           = azurerm_subnet.postgres.id
  private_dns_zone_id           = azurerm_private_dns_zone.postgres.id
  administrator_login           = "efe_admin"
  administrator_password        = random_password.postgres.result
  zone                          = "1"
  storage_mb                    = var.postgres_storage_mb
  storage_tier                  = "P30"

  sku_name = var.postgres_sku_name

  high_availability {
    mode                      = "ZoneRedundant"
    standby_availability_zone = "2"
  }

  backup_retention_days        = 14
  geo_redundant_backup_enabled = true

  customer_managed_key {
    key_vault_key_id = var.enable_kms_byok ? azurerm_key_vault_key.postgres[0].id : null
  }

  authentication {
    password_auth_enabled         = true
    active_directory_auth_enabled = false
  }

  tags = local.common_tags
}

resource "azurerm_postgresql_flexible_server_database" "efe" {
  name      = "efe"
  server_id = azurerm_postgresql_flexible_server.efe.id
  collation = "en_US.utf8"
  charset   = "utf8"
}

resource "azurerm_postgresql_flexible_server_configuration" "log_min_duration" {
  name      = "log_min_duration_statement"
  server_id = azurerm_postgresql_flexible_server.efe.id
  value     = "1000"
}

resource "azurerm_postgresql_flexible_server_configuration" "log_statement" {
  name      = "log_statement"
  server_id = azurerm_postgresql_flexible_server.efe.id
  value     = "ddl"
}
```

### 5.5 `redis.tf`

```hcl
resource "azurerm_redis_cache" "efe" {
  name                          = "${local.prefix}-redis"
  location                      = azurerm_resource_group.efe.location
  resource_group_name           = azurerm_resource_group.efe.name
  capacity                      = var.redis_capacity        # 1 = ~6 GB
  family                        = "P"                       # Premium
  sku_name                      = "Premium"
  non_ssl_port_enabled          = false
  minimum_tls_version           = "1.2"
  redis_version                 = "6"

  public_network_access_enabled = false

  redis_configuration {
    maxmemory_policy = "noeviction"
    rdb_backup_enabled = true
    rdb_backup_frequency = 60
    rdb_backup_max_snapshot_count = 3
    rdb_storage_connection_string = azurerm_storage_account.redis_backup.primary_blob_connection_string
  }

  tags = local.common_tags
}

resource "azurerm_private_endpoint" "redis" {
  name                = "${local.prefix}-redis-pe"
  location            = azurerm_resource_group.efe.location
  resource_group_name = azurerm_resource_group.efe.name
  subnet_id           = azurerm_subnet.redis.id

  private_service_connection {
    name                           = "${local.prefix}-redis-psc"
    private_connection_resource_id = azurerm_redis_cache.efe.id
    is_manual_connection           = false
    subresource_names              = ["redisCache"]
  }

  tags = local.common_tags
}
```

### 5.6 `keyvault.tf`

```hcl
data "azurerm_client_config" "current" {}

resource "azurerm_key_vault" "efe" {
  name                = "${local.prefix}-kv"
  location            = azurerm_resource_group.efe.location
  resource_group_name = azurerm_resource_group.efe.name
  tenant_id           = data.azurerm_client_config.current.tenant_id

  sku_name = "premium"

  enabled_for_disk_encryption     = true
  enabled_for_template_deployment = false
  enabled_for_deployment          = false
  purge_protection_enabled        = true
  soft_delete_retention_days      = 30

  enable_rbac_authorization = true

  network_acls {
    bypass         = "AzureServices"
    default_action = "Deny"
    virtual_network_subnet_ids = [azurerm_subnet.aks.id]
  }

  tags = local.common_tags
}

resource "azurerm_key_vault_key" "byok" {
  count = var.enable_kms_byok ? 1 : 0

  name         = "${local.prefix}-byok"
  key_vault_id = azurerm_key_vault.efe.id
  key_type     = "RSA"
  key_size     = 2048
  key_opts     = ["decrypt", "encrypt", "wrapKey", "unwrapKey", "sign", "verify"]

  rotation_policy {
    automatic {
      time_after_creation = "P90D"
    }
    expire_after         = "P365D"
    notify_before_expiry = "P30D"
  }

  tags = local.common_tags
}

resource "azurerm_key_vault_key" "postgres" {
  count = var.enable_kms_byok ? 1 : 0

  name         = "${local.prefix}-postgres"
  key_vault_id = azurerm_key_vault.efe.id
  key_type     = "RSA"
  key_size     = 2048
  key_opts     = ["decrypt", "encrypt", "wrapKey", "unwrapKey"]
}

# HMAC seed Secret.
resource "random_password" "audit_hmac_seed" {
  length  = 64
  special = false
}

resource "azurerm_key_vault_secret" "audit_hmac_seed" {
  name         = "${local.prefix}-audit-hmac-seed"
  value        = random_password.audit_hmac_seed.result
  key_vault_id = azurerm_key_vault.efe.id

  content_type = "audit-chain-hmac/p0"

  lifecycle {
    ignore_changes = [value]
  }
}
```

### 5.7 `identity.tf` — Azure AD Workload Identity

```hcl
resource "azurerm_user_assigned_identity" "efe_server" {
  name                = "${local.prefix}-efe-server-uai"
  resource_group_name = azurerm_resource_group.efe.name
  location            = azurerm_resource_group.efe.location

  tags = local.common_tags
}

resource "azurerm_federated_identity_credential" "efe_server" {
  name                = "${local.prefix}-efe-server-fic"
  resource_group_name = azurerm_resource_group.efe.name
  parent_id           = azurerm_user_assigned_identity.efe_server.id
  audience            = ["api://AzureADTokenExchange"]
  issuer              = azurerm_kubernetes_cluster.efe.oidc_issuer_url
  subject             = "system:serviceaccount:efe:efe-server"
}

# efe-server can use BYOK key.
resource "azurerm_role_assignment" "efe_server_kv_user" {
  count = var.enable_kms_byok ? 1 : 0

  scope                = azurerm_key_vault.efe.id
  role_definition_name = "Key Vault Crypto User"
  principal_id         = azurerm_user_assigned_identity.efe_server.principal_id
}

# efe-server can read HMAC seed.
resource "azurerm_role_assignment" "efe_server_kv_secrets" {
  scope                = azurerm_key_vault.efe.id
  role_definition_name = "Key Vault Secrets User"
  principal_id         = azurerm_user_assigned_identity.efe_server.principal_id
}

# efe-server can write to audit blob.
resource "azurerm_role_assignment" "efe_server_blob" {
  scope                = azurerm_storage_container.audit.resource_manager_id
  role_definition_name = "Storage Blob Data Contributor"
  principal_id         = azurerm_user_assigned_identity.efe_server.principal_id
}
```

### 5.8 `blob.tf`

```hcl
resource "azurerm_storage_account" "audit" {
  name                     = replace("${local.prefix}audit", "-", "")
  resource_group_name      = azurerm_resource_group.efe.name
  location                 = azurerm_resource_group.efe.location
  account_tier             = "Standard"
  account_replication_type = "GRS"            # geo-redundant
  account_kind             = "StorageV2"

  min_tls_version          = "TLS1_2"
  https_traffic_only_enabled = true
  public_network_access_enabled = false

  blob_properties {
    versioning_enabled = true

    delete_retention_policy {
      days = 7
    }

    container_delete_retention_policy {
      days = 30
    }
  }

  customer_managed_key {
    key_vault_key_id          = var.enable_kms_byok ? azurerm_key_vault_key.byok[0].id : null
    user_assigned_identity_id = azurerm_user_assigned_identity.storage.id
  }

  identity {
    type = "UserAssigned"
    identity_ids = [azurerm_user_assigned_identity.storage.id]
  }

  tags = local.common_tags
}

resource "azurerm_user_assigned_identity" "storage" {
  name                = "${local.prefix}-storage-uai"
  resource_group_name = azurerm_resource_group.efe.name
  location            = azurerm_resource_group.efe.location
}

resource "azurerm_storage_container" "audit" {
  name                  = "audit"
  storage_account_name  = azurerm_storage_account.audit.name
  container_access_type = "private"
}

resource "azurerm_storage_management_policy" "audit" {
  storage_account_id = azurerm_storage_account.audit.id

  rule {
    name    = "audit-retention"
    enabled = true

    filters {
      blob_types = ["blockBlob"]
      prefix_match = ["audit/"]
    }

    actions {
      base_blob {
        tier_to_cool_after_days_since_modification_greater_than    = 90
        tier_to_archive_after_days_since_modification_greater_than = 365
        delete_after_days_since_modification_greater_than          = var.audit_bucket_retention_days
      }
    }
  }
}
```

---

## §6. Runbooks RB-07 à RB-12

### 6.1 RB-07 : Latence p99 classify > 2× SLO sur 30 min

```markdown
# RB-07 — Latence p99 classify > 2× SLO

## Métadonnées
- **Sévérité** : P2
- **MTTR cible** : 1 h
- **Fréquence attendue** : 2-5 / mois
- **Owner primaire** : Perf Ops
- **Owner backup** : SRE

## Symptômes
- Alerte Prometheus : `EfeClassifyP99HighLatency` (cf. Sprint B § 4.6.3)
- Grafana panel "p99 Latency" → > 2× SLO (~ 500 ms vs 250 ms baseline)
- Logs efe-server : pas d'erreur explicite mais `request took 1234ms`

## Diagnostic
1. Identifier la dimension dégradée :
   ```bash
   kubectl exec -n efe deploy/efe-server -- \
     curl -s http://localhost:9090/metrics \
     | grep -E "efe_classify_duration_ms_(bucket|sum|count)"
   ```
2. Repository des slow queries Postgres :
   ```sql
   SELECT query, calls, mean_exec_time, max_exec_time
     FROM pg_stat_statements
    WHERE mean_exec_time > 100
    ORDER BY mean_exec_time DESC
    LIMIT 10;
   ```
3. Vérifier la pression mémoire pods :
   ```bash
   kubectl top pods -n efe
   ```

## Procédure
### Étape 1 : Stabiliser
- Si HPA scale-up déclenché → laisser converger.
- Sinon : forcer scale-up manuel :
  ```bash
  kubectl scale -n efe deploy/efe-server --replicas=12
  ```

### Étape 2 : Diagnostiquer cause racine

| Cause probable | Indicateur | Solution |
|----------------|------------|----------|
| LLM provider lent | provider p99 > 2× baseline | Cf. RB-02 (breaker) |
| DB requêtes slow | pg_stat_statements top | Add index ou optimize query |
| Cache Redis miss spike | hit ratio < 70 % | Investigate cache invalidation pattern |
| Tenant noisy-neighbor | concentration RPS sur 1 tenant | Augment per-tenant rate-limit |
| Garbage collection | mem RSS > 80 % limit | Bump memory limit |

### Étape 3 : Corriger
- Cas DB : ajouter index ou ANALYZE table
- Cas cache : `redis-cli FLUSHDB` (CAUTION) ou wait for TTL expiry
- Cas noisy-neighbor : `efe-admin quotas set <tenant> --max-rps <lower>`

### Étape 4 : Vérifier
- Attendre 10 min ; p99 doit revenir < SLO

## Procédure de validation
Force latence en staging :
```bash
kubectl exec -n efe-staging deploy/efe-server -- \
  curl -X POST http://localhost:8081/internal/inject-latency \
  -d '{"ms":600,"duration":"5m"}'
```
Doit déclencher l'alerte ≤ 10 min.
```

### 6.2 RB-08 : Migration DB échouée

```markdown
# RB-08 — Migration DB échouée (sqlx migrate run failure)

## Métadonnées
- **Sévérité** : P1 (blocking deployment)
- **MTTR cible** : 1 h
- **Owner primaire** : DBA on-call
- **Owner backup** : Backend Lead

## Symptômes
- Helm post-install hook Job `*-migrate` en `Failed`
- Logs : `sqlx migration X failed: <error>`
- New pods efe-server in CrashLoopBackOff (DB schema mismatch)

## Diagnostic
1. Logs Job migrate :
   ```bash
   kubectl logs -n efe job/efe-stack-migrate
   ```
2. État table `_sqlx_migrations` :
   ```sql
   SELECT version, description, success, execution_time, installed_on
     FROM _sqlx_migrations
    ORDER BY version DESC LIMIT 10;
   ```
3. Migration appliquée partiellement ? Vérifier transaction :
   ```sql
   SELECT * FROM pg_stat_activity WHERE state != 'idle';
   ```

## Procédure
### Étape 1 : Stabiliser
- **Bloquer le rollout** des nouveaux pods :
  ```bash
  kubectl rollout pause -n efe deploy/efe-server
  ```

### Étape 2 : Diagnostiquer
- Cas A : conflit data (e.g. NOT NULL ajouté avec data null)
- Cas B : SQL syntax error dans le fichier migration
- Cas C : timeout (long migration sur grosse table)

### Étape 3 : Corriger
- **Cas A** : data-fix SQL d'abord, puis re-run :
  ```sql
  -- Example : backfill nulls before NOT NULL constraint applies.
  UPDATE big_table SET new_col = 'default' WHERE new_col IS NULL;
  ```
  Puis :
  ```bash
  kubectl delete job -n efe efe-stack-migrate
  helm upgrade efe-stack ... --reset-then-reuse-values
  ```

- **Cas B** : revert via Helm rollback :
  ```bash
  helm rollback efe-stack <previous-revision>
  # Investigate fix ; re-deploy with corrected SQL.
  ```

- **Cas C** : étendre `backoffLimit` + `activeDeadlineSeconds` du Job :
  ```bash
  helm upgrade efe-stack ... \
    --set migrations.backoffLimit=10 \
    --set migrations.activeDeadlineSeconds=3600
  ```

### Étape 4 : Vérifier
```bash
kubectl get job -n efe efe-stack-migrate
# Status : Complete
kubectl rollout resume -n efe deploy/efe-server
kubectl wait -n efe pod -l app.kubernetes.io/name=efe-server \
  --for=condition=Ready --timeout=120s
```

## Communication
- Slack #efe-incidents
- Statuspage si downtime > 15 min

## Post-incident
- Test régression : ajouter le scenario data-fix dans pre-deploy CI
- Améliorer la migration : la rendre idempotente + supportant data null
```

### 6.3 RB-09 : `mode3_review_queue` saturée (> 1000 pending)

```markdown
# RB-09 — mode3_review_queue saturée

## Métadonnées
- **Sévérité** : P2 (compliance backlog)
- **MTTR cible** : 2 h
- **Owner primaire** : Compliance / DPO tenant
- **Owner backup** : Account Executive

## Symptômes
- Métrique : `efe_mode3_review_queue_pending > 1000`
- Logs : `Mode-3 call queued for review (threshold not reached + max_turns hit)`
- Email DPO tenant : « X pending reviews to action »

## Diagnostic
1. Per-tenant queue size :
   ```sql
   SELECT tenant_id, COUNT(*) AS pending,
          MIN(requested_at) AS oldest
     FROM mode3_review_queue
    WHERE status = 'pending'
    GROUP BY tenant_id
    ORDER BY pending DESC;
   ```
2. Convergence rate dégradé ?
   ```bash
   curl -s http://prometheus:9090/api/v1/query?query=\
'sum(rate(efe_mode3_calls_total{converged="true"}[1h])) / sum(rate(efe_mode3_calls_total[1h]))'
   ```

## Procédure
### Étape 1 : Identifier le tenant impacté
Si un seul tenant a > 80 % du backlog → cible spécifique.

### Étape 2 : Action
- **Court terme** : DPO tenant approve/reject reviews via portail (`/reviews`)
- **Moyen terme** : tuner `max_turns` upward (laisse plus de chance de converger) :
  ```bash
  efe-admin tenant update <tenant> --max-mode3-turns 5 --yes
  ```
- **Long terme** : analyser le pattern qui ne converge pas → ajout d'anchors ou prompt fix

### Étape 3 : Anti-spam
Si > 5000 pending → temporairement pause Mode-3 :
```bash
efe-admin tenant update <tenant> --disable-mode3 --yes
```
DPO traite le backlog, puis re-enable.

## Procédure de validation
Inject 1500 pending en staging :
```sql
INSERT INTO mode3_review_queue (review_token, tenant_id, ...) ...
```
Doit déclencher l'alerte ≤ 5 min.
```

### 6.4 RB-10 : Certificat TLS expirant < 30 j

```markdown
# RB-10 — Certificat TLS expirant (< 30 j)

## Métadonnées
- **Sévérité** : P2 (downtime risk)
- **MTTR cible** : 2 h
- **Owner primaire** : K8s Ops
- **Owner backup** : Security Lead

## Symptômes
- cert-manager event : `Certificate <name> renewal failed`
- Browser warning : « certificate will expire in X days »
- Métrique : `certmanager_certificate_expiration_timestamp_seconds < now() + 30d`

## Diagnostic
```bash
kubectl get certs -A | grep -i efe
kubectl describe cert -n efe portal-admin-tls
kubectl logs -n cert-manager deploy/cert-manager --tail=100
```

## Procédure
### Étape 1 : Identifier la cause
- Cas A : ACME challenge fail (DNS-01 → record manquant ; HTTP-01 → endpoint inacessible)
- Cas B : Issuer disabled
- Cas C : rate limit Let's Encrypt

### Étape 2 : Corriger
- **Cas A DNS-01** :
  ```bash
  # Trigger renewal manually
  kubectl annotate cert -n efe portal-admin-tls \
    cert-manager.io/issue-temporary-certificate="true"
  kubectl delete cert -n efe portal-admin-tls
  helm upgrade efe-stack ... --reuse-values
  ```
- **Cas B** : vérifier ClusterIssuer state :
  ```bash
  kubectl get clusterissuer letsencrypt-prod -o yaml
  ```
- **Cas C** : passer en staging Let's Encrypt issuer (test) ou attendre fenêtre rate limit (1 sem)

### Étape 3 : Vérifier
```bash
kubectl get cert -n efe portal-admin-tls -o yaml | grep -A 5 status
# Status: Ready=True
```

## Procédure de validation
Force expiry simulation en staging :
```bash
kubectl annotate cert -n efe-staging test-cert \
  cert-manager.io/forced-expiration="2026-06-01T00:00:00Z"
```
Doit déclencher l'alerte ≤ 1 h.
```

### 6.5 RB-11 : Quota tokens LLM tenant à 95 %

```markdown
# RB-11 — Quota tokens LLM tenant à 95 % du daily cap

## Métadonnées
- **Sévérité** : P3 (proactive notification)
- **MTTR cible** : 4 h (préventif)
- **Owner primaire** : Account Executive
- **Owner backup** : LLM Ops

## Symptômes
- Métrique : `efe_tenant_daily_tokens_used / efe_tenant_daily_tokens_cap > 0.95`
- Email tenant : « Approaching daily token limit »

## Diagnostic
```bash
efe-admin quotas get <tenant> --format json
```

## Procédure
### Étape 1 : Notifier proactive
Account Executive contacte tenant :
- Communication : « Daily token cap will be hit in ~ 1 h at current rate »
- Options : up-cap pour 24h, ou throttle requests jusqu'au reset minuit UTC

### Étape 2 : Si confirmation tenant pour up-cap
```bash
efe-admin quotas set <tenant> --max-tokens-per-day <new-cap> --yes
```

### Étape 3 : Si throttle
Pas d'action nécessaire — l'app auto-throttle quand cap hit (Mode-2/3 errors gracefully avec `BudgetExhausted`).

## Post-incident
- Pattern récurrent (> 3 fois / mois) → revoir le cap initial avec Account Executive
- Long terme : alerter à 80 % au lieu de 95 % ?
```

### 6.6 RB-12 : Failover Postgres

```markdown
# RB-12 — Failover Postgres déclenché

## Métadonnées
- **Sévérité** : P1
- **MTTR cible** : 30 min
- **Fréquence attendue** : < 1 / an / cluster
- **Owner primaire** : DBA
- **Owner backup** : SRE

## Symptômes
- Logs efe-server : `connection refused` ou `read-only transaction`
- AWS RDS event : `Multi-AZ failover completed`
- GCP Cloud SQL event : `failover from primary to standby`
- Azure PG event : `HA failover completed`

## Diagnostic
### AWS
```bash
aws rds describe-events \
  --source-identifier efe-acme-postgres \
  --source-type db-instance \
  --duration 60 | jq .Events[]
```

### GCP
```bash
gcloud sql operations list --instance=efe-acme-postgres --limit=10
```

### Azure
```bash
az postgres flexible-server show \
  --resource-group efe-acme-rg \
  --name efe-acme-postgres \
  --query 'highAvailability'
```

## Procédure
### Étape 1 : Vérifier que les apps reconnectent
```bash
kubectl logs -n efe deploy/efe-server --tail=50 \
  | grep -E "(reconnect|connection)"
```
Si pods CrashLoopBackOff → restart :
```bash
kubectl rollout restart -n efe deploy/efe-server deploy/mcp-server
```

### Étape 2 : Vérifier intégrité audit chain
```bash
kubectl exec -n efe deploy/efe-server -- \
  /app/bin/efe-verify-audit-chain --tenant=<tenant>
```
Doit retourner OK. Si broken → cf. RB-03.

### Étape 3 : Plan de failback
- AWS Multi-AZ : auto-réplication standby ↔ primary, pas d'action
- GCP HA Regional : auto-failback quand zone primaire reviendra
- Azure ZoneRedundant : auto-failback

### Étape 4 : Communication
- Statuspage : « Database failover completed at HH:MM, service restored » (< 5 min downtime expected)
- Slack #efe-incidents
- Pas d'email tenant (downtime < 1 min typique)

## Procédure de validation
Force failover en staging :
### AWS
```bash
aws rds reboot-db-instance \
  --db-instance-identifier efe-staging-postgres \
  --force-failover
```
### GCP
```bash
gcloud sql instances failover efe-staging-postgres
```
### Azure
```bash
az postgres flexible-server restart \
  --resource-group efe-staging-rg \
  --name efe-staging-postgres \
  --failover Forced
```
Doit déclencher reconnexion automatique des pods en ≤ 60 s.

## Post-incident
- Postmortem si downtime > 5 min
- Vérifier log-of-failover dans `_sqlx_migrations` (cohérence migrations)
```

---

## §7. Tests + CI

### 7.1 Tests Sprint C cumulés

| Type | Volume | Fichiers |
|------|--------|----------|
| Vitest unit | 110 | `tests/unit/` |
| Playwright E2E | 145 (× 3 browsers = 435 actuels) | `tests/e2e/` |
| terratest GCP | 8 | `terraform/modules/gcp/test/` |
| terratest Azure | 8 | `terraform/modules/azure/test/` |
| Runbook validation | +6 (cumul 12) | `scripts/runbook-validate.sh` |
| **Total Sprint C** | **+277** | |

### 7.2 CI workflows

- `.github/workflows/portal-admin-test.yml` (NEW Sprint C)
- `.github/workflows/terraform-gcp.yml` (NEW Sprint C)
- `.github/workflows/terraform-azure.yml` (NEW Sprint C)
- `.github/workflows/helm-stack-test.yml` (Sprint B — already covers portal-admin sub-chart with `enabled=false`)

---

## §8. Plan de delivery (Sprint C 5 jours)

### 8.1 J11-J15

| Jour | Livrable | Tests |
|------|----------|-------|
| **J11** | Portal Next.js skeleton : `app/` + middleware + auth + nav + page `/dashboard` + `/settings` | ~30 vitest |
| **J12** | Pages `/audit` + `/audit/[id]` + concept_stack viewer + PDF export | ~25 vitest |
| **J13** | Pages `/reviews` + `/compliance` + `/keys` + `/quotas` | ~35 vitest |
| **J14** | E2E Playwright (chromium + firefox + webkit) + Terraform GCP module complet | ~50 e2e + 8 terratest |
| **J15** | Terraform Azure + 6 runbooks RB-07..12 + audit pass + bug fixes | 8 terratest + remaining e2e + 6 validation runs |

### 8.2 Cumul tests post-Sprint C

| Couche | Pre-Sprint C | Post-Sprint C | Δ |
|--------|--------------|---------------|---|
| Rust efe-admin | 54 | 54 | — |
| Rust autres crates | 2 426 | 2 426 | — |
| Python mcp-server | 620 | 620 | — |
| Python D8/M20/M21/M22 | 73 | 73 | — |
| Helm + Terraform AWS (lint+kubeconform+tfsec+terratest) | ~25 | ~25 | — |
| Vitest portal-admin | 0 | **110** | +110 |
| Playwright portal-admin (specs Hommage) | 0 | **145** (× 3 browsers) | +145 |
| terratest GCP | 0 | **8** | +8 |
| terratest Azure | 0 | **8** | +8 |
| Runbook validations | 6 | **12** | +6 |
| **Cumul GREEN** | **~3 200** | **~3 477** | **+277** |

---

## §9. Critères d'acceptation détaillés

### 9.1 Performance

- Portal `/dashboard` Time-to-Interactive ≤ 1.5 s (P95) sur 3G
- Portal `/audit` recherche 1 M rows ≤ 800 ms (P95)
- Portal Docker image ≤ 200 MB
- Terraform GCP `apply` ≤ 15 min
- Terraform Azure `apply` ≤ 18 min

### 9.2 Sécurité

- Portal CSP strict (no `unsafe-eval`, no `unsafe-inline` JS)
- Portal session cookie `__Secure-`, `httpOnly`, `sameSite=strict`
- Cross-tenant access blocked (test E2E)
- 100 % tools mcp-server write require `confirm:true`
- Distroless container (no shell, no apt)
- tfsec sur Terraform GCP + Azure : 0 HIGH / CRITICAL post-config

### 9.3 Compliance

- AI Act Art. 14 + GDPR Art. 22 : portail expose attestation flow + concept_stack viewer
- Right-to-explanation PDF export functional (testé E2E)
- UI audit log : 100 % actions write logées (best-effort acceptable v1.0.0)

### 9.4 Backward compat

- 0 régression Sprint A+B : helm chart inchangé sauf activation portal-admin sub-chart
- 0 modification migration DB
- 0 modification mcp-server tools (portail consomme uniquement existants)

---

## §10. Risques & mitigations

| ID | Risque | Probabilité | Impact | Mitigation |
|----|--------|-------------|--------|------------|
| **C-R1** | NextAuth v5 toujours en RC à la livraison | M | H | Fallback Auth.js v4 stable ; abstraction repo `lib/auth/config.ts` |
| **C-R2** | Compliance attestation : social engineering du DPO | L | H | UI rate-limit (max 5 attempts / hour) ; double signature email confirmation ; audit log immutable |
| **C-R3** | Cross-tenant data leak via tRPC | L | C | Tests E2E `rbac-enforcement.spec.ts` ; tRPC procedures vérifient `tenant ∈ session.user.tenants` ; pen-test interne pre-release |
| **C-R4** | API key leak via portal logs | L | C | Linter test `tests/test_no_secret_logged.spec.ts` ; plaintext displayed once + immédiatement effacé du state ; audit log scrub |
| **C-R5** | Memorystore Redis pas Stack-compatible (no HNSW) | H (déjà arrivé) | M | Documenté dans README.md ; tenants Mode-3 doivent utiliser in-cluster redis-stack |
| **C-R6** | GCP Workload Identity vs Cloud SQL connector deprecation | M | M | Pin `google_sql_database_instance` provider 5.30 ; renovate-bot PR mensuelle |
| **C-R7** | Azure Flexible Server PG version sub-optimale (default v15 historique) | L | M | Pin v16 explicit ; test upgrade path |
| **C-R8** | Playwright tests flaky (3 browsers × 145 specs = 435 cases) | M | L | Playwright `retries=2` pour CI ; quarantine pattern pour spec flaky |
| **C-R9** | Bundle JS portal trop gros (> 1 MB) | M | M | Bundle analyzer in CI ; budget gate (MAX_INITIAL_KB=500) ; lazy-load charts |
| **C-R10** | Distroless image : debug pas évident en prod | M | L | Sidebar `kubectl debug` documented in RB-06 ; observability bien instrumentée (logs JSON) |

---

## §11. Annexes

### 11.1 Sample env.production pour portal-admin

```bash
# .env.production (réinjecté par Helm via Secret + ConfigMap)
NEXT_PUBLIC_VERSION=1.0.0
NODE_ENV=production
PORT=3000

OIDC_ISSUER=https://idp.acme.com/realms/efe
OIDC_CLIENT_ID=efe-portal
OIDC_CLIENT_SECRET=...                        # injected from Secret

EFE_BACKEND_GRPC_URL=http://efe-stack-efe-server:50051
EFE_MCP_HTTP_URL=http://efe-stack-mcp-server:8080
PROMETHEUS_URL=http://prometheus:9090

NEXTAUTH_URL=https://portal.acme.com
NEXTAUTH_SECRET=...                          # injected from Secret

LOG_LEVEL=info
NEXT_TELEMETRY_DISABLED=1
```

### 11.2 Sample Helm activation portal-admin

```bash
helm upgrade efe-stack ./charts/efe-stack \
  --reuse-values \
  --set portal-admin.enabled=true \
  --set portal-admin.image.tag=1.0.0 \
  --set portal-admin.sso.issuer=https://idp.acme.com/realms/efe \
  --set portal-admin.sso.clientId=efe-portal \
  --set portal-admin.ingress.host=portal.acme.com
```

### 11.3 Checklist pré-release v1.0.0

```
## Pre-flight v1.0.0 (post Sprint C)

### Code
- [ ] cargo test --workspace --all-features GREEN
- [ ] pytest mcp-server/tests/ GREEN (620 + 73 tests D8)
- [ ] pnpm test:unit GREEN (110 tests Vitest)
- [ ] pnpm test:e2e GREEN sur 3 browsers (145 × 3 specs)
- [ ] helm-unittest GREEN (25 tests)
- [ ] terratest GREEN (8 AWS + 8 GCP + 8 Azure)
- [ ] cargo audit 0 vulnerabilities HIGH/CRITICAL
- [ ] pnpm audit 0 vulnerabilities HIGH/CRITICAL
- [ ] Distroless image scan (Trivy) 0 HIGH/CRITICAL

### Docs
- [ ] CHANGELOG v1.0.0 finalisé
- [ ] docs/onboarding/SE_PLAYBOOK.md à jour
- [ ] docs/onboarding/DPO_QUICKSTART.md à jour (lien portail)
- [ ] 12 runbooks reviewed by Ops Lead
- [ ] terraform/modules/{aws,gcp,azure}/README.md à jour
- [ ] charts/efe-stack/README.md à jour

### Artefacts
- [ ] Docker images signed Sigstore (efe-server, mcp-server, portal-admin, efe-server-migrate)
- [ ] Helm chart packaged + pushed OCI registry oci://ghcr.io/efe-ai/charts/efe-stack:1.0.0
- [ ] efe-admin binaries built for 6 targets + signed
- [ ] Terraform modules tagged tf-aws/v1.0.0, tf-gcp/v1.0.0, tf-azure/v1.0.0

### Validation finale
- [ ] Onboarding 30-min reproduit avec succès par 3 SE différents (asciinema enregistré)
- [ ] Audit pass chirurgical Sprint C : ≤ 6 défauts trouvés et corrigés
- [ ] Pen-test interne portail (RBAC, CSP, CSRF, secret leak)
- [ ] DPO Acme pilote a validé le portail en preuve de concept

### Communication
- [ ] Release notes draftées + revues marketing
- [ ] Webinaire SE training programmé
- [ ] Customer-starter repo published
- [ ] HackerOne / bug bounty program scoped
```

---

## §12. Conclusion

Sprint C clôt GLOSS avec :
- **Portail Next.js 14** : interface DPO/admin compliance-aware, 8 pages, SSO OIDC, RBAC tenant-scoped
- **Modules Terraform GCP + Azure** : équivalents fonctionnels du module AWS Sprint B, idiomatic chacun
- **Runbooks RB-07 à RB-12** : 12 runbooks au total post-GLOSS, couvrent les incidents les plus probables sur la première année
- **+277 tests** (110 Vitest + 145 Playwright × 3 browsers + 16 terratest + 6 validations)

L'investissement (15 jours-homme × 1 ing senior + audit pass Sprint C) débloque la **release v1.0.0 GA** :
- Le produit est commercialisable sur les 3 hyperscalers
- L'onboarding self-service 30-min est reproductible par tout SE
- Les 12 runbooks formalisent 80 % des scenarios incident

Sprint C est par conception **opt-in** : `portal-admin.enabled: false` par défaut dans Helm chart Sprint B. Un tenant peut tourner sans portail (utilise `efe-admin` CLI directement). Un autre peut activer le portail à tout moment via `helm upgrade --set portal-admin.enabled=true`.

Le successeur logique post-GLOSS est **M23 (DPO Export + Active Learning)**, désormais possible parce que :
- Sprint A+B+C livrent la machinerie pour avoir des tenants réels
- Le portail expose le concept_stack viewer = base pour DPO consent + dataset export
- Les 12 runbooks formalisent les ops nécessaires pour scale au-delà du MVP

---

**Fin du document — Architecture v1.0 design freeze 2026-05-09**
