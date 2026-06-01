# EFE GLOSS Sprint B — Architecture Complète (Helm + Terraform AWS + Runbooks)

**Codename** : « GLOSS-B »
**Status** : Architecture v1.0 — design freeze
**Auteur** : Architecture EFE Core
**Date** : 2026-05-08
**Pré-requis** : Sprint A (D8) GA — `efe-admin` CLI + migration `0013` + 16 outils mcp-server D8 + tests 674 GREEN
**Successeur** : Sprint C (Portail Next.js + Terraform GCP/Azure + RB-07 à RB-12)

---

## §0. Executive Summary

### 0.1 Mission

Sprint B livre **trois piliers d'infrastructure** indissociables qui, combinés au Sprint A, permettent à un Solutions Engineer (SE) EFE de provisionner un tenant POC opérationnel en moins de 30 minutes :

1. **Helm chart `efe-stack`** — umbrella déployable d'un seul `helm install`, vendoring 6 sub-charts indépendamment versionnables
2. **Module Terraform AWS** — provisioning complet de l'infrastructure cloud sous-jacente (VPC + EKS + RDS + ElastiCache + KMS + S3 + IAM)
3. **Runbooks ops RB-01 à RB-06** — procédures formalisées pour les 6 incidents les plus probables sur la première année (DB full, breaker open, audit chain broken, key rotation, BudgetExhausted, CrashLoopBackOff)

### 0.2 Trois piliers et leurs interactions

```
┌──────────────────────────────────────────────────────────────────────┐
│ 1. SE Laptop                                                         │
│    │                                                                  │
│    └─ terraform apply (Pilier 2) ────► AWS Cloud                     │
│         │                                ├─ VPC + 9 subnets          │
│         │                                ├─ EKS cluster + node groups│
│         │                                ├─ RDS Postgres multi-AZ    │
│         │                                ├─ ElastiCache Redis multi-AZ│
│         │                                ├─ KMS BYOK key             │
│         │                                ├─ S3 audit bucket (7y)     │
│         │                                ├─ IAM roles (IRSA)         │
│         │                                └─ Secrets Manager (DB creds)│
│         │                                                            │
│         └─ helm install efe-stack (Pilier 1) ─► Pods sur EKS        │
│              │                                  ├─ efe-server x3    │
│              │                                  ├─ mcp-server x2    │
│              │                                  ├─ portal-admin x3  │
│              │                                  ├─ Postgres (PGO)    │
│              │                                  ├─ Redis Stack       │
│              │                                  └─ kube-prometheus  │
│              │                                                       │
│              └─ Si incident → consulter Pilier 3 (RB-01 à RB-06)    │
└──────────────────────────────────────────────────────────────────────┘
```

### 0.3 Scope synthétique

| Pilier | Output | Effort | LOC |
|--------|--------|--------|-----|
| Helm umbrella + 6 sub-charts | YAML+templates | 3 j | ~3 200 |
| Terraform AWS module | HCL + tests terratest | 4 j | ~1 900 |
| 6 runbooks formalisés | Markdown + scripts validation | 2 j | ~2 600 |
| Tests Helm + Terraform + scripts CI | helm-unittest + terratest | 1 j | ~1 100 |
| **Total Sprint B** | | **10 j** | **~8 800** |

### 0.4 Non-goals Sprint B (réservés Sprint C)

- ❌ **Portail Next.js** : architecture livrée Sprint A, code Sprint C
- ❌ **Terraform GCP / Azure** : Sprint C
- ❌ **Runbooks RB-07 à RB-12** : Sprint C (latency, cert TLS, quota, failover, mode3 review queue, certificat)
- ❌ **Multi-region active-active** : M25
- ❌ **Service mesh (Istio/Linkerd)** : pas dans GLOSS, surcouche optionnelle ultérieure

### 0.5 Critères d'acceptation Sprint B (DoD synthèse)

1. ✅ `helm install efe-stack` réussit en ≤ 5 min sur cluster k3d local (laptop 16 GB RAM)
2. ✅ `helm install efe-stack` réussit en ≤ 8 min sur cluster EKS cloud
3. ✅ `terraform apply` AWS réussit en ≤ 12 min sur compte sandbox
4. ✅ 25 tests `helm-unittest` GREEN (templating sans cluster réel)
5. ✅ 8 tests `terratest` GREEN (déploiement réel sur AWS)
6. ✅ 6 runbooks publiés + chacun a une procédure de validation reproductible (testable en staging)
7. ✅ Helm chart vendoring respecte la chaîne d'approvisionnement (pas de pulls hors registry approuvé)
8. ✅ Terraform module respecte `tfsec` sans HIGH/CRITICAL findings
9. ✅ Compliance AWS : tous les buckets S3 + RDS + ElastiCache encrypted at rest avec KMS
10. ✅ 0 régression Sprint A (CLI `efe-admin` toujours fonctionne contre cluster Helm-déployé)

---

## §1. Contexte & justification

### 1.1 État actuel post-Sprint A

Sprint A a livré :
- CLI `efe-admin` (Rust, 4 500 LOC, 18 commandes)
- Migration `0013_d8_admin_self_service.sql` (199 LOC, 4 nouvelles tables)
- 16 outils mcp-server D8 (1 500 LOC Python, default OFF gated `EFE_FEATURE_D8_ADMIN_ENABLED`)
- 674 tests GREEN (54 Rust + 620 Python)

**Mais** sans Sprint B, le CLI ne peut pas être utilisé : il n'existe pas encore d'**infrastructure déployée** où il pourrait pointer. Sprint A est conceptuellement complet mais **opérationnellement bloqué** sans :
- Un cluster Kubernetes provisionné (Terraform AWS)
- EFE-server + mcp-server + Postgres + Redis déployés sur ce cluster (Helm)
- Une référence opérationnelle pour gérer les incidents au runtime (Runbooks)

### 1.2 Pourquoi Helm + Terraform plutôt que Docker Compose ou Pulumi

**Helm vs Docker Compose** :
- Helm est le standard de fait pour déploiements Kubernetes
- Docker Compose ne supporte pas le clustering, l'auto-scaling, les NetworkPolicies, les PodDisruptionBudgets
- Pour un produit B2B SaaS multi-tenant à 99.9 % SLA, K8s est requis
- Helm permet rolling updates atomiques, rollbacks one-shot, value templating

**Terraform vs Pulumi** :
- Terraform est l'outil le plus largement adopté chez nos clients enterprise (sondage : 87 % AWS-Terraform)
- Pulumi requiert un runtime de programmation (TS/Python/Go) que les ops EFE n'ont pas systématiquement
- Terraform a une stabilité prouvée sur 10 ans et un écosystème de modules plus riche
- HCL est déclaratif et plus lisible que TS pour des ops non-développeurs

**Terraform vs Crossplane** :
- Crossplane vit dans Kubernetes (« provisioning via CRDs »), créant un chicken-and-egg : il faut un cluster pour provisionner le cluster
- Terraform fonctionne depuis n'importe quelle machine locale, sans état préalable

### 1.3 Pourquoi vendoring des sub-charts plutôt que `dependencies.yaml`

Trois options Helm pour assembler des sub-charts :

| Option | Avantages | Inconvénients | Choix |
|--------|-----------|----------------|-------|
| `dependencies` (helm.dependency) | Léger, declarative | Pull dynamique → risque chaîne d'appro, lock-file pas universel | ❌ |
| Vendoring `charts/` | Reproductible 100 %, audit trail | Manuel, taille repo | ✅ |
| Helm OCI registry | Versioning fort, signing | Setup registry interne lourd | Plus tard |

GLOSS-B vendor les 6 sub-charts dans `charts/efe-stack/charts/` pour :
- ✅ **Reproducibility byte-for-byte** : un même commit donne le même `helm install`
- ✅ **Air-gapped support** : SE pourra installer même sans accès Internet (clients militaires, banques)
- ✅ **Supply chain audit** : SBOM Helm chart inclut tous les composants directs

### 1.4 Anti-patterns à éviter

#### 1.4.1 Anti-pattern : Helm chart « kitchen sink »

❌ **Mauvais** : un chart unique `efe-stack` avec 200 KB de templates, tout monolithique.

✅ **Bon** : umbrella + 6 sub-charts avec **boundaries clairs** :
- `efe-server` : application gRPC (M16-M22 + D8)
- `mcp-server` : application HTTP (49+5+11=65 outils MCP au total)
- `portal-admin` : application web (Sprint C)
- `postgres-operator` : opérateur PGO + CRD `PostgresCluster`
- `redis-stack` : StatefulSet Redis Stack 7.4
- `observability` : Prometheus + Grafana + Loki + Tempo

Chaque sub-chart est :
- Versionné indépendamment (semver)
- Testable indépendamment (`helm template charts/efe-stack/charts/efe-server`)
- Désactivable (`efe-server.enabled: false`)

#### 1.4.2 Anti-pattern : Terraform module « God object »

❌ **Mauvais** : un fichier `main.tf` de 2 500 lignes avec tout dedans.

✅ **Bon** : split par concern :
- `vpc.tf` : réseau
- `eks.tf` : cluster Kubernetes
- `rds.tf` : Postgres
- `elasticache.tf` : Redis
- `kms.tf` : clés
- `iam.tf` : rôles + IRSA
- `s3.tf` : audit bucket
- `secrets.tf` : Secrets Manager
- `route53.tf` : DNS (optionnel)

#### 1.4.3 Anti-pattern : runbooks comme « notes d'ingénieur »

❌ **Mauvais** : Markdown vague avec « voir le dashboard X et faire ce que tu sais faire ».

✅ **Bon** : runbook structuré (template imposé) avec :
- Métadonnées (sévérité, MTTR cible, fréquence, owners)
- Symptômes (alertes Prometheus, métriques Grafana, log patterns)
- Diagnostic step-by-step (≤ 3 commandes)
- Procédure résolution (≤ 5 étapes, time-boxées)
- Procédure de validation (reproductible en staging)
- Annexes (links dashboards, historique incidents)

### 1.5 Glossaire Sprint B

| Terme | Définition |
|-------|------------|
| **Umbrella chart** | Helm chart parent qui aggrège des sub-charts |
| **Sub-chart** | Chart enfant vendoré dans `charts/`, peut être déployé seul |
| **PGO** | Postgres Operator (CrunchyData) — opérateur Kubernetes pour Postgres |
| **PSS** | Pod Security Standards (Kubernetes 1.25+) — `privileged|baseline|restricted` |
| **HPA** | HorizontalPodAutoscaler — auto-scaling based on metrics |
| **PDB** | PodDisruptionBudget — garantit min replicas pendant rolling updates |
| **IRSA** | IAM Roles for Service Accounts (AWS) — pas de credentials statiques dans pods |
| **terratest** | Framework Go pour tester déploiements Terraform |
| **helm-unittest** | Plugin pour unit-tester les Helm templates |
| **MTTR** | Mean Time To Restore — temps moyen pour revenir à `healthy` |
| **RPO** | Recovery Point Objective — données potentiellement perdues |
| **RTO** | Recovery Time Objective — temps de remise en service |

---

## §2. Architecture cible Helm — vue d'ensemble

### 2.1 Layout du repo

```
charts/efe-stack/
├── Chart.yaml                         # umbrella v1.0.0
├── Chart.lock                         # lock file (gen via `helm dependency update`)
├── values.yaml                        # values agrégés (defaults)
├── values-local.yaml                  # values pour k3d local (replicas=1, low resources)
├── values-prod.yaml                   # values pour cloud production (replicas≥3, autoscale ON)
├── values.schema.json                 # JSON Schema Draft 2020-12 pour validation values
├── templates/
│   ├── _helpers.tpl                   # helpers Helm (name truncation, labels)
│   ├── NOTES.txt                      # message post-install
│   ├── secrets.yaml                   # secrets (DB url, BYOK key id, sso client secret)
│   ├── configmap-shared.yaml          # config partagée inter-sub-charts
│   ├── networkpolicy-default.yaml     # deny-all baseline
│   └── tests/
│       └── connection-test.yaml       # test pod : `helm test`
└── charts/                             # sub-charts vendorés
    ├── efe-server/
    ├── mcp-server/
    ├── portal-admin/
    ├── postgres-operator/
    ├── redis-stack/
    └── observability/
```

### 2.2 Diagramme dépendances inter-sub-charts

```
                ┌──────────────────────────┐
                │   observability          │
                │   (independent)          │
                └──────────────────────────┘
                          ▲
                          │ scrapes
                          │
        ┌─────────────────┼─────────────────┐
        │                 │                  │
┌───────┴────────┐ ┌──────┴───────┐ ┌────────┴──────────┐
│  efe-server    │ │  mcp-server  │ │  portal-admin     │
│                │ │              │ │                    │
│  needs:        │ │ needs:       │ │ needs:             │
│  - postgres    │ │ - postgres   │ │ - postgres (audit) │
│  - redis       │ │ - efe-server │ │ - mcp-server       │
└────────────────┘ └──────────────┘ └────────────────────┘
        │                 │                  │
        └────────┬────────┴────────┬─────────┘
                 │                  │
       ┌─────────┴──────┐  ┌────────┴──────┐
       │ postgres-      │  │  redis-stack  │
       │ operator       │  │               │
       └────────────────┘  └───────────────┘
```

**Ordre d'installation** (Helm hooks `post-install`) :
1. `postgres-operator` (CRDs + opérateur)
2. `redis-stack`
3. Postgres `PostgresCluster` créé via CRD post-install hook
4. Migrations DB exécutées via Job init-container
5. `efe-server`, `mcp-server` lancés
6. `portal-admin`
7. `observability` (peut démarrer en parallèle avec 1-6)

### 2.3 Versioning

```yaml
# charts/efe-stack/Chart.yaml
apiVersion: v2
name: efe-stack
description: EFE — multi-tenant semantic classification stack
type: application
version: 1.0.0                          # umbrella semver
appVersion: "1.6.0"                     # corresponds to efe-server v1.6 (D8 ready)
kubeVersion: ">=1.28.0-0"
home: https://efe.example.com
sources:
  - https://github.com/efe-ai/efe
maintainers:
  - name: EFE Core Team
    email: ops@efe.example.com
keywords: [classification, llm, mlops, gdpr, ai-act]
icon: https://efe.example.com/static/logo.png
annotations:
  artifacthub.io/license: UNLICENSED
  artifacthub.io/operator: "false"
  artifacthub.io/changes: |
    - kind: added
      description: Sprint GLOSS-B — initial release
```

Chaque sub-chart possède son propre `Chart.yaml` avec semver indépendant. La dépendance umbrella ↔ sub-chart est exprimée par **vendoring** (le sous-dossier `charts/` est versionné dans Git, pas tiré dynamiquement).

---

## §3. Helm umbrella `efe-stack`

### 3.1 `values.yaml` — schéma complet

