# Sprint D9 — Architecture : Docker image + Helm chart + Load test

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : suit D8 (Plan B activation + Real-Data validation infra).
> D9 est **le sprint qui débloque le Pilot 0.1 milestone** : transformer
> EFE de "binaire qui tourne sur le M3 Pro de l'auteur" en "service
> déployable sur n'importe quel cluster k8s ≥ 1.27".
> **Pré-requis** : avoir lu
> [`d8_hybrid_realdata_architecture.md`](d8_hybrid_realdata_architecture.md)
> et l'audit
> [`docs/audits/d8_decoder_comparison.md`](../audits/d8_decoder_comparison.md)
> qui établissent l'état post-D8 (1 726 Rust tests + 59 Python tests
> verts, 16 SLOs gated, accuracy 0.90 sur 3 catalogues, hybrid
> aggregate macro_f1 = 0.857).
>
> **Compagnon analytique** :
> [`d9_deep_analysis_supplement.md`](d9_deep_analysis_supplement.md)
> (1 119 lignes) — fondations mathématiques du load test, puissance
> statistique du soak, capacity planning Little's law, analyse
> symboles distroless, comparaison rigoureuse alternatives (5×5),
> modèle TCO, HAZOP 12 modes, reproductibilité bit-à-bit, chaos
> DR, frontière D9↔D11. Liste **9 amendements à appliquer à cette
> spec** (Annexe A du supplement) avant implémentation.

---

## Table des matières

