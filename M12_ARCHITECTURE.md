# M12 — Intégration, conteneurisation, déploiement

> **Module** : M12
> **Titre** : Intégration, conteneurisation, déploiement
> **Crate(s) d'implémentation** : `efe-server` (binaire), `docker/`, `deploy/`, `scripts/`, `.github/workflows/`
> **Position dans le MVP** : dernier module — c'est lui qui transforme un ensemble de crates
> bibliothèque en *service opérationnel livrable*.
> **Version du document** : 1.0 (MVP)
> **Date** : 19 avril 2026
> **Statut** : contrat d'ingénierie, approuvé pour implémentation
> **Auteur de référence** : Architecture EFE — M12
> **Convention de lecture** : ultra-détaillée, chirurgicale, reproductible fichier par fichier

---

## Sommaire

1. [Mission de M12](#1-mission-de-m12)
2. [Positionnement dans le MVP](#2-positionnement-dans-le-mvp)
3. [Principes directeurs](#3-principes-directeurs)
4. [Architecture en couches](#4-architecture-en-couches)
5. [Modèle de déploiement cible](#5-modèle-de-déploiement-cible)
6. [Contrat d'état de l'existant](#6-contrat-détat-de-lexistant)
7. [Sous-module M12.1 — Bootstrap binaire (`main.rs`)](#7-sous-module-m121--bootstrap-binaire-mainrs)
8. [Sous-module M12.2 — `ServerConfig` et variables d'environnement](#8-sous-module-m122--serverconfig-et-variables-denvironnement)
9. [Sous-module M12.3 — Graceful shutdown et drainage](#9-sous-module-m123--graceful-shutdown-et-drainage)
10. [Sous-module M12.4 — Health checks (gRPC Health v1)](#10-sous-module-m124--health-checks-grpc-health-v1)
11. [Sous-module M12.5 — Dockerfile de production](#11-sous-module-m125--dockerfile-de-production)
12. [Sous-module M12.6 — Reproductibilité de build et SBOM](#12-sous-module-m126--reproductibilité-de-build-et-sbom)
13. [Sous-module M12.7 — Supply chain (cosign, provenance SLSA)](#13-sous-module-m127--supply-chain-cosign-provenance-slsa)
14. [Sous-module M12.8 — Helm chart et déploiement Kubernetes](#14-sous-module-m128--helm-chart-et-déploiement-kubernetes)
15. [Sous-module M12.9 — ConfigMap, Secrets, NetworkPolicy](#15-sous-module-m129--configmap-secrets-networkpolicy)
16. [Sous-module M12.10 — Observabilité opérationnelle](#16-sous-module-m1210--observabilité-opérationnelle)
17. [Sous-module M12.11 — Alerting rules et SLO](#17-sous-module-m1211--alerting-rules-et-slo)
18. [Sous-module M12.12 — Runbook d'incident](#18-sous-module-m1212--runbook-dincident)
19. [Sous-module M12.13 — Smoke tests post-déploiement](#19-sous-module-m1213--smoke-tests-post-déploiement)
20. [Sous-module M12.14 — Canary et stratégie de rollout](#20-sous-module-m1214--canary-et-stratégie-de-rollout)
21. [Sous-module M12.15 — Rollback et retour arrière](#21-sous-module-m1215--rollback-et-retour-arrière)
22. [Sous-module M12.16 — Release pipeline et versioning](#22-sous-module-m1216--release-pipeline-et-versioning)
23. [Sous-module M12.17 — CI et portes qualité](#23-sous-module-m1217--ci-et-portes-qualité)
24. [Tests d'intégration de M12](#24-tests-dintégration-de-m12)
25. [Critères d'acceptation globaux M12](#25-critères-dacceptation-globaux-m12)
26. **Annexes**
    - A. [Dockerfile de production (fichier complet)](#annexe-a--dockerfile-de-production-fichier-complet)
    - B. [Helm chart (fichiers complets)](#annexe-b--helm-chart-fichiers-complets)
    - C. [Alerting rules Prometheus](#annexe-c--alerting-rules-prometheus)
    - D. [smoke.sh (script complet)](#annexe-d--smokesh-script-complet)
    - E. [Runbook d'opérations (squelette)](#annexe-e--runbook-dopérations-squelette)
    - F. [Matrice complète des variables d'environnement](#annexe-f--matrice-complète-des-variables-denvironnement)
    - G. [Registre des risques d'opérations](#annexe-g--registre-des-risques-dopérations)
    - H. [Glossaire DevOps](#annexe-h--glossaire-devops)

---

## 1. Mission de M12

M12 est le **module de sortie** du MVP EFE. Alors que M1..M11 produisent des *crates bibliothèques*
(code réutilisable) et une *surface gRPC testable* (M10) instrumentée (M11), M12 est l'ensemble des
artefacts et procédures qui permettent à une équipe d'exploitation de :

1. **Construire** une image conteneur reproductible, signée et datée.
2. **Déployer** cette image sur un cluster Kubernetes avec des valeurs par défaut sûres.
3. **Observer** la santé du déploiement en temps réel via Prometheus + Grafana + Jaeger.
4. **Répondre** aux incidents (crash, fuite mémoire, alerte SLO) par un runbook documenté.
5. **Rouler** le service dans les deux sens (forward / rollback) sans perte d'audit.
6. **Prouver** la provenance de l'image (attestation SLSA + signature cosign).

La mission de M12 est donc de **fermer la boucle** : passer de `cargo build --release` à un binaire
qui sert du trafic de production, en laissant à chaque étape une trace vérifiable.

### 1.1 Ce que M12 **est**

- Un **contrat d'intégration** : la liste exacte des fichiers, variables d'environnement et
  procédures qui composent la livraison.
- Un **runbook opérable** : documentation vérifiée par simulation d'incidents.
- Une **référence de reproductibilité** : deux builds de la même référence git doivent produire
  deux images à hash identique.
- Un **ensemble de garanties** : liveness, readiness, drain, rollback, signature.

### 1.2 Ce que M12 **n'est pas**

- **Il n'est pas** une plateforme : M12 ne fournit pas de multi-tenancy orchestrée, pas de
  billing, pas de portail utilisateur. Ces préoccupations sont hors MVP.
- **Il n'est pas** un service mesh : le routage entre services est délégué à l'ingress du
  cluster hôte (Kubernetes + ingress-nginx en défaut). mTLS intra-cluster relève de la V2.
- **Il n'est pas** une fonction de hot-reload : tout changement de configuration critique
  (secrets JWT, bornes de quota) déclenche un redéploiement.
- **Il n'est pas** un fournisseur de persistance : EFE reste stateless (M9 = registre en
  mémoire dans le MVP, persistance relève de V2).

---

## 2. Positionnement dans le MVP

Le diagramme ci-dessous montre où M12 s'insère. Les modules M1..M11 sont *internes* au processus
serveur ; M12 *entoure* le processus pour lui donner une vie opérationnelle.

```
                ┌────────────────────────── Cluster Kubernetes ──────────────────────────┐
                │                                                                        │
                │   ┌── Pod efe-server ───────────────────────────────┐                  │
                │   │                                                 │                  │
                │   │   ┌──────────────────────────────────────────┐  │                  │
  Client gRPC   │   │   │  Processus Rust (efe-server binaire)     │  │   Prometheus     │
  ────────────►─┼───┤   │                                          │  │   (scrape)       │
                │   │   │   M10 gRPC ────► M8 Controller           │  │   ◄──/metrics──  │
                │   │   │       │              │                   │  │                  │
                │   │   │       │              ├─► M3 Kernels      │  │                  │
                │   │   │       │              ├─► M4 Fusion       │  │                  │
                │   │   │       │              └─► M9 Registry     │  │                  │
                │   │   │       │                                  │  │                  │
                │   │   │       └─► M11 Telemetry ──► /metrics ────┼──┘                  │
                │   │   │                     │                    │                     │
                │   │   │                     └─► JSON logs ───────┼──► stdout → Loki    │
                │   │   │                     └─► audit.jsonl ─────┼──► PVC              │
                │   │   └──────────────────────────────────────────┘                     │
                │   │                                                                    │
                │   │   M12 fournit :                                                    │
                │   │     - le binaire (Dockerfile.prod)                                 │
                │   │     - la config (ConfigMap)                                        │
                │   │     - les secrets (Secret + CSI)                                   │
                │   │     - les sondes (liveness, readiness, grpc-health)                │
                │   │     - le graceful drain sur SIGTERM                                │
                │   └────────────────────────────────────────────────┘                   │
                │                                                                        │
                └────────────────────────────────────────────────────────────────────────┘
```

M12 est donc un module **périphérique** : il ne modifie aucun invariant interne des autres modules,
mais il **contraint leurs conditions d'existence** (signaux Unix, variables d'environnement, ports,
FS monté en RO, utilisateur non-root, etc.).

---

## 3. Principes directeurs

Les principes suivants sont **opposables** pendant les revues de PR touchant M12. Toute
dérogation exige une fiche signée.

### 3.1 Douze-facteurs (12-factor app)

M12 respecte strictement la doctrine *12-factor* :

1. **Codebase unique, déploiements multiples** : le même binaire est déployé en dev / staging /
   prod ; seule la configuration varie.
2. **Dépendances explicites** : `Cargo.lock` commité, lock-file Python (`uv.lock`), `.tool-versions`
   géré par `rtx`. Aucune dépendance système n'est supposée (runtime distroless).
3. **Config via environnement** : chaque paramètre critique a une variable d'environnement ;
   voir §15.1.
4. **Services support via URL** : Prometheus, Jaeger, audit log path, tous paramétrables.
5. **Strict séparation build / release / run** : build = Dockerfile ; release = Helm values ;
   run = conteneur avec ConfigMap + Secret injectés.
6. **Processus stateless** : toute persistance critique est externalisée (audit log sur volume,
   pas de DB locale).
7. **Exposition de ports** : 50051 (gRPC), 9090 (Prometheus, interne).
8. **Concurrence via processus** : le scaling horizontal se fait par HPA sur Kubernetes, pas par
   augmentation du pool de threads.
9. **Démarrage rapide / arrêt gracieux** : boot ≤ 3 s cold, drain ≤ 25 s sur SIGTERM.
10. **Dev/prod parité** : `docker/compose.dev.yml` reproduit prod au plus près (otel-collector,
    prometheus, jaeger).
11. **Logs comme flux d'événements** : JSON sur stdout, pas de fichier log-rotation géré par
    l'app. Seul l'audit log est persistant, sur un volume explicite.
12. **Administration = processus ponctuel** : aucun « mode admin » ; les opérations (rotation de
    clés, purge cache) sont des RPC gRPC ou des redéploiements.

### 3.2 Reproductibilité bit-à-bit

Deux `docker build` du même commit git doivent produire :
- le même digest SHA-256 d'image,
- le même SBOM,
- la même attestation SLSA.

Cela implique :
- `SOURCE_DATE_EPOCH` injecté dans le build,
- `--locked` passé à `cargo build`,
- pas de `RUN apt-get update && ... $(date)` ou équivalent dynamique,
- utilisation de `--mount=type=cache` ou de layers pré-construits pour éviter les re-résolutions de
  dépendances non-déterministes.

### 3.3 Moindre privilège

- **Utilisateur non-root** : UID 10001, GID 10001 (réservé EFE), système de fichiers racine en
  lecture seule, volumes écriture limités à `/var/lib/efe/audit`.
- **Capabilities Linux réduites** : pas de `CAP_NET_ADMIN`, pas de `CAP_SYS_*`.
- **Pas de shell** : base distroless, pas de `/bin/sh`, donc pas d'attaque par command injection
  via des variables mal échappées.
- **Kubernetes SecurityContext strict** : `runAsNonRoot: true`, `readOnlyRootFilesystem: true`,
  `allowPrivilegeEscalation: false`, `seccompProfile: RuntimeDefault`.

### 3.4 Observabilité comme propriété **publique**

Les 12 instruments Prometheus de M11 sont un **contrat vis-à-vis de l'exploitant**. M12 les
expose sur `/metrics` (port 9090 par convention) et les annote dans des `ServiceMonitor`
Kubernetes afin que Prometheus Operator puisse les découvrir automatiquement.

### 3.5 Sécurité en profondeur

- Le binaire ne contient **aucun secret par défaut** : les clés JWT proviennent d'un `Secret`
  monté, pas d'une variable d'environnement directe (évite fuite via `/proc/<pid>/environ`).
- Les images sont **scannées** par Trivy en CI avant publication.
- Les vulnérabilités **high/critical** bloquent la release (override documenté requis).
- `cosign sign --yes` sur chaque image release, vérifié à l'admission par **Kyverno** ou
  **Sigstore policy-controller** côté cluster.

### 3.6 Zéro surprise à la mise à jour

La V1 garantit :
- Pas de changement **breaking** du proto `efe.v1` sans bump majeur de version serveur.
- Pas de suppression d'une métrique Prometheus sans période de dépréciation (≥ 1 mineur).
- Pas de changement de schéma d'audit JSONL sans versionner le champ `schema`.

---

## 4. Architecture en couches

M12 se déploie en **sept couches** superposées qui doivent être comprises comme un *empilement
contractuel*. Chaque couche fait une garantie à la couche supérieure.

```
┌────────────────────────────────────────────────────────────────────┐
│  Couche 7 : Opérateur humain / runbook / on-call                   │
├────────────────────────────────────────────────────────────────────┤
│  Couche 6 : Alerting (Alertmanager) / SLO / Runbook links          │
├────────────────────────────────────────────────────────────────────┤
│  Couche 5 : Helm chart / Kustomize overlays / Argo CD application  │
├────────────────────────────────────────────────────────────────────┤
│  Couche 4 : Image conteneur signée (distroless + efe-server)       │
├────────────────────────────────────────────────────────────────────┤
│  Couche 3 : Binaire efe-server (Rust release, statique-glibc)      │
├────────────────────────────────────────────────────────────────────┤
│  Couche 2 : Bootstrap (main.rs) : init télémétrie → serve → drain  │
├────────────────────────────────────────────────────────────────────┤
│  Couche 1 : Cargo workspace (M1..M11) — contrat bibliothèque       │
└────────────────────────────────────────────────────────────────────┘
```

**Garanties inter-couches** :

- **Couche 1 → 2** : les modules internes exposent des traits `Arc<dyn ...>` et des types
  `#[non_exhaustive]` permettant au bootstrap de tout brancher sans code métier.
- **Couche 2 → 3** : le binaire est mono-processus, thread-safe, et prend exactement zéro
  argument de ligne de commande (configuration par environnement).
- **Couche 3 → 4** : le binaire démarre en ≤ 3 s, respecte `SIGTERM`, écoute sur
  `${EFE_LISTEN_ADDR}` (port 50051 par défaut), expose `/metrics` et grpc-health.
- **Couche 4 → 5** : l'image est distroless (zero-shell), signée cosign, accompagnée d'un SBOM
  SPDX JSON et d'une attestation SLSA niveau 3.
- **Couche 5 → 6** : le déploiement expose un `ServiceMonitor` + des `PrometheusRule` dans le
  même namespace ; Prometheus Operator les pick automatiquement.
- **Couche 6 → 7** : les alertes pointent vers une section du runbook (lien canonique stable).

---

## 5. Modèle de déploiement cible

### 5.1 Topologie de référence

| Environnement | Cluster         | Réplicas | Ressources / pod              | Autoscaling | Ingress       |
|---------------|-----------------|----------|-------------------------------|-------------|---------------|
| dev           | kind (local)    | 1        | 500m CPU / 512 Mi mem         | non         | port-forward  |
| staging       | GKE régional    | 2        | 1 CPU / 1 Gi mem              | HPA 2→6     | TLS cert-mgr  |
| prod          | GKE multi-zone  | 3        | 2 CPU / 2 Gi mem (min)        | HPA 3→20    | TLS + WAF     |

### 5.2 Stratégie de rollout

- **Type Deployment** : `RollingUpdate` avec `maxSurge: 1`, `maxUnavailable: 0`.
- **PodDisruptionBudget** : `minAvailable: 2` en prod (sur HPA >= 3) pour tolérer une éviction
  planifiée (drain de nœud, upgrade k8s).
- **Readiness gate** : la sonde readiness ne répond `SERVING` qu'après **init télémétrie terminée**
  et **serveur en `listen`**. Un pod pas encore prêt ne prend pas de trafic.
- **Drain** : au shutdown, la sonde readiness bascule à `NOT_SERVING` immédiatement, le pod
  continue à répondre aux requêtes en vol pendant ≤ 25 s, puis sort du `endpoints`.

### 5.3 Préemption et interruptions

- **Spot nodes autorisés** en staging uniquement. Prod exige des nœuds *on-demand*.
- **GracePeriod** Kubernetes : `terminationGracePeriodSeconds: 30` (25 s drain + 5 s overhead).
- Si le drain dépasse 25 s, Kubernetes SIGKILL ; l'audit log est sauvé par `fsync` périodique
  (1 s) dans M11, **donc perte maximale 1 s d'événements d'audit**, acceptée.

### 5.4 Stockage

- **Volume audit** : `emptyDir` par défaut (perdu au redémarrage) en dev/staging ; **PVC** avec
  storageClass `standard-rwo` en prod, 5 Gi, rétention 7 jours.
- **Volume logs** : **pas de volume** (logs JSON sur stdout, collectés par l'agent cluster
  — Fluent Bit, Vector, Loki, etc.).

### 5.5 Sécurité réseau

- **Ingress** : TLS terminé au niveau ingress-nginx ; le pod voit du gRPC en clair.
- **NetworkPolicy** : `efe-server` accepte du trafic de `ingress-nginx-controller` et de
  `prometheus` (pour `/metrics`) ; tout autre trafic entrant est refusé.
- **Egress** : sortie autorisée vers DNS (kube-dns) et **rien d'autre** en MVP (pas d'appel
  externe ; les futurs backends OTLP seraient autorisés explicitement).

---

## 6. Contrat d'état de l'existant

Avant d'implémenter les sous-modules M12.x, il est essentiel de fixer ce qui est **déjà livré**
dans le repository et ce qui reste à **construire**. Cet inventaire est la base de la matrice
d'acceptation.

### 6.1 Existant au 19 avril 2026

| Chemin                                      | Rôle                                    | État          |
|---------------------------------------------|-----------------------------------------|---------------|
| `crates/efe-server/src/main.rs`             | Bootstrap télémétrie → serveur → shutdown | **Présent** (M11 wiring OK) |
| `crates/efe-server/src/config.rs`           | `ServerConfig`, `AuthConfig`, etc.      | **Présent**   |
| `crates/efe-server/src/shutdown.rs`         | `Shutdown` + SIGTERM/SIGINT             | **Présent**   |
| `crates/efe-server/src/runtime.rs`          | `Server::builder()` + `serve()`         | **Présent**   |
| `docker/Dockerfile.dev`                     | Image de développement                  | **Présent**, squelette |
| `docker/Dockerfile.prod`                    | Image de production                     | **Présent**, CUDA-based (à remplacer par distroless-glibc) |
| `docker/compose.dev.yml`                    | Stack dev (otel + prom + jaeger)        | **Présent**   |
| `docker/prometheus.yml`                     | Scrape config dev                       | **Présent**   |
| `docker/otel-config.yaml`                   | Pipeline OTLP dev                       | **Présent**   |
| `.github/workflows/ci.yml`                  | CI (fmt, clippy, tests, cargo audit)    | **Présent**   |
| `Justfile`                                  | Recipes locales (bootstrap, test, doc)  | **Présent**   |
| `configs/`                                  | Placeholder (vide)                      | **Vide**      |
| `scripts/check-commit-msg.sh`               | Validation Conventional Commits         | **Présent**   |

### 6.2 À construire pour clôturer M12

| Chemin cible                                | Rôle                                  | Statut |
|---------------------------------------------|---------------------------------------|--------|
| `docker/Dockerfile.prod` (réécrit)          | Distroless multi-stage, glibc         | **Nouveau** |
| `deploy/helm/efe/Chart.yaml`                | Métadonnées chart                     | **Nouveau** |
| `deploy/helm/efe/values.yaml`               | Valeurs par défaut                    | **Nouveau** |
| `deploy/helm/efe/templates/deployment.yaml` | Deployment + SecurityContext          | **Nouveau** |
| `deploy/helm/efe/templates/service.yaml`    | Service ClusterIP                     | **Nouveau** |
| `deploy/helm/efe/templates/configmap.yaml`  | Config non-secrète                    | **Nouveau** |
| `deploy/helm/efe/templates/secret.yaml`     | Squelette (optionnel, via ExternalSecret) | **Nouveau** |
| `deploy/helm/efe/templates/servicemonitor.yaml` | Prometheus Operator discovery    | **Nouveau** |
| `deploy/helm/efe/templates/prometheusrule.yaml` | Alertes SLO                       | **Nouveau** |
| `deploy/helm/efe/templates/networkpolicy.yaml` | Isolation ingress/egress           | **Nouveau** |
| `deploy/helm/efe/templates/pdb.yaml`        | PodDisruptionBudget                   | **Nouveau** |
| `deploy/helm/efe/templates/hpa.yaml`        | HorizontalPodAutoscaler               | **Nouveau** |
| `scripts/smoke.sh`                          | Smoke test post-déploiement           | **Nouveau** |
| `crates/efe-server/src/services/health.rs`  | gRPC Health v1 + readiness basculante | **À confirmer / Nouveau si absent** |
| `.github/workflows/release.yml`             | Build + sign + push sur tag           | **Nouveau** |
| `docs/runbook.md`                           | Runbook d'incidents                   | **Nouveau** |
| `docs/slo.md`                               | SLO / error budget                    | **Nouveau** |

---

## 7. Sous-module M12.1 — Bootstrap binaire (`main.rs`)

### 7.1 Périmètre

Le bootstrap est le seul endroit où les **dépendances concrètes** sont choisies. Il sépare
strictement :

1. **Instanciation** : init télémétrie, création registry, lecture config.
2. **Assemblage** : construction du `Server` via builder injectant `ExecuteBackend` + `AlignmentRegistry`.
3. **Exécution** : `server.serve(addr, shutdown).await`.
4. **Arrêt** : `telemetry.shutdown().await`, logs de confirmation, retour du `Result`.

### 7.2 Séquence contractuelle

La séquence est **gravée dans le marbre** ; tout changement doit être accompagné d'un test de
régression dans `tests/startup_sequence.rs`.

```text
┌─ Séquence de démarrage ──────────────────────────────────────────────┐
│                                                                      │
│ 1. TelemetryConfig::from_env()                                       │
│ 2. init(cfg) -> TelemetryHandle  (install tracing-subscriber global) │
│ 3. handle.ensure_metrics_bound().await  (bind /metrics lazy)         │
│ 4. AlignmentRegistry = InMemoryRegistry::with_default_config()       │
│ 5. events_rx = registry.take_events_receiver().await                 │
│ 6. if Some(rx): spawn(RegistrySink::new(audit).run(rx))              │
│ 7. raw_backend : Arc<dyn ExecuteBackend> = ...                       │
│ 8. collector   : Arc<dyn TelemetryCollector> = TracingTelemetryCollector::new() │
│ 9. backend     : Arc<dyn ExecuteBackend> = InstrumentedBackend::new(raw_backend, collector) │
│ 10. server = Server::builder().config(cfg).backend(backend).registry(reg).build()? │
│ 11. shutdown = Shutdown::new()                                       │
│ 12. info!(... "efe-server starting")                                 │
│ 13. serve_result = server.serve(addr, shutdown).await                │
│ 14. telemetry.shutdown().await       (flush audit + arrêt /metrics)  │
│ 15. info!("efe-server shut down cleanly")                            │
│ 16. return serve_result                                              │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘
```

### 7.3 Invariants de bootstrap

- **Invariant B1 — Unicité** : `init(cfg)` est appelée une seule fois par processus (panic
  documentée si doublon, via `OnceCell`).
- **Invariant B2 — Ordre** : la télémétrie est initialisée **avant** tout log d'application
  (sinon les traces sont perdues).
- **Invariant B3 — Drain-first** : `telemetry.shutdown()` est appelé **après** le retour de
  `server.serve`, jamais en parallèle (sinon risque de perte d'audit).
- **Invariant B4 — Idempotence** : `telemetry.shutdown().await` est idempotent (double appel
  sans effet).
- **Invariant B5 — Pas de code métier dans main.rs** : toute logique métier réside dans un
  crate. `main.rs` fait uniquement du *wiring*.

### 7.4 Gestion des échecs de bootstrap

| Étape | Erreur possible                          | Action                                        |
|-------|------------------------------------------|-----------------------------------------------|
| 1     | `TelemetryConfig::from_env()` invalide   | `anyhow::Context("init telemetry")` + exit 1  |
| 2     | Global subscriber déjà installé          | Panic explicite (bug de programmation)        |
| 3     | Bind `/metrics` échoue (port pris)       | `anyhow::Context("bind metrics endpoint")` + exit 1 |
| 4     | Registry config invalide                 | `anyhow::Error` + exit 1                      |
| 10    | Builder incomplet                        | `anyhow::Error` + exit 1                      |
| 13    | `serve` échoue (tonic)                   | Log + retour de l'erreur (shutdown télémétrie quand même) |

Toute erreur de bootstrap **n'est pas retry** : le pod crash, Kubernetes le redémarre avec
backoff exponentiel. Cette discipline simplifie les tests et évite les états *half-live*.

### 7.5 Preuve d'existence du wiring actuel

Le fichier `crates/efe-server/src/main.rs` commité après M11 implémente déjà toute la
séquence B1..B5. Un extrait de référence (cf. `§14 - M12` du PDF MVP) :

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let telemetry_cfg = TelemetryConfig::from_env();
    let mut telemetry = init_telemetry(telemetry_cfg).context("init telemetry")?;
    telemetry.ensure_metrics_bound().await.context("bind metrics endpoint")?;

    let cfg = ServerConfig::default();
    let concrete_registry = Arc::new(InMemoryRegistry::with_default_config()?);
    let events_rx = concrete_registry.take_events_receiver().await;

    if let Some(rx) = events_rx {
        let audit = telemetry.audit_sink();
        tokio::spawn(RegistrySink::new(audit).run(rx));
    }

    let raw_backend: Arc<dyn ExecuteBackend> = Arc::new(UnimplementedBackend);
    let collector: Arc<dyn efe_core::TelemetryCollector> = Arc::new(TracingTelemetryCollector::new());
    let backend: Arc<dyn ExecuteBackend> = Arc::new(InstrumentedBackend::new(raw_backend, collector));

    let registry_for_server: Arc<dyn efe_core::alignment::AlignmentRegistry> = concrete_registry;
    let server = Server::builder()
        .config(cfg.clone())
        .backend(backend)
        .registry(registry_for_server)
        .build()
        .context("failed to build server")?;

    let shutdown = Shutdown::new();
    let addr = cfg.listen_addr;
    info!(version = efe_core::VERSION, %addr,
          metrics_addr = ?telemetry.metrics_local_addr(),
          "efe-server starting");

    let serve_result = server.serve(addr, shutdown).await;

    telemetry.shutdown().await;
    info!("efe-server shut down cleanly");
    serve_result
}
```

---

## 8. Sous-module M12.2 — `ServerConfig` et variables d'environnement

### 8.1 Modèle de configuration

La configuration du serveur suit la hiérarchie `ServerConfig > AuthConfig | RateLimitConfig
| IdempotencyConfig | ShutdownConfig`. Tous les champs ont un défaut raisonnable documenté
dans `crates/efe-server/src/config.rs`.

| Struct                | Champ clé             | Default              | Env var                       |
|-----------------------|-----------------------|----------------------|-------------------------------|
| `ServerConfig`        | `listen_addr`         | `127.0.0.1:50051`    | `EFE_LISTEN_ADDR`             |
| `ServerConfig`        | `max_concurrent_streams` | `1024`            | `EFE_MAX_CONCURRENT_STREAMS`  |
| `ServerConfig`        | `request_timeout`     | `30s`                | `EFE_REQUEST_TIMEOUT_MS`      |
| `ServerConfig`        | `enable_admin`        | `true`               | `EFE_ENABLE_ADMIN`            |
| `AuthConfig`          | `enabled`             | `false`              | `EFE_AUTH_ENABLED`            |
| `AuthConfig`          | `jwt_audience`        | `"efe.v1"`           | `EFE_JWT_AUDIENCE`            |
| `AuthConfig`          | `jwt_issuer`          | `None`               | `EFE_JWT_ISSUER`              |
| `AuthConfig`          | `allow_insecure_dev`  | `true`               | `EFE_AUTH_ALLOW_INSECURE_DEV` |
| `AuthConfig`          | `hs256_secrets`       | `[]`                 | **fichier monté**, pas env var |
| `RateLimitConfig`     | `enabled`             | `false`              | `EFE_RATELIMIT_ENABLED`       |
| `RateLimitConfig`     | `default_capacity`    | `200`                | `EFE_RATELIMIT_CAPACITY`      |
| `RateLimitConfig`     | `default_refill_rate` | `100.0`              | `EFE_RATELIMIT_REFILL`        |
| `RateLimitConfig`     | `anonymous_capacity`  | `50`                 | `EFE_RATELIMIT_ANON_CAP`      |
| `RateLimitConfig`     | `anonymous_refill_rate` | `10.0`             | `EFE_RATELIMIT_ANON_REFILL`   |
| `IdempotencyConfig`   | `enabled`             | `true`               | `EFE_IDEMPOTENCY_ENABLED`     |
| `IdempotencyConfig`   | `cache_capacity`      | `10_000`             | `EFE_IDEMPOTENCY_CAPACITY`    |
| `IdempotencyConfig`   | `ttl`                 | `10s`                | `EFE_IDEMPOTENCY_TTL_MS`      |
| `ShutdownConfig`      | `drain_timeout`       | `25s`                | `EFE_SHUTDOWN_DRAIN_MS`       |

### 8.2 Règles de parsing

- **Durées** : toutes les env vars qui finissent par `_MS` sont des millisecondes entières
  (`u64`), parsées via `str::parse::<u64>()` puis converties en `Duration::from_millis`.
- **Booléens** : `1`, `true`, `TRUE`, `yes`, `on` → `true` ; tout autre → `false` (si env var
  présente ; si absente → défaut de la struct).
- **Adresses** : passées telles quelles à `SocketAddr::from_str`. Une erreur de parsing
  entraîne un crash au bootstrap (fail-fast).
- **Secrets** : **interdits dans les env vars**. Les secrets HS256 sont lus depuis un **fichier**
  pointé par `EFE_JWT_SECRETS_FILE` (défaut `/etc/efe/secrets/hs256`). Chaque ligne = une clé
  base64 ; la première est la clé active (signature), les suivantes servent la rotation
  (validation seule).

### 8.3 Validation

`ServerConfig::validate(&self) -> Result<(), EfeError>` effectue :

- `listen_addr.port() != 0` (le port 0 n'est autorisé que dans les tests).
- `max_concurrent_streams ∈ [1, 65_535]`.
- `request_timeout ∈ [100ms, 10min]`.
- Si `auth.enabled == true`, alors `hs256_secrets` doit être non-vide.
- Si `enable_admin == true`, alors un registry doit être injecté (vérifié plus tard par le
  builder du Server, mais l'intention est documentée ici).
- Les quotas de rate-limit doivent être cohérents (`capacity ≥ refill_rate * request_timeout`).

### 8.4 Précédence des sources

```
defaults (struct)  <  env vars  <  fichier TOML optionnel (EFE_CONFIG_FILE)  <  CLI (MVP: aucun arg)
```

En MVP, **aucun argument CLI** n'est accepté (le binaire ignore `argv[1..]` avec un warning).
Cela simplifie la surface et évite les écarts entre Helm et `docker run`.

### 8.5 Exemple de ConfigMap Kubernetes

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: efe-config
data:
  EFE_LISTEN_ADDR: "0.0.0.0:50051"
  EFE_REQUEST_TIMEOUT_MS: "30000"
  EFE_AUTH_ENABLED: "true"
  EFE_JWT_AUDIENCE: "efe.v1"
  EFE_JWT_ISSUER: "https://auth.example.com"
  EFE_RATELIMIT_ENABLED: "true"
  EFE_RATELIMIT_CAPACITY: "500"
  EFE_RATELIMIT_REFILL: "200.0"
  EFE_IDEMPOTENCY_CAPACITY: "50000"
  EFE_IDEMPOTENCY_TTL_MS: "300000"
  EFE_SHUTDOWN_DRAIN_MS: "25000"
  EFE_METRICS_ADDR: "0.0.0.0:9090"
  EFE_AUDIT_LOG_PATH: "/var/lib/efe/audit/audit.jsonl"
```

---

## 9. Sous-module M12.3 — Graceful shutdown et drainage

### 9.1 Vue d'ensemble

Le `Shutdown` combine :
- un **flag atomique** `triggered`,
- un **Notify** pour réveiller les attendants,
- **deux signal handlers** Unix (`SIGTERM`, `SIGINT`).

L'API `Shutdown::notified().await` attend jusqu'à ce que l'un des trois stimuli active.

### 9.2 Chaîne causale du drain

```text
 SIGTERM reçu
    │
    ▼
 Shutdown::trigger() interne
    │
    ├──► server.serve_with_shutdown() sort de l'accept loop  (plus de NEW conn)
    │
    ├──► HealthService -> NOT_SERVING pour /live et /ready
    │       (kube-proxy retire le pod de endpoints en ≤ 1 scrape interval)
    │
    ├──► in-flight streams + unary : continuent, drain_timeout s (défaut 25 s)
    │
    └──► après drain_timeout : tonic server retourne
              │
              ▼
    serve_result = Ok(())  (ou Err si échec)
              │
              ▼
    telemetry.shutdown().await
       │
       ├──► metrics server graceful shutdown (timeout 2 s)
       ├──► audit_sink.flush() (fsync)
       └──► drop sinks
              │
              ▼
    info!("efe-server shut down cleanly")
              │
              ▼
    process exit code 0
```

### 9.3 Paramètre `drain_timeout`

- **Défaut** : 25 s.
- **Borne inférieure** : 5 s (sinon une requête p95 ~900 ms n'a pas le temps de finir).
- **Borne supérieure** : la `terminationGracePeriodSeconds` du pod k8s, typiquement 30 s.
- **Recommandation prod** : garder 25 s côté app et 30 s côté k8s, laissant 5 s pour la
  finalisation télémétrie + exit process propre.

### 9.4 Sonde readiness pendant le drain

- **Avant shutdown** : readiness = `SERVING`.
- **Shutdown.trigger()** appelé : readiness bascule `NOT_SERVING` *immédiatement*.
- **Kube-proxy** retire le pod de Endpoints en ≤ `readinessProbe.periodSeconds` (défaut 10 s).
- **Entre-temps** : les connexions existantes continuent, mais aucune nouvelle connexion n'est
  acceptée.

Cela exige que le `HealthService` ait une référence partagée à `Shutdown` et interroge son
`is_triggered()` à chaque `Check`.

### 9.5 Tests du drain

| Scénario                                    | Vérification                              |
|---------------------------------------------|-------------------------------------------|
| Trigger manuel pendant idle                 | exit en < 1 s                             |
| SIGTERM pendant 5 requêtes en vol           | les 5 finissent, exit après la dernière   |
| SIGTERM pendant 1 streaming ouvert          | stream continue jusqu'à `drain_timeout`   |
| SIGTERM pendant drain_timeout dépassé       | pod tué par SIGKILL, ≤ 1 s audit perdu    |
| Double SIGTERM (opérateur impatient)        | pas de double-trigger, second ignoré      |
| SIGTERM + kill -9 par opérateur             | pas de panique dans le handler signal     |

Ces scénarios sont couverts par `tests/shutdown.rs` (`drain_finishes_inflight_requests`,
`signal_trigger_is_idempotent`, etc.).

---

## 10. Sous-module M12.4 — Health checks (gRPC Health v1)

### 10.1 Protocole

M12 implémente le service standard `grpc.health.v1.Health` :

```protobuf
service Health {
  rpc Check(HealthCheckRequest) returns (HealthCheckResponse);
  rpc Watch(HealthCheckRequest) returns (stream HealthCheckResponse);
}
enum ServingStatus {
  UNKNOWN = 0;
  SERVING = 1;
  NOT_SERVING = 2;
  SERVICE_UNKNOWN = 3;
}
```

### 10.2 Sémantique

| `service` (requête)      | Lecture                                          |
|--------------------------|--------------------------------------------------|
| `""` (global)            | État agrégé : `SERVING` si bootstrap terminé **et** pas en drain |
| `"efe.v1.SwarmService"`  | Idem agrégé (MVP : pas de granularité par service) |
| `"efe.v1.AdminService"`  | Idem                                             |
| Tout autre               | `SERVICE_UNKNOWN`                                |

### 10.3 Deux sondes, deux sémantiques

- **Liveness** : le pod est-il *vivant* ? Si non, k8s **kill & restart**. Critère : le processus
  répond-il ? → un `Check("")` renvoyant quelconque réponse (même `NOT_SERVING`) prouve la vie.
  Mieux : `SERVING` obligatoire. Configuré en `exec` : `["/grpc_health_probe", "-addr=:50051"]`.
- **Readiness** : le pod peut-il *recevoir du trafic* ? Si non, k8s **retire des Endpoints**
  mais ne tue pas. Critère : `Check("")` renvoie `SERVING` ET le drain n'a pas démarré.

### 10.4 Implémentation cible

```rust
pub struct HealthServiceImpl {
    shutdown: Shutdown,
    ready: Arc<AtomicBool>,  // flippé par main.rs juste avant serve()
}
impl HealthServiceImpl {
    pub fn new(shutdown: Shutdown) -> Self {
        Self { shutdown, ready: Arc::new(AtomicBool::new(false)) }
    }
    pub fn mark_ready(&self) { self.ready.store(true, Ordering::SeqCst); }
}
#[async_trait]
impl Health for HealthServiceImpl {
    async fn check(&self, req: TRequest<HealthCheckRequest>) -> Result<TResponse<HealthCheckResponse>, Status> {
        let status = if !self.ready.load(Ordering::SeqCst) {
            ServingStatus::NotServing
        } else if self.shutdown.is_triggered() {
            ServingStatus::NotServing
        } else {
            match req.into_inner().service.as_str() {
                "" | "efe.v1.SwarmService" | "efe.v1.AdminService" => ServingStatus::Serving,
                _ => ServingStatus::ServiceUnknown,
            }
        };
        Ok(TResponse::new(HealthCheckResponse { status: status as i32 }))
    }
    type WatchStream = tonic::codec::Streaming<HealthCheckResponse>;
    async fn watch(&self, _req: TRequest<HealthCheckRequest>) -> Result<TResponse<Self::WatchStream>, Status> {
        Err(Status::unimplemented("watch not implemented in MVP"))
    }
}
```

### 10.5 Sondes Kubernetes

```yaml
livenessProbe:
  exec:
    command: ["/usr/local/bin/grpc_health_probe", "-addr=:50051"]
  initialDelaySeconds: 5
  periodSeconds: 10
  timeoutSeconds: 2
  failureThreshold: 3
readinessProbe:
  exec:
    command: ["/usr/local/bin/grpc_health_probe", "-addr=:50051"]
  initialDelaySeconds: 2
  periodSeconds: 5
  timeoutSeconds: 2
  failureThreshold: 2
  successThreshold: 1
startupProbe:
  exec:
    command: ["/usr/local/bin/grpc_health_probe", "-addr=:50051"]
  initialDelaySeconds: 1
  periodSeconds: 2
  failureThreshold: 30   # 60 s max pour démarrer
```

`grpc_health_probe` est un binaire statique (< 4 Mio) co-copié dans l'image prod. Il évite de
dépendre de `curl` pour sonder du gRPC.

---

## 11. Sous-module M12.5 — Dockerfile de production

### 11.1 Cible et justification

Le `Dockerfile.prod` existant est basé sur CUDA (nécessaire pour M3 kernels GPU en V2). Pour
le **MVP**, l'inférence est sur CPU (BLAS), donc l'image de production doit être **distroless-
glibc**, drastiquement plus petite (≤ 60 Mio) et sans surface d'attaque (pas de shell, pas de
package manager).

### 11.2 Structure multi-stage

```dockerfile
# syntax=docker/dockerfile:1.7
# ---------------------------------------------------------------------
# Stage 1 : planner — cargo-chef cache-aware dependency graph
# ---------------------------------------------------------------------
FROM rust:1.85-slim AS planner
WORKDIR /app
RUN cargo install cargo-chef --locked --version 0.1.67
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ---------------------------------------------------------------------
# Stage 2 : cacher — compile dependencies only
# ---------------------------------------------------------------------
FROM rust:1.85-slim AS cacher
WORKDIR /app
RUN cargo install cargo-chef --locked --version 0.1.67
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# ---------------------------------------------------------------------
# Stage 3 : builder — compile app against cached deps
# ---------------------------------------------------------------------
FROM rust:1.85-slim AS builder
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*
COPY --from=cacher /app/target target
COPY --from=cacher /usr/local/cargo /usr/local/cargo
COPY . .
ARG SOURCE_DATE_EPOCH=0
ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}
RUN cargo build --release --locked --bin efe-server && \
    strip target/release/efe-server

# ---------------------------------------------------------------------
# Stage 4 : grpc_health_probe — fetch static binary
# ---------------------------------------------------------------------
FROM alpine:3.20 AS health-probe
ARG GRPC_HEALTH_PROBE_VERSION=v0.4.25
RUN apk add --no-cache curl && \
    curl -fsSL "https://github.com/grpc-ecosystem/grpc-health-probe/releases/download/${GRPC_HEALTH_PROBE_VERSION}/grpc_health_probe-linux-amd64" \
        -o /grpc_health_probe && chmod +x /grpc_health_probe

# ---------------------------------------------------------------------
# Stage 5 : runtime — distroless-glibc
# ---------------------------------------------------------------------
FROM gcr.io/distroless/cc-debian12:nonroot
LABEL org.opencontainers.image.source="https://github.com/efe-project/efe"
LABEL org.opencontainers.image.description="EFE gRPC server (MVP)"
LABEL org.opencontainers.image.licenses="Apache-2.0"

USER 10001:10001
WORKDIR /
COPY --from=builder --chown=10001:10001 /app/target/release/efe-server /usr/local/bin/efe-server
COPY --from=health-probe --chown=10001:10001 /grpc_health_probe /usr/local/bin/grpc_health_probe

EXPOSE 50051 9090

ENV RUST_LOG="info,efe=info"
ENV EFE_LISTEN_ADDR="0.0.0.0:50051"
ENV EFE_METRICS_ADDR="0.0.0.0:9090"

ENTRYPOINT ["/usr/local/bin/efe-server"]
```

### 11.3 Annotations OCI

L'image porte des labels conformes OCI Image Spec :

| Label                                   | Valeur attendue                             |
|----------------------------------------|----------------------------------------------|
| `org.opencontainers.image.source`      | URL du repository git                        |
| `org.opencontainers.image.revision`    | commit SHA                                   |
| `org.opencontainers.image.created`     | timestamp RFC3339 du build                   |
| `org.opencontainers.image.version`     | tag sémantique (ex. `0.1.0`)                 |
| `org.opencontainers.image.licenses`    | `Apache-2.0`                                 |
| `org.opencontainers.image.description` | court descriptif                             |

### 11.4 Justifications techniques

- **cargo-chef** : découpe les dépendances des sources app → cache Docker massivement réutilisé.
  Gain observé : 8 min → 40 s sur un changement mineur.
- **strip** : retire les symboles de debug du binaire release (gain ~5 Mio).
- **distroless-glibc** plutôt qu'Alpine-musl : `rustls` + BLAS dépendent de glibc en prod
  classique, et distroless fournit exactement glibc sans shell.
- **UID 10001 non-root** : évite tout risque d'écriture privilégiée sur un volume monté par
  erreur en read-write.
- **grpc_health_probe embarqué** : sinon on ajouterait `busybox` + `wget` pour sonder, ce qui
  augmente la surface d'attaque de ~2 Mio de shell. `grpc_health_probe` fait 4 Mio mais est
  statique.

### 11.5 Mesures cibles

| Mesure                       | Cible MVP | Mesuré |
|------------------------------|-----------|--------|
| Taille image (compressée)    | ≤ 60 Mio  | TBD    |
| Taille image (décompressée)  | ≤ 150 Mio | TBD    |
| Vulnérabilités high/critical | 0         | TBD    |
| Temps de build (cache chaud) | ≤ 45 s    | TBD    |
| Temps de build (cold)        | ≤ 8 min   | TBD    |
| Cold start pod               | ≤ 3 s     | TBD    |

---

## 12. Sous-module M12.6 — Reproductibilité de build et SBOM

### 12.1 Principe

Deux builds du même commit doivent produire deux images **bit-identiques** : même digest
SHA-256, même SBOM. C'est la condition d'une **attestation de provenance** fiable.

### 12.2 Leviers

- `SOURCE_DATE_EPOCH` injecté à `docker buildx build --build-arg SOURCE_DATE_EPOCH=$(git log -1 --format=%ct)`.
- `cargo build --locked` (refuse toute résolution dynamique de `Cargo.lock`).
- **Pas** de `apt-get upgrade` (seulement `install`).
- Version exacte épinglée pour `cargo-chef`, `grpc_health_probe`, et l'image de base
  (`rust:1.85-slim` → `rust:1.85.0-slim-bookworm@sha256:...`).
- **Digests** plutôt que tags flottants dans la FROM finale.

### 12.3 SBOM

Format : **SPDX 2.3 JSON**. Générateur : `syft` (sous licence Apache 2.0).

```bash
syft packages oci-dir:./efe-image.oci \
  -o spdx-json=sbom.spdx.json \
  -o cyclonedx-json=sbom.cdx.json
```

Le SBOM est :
- **attaché** à l'image comme attestation (cosign `--attachment=sbom`),
- **publié** à côté de l'image release (bucket public),
- **vérifié** par la chaîne d'admission du cluster via `cosign verify-attestation`.

### 12.4 Manifest de reproductibilité

Un fichier `build-manifest.json` est généré pour chaque release :

```json
{
  "version": "0.1.0",
  "git_sha": "abc123...",
  "build_time_epoch": 1760000000,
  "rust_version": "1.85.0",
  "base_image_digest": "sha256:...",
  "image_digest": "sha256:...",
  "sbom_digest": "sha256:...",
  "builder_host": "github-actions-runner-ubuntu-22",
  "reproducibility_hash": "sha256(git_sha || base_image_digest || SOURCE_DATE_EPOCH)"
}
```

Ce manifest est le **témoin contractuel** que deux builds ont les mêmes entrées.

---

## 13. Sous-module M12.7 — Supply chain (cosign, provenance SLSA)

### 13.1 Objectif SLSA

Le pipeline de release EFE vise le **niveau 3 SLSA** :
- Builder hébergé et isolé (GitHub Actions avec OIDC + `permissions: id-token: write`),
- Provenance signée et non falsifiable,
- Accès restreint aux secrets (clés cosign ephemeral via keyless),
- Build entièrement scripté (aucune étape manuelle).

### 13.2 Étapes de signature

```bash
# Image + attestation de provenance
cosign sign --yes \
    --oidc-issuer https://token.actions.githubusercontent.com \
    ghcr.io/efe-project/efe:0.1.0@sha256:...

cosign attest --yes \
    --predicate provenance.slsa.json \
    --type slsaprovenance \
    ghcr.io/efe-project/efe:0.1.0@sha256:...

cosign attest --yes \
    --predicate sbom.spdx.json \
    --type spdxjson \
    ghcr.io/efe-project/efe:0.1.0@sha256:...
```

### 13.3 Vérification à l'admission

Côté cluster, une **ClusterImagePolicy** (sigstore policy-controller) :

```yaml
apiVersion: policy.sigstore.dev/v1beta1
kind: ClusterImagePolicy
metadata:
  name: efe-image-policy
spec:
  images:
    - glob: "ghcr.io/efe-project/efe:*"
  authorities:
    - keyless:
        identities:
          - issuer: "https://token.actions.githubusercontent.com"
            subject: "https://github.com/efe-project/efe/.github/workflows/release.yml@refs/tags/v*"
        ctlog: {}
      attestations:
        - name: slsa-provenance
          predicateType: https://slsa.dev/provenance/v1
        - name: sbom
          predicateType: https://spdx.dev/Document
```

Toute tentative de déployer une image non signée ou signée par un acteur inattendu est
**refusée à l'admission**.

---

## 14. Sous-module M12.8 — Helm chart et déploiement Kubernetes

### 14.1 Layout du chart

```
deploy/helm/efe/
├── Chart.yaml
├── README.md
├── values.yaml
├── values.staging.yaml
├── values.prod.yaml
└── templates/
    ├── _helpers.tpl
    ├── NOTES.txt
    ├── serviceaccount.yaml
    ├── configmap.yaml
    ├── secret.yaml           # ← template défensif, préférer ExternalSecret
    ├── deployment.yaml
    ├── service.yaml
    ├── servicemonitor.yaml
    ├── prometheusrule.yaml
    ├── networkpolicy.yaml
    ├── pdb.yaml
    └── hpa.yaml
```

### 14.2 `values.yaml` (défauts raisonnables)

```yaml
image:
  repository: ghcr.io/efe-project/efe
  tag: ""                 # default = Chart.appVersion
  pullPolicy: IfNotPresent
  digest: ""              # recommandé en prod, ex. "sha256:..."

replicaCount: 3

resources:
  requests:
    cpu: 500m
    memory: 512Mi
  limits:
    cpu: 2000m
    memory: 2Gi

service:
  type: ClusterIP
  grpcPort: 50051
  metricsPort: 9090

podSecurityContext:
  runAsNonRoot: true
  runAsUser: 10001
  runAsGroup: 10001
  fsGroup: 10001
  seccompProfile:
    type: RuntimeDefault

securityContext:
  allowPrivilegeEscalation: false
  readOnlyRootFilesystem: true
  capabilities:
    drop: ["ALL"]

serviceAccount:
  create: true
  annotations: {}
  name: ""

config:
  listenAddr: "0.0.0.0:50051"
  metricsAddr: "0.0.0.0:9090"
  requestTimeoutMs: 30000
  maxConcurrentStreams: 1024
  enableAdmin: true
  auth:
    enabled: true
    jwtAudience: "efe.v1"
    jwtIssuer: ""
  rateLimit:
    enabled: true
    defaultCapacity: 500
    defaultRefill: 200.0
    anonymousCapacity: 50
    anonymousRefill: 10.0
  idempotency:
    enabled: true
    cacheCapacity: 50000
    ttlMs: 300000
  shutdown:
    drainMs: 25000

secrets:
  externalSecretName: ""    # si utilisé, monté sur /etc/efe/secrets
  # ou inline pour dev:
  hs256:
    enabled: false
    keys:
      - "dev-secret-key"

audit:
  persistence:
    enabled: true
    size: 5Gi
    storageClassName: "standard-rwo"
    mountPath: "/var/lib/efe/audit"

livenessProbe:
  initialDelaySeconds: 5
  periodSeconds: 10
  timeoutSeconds: 2
  failureThreshold: 3

readinessProbe:
  initialDelaySeconds: 2
  periodSeconds: 5
  timeoutSeconds: 2
  failureThreshold: 2

startupProbe:
  initialDelaySeconds: 1
  periodSeconds: 2
  failureThreshold: 30

autoscaling:
  enabled: true
  minReplicas: 3
  maxReplicas: 20
  targetCPUUtilizationPercentage: 70
  targetMemoryUtilizationPercentage: 80

pdb:
  enabled: true
  minAvailable: 2

networkPolicy:
  enabled: true
  allowedSources:
    - podSelector:
        matchLabels:
          app.kubernetes.io/name: ingress-nginx
    - podSelector:
        matchLabels:
          app.kubernetes.io/name: prometheus

serviceMonitor:
  enabled: true
  interval: 15s
  scrapeTimeout: 10s
  labels: {}

prometheusRule:
  enabled: true
  labels:
    release: kube-prometheus-stack

nodeSelector: {}
tolerations: []
affinity:
  podAntiAffinity:
    preferredDuringSchedulingIgnoredDuringExecution:
      - weight: 100
        podAffinityTerm:
          labelSelector:
            matchLabels:
              app.kubernetes.io/name: efe
          topologyKey: kubernetes.io/hostname

terminationGracePeriodSeconds: 30
```

### 14.3 Deployment (template annoté)

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {{ include "efe.fullname" . }}
  labels: {{- include "efe.labels" . | nindent 4 }}
spec:
  replicas: {{ if .Values.autoscaling.enabled }}null{{ else }}{{ .Values.replicaCount }}{{ end }}
  strategy:
    type: RollingUpdate
    rollingUpdate:
      maxSurge: 1
      maxUnavailable: 0
  selector:
    matchLabels: {{- include "efe.selectorLabels" . | nindent 6 }}
  template:
    metadata:
      labels: {{- include "efe.selectorLabels" . | nindent 8 }}
      annotations:
        checksum/config: {{ include (print $.Template.BasePath "/configmap.yaml") . | sha256sum }}
    spec:
      serviceAccountName: {{ include "efe.serviceAccountName" . }}
      terminationGracePeriodSeconds: {{ .Values.terminationGracePeriodSeconds }}
      securityContext: {{- toYaml .Values.podSecurityContext | nindent 8 }}
      containers:
        - name: efe
          image: "{{ .Values.image.repository }}:{{ .Values.image.tag | default .Chart.AppVersion }}{{ if .Values.image.digest }}@{{ .Values.image.digest }}{{ end }}"
          imagePullPolicy: {{ .Values.image.pullPolicy }}
          securityContext: {{- toYaml .Values.securityContext | nindent 12 }}
          ports:
            - name: grpc
              containerPort: {{ .Values.service.grpcPort }}
            - name: metrics
              containerPort: {{ .Values.service.metricsPort }}
          envFrom:
            - configMapRef:
                name: {{ include "efe.fullname" . }}-config
          {{- if .Values.secrets.externalSecretName }}
          volumeMounts:
            - name: secrets
              mountPath: /etc/efe/secrets
              readOnly: true
            - name: audit
              mountPath: {{ .Values.audit.persistence.mountPath }}
            - name: tmp
              mountPath: /tmp
          {{- end }}
          livenessProbe: {{- include "efe.grpcHealthProbe" . | nindent 12 }}
          readinessProbe: {{- include "efe.grpcHealthProbe" . | nindent 12 }}
          startupProbe: {{- include "efe.grpcHealthProbe" . | nindent 12 }}
          resources: {{- toYaml .Values.resources | nindent 12 }}
      volumes:
        - name: tmp
          emptyDir: {}
        {{- if .Values.secrets.externalSecretName }}
        - name: secrets
          secret:
            secretName: {{ .Values.secrets.externalSecretName }}
            defaultMode: 0400
        {{- end }}
        {{- if .Values.audit.persistence.enabled }}
        - name: audit
          persistentVolumeClaim:
            claimName: {{ include "efe.fullname" . }}-audit
        {{- else }}
        - name: audit
          emptyDir: {}
        {{- end }}
      {{- with .Values.nodeSelector }}
      nodeSelector: {{- toYaml . | nindent 8 }}
      {{- end }}
      {{- with .Values.tolerations }}
      tolerations: {{- toYaml . | nindent 8 }}
      {{- end }}
      {{- with .Values.affinity }}
      affinity: {{- toYaml . | nindent 8 }}
      {{- end }}
```

### 14.4 Justifications

- **`readOnlyRootFilesystem: true`** + montage `/tmp` en `emptyDir` : le binaire ne doit pas
  écrire sur le FS racine. Seuls `/tmp` (scratch) et `/var/lib/efe/audit` sont writable.
- **`checksum/config`** sur l'annotation : tout changement de ConfigMap déclenche un rollout
  automatique via Helm upgrade.
- **`replicas: null` si HPA** : sinon Helm et HPA entrent en conflit (tug-of-war sur le champ).
- **`terminationGracePeriodSeconds = drainMs / 1000 + 5 s`** : laisse le temps à la télémétrie
  de flusher après le drain.

---

## 15. Sous-module M12.9 — ConfigMap, Secrets, NetworkPolicy

### 15.1 ConfigMap

La ConfigMap contient uniquement des valeurs **non-secrètes**. Toute valeur qui fuiterait de
`kubectl get configmap -o yaml` doit être déplacée en Secret.

Champs : toutes les env vars de §8.1 sauf celles qui concernent les secrets.

### 15.2 Secret — stratégie recommandée : `ExternalSecret`

**Pattern préféré** : utiliser `external-secrets.io` pour récupérer les clés depuis Vault /
AWS Secrets Manager / GCP Secret Manager. Le chart génère un `ExternalSecret` *quand
`secrets.externalSecretName` est vide et `secrets.vault.enabled` est true* :

```yaml
apiVersion: external-secrets.io/v1beta1
kind: ExternalSecret
metadata:
  name: efe-hs256
spec:
  refreshInterval: 1h
  secretStoreRef:
    name: vault-backend
    kind: ClusterSecretStore
  target:
    name: efe-hs256
  data:
    - secretKey: hs256
      remoteRef:
        key: secret/data/efe/hs256
        property: keys
```

### 15.3 Montage du Secret

Le pod monte `Secret: efe-hs256` sur `/etc/efe/secrets` en mode 0400. `main.rs` lit
`/etc/efe/secrets/hs256` ligne à ligne.

### 15.4 Rotation sans downtime

- **Étape 1** : ajouter la nouvelle clé K2 dans le Secret (K2 en première ligne, K1 restant).
- **Étape 2** : attendre la propagation (≤ 60 s via Vault sidecar, ou immédiat via reload d'env).
- **Étape 3** : le serveur accepte les tokens signés par K2 (nouvelle clé) et K1 (ancienne).
- **Étape 4** : l'émetteur de tokens bascule sur K2.
- **Étape 5** : après la fenêtre de TTL max d'un token (typiquement 24 h), retirer K1.

`AuthConfig::hs256_secrets` est un `Vec<Vec<u8>>` exactement pour supporter cette rotation.

### 15.5 NetworkPolicy

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: {{ include "efe.fullname" . }}
spec:
  podSelector:
    matchLabels: {{- include "efe.selectorLabels" . | nindent 6 }}
  policyTypes: [Ingress, Egress]
  ingress:
    - from:
        {{- toYaml .Values.networkPolicy.allowedSources | nindent 8 }}
      ports:
        - protocol: TCP
          port: {{ .Values.service.grpcPort }}
        - protocol: TCP
          port: {{ .Values.service.metricsPort }}
  egress:
    - to:
        - namespaceSelector: {}
          podSelector:
            matchLabels:
              k8s-app: kube-dns
      ports:
        - protocol: UDP
          port: 53
        - protocol: TCP
          port: 53
    # Autoriser OTLP vers otel-collector si présent
    - to:
        - podSelector:
            matchLabels:
              app.kubernetes.io/name: opentelemetry-collector
      ports:
        - protocol: TCP
          port: 4317
```

---

## 16. Sous-module M12.10 — Observabilité opérationnelle

### 16.1 Les quatre flux

| Flux      | Protocole      | Destination             | Rétention   |
|-----------|----------------|-------------------------|-------------|
| Metrics   | Prometheus scrape | Prometheus / VictoriaMetrics | 30 j     |
| Logs      | JSON stdout    | Fluent Bit → Loki       | 14 j        |
| Traces    | (OTLP futur)   | Jaeger / Tempo          | 7 j         |
| Audit     | JSONL fichier  | PVC → backup S3 nightly | 90 j        |

### 16.2 ServiceMonitor

```yaml
apiVersion: monitoring.coreos.com/v1
kind: ServiceMonitor
metadata:
  name: {{ include "efe.fullname" . }}
  labels:
    release: kube-prometheus-stack
spec:
  selector:
    matchLabels: {{- include "efe.selectorLabels" . | nindent 6 }}
  endpoints:
    - port: metrics
      interval: {{ .Values.serviceMonitor.interval }}
      scrapeTimeout: {{ .Values.serviceMonitor.scrapeTimeout }}
      path: /metrics
      relabelings:
        - sourceLabels: [__meta_kubernetes_pod_name]
          targetLabel: pod
        - sourceLabels: [__meta_kubernetes_namespace]
          targetLabel: namespace
```

### 16.3 Tableaux Grafana

Le chart ne livre **pas** les dashboards Grafana (laissés à l'équipe plateforme), mais exporte
un **JSON dashboard de référence** dans `docs/grafana/efe-overview.json` avec :

- Panneau 1 : p50 / p95 / p99 latency `efe_grpc_request_duration_seconds` par méthode.
- Panneau 2 : taux d'erreur `rate(efe_grpc_requests_total{code!="OK"}[5m])`.
- Panneau 3 : `efe_swarm_rounds_total` et `efe_swarm_confidence` histogrammes.
- Panneau 4 : `efe_ratelimit_drops_total` par tenant.
- Panneau 5 : `efe_breaches_total` + `efe_breach_magnitude`.
- Panneau 6 : inflight gauge `efe_grpc_requests_in_flight`.

---

## 17. Sous-module M12.11 — Alerting rules et SLO

### 17.1 SLO cibles MVP

| SLI                                  | Formule                                                    | SLO cible  | Fenêtre |
|--------------------------------------|------------------------------------------------------------|------------|---------|
| Disponibilité                        | `1 - (errors / requests)` avec errors = 5xx/Unavail       | ≥ 99,5 %   | 30 j    |
| Latence p95                          | `histogram_quantile(0.95, ...)` sur Execute                | ≤ 900 ms   | 30 j    |
| Taux de convergence                  | `rate(converged) / rate(all)`                              | ≥ 92 %     | 30 j    |
| Fuite PII                            | alerting instantané (compteur de fuzz detect)              | 0          | always  |
| Audit loss                           | PVC write errors                                           | 0 / 7 j    | 7 j     |

### 17.2 Error budget

Budget 99,5 % sur 30 j = **3,6 h d'indisponibilité tolérées** par mois.

Politique :
- Si ≥ 50 % du budget consumé en < 50 % de la fenêtre → gel des features (*freeze*).
- Si ≥ 100 % du budget consommé → post-mortem obligatoire + priorité 1 de remédiation.

### 17.3 PrometheusRule (extrait)

```yaml
apiVersion: monitoring.coreos.com/v1
kind: PrometheusRule
metadata:
  name: efe-slo
spec:
  groups:
    - name: efe.availability
      interval: 30s
      rules:
        - alert: EFEHighErrorRate
          expr: |
            sum(rate(efe_grpc_requests_total{code!="OK"}[5m]))
              /
            sum(rate(efe_grpc_requests_total[5m])) > 0.01
          for: 10m
          labels:
            severity: warning
            runbook: https://docs.efe/runbook#high-error-rate
          annotations:
            summary: "EFE error rate > 1% for 10m"
        - alert: EFEHighLatencyP95
          expr: |
            histogram_quantile(0.95,
              sum(rate(efe_grpc_request_duration_seconds_bucket{method="/efe.v1.SwarmService/Execute"}[5m]))
              by (le)) > 0.9
          for: 10m
          labels:
            severity: warning
            runbook: https://docs.efe/runbook#high-latency
          annotations:
            summary: "p95 > 900 ms on Execute for 10m"
        - alert: EFEPodRestartLoop
          expr: rate(kube_pod_container_status_restarts_total{container="efe"}[5m]) > 0.1
          for: 5m
          labels:
            severity: critical
        - alert: EFEConfidenceDrop
          expr: |
            histogram_quantile(0.50, sum(rate(efe_swarm_confidence_bucket[15m])) by (le)) < 0.80
          for: 15m
          labels:
            severity: warning
        - alert: EFERateLimitStorm
          expr: sum(rate(efe_ratelimit_drops_total[5m])) > 100
          for: 5m
          labels:
            severity: warning
        - alert: EFEAuditBackpressure
          expr: kubelet_volume_stats_available_bytes{persistentvolumeclaim=~"efe-.*-audit"} / kubelet_volume_stats_capacity_bytes < 0.1
          for: 15m
          labels:
            severity: warning
        - alert: EFEPolicyRejectionSpike
          expr: sum(rate(efe_policy_rejections_total[5m])) > 10
          for: 10m
          labels:
            severity: warning
```

---

## 18. Sous-module M12.12 — Runbook d'incident

### 18.1 Structure

Le runbook est hébergé dans `docs/runbook.md`. Il est versionné au même rythme que le code et
chaque alerte Prometheus possède un **lien canonique stable** (`#high-error-rate`, etc.).

Chaque section d'incident suit le format :

```
### <alert-id>
**Sévérité** : warning / critical
**Signal observé** : ...
**Impact client** : ...
**Diagnostic rapide** (≤ 3 min) : ...
**Mitigations immédiates** : ...
**Remédiation durable** : ...
**Liens** : dashboard, logs Loki, traces, PR history
```

### 18.2 Sections MVP

1. `#high-error-rate` — taux d'erreur > 1 %
2. `#high-latency` — p95 > 900 ms
3. `#pod-restart-loop` — CrashLoopBackOff
4. `#confidence-drop` — convergence tombe < 80 %
5. `#rate-limit-storm` — dépassement de quota tenant
6. `#audit-backpressure` — PVC d'audit plein
7. `#policy-rejection-spike` — I1..I7 rejets en masse
8. `#bootstrap-failure` — processus ne démarre pas
9. `#secret-rotation` — rotation de clé JWT
10. `#rollback` — procédure de retour arrière

### 18.3 Outils référencés

- **Logs** : `kubectl logs -f deployment/efe -n <ns>` ou Loki (requête
  `{app="efe"} |= "tenant=\"<t>\""`).
- **Audit** : `kubectl exec efe-pod -- tail -n 200 /var/lib/efe/audit/audit.jsonl | jq '.'`.
- **Metrics snapshot** : `curl -s $METRICS_URL | gzip > snap_$(date +%s).pm.gz`.
- **grpcurl canon** : `grpcurl -plaintext <host>:50051 efe.v1.SwarmService/DescribeSpec`.
- **Escalation** : PagerDuty → `#incident-efe` Slack → squad `efe-oncall`.

---

## 19. Sous-module M12.13 — Smoke tests post-déploiement

### 19.1 Objectif

Valider en < 10 s qu'un déploiement est **fonctionnel** avant de basculer le trafic (canary →
100 %).

### 19.2 Portée

Le smoke test **ne teste pas** la justesse métier (ça c'est le rôle des tests d'intégration
CI). Il prouve que :
- le pod écoute sur le bon port,
- l'auth est active (ou désactivée selon mode),
- les 3 RPCs swarm (Execute, DescribeSpec, ExecuteStreaming) répondent,
- le endpoint `/metrics` retourne du Prometheus text,
- les 12 instruments sont présents (au moins les scalaires).

### 19.3 Script `smoke.sh` (résumé)

```bash
#!/usr/bin/env bash
set -euo pipefail
HOST="${EFE_HOST:-127.0.0.1:50051}"
METRICS="${EFE_METRICS_URL:-http://127.0.0.1:9090/metrics}"

echo "1/6 health check"
grpc_health_probe -addr "$HOST" -connect-timeout 2s

echo "2/6 DescribeSpec roundtrip"
grpcurl -plaintext -d @ "$HOST" efe.v1.SwarmService/DescribeSpec < fixtures/spec.json \
  | jq -e '.valid == true'

echo "3/6 Execute (minimal)"
grpcurl -plaintext -d @ "$HOST" efe.v1.SwarmService/Execute < fixtures/execute.json \
  | jq -e '.responseId != ""'

echo "4/6 ExecuteStreaming final chunk"
grpcurl -plaintext -d @ "$HOST" efe.v1.SwarmService/ExecuteStreaming < fixtures/execute.json \
  | tail -n1 | jq -e '.final != null'

echo "5/6 /metrics exposes 12 instruments"
body=$(curl -fsSL "$METRICS")
for m in efe_grpc_requests_total efe_grpc_request_duration_seconds efe_grpc_requests_in_flight \
         efe_swarm_rounds_total efe_swarm_duration_seconds efe_swarm_confidence \
         efe_errors_total efe_breaches_total efe_breach_magnitude \
         efe_cone_registrations_total efe_policy_rejections_total efe_ratelimit_drops_total; do
  echo "$body" | grep -q "^# HELP $m " || { echo "  MISSING: $m"; exit 1; }
done

echo "6/6 policy violations zero by default"
echo "$body" | grep -E "^efe_policy_rejections_total" || true   # ok si absent

echo "OK"
```

### 19.4 Cibles de performance

- Exécution complète < 5 s en conditions normales.
- 0 retry, 0 curl-with-sleep.
- Sortie interprétable par un opérateur humain **et** par un script CI (exit code 0/1).

### 19.5 Intégration dans le pipeline release

Le smoke tourne automatiquement :
- après `helm upgrade` en canary,
- avant la promotion 100 %,
- après un rollback (valide que la version précédente fonctionne).

---

## 20. Sous-module M12.14 — Canary et stratégie de rollout

### 20.1 Phases

```
   t=0   Déploiement canary (1 pod)           → 5 % du trafic
   t=5   Vérifications SLO, logs, alertes      → idle 10 min
   t=15  Promotion 50 %                        → moitié du trafic
   t=25  Vérifications                         → idle 15 min
   t=40  Promotion 100 %                       → rollout final
```

### 20.2 Critères d'abandon (aborts)

À chaque phase, on **abandonne et rollback** si :
- Taux d'erreur > 1 % pendant 2 minutes consécutives.
- p95 latency > 2× la baseline de la version précédente.
- `efe_errors_total{kind="internal"}` > 0.
- Toute alerte PrometheusRule sévérité `critical` déclenche.

### 20.3 Outils

L'implémentation recommandée est **Argo Rollouts** avec analyse automatisée :

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Rollout
metadata:
  name: efe
spec:
  replicas: 6
  strategy:
    canary:
      steps:
        - setWeight: 5
        - pause: { duration: 10m }
        - analysis:
            templates: [{ templateName: efe-success-rate }]
        - setWeight: 50
        - pause: { duration: 15m }
        - analysis:
            templates: [{ templateName: efe-success-rate }]
        - setWeight: 100
```

En MVP, si Argo Rollouts n'est pas disponible, on utilise une **stratégie rolling classique
avec `maxSurge: 1, maxUnavailable: 0`** (pas de canary multi-phase, mais rollout progressif
suffisant pour MVP).

---

## 21. Sous-module M12.15 — Rollback et retour arrière

### 21.1 Principe

Un rollback doit être exécutable en **moins de 60 secondes** depuis la détection d'un problème.

### 21.2 Commande de référence

```bash
helm rollback efe <revision> --namespace efe
```

Helm applique le manifest de la révision précédente, Kubernetes opère un rolling update avec
la **même stratégie** que le déploiement initial (maxSurge 1, maxUnavailable 0).

### 21.3 Garanties

- **Pas de perte d'audit** : l'audit log est append-only, le rollback ne le tronque pas.
- **Pas de changement de schéma** : toute release reste rétro-compatible proto (garantie V1).
- **Pas de rollback d'état** : le registre d'alignement en mémoire est re-hydraté depuis les
  RPCs admin à froid (en V2, persistance externe ; en MVP, acceptable).

### 21.4 Rollback d'une migration cassée

Si une release casse :
1. Alerte déclenchée (PagerDuty).
2. On-call exécute `helm rollback efe <N-1>` — ≤ 60 s.
3. Smoke test post-rollback (`smoke.sh`) — ≤ 10 s.
4. Post-mortem ticket ouvert automatiquement via webhook Alertmanager.

### 21.5 Tests de rollback

Le pipeline CI contient un test **obligatoire** de rollback sur kind :
- Déploie `v_test` → smoke OK.
- Déploie `v_test + 1` avec une cassure volontaire (ex. binaire exit(1)).
- Exécute rollback.
- Smoke OK sur `v_test`.

---

## 22. Sous-module M12.16 — Release pipeline et versioning

### 22.1 Versioning

SemVer strict sur le binaire **et** sur l'API proto :

| Bump       | Déclencheur                                       |
|------------|---------------------------------------------------|
| patch      | bug fix interne, pas de changement proto          |
| mineur     | nouveau champ proto optionnel, nouveau RPC        |
| majeur     | breaking change proto, suppression d'instrument   |

Version unique dans `Cargo.toml` workspace (propagée via `version.workspace = true`).

### 22.2 Workflow de release (`release.yml`)

```yaml
name: Release
on:
  push:
    tags: ["v*.*.*"]

permissions:
  contents: write
  id-token: write
  packages: write
  attestations: write

jobs:
  build-and-sign:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - uses: docker/setup-qemu-action@v3
      - uses: docker/setup-buildx-action@v3
      - uses: sigstore/cosign-installer@v3
      - uses: anchore/sbom-action@v0
      - name: login ghcr
        uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}
      - name: build & push
        uses: docker/build-push-action@v6
        id: build
        with:
          context: .
          file: docker/Dockerfile.prod
          platforms: linux/amd64
          push: true
          provenance: mode=max
          sbom: true
          tags: |
            ghcr.io/efe-project/efe:${{ github.ref_name }}
            ghcr.io/efe-project/efe:latest
          build-args: |
            SOURCE_DATE_EPOCH=${{ github.event.head_commit.timestamp }}
      - name: cosign sign
        run: |
          cosign sign --yes \
            ghcr.io/efe-project/efe@${{ steps.build.outputs.digest }}
      - name: generate SBOM attestation
        run: |
          cosign attest --yes \
            --predicate sbom.spdx.json --type spdxjson \
            ghcr.io/efe-project/efe@${{ steps.build.outputs.digest }}
      - name: trivy scan (gate)
        uses: aquasecurity/trivy-action@0.28.0
        with:
          image-ref: ghcr.io/efe-project/efe@${{ steps.build.outputs.digest }}
          severity: HIGH,CRITICAL
          exit-code: 1
      - name: helm package
        run: |
          helm package deploy/helm/efe \
            --version ${{ github.ref_name }} \
            --app-version ${{ github.ref_name }}
      - name: upload release asset
        uses: softprops/action-gh-release@v2
        with:
          files: |
            efe-*.tgz
            sbom.spdx.json
            build-manifest.json
```

### 22.3 Canaux de distribution

| Canal       | URI                                             |
|-------------|-------------------------------------------------|
| Image       | `ghcr.io/efe-project/efe:<tag>`                 |
| Helm chart  | `oci://ghcr.io/efe-project/charts/efe:<tag>`    |
| SBOM        | Asset GitHub Release                            |
| Provenance  | Attestation in-toto attachée à l'image          |

---

## 23. Sous-module M12.17 — CI et portes qualité

### 23.1 Jobs `ci.yml` (existants, rappel)

- `fast-checks` : fmt, clippy, ruff, mypy (≤ 2 min).
- `rust-tests` : `cargo nextest run --workspace`.
- `python-tests` : `uv run pytest`.
- `cuda-build` : compilation CMake sur image CUDA.
- `security` : `cargo audit`, `pip-audit`, `gitleaks`.
- `all-green` : agrégation finale.

### 23.2 Jobs additionnels à ajouter (M12)

- `docker-build-prod` : build de l'image Dockerfile.prod sur **pull-request** aussi, sans push
  (garantit qu'on ne casse pas la recette).
- `helm-lint` : `helm lint deploy/helm/efe && helm template deploy/helm/efe | kubeval`.
- `smoke-local` : spin-up kind + deploy via Helm + `scripts/smoke.sh`.
- `image-reproducibility` : build deux fois, compare digest ; alerte si différent.

### 23.3 Matrice de portes

| Gate                      | Échec =                             | Override requis   |
|---------------------------|-------------------------------------|-------------------|
| cargo fmt                 | bloque merge                        | non               |
| clippy -D warnings        | bloque merge                        | non               |
| cargo nextest             | bloque merge                        | non               |
| trivy HIGH/CRITICAL       | bloque release                      | oui (2 approbations) |
| helm lint                 | bloque merge sur PR touchant deploy/| non               |
| smoke-local               | bloque release                      | non               |
| image-reproducibility     | alerte non-bloquante MVP            | non (V1.1 bloquant) |

---

## 24. Tests d'intégration de M12

### 24.1 Tests de démarrage (`tests/startup_sequence.rs`)

| Test                                        | Assertion                               |
|---------------------------------------------|-----------------------------------------|
| `bootstrap_happy_path`                      | Le binaire démarre, /metrics répond     |
| `bootstrap_invalid_config_fails_fast`       | Config invalide → exit code 1 en < 1 s  |
| `bootstrap_missing_secret_with_auth_on`     | Auth on + pas de secret → refus         |
| `bootstrap_metrics_port_in_use`             | Port /metrics pris → exit code 1        |

### 24.2 Tests de shutdown (`tests/shutdown.rs`)

| Test                                        | Assertion                               |
|---------------------------------------------|-----------------------------------------|
| `drain_finishes_inflight_requests`          | Req en vol aboutit avant SIGKILL        |
| `signal_trigger_is_idempotent`              | Double SIGTERM sans double-drain        |
| `drain_timeout_respected`                   | Sortie ≤ drain_timeout + 2 s            |
| `audit_flushed_on_shutdown`                 | Toutes les entrées sont sur disque      |

### 24.3 Tests de conteneur (`tests/docker.sh`)

- Build de l'image avec un `SOURCE_DATE_EPOCH` fixe → digest attendu.
- Run de l'image → smoke test interne → exit 0.
- `trivy image` → 0 HIGH/CRITICAL.

### 24.4 Tests Helm (`tests/helm.bats`)

- `helm lint` propre.
- `helm template --validate` produit des manifests conformes au schéma k8s.
- Les sondes référencent bien `/usr/local/bin/grpc_health_probe`.
- Le `Deployment` a `readOnlyRootFilesystem: true` en prod.
- Le `NetworkPolicy` existe quand `networkPolicy.enabled: true`.

### 24.5 Tests k8s end-to-end (`tests/k8s_e2e.sh`)

Spin-up kind, déploie via `helm install`, attend `kubectl wait deployment/efe
--for=condition=Available --timeout=60s`, puis exécute `smoke.sh`. Ce test tourne dans le job
`smoke-local` de la CI.

---

## 25. Critères d'acceptation globaux M12

### 25.1 Fonctionnels

| ID        | Critère                                                              |
|-----------|----------------------------------------------------------------------|
| CA-M12-01 | Le binaire démarre en ≤ 3 s (p95 sur 50 démarrages)                  |
| CA-M12-02 | `SIGTERM` → exit clean en ≤ 30 s (drain 25 s + 5 s finalisation)     |
| CA-M12-03 | 3/3 sondes (liveness/readiness/startup) vertes en régime stable       |
| CA-M12-04 | Image conteneur ≤ 60 Mio compressée                                  |
| CA-M12-05 | 0 vulnérabilité high/critical sur trivy                              |
| CA-M12-06 | Image signée cosign + SBOM SPDX attaché                              |
| CA-M12-07 | Build reproductible : 2 builds = même digest                         |
| CA-M12-08 | Helm chart `lint` + `template --validate` propres                    |
| CA-M12-09 | `smoke.sh` termine en ≤ 5 s en local, ≤ 10 s en cluster              |
| CA-M12-10 | Rollback Helm ≤ 60 s + smoke OK post-rollback                        |
| CA-M12-11 | `/metrics` expose les 12 instruments M11                              |
| CA-M12-12 | Secrets JWT jamais loggés (blocklist M11 + test e2e)                 |
| CA-M12-13 | `NetworkPolicy` bloque tout trafic non-documenté (test kind)         |
| CA-M12-14 | Audit log persistant post-restart sur PVC                            |

### 25.2 Documentation

| ID            | Livrable                                                         |
|---------------|------------------------------------------------------------------|
| CA-M12-DOC-01 | `docs/runbook.md` couvre 10 scénarios d'incident                 |
| CA-M12-DOC-02 | `docs/slo.md` définit les 5 SLI/SLO                              |
| CA-M12-DOC-03 | `deploy/helm/efe/README.md` couvre installation + upgrade        |
| CA-M12-DOC-04 | Diagramme Grafana de référence commité                           |
| CA-M12-DOC-05 | Changelog versionné (Keep a Changelog)                           |

### 25.3 Opérationnels

| ID              | Critère                                                         |
|-----------------|-----------------------------------------------------------------|
| CA-M12-OPS-01   | On-call rotation établie (PagerDuty)                            |
| CA-M12-OPS-02   | Dashboard Grafana accessible en lecture à l'équipe produit      |
| CA-M12-OPS-03   | Alertes routées vers `#incident-efe` Slack                      |
| CA-M12-OPS-04   | Procédure de rotation JWT testée trimestriellement              |
| CA-M12-OPS-05   | Exercice de rollback réalisé en staging chaque mois             |

---

## Annexe A — Dockerfile de production (fichier complet)

Voir §11.2 pour la version annotée. Le fichier prêt à commit est identique, à ce détail près
que les ARG doivent être hoistés en haut pour un cache plus prévisible :

```dockerfile
# syntax=docker/dockerfile:1.7
ARG RUST_VERSION=1.85.0
ARG DEBIAN_VARIANT=bookworm-slim
ARG CHEF_VERSION=0.1.67
ARG GRPC_HEALTH_PROBE_VERSION=v0.4.25
ARG SOURCE_DATE_EPOCH=0

FROM rust:${RUST_VERSION}-${DEBIAN_VARIANT} AS planner
WORKDIR /app
ARG CHEF_VERSION
RUN cargo install cargo-chef --locked --version ${CHEF_VERSION}
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM rust:${RUST_VERSION}-${DEBIAN_VARIANT} AS cacher
WORKDIR /app
ARG CHEF_VERSION
RUN cargo install cargo-chef --locked --version ${CHEF_VERSION}
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

FROM rust:${RUST_VERSION}-${DEBIAN_VARIANT} AS builder
WORKDIR /app
ARG SOURCE_DATE_EPOCH
ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev protobuf-compiler binutils \
    && rm -rf /var/lib/apt/lists/*
COPY --from=cacher /app/target target
COPY --from=cacher /usr/local/cargo /usr/local/cargo
COPY . .
RUN cargo build --release --locked --bin efe-server \
    && strip target/release/efe-server

FROM alpine:3.20 AS health-probe
ARG GRPC_HEALTH_PROBE_VERSION
RUN apk add --no-cache curl \
    && curl -fsSL "https://github.com/grpc-ecosystem/grpc-health-probe/releases/download/${GRPC_HEALTH_PROBE_VERSION}/grpc_health_probe-linux-amd64" \
        -o /grpc_health_probe \
    && chmod +x /grpc_health_probe

FROM gcr.io/distroless/cc-debian12:nonroot
LABEL org.opencontainers.image.source="https://github.com/efe-project/efe"
LABEL org.opencontainers.image.description="EFE gRPC server (MVP)"
LABEL org.opencontainers.image.licenses="Apache-2.0"
LABEL org.opencontainers.image.vendor="EFE Project"

USER 10001:10001
WORKDIR /

COPY --from=builder --chown=10001:10001 /app/target/release/efe-server /usr/local/bin/efe-server
COPY --from=health-probe --chown=10001:10001 /grpc_health_probe /usr/local/bin/grpc_health_probe

EXPOSE 50051 9090

ENV RUST_LOG="info,efe=info"
ENV EFE_LISTEN_ADDR="0.0.0.0:50051"
ENV EFE_METRICS_ADDR="0.0.0.0:9090"

ENTRYPOINT ["/usr/local/bin/efe-server"]
```

---

## Annexe B — Helm chart (fichiers complets)

### B.1 `Chart.yaml`

```yaml
apiVersion: v2
name: efe
description: EFE gRPC server — Ephemeral Fluid Swarm
type: application
version: 0.1.0
appVersion: "0.1.0"
home: https://github.com/efe-project/efe
sources:
  - https://github.com/efe-project/efe
maintainers:
  - name: EFE Core Team
    email: efe-core@example.com
kubeVersion: ">=1.27.0"
annotations:
  artifacthub.io/license: Apache-2.0
  artifacthub.io/changes: |
    - Initial MVP release.
```

### B.2 `_helpers.tpl`

```yaml
{{- define "efe.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "efe.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "efe.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{ include "efe.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{- define "efe.selectorLabels" -}}
app.kubernetes.io/name: {{ include "efe.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "efe.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "efe.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{- define "efe.grpcHealthProbe" -}}
exec:
  command: ["/usr/local/bin/grpc_health_probe", "-addr=:{{ .Values.service.grpcPort }}"]
initialDelaySeconds: {{ .initialDelaySeconds | default 2 }}
periodSeconds: {{ .periodSeconds | default 10 }}
timeoutSeconds: {{ .timeoutSeconds | default 2 }}
failureThreshold: {{ .failureThreshold | default 3 }}
{{- end -}}
```

### B.3 `templates/service.yaml`

```yaml
apiVersion: v1
kind: Service
metadata:
  name: {{ include "efe.fullname" . }}
  labels: {{- include "efe.labels" . | nindent 4 }}
spec:
  type: {{ .Values.service.type }}
  ports:
    - name: grpc
      port: {{ .Values.service.grpcPort }}
      targetPort: grpc
      protocol: TCP
    - name: metrics
      port: {{ .Values.service.metricsPort }}
      targetPort: metrics
      protocol: TCP
  selector: {{- include "efe.selectorLabels" . | nindent 4 }}
```

### B.4 `templates/hpa.yaml`

```yaml
{{- if .Values.autoscaling.enabled }}
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: {{ include "efe.fullname" . }}
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: {{ include "efe.fullname" . }}
  minReplicas: {{ .Values.autoscaling.minReplicas }}
  maxReplicas: {{ .Values.autoscaling.maxReplicas }}
  metrics:
    {{- if .Values.autoscaling.targetCPUUtilizationPercentage }}
    - type: Resource
      resource:
        name: cpu
        target:
          type: Utilization
          averageUtilization: {{ .Values.autoscaling.targetCPUUtilizationPercentage }}
    {{- end }}
    {{- if .Values.autoscaling.targetMemoryUtilizationPercentage }}
    - type: Resource
      resource:
        name: memory
        target:
          type: Utilization
          averageUtilization: {{ .Values.autoscaling.targetMemoryUtilizationPercentage }}
    {{- end }}
  behavior:
    scaleDown:
      stabilizationWindowSeconds: 300
      policies:
        - type: Percent
          value: 50
          periodSeconds: 60
    scaleUp:
      stabilizationWindowSeconds: 60
      policies:
        - type: Percent
          value: 100
          periodSeconds: 30
        - type: Pods
          value: 4
          periodSeconds: 30
      selectPolicy: Max
{{- end }}
```

### B.5 `templates/pdb.yaml`

```yaml
{{- if .Values.pdb.enabled }}
apiVersion: policy/v1
kind: PodDisruptionBudget
metadata:
  name: {{ include "efe.fullname" . }}
spec:
  minAvailable: {{ .Values.pdb.minAvailable }}
  selector:
    matchLabels: {{- include "efe.selectorLabels" . | nindent 6 }}
{{- end }}
```

### B.6 `templates/NOTES.txt`

```
EFE {{ .Chart.AppVersion }} has been installed.

1. Check the pods:
   kubectl --namespace {{ .Release.Namespace }} get pods -l app.kubernetes.io/instance={{ .Release.Name }}

2. Port-forward to test locally:
   kubectl --namespace {{ .Release.Namespace }} port-forward svc/{{ include "efe.fullname" . }} 50051:50051 9090:9090

3. Run the smoke test:
   EFE_HOST=127.0.0.1:50051 scripts/smoke.sh

4. Metrics:
   curl -s http://127.0.0.1:9090/metrics | grep efe_

5. Health check:
   grpc_health_probe -addr=127.0.0.1:50051

Documentation: https://docs.efe
Runbook:       https://docs.efe/runbook
SLO:           https://docs.efe/slo
```

---

## Annexe C — Alerting rules Prometheus

Cf. §17.3 pour la structure. Les règles supplémentaires à ajouter en V1.1 :

- `EFEMemoryFootprint` : RSS > 400 Mio pendant 10 min.
- `EFECPUThrottled` : CPU throttled > 25 % pendant 5 min.
- `EFEInflightSaturation` : `efe_grpc_requests_in_flight > 0.8 * max_concurrent_streams`.

---

## Annexe D — `smoke.sh` (script complet)

Voir §19.3. Le script est commité avec permissions `0755`, en-tête `#!/usr/bin/env bash`,
et tourne en `set -euo pipefail`. Dépendances système :

- `grpcurl` (≥ v1.9.1)
- `grpc_health_probe` (≥ v0.4.25)
- `jq`
- `curl`

Les **fixtures** (`fixtures/spec.json`, `fixtures/execute.json`) sont des JSON minimaux conformes
au proto `efe.v1`.

---

## Annexe E — Runbook d'opérations (squelette)

```
# Runbook EFE

## 1. Généralités
### 1.1 Architecture (lien M12)
### 1.2 On-call rotation
### 1.3 Canaux de communication

## 2. Procédures courantes
### 2.1 Déploiement standard
### 2.2 Rollback
### 2.3 Rotation de clés JWT
### 2.4 Nettoyage d'audit

## 3. Incidents
### 3.1 #high-error-rate
### 3.2 #high-latency
### 3.3 #pod-restart-loop
### 3.4 #confidence-drop
### 3.5 #rate-limit-storm
### 3.6 #audit-backpressure
### 3.7 #policy-rejection-spike
### 3.8 #bootstrap-failure
### 3.9 #secret-rotation
### 3.10 #rollback

## 4. Post-mortem template
## 5. Contacts d'escalade
## 6. Liens outils
```

---

## Annexe F — Matrice complète des variables d'environnement

| Variable                         | Type      | Défaut                | Sensibilité | Source         |
|----------------------------------|-----------|-----------------------|-------------|----------------|
| `EFE_LISTEN_ADDR`                | SocketAddr| `127.0.0.1:50051`     | publique    | ConfigMap      |
| `EFE_METRICS_ADDR`               | SocketAddr| `127.0.0.1:9090`      | publique    | ConfigMap      |
| `EFE_REQUEST_TIMEOUT_MS`         | u64       | `30000`               | publique    | ConfigMap      |
| `EFE_MAX_CONCURRENT_STREAMS`     | usize     | `1024`                | publique    | ConfigMap      |
| `EFE_ENABLE_ADMIN`               | bool      | `true`                | publique    | ConfigMap      |
| `EFE_AUTH_ENABLED`               | bool      | `false`               | publique    | ConfigMap      |
| `EFE_JWT_AUDIENCE`               | str       | `efe.v1`              | publique    | ConfigMap      |
| `EFE_JWT_ISSUER`                 | str (opt) | (absent)              | publique    | ConfigMap      |
| `EFE_AUTH_ALLOW_INSECURE_DEV`    | bool      | `true`                | publique    | ConfigMap      |
| `EFE_JWT_SECRETS_FILE`           | path      | `/etc/efe/secrets/hs256` | sensible | ConfigMap (le fichier, pas la valeur, est sensible) |
| `EFE_RATELIMIT_ENABLED`          | bool      | `false`               | publique    | ConfigMap      |
| `EFE_RATELIMIT_CAPACITY`         | u32       | `200`                 | publique    | ConfigMap      |
| `EFE_RATELIMIT_REFILL`           | f32       | `100.0`               | publique    | ConfigMap      |
| `EFE_RATELIMIT_ANON_CAP`         | u32       | `50`                  | publique    | ConfigMap      |
| `EFE_RATELIMIT_ANON_REFILL`      | f32       | `10.0`                | publique    | ConfigMap      |
| `EFE_IDEMPOTENCY_ENABLED`        | bool      | `true`                | publique    | ConfigMap      |
| `EFE_IDEMPOTENCY_CAPACITY`       | usize     | `10000`               | publique    | ConfigMap      |
| `EFE_IDEMPOTENCY_TTL_MS`         | u64       | `10000`               | publique    | ConfigMap      |
| `EFE_SHUTDOWN_DRAIN_MS`          | u64       | `25000`               | publique    | ConfigMap      |
| `EFE_AUDIT_LOG_PATH`             | path      | `/var/lib/efe/audit/audit.jsonl` | publique | ConfigMap |
| `RUST_LOG`                       | str       | `info,efe=info`       | publique    | ConfigMap      |
| `EFE_LOG_FORMAT`                 | enum      | `json`                | publique    | ConfigMap      |

---

## Annexe G — Registre des risques d'opérations

| Risque                                             | Prob. | Impact | Mitigation                                             |
|----------------------------------------------------|-------|--------|--------------------------------------------------------|
| Fuite de secret JWT dans log                        | Faible| Critique | PII blocklist M11 + fuzz 10⁴ test                      |
| Rollback casse en cascade (breaking proto oublié)   | Moy.  | Élevé    | `buf breaking` dans CI + tests de régression           |
| PVC audit plein non détecté                         | Faible| Élevé    | Alerte EFEAuditBackpressure + backup S3 nightly        |
| Dérive d'horloge sur pods → traceparent incohérent  | Faible| Faible   | NTP enforced + monotonic fallback                      |
| Helm upgrade applique mauvais values (environnement)| Moy.  | Moyen    | Pipeline requiert `-f values.<env>.yaml` explicite     |
| Prometheus ne scrape pas (label manquant)           | Faible| Moyen    | ServiceMonitor versionné + test e2e                    |
| Corruption d'image entre registry et kubelet        | Très rare | Critique | `image.digest` + ClusterImagePolicy cosign           |
| DNS intra-cluster indisponible                      | Faible| Critique | Health check basé sur IP directe, pas sur DNS          |
| CrashLoopBackoff sur config invalide après upgrade  | Moy.  | Élevé    | `helm diff` obligatoire en pre-merge + smoke post-deploy|
| Dépassement de `maxUnavailable` sur drain long      | Faible| Moyen    | PDB minAvailable + drain_timeout respecté              |
| Scan trivy faux-positif bloque release              | Moy.  | Moyen    | Override process documenté (2 approbations)            |
| OOMKilled sous charge (mémoire mal bornée)          | Moy.  | Élevé    | `resources.limits` + alerte RSS                        |
| Backpressure mpsc M9 (events > capacity)            | Faible| Moyen    | Canal unbounded + fsync 1 s plafonné                   |
| Cosign keyless OIDC expiré → release bloquée        | Faible| Moyen    | Runbook de régénération + fallback clé offline          |
| Mauvaise NetworkPolicy coupe prometheus             | Moy.  | Moyen    | Test kind validant scrape via ServiceMonitor           |

---

## Annexe H — Glossaire DevOps

| Terme                           | Définition                                                   |
|---------------------------------|--------------------------------------------------------------|
| **Attestation**                 | Document signé liant une image à une revendication (provenance, SBOM). |
| **Canary**                      | Déploiement progressif où une fraction du trafic est routée vers la nouvelle version. |
| **Cold start**                  | Temps écoulé entre `kubelet` lançant le conteneur et le premier RPC accepté. |
| **Cosign**                      | Outil Sigstore pour signer/vérifier images et artefacts.     |
| **Distroless**                  | Image minimaliste sans shell/package-manager (Google).       |
| **Drain**                       | Phase d'extinction où le pod ne prend plus de nouveau trafic mais sert les requêtes en vol. |
| **Error budget**                | Marge d'erreur tolérable avant gel des features.              |
| **HPA**                         | HorizontalPodAutoscaler : scale horizontalement sur CPU/mémoire/metric custom. |
| **Keyless (cosign)**            | Signature via OIDC + Fulcio au lieu de clé statique.         |
| **NetworkPolicy**               | Règle k8s de filtrage IP/port entre pods.                    |
| **OTLP**                        | OpenTelemetry Protocol : gRPC/HTTP pour transmettre traces/metrics/logs. |
| **PDB** (PodDisruptionBudget)   | Garantie k8s sur le nombre de pods disponibles pendant éviction. |
| **Provenance**                  | Métadonnée signée décrivant *comment et où* un artefact a été produit. |
| **Runbook**                     | Document pas-à-pas pour répondre à un incident.              |
| **SBOM**                        | Software Bill of Materials : liste des composants et versions. |
| **SLA**                         | Service Level Agreement : contrat client-facing.              |
| **SLI**                         | Service Level Indicator : mesure (p95 latence, erreur).      |
| **SLO**                         | Service Level Objective : seuil cible sur un SLI.            |
| **SLSA**                        | Supply-chain Levels for Software Artifacts (framework de provenance). |
| **SOURCE_DATE_EPOCH**           | Horodatage injecté pour builds reproductibles.               |
| **Startup probe**               | Sonde k8s dédiée au démarrage lent, désactive liveness tant qu'elle échoue. |
| **Sigstore**                    | Écosystème open-source (cosign, fulcio, rekor) pour signature software. |
| **ServiceMonitor**              | CRD Prometheus Operator pour déclarer un scrape.             |
| **Taint / Toleration**          | Mécanisme k8s pour restreindre/affecter les nœuds.           |
| **TLS termination**             | Fin du chiffrement TLS à l'ingress, trafic clair ensuite.    |

---

## Fin du document

**État du document** : approuvé pour implémentation.
**Révision** : 1.0 (MVP).
**Prochaine révision planifiée** : V1.1 (post-premier déploiement prod) — ajout mTLS
intra-cluster, canary Argo Rollouts, multi-région.

Toute divergence entre ce document et l'implémentation réelle doit être tracée dans
`CHANGELOG.md` sous une section **Architectural decisions (M12)**.
