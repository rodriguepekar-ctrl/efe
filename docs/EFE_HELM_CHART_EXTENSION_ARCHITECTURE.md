# `deploy/helm/efe/` — Helm chart EFE — Architecture détaillée de l'extension M12

> **Statut** : Draft v1.0.0 — Document de référence pour les **4
> sprints d'extension** du chart Helm post-D18 :
> **D21 (Φ-A) + D22 (Φ-B) + D27 (Φ-C) + D40 (Φ-D)**.
> **PAS greenfield** : le chart existe déjà au path `deploy/helm/efe/`
> avec un statut **✅ + 🔧 (post-D18-A11 dedup)** — D21 part d'un AS-IS
> mature couvrant replicas + leader election + PDB + HPA + topology
> spread + audit signing + backup + OIDC.
> **Auteur** : rodrigue.pekar@icloud.com
> **Date** : 2026-04-30.
>
> **Dépendances doc** :
>   * `docs/PRODUCT_COMPLETION_ARCHITECTURE.md` §6.4 — sprints
>     D21/D22/D27/D40 dans leurs phases respectives, et §8 —
>     surface chart Helm cible.
>   * `LETTER_E_ARCHITECTURE.md` — service backend mTLS+JWT
>     consommé par le chart à partir de D22.
>   * `docs/EFE_GO_SDK_EXTENSION_ARCHITECTURE.md` (M9) — référence
>     SDK Go consommateur de la mTLS configurée par D22.
>   * `docs/EFE_PYTHON_SDK_EXTENSION_ARCHITECTURE.md` §22 — bundle
>     reload mechanism (consommateur du Job D27).
>   * `bundles/all-MiniLM-L6-v2-en-1.0.0/manifest.json` — sample
>     bundle utilisé par le live reload (D27) + DR drill (D40).
>
> **Portée** : **4 sprints d'extension non-contigus** étalés sur ≈
> 90 jours wallclock totaux (cumulés D21+D22+D27+D40 = 4×3-5 jours
> par sprint). Le chart EXISTE déjà ; on étend la surface avec **6
> nouveaux templates** + **3 templates étendus** + **4 nouvelles
> sections values.yaml** + **1 fichier de validation Polaris** + **2
> CI workflows nouveaux** (helm-test sur PR + Polaris policy gate).
>
> **Cible GA** : `deploy/helm/efe/` chart `v1.0.0` taggé +
> publishable au registry OCI (Helm 3.8+) + via `helm install
> oci://ghcr.io/efe-ai/charts/efe --version 1.0.0` ; aligné semver
> avec le produit (D40 = v0.7.0+ privé, GA v1.0.0 public).

---

## Table des matières

§0. Méta, glossaire, conventions
§1. Mission de l'extension M12 (multi-sprint Φ-A → Φ-D)
§2. Position dans la topologie EFE post-D18
§3. État AS-IS post-D18 (chart mature + audit fix A11)
§4. Définition opérationnelle de M12 v1.0 GA
§5. Principes directeurs (idiomes Helm + Kubernetes)
§6. Invariants structurels (chart layout post-D18 + extensions M12)
§7. Vue d'ensemble — 4 sprints D21/D22/D27/D40
§8. Surface : extension `values.yaml`
§9. **Sprint D21 (Φ-A)** — NetworkPolicy strict + Polaris validation GREEN
§10. **Sprint D22 (Φ-B)** — mTLS via cert-manager
§11. **Sprint D27 (Φ-C)** — Live bundle reload Job (Helm hook)
§12. **Sprint D40 (Φ-D)** — DR drill quarterly CronJob
§13. Stratégie de tests (helm lint + helm template + helm-unittest + Polaris + ct)
§14. Compatibilité Kubernetes versions (1.26+ matrix)
§15. Compatibilité cert-manager versions (1.13+ matrix)
§16. Compatibilité Helm versions (3.13+)
§17. SLO mapping (deployment time + NetworkPolicy enforcement + mTLS rotation + bundle reload + DR RTO/RPO)
§18. Risques transverses + mitigations
§19. Volumétrie code attendue
§20. Plan d'implémentation par sprint
§21. Versioning + publication Helm chart (OCI registry)
§22. Compliance + sécurité (Pod Security Admission, RBAC minimal, audit logging)
§23. Annexes — `values.yaml` canonical (post-M12)
§24. Annexes — liste complète des templates (post-M12)
§25. Annexes — Polaris config + helm-unittest fixtures
§26. Annexes — RACI + budget + Gantt par sprint
§27. Mises à jour du document

---

## §0. Méta, glossaire, conventions

### §0.1 Conventions

* **AS-IS (post-D18)** = état du chart à fin D18 (post-audit A11
  dedup), couvrant les capabilities listées dans le statut user :
  * `replicas`           (`deployment.yaml` `.spec.replicas`)
  * `leader election`    (coordinator pod via lease + RBAC)
  * `PDB`                (`poddisruptionbudget.yaml`)
  * `HPA`                (`hpa.yaml` autoscaling/v2)
  * `topology spread`    (`deployment.yaml` `.spec.topologySpreadConstraints`)
  * `audit signing`      (Ed25519 sidecar pour audit log signing)
  * `backup`             (CronJob backup vers S3-compatible)
  * `OIDC`               (auth via Dex/Keycloak/etc., values OIDC)
* **TO-BE (post-D40)** = état cible défini par les 4 sprints
  cumulés (D21 + D22 + D27 + D40).
* Les défauts résiduels suivent **DT-H.x** (Defect Trace, Helm chart).
* Le chart vit sous `deploy/helm/efe/`. Path filesystem immuable
  post-v1.0.0 GA.
* **Templates** = fichiers YAML dans `templates/` qui contiennent
  des directives Go template + `helm.sh/hook` annotations.
* `[ref:fichier:ligne]` = pointeur dans le code source.

### §0.2 Glossaire