1. [Contexte & mission](#1-contexte--mission)
2. [État post-D8 et écart vers Pilot 0.1](#2-état-post-d8-et-écart-vers-pilot-01)
3. [Architecture Docker image](#3-architecture-docker-image)
4. [Architecture Helm chart](#4-architecture-helm-chart)
5. [Architecture load test (ghz harness)](#5-architecture-load-test-ghz-harness)
6. [Architecture soak test (24 h)](#6-architecture-soak-test-24-h)
7. [Operational hardening](#7-operational-hardening)
8. [12 décisions de conception clés](#8-12-décisions-de-conception-clés)
9. [10 risques identifiés et mitigations](#9-10-risques-identifiés-et-mitigations)
10. [Plan jour-par-jour (4 j wallclock)](#10-plan-jour-par-jour-4-j-wallclock)
11. [SLOs ajoutés post-D9](#11-slos-ajoutés-post-d9)
12. [Anti-régression & realism check](#12-anti-régression--realism-check)
13. [Intégration CI : release workflow + load test job](#13-intégration-ci--release-workflow--load-test-job)
14. [Critères d'acceptation (DoD)](#14-critères-dacceptation-dod)
15. [Out of scope & roadmap follow-ups](#15-out-of-scope--roadmap-follow-ups)
16. [Annexes](#16-annexes)

---

## 1. Contexte & mission

### 1.1 État post-D8

Sprint D8 a livré l'infrastructure de productisation :
* `HybridClassifier` activé comme Plan B (aggregate macro_f1 0.815 →
  0.857, +0.078 sur `billing_escalation`).
* PII anonymization pipeline + CI gate (`pii_check.yml`).
* `AnchorAwareTemplateRegistry` opt-in (10 tests épinglés).
* 7 nouveaux SLOs `D8-*` (4 hybrid + 2 real-data + 1 latency).
* 6 scripts Python production-grade.
* 1 example Python end-to-end (`hybrid_classification.py`).

**Et pourtant, EFE ne tourne nulle part hors de la machine de
référence.** Aucun client ne peut installer le serveur ; aucun
partenaire ne peut tester sur ses données ; aucune mesure n'a été
faite sous charge. C'est le verrou critique avant le Pilot 0.1.

### 1.2 Mission D9 (énoncé chirurgical)

> 1. **Livrer une image Docker** multi-stage, multi-arch
>    (`linux/amd64` + `linux/arm64`), distroless-based, < 50 MB,
>    publiée sur GHCR à chaque release tag, signée par cosign avec
>    SBOM attesté.
>
> 2. **Livrer un Helm chart** dans `helm/efe/`, publié comme OCI
>    artifact sur GHCR, qui déploie un cluster `Mode 1` complet :
>    Deployment + Service + HPA + PodDisruptionBudget +
>    ServiceMonitor + PrometheusRule + ConfigMap + Secret +
>    NetworkPolicy + ServiceAccount + RBAC. Installation via
>    `helm install efe oci://ghcr.io/efe-ai/charts/efe --version 0.1.9`.
>
> 3. **Mesurer empiriquement** la performance sous charge avec un
>    harness `ghz` (gRPC load tester) reproductible : scénarios
>    smoke / sustained / stress / spike, p50/p95/p99 latency, error
>    rate, throughput, mémoire RSS, FD count.
>
> 4. **Conduire un soak test 24 h** à 5 RPS sur un cluster minikube
>    pour détecter tout memory leak / FD leak / accumulation de tâches
>    tokio non-collectées sur des durées de production typiques.
>
> 5. **Définir 4 SLOs `D9-LOAD-*`** qui chiffrent le contrat de
>    performance et les wirent au workflow CI bench (en mode
>    `continue-on-error: true` jusqu'à 3 main runs PASS).
>
> 6. **Documenter pour partenaire pilote** : un quickstart de
>    10 minutes (`docs/operations/quickstart.md`), un runbook de
>    déploiement (`docs/operations/deployment.md`), un troubleshooting
>    (`docs/operations/troubleshooting.md`).

### 1.3 Localisation des changements

| Fichier | Modification | Sub-step |
|---------|--------------|---------|
| `Dockerfile` | **Nouveau** — multi-stage builder + distroless runtime | D9.1 |
| `.dockerignore` | Existe — étendre pour exclure tests, fixtures, target | D9.1 |
| `helm/efe/Chart.yaml` | **Nouveau** — chart manifest | D9.2 |
| `helm/efe/values.yaml` | **Nouveau** — defaults + production overrides | D9.2 |
| `helm/efe/templates/deployment.yaml` | **Nouveau** | D9.2 |
| `helm/efe/templates/service.yaml` | **Nouveau** | D9.2 |
| `helm/efe/templates/configmap.yaml` | **Nouveau** | D9.2 |
| `helm/efe/templates/secret.yaml` | **Nouveau** (optionnel — généré ExternalSecret) | D9.2 |
| `helm/efe/templates/serviceaccount.yaml` | **Nouveau** + RBAC | D9.2 |
| `helm/efe/templates/hpa.yaml` | **Nouveau** | D9.2 |
| `helm/efe/templates/pdb.yaml` | **Nouveau** | D9.2 |
| `helm/efe/templates/networkpolicy.yaml` | **Nouveau** | D9.2 |
| `helm/efe/templates/servicemonitor.yaml` | **Nouveau** (Prometheus Operator) | D9.2 |
| `helm/efe/templates/prometheusrule.yaml` | **Nouveau** (alerting) | D9.2 |
| `helm/efe/templates/_helpers.tpl` | **Nouveau** — naming helpers | D9.2 |
| `helm/efe/templates/tests/test-grpc-health.yaml` | **Nouveau** — `helm test` | D9.2 |
| `helm/efe/README.md` | **Nouveau** — chart usage doc | D9.2 |
| `helm/efe/values.schema.json` | **Nouveau** — JSON Schema validation | D9.2 |
| `bench/load/ghz_smoke.json` | **Nouveau** — ghz config | D9.3 |
| `bench/load/ghz_sustained.json` | **Nouveau** | D9.3 |
| `bench/load/ghz_stress.json` | **Nouveau** | D9.3 |
| `bench/load/ghz_spike.json` | **Nouveau** | D9.3 |
| `bench/load/run_load_test.sh` | **Nouveau** — orchestrator | D9.3 |
| `bench/load/parse_ghz_report.py` | **Nouveau** — extract metrics | D9.3 |
| `bench/soak/run_soak_test.sh` | **Nouveau** — 24 h orchestrator | D9.4 |
| `bench/soak/leak_detector.py` | **Nouveau** — RSS regression analysis | D9.4 |
| `scripts/parse_criterion_report.py::SLO_DEFINITIONS` | **Modifié** — 4 nouveaux SLOs `D9-LOAD-*` | D9.5 |
| `.github/workflows/release.yml` | **Nouveau** — Docker build + Helm publish on tag | D9.6 |
| `.github/workflows/bench.yml` | **Modifié** — ajout `bench-load` job | D9.6 |
| `docs/operations/quickstart.md` | **Nouveau** — 10 min onboarding | D9.7 |
| `docs/operations/deployment.md` | **Nouveau** — production runbook | D9.7 |
| `docs/operations/troubleshooting.md` | **Nouveau** — error catalog | D9.7 |
| `docs/operations/load_test.md` | **Nouveau** — reproduce load tests | D9.7 |
| `BENCHMARKS.md` § 11 | **Modifié** — ajout section D9 | D9.7 |
| `bench_baselines/v0.1.9/baseline.json` | **Nouveau** | D9.7 |
| `docs/audits/d9_load_soak_report.md` | **Nouveau** | D9.7 |

---

## 2. État post-D8 et écart vers Pilot 0.1

### 2.1 Ce qui est déjà prêt côté serveur

* `efe-server` binaire release fonctionnel (Apple M3 + Linux x86_64).
* Configuration 100 % env-driven (`EFE_*` namespace, M12 §8).
* Graceful shutdown (`SIGTERM` → drain → force-abort).
* Auth JWT HS256 multi-key (rotation supportée).
* Rate limit token-bucket (per-tenant + anonymous).
* Idempotency cache LRU (10 000 entrées, TTL 10 s).
* OpenTelemetry : 12 métriques Prometheus + tracing JSON + audit log.
* gRPC health check (`grpc.health.v1.Health/Check`).

### 2.2 Ce qui manque pour un déploiement chez un partenaire

| Manque | Impact si absent | Sprint D9 sub-step |
|--------|------------------|--------------------|
| Image Docker | Aucun déploiement possible | D9.1 |
| Helm chart | Pas de IaC, install manuelle 30+ steps | D9.2 |
| Load test results | Pas de SLA possible (capacity unknown) | D9.3 |
| Soak test 24 h | Pas de garantie pas de leak en prod | D9.4 |
| SLOs `D9-LOAD-*` | Pas de gate régression perf en CI | D9.5 |
| Release workflow | Build manuel, pas de provenance | D9.6 |
| Quickstart doc | Onboarding partenaire en jours, pas en minutes | D9.7 |
| Deployment runbook | Incidents → blame Claude | D9.7 |

### 2.3 Critère de "Pilot 0.1 ready"

À l'issue de D9 (mais avant D9.6 release public) :

```
$ helm install efe-pilot oci://ghcr.io/efe-ai/charts/efe --version 0.1.9
$ kubectl wait --for=condition=available deployment/efe-pilot --timeout=120s
$ kubectl port-forward svc/efe-pilot 50051:50051 &
$ python3 examples/hybrid_classification.py
  Someone charged my card without my consent.    → fraud (conf=0.92)
  Where do I find a record of my past invoices?  → question (conf=0.88)
  L'application plante quand j'ouvre le menu     → bug (conf=0.91)
```

**Si ces 4 commandes succèdent en moins de 5 minutes sur un cluster
minikube fraîchement créé, D9 est un succès.**

---

## 3. Architecture Docker image

### 3.1 Stratégie multi-stage

```dockerfile
# syntax=docker/dockerfile:1.7

# =====================================================================
# Stage 1 — Rust builder. Pinned toolchain, --frozen build, no network.
# =====================================================================
FROM --platform=$BUILDPLATFORM rust:1.85.0-slim-bookworm AS builder

ARG TARGETPLATFORM
ARG CARGO_NET_GIT_FETCH_WITH_CLI=true

WORKDIR /build

# Copy ONLY Cargo.toml + Cargo.lock first to enable BuildKit cache.
# This makes the dependency build layer reusable across source changes.
COPY Cargo.toml Cargo.lock ./
COPY crates/efe-core/Cargo.toml crates/efe-core/Cargo.toml
COPY crates/efe-kernels/Cargo.toml crates/efe-kernels/Cargo.toml
COPY crates/efe-ffi/Cargo.toml crates/efe-ffi/Cargo.toml
COPY crates/efe-server/Cargo.toml crates/efe-server/Cargo.toml
COPY crates/efe-telemetry/Cargo.toml crates/efe-telemetry/Cargo.toml

# Create stub `src/lib.rs` files so cargo can resolve the workspace.
RUN mkdir -p crates/efe-core/src crates/efe-kernels/src \
             crates/efe-ffi/src crates/efe-server/src \
             crates/efe-telemetry/src && \
    echo 'fn main() {}' > crates/efe-server/src/main.rs && \
    for c in efe-core efe-kernels efe-ffi efe-telemetry; do \
      echo '' > crates/$c/src/lib.rs ; \
    done

# Pre-build dependencies. The `--mount=type=cache` is BuildKit-specific
# but degrades gracefully when not available.
RUN --mount=type=cache,target=/build/target \
    --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --release -p efe-server --bin efe-server \
                --frozen 2>/dev/null || \
    cargo build --release -p efe-server --bin efe-server

# Now copy the real source.
COPY . .

# Touch source so cargo notices.
RUN touch crates/efe-server/src/main.rs

# Final build with real sources.
RUN --mount=type=cache,target=/build/target \
    --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --release -p efe-server --bin efe-server --frozen && \
    cp /build/target/release/efe-server /efe-server && \
    strip /efe-server

# =====================================================================
# Stage 2 — Distroless runtime. < 25 MB final image.
# =====================================================================
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime

ARG VERSION=dev
ARG GIT_SHA=unknown
ARG BUILD_DATE=1970-01-01T00:00:00Z

LABEL org.opencontainers.image.title="efe-server" \
      org.opencontainers.image.description="EFE Mode 1 inference server" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${GIT_SHA}" \
      org.opencontainers.image.created="${BUILD_DATE}" \
      org.opencontainers.image.source="https://github.com/efe-ai/efe" \
      org.opencontainers.image.licenses="Apache-2.0"

# Distroless `nonroot` user has UID 65532, GID 65532.
USER 65532:65532

# The binary lives at /usr/local/bin/efe-server.
COPY --from=builder --chown=65532:65532 /efe-server /usr/local/bin/efe-server

# Default port (overridable via EFE_LISTEN_ADDR).
EXPOSE 50051

# Default metrics port (overridable via EFE_METRICS_ADDR).
EXPOSE 9090

ENTRYPOINT ["/usr/local/bin/efe-server"]
```

### 3.2 Pourquoi distroless

| Choix | Pro | Con |
|-------|-----|-----|
| `gcr.io/distroless/cc-debian12:nonroot` | Pas de shell → pas d'exec arbitraire ; pre-installed glibc + ssl libs ; CVE surface minimale ; ~20 MB | Pas de shell → pas de debug post-mortem en pod ; nécessite ephemeral container pour debug |
| Alternative `alpine:3.19` | Shell pour debug ; pkg manager | musl libc → recompile sentence-transformers serait pénible plus tard ; +5 MB |
| Alternative `scratch` | < 5 MB | Zéro libs → pas d'OpenSSL pour gRPC TLS |

**Choix : distroless.** Le shell-less est une bonne hygiène de sécurité pour le MVP. Pour debug, on utilise `kubectl debug -it pod/X --image=busybox` (ephemeral container).

### 3.3 Multi-arch build

* `linux/amd64` — x86_64 standard. Plus de CPUs cloud que arm64 en 2026.
* `linux/arm64` — Apple Silicon dev (M3 Pro reference) + Graviton AWS (cost-optimized).

Build matrix dans `release.yml` :

```yaml
strategy:
  matrix:
    platform: [linux/amd64, linux/arm64]
```

Avec `docker/build-push-action@v5` :

```yaml
- uses: docker/build-push-action@v5
  with:
    platforms: linux/amd64,linux/arm64
    push: true
    tags: |
      ghcr.io/efe-ai/efe-server:${{ github.ref_name }}
      ghcr.io/efe-ai/efe-server:latest
    build-args: |
      VERSION=${{ github.ref_name }}
      GIT_SHA=${{ github.sha }}
      BUILD_DATE=${{ steps.date.outputs.iso }}
    cache-from: type=gha,scope=${{ matrix.platform }}
    cache-to: type=gha,mode=max,scope=${{ matrix.platform }}
```

### 3.4 Image-size budget

| Composant | Taille estimée |
|-----------|---------------:|
| distroless cc base | ~20 MB |
| `efe-server` stripped binary | ~6-8 MB |
| TLS roots (incluse dans distroless) | ~1 MB |
| **Total** | **~25-30 MB** |

**Budget D9.1** : `< 50 MB` final image (margin pour futures additions de catalogues compilés bundlés).

### 3.5 Reproducibility & SBOM

* `cargo build --frozen` : refuse de builder si `Cargo.lock` est obsolete.
* `BUILD_DATE` injecté via build-arg pour traçabilité.
* `SOURCE_DATE_EPOCH` pour timestamps déterministes (où le base image le permet).
* SBOM généré via `cyclonedx-cargo` après le build, attaché au manifeste OCI :

```yaml
- name: Generate SBOM
  run: |
    cargo install cyclonedx-cargo --locked
    cyclonedx-cargo --format json --output sbom.cdx.json
- name: Attach SBOM to image
  uses: docker/[email protected]
  with:
    image: ghcr.io/efe-ai/efe-server:${{ github.ref_name }}
    sbom: sbom.cdx.json
```

### 3.6 Image signing (cosign keyless)

```yaml
- uses: sigstore/cosign-installer@v3
- name: Sign image (keyless via OIDC)
  run: |
    cosign sign --yes \
      ghcr.io/efe-ai/efe-server@${{ steps.build.outputs.digest }}
  env:
    COSIGN_EXPERIMENTAL: "true"
```

Verification post-pull :

```bash
cosign verify ghcr.io/efe-ai/efe-server:0.1.9 \
  --certificate-identity-regexp="https://github.com/efe-ai/efe/.github/workflows/release.yml@.*" \
  --certificate-oidc-issuer="https://token.actions.githubusercontent.com"
```

### 3.7 CVE scanning

Workflow `release.yml` étape post-build :

```yaml
- uses: aquasecurity/trivy-action@master
  with:
    image-ref: ghcr.io/efe-ai/efe-server:${{ github.ref_name }}
    severity: 'CRITICAL,HIGH'
    exit-code: '1'
    format: 'sarif'
    output: 'trivy-results.sarif'
- uses: github/codeql-action/upload-sarif@v3
  with:
    sarif_file: trivy-results.sarif
```

**Gate** : un seul CVE CRITICAL ou HIGH non-fixable bloque le release. Mitigation : whitelist documentée dans `.trivyignore` avec justification.

---

## 4. Architecture Helm chart

### 4.1 Structure du chart

```
helm/efe/
├── Chart.yaml
├── README.md
├── values.yaml
├── values.schema.json
└── templates/
    ├── _helpers.tpl
    ├── configmap.yaml
    ├── deployment.yaml
    ├── hpa.yaml
    ├── networkpolicy.yaml
    ├── pdb.yaml
    ├── prometheusrule.yaml
    ├── secret.yaml
    ├── service.yaml
    ├── serviceaccount.yaml
    ├── servicemonitor.yaml
    └── tests/
        └── test-grpc-health.yaml
```

### 4.2 `Chart.yaml`

```yaml
apiVersion: v2
name: efe
description: EFE Mode 1 inference server — Helm chart
type: application
version: 0.1.9
appVersion: "0.1.9"
icon: https://avatars.githubusercontent.com/efe-ai
home: https://github.com/efe-ai/efe
sources:
  - https://github.com/efe-ai/efe
maintainers:
  - name: EFE team
    email: maintainers@efe.ai
keywords:
  - inference
  - classification
  - mode-1
  - grpc
  - swarm
kubeVersion: ">=1.27.0"
annotations:
  artifacthub.io/license: Apache-2.0
  artifacthub.io/changes: |
    - Initial release of efe Helm chart (Sprint D9).
    - Multi-arch support (linux/amd64 + linux/arm64).
    - HPA on CPU + custom metric (efe_swarm_active_count).
    - PrometheusRule with 7 alerts.
```

### 4.3 `values.yaml` (extrait clé, ~250 LOC complet)

```yaml
# ─────────────────────────────────────────────────────────────────
# Image
# ─────────────────────────────────────────────────────────────────
image:
  repository: ghcr.io/efe-ai/efe-server
  tag: ""  # defaults to .Chart.AppVersion if empty
  pullPolicy: IfNotPresent
  pullSecrets: []

# ─────────────────────────────────────────────────────────────────
# Replica + scaling
# ─────────────────────────────────────────────────────────────────
replicaCount: 2

autoscaling:
  enabled: true
  minReplicas: 2
  maxReplicas: 10
  # CPU target — based on D9.3 load test (10 RPS at 60 % CPU on
  # 2 vCPU pod). Production tenants may override.
  targetCPUUtilizationPercentage: 70
  # Custom metric (requires Prometheus Adapter or Keda) :
  # scale up when efe_swarm_active_count exceeds 50 % of max_arenas.
  customMetric:
    enabled: false  # requires k8s metric server addon
    name: efe_swarm_active_count
    targetAverageValue: "4"

# ─────────────────────────────────────────────────────────────────
# Resources
# ─────────────────────────────────────────────────────────────────
resources:
  requests:
    cpu: "500m"
    memory: "1Gi"
  limits:
    cpu: "2"
    memory: "4Gi"

# ─────────────────────────────────────────────────────────────────
# Mode 1 / EFE configuration (env-var-driven)
# ─────────────────────────────────────────────────────────────────
config:
  # Server
  listenAddr: "0.0.0.0:50051"
  metricsAddr: "0.0.0.0:9090"
  requestTimeoutMs: 30000
  maxConcurrentStreams: 1024
  enableAdmin: true

  # Auth
  auth:
    enabled: false
    jwtAudience: "efe.v1"
    jwtIssuer: ""
    # JWT secrets file — mounted from a k8s Secret (see secret.yaml)
    secretsFilePath: "/etc/efe/secrets/hs256"

  # Rate limit
  rateLimit:
    enabled: true
    defaultCapacity: 200
    defaultRefillRate: 100.0
    anonymousCapacity: 50
    anonymousRefillRate: 10.0

  # Idempotency
  idempotency:
    enabled: true
    cacheCapacity: 10000
    ttlMs: 10000

  # Shutdown
  shutdown:
    drainMs: 30000
    forceAbortMs: 45000

  # Mode 1
  mode1:
    dimLatent: 384
    preallocArenas: 4
    maxArenas: 8
    nStreams: 4

  # Embedder model — must match the catalogs the clients will use
  embedderModel: "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2"

# ─────────────────────────────────────────────────────────────────
# Secrets — JWT HS256 keys
# ─────────────────────────────────────────────────────────────────
secrets:
  # Inline value (base64 keys, one per line). Use only in dev.
  inline: ""
  # Reference to an existing k8s Secret (preferred for prod).
  existingSecret: ""
  # ExternalSecret CR — leaves secret management to ESO.
  externalSecretRef: ""

# ─────────────────────────────────────────────────────────────────
# Service
# ─────────────────────────────────────────────────────────────────
service:
  type: ClusterIP
  port: 50051
  metricsPort: 9090
  annotations: {}

# ─────────────────────────────────────────────────────────────────
# Pod-level controls
# ─────────────────────────────────────────────────────────────────
serviceAccount:
  create: true
  name: ""
  annotations: {}

podAnnotations: {}
podLabels: {}

podSecurityContext:
  runAsNonRoot: true
  runAsUser: 65532
  runAsGroup: 65532
  fsGroup: 65532
  seccompProfile:
    type: RuntimeDefault

securityContext:
  allowPrivilegeEscalation: false
  readOnlyRootFilesystem: true
  capabilities:
    drop:
      - ALL

nodeSelector: {}
tolerations: []
affinity:
  podAntiAffinity:
    preferredDuringSchedulingIgnoredDuringExecution:
      - weight: 100
        podAffinityTerm:
          labelSelector:
            matchExpressions:
              - key: app.kubernetes.io/name
                operator: In
                values: ["efe"]
          topologyKey: kubernetes.io/hostname

# ─────────────────────────────────────────────────────────────────
# Liveness / readiness / startup probes
# ─────────────────────────────────────────────────────────────────
probes:
  liveness:
    grpc:
      port: 50051
      service: ""
    initialDelaySeconds: 10
    periodSeconds: 30
    timeoutSeconds: 5
    failureThreshold: 3
  readiness:
    grpc:
      port: 50051
      service: ""
    initialDelaySeconds: 5
    periodSeconds: 10
    timeoutSeconds: 3
    failureThreshold: 3
  startup:
    grpc:
      port: 50051
      service: ""
    initialDelaySeconds: 0
    periodSeconds: 5
    timeoutSeconds: 5
    failureThreshold: 30

# ─────────────────────────────────────────────────────────────────
# PodDisruptionBudget
# ─────────────────────────────────────────────────────────────────
podDisruptionBudget:
  enabled: true
  minAvailable: 1
  # Alternative : maxUnavailable: 1

# ─────────────────────────────────────────────────────────────────
# NetworkPolicy
# ─────────────────────────────────────────────────────────────────
networkPolicy:
  enabled: false  # opt-in — many clusters don't have a CNI plugin
  ingress:
    # Only allow ingress from labeled namespaces
    fromNamespaces:
      - name: app
  egress:
    # Default-allow DNS + Prometheus scrape
    allowDns: true

# ─────────────────────────────────────────────────────────────────
# Prometheus monitoring
# ─────────────────────────────────────────────────────────────────
serviceMonitor:
  enabled: false  # requires Prometheus Operator
  interval: 30s
  scrapeTimeout: 10s
  labels: {}

prometheusRule:
  enabled: false  # requires Prometheus Operator
  labels: {}

# ─────────────────────────────────────────────────────────────────
# Lifecycle hooks
# ─────────────────────────────────────────────────────────────────
lifecycle:
  preStop:
    # Allow N seconds for in-flight RPCs to drain.
    sleepSeconds: 25  # < terminationGracePeriodSeconds (30)

terminationGracePeriodSeconds: 60
```

### 4.4 `templates/deployment.yaml`

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {{ include "efe.fullname" . }}
  namespace: {{ .Release.Namespace }}
  labels:
    {{- include "efe.labels" . | nindent 4 }}
spec:
  {{- if not .Values.autoscaling.enabled }}
  replicas: {{ .Values.replicaCount }}
  {{- end }}
  strategy:
    type: RollingUpdate
    rollingUpdate:
      maxUnavailable: 0
      maxSurge: 1
  selector:
    matchLabels:
      {{- include "efe.selectorLabels" . | nindent 6 }}
  template:
    metadata:
      annotations:
        # Prometheus scrape annotations (also picked up if no
        # ServiceMonitor)
        prometheus.io/scrape: "true"
        prometheus.io/port: "{{ .Values.service.metricsPort }}"
        prometheus.io/path: "/metrics"
        # Force pod restart when ConfigMap changes
        checksum/config: {{ include (print $.Template.BasePath "/configmap.yaml") . | sha256sum }}
        {{- with .Values.podAnnotations }}
        {{- toYaml . | nindent 8 }}
        {{- end }}
      labels:
        {{- include "efe.selectorLabels" . | nindent 8 }}
        {{- with .Values.podLabels }}
        {{- toYaml . | nindent 8 }}
        {{- end }}
    spec:
      serviceAccountName: {{ include "efe.serviceAccountName" . }}
      terminationGracePeriodSeconds: {{ .Values.terminationGracePeriodSeconds }}
      securityContext:
        {{- toYaml .Values.podSecurityContext | nindent 8 }}
      containers:
      - name: efe-server
        image: "{{ .Values.image.repository }}:{{ .Values.image.tag | default .Chart.AppVersion }}"
        imagePullPolicy: {{ .Values.image.pullPolicy }}
        securityContext:
          {{- toYaml .Values.securityContext | nindent 10 }}
        ports:
        - name: grpc
          containerPort: {{ regexReplaceAll ".*:" .Values.config.listenAddr "" | int }}
          protocol: TCP
        - name: metrics
          containerPort: {{ regexReplaceAll ".*:" .Values.config.metricsAddr "" | int }}
          protocol: TCP
        envFrom:
        - configMapRef:
            name: {{ include "efe.fullname" . }}-config
        {{- if or .Values.secrets.inline .Values.secrets.existingSecret }}
        volumeMounts:
        - name: jwt-secrets
          mountPath: /etc/efe/secrets
          readOnly: true
        {{- end }}
        resources:
          {{- toYaml .Values.resources | nindent 10 }}
        startupProbe:
          grpc:
            port: {{ regexReplaceAll ".*:" .Values.config.listenAddr "" | int }}
          {{- toYaml .Values.probes.startup | nindent 10 }}
        livenessProbe:
          grpc:
            port: {{ regexReplaceAll ".*:" .Values.config.listenAddr "" | int }}
          {{- toYaml (omit .Values.probes.liveness "grpc") | nindent 10 }}
        readinessProbe:
          grpc:
            port: {{ regexReplaceAll ".*:" .Values.config.listenAddr "" | int }}
          {{- toYaml (omit .Values.probes.readiness "grpc") | nindent 10 }}
        lifecycle:
          preStop:
            exec:
              command:
              - sleep
              - "{{ .Values.lifecycle.preStop.sleepSeconds }}"
      {{- if or .Values.secrets.inline .Values.secrets.existingSecret }}
      volumes:
      - name: jwt-secrets
        secret:
          secretName: {{ if .Values.secrets.existingSecret }}{{ .Values.secrets.existingSecret }}{{ else }}{{ include "efe.fullname" . }}-jwt{{ end }}
          defaultMode: 0400
      {{- end }}
      {{- with .Values.nodeSelector }}
      nodeSelector:
        {{- toYaml . | nindent 8 }}
      {{- end }}
      {{- with .Values.affinity }}
      affinity:
        {{- toYaml . | nindent 8 }}
      {{- end }}
      {{- with .Values.tolerations }}
      tolerations:
        {{- toYaml . | nindent 8 }}
      {{- end }}
```

### 4.5 `templates/configmap.yaml`

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: {{ include "efe.fullname" . }}-config
  namespace: {{ .Release.Namespace }}
  labels:
    {{- include "efe.labels" . | nindent 4 }}
data:
  # Server
  EFE_LISTEN_ADDR: {{ .Values.config.listenAddr | quote }}
  EFE_METRICS_ADDR: {{ .Values.config.metricsAddr | quote }}
  EFE_REQUEST_TIMEOUT_MS: {{ .Values.config.requestTimeoutMs | quote }}
  EFE_MAX_CONCURRENT_STREAMS: {{ .Values.config.maxConcurrentStreams | quote }}
  EFE_ENABLE_ADMIN: {{ .Values.config.enableAdmin | quote }}
  # Auth
  EFE_AUTH_ENABLED: {{ .Values.config.auth.enabled | quote }}
  EFE_JWT_AUDIENCE: {{ .Values.config.auth.jwtAudience | quote }}
  {{- if .Values.config.auth.jwtIssuer }}
  EFE_JWT_ISSUER: {{ .Values.config.auth.jwtIssuer | quote }}
  {{- end }}
  EFE_JWT_SECRETS_FILE: {{ .Values.config.auth.secretsFilePath | quote }}
  # Rate limit
  EFE_RATELIMIT_ENABLED: {{ .Values.config.rateLimit.enabled | quote }}
  EFE_RATELIMIT_CAPACITY: {{ .Values.config.rateLimit.defaultCapacity | quote }}
  EFE_RATELIMIT_REFILL: {{ .Values.config.rateLimit.defaultRefillRate | quote }}
  EFE_RATELIMIT_ANON_CAP: {{ .Values.config.rateLimit.anonymousCapacity | quote }}
  EFE_RATELIMIT_ANON_REFILL: {{ .Values.config.rateLimit.anonymousRefillRate | quote }}
  # Idempotency
  EFE_IDEMPOTENCY_ENABLED: {{ .Values.config.idempotency.enabled | quote }}
  EFE_IDEMPOTENCY_CAPACITY: {{ .Values.config.idempotency.cacheCapacity | quote }}
  EFE_IDEMPOTENCY_TTL_MS: {{ .Values.config.idempotency.ttlMs | quote }}
  # Shutdown
  EFE_SHUTDOWN_DRAIN_MS: {{ .Values.config.shutdown.drainMs | quote }}
  EFE_SHUTDOWN_FORCE_ABORT_MS: {{ .Values.config.shutdown.forceAbortMs | quote }}
  # Mode 1
  EFE_SWARM_DIM_LATENT: {{ .Values.config.mode1.dimLatent | quote }}
  EFE_PREALLOC_ARENAS: {{ .Values.config.mode1.preallocArenas | quote }}
  EFE_MAX_ARENAS: {{ .Values.config.mode1.maxArenas | quote }}
  EFE_N_STREAMS: {{ .Values.config.mode1.nStreams | quote }}
  # Embedder fingerprint (used by clients to validate catalog match)
  EFE_EMBEDDER_MODEL: {{ .Values.config.embedderModel | quote }}
```

### 4.6 `templates/hpa.yaml`

```yaml
{{- if .Values.autoscaling.enabled }}
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: {{ include "efe.fullname" . }}
  namespace: {{ .Release.Namespace }}
  labels:
    {{- include "efe.labels" . | nindent 4 }}
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: {{ include "efe.fullname" . }}
  minReplicas: {{ .Values.autoscaling.minReplicas }}
  maxReplicas: {{ .Values.autoscaling.maxReplicas }}
  metrics:
  - type: Resource
    resource:
      name: cpu
      target:
        type: Utilization
        averageUtilization: {{ .Values.autoscaling.targetCPUUtilizationPercentage }}
  {{- if .Values.autoscaling.customMetric.enabled }}
  - type: Pods
    pods:
      metric:
        name: {{ .Values.autoscaling.customMetric.name }}
      target:
        type: AverageValue
        averageValue: {{ .Values.autoscaling.customMetric.targetAverageValue }}
  {{- end }}
  behavior:
    scaleDown:
      stabilizationWindowSeconds: 300
      policies:
      - type: Percent
        value: 50
        periodSeconds: 60
    scaleUp:
      stabilizationWindowSeconds: 30
      policies:
      - type: Pods
        value: 2
        periodSeconds: 30
{{- end }}
```

### 4.7 `templates/prometheusrule.yaml`

```yaml
{{- if .Values.prometheusRule.enabled }}
apiVersion: monitoring.coreos.com/v1
kind: PrometheusRule
metadata:
  name: {{ include "efe.fullname" . }}
  namespace: {{ .Release.Namespace }}
  labels:
    {{- include "efe.labels" . | nindent 4 }}
    {{- with .Values.prometheusRule.labels }}
    {{- toYaml . | nindent 4 }}
    {{- end }}
spec:
  groups:
  - name: efe.rules
    interval: 30s
    rules:
    - alert: EfeHighErrorRate
      expr: |
        sum by (tenant_id) (rate(efe_errors_total[5m]))
        /
        sum by (tenant_id) (rate(efe_swarm_rounds_total[5m]))
        > 0.05
      for: 5m
      labels:
        severity: warning
        sprint: D9
      annotations:
        summary: "EFE error rate > 5 % for tenant {{`{{ $labels.tenant_id }}`}}"
        description: |
          Error rate exceeds 5 % of total requests. Check
          /metrics endpoint for the failing classes.

    - alert: EfeP99LatencyHigh
      expr: |
        histogram_quantile(0.99,
          rate(efe_swarm_duration_seconds_bucket[5m])) > 5.0
      for: 10m
      labels:
        severity: warning
        sprint: D9
      annotations:
        summary: "EFE p99 latency > 5 s"
        description: |
          p99 latency exceeds the SLO-D8-LAT-2 budget (5 s). Likely
          causes : embedder slow, fusion divergence, memory pool
          contention.

    - alert: EfeMemoryRSSGrowth
      expr: |
        deriv(container_memory_rss{
          pod=~"{{ include "efe.fullname" . }}.*"
        }[1h]) > 50000000
      for: 1h
      labels:
        severity: warning
        sprint: D9
      annotations:
        summary: "EFE pod RSS growing > 50 MB/h"
        description: |
          The pod's RSS is increasing — possible memory leak. Run
          `kubectl debug` and check /metrics for arena pool stats.

    - alert: EfePodCrashLooping
      expr: |
        rate(kube_pod_container_status_restarts_total{
          pod=~"{{ include "efe.fullname" . }}.*"
        }[15m]) > 0
      for: 15m
      labels:
        severity: critical
        sprint: D9
      annotations:
        summary: "EFE pod crash-looping"
        description: |
          Pod restarted in last 15 min. Check container logs.

    - alert: EfeMetricsDown
      expr: |
        up{job=~".*efe.*"} == 0
      for: 5m
      labels:
        severity: critical
        sprint: D9
      annotations:
        summary: "EFE metrics endpoint unreachable"
        description: |
          Prometheus cannot scrape the EFE pod's /metrics endpoint.

    - alert: EfeAuthFailureBurst
      expr: |
        rate(efe_auth_failures_total[5m]) > 1.0
      for: 5m
      labels:
        severity: warning
        sprint: D9
      annotations:
        summary: "EFE auth failures > 1/s"
        description: |
          Possible attack or misconfigured client JWT. Check audit log.

    - alert: EfeRateLimitExhaustion
      expr: |
        rate(efe_rate_limit_drops_total[5m]) > 5.0
      for: 10m
      labels:
        severity: warning
        sprint: D9
      annotations:
        summary: "EFE rate-limit drops > 5/s"
        description: |
          Many requests being rate-limited. Either legit traffic
          spike (raise capacity) or attack (raise alert).
{{- end }}
```

### 4.8 `templates/networkpolicy.yaml`

```yaml
{{- if .Values.networkPolicy.enabled }}
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: {{ include "efe.fullname" . }}
  namespace: {{ .Release.Namespace }}
  labels:
    {{- include "efe.labels" . | nindent 4 }}
spec:
  podSelector:
    matchLabels:
      {{- include "efe.selectorLabels" . | nindent 6 }}
  policyTypes:
  - Ingress
  - Egress
  ingress:
  {{- range .Values.networkPolicy.ingress.fromNamespaces }}
  - from:
    - namespaceSelector:
        matchLabels:
          kubernetes.io/metadata.name: {{ .name }}
    ports:
    - port: 50051
      protocol: TCP
  {{- end }}
  - from:
    - namespaceSelector:
        matchLabels:
          name: monitoring
    ports:
    - port: 9090
      protocol: TCP
  egress:
  {{- if .Values.networkPolicy.egress.allowDns }}
  - to:
    - namespaceSelector: {}
      podSelector:
        matchLabels:
          k8s-app: kube-dns
    ports:
    - port: 53
      protocol: UDP
    - port: 53
      protocol: TCP
  {{- end }}
{{- end }}
```

### 4.9 `helm test` smoke

`helm/efe/templates/tests/test-grpc-health.yaml` :

```yaml
apiVersion: v1
kind: Pod
metadata:
  name: "{{ include "efe.fullname" . }}-test-grpc-health"
  labels:
    {{- include "efe.labels" . | nindent 4 }}
  annotations:
    "helm.sh/hook": test
    "helm.sh/hook-delete-policy": before-hook-creation,hook-succeeded
spec:
  restartPolicy: Never
  containers:
  - name: grpc-health-check
    image: ghcr.io/grpc-ecosystem/grpc-health-probe:v0.4.27
    args:
    - "-addr={{ include "efe.fullname" . }}:{{ .Values.service.port }}"
    - "-connect-timeout=5s"
    - "-rpc-timeout=5s"
```

---

## 5. Architecture load test (ghz harness)

### 5.1 Pourquoi ghz

| Outil | Pour | Contre |
|-------|------|--------|
| **ghz** | gRPC-native ; supporte unary/streaming/bidi ; output JSON ; concurrence configurable ; binaire single-file | Pas de scénarios complexes (pas de scripting Lua) |
| ghz alternatives | ghz-pro paid, k6 | k6 a un grpc plugin maintenant mais le tooling est plus lourd |

**Choix : ghz.** Léger, JSON-out parfait pour CI parsing.

### 5.2 Scénarios de test

| Scénario | Concurrence | Durée | RPS cible | But |
|----------|------------:|-------|----------:|-----|
| `smoke` | 1 | 30 s | ~0.4 (1/2.5s) | Sanity check, exit 0 |
| `sustained` | 4 | 5 min | ~10 (4 × 0.4 × 6) | RPS soutenable mesuré |
| `stress` | 16 | 2 min | ~30 | Trouver le point de saturation |
| `spike` | 1→32→1 | 1 min | bursty | Test de récupération |

### 5.3 Configuration ghz

`bench/load/ghz_smoke.json` :

```json
{
  "proto": "protos/efe.proto",
  "import-paths": ["protos"],
  "call": "efe.v1.EfeService/Execute",
  "host": "127.0.0.1:50051",
  "insecure": true,
  "concurrency": 1,
  "duration": "30s",
  "data-file": "bench/load/fixtures/sample_request.json",
  "metadata": {"x-tenant-id": "loadtest"},
  "format": "json",
  "output": "target/load/smoke.json",
  "skip-first": 5,
  "load-schedule": "const"
}
```

Le payload `sample_request.json` est un `SwarmSpec` synthétique (dim_latent=384, ~30 anchors, ~200 templates). Réaliste mais déterministe.

### 5.4 Script orchestrateur

`bench/load/run_load_test.sh` (~150 LOC) :

```bash
#!/usr/bin/env bash
#
# Sprint D9 — Run load tests against a running efe-server.
#
# Usage :
#   bench/load/run_load_test.sh [smoke|sustained|stress|spike|all] \
#     [--endpoint 127.0.0.1:50051] [--output-dir target/load/]
#
# Requires : ghz binary on PATH (https://ghz.sh/).
#
# Exit code : 0 if all scenarios PASS their SLOs ; 1 otherwise.

set -euo pipefail

SCENARIO="${1:-all}"
ENDPOINT="${ENDPOINT:-127.0.0.1:50051}"
OUTPUT_DIR="${OUTPUT_DIR:-target/load}"
mkdir -p "$OUTPUT_DIR"

run_scenario() {
  local name=$1
  local config="bench/load/ghz_${name}.json"
  local out="$OUTPUT_DIR/${name}.json"
  echo "─── Running ghz scenario : $name ───"
  ghz --config "$config" \
      --host "$ENDPOINT" \
      --output "$out"
  echo "Output : $out"
}

case "$SCENARIO" in
  smoke|sustained|stress|spike)
    run_scenario "$SCENARIO"
    ;;
  all)
    for s in smoke sustained stress spike; do
      run_scenario "$s"
    done
    ;;
  *)
    echo "Unknown scenario : $SCENARIO" >&2
    exit 2
    ;;
esac

# Parse + emit bench-format for SLO compliance.
python3 bench/load/parse_ghz_report.py \
  --input-dir "$OUTPUT_DIR" \
  --output "$OUTPUT_DIR/load_test_bench.json" \
  --output-md "$OUTPUT_DIR/load_test_summary.md"

echo "✅ Load test orchestration done."
```

### 5.5 Parser des rapports ghz

`bench/load/parse_ghz_report.py` (~250 LOC) :

```python
"""Sprint D9 — Parse ghz JSON reports and emit bench-format JSON
consumed by `parse_criterion_report.py`.

Each scenario produces 4 metrics :
  - `load_<scenario>::p99_latency_ns`
  - `load_<scenario>::p50_latency_ns`
  - `load_<scenario>::error_rate`
  - `load_<scenario>::throughput_rps`

Markdown summary table + exit code 1 if any SLO fails.
"""
```

Scenario → SLO mapping :

| Scénario | Métrique | SLO ID | Cible |
|----------|----------|--------|------:|
| sustained | p99 latency | `SLO-D9-LOAD-1` | ≤ 5 s |
| sustained | error_rate | `SLO-D9-LOAD-2` | ≤ 1 % |
| stress | throughput | `SLO-D9-LOAD-3` | ≥ 5 RPS |
| spike | recovery_time | `SLO-D9-LOAD-4` | ≤ 30 s post-burst back to baseline |

---

## 6. Architecture soak test (24 h)

### 6.1 Mission

Sur 24 h continues à 5 RPS (soit ~432 000 RPCs total), vérifier que :

* La RSS du pod ne croît pas > 5 % linéaire (pas de leak).
* Le nombre de FDs ouverts reste borné (< 200).
* Le nombre de tokio tasks vivantes reste borné (< n_streams × concurrency).
* L'accuracy mesurée à T+24h est égale à celle T+0h (pas de model drift / memory corruption).

### 6.2 Architecture

Le soak tourne sur **minikube** (single-node k8s) avec un job dédié :

```yaml
# bench/soak/k8s_soak_job.yaml
apiVersion: batch/v1
kind: Job
metadata:
  name: efe-soak-${TIMESTAMP}
spec:
  ttlSecondsAfterFinished: 86400   # auto-delete after 24h
  backoffLimit: 0
  template:
    spec:
      restartPolicy: Never
      containers:
      - name: load-runner
        image: ghcr.io/bojand/ghz:latest
        args:
          - "--proto=/proto/efe.proto"
          - "--call=efe.v1.EfeService/Execute"
          - "--insecure"
          - "--host=efe:50051"
          - "--total=432000"
          - "--rps=5"
          - "--data-file=/proto/sample_request.json"
          - "--output=/output/soak.json"
        volumeMounts:
        - name: proto
          mountPath: /proto
        - name: output
          mountPath: /output
      volumes:
      - name: proto
        configMap:
          name: efe-soak-proto
      - name: output
        emptyDir: {}
```

### 6.3 Memory leak detector

`bench/soak/leak_detector.py` :

```python
"""Sprint D9 — Detect linear RSS growth in soak data via OLS regression.

Reads Prometheus metrics dumped to `target/soak/metrics_dump.json`
(via promtool query_range over the soak window). Fits a linear
regression `rss_bytes = a * t_seconds + b` ; if the slope `a` exceeds
the threshold (5 % per 24 h × initial_rss), flag a leak.

Output : JSON report + exit code (0 = no leak, 1 = leak detected).
"""
```

Calcul du seuil :

```
initial_rss = 1.5 GB (typical post-warmup)
budget = 5 % per 24 h
max_slope = (0.05 * 1.5e9) / 86400 = ~870 bytes/s
```

Tout slope > 870 B/s sur la fenêtre 24 h fail.

### 6.4 Promtool dump pour analyse

```bash
promtool query range \
  --start='2026-04-27T08:00:00Z' \
  --end='2026-04-28T08:00:00Z' \
  --step=1m \
  http://prometheus:9090 \
  'container_memory_rss{pod=~"efe-pilot-.*"}' > target/soak/metrics_dump.json
```

---

## 7. Operational hardening

### 7.1 Graceful shutdown — wiring

Le serveur EFE expose déjà :
* `Shutdown::trigger()` côté code (M12 §15).
* `EFE_SHUTDOWN_DRAIN_MS` (default 30 s) pour le drain.
* `EFE_SHUTDOWN_FORCE_ABORT_MS` pour le hard-kill.

Helm wiring :
* `terminationGracePeriodSeconds: 60` (covers drain + force_abort).
* `lifecycle.preStop.exec.command: ["sleep", "25"]` — laisse 25 s aux RPCs in-flight pour terminer AVANT le SIGTERM (durant ce sleep, le service endpoint est déjà retiré du load balancer parce que k8s a déjà commencé le terminer process).

### 7.2 Probe semantics

| Probe | But | Échec → action |
|-------|-----|---------------|
| `startupProbe` | Le pod a-t-il fini de démarrer ? (model load, pool init) | k8s ne tente PAS liveness/readiness avant succès |
| `readinessProbe` | Le pod peut-il servir du trafic ? | retiré du Service endpoint |
| `livenessProbe` | Le pod est-il vivant (deadlock détection) ? | redémarrage du container |

Les 3 probes utilisent `grpc.health.v1.Health/Check` exposed par `tonic-health` (déjà câblé en M10).

### 7.3 Resource sizing post-load-test

Les valeurs `requests`/`limits` du `values.yaml` sont des **points de départ** ; le load test (D9.3) raffinera :

| Mesure | Initial | Post-load-test (TBD) |
|--------|--------:|---------------------:|
| CPU request | 500m | mesuré au p50 sustained |
| CPU limit | 2 | mesuré au p99 stress |
| Memory request | 1 Gi | mesuré au steady-state |
| Memory limit | 4 Gi | mesuré au peak + 50 % |

---

## 8. 12 décisions de conception clés

| # | Décision | Alternative rejetée | Rationale |
|---|----------|---------------------|-----------|
| D1 | Image distroless | Alpine, scratch, full Debian | Sécurité (no shell) ; OK pour MVP, debug via ephemeral container |
| D2 | Multi-stage Dockerfile + BuildKit cache | Single-stage builder | Cache layer dependency-only → re-build cargo source uniquement (~10× speedup) |
| D3 | Image multi-arch via `docker buildx` | Build amd64 only puis arm64 plus tard | Apple Silicon dev OOTB + Graviton AWS ready |
| D4 | Helm chart published as **OCI** artifact | helm-repo (gh-pages) | OCI push standard 2026 ; version pinning par digest possible ; 1 secret au lieu de 2 |
| D5 | gRPC health check (tonic-health) pour les 3 probes | HTTP /healthz | gRPC-native, pas de port supplémentaire, supporte sub-services |
| D6 | HPA basé sur CPU + custom metric (opt-in) | CPU only | Custom `efe_swarm_active_count` permet scale-up sur load applicatif vrai |
| D7 | NetworkPolicy opt-in (default off) | Default on | Beaucoup de clusters n'ont pas de CNI (kindnet, etc.) ; opt-in défensif |
| D8 | PrometheusRule opt-in (default off) | Default on | Requires Prometheus Operator ; opt-in pour éviter erreurs sur clusters non-équipés |
| D9 | ghz pour load test (vs k6, locust) | k6 + xk6-grpc | ghz natif gRPC, single binary, JSON-out, < 10 MB |
| D10 | Soak 24 h en minikube (vs cluster cloud) | Soak en EKS/GKE | Minikube tourne sur la machine de référence M3 Pro, reproducible 100 %, $0 |
| D11 | SLO `D9-LOAD-*` séparés des `D8-*` | Réutiliser `D8-LAT-2` | Latency under load ≠ latency single-shot ; séparer pour interpretability |
| D12 | Continue-on-error: true sur `bench-load` initialement | Fail-closed dès D9 | Charge runner ubuntu-latest variable ; flip après 3 main runs PASS |

---

## 9. 10 risques identifiés et mitigations

### R1 — Image > 50 MB budget

**Impact** : pull plus lent, footprint cloud-storage plus gros.

**Mitigation** :
* Use `gcr.io/distroless/cc-debian12:nonroot` (pas la `cc-debug` qui a + de 100 MB).
* `strip` du binaire en builder stage.
* `cargo build --release` avec LTO activé dans `Cargo.toml [profile.release]`.
* Si dépasse : analyser `du -sh /efe-server` ; option de fallback `gcr.io/distroless/static`.

### R2 — Multi-arch build prend > 30 min

**Impact** : feedback loop sur PR très lent.

**Mitigation** :
* BuildKit cache via GHA cache-from/to.
* Build matrix parallélisée (amd64 + arm64 sur 2 runners).
* `cargo build --frozen` évite re-resolve de dépendances.

### R3 — Helm chart casse sur k8s < 1.27

**Impact** : partenaires sur k8s legacy ne peuvent pas installer.

**Mitigation** :
* `kubeVersion: ">=1.27.0"` dans `Chart.yaml` → helm refuse l'install sur 1.26.
* gRPC liveness probe a besoin de k8s ≥ 1.27 (sinon utiliser `exec` avec `grpc-health-probe` binary).
* Documenter la support matrix dans `helm/efe/README.md`.

### R4 — JWT secret leak via ConfigMap

**Impact** : key material exposé à kubectl get pods.

**Mitigation** :
* JWT keys vivent dans **Secret** (jamais ConfigMap).
* Default `secrets.inline = ""` (vide) — production via `existingSecret` ou ESO.
* Mounted at `/etc/efe/secrets/hs256` avec `defaultMode: 0400`.

### R5 — Load test ghz crashe sur ubuntu-latest

**Impact** : `bench-load` job CI échoue, pas de SLO measurement.

**Mitigation** :
* Pin ghz version (`ghz_VERSION=0.118.0`).
* `continue-on-error: true` initialement.
* Run sur self-hosted runner si problème persiste.

### R6 — Soak test 24 h prend > 24 h CI budget

**Impact** : pas de soak test en CI, manuel uniquement.

**Mitigation** :
* Soak ne tourne PAS dans CI (out of scope CI budget).
* Soak tourne en local ou sur un cluster dédié.
* CI émet le rapport JSON, ne déclenche pas le soak.
* Faire un "mini-soak" 1h dans CI sur PRs qui touchent le code allocation (memory pool).

### R7 — gRPC liveness probe failure cascade

**Impact** : un slow request fait fail liveness → pod restart → cascade sur autres pods.

**Mitigation** :
* `livenessProbe.timeoutSeconds: 5` (vs 1 par défaut) — laisse temps à un cold cache.
* `failureThreshold: 3` — 3 failures consécutifs avant kill.
* `initialDelaySeconds: 10` — laisse temps au model load.
* PodDisruptionBudget (`minAvailable: 1`) garantit qu'un pod survit même cascadant.

### R8 — Cosign signing fails (OIDC token issue)

**Impact** : image publiée non-signée → impossible à verifier downstream.

**Mitigation** :
* Cosign keyless requires `id-token: write` permission dans le workflow.
* Documenter le set-up GitHub OIDC dans `docs/operations/release.md`.
* Fallback : key-based signing avec key dans GitHub Secrets (moins idéal).

### R9 — Trivy bloque le release sur CVE upstream

**Impact** : un CVE CRITICAL non-fixable dans glibc bloque toute release.

**Mitigation** :
* `.trivyignore` avec justification documentée.
* Workflow continue-on-error: true sur le scan, mais émet annotation.
* Auto-update de la base image distroless via dependabot.

### R10 — Helm chart values mismatch entre tag et docs

**Impact** : utilisateur installe avec `--version 0.1.9` mais lit la README de `main` qui décrit 0.2.0.

**Mitigation** :
* `helm/efe/README.md` est versionné avec le chart.
* Artifact Hub auto-rendering met à jour la doc par version.
* `values.schema.json` valide les inputs au moment de l'install.

---

## 10. Plan jour-par-jour (4 j wallclock)

### D9.1 (½ j) — Dockerfile + .dockerignore + local build

* `Dockerfile` multi-stage (~80 LOC).
* `.dockerignore` étendu pour exclure `target/`, `tests/`, `*.log`.
* Build local : `docker build -t efe-server:dev .` → vérifier taille < 50 MB.
* Run smoke : `docker run -p 50051:50051 efe-server:dev` + `grpcurl ... Health/Check`.

### D9.2 (1.5 j) — Helm chart complet

#### D9.2.1 (½ j) — Squelette
* `Chart.yaml`, `values.yaml`, `_helpers.tpl`, `README.md`.
* `helm lint helm/efe/` clean.

#### D9.2.2 (½ j) — Templates
* `deployment.yaml`, `service.yaml`, `configmap.yaml`, `secret.yaml` (optionnel).
* `serviceaccount.yaml` + RBAC minimal.
* `helm template helm/efe/ | kubectl apply --dry-run=server -f -` clean.

#### D9.2.3 (½ j) — Scaling + monitoring
* `hpa.yaml`, `pdb.yaml`, `networkpolicy.yaml`.
* `servicemonitor.yaml`, `prometheusrule.yaml` (opt-in).
* `tests/test-grpc-health.yaml`.
* Schema validation : `values.schema.json`.

### D9.3 (½ j) — Load test harness

* `bench/load/ghz_{smoke,sustained,stress,spike}.json`.
* `bench/load/run_load_test.sh` orchestrator.
* `bench/load/parse_ghz_report.py` (~250 LOC + 6 tests).
* `bench/load/fixtures/sample_request.json` (proto-encoded SwarmSpec).

### D9.4 (½ j wallclock + 24h compute) — Soak test

* `bench/soak/k8s_soak_job.yaml`.
* `bench/soak/run_soak_test.sh`.
* `bench/soak/leak_detector.py` (~120 LOC + 5 tests).
* Lancer le soak 24h en local (minikube) ; ne pas bloquer le sprint.

### D9.5 (¼ j) — SLOs + parser

* Ajouter 4 SLOs `D9-LOAD-*` dans `parse_criterion_report.py`.
* Tests anti-régression dans `scripts/tests/test_d9_parser_load_slos.py` (~80 LOC + 5 tests).

### D9.6 (½ j) — CI workflows

* `.github/workflows/release.yml` — Docker multi-arch build + push + cosign + Trivy + Helm package + push à GHCR.
* `.github/workflows/bench.yml` ajout job `bench-load` qui :
  * `helm install efe oci://ghcr.io/...` (sur kind ou minikube en CI).
  * Run `bench/load/run_load_test.sh sustained`.
  * Upload bench-format JSON pour aggregator.

### D9.7 (½ j) — Documentation + baseline + audit

* `docs/operations/quickstart.md` (~150 LOC).
* `docs/operations/deployment.md` (~250 LOC).
* `docs/operations/troubleshooting.md` (~200 LOC).
* `docs/operations/load_test.md` (~150 LOC).
* `BENCHMARKS.md § 11` ajout section D9.
* `bench_baselines/v0.1.9/baseline.json` placeholder + post-load freeze.
* `docs/audits/d9_load_soak_report.md` skeleton.

---

## 11. SLOs ajoutés post-D9

### 11.1 Cibles attendues

4 nouveaux SLOs dans `scripts/parse_criterion_report.py::SLO_DEFINITIONS` :

| ID | Bench key | Comparator | Target | Cible D9 |
|----|-----------|:----------:|-------:|---------:|
| `SLO-D9-LOAD-1` | `load_sustained::p99_latency_ns` | lte | 5 s | p99 ≤ 5 s @ 10 RPS |
| `SLO-D9-LOAD-2` | `load_sustained::error_rate` | lte | 0.01 (1 %) | 99 % success @ 10 RPS |
| `SLO-D9-LOAD-3` | `load_stress::throughput_rps` | gte | 5 | ≥ 5 RPS soutenable |
| `SLO-D9-LOAD-4` | `load_spike::recovery_time_ns` | lte | 30 s | recovery ≤ 30 s post-burst |

### 11.2 Evolution `bench_baselines/v0.1.9/baseline.json`

```json
{
  "version": "1.0",
  "_note": "PLACEHOLDER baseline for v0.1.9 (Sprint D9 — Docker + Helm + Load test). See docs/audits/d9_load_soak_report.md for the freeze procedure.",
  "slo_compliance": {
    "...": "(carry-over from v0.1.8)",
    "SLO-D9-LOAD-1": { "target_ns": 5000000000, "comparator": "lte" },
    "SLO-D9-LOAD-2": { "target_ns": 10000000, "comparator": "lte", "display_as": "ratio" },
    "SLO-D9-LOAD-3": { "target_ns": 5000000000, "comparator": "gte" },
    "SLO-D9-LOAD-4": { "target_ns": 30000000000, "comparator": "lte" }
  }
}
```

**Total post-D9** : 20 SLOs gated (5 D6 + 4 D7 + 7 D8 + 4 D9).

---

## 12. Anti-régression & realism check

### 12.1 Tests à pinner

#### Côté Rust

Aucun nouveau test Rust en D9 (le code Rust ne change pas ; on emballe le binaire).

Vérification : les 1 726 tests existants doivent rester verts après D9 (image Docker n'altère pas le code).

#### Côté Python

| Fichier | Tests | But |
|---------|------:|------|
| `bench/load/test_parse_ghz_report.py` | 6 | Parse JSON ghz, extract metrics, SLO mapping |
| `bench/soak/test_leak_detector.py` | 5 | OLS regression, threshold check, false positives |
| `scripts/tests/test_d9_parser_load_slos.py` | 5 | SLO definitions consistency |
| `scripts/tests/test_d9_helm_chart.py` | 4 | `helm lint`, `helm template` rendering, schema validation |
| `scripts/tests/test_d9_dockerfile_multistage.py` | 3 | Dockerfile syntax, image size estimation |

**Total nouveau D9** : 23 tests Python.

### 12.2 Helm chart validation

`scripts/tests/test_d9_helm_chart.py` :

```python
def test_helm_lint_clean():
    """`helm lint helm/efe/` returns 0."""
def test_helm_template_renders():
    """`helm template helm/efe/ -f test-values.yaml` renders all manifests."""
def test_values_schema_validates():
    """values.yaml passes its own schema."""
def test_chart_version_matches_app():
    """Chart.version == Chart.appVersion."""
```

### 12.3 Dockerfile validation

`scripts/tests/test_d9_dockerfile_multistage.py` :

```python
def test_dockerfile_syntax_valid():
    """`docker buildx build --check` clean."""
def test_dockerfile_uses_distroless_runtime():
    """Final FROM is gcr.io/distroless/cc-debian12:nonroot."""
def test_dockerfile_runs_as_nonroot():
    """USER directive set to 65532:65532."""
```

### 12.4 Load test SLO realism check

L'écart entre `target` et `measured` mesure la marge de sécurité. Si la marge est < 20 % du budget, lever un warning post-load-test :

```python
margin = (target - measured) / target
if margin < 0.20:
    warn(f"SLO {slo_id} margin only {margin:.1%}, consider tightening or scaling")
```

---

## 13. Intégration CI : release workflow + load test job

### 13.1 `release.yml` (nouveau)

Triggered on `release.published` ou tag `v*`.

```yaml
name: Release

on:
  push:
    tags:
      - 'v*'
  release:
    types: [published]
  workflow_dispatch:
    inputs:
      tag:
        description: 'Tag to publish (e.g. v0.1.9)'
        required: true

permissions:
  contents: read
  packages: write
  id-token: write   # for cosign keyless

jobs:
  docker-build-publish:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        platform: [linux/amd64, linux/arm64]
    steps:
      - uses: actions/checkout@v4
      - uses: docker/setup-buildx-action@v3
      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}
      - name: Build + push
        id: build
        uses: docker/build-push-action@v5
        with:
          platforms: ${{ matrix.platform }}
          push: true
          tags: |
            ghcr.io/efe-ai/efe-server:${{ github.ref_name }}
            ghcr.io/efe-ai/efe-server:latest
          build-args: |
            VERSION=${{ github.ref_name }}
            GIT_SHA=${{ github.sha }}
            BUILD_DATE=${{ steps.date.outputs.iso }}
          cache-from: type=gha,scope=${{ matrix.platform }}
          cache-to: type=gha,mode=max,scope=${{ matrix.platform }}
      - uses: aquasecurity/trivy-action@master
        with:
          image-ref: ghcr.io/efe-ai/efe-server:${{ github.ref_name }}
          severity: CRITICAL,HIGH
          exit-code: '1'

  cosign-sign:
    needs: docker-build-publish
    runs-on: ubuntu-latest
    steps:
      - uses: sigstore/cosign-installer@v3
      - run: |
          cosign sign --yes \
            ghcr.io/efe-ai/efe-server:${{ github.ref_name }}

  helm-publish:
    needs: docker-build-publish
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: azure/setup-helm@v4
      - run: |
          helm package helm/efe/ --version ${{ github.ref_name }}
          helm push efe-${{ github.ref_name }}.tgz oci://ghcr.io/efe-ai/charts
```

### 13.2 `bench.yml` ajout du job `bench-load`

```yaml
bench-load:
  name: Run gRPC load test (Sprint D9 SLOs)
  needs: bench-build
  runs-on: ubuntu-latest
  timeout-minutes: 20
  continue-on-error: true   # flip to false post-3-runs-PASS
  steps:
    - uses: actions/checkout@v4
    - name: Install ghz
      run: |
        wget -q https://github.com/bojand/ghz/releases/download/v0.118.0/ghz_0.118.0_Linux_x86_64.tar.gz
        tar xzf ghz_0.118.0_Linux_x86_64.tar.gz
        sudo mv ghz /usr/local/bin/
    - name: Build efe-server
      run: cargo build --release -p efe-server
    - name: Start server
      run: |
        EFE_LISTEN_ADDR=127.0.0.1:50051 EFE_AUTH_ENABLED=false \
            ./target/release/efe-server &
        echo $! > server.pid
        for _ in $(seq 1 60); do
          nc -z 127.0.0.1 50051 && break
          sleep 1
        done
    - name: Run sustained load test (D9-LOAD-1, D9-LOAD-2)
      run: |
        bash bench/load/run_load_test.sh sustained \
          --endpoint 127.0.0.1:50051 \
          --output-dir target/load
    - name: Run stress test (D9-LOAD-3)
      run: |
        bash bench/load/run_load_test.sh stress \
          --endpoint 127.0.0.1:50051 \
          --output-dir target/load
    - name: Run spike test (D9-LOAD-4)
      run: |
        bash bench/load/run_load_test.sh spike \
          --endpoint 127.0.0.1:50051 \
          --output-dir target/load
    - name: Stop server
      if: always()
      run: kill $(cat server.pid) 2>/dev/null || true
    - name: Upload artefacts
      if: always()
      uses: actions/upload-artifact@v4
      with:
        name: bench-partial-load-${{ github.sha }}
        path: target/load/
```

L'aggregator pickup `target/load/load_test_bench.json` comme un eval-bench-format additionnel.

### 13.3 Flip fail-closed

Après 3 main runs PASS post-merge D9 :
* PR séparée flip `bench-load.continue-on-error: false`.
* Toute régression > 5 % sur p99 ou throughput bloque le merge.

---

## 14. Critères d'acceptation (DoD)

### 14.1 Image Docker

- [ ] `Dockerfile` multi-stage builds clean (`docker buildx build --check`).
- [ ] Image finale < 50 MB (`docker images ghcr.io/efe-ai/efe-server`).
- [ ] Multi-arch : amd64 + arm64 publiés.
- [ ] Image signée par cosign (verified by `cosign verify`).
- [ ] SBOM attaché (verified by `cosign download sbom`).
- [ ] Trivy scan : 0 CRITICAL, 0 HIGH non-whitelisted.
- [ ] `docker run` → server bind on 50051, gRPC health check OK.

### 14.2 Helm chart

- [ ] `helm lint helm/efe/` clean.
- [ ] `helm template helm/efe/` renders all 11 manifests.
- [ ] `helm install` succeeds on minikube 1.27+.
- [ ] `kubectl wait --for=condition=available` returns within 120 s.
- [ ] `helm test` (gRPC health probe) succeeds.
- [ ] `helm uninstall` cleans up all resources.
- [ ] `helm push` to OCI registry succeeds.

### 14.3 Load test

- [ ] All 4 ghz scenarios (smoke / sustained / stress / spike) execute.
- [ ] `parse_ghz_report.py` emits valid bench-format JSON.
- [ ] 4 SLOs `D9-LOAD-*` evaluated by parser.
- [ ] CI job `bench-load` executes within 20 min.
- [ ] Markdown summary report emitted to `target/load/load_test_summary.md`.

### 14.4 Soak test

- [ ] `bench/soak/run_soak_test.sh 24h` completes on minikube without error.
- [ ] `leak_detector.py` runs on the dump and reports no leak.
- [ ] Post-soak accuracy on hand-curated 9-case test : ≥ 8/9 correct.
- [ ] 24h soak report committed to `docs/audits/d9_load_soak_report.md`.

### 14.5 Documentation

- [ ] `docs/operations/quickstart.md` (10 min onboarding).
- [ ] `docs/operations/deployment.md` (production runbook).
- [ ] `docs/operations/troubleshooting.md` (error catalog).
- [ ] `docs/operations/load_test.md` (reproducibility).
- [ ] `helm/efe/README.md` (chart usage).
- [ ] `BENCHMARKS.md § 11` updated.

### 14.6 CI

- [ ] `.github/workflows/release.yml` triggers on tag `v*`.
- [ ] `.github/workflows/bench.yml` includes `bench-load` job.
- [ ] (Post-3-runs-PASS) PR séparée flips `bench-load.continue-on-error: false`.

### 14.7 Validation finale

- [ ] `helm install efe oci://ghcr.io/efe-ai/charts/efe --version 0.1.9` succeeds on a fresh minikube.
- [ ] `kubectl port-forward svc/efe 50051:50051` + `python3 examples/hybrid_classification.py` works.
- [ ] All 1 726 + N Rust tests + 59 + 23 = 82 Python tests PASS.
- [ ] `cargo clippy --workspace --tests -- -D warnings` clean.

---

## 15. Out of scope & roadmap follow-ups

### 15.1 Hors-scope D9

* **Multi-tenant resource quotas** — D11.
* **OAuth2/OIDC** — D15 (Phase 3 SaaS).
* **Per-tenant billing** — D16.
* **Multi-region failover** — D17.
* **CUDA backend** — Phase 5 / E1.
* **Catalog hot-reload** — D12.
* **Streaming RPC** — D10 (Letter B).

### 15.2 Roadmap après D9

| Sprint | Cible | Justification |
|--------|-------|---------------|
| **D9.bis** | First partner pilot deployment (real customer cluster) | Valide la chaîne D8→D9 sur infrastructure non-Anthropic |
| **D10** | Streaming RPC (`execute_streaming` réel) | Letter B follow-up |
| **D11** | Per-tenant resource quotas | Phase 2 milestone |
| **D12** | Catalog admin endpoints + versioning | Hot-reload prod |
| **D13** | Observability dashboards (Grafana) | Production monitoring |
| **D14** | Production cutover | Premier client payant |

### 15.3 Métrique de succès post-D9

À 30 jours après merge D9 :

> *« 1 partenaire pilote a installé l'image v0.1.9 via Helm chart sur
> son cluster k8s. Le quickstart `docs/operations/quickstart.md` est
> validé en moins de 15 minutes. Le load test produit p99 ≤ 5 s à
> 10 RPS sur les 3 catalogs synthétiques. Le soak 24 h n'a montré
> aucun memory leak. Les 4 SLOs `D9-LOAD-*` sont gated en CI et le
> job `bench-load` est passé en fail-closed. »*

---

## 16. Annexes

### 16.1 Annexe A — Helm install cheat-sheet

```bash
# Minimal install (no auth, single-tenant)
helm install efe oci://ghcr.io/efe-ai/charts/efe --version 0.1.9

# Production install (auth + monitoring)
helm install efe-prod oci://ghcr.io/efe-ai/charts/efe \
  --version 0.1.9 \
  --namespace efe \
  --create-namespace \
  --set replicaCount=3 \
  --set autoscaling.maxReplicas=20 \
  --set config.auth.enabled=true \
  --set secrets.existingSecret=efe-jwt-prod \
  --set serviceMonitor.enabled=true \
  --set prometheusRule.enabled=true \
  --set networkPolicy.enabled=true

# Smoke test
helm test efe -n efe

# Upgrade
helm upgrade efe oci://ghcr.io/efe-ai/charts/efe --version 0.1.10

# Rollback
helm rollback efe 1

# Uninstall
helm uninstall efe -n efe
```

### 16.2 Annexe B — Load test reproductibilité (local)

```bash
# 1. Build server
cargo build --release -p efe-server

# 2. Start
EFE_LISTEN_ADDR=127.0.0.1:50051 EFE_AUTH_ENABLED=false \
    ./target/release/efe-server &
SERVER_PID=$!

# 3. Wait for bind
for _ in $(seq 1 60); do nc -z 127.0.0.1 50051 && break ; sleep 1 ; done

# 4. Run all 4 scenarios
bash bench/load/run_load_test.sh all \
  --endpoint 127.0.0.1:50051 \
  --output-dir target/load

# 5. Inspect markdown report
cat target/load/load_test_summary.md

# 6. Stop server
kill $SERVER_PID
```

### 16.3 Annexe C — Glossary

| Terme | Définition |
|-------|------------|
| **Distroless** | Famille d'images de base Google sans shell/pkg manager — surface CVE minimale |
| **OCI artifact** | Format générique pour publier des objets non-image (Helm chart, SBOM, signatures) sur un registry compliant |
| **Cosign** | Outil sigstore pour signer/vérifier des images container avec OIDC keyless |
| **SBOM** | Software Bill Of Materials — inventaire structuré des dépendances |
| **CycloneDX** | Format SBOM standard supporté par cosign |
| **HPA** | Horizontal Pod Autoscaler — scale les replicas en fonction de métriques |
| **PDB** | PodDisruptionBudget — garantit min_available pendant un voluntary disruption |
| **PreStop hook** | Lifecycle hook k8s exécuté avant SIGTERM, idéal pour drain |
| **Soak test** | Test de longue durée pour détecter les leaks invisibles à court terme |
| **ghz** | Outil de load test gRPC-native, single binary |

### 16.4 Annexe D — Checklist d'exécution architect → impl

```
□ D9.1.1  Dockerfile multi-stage (~80 LOC)
□ D9.1.2  .dockerignore étendu
□ D9.1.3  docker build local → check size < 50 MB
□ D9.1.4  docker run → grpcurl Health/Check OK

□ D9.2.1  helm/efe/Chart.yaml + values.yaml + _helpers.tpl
□ D9.2.2  templates/{deployment, service, configmap, sa, hpa, pdb}.yaml
□ D9.2.3  templates/{networkpolicy, servicemonitor, prometheusrule}.yaml (opt-in)
□ D9.2.4  templates/tests/test-grpc-health.yaml
□ D9.2.5  values.schema.json + helm lint clean
□ D9.2.6  helm/efe/README.md

□ D9.3.1  bench/load/ghz_*.json (4 scenarios)
□ D9.3.2  bench/load/run_load_test.sh
□ D9.3.3  bench/load/parse_ghz_report.py + 6 tests
□ D9.3.4  bench/load/fixtures/sample_request.json

□ D9.4.1  bench/soak/k8s_soak_job.yaml
□ D9.4.2  bench/soak/run_soak_test.sh
□ D9.4.3  bench/soak/leak_detector.py + 5 tests

□ D9.5.1  4 SLOs D9-LOAD-* dans parse_criterion_report.py
□ D9.5.2  scripts/tests/test_d9_parser_load_slos.py + 5 tests

□ D9.6.1  .github/workflows/release.yml (Docker + Helm + Trivy + cosign)
□ D9.6.2  .github/workflows/bench.yml ajout bench-load job
□ D9.6.3  .github/workflows/release.yml smoke run sur tag de test

□ D9.7.1  docs/operations/quickstart.md
□ D9.7.2  docs/operations/deployment.md
□ D9.7.3  docs/operations/troubleshooting.md
□ D9.7.4  docs/operations/load_test.md
□ D9.7.5  BENCHMARKS.md § 11 update
□ D9.7.6  bench_baselines/v0.1.9/baseline.json placeholder
□ D9.7.7  docs/audits/d9_load_soak_report.md skeleton

□ Validation finale
  □ docker build & run smoke
  □ helm lint && helm template
  □ minikube start && helm install efe → kubectl wait
  □ python3 examples/hybrid_classification.py via port-forward
  □ cargo test --workspace : 1 726 tests still PASS
  □ cargo clippy : clean
  □ All Python tests (D6 + D7 + D8 + D9) : 59 + 23 = 82 PASS
  □ release.yml dry-run sur tag de test (v0.1.9-rc1)
```

### 16.5 Annexe E — Mini-checklist Pilot 0.1

```
□ Helm chart installable sur cluster fresh minikube
□ Quickstart 10 min validé par 1 dev externe
□ Load test sustained passe ≥ 10 RPS @ p99 ≤ 5 s
□ Soak 24h sans leak (RSS slope < 870 B/s)
□ 500 cas réels d'un partenaire pilote ≥ 0.85 acc (carry-over D8.2)
□ Release v0.1.9 publié sur GHCR (image + chart)
□ Documentation onboarding partenaire réelle
```

---

**Fin de l'architecture D9.** Le sprinter peut implémenter en
suivant Annexe D, en revenant à §3 pour le détail Docker, §4 pour
Helm, §5-6 pour load/soak.

> **Approbation requise avant implémentation** :
> - [ ] Lead infra (revue Helm chart conformity à votre standard k8s)
> - [ ] Lead sécurité (revue distroless + cosign + Trivy)
> - [ ] Owner produit (revue quickstart partenaire)
> - [ ] Lead dev (revue Dockerfile + multi-arch matrix)