```yaml
# charts/efe-stack/values.yaml
# Default values for efe-stack umbrella chart.
# Override per environment with --values <file>.

# ─── Global settings (shared by all sub-charts) ───────────────────────
global:
  domain: efe.example.com
  imageRegistry: ghcr.io/efe-ai
  imagePullSecrets:
    - name: ghcr-pull-secret
  storageClass: gp3
  # Tenant slug used for resource naming + tagging.
  tenant: ""                            # MUST be set by SE
  environment: prod                     # prod | staging | dev | poc
  # Common labels merged into every resource (used by NetworkPolicies + RBAC).
  commonLabels:
    app.kubernetes.io/part-of: efe-stack
    efe.io/managed-by: helm

# ─── efe-server (Rust gRPC service) ───────────────────────────────────
efe-server:
  enabled: true
  replicas: 3
  image:
    repository: ghcr.io/efe-ai/efe-server
    tag: ""                             # defaults to .Chart.AppVersion
    pullPolicy: IfNotPresent
  service:
    type: ClusterIP
    grpcPort: 50051
    metricsPort: 9090
    healthPort: 8081
  resources:
    requests:
      cpu: 500m
      memory: 1Gi
    limits:
      cpu: 2
      memory: 4Gi
  autoscaling:
    enabled: true
    minReplicas: 3
    maxReplicas: 12
    targetCPUUtilizationPercentage: 70
    targetClassifyRPS: 200
  podDisruptionBudget:
    minAvailable: 2
  topologySpreadConstraints:
    enabled: true
    topologyKey: topology.kubernetes.io/zone
  networkPolicy:
    enabled: true
  byok:
    backend: aws-kms                    # env|file|aws-kms|gcp-kms|azure-kv|vault|pkcs11
    awsKmsKeyId: ""                     # ARN, populated post-Terraform
  serviceAccount:
    create: true
    annotations:
      eks.amazonaws.com/role-arn: ""    # IRSA, populated post-Terraform
  envOverrides: []
  # Audit chain HMAC seed lives in Secrets Manager, mounted as env var.
  auditHmacSecretRef:
    name: efe-audit-hmac
    key: seed
  features:
    m20HelperToolsEnabled: false
    m21Mode2Enabled: false
    m22Mode3Enabled: false
    d8AdminEnabled: false               # opt-in : Sprint A admin tools

# ─── mcp-server (Python HTTP service) ─────────────────────────────────
mcp-server:
  enabled: true
  replicas: 2
  image:
    repository: ghcr.io/efe-ai/mcp-server
    tag: ""
  service:
    type: ClusterIP
    httpPort: 8080
    metricsPort: 9091
  resources:
    requests:
      cpu: 200m
      memory: 512Mi
    limits:
      cpu: 1
      memory: 2Gi
  autoscaling:
    enabled: true
    minReplicas: 2
    maxReplicas: 8
  profile: agent-native-deploy          # core|advanced|agent-native|agent-native-deploy|experimental
  podDisruptionBudget:
    minAvailable: 1
  serviceAccount:
    create: true

# ─── portal-admin (Next.js, Sprint C) ─────────────────────────────────
portal-admin:
  enabled: false                        # default OFF until Sprint C ships
  replicas: 3
  image:
    repository: ghcr.io/efe-ai/portal-admin
    tag: ""
  service:
    type: ClusterIP
    httpPort: 3000
  ingress:
    enabled: true
    className: nginx
    host: ""                            # defaults to portal.{global.domain}
    tls:
      enabled: true
      issuerRef:
        kind: ClusterIssuer
        name: letsencrypt-prod
  sso:
    issuer: ""                          # https://idp.example.com/realms/efe
    clientId: efe-portal
    clientSecretRef:
      name: efe-portal-sso
      key: client-secret
  resources:
    requests:
      cpu: 200m
      memory: 256Mi
    limits:
      cpu: 1
      memory: 1Gi

# ─── postgres-operator (CrunchyData PGO) ──────────────────────────────
postgres:
  enabled: true                         # set false to use external RDS via secret
  operator:
    enabled: true
    image:
      repository: registry.developers.crunchydata.com/crunchydata/postgres-operator
      tag: ubi8-5.5.0-0
  cluster:
    name: efe-postgres
    postgresVersion: 16
    instances:
    - name: instance1
      replicas: 3
      dataVolumeClaimSpec:
        accessModes: [ReadWriteOnce]
        resources:
          requests:
            storage: 100Gi
        storageClassName: ""            # falls back to global.storageClass
      affinity:
        podAntiAffinity:
          requiredDuringSchedulingIgnoredDuringExecution:
          - labelSelector:
              matchLabels:
                postgres-operator.crunchydata.com/cluster: efe-postgres
            topologyKey: topology.kubernetes.io/zone
  backup:
    pgbackrest:
      repos:
      - name: repo1
        s3:
          bucket: ""                    # populated post-Terraform
          endpoint: s3.amazonaws.com
          region: ""
        schedules:
          full: "0 1 * * 0"
          differential: "0 1 * * 1-6"
          incremental: "0 */4 * * *"
  monitoring:
    enabled: true

# When enabled=false, expects an external PG via secret :
externalPostgres:
  enabled: false
  secretRef:
    name: efe-postgres-external
    keys:
      host: host
      port: port
      database: database
      username: username
      password: password
      sslMode: ssl_mode

# ─── redis-stack ──────────────────────────────────────────────────────
redis:
  enabled: true                         # set false to use external ElastiCache
  image:
    repository: redis/redis-stack-server
    tag: 7.4-v0
  storage: 50Gi
  storageClass: ""
  resources:
    requests:
      cpu: 500m
      memory: 2Gi
    limits:
      cpu: 2
      memory: 8Gi
  authToken:
    enabled: true
    secretRef:
      name: efe-redis-auth
      key: token
  persistence:
    enabled: true
    rdbCompression: true
    aofEnabled: true
    aofFsync: everysec
  maxMemoryPolicy: noeviction           # critical for caching correctness

externalRedis:
  enabled: false
  host: ""
  port: 6379
  authTokenSecretRef:
    name: efe-redis-external-auth
    key: token

# ─── observability ────────────────────────────────────────────────────
observability:
  enabled: true
  prometheus:
    enabled: true
    retention: 30d
    persistentVolumeSize: 100Gi
    storageClass: ""
    scrapeInterval: 15s
    extraScrapeConfigs: []
  grafana:
    enabled: true
    adminPasswordSecretRef:
      name: efe-grafana-admin
      key: admin-password
    dashboards:
      efeOverview: true
      efeMode3: true
      efeCompliance: true
    persistentVolumeSize: 10Gi
  loki:
    enabled: true
    retention: 30d
    persistentVolumeSize: 200Gi
  tempo:
    enabled: true
    samplingRate: 0.01                  # prod default ; dev = 1.0
    retention: 7d
  alertmanager:
    enabled: true
    receivers:
      slack:
        enabled: false
        webhookSecretRef: ""
      pagerduty:
        enabled: false
        serviceKeySecretRef: ""

# ─── Network ──────────────────────────────────────────────────────────
ingress:
  enabled: true
  className: nginx
  annotations:
    cert-manager.io/cluster-issuer: letsencrypt-prod
  tls:
    enabled: true

networkPolicy:
  defaultDeny: true                     # baseline NetworkPolicy = deny-all

podSecurityPolicy:
  enforce: restricted                   # PSS level

# ─── Init job : DB migrations ─────────────────────────────────────────
migrations:
  enabled: true
  image:
    repository: ghcr.io/efe-ai/efe-server-migrate
    tag: ""
  backoffLimit: 5
  resources:
    requests:
      cpu: 100m
      memory: 256Mi
```

### 3.2 `values.schema.json` — validation stricte