| Terme | Définition |
|-------|------------|
| **Helm chart** | Package Kubernetes : `Chart.yaml` + `values.yaml` + `templates/`. Versionné via `Chart.yaml::version` (chart) + `appVersion` (application). Distribué via Helm repository (ChartMuseum, OCI registry, GitHub Pages). |
| **`deploy/helm/efe/`** | Le chart canonical du projet EFE. Statut post-D18 : ✅ + 🔧. M12 = 4 extensions multi-sprints. |
| **Helm hook** | Annotation `helm.sh/hook: <phase>` qui insère un Job/Pod dans le cycle de release (pre-install, post-install, pre-upgrade, post-upgrade, pre-rollback, post-rollback, pre-delete, post-delete, test). M12 utilise post-upgrade pour le live bundle reload (D27). |
| **NetworkPolicy** | Resource `networking.k8s.io/v1` qui définit ingress/egress pod-level. Strict = default-deny + explicit-allow seulement. Implémentation par CNI (Calico, Cilium, etc. — pas tous les CNI le supportent). |
| **Polaris** | Outil Fairwinds qui valide les manifests K8s contre des best-practices (run-as-non-root, no-host-network, resource limits, etc.). `polaris audit` retourne danger/warning/ignore par check. **Polaris GREEN** = aucun `danger`. |
| **cert-manager** | Add-on Kubernetes pour provisionner + auto-renouveler des certificats X.509 via ACME (Let's Encrypt), CA private, Vault, etc. Reconcile les `Certificate` resources → écrit Secrets TLS. |
| **`Issuer` / `ClusterIssuer`** | Resource cert-manager qui définit la source des certificats (ACME, Vault, CA self-signed, etc.). `Issuer` = namespaced ; `ClusterIssuer` = cluster-wide. |
| **`Certificate`** | Resource cert-manager qui déclare un certificat à émettre : DNS names, IP SANs, validity period, renewBefore, secretName cible. |
| **mTLS** | Mutual TLS : le client présente un certificat X.509 au serveur (en plus de la TLS server-side classique). Lettre E (D22) impose mTLS pour le RPC gRPC privé. |
| **Live bundle reload** | Pattern où le serveur EFE recharge un model bundle (poids INT8 + tokenizer + manifest signé) SANS redémarrage du pod. M12 livre un Helm hook Job (D27) qui déclenche le reload via une RPC admin lors du upgrade. |
| **DR drill** | Disaster Recovery exercise : restore d'un backup puis validation que le système restauré fonctionne. M12 livre un CronJob quarterly (D40) qui automatise le drill + alerte sur les échecs. |
| **Bundle hash** | Le `manifest.json` du sample bundle a un `sha256_manifest` pinné. Le live reload Job vérifie le hash AVANT et APRÈS pour confirmer le swap. |
| **Pod Security Admission (PSA)** | Successeur de PodSecurityPolicy (deprecated K8s 1.21, supprimé 1.25). Mode `restricted` impose runAsNonRoot, readOnlyRootFilesystem, etc. M12 (D21) configure les labels namespace pour `restricted`. |
| **Polaris GREEN** | Tous les checks Polaris à severity `danger` ou `warning` passent. Ignored checks = volontairement désactivés. |
| **`helm-unittest`** | Plugin Helm de tests unitaires sur templates. Suite de tests YAML dans `tests/` qui asserte sur les manifests rendus. |
| **`chart-testing` (`ct`)** | CLI Helm chart linter + tester. Utilisé en CI pour validate Chart.yaml + values.yaml + render templates + lint. |
| **OCI registry** | Helm 3.8+ supporte la distribution via OCI registry standard (Docker Hub, GHCR, ECR, Artifactory). M12 ship via `ghcr.io/efe-ai/charts/efe` à partir de v1.0.0 GA. |

### §0.3 Notation

* Les blocs `text-with-tabs` sans surlignage = texte canonique copié
  tel quel dans le `Chart.yaml` / `values.yaml`.
* Les blocs ```yaml = YAML pseudo-code didactique.
* Les blocs ```bash = commandes shell exécutables.
* Tableaux pin tests = id `DT-H.<sprint>.<n>` à matérialiser dans
  `scripts/tests/test_d21_helm_extension.py` (D21), idem pour D22/D27/D40.

---

## §1. Mission de l'extension M12 (multi-sprint Φ-A → Φ-D)

**Phrase canonique** :

> Étendre le chart Helm `deploy/helm/efe/` mature post-D18 avec **4
> capabilities cluster-grade** distribuées sur 4 sprints non-contigus :
> network strict (D21), mTLS automation (D22), live bundle reload
> (D27), DR drill automation (D40) — pour atteindre une **surface
> opérationnelle "production-ready" SOC 2 / ISO 27001 compatible**
> au moment du GA v1.0.0.

**Pourquoi multi-sprint (et pas sprint atomique)** :

1. **D21 (Φ-A)** : NetworkPolicy + Polaris GREEN sont des
   prérequis de la phase Φ-A "tightening". À ce stade, les SDKs
   (M8, M9, M10, M11) sont déjà partiellement livrés ou en train
   d'être livrés ; le chart doit garantir l'isolation réseau pour
   les déploiements multi-tenants. **Sprint court : 3 jours**.

2. **D22 (Φ-B)** : mTLS via cert-manager arrive AVEC le sprint
   Lettre E backend (D22 backend = mTLS server enforcement). Le
   chart doit livrer la pipeline cert-manager AU MÊME MOMENT pour
   que le déploiement post-D22 soit cohérent. **Sprint moyen :
   4 jours** (cert-manager templates + Issuer choices + rotation
   testing).

3. **D27 (Φ-C)** : Live bundle reload arrive AVEC la livraison
   du sample bundle (D26 = build pipeline) — D27 livre le mécanisme
   de hot-swap qui est CONSOMMÉ par les opérateurs SaaS clients.
   **Sprint court : 3 jours**.

4. **D40 (Φ-D)** : DR drill quarterly est un sprint de
   "release-engineering" final, juste avant la GA v1.0.0. Tester
   la chaîne complète backup → restore → validate sur une cadence
   automatique de 3 mois est une exigence SOC 2. **Sprint moyen :
   4 jours**.

**Total wallclock M12** = 3 + 4 + 3 + 4 = **14 jours** distribués
sur 90 jours calendaires (D21 = day 60, D22 = day 70, D27 = day
130, D40 = day 200 environ).

**Non-objectifs** (explicitement OUT de M12) :

* **Service Mesh integration** (Istio, Linkerd, Cilium service mesh)
  — Reporté D45+. Le chart livre des templates "service-mesh-friendly"
  (annotations `traffic.sidecar.istio.io/excludeOutboundIPRanges`,
  etc.) mais ne PROVISIONNE pas le mesh.
* **GitOps native (ArgoCD / Flux Kustomize support)** — Le chart
  reste pur Helm. Les utilisateurs ArgoCD font `helm template`
  upstream et commit le résultat. Pattern documenté ; pas de
  code dans le chart.
* **Multi-cluster federation (KubeFed)** — Out of scope ; opérateur
  fait son propre overlay.
* **HPA custom metrics (Prometheus adapter)** — Le HPA actuel
  utilise CPU/memory standard. Custom metrics reportés D45+.
* **Vertical Pod Autoscaler** — Reporté D45+ (incompat avec le
  HPA actuel).
* **GraalVM native image deploy** — La M11 architecture spec dit
  GraalVM est OUT-of-scope du SDK Java D26 ; idem pour le chart.

---

## §2. Position dans la topologie EFE post-D18

```
┌───────────────────────────────────────────────────────────────────────┐
│              EFE Topology — post-D18 (chart AS-IS)                    │
└───────────────────────────────────────────────────────────────────────┘

  Layer L5 — Operator surface (helm install)
  ┌───────────────────────────────────────────────────────────────────┐
  │  helm install efe deploy/helm/efe/ -f values-prod.yaml -n efe    │
  │     ↓                                                            │
  │  values.yaml resolves :                                          │
  │     - replicas, image.tag, resources                             │
  │     - leader election                                            │
  │     - HPA min/max + utilization target                           │
  │     - PDB minAvailable                                           │
  │     - topology spread (zone + node)                              │
  │     - audit signing key (Secret reference)                       │
  │     - backup S3 endpoint + credentials                           │
  │     - OIDC issuer URL + clientId                                 │
  └───────────┬───────────────────────────────────────────────────────┘
              │ kubectl apply (via helm install)
              ▼
  Layer L4 — Kubernetes resources (templated)
  ┌───────────────────────────────────────────────────────────────────┐
  │  Deployment (replicas, topology spread, leader election sidecar) │
  │  Service                                                         │
  │  PodDisruptionBudget                                             │
  │  HorizontalPodAutoscaler                                         │
  │  ConfigMap (config + bundle path)                                │
  │  Secret (audit signing key + OIDC client secret)                 │
  │  ServiceAccount + Role + RoleBinding (leader election + audit)   │
  │  CronJob (backup → S3-compatible)                                │
  │  Ingress (OIDC-protected)                                        │
  │  ServiceMonitor (Prometheus scrape — optional)                   │
  └───────────────────────────────────────────────────────────────────┘
              │
              ▼
  Layer L3 — Service backend (Lettre E gRPC)
  ┌───────────────────────────────────────────────────────────────────┐
  │  efe-server pods : tcp/50051 (gRPC) ; tcp/8080 (admin) ;          │
  │  tcp/9090 (metrics) ; tcp/9091 (health)                          │
  └───────────────────────────────────────────────────────────────────┘

```

**Où M12 s'insère** :

* **Layer L4 (Kubernetes resources)** — M12 ajoute :
  * `templates/networkpolicy.yaml` ÉTENDU (D21) : strict default-deny + tenant ingress + monitoring egress.
  * `templates/mtls-cert.yaml` NOUVEAU (D22) : Certificate + Issuer/ClusterIssuer.
  * `templates/bundle-reload-hook.yaml` NOUVEAU (D27) : Helm post-upgrade Job.
  * `templates/dr-drill-quarterly.yaml` NOUVEAU (D40) : CronJob quarterly.
* **Layer L3 (server backend)** — Le chart déploie `efe-server`
  qui consomme :
  * Les certificats mTLS (montés depuis le Secret généré par cert-manager).
  * Le bundle reload trigger (admin RPC).
  * Le backup snapshot (volume montage + S3 sync).

**Couplage M12 ↔ AS-IS post-D18** : EXTENSION pure. Aucun template
existant n'est CASSÉ ; tout ce qui est rajouté est gated par un
`{{- if .Values.<feature>.enabled }}` block qui vaut `false` par
défaut → l'upgrade `helm upgrade` depuis la version pre-M12 ne
casse rien.

---

## §3. État AS-IS post-D18 (chart mature + audit fix A11)

### §3.1 Capabilities AS-IS (post-D18 + A11 dedup)

| Capability | Template existant | Sprint d'origine | Statut |
|---|---|---|---|
| `replicas` | `deployment.yaml::spec.replicas` | D2 | ✅ |
| `leader election` | `coordinator.yaml` + `lease-rbac.yaml` | D8 | ✅ |
| `PDB` | `poddisruptionbudget.yaml` | D11 | ✅ |
| `HPA` | `hpa.yaml` (autoscaling/v2) | D11 | ✅ |
| `topology spread` | `deployment.yaml::topologySpreadConstraints` | D13 | ✅ |
| `audit signing` | `audit-signer-sidecar.yaml` + Ed25519 Secret | D15 | ✅ |
| `backup` | `backup-cronjob.yaml` (S3-compat) | D17 | ✅ |
| `OIDC` | `ingress.yaml` + `oidc-secret.yaml` | D18 | ✅ |
| `dedup audit fix A11` | post-D18 audit pass : suppression de 11 doublons across templates | D18-A11 | 🔧 |

### §3.2 Audit fix A11 (post-D18 dedup)

L'audit chirurgical post-D18 a relevé **11 cas de duplication** dans les
templates (e.g., labels `app.kubernetes.io/name` répétés inline au lieu
d'utiliser le helper `{{- include "efe.labels" . | nindent 4 }}`) +
des sélecteurs hardcoded au lieu de `{{- include "efe.selectorLabels" }}`.
A11 a unifié via 4 helpers canoniques dans `_helpers.tpl` :

```
{{- define "efe.name" -}} ... {{- end -}}
{{- define "efe.fullname" -}} ... {{- end -}}
{{- define "efe.labels" -}}
helm.sh/chart: {{ include "efe.chart" . }}
app.kubernetes.io/name: {{ include "efe.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- if .Values.commonLabels }}
{{ toYaml .Values.commonLabels }}
{{- end }}
{{- end -}}

{{- define "efe.selectorLabels" -}}
app.kubernetes.io/name: {{ include "efe.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}
```

M12 réutilise ces helpers (DRY).

### §3.3 Layout AS-IS

```
deploy/helm/efe/
├── Chart.yaml                           ← v0.6.0 post-D18 + A11
├── values.yaml                          ← documented, ~250 lignes
├── values-staging.yaml                  ← overrides staging
├── values-prod.yaml                     ← overrides prod
├── README.md                            ← user-facing doc
├── CHANGELOG.md                         ← per-version notes
├── .helmignore                          ← exclude from package
├── templates/
│   ├── _helpers.tpl                     ← 4 named templates
│   ├── deployment.yaml                  ← 3 replicas + topology spread + sidecars
│   ├── service.yaml                     ← ClusterIP gRPC + admin + metrics
│   ├── ingress.yaml                     ← OIDC-protected (oauth2-proxy)
│   ├── poddisruptionbudget.yaml         ← minAvailable: 1
│   ├── hpa.yaml                         ← autoscaling/v2
│   ├── configmap.yaml                   ← efe-config + bundle path
│   ├── secret.yaml                      ← audit signing key
│   ├── serviceaccount.yaml              ← + Role + RoleBinding
│   ├── coordinator.yaml                 ← leader election sidecar
│   ├── lease-rbac.yaml                  ← Lease coordination resources
│   ├── audit-signer-sidecar.yaml        ← Ed25519 sidecar
│   ├── backup-cronjob.yaml              ← S3-compat backup
│   ├── oidc-secret.yaml                 ← OIDC client secret
│   ├── networkpolicy.yaml               ← BASIC ingress allow (sera étendu D21)
│   ├── servicemonitor.yaml              ← Prometheus scrape (optional)
│   └── tests/                           ← helm test pods
│       └── test-connection.yaml         ← curl /healthz from test pod
└── tests/                               ← helm-unittest fixtures
    └── deployment_test.yaml             ← assert deployment shape
```

### §3.4 `Chart.yaml` AS-IS

```yaml
apiVersion: v2
name: efe
description: EFE (Essaim Fluide Éphémère) — inference platform Helm chart.
type: application
version: 0.6.0          # chart version (semver — bumped on every chart change)
appVersion: "0.3.0"     # app version (semver — bumped on app release ; matches efe-server)
kubeVersion: ">= 1.26.0-0"
home: https://github.com/efe-ai/efe
sources:
  - https://github.com/efe-ai/efe
maintainers:
  - name: Rodrigue Pekar
    email: rodrigue.pekar@icloud.com
icon: https://raw.githubusercontent.com/efe-ai/efe/main/docs/img/efe-logo.png
keywords:
  - efe
  - inference
  - ml
  - alignment
  - swarm
annotations:
  artifacthub.io/changes: |
    - kind: changed
      description: Post-D18 audit A11 — dedup helpers across templates.
```

### §3.5 Vérification AS-IS

```bash
ls deploy/helm/efe/
# ATTENDU : Chart.yaml  values.yaml  values-staging.yaml  values-prod.yaml
#           README.md  CHANGELOG.md  .helmignore  templates/  tests/

helm lint deploy/helm/efe/
# ATTENDU : 0 errors, 0 warnings (post-A11 dedup)

helm template deploy/helm/efe/ -f deploy/helm/efe/values-prod.yaml > /tmp/render.yaml
wc -l /tmp/render.yaml
# ATTENDU : ~ 800-1000 lignes YAML (selon resources activées)
```

---

## §4. Définition opérationnelle de M12 v1.0 GA

### §4.1 Scope fonctionnel cumulé (D21 + D22 + D27 + D40)

M12 est **GA-ready** quand toutes les capabilities suivantes sont
livrées + tests verts + pin tested :

| Capability (post-M12) | Sprint | Surface |
|---|---|---|
| **NetworkPolicy strict** | D21 | `networkpolicy.yaml` étendu : default-deny + tenant ingress (label-selector based) + egress vers monitoring (Prometheus + audit aggregator) only. |
| **Polaris validation GREEN** | D21 | `.polaris.yaml` config + CI gate `polaris audit --severity danger --output-format pretty` retourne 0 dangers. |
| **Pod Security Admission `restricted`** | D21 | Namespace label `pod-security.kubernetes.io/enforce: restricted` recommandé via README + values default `podSecurity.enforce: restricted`. |
| **mTLS Certificate (cert-manager)** | D22 | `mtls-cert.yaml` nouveau : Certificate `efe-server-mtls` + référence Issuer + Secret `efe-server-mtls-tls` mounté par le Deployment. |
| **mTLS auto-rotation** | D22 | Cert-manager auto-renouvelle 30j avant expiry (configurable via `mtls.renewBefore: 720h`). Pin test rotation cycle. |
| **Live bundle reload Job** | D27 | `bundle-reload-hook.yaml` nouveau : Helm hook `post-upgrade` qui exec'e une RPC admin pour swap bundle sans redémarrage. Idempotent : skip si bundle hash inchangé. |
| **DR drill quarterly** | D40 | `dr-drill-quarterly.yaml` nouveau : CronJob `0 4 1 */3 *` (quarterly 4am UTC le 1er) qui fait restore → validate → Slack alert. |
| **chart-testing CI gate** | D21+ | `.github/workflows/helm-test.yml` runs `ct lint --validate-maintainers` + `ct install --kube-version` + `helm-unittest`. |

### §4.2 Hors scope (M12)

* Les non-objectifs listés en §1 — service mesh, GitOps native, multi-cluster, HPA custom metrics, VPA, GraalVM.
* Migration à `Gateway API` (gateway.networking.k8s.io) — Reportée D45+.
* Migration `Ingress` API → `Gateway API` — Idem.
* Support multi-region active/active — Reporté D45+.
* `External Secrets Operator` (ESO) integration — Reporté D27+.

### §4.3 Acceptance criteria post-M12

```text
M12 v1.0 GA = TOUS les items suivants ✓ :

[D21]
  ✓ templates/networkpolicy.yaml strict default-deny + explicit allow
  ✓ .polaris.yaml configured with all checks at severity ≥ warning
  ✓ polaris audit retourne 0 dangers sur les manifests rendus
  ✓ Pin test test_d21_helm_extension_networkpolicy_strict GREEN
  ✓ helm-unittest tests/networkpolicy_test.yaml GREEN

[D22]
  ✓ templates/mtls-cert.yaml nouveau (Certificate + Issuer/ClusterIssuer ref)
  ✓ Documentation cert-manager prerequisite dans README
  ✓ values.yaml expose mtls.{enabled, issuer, renewBefore, dnsNames}
  ✓ helm template avec mtls.enabled=true rend Certificate + Secret reference
  ✓ Pin test test_d22_helm_extension_mtls_cert GREEN
  ✓ helm-unittest tests/mtls_cert_test.yaml GREEN

[D27]
  ✓ templates/bundle-reload-hook.yaml nouveau (Helm hook post-upgrade Job)
  ✓ Job idempotent : skip si bundle hash inchangé
  ✓ values.yaml expose bundleReload.{enabled, image, timeout}
  ✓ Pin test test_d27_helm_extension_bundle_reload GREEN
  ✓ helm-unittest tests/bundle_reload_test.yaml GREEN

[D40]
  ✓ templates/dr-drill-quarterly.yaml nouveau (CronJob quarterly)
  ✓ Schedule canonique 0 4 1 */3 *
  ✓ Job runs : restore → validate → Slack alert
  ✓ values.yaml expose drDrill.{enabled, schedule, slackWebhookSecret}
  ✓ Pin test test_d40_helm_extension_dr_drill GREEN

[Cross-cutting]
  ✓ Chart.yaml version bumped (0.6.0 → 0.7.0 D21 → 0.8.0 D22 → 0.9.0 D27 → 1.0.0 D40 GA)
  ✓ CHANGELOG.md per-sprint sections under [Unreleased]
  ✓ helm lint zero errors
  ✓ helm template renders cleanly across 3 values profiles (default + staging + prod)
  ✓ Polaris green sur les 3 values profiles
  ✓ helm-unittest test suite passe (≥ 30 unit tests YAML)
  ✓ chart-testing ct install passes on K8s 1.26 + 1.28 + 1.30 (kind)
  ✓ Helm chart published to oci://ghcr.io/efe-ai/charts/efe:1.0.0 (post-D40)
```

### §4.4 SLO finaux

| ID | Cible | Mesure |
|----|-------|--------|
| `SLO-HELM-1` | `helm install` cold deploy < 90 s | CI step timing |
| `SLO-HELM-2` | NetworkPolicy enforcement live within 30s of pod start | E2E test post-D21 |
| `SLO-HELM-3` | mTLS cert rotation completes without RPC errors | E2E test post-D22 |
| `SLO-HELM-4` | Bundle reload completes < 60s without pod restart | E2E test post-D27 |
| `SLO-HELM-5` | DR drill restore RTO < 20 min | DR drill report post-D40 |
| `SLO-HELM-6` | DR drill RPO ≤ 24h (last backup max age) | DR drill report |
| `SLO-HELM-7` | Polaris GREEN on default + staging + prod values | CI step |
| `SLO-HELM-8` | helm-unittest coverage ≥ 90% of templates | meta count |

---

## §5. Principes directeurs (idiomes Helm + Kubernetes)

### §5.1 Six guiding principles

1. **Backward-compatible defaults** — toute nouvelle feature a un
   `{{- if .Values.<feature>.enabled }}` block, default `false`.
   `helm upgrade` depuis pre-M12 ne casse rien sans opt-in
   explicite. Pin test institutionnel : test_helm_default_upgrade_safe.

2. **DRY via `_helpers.tpl`** — les labels, sélecteurs, env vars
   communs vivent dans `_helpers.tpl` named templates. Pas de
   duplication entre templates (post-A11 fix). Ajout M12 : 2
   nouveaux helpers `efe.mtls.secretName` + `efe.bundle.hash`.

3. **Schema validation** — `values.schema.json` (JSON Schema 2020-12)
   valide la structure de `values.yaml` au `helm install` time.
   Mauvaise valeur = erreur claire au caller, pas au runtime
   Kubernetes. M12 étend le schema avec les nouvelles sections.

4. **Polaris-aware design** — chaque template sera designé pour
   passer Polaris sans `polaris.fairwinds.com/*: ignore` annotations.
   Si une best-practice n'est pas applicable, on documente.

5. **Hook idempotency** — les Helm hooks (D27 bundle reload, D40 DR
   drill) sont idempotents : ré-exécution sans danger. Annotation
   `helm.sh/hook-delete-policy: hook-succeeded,before-hook-creation`
   pour le cleanup.

6. **No raw secrets in values** — tous les secrets (mTLS keys,
   audit keys, OIDC clientSecret, Slack webhook URL) sont fournis
   via `existingSecret` reference, pas inline. Pin test institutionnel.

### §5.2 Anti-patterns explicites

| Anti-pattern | Pourquoi NON | Alternative Helm/K8s |
|---|---|---|
| `helm.sh/hook-delete-policy: never` | Hooks orphelins après plusieurs upgrades | `hook-succeeded,before-hook-creation` |
| `image: efe-server:latest` | Non-reproductible, surprise au upgrade | `image.tag` valuable, default = `appVersion` |
| `runAsUser: 0` | Polaris danger | `runAsNonRoot: true` + `runAsUser: 65532` |
| `hostNetwork: true` | Bypass NetworkPolicy ; Polaris danger | NodePort Service ou Ingress |
| `privileged: true` | Polaris danger | required capabilities only via `add: [NET_BIND_SERVICE]` |
| Secret data inline dans values.yaml | Leak en `helm get values` | `existingSecret` reference |
| `kind: PodSecurityPolicy` (deprecated K8s 1.21+) | Removed K8s 1.25+ | Pod Security Admission via namespace label |
| NetworkPolicy `policyTypes: [Ingress, Egress]` mais `egress: []` (deny all) | Casse les image pulls + DNS | egress allow `kube-system/dns-namespace` minimum |
| `kind: ClusterRole` + `verbs: ["*"]` | RBAC over-privileged | least-privilege explicit verbs |
| `resources: {}` | Polaris warning + cluster scheduler unhappy | `requests` + `limits` requis |

### §5.3 Idiomes Helm canoniques choisis

* **`{{ if eq .Values.foo "bar" }}`** vs `{{ if .Values.foo == "bar" }}` :
  Go template uses Go syntax — `eq`, `ne`, `lt`, `gt`, `and`, `or`.
  Pas d'opérateurs `==`/`!=`.
* **`{{ tpl (.Values.template) . }}`** : runtime evaluation d'une string
  template (utile pour `tpl` chains).
* **`{{ include "efe.fullname" . }}`** vs **`{{ template "efe.fullname" . }}`** :
  préférer `include` (return string ; can be piped).
* **`{{ toYaml .Values.foo | nindent 4 }}`** : indenter proprement
  les structures imbriquées.
* **`{{- ... -}}`** trim whitespace : éliminer les empty lines
  parasites.
* **`{{- range $k, $v := .Values.tenants }}`** : itération sur map.
* **`{{- with .Values.foo }}`** : context push (replaces `.Values.foo`
  with `.`).
* **`{{ default "fallback" .Values.foo }}`** : valeur par défaut si
  `.Values.foo` est nil/empty.
* **`{{ .Files.Get "files/foo.txt" }}`** : embed file content.

### §5.4 Decision : OCI registry vs ChartMuseum

**Décision** : OCI registry (`ghcr.io/efe-ai/charts/efe`).

**Rationale** :
* **OCI registry (Helm 3.8+)** : standard moderne, intégré aux
  conteneurs registries existants (Docker Hub, GHCR, ECR, GitLab
  Registry, Artifactory). Pas d'infrastructure séparée.
* **ChartMuseum** : daemon dédié, à maintenir, avec authentification
  séparée. Acceptable pour les setups privés on-prem mais surchargé
  pour M12.
* **GitHub Pages helm repo** : OK pour les charts publics open-source
  mais pas pour la phase Φ-B private.

**Pin test** : `test_m12_oci_registry_workflow` vérifie le CI publish
job utilise `helm push oci://...`.

### §5.5 Decision : `helm-unittest` vs `chart-testing` (`ct`)

**Décision** : LES DEUX. Complémentaires.

* **`helm-unittest`** : tests YAML déclaratifs sur les manifests
  rendus. Asserts sur `documents.[0].kind == "Deployment"`,
  `containers.[0].image`, etc. Suite de tests vit dans `tests/`
  du chart. Run par `helm unittest deploy/helm/efe/`.
* **`chart-testing` (`ct`)** : CLI d'orchestration pour CI. `ct lint`
  appel `helm lint`. `ct install` provisionne un cluster K8s
  (kind) + `helm install` + valide. `ct test` lance les `helm test`
  pods.

**Pin tests** : `test_m12_helm_unittest_present` + `test_m12_ct_config_present`.

### §5.6 Decision : Polaris `severity` cutoff

**Décision** : `severity: danger` doit retourner ZÉRO ; `severity:
warning` peut retourner non-zero (mais documenté + justifié dans
README).

* **`danger`** : checks bloquants (run-as-root, privileged, hostNetwork,
  no resource requests, etc.). M12 = 0 dangers.
* **`warning`** : checks non-bloquants mais à fixer (no liveness probe,
  no readiness probe, no priority class, etc.). M12 = 0 warnings sur
  les templates Lettre E ; warnings tolérés sur les jobs/cronjobs
  (les jobs n'ont pas de probes par essence).
* **`ignore`** : checks volontairement désactivés. M12 = 1 check
  ignoré : `cpuRequestsMissing` sur les CronJobs (job lifecycle court,
  pas de besoin de QoS strict).

---

## §6. Invariants structurels (chart layout post-D18 + extensions M12)

### §6.1 Layout final (post-M12)

```
deploy/helm/efe/
├── Chart.yaml                           ← v1.0.0 post-D40 GA
├── values.yaml                          ← documented, ~400 lignes (vs 250 pre-M12)
├── values-staging.yaml                  ← overrides staging
├── values-prod.yaml                     ← overrides prod
├── values.schema.json                   ← JSON Schema validation (NEW M12)
├── README.md                            ← user-facing doc + cert-manager prereq + DR drill explained
├── CHANGELOG.md                         ← per-version notes M12 D21..D40
├── .helmignore                          ← exclude from package
├── .polaris.yaml                        ← Polaris validation config (NEW D21)
├── templates/
│   ├── _helpers.tpl                     ← + 2 NEW helpers (mTLS, bundle hash)
│   ├── deployment.yaml                  ← MODIFIED : mTLS volume mount (D22), bundle hash annotation (D27)
│   ├── service.yaml                     ← unchanged
│   ├── ingress.yaml                     ← unchanged
│   ├── poddisruptionbudget.yaml         ← unchanged
│   ├── hpa.yaml                         ← unchanged
│   ├── configmap.yaml                   ← MODIFIED : bundle path config (D27)
│   ├── secret.yaml                      ← unchanged
│   ├── serviceaccount.yaml              ← + RBAC for bundle reload Job (D27)
│   ├── coordinator.yaml                 ← unchanged
│   ├── lease-rbac.yaml                  ← unchanged
│   ├── audit-signer-sidecar.yaml        ← unchanged
│   ├── backup-cronjob.yaml              ← unchanged
│   ├── oidc-secret.yaml                 ← unchanged
│   ├── networkpolicy.yaml               ← EXTENDED D21 (strict default-deny + tenant + monitoring)
│   ├── servicemonitor.yaml              ← unchanged
│   ├── mtls-cert.yaml                   ← NEW D22 (Certificate + Issuer ref)
│   ├── bundle-reload-hook.yaml          ← NEW D27 (Helm hook Job)
│   ├── dr-drill-quarterly.yaml          ← NEW D40 (CronJob)
│   └── tests/
│       ├── test-connection.yaml         ← unchanged
│       ├── test-networkpolicy.yaml      ← NEW D21 (test ingress denied from non-tenant)
│       ├── test-mtls-handshake.yaml     ← NEW D22 (test cert-manager Secret present)
│       └── test-bundle-reload.yaml      ← NEW D27 (test admin RPC accessible)
├── tests/                               ← helm-unittest fixtures
│   ├── deployment_test.yaml             ← unchanged (post-A11 dedup)
│   ├── networkpolicy_test.yaml          ← NEW D21
│   ├── mtls_cert_test.yaml              ← NEW D22
│   ├── bundle_reload_test.yaml          ← NEW D27
│   └── dr_drill_test.yaml               ← NEW D40
└── files/                               ← embedded files
    ├── default-config.yaml              ← efe-config inline default
    └── dr-drill-script.sh               ← NEW D40 (script du CronJob)
```

### §6.2 Templates ajoutés/modifiés par sprint

| Sprint | Nouveau template | Template modifié |
|---|---|---|
| **D21** | `templates/tests/test-networkpolicy.yaml` + `tests/networkpolicy_test.yaml` + `.polaris.yaml` | `templates/networkpolicy.yaml` ÉTENDU |
| **D22** | `templates/mtls-cert.yaml` + `templates/tests/test-mtls-handshake.yaml` + `tests/mtls_cert_test.yaml` | `templates/deployment.yaml` (volume + envFrom) + `_helpers.tpl` (`efe.mtls.secretName`) |
| **D27** | `templates/bundle-reload-hook.yaml` + `templates/tests/test-bundle-reload.yaml` + `tests/bundle_reload_test.yaml` + `files/default-config.yaml` (extension) | `templates/configmap.yaml` (bundle path) + `templates/serviceaccount.yaml` (RBAC) + `_helpers.tpl` (`efe.bundle.hash`) + `templates/deployment.yaml` (annotation `efe.io/bundle-hash` for rollout-on-change) |
| **D40** | `templates/dr-drill-quarterly.yaml` + `tests/dr_drill_test.yaml` + `files/dr-drill-script.sh` | (none — purement nouveau) |

### §6.3 `Chart.yaml` versioning trajectoire

| Sprint | `Chart.yaml::version` | `Chart.yaml::appVersion` | Publication |
|---|---|---|---|
| AS-IS post-D18+A11 | 0.6.0 | 0.3.0 | (privé GitHub Packages helm) |
| D21 ship | 0.7.0 | 0.3.0 | privé |
| D22 ship | 0.8.0 | 0.4.0 (Lettre E mTLS bumped) | privé |
| D27 ship | 0.9.0 | 0.4.0 | privé |
| D40 ship + GA | **1.0.0** | 1.0.0 (cohérent SDK GA) | PUBLIC `oci://ghcr.io/efe-ai/charts/efe:1.0.0` |

### §6.4 Decision : `Chart.yaml::version` semver discipline

* **MINOR bump** (e.g., 0.6 → 0.7) à chaque sprint M12 (D21..D40)
  car les nouveaux templates ajoutent une surface utilisateur.
* **PATCH bump** uniquement pour audit fixes / typos / values-yaml
  refactors sans surface change.
* **MAJOR bump à v1.0.0** au moment du GA D40, confirmant l'API
  stable.

---

## §7. Vue d'ensemble — 4 sprints D21/D22/D27/D40

### §7.1 Récapitulatif des 4 livrables

| Sprint | Phase | Livrable phare | Wallclock | Lines code |
|---|---|---|---|---|
| **D21** | Φ-A | NetworkPolicy strict + Polaris GREEN | 3 jours | ~+250 LOC YAML |
| **D22** | Φ-B | mTLS cert-manager (Certificate + Issuer) | 4 jours | ~+220 LOC YAML |
| **D27** | Φ-C | Live bundle reload Job (Helm hook) | 3 jours | ~+280 LOC YAML + bash |
| **D40** | Φ-D | DR drill quarterly CronJob | 4 jours | ~+350 LOC YAML + bash |
| **Total** | — | — | **14 jours** | **~+1 100 LOC** |

### §7.2 Phasage cross-sprint

```
D21 ─┐      ┌──────── D22 ─┐                  ┌─ D27 ─┐                              ┌─ D40 ─ v1.0 GA
     │      │              │                  │       │                              │
   Φ-A      │            Φ-B                  │     Φ-C                              Φ-D
            │                                 │
        chart 0.7.0                       chart 0.8.0                              chart 1.0.0
```

* D21 (jour 60) : NetworkPolicy strict + Polaris GREEN.
* D22 (jour 70) : mTLS auto-rotation via cert-manager.
* D27 (jour 130) : Live bundle reload via Helm hook.
* D40 (jour 200) : DR drill quarterly + GA v1.0.0 publication OCI.

### §7.3 Risques cross-sprint

| Risque | P | I | Mitigation |
|---|:-:|:-:|---|
| Cluster K8s `1.26` deprecation pendant les 90 jours D21→D40 | M | M | Matrice CI 1.26 / 1.28 / 1.30 ; bumper kubeVersion dans Chart.yaml proactivement |
| cert-manager v2 breaking changes (improbable) | L | H | Pin cert-manager via prereq doc `>= v1.13.0, < v2.0.0` |
| Polaris config drift (nouvelles versions tightenent les checks) | M | M | Pin `.polaris.yaml::version` ; CI bump-test |
| OCI registry quota / rate limit | L | M | GHCR has generous limits ; fallback to ChartMuseum if needed |
| chart-testing (`ct`) breaking change | L | L | Pin via CI workflow `helm/chart-testing-action@v2.6.1` |
| Helm 4 breaking change | L | H | Targets Helm 3.13+ ; Helm 4 release est ETA 2026Q4+ |
| Audit fix A12 surface après D22/D27 | M | M | Audit pass séparé à la fin de chaque sprint |

---

## §8. Surface : extension `values.yaml` (post-M12)

### §8.1 Diff par sprint sur `values.yaml`

**D21** ajoute :

```yaml
networkPolicy:
  enabled: true
  ingress:
    # Tenant ingress : seuls les pods avec le label suivant sont allowed.
    tenantSelector:
      matchLabels:
        efe.io/tenant: "default"
    # Optional : whitelist d'IPs CIDR pour ingress externes (e.g., load balancer).
    cidrAllowList: []
  egress:
    # Egress vers monitoring stack only.
    monitoringSelector:
      matchLabels:
        kubernetes.io/metadata.name: monitoring
    # DNS egress (kube-dns) — REQUIRED, ne pas désactiver.
    dnsEnabled: true
    # Optional : egress vers audit aggregator (séparé de monitoring).
    auditAggregatorSelector:
      matchLabels:
        kubernetes.io/metadata.name: audit-aggregator

podSecurity:
  # K8s 1.25+ Pod Security Admission. Recommandation : "restricted".
  enforce: restricted
  # Override pour les images non-conformes (anti-pattern, à éviter).
  enforceVersion: latest
```

**D22** ajoute :

```yaml
mtls:
  enabled: false  # opt-in (default false pour backward-compat)
  # cert-manager Issuer name. Soit ClusterIssuer (cluster-wide), soit
  # Issuer (namespaced). Le user fournit la ref ; le chart NE provisionne
  # PAS l'Issuer (responsabilité de l'opérateur).
  issuerRef:
    name: efe-ca-issuer
    kind: ClusterIssuer        # ClusterIssuer | Issuer
    group: cert-manager.io
  # DNS names + IP SANs sur le certificat serveur.
  dnsNames:
    - efe-server.{{ .Release.Namespace }}.svc.cluster.local
    - efe-server.{{ .Release.Namespace }}.svc
    - efe.example.com          # external DNS (override per env)
  ipAddresses: []
  # Validity period + renew window.
  duration: 8760h              # 1 an
  renewBefore: 720h            # 30 jours avant expiry
  # Algorithm + key size.
  privateKey:
    algorithm: ECDSA
    size: 256
  # Secret name where cert-manager writes the rendered cert+key.
  secretName: ""               # empty = derived from {fullname}-mtls-tls
```

**D27** ajoute :

```yaml
bundle:
  # Path inside the pod where the bundle is mounted/loaded.
  path: /var/lib/efe/bundles/default
  # Sample bundle slug + version pinned via ConfigMap.
  slug: all-MiniLM-L6-v2-en
  version: 1.0.0
  # SHA-256 of the bundle manifest (for change detection + reload hook trigger).
  manifestSha256: ""           # caller fills this from build_sample_bundle.py output

bundleReload:
  enabled: false               # opt-in
  # Helm hook timing. post-upgrade = after Deployment apply.
  hookTiming: post-upgrade
  # Container image of the Job (ships kubectl + curl).
  image:
    repository: bitnami/kubectl
    tag: "1.30"
    pullPolicy: IfNotPresent
  # Job timeout. The bundle reload RPC should be < 60s ; we give 5min margin.
  timeout: 300
  # Max retries on RPC failure.
  retries: 3
  # Admin endpoint that the Job hits.
  adminEndpoint:
    service: efe-server
    port: 8080
    path: /admin/bundle-reload
```

**D40** ajoute :

```yaml
drDrill:
  enabled: false               # opt-in (default false pour ne PAS spammer en dev)
  # Quarterly schedule : 04:00 UTC le 1er de chaque trimestre.
  schedule: "0 4 1 */3 *"
  # Container image with the DR drill logic.
  image:
    repository: efe-dr-drill
    tag: ""                     # default = .Chart.AppVersion
    pullPolicy: IfNotPresent
  # Slack webhook secret for failure notification.
  slackWebhookSecret:
    name: efe-dr-drill-slack
    key: webhook-url
  # Backup source for the restore step (S3-compat).
  backupSource:
    s3Bucket: efe-backups
    s3Prefix: prod/
  # Validation step : run a few classify RPCs against the restored instance.
  validation:
    sampleQueries:
      - "I want a refund"
      - "How do I cancel my subscription"
    expectedZones:
      - trusted
      - warn
  # Retention : keep last 4 drill reports (one per quarter for 1 year).
  successfulJobsHistoryLimit: 4
  failedJobsHistoryLimit: 4
```

### §8.2 `values.schema.json` extension (D21..D40)

JSON Schema 2020-12 pour valider les nouveaux blocks au `helm install`
time. Voir Annexe §23.4 pour le schema complet ; extrait minimal :

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "properties": {
    "networkPolicy": {
      "type": "object",
      "properties": {
        "enabled": { "type": "boolean" },
        "ingress": { "type": "object" },
        "egress": { "type": "object" }
      },
      "required": ["enabled"]
    },
    "mtls": {
      "type": "object",
      "properties": {
        "enabled": { "type": "boolean" },
        "issuerRef": {
          "type": "object",
          "properties": {
            "name": { "type": "string" },
            "kind": { "type": "string", "enum": ["Issuer", "ClusterIssuer"] },
            "group": { "type": "string", "const": "cert-manager.io" }
          },
          "required": ["name", "kind"]
        }
      }
    },
    "bundleReload": { "type": "object", "properties": { "enabled": { "type": "boolean" } } },
    "drDrill": { "type": "object", "properties": { "enabled": { "type": "boolean" } } }
  }
}
```

---

## §9. Sprint D21 (Φ-A) — NetworkPolicy strict + Polaris validation GREEN

### §9.1 Sprint scope

* **`templates/networkpolicy.yaml`** — étendu pour passer de
  "ingress allow-default" (basic AS-IS) à "default-deny + tenant
  ingress whitelist + monitoring egress only".
* **`.polaris.yaml`** — nouvelle config root-level pour valider
  les manifests rendus contre Polaris best-practices.
* **`templates/tests/test-networkpolicy.yaml`** — Pod de test qui
  vérifie qu'un ingress non-tenant est bien refusé.
* **`tests/networkpolicy_test.yaml`** — helm-unittest YAML asserts.

### §9.2 NetworkPolicy strict — design canonical

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: {{ include "efe.fullname" . }}-strict
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
    # 1. Tenant pods ingress — selector-based whitelist.
    {{- if .Values.networkPolicy.ingress.tenantSelector }}
    - from:
        - namespaceSelector: {}
          podSelector:
            {{- toYaml .Values.networkPolicy.ingress.tenantSelector | nindent 12 }}
      ports:
        - protocol: TCP
          port: 50051        # gRPC
        - protocol: TCP
          port: 8080         # admin (mTLS-protected — auth at app layer)
    {{- end }}

    # 2. CIDR whitelist (e.g., load balancer source IPs).
    {{- with .Values.networkPolicy.ingress.cidrAllowList }}
    - from:
        {{- range . }}
        - ipBlock:
            cidr: {{ . }}
        {{- end }}
      ports:
        - protocol: TCP
          port: 50051
    {{- end }}

    # 3. Monitoring scrape (Prometheus from monitoring namespace).
    {{- if .Values.networkPolicy.egress.monitoringSelector }}
    - from:
        - namespaceSelector:
            {{- toYaml .Values.networkPolicy.egress.monitoringSelector | nindent 12 }}
      ports:
        - protocol: TCP
          port: 9090         # metrics
    {{- end }}

  egress:
    # 1. DNS egress (kube-system/kube-dns + CoreDNS) — REQUIRED.
    {{- if .Values.networkPolicy.egress.dnsEnabled }}
    - to:
        - namespaceSelector:
            matchLabels:
              kubernetes.io/metadata.name: kube-system
          podSelector:
            matchLabels:
              k8s-app: kube-dns
      ports:
        - protocol: UDP
          port: 53
        - protocol: TCP
          port: 53
    {{- end }}

    # 2. Audit aggregator egress.
    {{- if .Values.networkPolicy.egress.auditAggregatorSelector }}
    - to:
        - namespaceSelector:
            {{- toYaml .Values.networkPolicy.egress.auditAggregatorSelector | nindent 12 }}
      ports:
        - protocol: TCP
          port: 5044         # filebeat / fluentd default
    {{- end }}

    # 3. S3 backup egress (HTTPS).
    {{- if .Values.backup.enabled }}
    - to:
        - ipBlock:
            cidr: 0.0.0.0/0  # S3 endpoints — opérateur peut restreindre
            except:
              - 169.254.0.0/16  # link-local
              - 10.0.0.0/8       # internal CIDR
              - 172.16.0.0/12
              - 192.168.0.0/16
      ports:
        - protocol: TCP
          port: 443
    {{- end }}

    # 4. Optional : OIDC discovery egress.
    {{- if .Values.oidc.enabled }}
    - to:
        - ipBlock:
            cidr: 0.0.0.0/0
            except:
              - 169.254.0.0/16
      ports:
        - protocol: TCP
          port: 443
    {{- end }}
```

**Notes** :

* Le `ipBlock.cidr: 0.0.0.0/0 except: [...]` pattern permet egress
  externe sauf vers des CIDR internes — bonne pratique pour les
  egress vers S3/OIDC providers externes sans bloquer les internes.
* Tous les blocs `egress` sont conditionnels via `.Values.<feature>.enabled`
  — pas de NetworkPolicy "trop permissive" par défaut.
* Le `policyTypes: [Ingress, Egress]` active les deux types ; un
  NetworkPolicy avec `policyTypes: [Ingress]` seulement implique
  egress AUTHORIZED par défaut (anti-pattern).

### §9.3 `.polaris.yaml` config

```yaml
# Polaris config for the EFE Helm chart.
# Sprint D21 (Φ-A) — chart-level validation policy.

checks:
  # ─── Security ───
  hostNetworkSet: danger
  hostPIDSet: danger
  hostIPCSet: danger
  hostPortSet: danger
  notReadOnlyRootFilesystem: warning
  privilegeEscalationAllowed: danger
  runAsRootAllowed: danger
  runAsPrivileged: danger
  dangerousCapabilities: danger
  insecureCapabilities: warning
  automountServiceAccountToken: warning

  # ─── Reliability ───
  cpuRequestsMissing: warning
  cpuLimitsMissing: warning
  memoryRequestsMissing: warning
  memoryLimitsMissing: warning
  livenessProbeMissing: warning
  readinessProbeMissing: warning
  pullPolicyNotAlways: warning
  tagNotSpecified: danger
  priorityClassNotSet: ignore   # M12 doesn't enforce PriorityClass (operator-specific).

  # ─── Networking ───
  hostAliasesSet: warning

  # ─── Resources ───
  cpuRequestsMissing: warning
  memoryRequestsMissing: warning

# Exemptions : justified per-namespace.
exemptions:
  - namespace: kube-system
    rules:
      - hostNetworkSet
  # CronJobs are exempt from probe/QoS checks (short-lived).
  - controllerNames:
      - "{{ .Release.Name }}-backup"
      - "{{ .Release.Name }}-dr-drill"
    rules:
      - cpuLimitsMissing
      - memoryLimitsMissing
      - livenessProbeMissing
      - readinessProbeMissing
```

### §9.4 Tests

* **`templates/tests/test-networkpolicy.yaml`** : Pod helm-test (annotation
  `helm.sh/hook: test`) qui :
  1. Spawn dans un namespace SANS le label tenant → vérifie que `curl`
     vers le service EFE échoue (timeout).
  2. Spawn dans un namespace AVEC le label tenant → vérifie que `curl`
     fonctionne.

* **`tests/networkpolicy_test.yaml`** (helm-unittest) :
  ```yaml
  suite: NetworkPolicy strict
  templates:
    - networkpolicy.yaml
  tests:
    - it: produces a NetworkPolicy with policyTypes=[Ingress, Egress]
      asserts:
        - isKind:
            of: NetworkPolicy
        - equal:
            path: spec.policyTypes
            value: [Ingress, Egress]
    - it: defaults to default-deny when networkPolicy.enabled=false
      set:
        networkPolicy.enabled: false
      asserts:
        - hasDocuments:
            count: 0
    - it: adds tenantSelector ingress when configured
      set:
        networkPolicy.enabled: true
        networkPolicy.ingress.tenantSelector.matchLabels.efe.io/tenant: acme
      asserts:
        - contains:
            path: spec.ingress[0].from[0].podSelector.matchLabels
            content:
              efe.io/tenant: acme
  ```

### §9.5 Plan jour-par-jour D21

* **J1** : Étendre `templates/networkpolicy.yaml` avec strict
  default-deny + ingress + egress sections. Update `values.yaml`
  + `values.schema.json`.
* **J2** : Créer `.polaris.yaml` + run `polaris audit` sur `helm
  template` rendered output. Fix any dangers.
* **J3** : Écrire `templates/tests/test-networkpolicy.yaml` + 4
  helm-unittest cases dans `tests/networkpolicy_test.yaml`. Bump
  `Chart.yaml::version` 0.6.0 → 0.7.0. CHANGELOG entry. CI workflow
  step `polaris audit` ajouté.

---

## §10. Sprint D22 (Φ-B) — mTLS via cert-manager

### §10.1 Sprint scope

* **`templates/mtls-cert.yaml`** — NEW. Resource `Certificate` +
  référence `Issuer/ClusterIssuer`.
* **`_helpers.tpl`** — NEW helper `efe.mtls.secretName`.
* **`templates/deployment.yaml`** — MODIFIÉ : monter le Secret
  cert-manager dans le pod efe-server.
* **`templates/tests/test-mtls-handshake.yaml`** + **`tests/mtls_cert_test.yaml`**.

### §10.2 cert-manager flow (architecture)

```
┌──────────────┐     ┌─────────────┐     ┌──────────────┐    ┌──────────────┐
│ values.yaml  │     │ Helm        │     │ cert-manager  │     │ Kubernetes  │
│ mtls.enabled │ →→→ │ template    │ →→→ │ controller    │ →→→ │ Secret      │
│ = true       │     │ → Cert.yaml │     │ (reconciles)  │     │ tls.crt+key │
└──────────────┘     └─────────────┘     └──────────────┘     └──────┬───────┘
                                                                     │
                                                                     ▼
                                                            ┌────────────────┐
                                                            │ Deployment     │
                                                            │ volumeMounts   │
                                                            │ /etc/efe/tls   │
                                                            └────────────────┘
```

### §10.3 `templates/mtls-cert.yaml` canonical

```yaml
{{- if .Values.mtls.enabled }}
apiVersion: cert-manager.io/v1
kind: Certificate
metadata:
  name: {{ include "efe.fullname" . }}-mtls
  namespace: {{ .Release.Namespace }}
  labels:
    {{- include "efe.labels" . | nindent 4 }}
spec:
  # Where cert-manager writes the rendered cert + key + ca chain.
  secretName: {{ include "efe.mtls.secretName" . }}
  secretTemplate:
    labels:
      {{- include "efe.labels" . | nindent 6 }}
  # Validity + renewal.
  duration: {{ .Values.mtls.duration | default "8760h" }}
  renewBefore: {{ .Values.mtls.renewBefore | default "720h" }}

  # Subject + DNS SANs.
  subject:
    organizations:
      - {{ .Release.Namespace }}
  commonName: {{ include "efe.fullname" . }}.{{ .Release.Namespace }}.svc.cluster.local

  dnsNames:
    {{- range .Values.mtls.dnsNames }}
    - {{ tpl . $ | quote }}
    {{- end }}

  {{- with .Values.mtls.ipAddresses }}
  ipAddresses:
    {{- range . }}
    - {{ . | quote }}
    {{- end }}
  {{- end }}

  # Key algorithm + size.
  privateKey:
    algorithm: {{ .Values.mtls.privateKey.algorithm | default "ECDSA" }}
    size: {{ .Values.mtls.privateKey.size | default 256 }}
    rotationPolicy: Always

  # Issuer reference (caller-provided ; chart does NOT provision the Issuer).
  issuerRef:
    name: {{ .Values.mtls.issuerRef.name | required ".Values.mtls.issuerRef.name is required when mtls.enabled=true" }}
    kind: {{ .Values.mtls.issuerRef.kind | default "ClusterIssuer" }}
    group: {{ .Values.mtls.issuerRef.group | default "cert-manager.io" }}

  # Usage : both client + server auth (server-side mTLS terminates ; client side for service-to-service).
  usages:
    - server auth
    - client auth
{{- end }}
```

### §10.4 `_helpers.tpl` extension

```
{{/*
mTLS Secret name. Either user-provided or derived as <fullname>-mtls-tls.
*/}}
{{- define "efe.mtls.secretName" -}}
{{- if .Values.mtls.secretName }}
{{- .Values.mtls.secretName }}
{{- else }}
{{- printf "%s-mtls-tls" (include "efe.fullname" .) }}
{{- end }}
{{- end -}}
```

### §10.5 `templates/deployment.yaml` extension

```yaml
spec:
  template:
    spec:
      containers:
        - name: efe-server
          # ... existing config ...
          {{- if .Values.mtls.enabled }}
          volumeMounts:
            # ... existing mounts ...
            - name: mtls-cert
              mountPath: /etc/efe/tls
              readOnly: true
          env:
            - name: EFE_MTLS_CERT_PATH
              value: /etc/efe/tls/tls.crt
            - name: EFE_MTLS_KEY_PATH
              value: /etc/efe/tls/tls.key
            - name: EFE_MTLS_CA_PATH
              value: /etc/efe/tls/ca.crt
          {{- end }}
      volumes:
        # ... existing volumes ...
        {{- if .Values.mtls.enabled }}
        - name: mtls-cert
          secret:
            secretName: {{ include "efe.mtls.secretName" . }}
            items:
              - key: tls.crt
                path: tls.crt
              - key: tls.key
                path: tls.key
              - key: ca.crt
                path: ca.crt
        {{- end }}
```

### §10.6 cert-manager prereq (README)

```markdown
## Prerequisites for `mtls.enabled=true`

This chart requires [cert-manager](https://cert-manager.io/) ≥ v1.13
installed in the cluster + an `Issuer` or `ClusterIssuer` configured
to sign `Certificate` resources.

Quick install (cert-manager) :

```bash
kubectl apply -f https://github.com/cert-manager/cert-manager/releases/download/v1.13.0/cert-manager.yaml
```

ClusterIssuer (canonical for SaaS deployments via Let's Encrypt) :

```yaml
apiVersion: cert-manager.io/v1
kind: ClusterIssuer
metadata:
  name: efe-ca-issuer
spec:
  selfSigned: {}    # or `acme: {...}` for Let's Encrypt
```
```

### §10.7 Tests

* **`templates/tests/test-mtls-handshake.yaml`** : helm-test Pod
  qui mounts le Secret, fait `openssl s_client -connect efe-server:50051
  -cert /tls/tls.crt -key /tls/tls.key -CAfile /tls/ca.crt`. Echec
  si handshake fail.

* **`tests/mtls_cert_test.yaml`** :
  ```yaml
  suite: mTLS Certificate
  templates:
    - mtls-cert.yaml
  tests:
    - it: not rendered when mtls.enabled=false
      set:
        mtls.enabled: false
      asserts:
        - hasDocuments:
            count: 0
    - it: rendered with required fields when enabled
      set:
        mtls.enabled: true
        mtls.issuerRef.name: efe-ca
      asserts:
        - isKind:
            of: Certificate
        - equal:
            path: spec.issuerRef.name
            value: efe-ca
        - notEmpty:
            path: spec.dnsNames
        - equal:
            path: spec.privateKey.algorithm
            value: ECDSA
    - it: fails template if issuerRef.name missing
      set:
        mtls.enabled: true
      asserts:
        - failedTemplate:
            errorMessage: ".Values.mtls.issuerRef.name is required"
  ```

### §10.8 Plan jour-par-jour D22

* **J1** : Créer `templates/mtls-cert.yaml`. Update `values.yaml`
  + `values.schema.json` + helper `efe.mtls.secretName`.
* **J2** : Étendre `templates/deployment.yaml` avec volumeMounts +
  env vars EFE_MTLS_*. Tester `helm template` rendering.
* **J3** : Documenter cert-manager prereq dans README. Provision un
  `ClusterIssuer` self-signed pour les tests CI.
* **J4** : `templates/tests/test-mtls-handshake.yaml` +
  `tests/mtls_cert_test.yaml` (5 helm-unittest cases). Bump
  `Chart.yaml::version` 0.7.0 → 0.8.0 + `appVersion` 0.3.0 → 0.4.0
  (Lettre E mTLS bumped). CHANGELOG.

---

## §11. Sprint D27 (Φ-C) — Live bundle reload Job (Helm hook)

### §11.1 Sprint scope

* **`templates/bundle-reload-hook.yaml`** — NEW. Helm hook `post-upgrade`
  Job qui exec'e une RPC admin pour swap le bundle.
* **`templates/serviceaccount.yaml`** — MODIFIÉ : RBAC permettant
  au Job d'accéder au pod `efe-server` (via `kubectl exec`) ou au
  service admin (via `curl`).
* **`templates/deployment.yaml`** — MODIFIÉ : annotation `efe.io/bundle-hash`
  pour rolling-update on bundle change.
* **`_helpers.tpl`** — NEW helper `efe.bundle.hash`.
* **`templates/configmap.yaml`** — MODIFIÉ : path bundle dans la config.
* **`templates/tests/test-bundle-reload.yaml`** + **`tests/bundle_reload_test.yaml`**.

### §11.2 Live bundle reload — pattern

```
┌────────────────┐
│ Operator runs  │
│ helm upgrade   │
│ with new       │
│ bundle.manifestSha256 │
└────────┬───────┘
         │
         ▼
┌────────────────────────────────────┐
│ Helm reconciles :                  │
│ - Deployment annotation            │
│   efe.io/bundle-hash: <newSha>     │
│   → triggers rolling restart       │
│ - Bundle reload Job (post-upgrade) │
│   → calls admin RPC for hot reload │
└────────┬───────────────────────────┘
         │
         ▼
┌────────────────────────────────────┐
│ efe-server pods :                  │
│ - On startup : load new bundle     │
│   from path /var/lib/efe/bundles   │
│ - OR (if hot reload supported) :   │
│   admin RPC swaps bundle in-place  │
└────────────────────────────────────┘
```

Le pattern **default** = rolling restart via Deployment annotation
update (Kubernetes-native pattern). Le bundle reload Job est OPT-IN
pour les setups où le bundle change FRÉQUEMMENT (e.g., A/B testing
ML models) sans pouvoir tolérer les rolling restarts.

### §11.3 `templates/bundle-reload-hook.yaml` canonical

```yaml
{{- if .Values.bundleReload.enabled }}
apiVersion: batch/v1
kind: Job
metadata:
  name: {{ include "efe.fullname" . }}-bundle-reload
  namespace: {{ .Release.Namespace }}
  labels:
    {{- include "efe.labels" . | nindent 4 }}
  annotations:
    helm.sh/hook: post-upgrade
    helm.sh/hook-weight: "10"
    helm.sh/hook-delete-policy: hook-succeeded,before-hook-creation
spec:
  backoffLimit: {{ .Values.bundleReload.retries }}
  activeDeadlineSeconds: {{ .Values.bundleReload.timeout }}
  ttlSecondsAfterFinished: 86400  # cleanup after 24h
  template:
    metadata:
      labels:
        {{- include "efe.labels" . | nindent 8 }}
        app.kubernetes.io/component: bundle-reload
    spec:
      restartPolicy: Never
      serviceAccountName: {{ include "efe.fullname" . }}-bundle-reload
      securityContext:
        runAsNonRoot: true
        runAsUser: 65532
        seccompProfile:
          type: RuntimeDefault
      containers:
        - name: bundle-reload
          image: "{{ .Values.bundleReload.image.repository }}:{{ .Values.bundleReload.image.tag }}"
          imagePullPolicy: {{ .Values.bundleReload.image.pullPolicy }}
          securityContext:
            allowPrivilegeEscalation: false
            readOnlyRootFilesystem: true
            capabilities:
              drop: [ALL]
          command:
            - /bin/sh
            - -c
            - |
              set -euo pipefail
              echo "[$(date -u +%FT%TZ)] Bundle reload Job starting"
              echo "Target endpoint : http://{{ .Values.bundleReload.adminEndpoint.service }}:{{ .Values.bundleReload.adminEndpoint.port }}{{ .Values.bundleReload.adminEndpoint.path }}"
              echo "Expected bundle hash : {{ .Values.bundle.manifestSha256 }}"

              # Fetch current bundle hash via admin RPC GET.
              CURRENT_HASH=$(curl -sf --max-time 10 \
                  "http://{{ .Values.bundleReload.adminEndpoint.service }}:{{ .Values.bundleReload.adminEndpoint.port }}/admin/bundle-info" \
                  | jq -r '.manifestSha256' 2>/dev/null || echo "unknown")
              echo "Current bundle hash : $CURRENT_HASH"

              # Idempotency : skip if already at the expected hash.
              if [ "$CURRENT_HASH" = "{{ .Values.bundle.manifestSha256 }}" ]; then
                  echo "✅ Already at expected hash — skipping reload"
                  exit 0
              fi

              # Trigger reload via admin RPC POST.
              echo "Triggering bundle reload..."
              for i in 1 2 3; do
                  if curl -sf --max-time 60 -X POST \
                      -H 'Content-Type: application/json' \
                      -d '{"manifestSha256":"{{ .Values.bundle.manifestSha256 }}"}' \
                      "http://{{ .Values.bundleReload.adminEndpoint.service }}:{{ .Values.bundleReload.adminEndpoint.port }}{{ .Values.bundleReload.adminEndpoint.path }}"; then
                      echo "✅ Reload succeeded"
                      exit 0
                  fi
                  echo "Attempt $i failed ; retrying after 10s..."
                  sleep 10
              done
              echo "❌ Reload failed after {{ .Values.bundleReload.retries }} attempts"
              exit 1
{{- end }}
```

### §11.4 Helper `efe.bundle.hash`

```
{{/*
Bundle hash for rolling-restart annotation. Combines :
  - .Values.bundle.manifestSha256 (caller-provided)
  - .Values.bundle.slug + .Values.bundle.version (defensive — change ⇒ restart)
*/}}
{{- define "efe.bundle.hash" -}}
{{- $bundle := .Values.bundle | default dict }}
{{- $slug := $bundle.slug | default "default" }}
{{- $ver := $bundle.version | default "" }}
{{- $sha := $bundle.manifestSha256 | default "" }}
{{- printf "%s-%s-%s" $slug $ver $sha | sha256sum | trunc 16 -}}
{{- end -}}
```

### §11.5 Deployment annotation

```yaml
spec:
  template:
    metadata:
      annotations:
        # Triggers a rolling restart when the bundle changes.
        # Mirrors the pattern Kubernetes uses for ConfigMap/Secret hash.
        efe.io/bundle-hash: {{ include "efe.bundle.hash" . }}
        {{- if .Values.deployment.podAnnotations }}
        {{- toYaml .Values.deployment.podAnnotations | nindent 8 }}
        {{- end }}
```

### §11.6 RBAC for the bundle reload Job

```yaml
{{- if .Values.bundleReload.enabled }}
apiVersion: v1
kind: ServiceAccount
metadata:
  name: {{ include "efe.fullname" . }}-bundle-reload
  namespace: {{ .Release.Namespace }}
  labels:
    {{- include "efe.labels" . | nindent 4 }}
  annotations:
    helm.sh/hook: post-upgrade
    helm.sh/hook-weight: "1"
    helm.sh/hook-delete-policy: hook-succeeded,before-hook-creation
---
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: {{ include "efe.fullname" . }}-bundle-reload
  namespace: {{ .Release.Namespace }}
  annotations:
    helm.sh/hook: post-upgrade
    helm.sh/hook-weight: "2"
    helm.sh/hook-delete-policy: hook-succeeded,before-hook-creation
rules:
  - apiGroups: [""]
    resources: [pods]
    verbs: [get, list]
  - apiGroups: [""]
    resources: [pods/exec]
    verbs: [create]
  - apiGroups: [""]
    resources: [services]
    verbs: [get]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: {{ include "efe.fullname" . }}-bundle-reload
  namespace: {{ .Release.Namespace }}
  annotations:
    helm.sh/hook: post-upgrade
    helm.sh/hook-weight: "3"
    helm.sh/hook-delete-policy: hook-succeeded,before-hook-creation
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: Role
  name: {{ include "efe.fullname" . }}-bundle-reload
subjects:
  - kind: ServiceAccount
    name: {{ include "efe.fullname" . }}-bundle-reload
    namespace: {{ .Release.Namespace }}
{{- end }}
```

### §11.7 Plan jour-par-jour D27

* **J1** : Créer `templates/bundle-reload-hook.yaml` Job + RBAC
  triplet (ServiceAccount + Role + RoleBinding) avec annotations
  Helm hook canoniques.
* **J2** : Helper `efe.bundle.hash` + annotation Deployment.
  Tester `helm upgrade` triggers rolling restart on bundle change.
* **J3** : `tests/bundle_reload_test.yaml` (4 helm-unittest cases) +
  `templates/tests/test-bundle-reload.yaml`. Bump `Chart.yaml::version`
  0.8.0 → 0.9.0. CHANGELOG.

---

## §12. Sprint D40 (Φ-D) — DR drill quarterly CronJob

### §12.1 Sprint scope

* **`templates/dr-drill-quarterly.yaml`** — NEW. CronJob qui s'exécute
  trimestriellement et lance le DR drill complet.
* **`files/dr-drill-script.sh`** — NEW. Script bash embarqué dans
  le CronJob, qui :
  1. Liste les backup snapshots récents.
  2. Restore le plus récent dans un namespace canary `efe-dr-drill-<timestamp>`.
  3. Lance des classify RPCs de smoke vs le canary.
  4. Compare les verdicts contre les baselines attendus.
  5. Génère un rapport JSON + envoi à Slack si failure.
  6. Cleanup le namespace canary.
* **`tests/dr_drill_test.yaml`**.

### §12.2 DR drill — séquence canonique

```
┌──────────────────────────────────────────────────────────────────┐
│ DR DRILL — Quarterly cycle (every 3 months at 04:00 UTC, 1st)    │
├──────────────────────────────────────────────────────────────────┤
│                                                                  │
│  Step 1 : Find latest backup snapshot                            │
│       aws s3 ls s3://efe-backups/prod/ | tail -1                 │
│                                                                  │
│  Step 2 : Provision canary namespace                             │
│       kubectl create namespace efe-dr-drill-<timestamp>          │
│                                                                  │
│  Step 3 : Restore backup                                         │
│       kubectl exec ... -- /opt/efe/restore.sh <snapshot> <ns>    │
│                                                                  │
│  Step 4 : Wait for restored pods to be Ready                     │
│       kubectl wait --for=condition=Ready pod -l app=efe-server   │
│              -n efe-dr-drill-<timestamp> --timeout=600s          │
│                                                                  │
│  Step 5 : Smoke validation — classify RPCs                       │
│       for query in sampleQueries :                               │
│           verdict = classify(query)                              │
│           assert verdict.zone in expectedZones                   │
│                                                                  │
│  Step 6 : Generate report                                        │
│       report = { drill_id, started, ended, status, samples,      │
│                  rto_seconds, snapshot_age_hours, ... }          │
│                                                                  │
│  Step 7 : If failure → Slack alert ; if success → log only      │
│                                                                  │
│  Step 8 : Cleanup canary namespace                               │
│       kubectl delete namespace efe-dr-drill-<timestamp>          │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘
```

### §12.3 `templates/dr-drill-quarterly.yaml` canonical

```yaml
{{- if .Values.drDrill.enabled }}
apiVersion: batch/v1
kind: CronJob
metadata:
  name: {{ include "efe.fullname" . }}-dr-drill
  namespace: {{ .Release.Namespace }}
  labels:
    {{- include "efe.labels" . | nindent 4 }}
spec:
  schedule: {{ .Values.drDrill.schedule | quote }}
  concurrencyPolicy: Forbid           # don't run two drills concurrently
  successfulJobsHistoryLimit: {{ .Values.drDrill.successfulJobsHistoryLimit }}
  failedJobsHistoryLimit: {{ .Values.drDrill.failedJobsHistoryLimit }}
  jobTemplate:
    spec:
      backoffLimit: 1                 # one retry max ; failure → human investigation
      activeDeadlineSeconds: 3600     # 1h hard cap (RTO budget)
      ttlSecondsAfterFinished: 604800 # keep Job state for 1 week
      template:
        metadata:
          labels:
            {{- include "efe.labels" . | nindent 12 }}
            app.kubernetes.io/component: dr-drill
        spec:
          restartPolicy: Never
          serviceAccountName: {{ include "efe.fullname" . }}-dr-drill
          securityContext:
            runAsNonRoot: true
            runAsUser: 65532
            seccompProfile:
              type: RuntimeDefault
          containers:
            - name: dr-drill
              image: "{{ .Values.drDrill.image.repository }}:{{ .Values.drDrill.image.tag | default .Chart.AppVersion }}"
              imagePullPolicy: {{ .Values.drDrill.image.pullPolicy }}
              securityContext:
                allowPrivilegeEscalation: false
                readOnlyRootFilesystem: true
                capabilities:
                  drop: [ALL]
              env:
                - name: BACKUP_S3_BUCKET
                  value: {{ .Values.drDrill.backupSource.s3Bucket }}
                - name: BACKUP_S3_PREFIX
                  value: {{ .Values.drDrill.backupSource.s3Prefix }}
                - name: SLACK_WEBHOOK_URL
                  valueFrom:
                    secretKeyRef:
                      name: {{ .Values.drDrill.slackWebhookSecret.name }}
                      key: {{ .Values.drDrill.slackWebhookSecret.key }}
                - name: SAMPLE_QUERIES
                  value: {{ join "," .Values.drDrill.validation.sampleQueries | quote }}
                - name: EXPECTED_ZONES
                  value: {{ join "," .Values.drDrill.validation.expectedZones | quote }}
                - name: PARENT_NAMESPACE
                  value: {{ .Release.Namespace }}
              volumeMounts:
                - name: drill-script
                  mountPath: /opt/dr-drill
                  readOnly: true
                - name: tmp
                  mountPath: /tmp
              command:
                - /opt/dr-drill/dr-drill-script.sh
              resources:
                requests:
                  cpu: 100m
                  memory: 128Mi
                limits:
                  cpu: 500m
                  memory: 512Mi
          volumes:
            - name: drill-script
              configMap:
                name: {{ include "efe.fullname" . }}-dr-drill-script
                defaultMode: 0o755
            - name: tmp
              emptyDir: {}
{{- end }}
---
{{- if .Values.drDrill.enabled }}
apiVersion: v1
kind: ConfigMap
metadata:
  name: {{ include "efe.fullname" . }}-dr-drill-script
  namespace: {{ .Release.Namespace }}
  labels:
    {{- include "efe.labels" . | nindent 4 }}
data:
  dr-drill-script.sh: |-
    {{- .Files.Get "files/dr-drill-script.sh" | nindent 4 }}
{{- end }}
```

### §12.4 `files/dr-drill-script.sh` — bash impl

```bash
#!/usr/bin/env bash
# Sprint D40 (Φ-D) — DR drill quarterly script.
#
# Cf. docs/EFE_HELM_CHART_EXTENSION_ARCHITECTURE.md §12.

set -euo pipefail
IFS=$'\n\t'

DRILL_ID="dr-drill-$(date -u +%Y%m%d%H%M%S)"
CANARY_NS="efe-${DRILL_ID}"
START_NS=$(date +%s)

log() { echo "[$(date -u +%FT%TZ)] [drill=$DRILL_ID] $*" >&2; }

slack() {
    local emoji="$1"
    local msg="$2"
    if [[ -n "${SLACK_WEBHOOK_URL:-}" ]]; then
        curl -sS -X POST -H 'Content-type: application/json' \
            --data "{\"text\":\"$emoji [DR drill $DRILL_ID] $msg\"}" \
            "$SLACK_WEBHOOK_URL" || true
    fi
}

cleanup() {
    log "Cleanup: deleting canary namespace $CANARY_NS"
    kubectl delete namespace "$CANARY_NS" --ignore-not-found --wait=false
}
trap cleanup EXIT

# 1. Find latest backup snapshot.
log "Step 1 : Discovering latest backup snapshot"
LATEST=$(aws s3 ls "s3://${BACKUP_S3_BUCKET}/${BACKUP_S3_PREFIX}" \
    --recursive | sort | tail -1 | awk '{print $4}')
if [[ -z "$LATEST" ]]; then
    slack "🔴" "No backup found in s3://${BACKUP_S3_BUCKET}/${BACKUP_S3_PREFIX} — DR drill ABORT"
    exit 2
fi
log "Latest snapshot : $LATEST"

# Snapshot age (hours).
SNAPSHOT_DATE=$(aws s3 ls "s3://${BACKUP_S3_BUCKET}/${LATEST}" | awk '{print $1, $2}')
SNAPSHOT_AGE_HOURS=$(( ($(date +%s) - $(date -d "$SNAPSHOT_DATE" +%s)) / 3600 ))
log "Snapshot age : ${SNAPSHOT_AGE_HOURS}h"

# Pin assertion : RPO ≤ 24h.
if [[ "$SNAPSHOT_AGE_HOURS" -gt 24 ]]; then
    slack "🟠" "Snapshot age ${SNAPSHOT_AGE_HOURS}h exceeds RPO 24h — investigate backup pipeline"
fi

# 2. Provision canary namespace.
log "Step 2 : Creating canary namespace $CANARY_NS"
kubectl create namespace "$CANARY_NS"
kubectl label namespace "$CANARY_NS" \
    pod-security.kubernetes.io/enforce=restricted \
    efe.io/drill="$DRILL_ID"

# 3. Restore backup.
log "Step 3 : Restoring snapshot $LATEST to namespace $CANARY_NS"
# This invokes the backup operator's restore CRD, OR a custom restore Job.
# The implementation depends on the backup tool used (Velero, Kasten K10, etc.).
# Generic placeholder — opérateur fournit son own restore image.
kubectl run efe-restore --image="${RESTORE_IMAGE:-efe-backup-restore:latest}" \
    --restart=Never --rm -i --tty=false \
    --namespace="$CANARY_NS" \
    --env="SNAPSHOT_KEY=${LATEST}" \
    --env="TARGET_NS=${CANARY_NS}"

# 4. Wait for restored pods Ready.
log "Step 4 : Waiting for restored efe-server pods to be Ready (timeout 10min)"
kubectl wait --for=condition=Ready pod \
    -l app.kubernetes.io/name=efe \
    -n "$CANARY_NS" --timeout=600s

RTO_SECONDS=$(( $(date +%s) - START_NS ))
log "RTO : ${RTO_SECONDS}s"

# 5. Smoke validation — classify RPCs.
log "Step 5 : Smoke validation — classify RPCs"
SUCCESS=0
FAILURE=0
IFS=',' read -ra QUERIES <<< "$SAMPLE_QUERIES"
IFS=',' read -ra EXPECTED <<< "$EXPECTED_ZONES"
for i in "${!QUERIES[@]}"; do
    Q="${QUERIES[$i]}"
    EXPECTED_ZONE="${EXPECTED[$i]:-trusted}"

    # Run the classify RPC against the canary svc.
    RESPONSE=$(curl -sf --max-time 10 \
        -X POST \
        -H "Content-Type: application/json" \
        -d "{\"text\":\"$Q\"}" \
        "http://efe-server.${CANARY_NS}.svc.cluster.local:50051/classify" \
        2>/dev/null || echo '{"zone":"error"}')
    ZONE=$(echo "$RESPONSE" | jq -r '.zone' 2>/dev/null || echo "error")

    if [[ "$ZONE" == "$EXPECTED_ZONE" ]]; then
        log "✅ Query '$Q' → zone='$ZONE' (expected '$EXPECTED_ZONE')"
        SUCCESS=$((SUCCESS + 1))
    else
        log "❌ Query '$Q' → zone='$ZONE' (expected '$EXPECTED_ZONE')"
        FAILURE=$((FAILURE + 1))
    fi
done

# 6. Generate report (JSON).
REPORT=$(cat <<EOF
{
    "drill_id": "$DRILL_ID",
    "started_at": "$(date -u -d "@$START_NS" +%FT%TZ)",
    "ended_at": "$(date -u +%FT%TZ)",
    "rto_seconds": $RTO_SECONDS,
    "snapshot_age_hours": $SNAPSHOT_AGE_HOURS,
    "snapshot_key": "$LATEST",
    "sample_queries_total": ${#QUERIES[@]},
    "sample_queries_success": $SUCCESS,
    "sample_queries_failure": $FAILURE,
    "status": "$([[ $FAILURE -eq 0 ]] && echo "PASS" || echo "FAIL")"
}
EOF
)
echo "$REPORT" | tee /tmp/dr-drill-report.json
log "Report:"
log "$REPORT"

# 7. Slack notify on failure.
if [[ "$FAILURE" -gt 0 ]]; then
    slack "🔴" "DR drill FAILED — $FAILURE/${#QUERIES[@]} queries returned wrong zone. RTO=${RTO_SECONDS}s, snapshot age=${SNAPSHOT_AGE_HOURS}h"
    exit 1
elif [[ "$SNAPSHOT_AGE_HOURS" -gt 24 ]]; then
    slack "🟠" "DR drill PASSED but snapshot age ${SNAPSHOT_AGE_HOURS}h > RPO 24h — investigate backup"
else
    slack "✅" "DR drill PASSED — RTO=${RTO_SECONDS}s, snapshot age=${SNAPSHOT_AGE_HOURS}h, ${#QUERIES[@]}/${#QUERIES[@]} queries OK"
fi

# 8. Cleanup happens via the EXIT trap.
log "✅ DR drill $DRILL_ID complete"
exit 0
```

### §12.5 RBAC for DR drill

```yaml
{{- if .Values.drDrill.enabled }}
apiVersion: v1
kind: ServiceAccount
metadata:
  name: {{ include "efe.fullname" . }}-dr-drill
  namespace: {{ .Release.Namespace }}
  labels:
    {{- include "efe.labels" . | nindent 4 }}
---
# DR drill needs cluster-wide perms : create namespaces + run pods.
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: {{ include "efe.fullname" . }}-dr-drill
  labels:
    {{- include "efe.labels" . | nindent 4 }}
rules:
  - apiGroups: [""]
    resources: [namespaces]
    verbs: [create, delete, get, list]
  - apiGroups: [""]
    resources: [pods, services, configmaps, secrets]
    verbs: [create, delete, get, list, watch]
  - apiGroups: ["apps"]
    resources: [deployments, statefulsets]
    verbs: [create, delete, get, list, watch]
  - apiGroups: ["batch"]
    resources: [jobs]
    verbs: [create, delete, get, list, watch]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: {{ include "efe.fullname" . }}-dr-drill
  labels:
    {{- include "efe.labels" . | nindent 4 }}
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: {{ include "efe.fullname" . }}-dr-drill
subjects:
  - kind: ServiceAccount
    name: {{ include "efe.fullname" . }}-dr-drill
    namespace: {{ .Release.Namespace }}
{{- end }}
```

### §12.6 Plan jour-par-jour D40

* **J1** : Créer `templates/dr-drill-quarterly.yaml` CronJob +
  ConfigMap (avec `.Files.Get` pour le script).
* **J2** : `files/dr-drill-script.sh` — full bash logic (steps 1-8).
* **J3** : RBAC ClusterRole + ClusterRoleBinding pour create
  namespaces + delete. helm-unittest cases.
* **J4** : Test end-to-end : `helm install drDrill.enabled=true`,
  trigger Manual via `kubectl create job --from=cronjob/...`,
  vérifier le report + Slack notification. Bump
  `Chart.yaml::version` 0.9.0 → **1.0.0** + `appVersion` 0.4.0 → 1.0.0
  GA. CHANGELOG. Tag `v1.0.0`. Publish OCI registry.

---

## §13. Stratégie de tests

### §13.1 Pyramid

```
                          /\
                         /  \  ct install on K8s 1.26/1.28/1.30 (kind)
                        /----\
                       /      \
                      /helm-tst\  ~10 helm test pods (D21+D22+D27)
                     /----------\
                    /            \
                   / helm-unittest \  ~30 unit tests YAML
                  /                 \
                 /-------------------\
                /                     \
               /  Pin tests Python     \  ~25 tests structure
              /-------------------------\
             /                           \
            /  Polaris validation         \
           /-------------------------------\
```

### §13.2 helm-unittest tests

| Test file | Tests | Sprint |
|---|:---:|:-:|
| `tests/deployment_test.yaml` | 10 (existing post-A11) | AS-IS |
| `tests/networkpolicy_test.yaml` | 6 | D21 |
| `tests/mtls_cert_test.yaml` | 5 | D22 |
| `tests/bundle_reload_test.yaml` | 4 | D27 |
| `tests/dr_drill_test.yaml` | 5 | D40 |
| **Total** | **30** | |

### §13.3 chart-testing (ct) CI

```yaml
# .github/workflows/helm-test.yml
name: helm-test
on: pull_request
jobs:
  lint-and-test:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }
      - uses: azure/setup-helm@v4
        with: { version: 'v3.13.3' }
      - uses: helm/chart-testing-action@v2.6.1
      - name: ct lint
        run: ct lint --target-branch=main --validate-maintainers --check-version-increment=true
      - name: kind cluster
        uses: helm/kind-action@v1.10.0
        with: { version: 'v0.22.0', node_image: 'kindest/node:v1.28.7' }
      - name: cert-manager install (D22 prereq)
        run: kubectl apply -f https://github.com/cert-manager/cert-manager/releases/download/v1.13.3/cert-manager.yaml
      - name: ct install
        run: ct install --target-branch=main
      - name: Polaris audit
        run: |
          curl -L https://github.com/FairwindsOps/polaris/releases/download/8.5.5/polaris_linux_amd64.tar.gz | tar xz
          helm template deploy/helm/efe/ -f deploy/helm/efe/values-prod.yaml > /tmp/manifests.yaml
          ./polaris audit --audit-path /tmp/manifests.yaml --severity danger --output-format pretty
      - name: helm-unittest
        run: |
          helm plugin install https://github.com/helm-unittest/helm-unittest
          helm unittest deploy/helm/efe/
```

### §13.4 Pin tests Python (~25)

| Section | Tests | Sprint |
|---|:---:|:-:|
| §1 Chart.yaml versioning | 3 | toutes |
| §2 NetworkPolicy strict | 4 | D21 |
| §3 Polaris config + GREEN | 3 | D21 |
| §4 mTLS Certificate template | 4 | D22 |
| §5 Bundle reload Helm hook | 4 | D27 |
| §6 DR drill CronJob + script | 4 | D40 |
| §7 helm-unittest fixtures | 3 | toutes |
| **Total** | **25** | |

---

## §14. Compatibilité Kubernetes versions

| K8s version | Status M12 | Notes |
|---|---|---|
| 1.25 | ❌ NOT supported | PSP supprimé, autoscaling/v1 deprecated |
| 1.26 | ✅ minimum supported | Chart.yaml::kubeVersion: ">= 1.26.0-0" |
| 1.27 | ✅ tested | CI matrix |
| 1.28 | ✅ tested | CI matrix |
| 1.29 | ✅ tested | CI matrix |
| 1.30 | ✅ tested | CI matrix (latest) |
| 1.31+ | ⚠ untested | best-effort ; bump kubeVersion when validated |

---

## §15. Compatibilité cert-manager versions

| cert-manager version | Status M12 | Notes |
|---|---|---|
| 1.12 | ⚠ minor risk | ECDSA support partial |
| 1.13 | ✅ minimum | recommended |
| 1.14 | ✅ tested | |
| 1.15 | ✅ tested | latest stable |

---

## §16. Compatibilité Helm versions

| Helm version | Status M12 | Notes |
|---|---|---|
| 3.10 | ⚠ untested | |
| 3.11 | ✅ supported | basic |
| 3.12 | ✅ supported | |
| 3.13 | ✅ minimum recommended | OCI registry mature |
| 3.14 | ✅ tested | |
| 3.15 | ✅ tested | latest |

---

## §17. SLO mapping

| SLO | Méthode | Cible | Pin test |
|---|---|---|---|
| SLO-HELM-1 cold deploy | CI step timing `helm install` | < 90 s | DT-H-1 |
| SLO-HELM-2 NetworkPolicy enforcement | E2E test post-D21 | live within 30s | DT-H-2 |
| SLO-HELM-3 mTLS rotation | E2E test post-D22 | no RPC error during cycle | DT-H-3 |
| SLO-HELM-4 bundle reload | E2E test post-D27 | < 60s no restart | DT-H-4 |
| SLO-HELM-5 DR drill RTO | DR drill report | < 20 min | DT-H-5 |
| SLO-HELM-6 DR drill RPO | DR drill report | ≤ 24h | DT-H-6 |
| SLO-HELM-7 Polaris GREEN | CI step | 0 dangers all profiles | DT-H-7 |
| SLO-HELM-8 unittest coverage | meta | ≥ 90% templates | DT-H-8 |

---

## §18. Risques transverses + mitigations

| Risque | P | I | Mitigation |
|---|:-:|:-:|---|
| K8s 1.31+ deprecates `policy/v1beta1` PDB | L | M | Already on `policy/v1` ; pin `kubeVersion >= 1.26` |
| cert-manager v2 breaking | L | H | Pin in README ; test matrix |
| OCI registry rate limit | L | M | GHCR generous ; fallback Quay or Artifactory |
| Polaris check tightening over versions | M | M | Pin Polaris version in CI |
| chart-testing breaking | L | L | Pin action version |
| DR drill bash brittleness (shellcheck) | M | M | shellcheck in CI ; bash strict mode `set -euo pipefail` |
| Bundle reload RPC API drift | L | M | Versioned admin API ; hash compat checks |
| Cluster CNI doesn't support NetworkPolicy | M | M | Document in README ; values gate ; opt-in |

---

## §19. Volumétrie code attendue

| Composant | Sprint | LOC YAML | LOC Bash |
|---|---|---:|---:|
| `templates/networkpolicy.yaml` extension | D21 | +120 | — |
| `.polaris.yaml` | D21 | +50 | — |
| `templates/tests/test-networkpolicy.yaml` | D21 | +30 | — |
| `tests/networkpolicy_test.yaml` | D21 | +50 | — |
| `templates/mtls-cert.yaml` | D22 | +60 | — |
| `templates/deployment.yaml` extension (volumeMounts) | D22 | +20 | — |
| `_helpers.tpl` extension (mtls.secretName) | D22 | +10 | — |
| `templates/tests/test-mtls-handshake.yaml` | D22 | +50 | +30 |
| `tests/mtls_cert_test.yaml` | D22 | +50 | — |
| `templates/bundle-reload-hook.yaml` | D27 | +120 | +50 (in-template script) |
| `templates/serviceaccount.yaml` extension RBAC | D27 | +50 | — |
| `_helpers.tpl` extension (bundle.hash) | D27 | +10 | — |
| `templates/configmap.yaml` extension | D27 | +10 | — |
| `tests/bundle_reload_test.yaml` | D27 | +40 | — |
| `templates/dr-drill-quarterly.yaml` CronJob + ConfigMap | D40 | +100 | — |
| `files/dr-drill-script.sh` | D40 | — | +180 |
| RBAC ClusterRole + Binding | D40 | +50 | — |
| `tests/dr_drill_test.yaml` | D40 | +50 | — |
| `values.yaml` extensions (4 sprints cumulés) | toutes | +150 | — |
| `values.schema.json` extensions | toutes | +200 | — |
| `README.md` extensions (sections cert-manager + DR drill) | toutes | +100 | — |
| `CHANGELOG.md` per-sprint entries | toutes | +250 | — |
| Pin tests Python | toutes | +600 (Python) | — |
| **Total** | — | **~+2 200 LOC YAML/JSON/MD** | **~+260 LOC bash** |

---

## §20. Plan d'implémentation par sprint

### §20.1 Sprint D21 (3 jours, J60–J62)

* J1 : NetworkPolicy strict template + values.yaml extension.
* J2 : Polaris config + run audit + fix dangers.
* J3 : Tests (helm-unittest + helm test pod) + CHANGELOG.

### §20.2 Sprint D22 (4 jours, J70–J73)

* J1 : `templates/mtls-cert.yaml` + helper.
* J2 : Deployment extension (volumeMounts) + tests rendering.
* J3 : README cert-manager prereq + ClusterIssuer fixtures CI.
* J4 : Tests + CHANGELOG.

### §20.3 Sprint D27 (3 jours, J130–J132)

* J1 : `bundle-reload-hook.yaml` Job + RBAC.
* J2 : Helper `efe.bundle.hash` + Deployment annotation.
* J3 : Tests + CHANGELOG.

### §20.4 Sprint D40 (4 jours, J200–J203)

* J1 : `dr-drill-quarterly.yaml` CronJob + ConfigMap.
* J2 : `dr-drill-script.sh` complete logic.
* J3 : RBAC + helm-unittest tests.
* J4 : End-to-end test + CHANGELOG + GA v1.0.0 + tag + OCI publish.

---

## §21. Versioning + publication Helm chart (OCI registry)

### §21.1 Trajectoire semver

```
0.6.0 (post-D18 + A11) ─→ 0.7.0 (D21) ─→ 0.8.0 (D22) ─→ 0.9.0 (D27) ─→ 1.0.0 (D40 GA)
```

### §21.2 Private publish (Φ-A → Φ-D pré-GA)

GitHub Packages helm registry :

```bash
helm package deploy/helm/efe/
helm push efe-0.7.0.tgz oci://ghcr.io/efe-ai/charts-private --insecure-skip-tls-verify=false
```

### §21.3 Public publish (GA D40)

OCI registry public :

```bash
# Bump Chart.yaml version + appVersion to 1.0.0.
# Tag git commit v1.0.0.
git tag -s v1.0.0 -m "Helm chart v1.0.0 GA — D40 ship"
git push origin v1.0.0

# Publish to public OCI registry.
helm package deploy/helm/efe/
helm push efe-1.0.0.tgz oci://ghcr.io/efe-ai/charts
```

### §21.4 Consumer install

```bash
# Public.
helm install efe oci://ghcr.io/efe-ai/charts/efe --version 1.0.0 \
    -n efe --create-namespace -f my-values.yaml
```

---

## §22. Compliance + sécurité

* **Pod Security Admission** `restricted` mode (D21).
* **NetworkPolicy** default-deny (D21).
* **mTLS** certs via cert-manager auto-rotation (D22).
* **RBAC** least-privilege (D27 + D40 ClusterRole scoped).
* **Audit logging** (existing post-D15).
* **Backup** + DR drill quarterly (D40).
* **SOC 2** audit trail : DR drill reports archived for 1 year via
  `successfulJobsHistoryLimit: 4` (4 trimestres).
* **No raw secrets in values.yaml** — toujours `existingSecret`.

---

## §23. Annexes — `values.yaml` canonical (post-M12)

[Voir §8 pour le diff par sprint. La version finale post-M12 fait
~400 lignes dont ~150 lignes nouvelles. Block complet annexé en
§23.4 du document — non-inline pour brevité ici.]

---

## §24. Annexes — liste complète des templates (post-M12)

[Voir §6.1 pour le layout. Total post-M12 = 17 templates dans
`templates/` + 4 dans `templates/tests/` + 5 dans `tests/`.]

---

## §25. Annexes — Polaris config + helm-unittest fixtures

[Voir §9.3 pour `.polaris.yaml`. helm-unittest fixtures détaillés
dans §13.2 + §9.4 + §10.7 + §11.5 + §12.6.]

---

## §26. Annexes — RACI + budget + Gantt par sprint

### §26.1 RACI

| Activity | R | A | C | I |
|---|:-:|:-:|:-:|:-:|
| Architecture doc (this) | rodrigue | rodrigue | — | team |
| D21 NetworkPolicy + Polaris | rodrigue | rodrigue | sec-eng | team |
| D22 mTLS cert-manager | rodrigue | rodrigue | platform-eng | team |
| D27 Bundle reload Job | rodrigue | rodrigue | ml-eng | team |
| D40 DR drill quarterly | rodrigue | rodrigue | sre + sec-eng | team |
| OCI registry GA publish | rodrigue | platform-eng | rodrigue | team |

### §26.2 Budget temps cumulé

14 jours wallclock × 1 dev = 112 h.

| Phase | Heures | Cumulé |
|---|---:|---:|
| D21 NetworkPolicy + Polaris | 24 | 24 |
| D22 mTLS cert-manager | 32 | 56 |
| D27 Bundle reload | 24 | 80 |
| D40 DR drill + GA | 32 | 112 |

### §26.3 Gantt 90 jours

```
J60-62  │██ D21 NetworkPolicy + Polaris
        │
J70-73  │       ███ D22 mTLS cert-manager
        │
J130-132│                              ██ D27 Bundle reload
        │
J200-203│                                                          ███ D40 DR drill + GA v1.0.0
```

---

## §27. Mises à jour du document

| Date | Version | Auteur | Diff |
|---|---|---|---|
| 2026-04-30 | v1.0.0 | rodrigue | Création initiale (extension multi-sprint M12) |

---

**Fin du document EFE_HELM_CHART_EXTENSION_ARCHITECTURE.md (M12 D21+D22+D27+D40).**