`values.schema.json` (~ 800 lignes) impose le schéma JSON Schema Draft 2020-12. Tout `helm install` avec une valeur invalide est **bloqué au templating** avec erreur explicite.

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "required": ["global"],
  "properties": {
    "global": {
      "type": "object",
      "required": ["tenant"],
      "properties": {
        "tenant": {
          "type": "string",
          "pattern": "^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$",
          "description": "Tenant slug, 3-32 chars [a-z0-9-]"
        },
        "domain": {"type": "string", "format": "hostname"},
        "environment": {"type": "string", "enum": ["prod", "staging", "dev", "poc"]},
        "imageRegistry": {"type": "string"},
        "storageClass": {"type": "string"}
      }
    },
    "efe-server": {
      "type": "object",
      "properties": {
        "enabled": {"type": "boolean"},
        "replicas": {"type": "integer", "minimum": 1, "maximum": 100},
        "byok": {
          "type": "object",
          "properties": {
            "backend": {
              "type": "string",
              "enum": ["env", "file", "aws-kms", "gcp-kms", "azure-kv", "vault", "pkcs11"]
            }
          }
        },
        "features": {
          "type": "object",
          "properties": {
            "m20HelperToolsEnabled": {"type": "boolean"},
            "m21Mode2Enabled": {"type": "boolean"},
            "m22Mode3Enabled": {"type": "boolean"},
            "d8AdminEnabled": {"type": "boolean"}
          }
        }
      }
    }
    /* ... 700 more lines covering all sub-charts ... */
  }
}
```

Test : `helm install --dry-run efe-stack . --values bad.yaml` doit échouer si `global.tenant: "BadCase"` (uppercase rejected).

### 3.3 Helpers (`templates/_helpers.tpl`)

```yaml
{{/*
  Expand the name of the chart.
*/}}
{{- define "efe-stack.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
  Create a fully qualified app name.
*/}}
{{- define "efe-stack.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}

{{/*
  Common labels applied to every resource.
*/}}
{{- define "efe-stack.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
app.kubernetes.io/name: {{ include "efe-stack.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
efe.io/tenant: {{ .Values.global.tenant | quote }}
efe.io/environment: {{ .Values.global.environment | default "prod" | quote }}
{{- with .Values.global.commonLabels }}
{{ toYaml . }}
{{- end }}
{{- end }}

{{/*
  Compute the full image reference.
*/}}
{{- define "efe-stack.image" -}}
{{- $registry := .registry | default $.Values.global.imageRegistry -}}
{{- $repository := .repository -}}
{{- $tag := .tag | default $.Chart.AppVersion -}}
{{- printf "%s/%s:%s" $registry $repository $tag -}}
{{- end -}}
```

### 3.4 NOTES.txt (post-install message)

```
EFE stack {{ .Chart.AppVersion }} installed in namespace {{ .Release.Namespace }}.

Tenant : {{ .Values.global.tenant }}
Environment : {{ .Values.global.environment | default "prod" }}

Quick verification :
  $ kubectl get pods -n {{ .Release.Namespace }}
  $ helm test {{ .Release.Name }} -n {{ .Release.Namespace }}

Endpoints :
  - efe-server gRPC : {{ include "efe-stack.fullname" . }}-efe-server.{{ .Release.Namespace }}.svc.cluster.local:50051
  - mcp-server HTTP : {{ include "efe-stack.fullname" . }}-mcp-server.{{ .Release.Namespace }}.svc.cluster.local:8080
{{- if .Values.observability.enabled }}
  - Grafana : kubectl port-forward -n {{ .Release.Namespace }} svc/grafana 3000:80
{{- end }}

Next steps :
  1. Authenticate efe-admin : efe-admin login --cluster {{ .Values.global.tenant }}
  2. Create your first tenant : efe-admin tenant create acme --plan trial
  3. Run the smoke test : efe-admin smoke-test acme

Documentation : https://efe.example.com/docs/onboarding/SE_PLAYBOOK.md
Runbooks      : https://efe.example.com/docs/runbooks/
Support       : https://efe.example.com/support/
```

---

## §4. Sub-charts détaillés

### 4.1 `efe-server` sub-chart

#### 4.1.1 Layout

```
charts/efe-stack/charts/efe-server/
├── Chart.yaml
├── values.yaml
├── templates/
│   ├── _helpers.tpl
│   ├── serviceaccount.yaml
│   ├── deployment.yaml
│   ├── service.yaml
│   ├── pdb.yaml
│   ├── hpa.yaml
│   ├── networkpolicy.yaml
│   ├── configmap.yaml
│   └── tests/
│       └── grpc-healthcheck.yaml      # `helm test` pod
└── tests/                              # helm-unittest
    ├── deployment_test.yaml
    ├── service_test.yaml
    └── networkpolicy_test.yaml
```

#### 4.1.2 `templates/deployment.yaml`

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {{ include "efe-server.fullname" . }}
  labels:
    {{- include "efe-server.labels" . | nindent 4 }}
spec:
  replicas: {{ .Values.replicas | default 3 }}
  strategy:
    type: RollingUpdate
    rollingUpdate:
      maxSurge: 1
      maxUnavailable: 0
  selector:
    matchLabels:
      {{- include "efe-server.selectorLabels" . | nindent 6 }}
  template:
    metadata:
      labels:
        {{- include "efe-server.selectorLabels" . | nindent 8 }}
      annotations:
        prometheus.io/scrape: "true"
        prometheus.io/port: "9090"
        prometheus.io/path: "/metrics"
        # Force pod restart on config change (rolling).
        checksum/config: {{ include (print $.Template.BasePath "/configmap.yaml") . | sha256sum }}
    spec:
      serviceAccountName: {{ include "efe-server.serviceAccountName" . }}
      automountServiceAccountToken: true
      securityContext:
        runAsNonRoot: true
        runAsUser: 65532                # nonroot
        runAsGroup: 65532
        fsGroup: 65532
        seccompProfile:
          type: RuntimeDefault
      containers:
      - name: efe-server
        image: {{ include "efe-server.image" . | quote }}
        imagePullPolicy: {{ .Values.image.pullPolicy | default "IfNotPresent" }}
        ports:
        - name: grpc
          containerPort: {{ .Values.service.grpcPort | default 50051 }}
        - name: metrics
          containerPort: {{ .Values.service.metricsPort | default 9090 }}
        - name: health
          containerPort: {{ .Values.service.healthPort | default 8081 }}
        env:
        - name: EFE_DATABASE_URL
          valueFrom:
            secretKeyRef:
              {{- if $.Values.global.externalPostgres.enabled }}
              name: {{ $.Values.global.externalPostgres.secretRef.name }}
              key: connection_string
              {{- else }}
              name: efe-postgres-pguser-efe
              key: uri
              {{- end }}
        - name: EFE_REDIS_URL
          valueFrom:
            secretKeyRef:
              name: {{ default "efe-redis-auth" .Values.redisSecretName }}
              key: connection_string
        - name: EFE_BYOK_BACKEND
          value: {{ .Values.byok.backend | quote }}
        {{- if eq .Values.byok.backend "aws-kms" }}
        - name: EFE_BYOK_AWS_KMS_KEY_ID
          value: {{ .Values.byok.awsKmsKeyId | quote }}
        {{- end }}
        - name: EFE_AUDIT_HMAC_SEED
          valueFrom:
            secretKeyRef:
              name: {{ .Values.auditHmacSecretRef.name }}
              key: {{ .Values.auditHmacSecretRef.key }}
        - name: EFE_FEATURE_M20_HELPER_TOOLS_ENABLED
          value: {{ .Values.features.m20HelperToolsEnabled | quote }}
        - name: EFE_FEATURE_M21_MODE2_ENABLED
          value: {{ .Values.features.m21Mode2Enabled | quote }}
        - name: EFE_FEATURE_M22_MODE3_ENABLED
          value: {{ .Values.features.m22Mode3Enabled | quote }}
        - name: EFE_FEATURE_D8_ADMIN_ENABLED
          value: {{ .Values.features.d8AdminEnabled | quote }}
        {{- with .Values.envOverrides }}
        {{- toYaml . | nindent 8 }}
        {{- end }}
        livenessProbe:
          httpGet:
            path: /healthz
            port: health
          initialDelaySeconds: 10
          periodSeconds: 5
          failureThreshold: 3
          timeoutSeconds: 2
        readinessProbe:
          httpGet:
            path: /readyz
            port: health
          initialDelaySeconds: 5
          periodSeconds: 2
          failureThreshold: 3
          timeoutSeconds: 1
        startupProbe:
          httpGet:
            path: /healthz
            port: health
          failureThreshold: 30
          periodSeconds: 2
        resources:
          {{- toYaml .Values.resources | nindent 10 }}
        securityContext:
          allowPrivilegeEscalation: false
          readOnlyRootFilesystem: true
          capabilities:
            drop: ["ALL"]
        volumeMounts:
        - name: tmp
          mountPath: /tmp
        - name: cache
          mountPath: /var/cache/efe
      volumes:
      - name: tmp
        emptyDir: {}
      - name: cache
        emptyDir:
          sizeLimit: 1Gi
      {{- with .Values.topologySpreadConstraints }}
      {{- if .enabled }}
      topologySpreadConstraints:
      - maxSkew: 1
        topologyKey: {{ .topologyKey }}
        whenUnsatisfiable: DoNotSchedule
        labelSelector:
          matchLabels:
            {{- include "efe-server.selectorLabels" $ | nindent 12 }}
      {{- end }}
      {{- end }}
```

#### 4.1.3 `templates/hpa.yaml`

```yaml
{{- if .Values.autoscaling.enabled }}
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: {{ include "efe-server.fullname" . }}
  labels:
    {{- include "efe-server.labels" . | nindent 4 }}
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: {{ include "efe-server.fullname" . }}
  minReplicas: {{ .Values.autoscaling.minReplicas | default 3 }}
  maxReplicas: {{ .Values.autoscaling.maxReplicas | default 12 }}
  metrics:
  - type: Resource
    resource:
      name: cpu
      target:
        type: Utilization
        averageUtilization: {{ .Values.autoscaling.targetCPUUtilizationPercentage | default 70 }}
  - type: Pods
    pods:
      metric:
        name: efe_classify_rps
      target:
        type: AverageValue
        averageValue: "{{ .Values.autoscaling.targetClassifyRPS | default 200 }}"
  behavior:
    scaleUp:
      stabilizationWindowSeconds: 60
      policies:
      - type: Percent
        value: 50
        periodSeconds: 30
      - type: Pods
        value: 4
        periodSeconds: 30
      selectPolicy: Max
    scaleDown:
      stabilizationWindowSeconds: 300
      policies:
      - type: Percent
        value: 10
        periodSeconds: 60
      selectPolicy: Min
{{- end }}
```

#### 4.1.4 `templates/networkpolicy.yaml`

```yaml
{{- if .Values.networkPolicy.enabled }}
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: {{ include "efe-server.fullname" . }}
  labels:
    {{- include "efe-server.labels" . | nindent 4 }}
spec:
  podSelector:
    matchLabels:
      {{- include "efe-server.selectorLabels" . | nindent 6 }}
  policyTypes: [Ingress, Egress]
  ingress:
  # mcp-server can call efe-server gRPC
  - from:
    - podSelector:
        matchLabels:
          app.kubernetes.io/name: mcp-server
    ports:
    - protocol: TCP
      port: {{ .Values.service.grpcPort | default 50051 }}
  # portal-admin can call efe-server gRPC (read-only)
  - from:
    - podSelector:
        matchLabels:
          app.kubernetes.io/name: portal-admin
    ports:
    - protocol: TCP
      port: {{ .Values.service.grpcPort | default 50051 }}
  # Prometheus scrapes /metrics
  - from:
    - namespaceSelector:
        matchLabels:
          name: efe-observability
      podSelector:
        matchLabels:
          app.kubernetes.io/name: prometheus
    ports:
    - protocol: TCP
      port: {{ .Values.service.metricsPort | default 9090 }}
  egress:
  # Postgres
  - to:
    - podSelector:
        matchLabels:
          postgres-operator.crunchydata.com/cluster: efe-postgres
    ports:
    - protocol: TCP
      port: 5432
  # External RDS (when externalPostgres.enabled)
  {{- if $.Values.global.externalPostgres.enabled }}
  - to:
    - ipBlock:
        cidr: 10.0.0.0/16             # VPC CIDR ; refine in prod values
    ports:
    - protocol: TCP
      port: 5432
  {{- end }}
  # Redis Stack
  - to:
    - podSelector:
        matchLabels:
          app.kubernetes.io/name: redis-stack
    ports:
    - protocol: TCP
      port: 6379
  # External LLM providers (Anthropic, OpenAI, Bedrock, Azure, ...)
  - to:
    - ipBlock:
        cidr: 0.0.0.0/0
        except:
        - 169.254.0.0/16              # block IMDS
        - 10.0.0.0/8                  # block intranet
    ports:
    - protocol: TCP
      port: 443
  # DNS
  - to:
    ports:
    - protocol: UDP
      port: 53
    - protocol: TCP
      port: 53
{{- end }}
```

#### 4.1.5 helm-unittest tests

```yaml
# charts/efe-stack/charts/efe-server/tests/deployment_test.yaml
suite: efe-server deployment tests
templates:
  - deployment.yaml
release:
  name: test
  namespace: efe-test
chart:
  appVersion: 1.6.0
tests:
  - it: should render with default values
    asserts:
      - isKind:
          of: Deployment
      - equal:
          path: spec.replicas
          value: 3
      - equal:
          path: spec.template.spec.containers[0].name
          value: efe-server
      - matchRegex:
          path: spec.template.spec.containers[0].image
          pattern: "ghcr.io/efe-ai/efe-server:1\\.6\\.[0-9]+"

  - it: respects replicas override
    set:
      replicas: 7
    asserts:
      - equal:
          path: spec.replicas
          value: 7

  - it: includes M22 feature flag env var
    set:
      features.m22Mode3Enabled: true
    asserts:
      - contains:
          path: spec.template.spec.containers[0].env
          content:
            name: EFE_FEATURE_M22_MODE3_ENABLED
            value: "true"

  - it: enforces nonroot security context
    asserts:
      - equal:
          path: spec.template.spec.securityContext.runAsNonRoot
          value: true
      - equal:
          path: spec.template.spec.securityContext.runAsUser
          value: 65532

  - it: drops all capabilities
    asserts:
      - equal:
          path: spec.template.spec.containers[0].securityContext.capabilities.drop
          value: ["ALL"]

  - it: sets readOnlyRootFilesystem
    asserts:
      - equal:
          path: spec.template.spec.containers[0].securityContext.readOnlyRootFilesystem
          value: true

  - it: includes prometheus scrape annotations
    asserts:
      - equal:
          path: spec.template.metadata.annotations["prometheus.io/scrape"]
          value: "true"
      - equal:
          path: spec.template.metadata.annotations["prometheus.io/port"]
          value: "9090"
```

### 4.2 `mcp-server` sub-chart

#### 4.2.1 Différences clés vs `efe-server`

- HTTP service au lieu de gRPC (port 8080)
- Réplicas par défaut 2 (mcp-server est moins sollicité)
- Profile gating via env var `EFE_MCP_TOOL_PROFILE`
- Pas de BYOK (mcp-server délègue à efe-server)

#### 4.2.2 `templates/deployment.yaml` (extrait spécifique)

```yaml
        env:
        - name: EFE_MCP_TOOL_PROFILE
          value: {{ .Values.profile | default "agent-native" | quote }}
        - name: EFE_BACKEND_GRPC_URL
          value: {{ printf "http://%s-efe-server:%d" $.Release.Name (int $.Values.efeServer.service.grpcPort) | quote }}
        - name: EFE_FEATURE_M20_HELPER_TOOLS_ENABLED
          value: {{ $.Values.efeServer.features.m20HelperToolsEnabled | quote }}
        - name: EFE_FEATURE_M21_MODE2_ENABLED
          value: {{ $.Values.efeServer.features.m21Mode2Enabled | quote }}
        - name: EFE_FEATURE_M22_MODE3_ENABLED
          value: {{ $.Values.efeServer.features.m22Mode3Enabled | quote }}
        - name: EFE_FEATURE_D8_ADMIN_ENABLED
          value: {{ $.Values.efeServer.features.d8AdminEnabled | quote }}
```

Le mcp-server hérite des feature flags de `efe-server` (cohérence : si M22 est activé sur le backend, les outils M22 mcp-server doivent suivre).

#### 4.2.3 `templates/service.yaml`

```yaml
apiVersion: v1
kind: Service
metadata:
  name: {{ include "mcp-server.fullname" . }}
  labels:
    {{- include "mcp-server.labels" . | nindent 4 }}
spec:
  type: ClusterIP
  ports:
  - name: http
    port: 8080
    targetPort: http
    protocol: TCP
  - name: metrics
    port: 9091
    targetPort: metrics
    protocol: TCP
  selector:
    {{- include "mcp-server.selectorLabels" . | nindent 4 }}
```

### 4.3 `portal-admin` sub-chart

`enabled: false` par défaut tant que Sprint C n'a pas livré le code Next.js.

Squelette inclus pour faciliter Sprint C :

```yaml
# charts/efe-stack/charts/portal-admin/Chart.yaml
apiVersion: v2
name: portal-admin
description: EFE Portal Admin (Next.js 14) — TENANT DPO/admin UI
type: application
version: 0.1.0
appVersion: "0.0.1-pre"                   # not yet GA
```

#### 4.3.1 `templates/ingress.yaml`

```yaml
{{- if and .Values.enabled .Values.ingress.enabled }}
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: {{ include "portal-admin.fullname" . }}
  labels:
    {{- include "portal-admin.labels" . | nindent 4 }}
  annotations:
    {{- if eq .Values.ingress.tls.issuerRef.kind "ClusterIssuer" }}
    cert-manager.io/cluster-issuer: {{ .Values.ingress.tls.issuerRef.name }}
    {{- else }}
    cert-manager.io/issuer: {{ .Values.ingress.tls.issuerRef.name }}
    {{- end }}
    nginx.ingress.kubernetes.io/proxy-body-size: 10m
    nginx.ingress.kubernetes.io/proxy-read-timeout: "60"
    nginx.ingress.kubernetes.io/server-snippet: |
      add_header X-Frame-Options "DENY" always;
      add_header X-Content-Type-Options "nosniff" always;
      add_header Strict-Transport-Security "max-age=63072000; includeSubDomains; preload" always;
      add_header Referrer-Policy "strict-origin-when-cross-origin" always;
spec:
  ingressClassName: {{ .Values.ingress.className | default "nginx" }}
  tls:
  {{- if .Values.ingress.tls.enabled }}
  - hosts:
    - {{ .Values.ingress.host | default (printf "portal.%s" $.Values.global.domain) }}
    secretName: {{ include "portal-admin.fullname" . }}-tls
  {{- end }}
  rules:
  - host: {{ .Values.ingress.host | default (printf "portal.%s" $.Values.global.domain) }}
    http:
      paths:
      - path: /
        pathType: Prefix
        backend:
          service:
            name: {{ include "portal-admin.fullname" . }}
            port:
              number: 3000
{{- end }}
```

### 4.4 `postgres-operator` sub-chart (CrunchyData PGO)

#### 4.4.1 Stratégie

Vendoring du chart upstream **PGO** + ajout d'un template `PostgresCluster` pour notre cluster spécifique.

Le PGO opérateur est le contrôleur Kubernetes qui gère :
- Streaming replication primary → 2 replicas (sync)
- Failover automatique (Patroni-style)
- pgBackRest scheduling
- Rolling minor version upgrade

#### 4.4.2 `templates/postgrescluster.yaml`

```yaml
{{- if .Values.cluster.enabled | default true }}
apiVersion: postgres-operator.crunchydata.com/v1beta1
kind: PostgresCluster
metadata:
  name: {{ .Values.cluster.name | default "efe-postgres" }}
  labels:
    {{- include "postgres-operator.labels" . | nindent 4 }}
spec:
  postgresVersion: {{ .Values.cluster.postgresVersion | default 16 }}

  instances:
  {{- range .Values.cluster.instances }}
  - name: {{ .name }}
    replicas: {{ .replicas | default 3 }}
    dataVolumeClaimSpec:
      accessModes: [ReadWriteOnce]
      resources:
        requests:
          storage: {{ .dataVolumeClaimSpec.resources.requests.storage | default "100Gi" }}
      storageClassName: {{ .dataVolumeClaimSpec.storageClassName | default $.Values.global.storageClass }}
    affinity:
      podAntiAffinity:
        requiredDuringSchedulingIgnoredDuringExecution:
        - labelSelector:
            matchLabels:
              postgres-operator.crunchydata.com/cluster: {{ $.Values.cluster.name }}
          topologyKey: topology.kubernetes.io/zone
    resources:
      requests:
        cpu: 500m
        memory: 2Gi
      limits:
        cpu: 4
        memory: 16Gi
  {{- end }}

  backups:
    pgbackrest:
      repos:
      {{- range .Values.backup.pgbackrest.repos }}
      - name: {{ .name }}
        s3:
          bucket: {{ .s3.bucket | quote }}
          endpoint: {{ .s3.endpoint | quote }}
          region: {{ .s3.region | quote }}
        schedules:
          full: {{ .schedules.full | quote }}
          differential: {{ .schedules.differential | quote }}
          incremental: {{ .schedules.incremental | quote }}
      {{- end }}

  users:
  - name: efe
    databases: [efe]
    options: "SUPERUSER"                # required for migrations 0001-0013
  - name: efe_app
    databases: [efe]
    options: ""                         # restricted runtime user

  monitoring:
    pgmonitor:
      exporter:
        image: registry.developers.crunchydata.com/crunchydata/crunchy-postgres-exporter:ubi8-5.5.0-0
        resources:
          requests:
            cpu: 50m
            memory: 128Mi
{{- end }}
```

#### 4.4.3 Migrations job (init)

```yaml
{{- if $.Values.migrations.enabled }}
apiVersion: batch/v1
kind: Job
metadata:
  name: {{ include "efe-stack.fullname" . }}-migrate
  labels:
    {{- include "efe-stack.labels" . | nindent 4 }}
  annotations:
    "helm.sh/hook": post-install,post-upgrade
    "helm.sh/hook-weight": "10"
    "helm.sh/hook-delete-policy": before-hook-creation,hook-succeeded
spec:
  backoffLimit: {{ $.Values.migrations.backoffLimit | default 5 }}
  template:
    metadata:
      labels:
        {{- include "efe-stack.selectorLabels" . | nindent 8 }}
        component: migrate
    spec:
      restartPolicy: OnFailure
      serviceAccountName: {{ include "efe-stack.fullname" . }}-migrate
      containers:
      - name: migrate
        image: {{ include "efe-stack.image" $.Values.migrations.image | quote }}
        command: ["/usr/local/bin/sqlx", "migrate", "run"]
        env:
        - name: DATABASE_URL
          valueFrom:
            secretKeyRef:
              name: efe-postgres-pguser-efe
              key: uri
        resources:
          {{- toYaml $.Values.migrations.resources | nindent 10 }}
{{- end }}
```

### 4.5 `redis-stack` sub-chart

#### 4.5.1 `templates/statefulset.yaml`

```yaml
{{- if .Values.enabled }}
apiVersion: apps/v1
kind: StatefulSet
metadata:
  name: {{ include "redis-stack.fullname" . }}
  labels:
    {{- include "redis-stack.labels" . | nindent 4 }}
spec:
  serviceName: {{ include "redis-stack.fullname" . }}-headless
  replicas: 1                           # MVP single-node ; sentinel post-MVP
  selector:
    matchLabels:
      {{- include "redis-stack.selectorLabels" . | nindent 6 }}
  template:
    metadata:
      labels:
        {{- include "redis-stack.selectorLabels" . | nindent 8 }}
      annotations:
        prometheus.io/scrape: "true"
        prometheus.io/port: "9121"
    spec:
      securityContext:
        runAsNonRoot: true
        runAsUser: 999
        runAsGroup: 999
        fsGroup: 999
      containers:
      - name: redis-stack
        image: {{ include "efe-stack.image" .Values.image | quote }}
        ports:
        - name: redis
          containerPort: 6379
        env:
        - name: REDIS_ARGS
          value: >-
            --requirepass $(REDIS_AUTH_TOKEN)
            --maxmemory-policy {{ .Values.maxMemoryPolicy | default "noeviction" }}
            {{- if .Values.persistence.enabled }}
            --appendonly yes
            --appendfsync {{ .Values.persistence.aofFsync | default "everysec" }}
            {{- if .Values.persistence.rdbCompression }}
            --rdbcompression yes
            {{- end }}
            {{- end }}
        - name: REDIS_AUTH_TOKEN
          valueFrom:
            secretKeyRef:
              name: {{ .Values.authToken.secretRef.name }}
              key: {{ .Values.authToken.secretRef.key }}
        livenessProbe:
          tcpSocket:
            port: redis
          initialDelaySeconds: 15
          periodSeconds: 5
        readinessProbe:
          exec:
            command:
            - sh
            - -c
            - redis-cli -a $REDIS_AUTH_TOKEN PING
          initialDelaySeconds: 5
          periodSeconds: 2
        resources:
          {{- toYaml .Values.resources | nindent 10 }}
        volumeMounts:
        - name: data
          mountPath: /data
      - name: redis-exporter
        image: oliver006/redis_exporter:v1.62.0
        ports:
        - name: exporter
          containerPort: 9121
        env:
        - name: REDIS_ADDR
          value: localhost:6379
        - name: REDIS_PASSWORD
          valueFrom:
            secretKeyRef:
              name: {{ .Values.authToken.secretRef.name }}
              key: {{ .Values.authToken.secretRef.key }}
        resources:
          requests:
            cpu: 50m
            memory: 64Mi
          limits:
            cpu: 200m
            memory: 256Mi
  volumeClaimTemplates:
  - metadata:
      name: data
    spec:
      accessModes: [ReadWriteOnce]
      resources:
        requests:
          storage: {{ .Values.storage | default "50Gi" }}
      storageClassName: {{ .Values.storageClass | default $.Values.global.storageClass }}
{{- end }}
```

### 4.6 `observability` sub-chart

#### 4.6.1 Stratégie

Vendoring de `kube-prometheus-stack` (Prometheus + Grafana + Alertmanager) + add-ons :
- Loki (logs)
- Tempo (traces)
- 3 dashboards Grafana pré-configurés

#### 4.6.2 Dashboards Grafana embedded

```yaml
# charts/efe-stack/charts/observability/templates/grafana-dashboards.yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: efe-grafana-dashboards
  labels:
    {{- include "observability.labels" . | nindent 4 }}
    grafana_dashboard: "1"
  annotations:
    grafana_folder: "EFE"
data:
  efe-overview.json: |
    {{ .Files.Get "dashboards/efe-overview.json" | nindent 4 }}
  efe-mode3.json: |
    {{ .Files.Get "dashboards/efe-mode3.json" | nindent 4 }}
  efe-compliance.json: |
    {{ .Files.Get "dashboards/efe-compliance.json" | nindent 4 }}
```

#### 4.6.3 Alert rules Prometheus

```yaml
# charts/efe-stack/charts/observability/templates/prometheus-rules.yaml
apiVersion: monitoring.coreos.com/v1
kind: PrometheusRule
metadata:
  name: efe-alerts
spec:
  groups:
  - name: efe.critical
    interval: 30s
    rules:
    # RB-01 : DB disk saturation
    - alert: EfePostgresDiskFull
      expr: (kubelet_volume_stats_used_bytes{persistentvolumeclaim=~"data-efe-postgres-.*"} / kubelet_volume_stats_capacity_bytes{persistentvolumeclaim=~"data-efe-postgres-.*"}) > 0.85
      for: 5m
      labels:
        severity: P1
        runbook: RB-01
      annotations:
        summary: "Postgres data volume > 85% full ({{ "{{" }} $labels.persistentvolumeclaim {{ "}}" }})"
        runbook_url: "https://efe.example.com/docs/runbooks/RUNBOOK_RB_01_DB_DISK_FULL"

    # RB-02 : LLM circuit breaker open
    - alert: EfeLlmBreakerOpen
      expr: increase(efe_llm_breaker_state{state="open"}[5m]) > 0
      for: 5m
      labels:
        severity: P2
        runbook: RB-02
      annotations:
        summary: "LLM circuit breaker open for provider={{ "{{" }} $labels.provider {{ "}}" }}"
        runbook_url: "https://efe.example.com/docs/runbooks/RUNBOOK_RB_02_LLM_BREAKER_OPEN"

    # RB-03 : Audit chain HMAC broken
    - alert: EfeAuditChainBroken
      expr: efe_audit_chain_verify_failure_total > 0
      for: 1m
      labels:
        severity: P0
        runbook: RB-03
      annotations:
        summary: "Audit chain HMAC verification FAILED at audit_id={{ "{{" }} $labels.first_broken_id {{ "}}" }}"
        runbook_url: "https://efe.example.com/docs/runbooks/RUNBOOK_RB_03_AUDIT_CHAIN_BROKEN"

    # RB-05 : Tenant BudgetExhausted
    - alert: EfeTenantBudgetExhausted
      expr: efe_tenant_budget_remaining_usd <= 0
      for: 10m
      labels:
        severity: P3
        runbook: RB-05
      annotations:
        summary: "Tenant {{ "{{" }} $labels.tenant {{ "}}" }} budget exhausted"
        runbook_url: "https://efe.example.com/docs/runbooks/RUNBOOK_RB_05_BUDGET_EXHAUSTED"

    # RB-06 : Pod CrashLoopBackOff
    - alert: EfeServerCrashLoopBackOff
      expr: rate(kube_pod_container_status_restarts_total{container="efe-server"}[15m]) > 0.5
      for: 10m
      labels:
        severity: P1
        runbook: RB-06
      annotations:
        summary: "efe-server pod {{ "{{" }} $labels.pod {{ "}}" }} CrashLoopBackOff"
        runbook_url: "https://efe.example.com/docs/runbooks/RUNBOOK_RB_06_CRASH_LOOP"
```

---

## §5. Module Terraform AWS — détails complets

### 5.1 Layout

```
terraform/modules/aws/
├── main.tf                            # composition + locals + tags
├── variables.tf                       # 28 input variables
├── outputs.tf                         # 12 outputs
├── versions.tf                        # provider pins
├── vpc.tf                             # VPC + subnets + NAT
├── eks.tf                             # EKS cluster + node groups
├── rds.tf                             # RDS Postgres multi-AZ
├── elasticache.tf                     # ElastiCache Redis multi-AZ
├── kms.tf                             # KMS keys (RDS encryption + BYOK)
├── iam.tf                             # IAM roles + IRSA policies
├── s3.tf                              # Audit bucket (7y retention)
├── secrets.tf                         # Secrets Manager entries
├── route53.tf                         # DNS records (optional)
├── README.md                          # usage + variables + outputs
└── examples/
    ├── basic/
    │   ├── main.tf
    │   ├── terraform.tfvars
    │   └── README.md
    └── ha-production/
        ├── main.tf
        └── terraform.tfvars
```

### 5.2 `versions.tf`

```hcl
terraform {
  required_version = ">= 1.6, < 2.0"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.40"
    }
    random = {
      source  = "hashicorp/random"
      version = "~> 3.6"
    }
    null = {
      source  = "hashicorp/null"
      version = "~> 3.2"
    }
    kubernetes = {
      source  = "hashicorp/kubernetes"
      version = "~> 2.30"
    }
    helm = {
      source  = "hashicorp/helm"
      version = "~> 2.13"
    }
  }
}
```

### 5.3 `variables.tf`

```hcl
variable "region" {
  description = "AWS region for all resources."
  type        = string

  validation {
    condition     = can(regex("^[a-z]{2}-[a-z-]+-[0-9]+$", var.region))
    error_message = "Region must look like e.g. eu-west-1, us-east-2."
  }
}

variable "tenant" {
  description = "Tenant slug, used as resource prefix (e.g. 'acme')."
  type        = string

  validation {
    condition     = can(regex("^[a-z0-9-]{3,32}$", var.tenant))
    error_message = "Tenant slug must match [a-z0-9-]{3,32}."
  }
}

variable "environment" {
  description = "Deployment environment."
  type        = string
  default     = "prod"

  validation {
    condition     = contains(["prod", "staging", "dev", "poc"], var.environment)
    error_message = "Must be one of: prod, staging, dev, poc."
  }
}

variable "vpc_cidr" {
  description = "CIDR block for the VPC. Must be /16."
  type        = string
  default     = "10.42.0.0/16"

  validation {
    condition     = can(cidrhost(var.vpc_cidr, 0)) && tonumber(split("/", var.vpc_cidr)[1]) == 16
    error_message = "vpc_cidr must be a valid /16 CIDR block."
  }
}

variable "availability_zones_count" {
  description = "Number of AZs to span (3 recommended for prod)."
  type        = number
  default     = 3

  validation {
    condition     = var.availability_zones_count >= 2 && var.availability_zones_count <= 6
    error_message = "Must be between 2 and 6."
  }
}

variable "data_residency" {
  description = "Data residency requirement. Validated against region."
  type        = string
  default     = "any"

  validation {
    condition     = contains(["EU_ONLY", "US_ONLY", "any"], var.data_residency)
    error_message = "Must be one of: EU_ONLY, US_ONLY, any."
  }
}

variable "eks_version" {
  description = "EKS Kubernetes version."
  type        = string
  default     = "1.30"
}

variable "eks_cluster_endpoint_public_access" {
  description = "Allow EKS API public access (POC only ; set false for prod)."
  type        = bool
  default     = false
}

variable "eks_cluster_endpoint_public_access_cidrs" {
  description = "CIDRs allowed to reach the public EKS API endpoint."
  type        = list(string)
  default     = []
}

variable "eks_node_groups" {
  description = "EKS managed node groups config."
  type = map(object({
    instance_types = list(string)
    desired_size   = number
    min_size       = number
    max_size       = number
    capacity_type  = string             # ON_DEMAND | SPOT
    labels         = optional(map(string), {})
    taints = optional(list(object({
      key    = string
      value  = string
      effect = string
    })), [])
  }))
  default = {
    primary = {
      instance_types = ["m6i.xlarge"]
      desired_size   = 3
      min_size       = 3
      max_size       = 12
      capacity_type  = "ON_DEMAND"
      labels         = { role = "primary" }
    }
  }
}

variable "rds_instance_class" {
  description = "RDS Postgres instance class."
  type        = string
  default     = "db.r6i.large"
}

variable "rds_allocated_storage" {
  description = "RDS storage in GB."
  type        = number
  default     = 100

  validation {
    condition     = var.rds_allocated_storage >= 20 && var.rds_allocated_storage <= 16384
    error_message = "RDS storage must be in [20, 16384] GB."
  }
}

variable "rds_multi_az" {
  description = "Enable Multi-AZ for RDS."
  type        = bool
  default     = true
}

variable "rds_backup_retention_period" {
  description = "RDS backup retention in days."
  type        = number
  default     = 14

  validation {
    condition     = var.rds_backup_retention_period >= 7 && var.rds_backup_retention_period <= 35
    error_message = "Must be in [7, 35] days."
  }
}

variable "rds_deletion_protection" {
  description = "Enable RDS deletion protection."
  type        = bool
  default     = true
}

variable "redis_node_type" {
  description = "ElastiCache Redis node type."
  type        = string
  default     = "cache.r7g.large"
}

variable "redis_num_cache_clusters" {
  description = "Number of Redis nodes (≥ 2 for failover)."
  type        = number
  default     = 2
}

variable "enable_kms_byok" {
  description = "Provision a customer-managed KMS key for BYOK."
  type        = bool
  default     = true
}

variable "audit_bucket_retention_days" {
  description = "S3 audit bucket lifecycle retention in days (compliance ≥ 7y for EU AI Act)."
  type        = number
  default     = 2555                    # 7 years

  validation {
    condition     = var.audit_bucket_retention_days >= 365
    error_message = "Audit bucket retention must be ≥ 1 year (compliance baseline)."
  }
}

variable "tags" {
  description = "Additional tags merged into common_tags."
  type        = map(string)
  default     = {}
}
```

### 5.4 `main.tf` — composition + locals

```hcl
locals {
  prefix = "efe-${var.tenant}"

  common_tags = merge({
    "efe.io/tenant"         = var.tenant
    "efe.io/managed-by"     = "terraform"
    "efe.io/data-residency" = var.data_residency
    "efe.io/environment"    = var.environment
    "efe.io/version"        = "tf-aws-1.0.0"
  }, var.tags)
}

# Hard precondition : region must match data_residency.
data "aws_region" "current" {}

resource "null_resource" "data_residency_check" {
  count = var.data_residency != "any" ? 1 : 0
  lifecycle {
    precondition {
      condition = (
        var.data_residency == "EU_ONLY"
          ? startswith(data.aws_region.current.name, "eu-")
          : startswith(data.aws_region.current.name, "us-")
      )
      error_message = "Region ${data.aws_region.current.name} does not match data_residency=${var.data_residency}."
    }
  }
}

data "aws_availability_zones" "available" {
  state = "available"

  filter {
    name   = "opt-in-status"
    values = ["opt-in-not-required"]
  }
}
```

### 5.5 `vpc.tf` (extrait, ~ 200 lignes total)

```hcl
locals {
  azs = slice(data.aws_availability_zones.available.names, 0, var.availability_zones_count)

  # CIDR allocation pattern :
  # 10.42.0.0/16 → /20 sub-blocks via cidrsubnet(..., 4, idx)
  #   idx 0..2  : private subnets   (workloads)
  #   idx 4..6  : public subnets    (NAT + ALB)
  #   idx 8..10 : database subnets  (RDS + ElastiCache)
}

module "vpc" {
  source  = "terraform-aws-modules/vpc/aws"
  version = "~> 5.5"

  name = "${local.prefix}-vpc"
  cidr = var.vpc_cidr
  azs  = local.azs

  private_subnets  = [for i, az in local.azs : cidrsubnet(var.vpc_cidr, 4, i)]
  public_subnets   = [for i, az in local.azs : cidrsubnet(var.vpc_cidr, 4, i + 4)]
  database_subnets = [for i, az in local.azs : cidrsubnet(var.vpc_cidr, 4, i + 8)]

  enable_nat_gateway     = true
  single_nat_gateway     = false        # 1 NAT/AZ for HA
  one_nat_gateway_per_az = true

  enable_dns_hostnames = true
  enable_dns_support   = true

  enable_flow_log                      = true
  create_flow_log_cloudwatch_log_group = true
  create_flow_log_cloudwatch_iam_role  = true
  flow_log_max_aggregation_interval    = 60

  # Tags for EKS subnet discovery (shared with kubernetes-vpc-cni-controller).
  public_subnet_tags = {
    "kubernetes.io/role/elb" = "1"
  }

  private_subnet_tags = {
    "kubernetes.io/role/internal-elb" = "1"
  }

  tags = local.common_tags
}

# VPC Endpoints to keep traffic private for AWS APIs.
module "vpc_endpoints" {
  source  = "terraform-aws-modules/vpc/aws//modules/vpc-endpoints"
  version = "~> 5.5"

  vpc_id = module.vpc.vpc_id

  endpoints = {
    s3 = {
      service      = "s3"
      service_type = "Gateway"
      route_table_ids = concat(
        module.vpc.private_route_table_ids,
        module.vpc.database_route_table_ids,
      )
      tags = { Name = "${local.prefix}-vpce-s3" }
    }
    secretsmanager = {
      service             = "secretsmanager"
      service_type        = "Interface"
      private_dns_enabled = true
      subnet_ids          = module.vpc.private_subnets
      security_group_ids  = [aws_security_group.vpc_endpoints.id]
    }
    kms = {
      service             = "kms"
      service_type        = "Interface"
      private_dns_enabled = true
      subnet_ids          = module.vpc.private_subnets
      security_group_ids  = [aws_security_group.vpc_endpoints.id]
    }
  }

  tags = local.common_tags
}

resource "aws_security_group" "vpc_endpoints" {
  name        = "${local.prefix}-vpce-sg"
  description = "VPC Endpoints SG"
  vpc_id      = module.vpc.vpc_id

  ingress {
    from_port   = 443
    to_port     = 443
    protocol    = "tcp"
    cidr_blocks = [module.vpc.vpc_cidr_block]
  }

  tags = local.common_tags
}
```

### 5.6 `eks.tf`

```hcl
module "eks" {
  source  = "terraform-aws-modules/eks/aws"
  version = "~> 20.10"

  cluster_name    = "${local.prefix}-eks"
  cluster_version = var.eks_version

  vpc_id     = module.vpc.vpc_id
  subnet_ids = module.vpc.private_subnets

  enable_irsa = true

  cluster_endpoint_private_access      = true
  cluster_endpoint_public_access       = var.eks_cluster_endpoint_public_access
  cluster_endpoint_public_access_cidrs = var.eks_cluster_endpoint_public_access_cidrs

  cluster_addons = {
    coredns = {
      most_recent       = true
      resolve_conflicts = "OVERWRITE"
    }
    kube-proxy = {
      most_recent = true
    }
    vpc-cni = {
      most_recent              = true
      service_account_role_arn = module.vpc_cni_irsa.iam_role_arn
    }
    aws-ebs-csi-driver = {
      most_recent              = true
      service_account_role_arn = module.ebs_csi_irsa.iam_role_arn
    }
  }

  cluster_encryption_config = {
    provider_key_arn = aws_kms_key.eks_secrets.arn
    resources        = ["secrets"]
  }

  cluster_enabled_log_types = [
    "api",
    "audit",
    "authenticator",
    "controllerManager",
    "scheduler",
  ]

  eks_managed_node_groups = {
    for k, v in var.eks_node_groups : k => {
      instance_types = v.instance_types
      desired_size   = v.desired_size
      min_size       = v.min_size
      max_size       = v.max_size
      capacity_type  = v.capacity_type
      labels         = v.labels
      taints         = v.taints

      block_device_mappings = {
        xvda = {
          device_name = "/dev/xvda"
          ebs = {
            volume_size = 100
            volume_type = "gp3"
            iops        = 3000
            throughput  = 150
            encrypted   = true
            kms_key_id  = aws_kms_key.ebs.arn
          }
        }
      }

      iam_role_additional_policies = {
        ssm = "arn:aws:iam::aws:policy/AmazonSSMManagedInstanceCore"
      }
    }
  }

  tags = local.common_tags
}

# IRSA for VPC CNI (IPv6 + ENI quotas)
module "vpc_cni_irsa" {
  source  = "terraform-aws-modules/iam/aws//modules/iam-role-for-service-accounts-eks"
  version = "~> 5.39"

  role_name             = "${local.prefix}-vpc-cni-irsa"
  attach_vpc_cni_policy = true

  oidc_providers = {
    main = {
      provider_arn               = module.eks.oidc_provider_arn
      namespace_service_accounts = ["kube-system:aws-node"]
    }
  }

  tags = local.common_tags
}

# IRSA for EBS CSI driver (PVC encryption)
module "ebs_csi_irsa" {
  source  = "terraform-aws-modules/iam/aws//modules/iam-role-for-service-accounts-eks"
  version = "~> 5.39"

  role_name             = "${local.prefix}-ebs-csi-irsa"
  attach_ebs_csi_policy = true

  oidc_providers = {
    main = {
      provider_arn               = module.eks.oidc_provider_arn
      namespace_service_accounts = ["kube-system:ebs-csi-controller-sa"]
    }
  }

  tags = local.common_tags
}

# IRSA for efe-server (BYOK access to KMS BYOK key)
resource "aws_iam_policy" "efe_server_byok" {
  count       = var.enable_kms_byok ? 1 : 0
  name        = "${local.prefix}-efe-server-byok"
  description = "Allow efe-server to use BYOK KMS key for envelope encryption"

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect = "Allow"
        Action = [
          "kms:Encrypt",
          "kms:Decrypt",
          "kms:GenerateDataKey",
          "kms:DescribeKey",
        ]
        Resource = aws_kms_key.byok[0].arn
      },
    ]
  })
}

module "efe_server_irsa" {
  source  = "terraform-aws-modules/iam/aws//modules/iam-role-for-service-accounts-eks"
  version = "~> 5.39"

  role_name = "${local.prefix}-efe-server-irsa"

  role_policy_arns = merge(
    var.enable_kms_byok ? { byok = aws_iam_policy.efe_server_byok[0].arn } : {},
    {
      audit_bucket = aws_iam_policy.audit_bucket_write.arn
    }
  )

  oidc_providers = {
    main = {
      provider_arn               = module.eks.oidc_provider_arn
      namespace_service_accounts = ["efe:efe-server"]
    }
  }

  tags = local.common_tags
}
```

### 5.7 `rds.tf`

```hcl
resource "aws_db_subnet_group" "efe" {
  name        = "${local.prefix}-db-subnet"
  subnet_ids  = module.vpc.database_subnets
  description = "RDS subnet group for ${local.prefix}"

  tags = local.common_tags
}

resource "aws_security_group" "rds" {
  name        = "${local.prefix}-rds-sg"
  description = "Allow Postgres from EKS workloads only"
  vpc_id      = module.vpc.vpc_id

  ingress {
    description     = "Postgres from EKS"
    from_port       = 5432
    to_port         = 5432
    protocol        = "tcp"
    security_groups = [module.eks.cluster_primary_security_group_id]
  }

  egress {
    description = "All outbound (RDS doesn't initiate but Multi-AZ replication needs it)"
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = local.common_tags
}

resource "aws_kms_key" "rds" {
  description             = "RDS Postgres encryption for ${local.prefix}"
  deletion_window_in_days = 30
  enable_key_rotation     = true

  tags = local.common_tags
}

resource "aws_kms_alias" "rds" {
  name          = "alias/${local.prefix}-rds"
  target_key_id = aws_kms_key.rds.key_id
}

resource "random_password" "rds" {
  length      = 48
  special     = true
  min_lower   = 4
  min_upper   = 4
  min_numeric = 4
  min_special = 4

  override_special = "!#$%&*()-_=+[]{}<>?"
}

resource "aws_db_parameter_group" "efe" {
  name   = "${local.prefix}-pg16"
  family = "postgres16"

  parameter {
    name         = "rds.logical_replication"
    value        = "1"
    apply_method = "pending-reboot"
  }

  parameter {
    name  = "log_statement"
    value = "ddl"
  }

  parameter {
    name  = "log_min_duration_statement"
    value = "1000"                      # log queries > 1s
  }

  parameter {
    name  = "shared_preload_libraries"
    value = "pg_stat_statements,auto_explain"
  }

  parameter {
    name  = "auto_explain.log_min_duration"
    value = "5000"
  }

  tags = local.common_tags
}

resource "aws_db_instance" "efe" {
  identifier = "${local.prefix}-postgres"

  engine               = "postgres"
  engine_version       = "16.3"
  instance_class       = var.rds_instance_class
  allocated_storage    = var.rds_allocated_storage
  max_allocated_storage = var.rds_allocated_storage * 2

  storage_type      = "gp3"
  storage_encrypted = true
  kms_key_id        = aws_kms_key.rds.arn

  multi_az = var.rds_multi_az

  db_subnet_group_name   = aws_db_subnet_group.efe.name
  vpc_security_group_ids = [aws_security_group.rds.id]
  parameter_group_name   = aws_db_parameter_group.efe.name

  username = "efe_admin"
  password = random_password.rds.result
  port     = 5432

  backup_retention_period = var.rds_backup_retention_period
  backup_window           = "01:00-02:00"
  maintenance_window      = "sun:03:00-sun:04:00"
  copy_tags_to_snapshot   = true

  performance_insights_enabled          = true
  performance_insights_kms_key_id       = aws_kms_key.rds.arn
  performance_insights_retention_period = 7

  enabled_cloudwatch_logs_exports = ["postgresql", "upgrade"]

  deletion_protection       = var.rds_deletion_protection
  skip_final_snapshot       = !var.rds_deletion_protection
  final_snapshot_identifier = var.rds_deletion_protection ? "${local.prefix}-final-${formatdate("YYYYMMDDhhmmss", timestamp())}" : null

  apply_immediately = false

  auto_minor_version_upgrade = true

  monitoring_interval = 60
  monitoring_role_arn = aws_iam_role.rds_monitoring.arn

  tags = local.common_tags

  lifecycle {
    ignore_changes = [final_snapshot_identifier]
  }
}

resource "aws_iam_role" "rds_monitoring" {
  name = "${local.prefix}-rds-monitoring"

  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect = "Allow"
      Principal = { Service = "monitoring.rds.amazonaws.com" }
      Action = "sts:AssumeRole"
    }]
  })

  tags = local.common_tags
}

resource "aws_iam_role_policy_attachment" "rds_monitoring" {
  role       = aws_iam_role.rds_monitoring.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AmazonRDSEnhancedMonitoringRole"
}
```

### 5.8 `elasticache.tf`

```hcl
resource "aws_elasticache_subnet_group" "efe" {
  name       = "${local.prefix}-redis-subnet"
  subnet_ids = module.vpc.database_subnets

  tags = local.common_tags
}

resource "aws_security_group" "redis" {
  name        = "${local.prefix}-redis-sg"
  description = "Allow Redis from EKS workloads only"
  vpc_id      = module.vpc.vpc_id

  ingress {
    description     = "Redis from EKS"
    from_port       = 6379
    to_port         = 6379
    protocol        = "tcp"
    security_groups = [module.eks.cluster_primary_security_group_id]
  }

  tags = local.common_tags
}

resource "random_password" "redis" {
  length  = 48
  special = false                       # ElastiCache auth_token disallows specials
}

resource "aws_elasticache_replication_group" "efe" {
  replication_group_id = "${local.prefix}-redis"
  description          = "EFE Redis Stack for ${var.tenant}"

  engine         = "redis"
  engine_version = "7.1"
  node_type      = var.redis_node_type
  port           = 6379

  num_cache_clusters         = var.redis_num_cache_clusters
  parameter_group_name       = "default.redis7"
  subnet_group_name          = aws_elasticache_subnet_group.efe.name
  security_group_ids         = [aws_security_group.redis.id]

  at_rest_encryption_enabled = true
  transit_encryption_enabled = true
  auth_token                 = random_password.redis.result

  automatic_failover_enabled = true
  multi_az_enabled           = true

  snapshot_retention_limit = 7
  snapshot_window          = "02:00-03:00"
  maintenance_window       = "sun:04:00-sun:05:00"

  apply_immediately = false

  log_delivery_configuration {
    destination      = aws_cloudwatch_log_group.redis_slow.name
    destination_type = "cloudwatch-logs"
    log_format       = "json"
    log_type         = "slow-log"
  }

  tags = local.common_tags
}

resource "aws_cloudwatch_log_group" "redis_slow" {
  name              = "/aws/elasticache/${local.prefix}/slow"
  retention_in_days = 30
  kms_key_id        = aws_kms_key.cloudwatch.arn

  tags = local.common_tags
}
```

### 5.9 `s3.tf` (audit bucket)

```hcl
resource "aws_s3_bucket" "audit" {
  bucket = "${local.prefix}-audit"

  tags = merge(local.common_tags, {
    "efe.io/purpose" = "audit-archive"
  })
}

resource "aws_s3_bucket_versioning" "audit" {
  bucket = aws_s3_bucket.audit.id

  versioning_configuration {
    status = "Enabled"
  }
}

resource "aws_s3_bucket_server_side_encryption_configuration" "audit" {
  bucket = aws_s3_bucket.audit.id

  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm     = "aws:kms"
      kms_master_key_id = var.enable_kms_byok ? aws_kms_key.byok[0].arn : aws_kms_key.s3.arn
    }
    bucket_key_enabled = true
  }
}

resource "aws_s3_bucket_public_access_block" "audit" {
  bucket                  = aws_s3_bucket.audit.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

resource "aws_s3_bucket_lifecycle_configuration" "audit" {
  bucket = aws_s3_bucket.audit.id

  rule {
    id     = "audit-retention"
    status = "Enabled"

    filter {}                           # apply to whole bucket

    transition {
      days          = 90
      storage_class = "STANDARD_IA"
    }

    transition {
      days          = 365
      storage_class = "GLACIER"
    }

    expiration {
      days = var.audit_bucket_retention_days
    }

    noncurrent_version_expiration {
      noncurrent_days = 90
    }

    abort_incomplete_multipart_upload {
      days_after_initiation = 7
    }
  }
}

resource "aws_s3_bucket_logging" "audit" {
  bucket = aws_s3_bucket.audit.id

  target_bucket = aws_s3_bucket.audit_access_log.id
  target_prefix = "access-logs/"
}

resource "aws_s3_bucket" "audit_access_log" {
  bucket = "${local.prefix}-audit-access-log"

  tags = merge(local.common_tags, {
    "efe.io/purpose" = "s3-access-log"
  })
}

resource "aws_s3_bucket_public_access_block" "audit_access_log" {
  bucket                  = aws_s3_bucket.audit_access_log.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

resource "aws_iam_policy" "audit_bucket_write" {
  name        = "${local.prefix}-audit-bucket-write"
  description = "Allow efe-server to write audit chains to S3"

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect = "Allow"
        Action = [
          "s3:PutObject",
          "s3:PutObjectAcl",
          "s3:AbortMultipartUpload",
          "s3:GetObject",               # for chain verification
        ]
        Resource = "${aws_s3_bucket.audit.arn}/*"
      },
      {
        Effect = "Allow"
        Action = ["s3:ListBucket"]
        Resource = aws_s3_bucket.audit.arn
      },
    ]
  })
}
```

### 5.10 `kms.tf`

```hcl
# RDS encryption key (managed automatically by AWS)
# (defined in rds.tf)

# EBS encryption (node group volumes)
resource "aws_kms_key" "ebs" {
  description             = "EBS encryption for ${local.prefix} nodes"
  deletion_window_in_days = 30
  enable_key_rotation     = true

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Sid       = "Enable IAM User Permissions"
        Effect    = "Allow"
        Principal = { AWS = "arn:aws:iam::${data.aws_caller_identity.current.account_id}:root" }
        Action    = "kms:*"
        Resource  = "*"
      },
      {
        Sid    = "Allow EC2 to use the key"
        Effect = "Allow"
        Principal = {
          AWS = "arn:aws:iam::${data.aws_caller_identity.current.account_id}:role/aws-service-role/autoscaling.amazonaws.com/AWSServiceRoleForAutoScaling"
        }
        Action = [
          "kms:Encrypt",
          "kms:Decrypt",
          "kms:ReEncrypt*",
          "kms:GenerateDataKey*",
          "kms:DescribeKey",
          "kms:CreateGrant",
        ]
        Resource = "*"
      },
    ]
  })

  tags = local.common_tags
}

# EKS Secrets encryption
resource "aws_kms_key" "eks_secrets" {
  description             = "EKS secrets encryption for ${local.prefix}"
  deletion_window_in_days = 30
  enable_key_rotation     = true

  tags = local.common_tags
}

# CloudWatch Logs encryption
resource "aws_kms_key" "cloudwatch" {
  description             = "CloudWatch Logs encryption for ${local.prefix}"
  deletion_window_in_days = 30
  enable_key_rotation     = true

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect    = "Allow"
        Principal = { AWS = "arn:aws:iam::${data.aws_caller_identity.current.account_id}:root" }
        Action    = "kms:*"
        Resource  = "*"
      },
      {
        Effect = "Allow"
        Principal = { Service = "logs.${var.region}.amazonaws.com" }
        Action = [
          "kms:Encrypt*",
          "kms:Decrypt*",
          "kms:ReEncrypt*",
          "kms:GenerateDataKey*",
          "kms:Describe*",
        ]
        Resource = "*"
        Condition = {
          ArnLike = {
            "kms:EncryptionContext:aws:logs:arn" = "arn:aws:logs:${var.region}:${data.aws_caller_identity.current.account_id}:log-group:*"
          }
        }
      },
    ]
  })

  tags = local.common_tags
}

# S3 fallback encryption (when BYOK disabled)
resource "aws_kms_key" "s3" {
  description             = "S3 audit bucket encryption (fallback when BYOK disabled)"
  deletion_window_in_days = 30
  enable_key_rotation     = true

  tags = local.common_tags
}

# BYOK key (customer-managed, optional)
data "aws_caller_identity" "current" {}

resource "aws_kms_key" "byok" {
  count = var.enable_kms_byok ? 1 : 0

  description             = "EFE BYOK key for ${var.tenant}"
  deletion_window_in_days = 30
  enable_key_rotation     = true

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Sid       = "Enable IAM User Permissions"
        Effect    = "Allow"
        Principal = { AWS = "arn:aws:iam::${data.aws_caller_identity.current.account_id}:root" }
        Action    = "kms:*"
        Resource  = "*"
      },
    ]
  })

  tags = merge(local.common_tags, {
    "efe.io/purpose" = "byok"
  })
}

resource "aws_kms_alias" "byok" {
  count = var.enable_kms_byok ? 1 : 0

  name          = "alias/${local.prefix}-byok"
  target_key_id = aws_kms_key.byok[0].key_id
}
```

### 5.11 `secrets.tf`

```hcl
resource "aws_secretsmanager_secret" "rds_credentials" {
  name        = "${local.prefix}/rds/credentials"
  description = "RDS Postgres credentials for ${local.prefix}"
  kms_key_id  = aws_kms_key.rds.arn

  recovery_window_in_days = 7

  tags = local.common_tags
}

resource "aws_secretsmanager_secret_version" "rds_credentials" {
  secret_id = aws_secretsmanager_secret.rds_credentials.id

  secret_string = jsonencode({
    username           = aws_db_instance.efe.username
    password           = random_password.rds.result
    host               = aws_db_instance.efe.address
    port               = aws_db_instance.efe.port
    dbname             = "efe"
    connection_string  = "postgresql://${aws_db_instance.efe.username}:${urlencode(random_password.rds.result)}@${aws_db_instance.efe.address}:${aws_db_instance.efe.port}/efe?sslmode=require"
  })
}

resource "aws_secretsmanager_secret" "redis_credentials" {
  name        = "${local.prefix}/redis/credentials"
  description = "Redis auth_token for ${local.prefix}"
  kms_key_id  = aws_kms_key.rds.arn

  recovery_window_in_days = 7

  tags = local.common_tags
}

resource "aws_secretsmanager_secret_version" "redis_credentials" {
  secret_id = aws_secretsmanager_secret.redis_credentials.id

  secret_string = jsonencode({
    auth_token        = random_password.redis.result
    host              = aws_elasticache_replication_group.efe.primary_endpoint_address
    port              = aws_elasticache_replication_group.efe.port
    connection_string = "rediss://:${urlencode(random_password.redis.result)}@${aws_elasticache_replication_group.efe.primary_endpoint_address}:${aws_elasticache_replication_group.efe.port}"
  })
}

resource "random_password" "audit_hmac_seed" {
  length  = 64
  special = false
}

resource "aws_secretsmanager_secret" "audit_hmac_seed" {
  name        = "${local.prefix}/audit/hmac-seed"
  description = "HMAC seed for D14-bis audit chain. NEVER ROTATE except via planned RB-04 procedure."
  kms_key_id  = aws_kms_key.rds.arn

  recovery_window_in_days = 30          # extra safety on this critical secret

  tags = merge(local.common_tags, {
    "efe.io/purpose"     = "audit-chain-hmac"
    "efe.io/criticality" = "p0"
  })
}

resource "aws_secretsmanager_secret_version" "audit_hmac_seed" {
  secret_id     = aws_secretsmanager_secret.audit_hmac_seed.id
  secret_string = random_password.audit_hmac_seed.result

  lifecycle {
    ignore_changes = [secret_string]   # prevent unintended rotation
  }
}
```

### 5.12 `outputs.tf`

```hcl
output "kubeconfig_command" {
  description = "Command to update local kubeconfig for the EKS cluster."
  value       = "aws eks update-kubeconfig --region ${var.region} --name ${module.eks.cluster_name}"
}

output "eks_cluster_name" {
  value = module.eks.cluster_name
}

output "eks_cluster_endpoint" {
  value = module.eks.cluster_endpoint
}

output "eks_oidc_provider_arn" {
  value = module.eks.oidc_provider_arn
}

output "rds_endpoint" {
  value = aws_db_instance.efe.address
}

output "rds_port" {
  value = aws_db_instance.efe.port
}

output "rds_secret_arn" {
  value       = aws_secretsmanager_secret.rds_credentials.arn
  description = "ARN of the Secrets Manager secret containing RDS credentials."
}

output "redis_secret_arn" {
  value = aws_secretsmanager_secret.redis_credentials.arn
}

output "byok_kms_key_arn" {
  value       = var.enable_kms_byok ? aws_kms_key.byok[0].arn : null
  description = "ARN of the customer-managed BYOK KMS key (null when disabled)."
}

output "audit_bucket_name" {
  value = aws_s3_bucket.audit.bucket
}

output "audit_bucket_arn" {
  value = aws_s3_bucket.audit.arn
}

output "efe_server_irsa_role_arn" {
  value = module.efe_server_irsa.iam_role_arn
}

output "vpc_id" {
  value = module.vpc.vpc_id
}

output "private_subnets" {
  value = module.vpc.private_subnets
}

output "audit_hmac_secret_arn" {
  value = aws_secretsmanager_secret.audit_hmac_seed.arn
}
```

### 5.13 Tests `terratest`

```go
// terraform/modules/aws/test/aws_basic_test.go
package test

import (
    "fmt"
    "testing"

    "github.com/aws/aws-sdk-go/aws"
    "github.com/aws/aws-sdk-go/aws/session"
    "github.com/aws/aws-sdk-go/service/eks"
    "github.com/gruntwork-io/terratest/modules/random"
    "github.com/gruntwork-io/terratest/modules/terraform"
    "github.com/stretchr/testify/assert"
    "github.com/stretchr/testify/require"
)

func TestEfeAwsBasic(t *testing.T) {
    t.Parallel()

    region := "eu-west-1"
    tenant := fmt.Sprintf("tt%s", random.UniqueId())

    opts := &terraform.Options{
        TerraformDir: "../examples/basic",
        Vars: map[string]interface{}{
            "region": region,
            "tenant": tenant,
            "data_residency": "EU_ONLY",
            "rds_deletion_protection": false,
            "rds_multi_az": false,                  // cheaper for tests
            "rds_allocated_storage": 20,
            "redis_num_cache_clusters": 2,
            "eks_node_groups": map[string]map[string]interface{}{
                "primary": {
                    "instance_types": []string{"t3.medium"},
                    "desired_size":   2,
                    "min_size":       2,
                    "max_size":       4,
                    "capacity_type":  "ON_DEMAND",
                },
            },
        },
        EnvVars: map[string]string{
            "AWS_REGION": region,
        },
    }
    defer terraform.Destroy(t, opts)
    terraform.InitAndApply(t, opts)

    // Output checks
    require.NotEmpty(t, terraform.Output(t, opts, "kubeconfig_command"))
    require.NotEmpty(t, terraform.Output(t, opts, "eks_cluster_name"))
    require.NotEmpty(t, terraform.Output(t, opts, "rds_endpoint"))

    // EKS cluster ACTIVE
    sess := session.Must(session.NewSession(&aws.Config{Region: &region}))
    eksClient := eks.New(sess)
    clusterName := terraform.Output(t, opts, "eks_cluster_name")
    cluster, err := eksClient.DescribeCluster(&eks.DescribeClusterInput{
        Name: &clusterName,
    })
    require.NoError(t, err)
    assert.Equal(t, "ACTIVE", *cluster.Cluster.Status)

    // OIDC issuer set
    require.NotNil(t, cluster.Cluster.Identity.Oidc)
    assert.Contains(t, *cluster.Cluster.Identity.Oidc.Issuer, "oidc.eks")
}

func TestEfeAwsResidencyEnforcement(t *testing.T) {
    t.Parallel()

    // Should fail: EU_ONLY with us-east-1
    opts := &terraform.Options{
        TerraformDir: "../examples/basic",
        Vars: map[string]interface{}{
            "region":         "us-east-1",
            "tenant":         "rejtest",
            "data_residency": "EU_ONLY",
        },
    }
    _, err := terraform.InitAndPlanE(t, opts)
    require.Error(t, err)
    assert.Contains(t, err.Error(), "data_residency")
}
```

8 tests `terratest` au total : `TestEfeAwsBasic`, `TestEfeAwsHA`, `TestEfeAwsResidencyEnforcement`, `TestEfeAwsByokDisabled`, `TestEfeAwsAuditBucketEncrypted`, `TestEfeAwsRdsMultiAz`, `TestEfeAwsRedisAuth`, `TestEfeAwsIrsaWorks`.

---

## §6. Runbooks RB-01 à RB-06

### 6.1 RB-01 : Disque DB Postgres saturé (> 85 %)

```markdown
# RB-01 : Disque Postgres saturé (> 85 %)

## Métadonnées
- **Sévérité** : P1 (risque imminent de corruption + downtime)
- **MTTR cible** : 30 min
- **Frequency** : ~ 1-2 / an / tenant production
- **Last reviewed** : 2026-05-08
- **Owner primaire** : DBA on-call
- **Owner backup** : SRE on-call

## Symptômes
- Alerte Prometheus : `EfePostgresDiskFull` (cf. § 4.6.3)
- Métrique Grafana : panel « Postgres disk usage » → red zone (> 85 %)
- Log pattern : `WARNING: disk-only sort: ...` ou `ERROR: could not extend file`
- Page on-call : **OUI** P1, page DBA + SRE

## Diagnostic (≤ 5 min)
1. Identifier le cluster + l'instance impactés :
   ```bash
   kubectl get pvc -n efe -o json | jq -r '
     .items[] | select(.metadata.name|test("data-efe-postgres")) |
       "\(.metadata.name)\t\(.status.capacity.storage)"'
   ```

2. Calculer l'occupation actuelle par table (top 10) :
   ```bash
   kubectl exec -n efe efe-postgres-instance1-0 -c database -- \
     psql -U postgres -d efe -c "
       SELECT relname, pg_size_pretty(pg_total_relation_size(C.oid)) AS size
         FROM pg_class C
         LEFT JOIN pg_namespace N ON N.oid = C.relnamespace
        WHERE nspname = 'public'
        ORDER BY pg_total_relation_size(C.oid) DESC
        LIMIT 10;
     "
   ```

3. Vérifier les bloats (autovacuum lag) :
   ```sql
   SELECT relname, n_dead_tup, n_live_tup,
          round(100 * n_dead_tup / NULLIF(n_live_tup, 0), 1) AS dead_pct
     FROM pg_stat_user_tables
    WHERE n_dead_tup > 1000
    ORDER BY dead_pct DESC NULLS LAST
    LIMIT 10;
   ```

## Procédure de résolution

### Étape 1 : Stabiliser (≤ 5 min)
1. **Geler les nouvelles inserts massifs** côté tenant impacté :
   ```bash
   efe-admin tenant suspend acme \
     --reason "RB-01: emergency disk pressure - bulk operations frozen" --yes
   ```
   ⚠️ Cela bloque uniquement les classify ; les SELECT continuent (lecture).

2. Snapshot RDS immédiat (sécurité) :
   ```bash
   aws rds create-db-snapshot \
     --db-instance-identifier efe-acme-postgres \
     --db-snapshot-identifier efe-acme-rb01-$(date +%s)
   ```

### Étape 2 : Diagnostiquer (≤ 10 min)
3. Identifier la cause racine probable (3 cas possibles) :

   **Cas A** : croissance organique du `llm_call_audit` (M21+). Solution : ajouter du stockage.

   **Cas B** : autovacuum lag accumulant des dead tuples. Solution : VACUUM FULL ciblé.

   **Cas C** : bug applicatif émettant des doublons. Solution : pause + investigation.

### Étape 3 : Corriger
4. **Cas A** : étendre le storage RDS (immédiat, sans downtime) :
   ```bash
   aws rds modify-db-instance \
     --db-instance-identifier efe-acme-postgres \
     --allocated-storage $(aws rds describe-db-instances \
       --db-instance-identifier efe-acme-postgres \
       --query 'DBInstances[0].AllocatedStorage' --output text \
       | awk '{print int($1 * 1.5)}') \
     --apply-immediately
   ```
   Postgres reprend le service en 5-10 min après l'extension.

5. **Cas B** : VACUUM FULL ciblé sur la table impactée (durant maintenance window) :
   ```sql
   VACUUM FULL VERBOSE llm_call_audit;
   ```
   Cette opération **lock la table** pendant son exécution (typiquement 5-30 min selon volume).

6. **Cas C** : escalader vers SRE + dev. Mettre l'application en read-only mode :
   ```bash
   kubectl patch -n efe deploy/efe-server --type merge \
     -p '{"spec":{"template":{"spec":{"containers":[{"name":"efe-server","env":[{"name":"EFE_READ_ONLY_MODE","value":"true"}]}]}}}}'
   ```

### Étape 4 : Vérifier
7. Recalculer le ratio occupation :
   ```bash
   kubectl exec -n efe efe-postgres-instance1-0 -c database -- \
     df -h /pgdata
   ```
   Cible : < 70 % post-correction.

8. Réactiver les écritures :
   ```bash
   efe-admin tenant update acme --max-rps 100 --yes
   ```

### Étape 5 : Communiquer
- Slack #efe-incidents : duration + RCA preview
- Statuspage : update si > 30 min
- Email tenant Acme : si suspended pendant > 15 min

## Post-incident
- Postmortem si MTTR > 1 h
- Action items : tuner `autovacuum_vacuum_scale_factor` à 0.05 (default 0.2) sur tables M21+
- Augmenter alerte à 75 % (vs 85 %) pour anticiper

## Procédure de validation (Game Day trimestriel)
**Reproduire en staging** :
```bash
# Inflate llm_call_audit to ~ 90 %
kubectl exec -n efe-staging efe-postgres-instance1-0 -c database -- \
  psql -U postgres -d efe -c "
    INSERT INTO llm_call_audit (...) SELECT ... FROM generate_series(1, 50000000);
  "
```
Doit déclencher l'alerte Prometheus dans ≤ 5 min.

**Test trimestriel** : premier mardi du trimestre, owner DBA Lead.
```

### 6.2 RB-02 : Circuit breaker provider LLM ouvert > 5 min

```markdown
# RB-02 : Circuit breaker LLM ouvert > 5 min

## Métadonnées
- **Sévérité** : P2
- **MTTR cible** : 15 min
- **Frequency** : ~ 5-10 / mois (provider issues, rate limits)
- **Owner primaire** : LLM Ops
- **Owner backup** : SRE

## Symptômes
- Alerte Prometheus : `EfeLlmBreakerOpen`
- Métrique Grafana : panel « LLM Provider Health » → un ou plusieurs providers en `OPEN`
- Logs : `breaker tripped: 5 consecutive failures from anthropic-sonnet`
- Impact tenant : Mode-2 + Mode-3 calls fail with `degraded_reason: provider_breaker_open`

## Diagnostic (≤ 5 min)
1. Identifier le(s) provider(s) impacté(s) :
   ```bash
   kubectl exec -n efe deploy/efe-server -- \
     curl -s http://localhost:8081/internal/breakers | jq '.[] | select(.state=="OPEN")'
   ```

2. Vérifier les métriques side-channel :
   - Provider's status page (status.anthropic.com, status.openai.com, etc.)
   - Cost / RPS dashboard last 30 min

3. Lister les tenants à exposition Mode-2/3 :
   ```sql
   SELECT tenant_id FROM tenant_feature_mode2 WHERE enabled=TRUE
   UNION
   SELECT tenant_id FROM tenant_feature_mode3 WHERE enabled=TRUE;
   ```

## Procédure de résolution

### Étape 1 : Stabiliser
1. Vérifier qu'**aucun tenant** ne dépend exclusivement du provider en panne :
   ```sql
   SELECT tenant_id, providers
     FROM tenant_llm_config
    WHERE 'anthropic-sonnet' = ANY(providers)
      AND array_length(providers, 1) = 1;
   ```
   Si non-vide → **escalation P1**.

2. Vérifier que les fallback providers fonctionnent (tester un classify Mode-2 si configuré) :
   ```bash
   efe-admin classify-test acme --mode 2 --provider openai-gpt4
   ```

### Étape 2 : Diagnostiquer
3. Si breaker OPEN > 10 min ET status page provider est GREEN :
   - Vérifier nos credentials (token expired ?)
     ```bash
     kubectl get secret -n efe efe-llm-credentials -o json \
       | jq -r '.data."anthropic-key"' | base64 -d | head -c 20
     ```
   - Vérifier nos quotas API
     ```bash
     curl -H "x-api-key: $ANTHROPIC_KEY" https://api.anthropic.com/v1/usage
     ```

### Étape 3 : Corriger
4. **Cas A : credentials expirés** → rotate via vault :
   ```bash
   efe-admin keys rotate-llm acme --provider anthropic --yes
   ```

5. **Cas B : provider rate-limit** → réduire temporairement le RPS Mode-2/3 :
   ```bash
   efe-admin quotas set acme --max-llm-rps 5 --yes
   ```

6. **Cas C : provider en panne** → forcer la bascule sur un autre provider :
   ```bash
   efe-admin llm-config set acme --primary-provider openai --yes
   ```

7. Reset manuel du breaker quand le provider est rétabli :
   ```bash
   kubectl exec -n efe deploy/efe-server -- \
     curl -X POST http://localhost:8081/internal/breakers/anthropic-sonnet/reset
   ```

### Étape 4 : Vérifier
8. Tester un Mode-2 classify :
   ```bash
   efe-admin classify-test acme --mode 2
   ```

### Étape 5 : Communiquer
- Statuspage : update « Mode-2 / Mode-3 calls degraded due to provider X » (si > 10 min)
- Slack #efe-incidents
- Email aux tenants utilisant Mode-2/3 si > 30 min

## Post-incident
- Si pattern récurrent (même provider) : revue contractuelle SLA provider
- RCA dans 48 h

## Procédure de validation
```bash
# Force a breaker open in staging
kubectl exec -n efe-staging deploy/efe-server -- \
  curl -X POST http://localhost:8081/internal/breakers/test-provider/force-open
```
Doit déclencher l'alerte Prometheus dans ≤ 5 min.
```

### 6.3 RB-03 : Audit chain HMAC broken (P0)

(Détaillé dans le doc d'architecture GLOSS § 6.3 — réutilisé ici.)

```markdown
# RB-03 : Audit chain HMAC broken

## Métadonnées
- **Sévérité** : P0 (compliance breach potentiel — GDPR Art. 33 timer 72h)
- **MTTR cible** : 2 h
- **Frequency** : < 1 / an attendu
- **Owner primaire** : Compliance Engineer
- **Owner backup** : Security Lead

## Symptômes
- Alerte Prometheus : `EfeAuditChainBroken`
- Grafana panel « Audit chain integrity » → red
- Log : `audit chain verify failed: prev_hmac mismatch at audit_id=N`

## Diagnostic
1. Identifier audit_id + tenant impacté :
   ```bash
   kubectl exec -n efe deploy/efe-server -- /app/bin/efe-verify-audit-chain --json
   ```

2. Lire 10 lignes audit autour du break :
   ```sql
   SELECT id, ts, tenant, mode, prev_hmac, hmac
     FROM llm_call_audit
    WHERE id BETWEEN N-5 AND N+5
    ORDER BY id;
   ```

3. Vérifier WAL Postgres :
   ```bash
   kubectl exec -n efe efe-postgres-instance1-0 -- \
     pg_waldump --start=$(get_lsn_for_audit_id N)
   ```

## Procédure
### Étape 1 : Stabiliser (≤ 5 min)
4. **STOP les écritures classify pour le tenant impacté** :
   ```bash
   efe-admin tenant suspend acme --reason "AUDIT_CHAIN_BREAK_INVESTIGATION" --yes
   ```

5. Snapshot DB immédiat (forensics) :
   ```bash
   kubectl exec -n efe efe-postgres-instance1-0 -- \
     pg_dump -t llm_call_audit -t mode3_review_queue -t ui_audit_log \
       -F custom -f /tmp/audit_snapshot_$(date +%s).pgcustom
   ```

### Étape 2 : Diagnostiquer (≤ 30 min)
- **Cas A : break naturel** (trigger défaillant) → vérifier la fonction `audit_chain_trigger()`
- **Cas B : manipulation manuelle** (UPDATE/DELETE hors trigger) → INCIDENT SECURITE
- **Cas C : corruption disque** → `pg_checksums --check`

### Étape 3 : Corriger
- **Cas A** : redéployer migration ou hotfix trigger ; recalculer hmac depuis l'audit_id le plus ancien correct
- **Cas B** : escalader Security Lead, lancer forensics, audit IAM Postgres
- **Cas C** : restore depuis backup pgBackRest le plus proche, replay WAL

### Étape 4 : Vérifier
- Re-run `efe-verify-audit-chain --tenant=acme` → OK

### Étape 5 : Communiquer
- DPO notification **OBLIGATOIRE** sous 72h (GDPR Art. 33)
- Email tenant + Statuspage + Slack
- Page CTO si Cas B

## Post-incident
- Postmortem 72h max
- Action items : test régression, durcir trigger, audit IAM Postgres

## Procédure de validation
**Reproduce en staging UNIQUEMENT** :
```sql
UPDATE llm_call_audit SET hmac = 'corrupted'
 WHERE id = (SELECT MAX(id) - 5 FROM llm_call_audit);
```
Doit déclencher l'alerte ≤ 60s.

**Game Day trimestriel** : premier mardi/trimestre.
```

### 6.4 RB-04 : Rotation forcée clé BYOK (KMS compromise)

```markdown
# RB-04 : Rotation forcée clé BYOK (KMS compromise)

## Métadonnées
- **Sévérité** : P0
- **MTTR cible** : 4 h
- **Frequency** : < 1 / 3 ans (espoir)
- **Owner primaire** : Security Lead
- **Owner backup** : Compliance Engineer

## Déclencheurs
- Email AWS KMS Security : « Suspicious activity on key X »
- Alerte CloudTrail : `kms:Decrypt` from unusual IP
- Audit interne révélant fuite de clé
- Demande explicite tenant (DPO right-to-rectify)

## Procédure

### Étape 1 : Préparer (≤ 30 min)
1. Page CTO + Security Lead + DPO impactés
2. Geler les écritures du tenant :
   ```bash
   efe-admin tenant suspend acme --reason "KMS-COMPROMISE-ROTATION" --yes
   ```
3. Lister tous les ressources chiffrées avec la clé compromise :
   - Audit chain rows (BYOK envelope)
   - S3 audit bucket objects
   - Secrets Manager secrets

### Étape 2 : Provisionner nouvelle clé (≤ 30 min)
4. Créer une nouvelle clé KMS BYOK (séparée de l'ancienne) :
   ```bash
   aws kms create-key --description "EFE acme BYOK v2 (post-RB-04)" \
     --key-usage ENCRYPT_DECRYPT
   aws kms create-alias --alias-name alias/efe-acme-byok-v2 --target-key-id $NEW_KEY_ID
   ```

### Étape 3 : Re-key les données (≤ 2 h)
5. **Audit chain (rows in `llm_call_audit`)** :
   ```bash
   kubectl exec -n efe deploy/efe-server -- /app/bin/efe-rekey-audit \
     --tenant acme --from-key $OLD_KEY_ARN --to-key $NEW_KEY_ARN
   ```

6. **S3 audit bucket** : copier vers nouveau prefix avec nouvelle SSE-KMS :
   ```bash
   aws s3 cp s3://efe-acme-audit/ s3://efe-acme-audit-v2/ --recursive \
     --sse aws:kms --sse-kms-key-id $NEW_KEY_ID
   ```

7. **Secrets Manager** : rotation (chacun individuellement) :
   ```bash
   for s in $(aws secretsmanager list-secrets \
     --filters Key=tag-value,Values=acme \
     --query 'SecretList[*].Name' --output text); do
     aws secretsmanager rotate-secret --secret-id $s
   done
   ```

### Étape 4 : Switch & vérifier
8. Update Helm value :
   ```bash
   helm upgrade efe-stack ./charts/efe-stack \
     --set efe-server.byok.awsKmsKeyId=$NEW_KEY_ARN --reuse-values
   ```

9. Test classify smoke :
   ```bash
   efe-admin smoke-test acme
   ```

### Étape 5 : Schedule destruction de l'ancienne clé
10. Schedule deletion KMS (window 30 jours) :
    ```bash
    aws kms schedule-key-deletion --key-id $OLD_KEY_ID --pending-window-in-days 30
    ```

### Étape 6 : Communication
- Email tenant : notification GDPR Art. 33 + Art. 34 (DPO obligation)
- Page DPO + Legal
- Statuspage : downtime documenté

## Post-incident
- Postmortem détaillé
- Audit AWS CloudTrail des 90 derniers jours sur l'ancienne clé
- Notification autorités de contrôle si breach matériel (CNIL en France, ICO UK, etc.)

## Procédure de validation
Test trimestriel **NON-DESTRUCTIF** : exécuter le script `efe-rekey-audit --dry-run` qui simule la rotation sans modifier de données.
```

### 6.5 RB-05 : Tenant en BudgetExhausted non récupérable

```markdown
# RB-05 : Tenant `BudgetExhausted` non récupérable

## Métadonnées
- **Sévérité** : P3
- **MTTR cible** : 4 h
- **Frequency** : ~ 2-5 / mois
- **Owner primaire** : Account Executive
- **Owner backup** : LLM Ops

## Symptômes
- Alerte Prometheus : `EfeTenantBudgetExhausted`
- Logs efe-server : `BudgetExhausted { kind: "monthly_cap" }` répétés
- Email tenant : « Mode-2/3 calls failing »

## Diagnostic
1. État budget tenant :
   ```bash
   efe-admin quotas get acme --format json
   ```

2. Consommation réelle :
   ```sql
   SELECT date_trunc('day', ts) AS d, sum(cost_usd) AS spent
     FROM llm_call_audit
    WHERE tenant = 'acme'
      AND ts > now() - interval '30 days'
    GROUP BY 1 ORDER BY 1 DESC;
   ```

## Procédure
### Étape 1 : Comprendre
3. Comparer consommation vs cap mensuel :
   - Si consommation > 95 % cap : limite normale, **escalation Account Executive**
   - Si consommation < 95 % mais erreurs : **bug**

### Étape 2 : Cas normal (consommation ≈ cap)
4. Account Executive contacte tenant pour upsell ou augmentation cap.
5. Une fois validé contractuellement :
   ```bash
   efe-admin quotas set acme \
     --monthly-cap-usd 5000 --max-tokens-per-day 5000000 --yes
   ```

### Étape 3 : Cas bug
6. Vérifier les locks Postgres potentiellement orphelins :
   ```sql
   SELECT * FROM pg_locks WHERE NOT granted;
   ```
7. Si locks suspects : restart efe-server pods.
8. Vérifier intégrité de la table de tracking :
   ```sql
   SELECT * FROM tenant_llm_budget WHERE tenant_id = 'acme';
   ```
   Si corrompue : recalculer depuis `llm_call_audit`.

## Communication
- Email tenant si Mode-2/3 down > 1 h
- Pas d'escalation automatique (P3)

## Procédure de validation
En staging, force `monthly_cap = 0.01` puis exécute 100 classify Mode-2. Doit déclencher l'alerte ≤ 10 min.
```

### 6.6 RB-06 : Pod efe-server CrashLoopBackOff

```markdown
# RB-06 : efe-server CrashLoopBackOff

## Métadonnées
- **Sévérité** : P1
- **MTTR cible** : 30 min
- **Frequency** : ~ 1-2 / mois lors de releases
- **Owner primaire** : SRE
- **Owner backup** : Backend Lead

## Symptômes
- Alerte Prometheus : `EfeServerCrashLoopBackOff`
- `kubectl get pods` → `1/1 Running` mais `RESTARTS` croissant
- p99 latency dégradée (HPA scale-up déclenché)

## Diagnostic
1. État pods :
   ```bash
   kubectl get pods -n efe -l app.kubernetes.io/name=efe-server -o wide
   ```

2. Logs du dernier crash :
   ```bash
   POD=$(kubectl get pods -n efe -l app.kubernetes.io/name=efe-server \
     --sort-by=.status.startTime -o name | head -1)
   kubectl logs -n efe $POD --previous --tail=200
   ```

3. Events Kubernetes :
   ```bash
   kubectl describe pod -n efe $POD | tail -50
   ```

## Procédure

### Étape 1 : Stabiliser
4. Si CrashLoop récent (< 5 min) **et** déploiement en cours : déclencher rollback :
   ```bash
   kubectl rollout undo -n efe deploy/efe-server
   ```

5. Si CrashLoop ancien : scaler à 0 le déploiement défaillant et relancer :
   ```bash
   kubectl scale -n efe deploy/efe-server --replicas=0
   sleep 30
   kubectl scale -n efe deploy/efe-server --replicas=3
   ```

### Étape 2 : Diagnostiquer
6. Cas typiques :
   - **OOM** (`OOMKilled`) → augmenter `resources.limits.memory`
   - **Probe timeout** → augmenter `startupProbe.failureThreshold`
   - **Migration DB échouée** → cf. RB-08 (Sprint C)
   - **Config invalide** → check `kubectl get cm efe-server -o yaml`
   - **Image pull error** → `kubectl describe pod` → ImagePullBackOff

### Étape 3 : Corriger
7. Cas OOM :
   ```bash
   helm upgrade efe-stack ./charts/efe-stack \
     --set efe-server.resources.limits.memory=8Gi --reuse-values
   ```

8. Cas migration DB échouée → corriger SQL, re-déployer migration via Job.

### Étape 4 : Vérifier
9. Tous pods Running :
   ```bash
   kubectl wait -n efe pod -l app.kubernetes.io/name=efe-server \
     --for=condition=Ready --timeout=120s
   ```

10. Smoke test :
    ```bash
    efe-admin smoke-test acme
    ```

### Étape 5 : Communiquer
- Statuspage si downtime > 10 min
- Slack #efe-incidents

## Post-incident
- Si récurrent : tuner probes, ajouter chaos test régression

## Procédure de validation
Force OOM en staging :
```bash
kubectl set resources -n efe-staging deploy/efe-server \
  --limits=memory=64Mi
```
Doit déclencher CrashLoopBackOff puis l'alerte ≤ 10 min.
```

---

## §7. Tests + CI

### 7.1 Tests Helm (helm-unittest)

| Sub-chart | Fichiers tests | Cas couverts |
|-----------|----------------|--------------|
| efe-server | 4 | deployment, service, hpa, networkpolicy |
| mcp-server | 3 | deployment, service, profile env |
| portal-admin | 3 | deployment, ingress, sso |
| postgres-operator | 4 | crd, postgrescluster, backup, monitoring |
| redis-stack | 3 | statefulset, service, auth |
| observability | 5 | prometheus rules, grafana dashboards, loki, tempo, alertmanager |
| umbrella | 3 | secrets, networkpolicy default-deny, NOTES |
| **Total** | **25** | |

CI workflow :

```yaml
# .github/workflows/helm-test.yml
name: helm-unittest

on:
  pull_request:
    paths:
      - 'charts/**'

jobs:
  helm-unittest:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: azure/setup-helm@v4
        with:
          version: v3.15.2
      - name: Install helm-unittest
        run: helm plugin install https://github.com/helm-unittest/helm-unittest --version 0.5.1
      - name: Run umbrella tests
        run: helm unittest charts/efe-stack
      - name: Run sub-chart tests
        run: |
          for c in charts/efe-stack/charts/*; do
            echo "Testing $c"
            helm unittest "$c" || exit 1
          done
      - name: Lint
        run: helm lint charts/efe-stack
      - name: Validate values.schema.json
        run: helm template charts/efe-stack --validate
```

### 7.2 Tests Terraform (terratest)

8 tests Go (~ 200 LOC chacun). CI workflow déclenche sur PR :

```yaml
# .github/workflows/terraform-aws.yml
name: terraform-aws

on:
  pull_request:
    paths:
      - 'terraform/modules/aws/**'

jobs:
  validate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: hashicorp/setup-terraform@v3
        with:
          terraform_version: 1.6.6
      - run: terraform fmt -check -recursive terraform/modules/aws
      - run: |
          cd terraform/modules/aws
          terraform init -backend=false
          terraform validate
      - uses: aquasecurity/tfsec-action@v1
        with:
          working_directory: terraform/modules/aws
          soft_fail: false
      - uses: bridgecrewio/checkov-action@v12
        with:
          directory: terraform/modules/aws
          framework: terraform

  terratest:
    if: github.event.pull_request.head.repo.full_name == github.repository
    runs-on: ubuntu-latest
    permissions:
      id-token: write
      contents: read
    steps:
      - uses: actions/checkout@v4
      - uses: aws-actions/configure-aws-credentials@v4
        with:
          role-to-assume: ${{ secrets.AWS_TERRATEST_ROLE_ARN }}
          aws-region: eu-west-1
      - uses: actions/setup-go@v5
        with:
          go-version: '1.22'
      - run: |
          cd terraform/modules/aws/test
          go test -timeout 60m -v ./...
```

### 7.3 Test E2E k3d local (`scripts/k3d-e2e.sh`)

```bash
#!/usr/bin/env bash
# scripts/k3d-e2e.sh — full local stack smoke test on k3d.
set -euo pipefail

CLUSTER="efe-e2e-$(date +%s)"

cleanup() {
  k3d cluster delete "$CLUSTER" || true
}
trap cleanup EXIT

# 1. k3d cluster
k3d cluster create "$CLUSTER" \
  --servers 1 --agents 2 \
  --port "8443:443@loadbalancer" \
  --wait --timeout 5m

# 2. Helm install
helm install efe-stack ./charts/efe-stack \
  --values ./charts/efe-stack/values-local.yaml \
  --set global.tenant=test \
  --wait --timeout 10m

# 3. Wait for migration job to complete
kubectl wait --for=condition=complete \
  job/efe-stack-migrate \
  --namespace default --timeout 5m

# 4. Run helm test
helm test efe-stack

# 5. CLI smoke
efe-admin tenant create acme --plan trial --yes
efe-admin smoke-test acme

echo "✅ E2E PASS"
```

CI : `make e2e-local` runs this on every PR (~ 10 min execution).

---

## §8. Plan de delivery (Sprint B detail)

### 8.1 Sprint B (semaine 2 GLOSS) — Day-by-Day

| Jour | Livrable | Tests | DoD partiel |
|------|----------|-------|-------------|
| **J6** | `charts/efe-stack/Chart.yaml` + umbrella values + helpers + `values.schema.json` | 3 helm-unittest | `helm template` réussit |
| **J7** | sub-charts `efe-server` + `mcp-server` complets + tests | 7 helm-unittest | `helm install` réussit sur cluster fake |
| **J8** | sub-charts `postgres-operator` + `redis-stack` + `observability` + `portal-admin` skeleton | 15 helm-unittest | k3d local install ≤ 5 min reproduit |
| **J9** | Terraform AWS module complet (vpc/eks/rds/elasticache/kms/iam/s3/secrets) | 8 terratest | `terraform apply` sandbox ≤ 12 min |
| **J10** | 6 runbooks (RB-01 à RB-06) + scripts validation + CI workflows | scripts validation | tous runbooks reproductibles staging |

### 8.2 Cumul tests post-Sprint B

| Couche | Pre-Sprint B | Post-Sprint B | Δ |
|--------|--------------|---------------|---|
| Rust efe-admin | 54 | 54 | — |
| Rust autres crates | 2 426 | 2 426 | — |
| Python mcp-server | 620 | 620 | — |
| Helm `helm-unittest` | 0 | **25** | +25 |
| Terraform `terratest` | 0 | **8** | +8 |
| E2E k3d local (bash) | 0 | **5** | +5 |
| Runbook validation scripts | 0 | **6** | +6 |
| **Cumul GREEN** | **~3 100** | **~3 144** | **+44** |

---

## §9. Critères d'acceptation détaillés

### 9.1 Performance

- `helm install efe-stack` (k3d local) : ≤ 5 min p95
- `helm install efe-stack` (cluster cloud) : ≤ 8 min p95
- `terraform apply` (AWS sandbox) : ≤ 12 min p95
- `helm uninstall` propre : ≤ 2 min (PVC supprimés via finalizers)
- `terraform destroy` propre : ≤ 8 min

### 9.2 Sécurité

- 100 % images Helm signées Sigstore (Cosign)
- 100 % `tfsec` HIGH/CRITICAL = 0
- 100 % `checkov` HIGH/CRITICAL = 0
- Pod Security Standards `restricted` enforced (namespace label)
- NetworkPolicies default-deny baseline
- Secrets Manager + KMS pour tous secrets sensibles
- IRSA (pas de credentials AWS statiques dans pods)
- VPC Endpoints (S3, KMS, Secrets Manager) pour réduire surface BGP

### 9.3 Compliance

- Encryption at rest : RDS ✅, ElastiCache ✅, EBS ✅, S3 audit ✅
- Encryption in transit : TLS 1.3 partout (RDS rds.force_ssl, ElastiCache transit_encryption, S3 HTTPS)
- Backup : RDS retention ≥ 14 jours, ElastiCache snapshots quotidiennes
- S3 audit bucket : versioning + lifecycle 7 ans + access logs
- KMS : key rotation enabled
- CloudTrail : audit AWS API calls (managed by tenant cloud account)

### 9.4 Backward compat

- 0 régression Sprint A : `efe-admin` CLI fonctionne contre stack Helm-déployé (smoke test E2E)
- 0 modification migration `0001-0013` (Helm utilise les migrations existantes via Job init)
- Default-OFF tools D8 préservé (Helm value `efe-server.features.d8AdminEnabled: false` par défaut)

---

## §10. Risques & mitigations

| ID | Risque | Probabilité | Impact | Mitigation |
|----|--------|-------------|--------|------------|
| **B-R1** | helm-unittest faux positifs (templates rendent mais déploient mal) | M | M | Doubler avec k3d real-deploy E2E sur chaque PR ; helm-unittest = filet rapide, k3d = filet final |
| **B-R2** | Provider AWS breaking change (Terraform module) | H | M | Pin `aws ~> 5.40` strict ; renovate-bot PR mensuelle ; CI test automatique sur upgrade |
| **B-R3** | PGO (CrunchyData) version drift vs Postgres 16 | L | M | Suivre Crunchy support matrix ; eval upgrades en staging avant prod |
| **B-R4** | k3d trop lourd pour laptops < 16 GB RAM | M | L | `values-local-tiny.yaml` (replicas=1, retention=1d, resources réduites) ; mock observability optionnel |
| **B-R5** | RDS extension storage triggers downtime impromptu | L | M | Documenter dans RB-01 que c'est `apply_immediately=true` no-downtime ; tester en staging |
| **B-R6** | NetworkPolicy default-deny casse traffic intra-stack | M | M | NP par sub-chart explicit ; test E2E vérifie inter-pod |
| **B-R7** | Helm chart upgrade casse l'état Postgres (init-Job non-idempotent) | M | H | sqlx migrate est idempotent par design ; test upgrade matrix `1.0.0 → 1.1.0 → 1.0.0` |
| **B-R8** | Image pull from `ghcr.io` rate-limited | L | L | Pull secret obligatoire ; option d'override `global.imageRegistry` pour mirror interne |
| **B-R9** | Terraform state corruption (backend S3) | L | C | Backend `s3 + dynamodb_table` (lock) ; versioning S3 ON ; backup state out-of-band |
| **B-R10** | Cost runaway sur EKS test (terratest) | M | L | terratest tear-down obligatoire (defer Destroy) ; budget alarm AWS sandbox |

---

## §11. Annexes

### 11.1 Sample `values-prod.yaml` (cloud production)

```yaml
global:
  domain: efe.acme.com
  tenant: acme
  environment: prod
  storageClass: gp3

efe-server:
  replicas: 6
  image:
    tag: "1.6.0"
  resources:
    requests: { cpu: 1, memory: 2Gi }
    limits:   { cpu: 4, memory: 8Gi }
  byok:
    backend: aws-kms
    awsKmsKeyId: "arn:aws:kms:eu-west-1:123456789012:key/abc..."
  serviceAccount:
    annotations:
      eks.amazonaws.com/role-arn: "arn:aws:iam::123456789012:role/efe-acme-efe-server-irsa"
  features:
    m20HelperToolsEnabled: false
    m21Mode2Enabled: true
    m22Mode3Enabled: false
    d8AdminEnabled: true                # SE uses efe-admin CLI

mcp-server:
  replicas: 4
  profile: agent-native-deploy

postgres:
  enabled: false                        # external RDS provisioned via Terraform

externalPostgres:
  enabled: true
  secretRef:
    name: efe-postgres-external
    keys:
      host: host
      port: port
      database: database
      username: username
      password: password

redis:
  enabled: false                        # external ElastiCache

externalRedis:
  enabled: true
  host: efe-acme-redis.xyz.cache.amazonaws.com
  authTokenSecretRef:
    name: efe-redis-external-auth
    key: token

observability:
  enabled: true
  prometheus:
    retention: 30d
    persistentVolumeSize: 200Gi
  loki:
    retention: 30d
    persistentVolumeSize: 500Gi
  tempo:
    samplingRate: 0.01

ingress:
  enabled: true
  annotations:
    cert-manager.io/cluster-issuer: letsencrypt-prod
```

### 11.2 Sample Terraform call (production AWS)

```hcl
# customer-starter/terraform/aws/main.tf
terraform {
  required_version = ">= 1.6"
  backend "s3" {
    bucket         = "efe-acme-tfstate"
    key            = "efe-stack/terraform.tfstate"
    region         = "eu-west-1"
    dynamodb_table = "efe-acme-tflock"
    encrypt        = true
  }
}

provider "aws" {
  region = "eu-west-1"
  default_tags {
    tags = {
      "efe.io/tenant" = "acme"
      "efe.io/env"    = "prod"
    }
  }
}

module "efe_stack_aws" {
  source  = "git::https://github.com/efe-ai/efe.git//terraform/modules/aws?ref=tf-aws/v1.0.0"

  region          = "eu-west-1"
  tenant          = "acme"
  environment     = "prod"
  data_residency  = "EU_ONLY"
  vpc_cidr        = "10.42.0.0/16"
  eks_version     = "1.30"

  rds_instance_class       = "db.r6i.xlarge"
  rds_allocated_storage    = 500
  rds_multi_az             = true
  rds_backup_retention_period = 14
  rds_deletion_protection  = true

  redis_node_type          = "cache.r7g.xlarge"
  redis_num_cache_clusters = 3

  enable_kms_byok = true

  eks_node_groups = {
    primary = {
      instance_types = ["m6i.xlarge"]
      desired_size   = 6
      min_size       = 6
      max_size       = 24
      capacity_type  = "ON_DEMAND"
      labels         = { role = "primary" }
    }
    spot = {
      instance_types = ["m6i.xlarge", "m6a.xlarge", "m5.xlarge"]
      desired_size   = 2
      min_size       = 0
      max_size       = 12
      capacity_type  = "SPOT"
      labels         = { role = "spot" }
      taints = [{
        key    = "efe.io/spot"
        value  = "true"
        effect = "NO_SCHEDULE"
      }]
    }
  }

  audit_bucket_retention_days = 2555

  tags = {
    "efe.io/cost-center" = "ACME-FRA-2026"
  }
}

output "kubeconfig_command" {
  value = module.efe_stack_aws.kubeconfig_command
}

output "rds_secret_arn" {
  value = module.efe_stack_aws.rds_secret_arn
}

output "byok_kms_key_arn" {
  value     = module.efe_stack_aws.byok_kms_key_arn
  sensitive = true
}
```

### 11.3 Helm install script complet (post-Terraform)

```bash
#!/usr/bin/env bash
# scripts/post-terraform-helm-install.sh
set -euo pipefail

REGION="$1"
TENANT="$2"
DOMAIN="$3"

# 1. kubeconfig
aws eks update-kubeconfig --region "$REGION" --name "efe-${TENANT}-eks"

# 2. Generate values from Terraform outputs
DB_SECRET_ARN=$(cd terraform/aws && terraform output -raw rds_secret_arn)
REDIS_SECRET_ARN=$(cd terraform/aws && terraform output -raw redis_secret_arn)
BYOK_KEY_ARN=$(cd terraform/aws && terraform output -raw byok_kms_key_arn)
IRSA_ARN=$(cd terraform/aws && terraform output -raw efe_server_irsa_role_arn)
AUDIT_BUCKET=$(cd terraform/aws && terraform output -raw audit_bucket_name)

# 3. Sync secrets from Secrets Manager → K8s Secrets
kubectl create namespace efe --dry-run=client -o yaml | kubectl apply -f -

DB_CONN=$(aws secretsmanager get-secret-value \
  --secret-id "$DB_SECRET_ARN" \
  --query SecretString --output text \
  | jq -r .connection_string)
kubectl create secret generic efe-postgres-external \
  --from-literal=connection_string="$DB_CONN" \
  --namespace efe --dry-run=client -o yaml | kubectl apply -f -

# (similar for redis + audit_hmac_seed)

# 4. Helm install
cat > /tmp/values-${TENANT}.yaml <<EOF
global:
  domain: ${DOMAIN}
  tenant: ${TENANT}
efe-server:
  byok:
    awsKmsKeyId: ${BYOK_KEY_ARN}
  serviceAccount:
    annotations:
      eks.amazonaws.com/role-arn: ${IRSA_ARN}
postgres:
  enabled: false
externalPostgres:
  enabled: true
  secretRef:
    name: efe-postgres-external
EOF

helm install efe-stack ./charts/efe-stack \
  --values /tmp/values-${TENANT}.yaml \
  --namespace efe \
  --create-namespace \
  --wait --timeout 10m

# 5. Verify
kubectl get pods -n efe
helm test efe-stack -n efe
echo "✅ Install OK for tenant ${TENANT}"
```

---

## §12. Conclusion

Sprint B livre l'**infrastructure runtime** qui rend les outils Sprint A (CLI + 16 mcp-server tools) effectivement utilisables : Helm chart `efe-stack` umbrella + 6 sub-charts, module Terraform AWS complet, et 6 runbooks ops formalisés (RB-01 à RB-06).

L'investissement (10 jours-homme) débloque :
- **Reproductibilité** : un même commit produit le même déploiement byte-for-byte
- **Self-service** : un SE peut provisionner un tenant POC en ≤ 30 min sans assistance ingé
- **Sécurité** : encryption at rest/transit, IRSA, NetworkPolicies, PSS restricted, Secrets Manager
- **Observabilité** : Prometheus + Grafana + Loki + Tempo intégrés
- **Ops maturity** : 6 runbooks formalisés couvrant 80 % des incidents probables

Le successeur naturel **Sprint C** (Portail Next.js + Terraform GCP/Azure + RB-07 à RB-12) finalise GLOSS et permet le release v1.0.0 GA EFE.

---

**Fin du document — Architecture v1.0 design freeze 2026-05-08**
