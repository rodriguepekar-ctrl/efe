# Changelog — `deploy/helm/efe/` Helm chart

All notable changes to the EFE Helm chart documented per
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) +
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Mapping** — Chart `version` ↔ EFE platform sprint Dx :
>
> | Chart | Sprint | App | Phase |
> |---|---|---|---|
> | 0.1.0 | D2 | 0.1.0 | bootstrap |
> | 0.6.0 | D18 + A11 dedup | 0.3.0 | post-audit |
> | 0.7.0 | **D21** | 0.3.0 | M12 Φ-A |
> | 0.8.0 | **D22** | 0.4.0 | M12 Φ-B |
> | 0.9.0 | **D27** | 0.4.0 | M12 Φ-C |
> | **1.0.0** | **D40** | 1.0.0 | **M12 GA** |

## [Unreleased]

### In progress (Sprint **M12 — Helm chart extension** — D21 + D22 + D27 + D40 — **livraison multi-sprints**)

> **Portée M12** : 4 sprints d'extension du chart mature post-D18+A11
> dedup. NEW : 4 templates + 1 helper file + 1 Polaris config + 4
> helm-unittest fixtures + 4 helm test pods + values extension. Pin
> tested anti-régression Python.

#### Code drop M12 — par sprint

##### Sprint D21 (Φ-A) — NetworkPolicy strict + Polaris validation GREEN

* **`.polaris.yaml`** (NEW, 75 LOC) — Polaris validation config
  pinned : **9 checks à `danger`** (hostNetworkSet, hostPIDSet,
  hostIPCSet, hostPortSet, privilegeEscalationAllowed, runAsRootAllowed,
  runAsPrivileged, dangerousCapabilities, tagNotSpecified) + **12
  à `warning`** (notReadOnlyRootFilesystem, insecureCapabilities,
  automountServiceAccountToken, linuxHardening, cpu/memory
  Requests/Limits, liveness/readiness Probe, pullPolicyNotAlways,
  hostAliasesSet) + **1 à `ignore`** (priorityClassNotSet —
  operator-specific). Exemptions justifiées pour CronJobs (efe-backup,
  efe-dr-drill, efe-dr-drill-quarterly, efe-bundle-reload) + helm test
  pods (efe-test-{connection,networkpolicy,mtls-handshake,bundle-reload})
  — lifecycle court → exempts probes + QoS.
* **`templates/d21-networkpolicy-strict.yaml`** (NEW, 110 LOC) —
  NetworkPolicy strict default-deny + tenant ingress (label-selector)
  + CIDR allow-list + monitoring scrape ingress + DNS/audit/S3/OIDC/
  cert-manager egress. Gated par `.Values.networkPolicyStrict.enabled`
  (default false). Coexiste avec le `templates/networkpolicy.yaml`
  basic AS-IS (ne le remplace pas — opt-in opérateur).

##### Sprint D22 (Φ-B) — mTLS via cert-manager

* **`templates/d22-mtls-cert.yaml`** (NEW, 80 LOC) — `Certificate`
  cert-manager.io/v1 avec : duration 8760h (1 an), renewBefore
  720h (30j), ECDSA P-256 (vs RSA), rotationPolicy=Always, usages
  {server auth, client auth}. PREREQUISITE : cert-manager ≥ v1.13
  + ClusterIssuer/Issuer fourni par l'opérateur.
* **`templates/d22-mtls-helpers.tpl`** (NEW, 40 LOC) — helpers
  `efe.mtls.secretName` (derived OR user-provided) + `efe.bundle.hash`
  (SHA-256 court 16 chars depuis slug+version+manifestSha256).
* Le Deployment AS-IS lit `.Values.mtls.enabled` pour mounter le
  Secret cert-manager — pas de modification template requise (le
  pattern existait déjà post-D18 audit fix M7-D22-A3).

##### Sprint D27 (Φ-C) — Live bundle reload Helm post-upgrade hook

* **`templates/d27-bundle-reload-hook.yaml`** (NEW, 130 LOC) — Job
  `helm.sh/hook: post-upgrade` qui exec'e `wget -POST` vers le
  service admin pour swap le bundle sans rolling restart. Idempotent :
  fetch CURRENT hash via `/admin/bundle-info` ; skip si === expected.
  Sinon POST `{manifestSha256}` à `/admin/bundle-reload` avec 3
  retries + backoff 10s. Image canonical `bitnami/kubectl:1.30`
  (ships sh + wget). securityContext strict (runAsNonRoot:65532 +
  readOnlyRootFilesystem + drop ALL caps + RuntimeDefault seccomp).
* **`templates/d27-bundle-reload-rbac.yaml`** (NEW, 75 LOC) —
  ServiceAccount + Role + RoleBinding triplet, gated par hook
  weights 1/2/3 (créés AVANT le Job weight=10) + cleanup auto via
  `hook-delete-policy: hook-succeeded,before-hook-creation`.
  Permissions least-privilege : `get/list pods/services/configmaps`
  + optional `pods/exec` si `useKubectlExec=true`.

##### Sprint D40 (Φ-D) — DR drill quarterly (déjà livré post-audit)

* **`templates/d40-dr-drill-quarterly-cronjob.yaml`** déjà existe
  post-D17/D40 audit pass (M7-D40-A6 + M7-D40-A17 + M7-D40-A24
  defects fixed). M12 livre la **validation pin test** confirmant
  le schedule quarterly + Object Lock COMPLIANCE 7-year retention
  + 4 compliance frameworks asserted via labels (SOC 2 CC9.1 / ISO
  27001 A.17.1.3 / NIS2 Art21.2(c) / RGPD Art32.1(c,d)).

#### Values.yaml extension (M12)

* **`values.m12-extension.yaml`** (NEW, 130 LOC) — fragment
  documentant les 4 nouvelles sections values : `networkPolicyStrict`
  (D21), `mtls` (D22), `bundle` + `bundleReload` (D27). Toutes
  opt-in (default `enabled: false`) → backward-compat pre-M12.

#### Tests + CI

* **`tests/networkpolicy-strict_test.yaml`** (NEW) — 6 helm-unittest
  cases (D21).
* **`tests/mtls_cert_test.yaml`** (NEW) — 5 helm-unittest cases (D22).
* **`tests/bundle_reload_test.yaml`** (NEW) — 4 helm-unittest cases (D27).
* **`tests/dr_drill_quarterly_test.yaml`** (NEW) — 5 helm-unittest
  cases (D40 verification).
* **`templates/tests/test-networkpolicy.yaml`** (NEW) — helm test
  Pod (D21).
* **`templates/tests/test-mtls-handshake.yaml`** (NEW) — helm test
  Pod (D22).
* **`templates/tests/test-bundle-reload.yaml`** (NEW) — helm test
  Pod (D27).
* **`.github/workflows/helm-test.yml`** (NEW) — chart-testing (`ct`)
  + Polaris audit + helm-unittest CI gate.
* **`.github/workflows/helm-publish.yml`** (NEW) — `helm push` to
  OCI registry (GHCR) on tag `v*.*.*`.

#### Pin tests anti-régression structurelle (M12)

* **`scripts/tests/test_m12_helm_extension.py`** (NEW, 25 pin
  tests) répartis en 8 sections : Chart.yaml versioning + Polaris
  config + NetworkPolicy strict + mTLS Certificate + Bundle reload
  Job + Bundle reload RBAC + DR drill quarterly verification +
  helm-unittest fixtures.

#### Notes

* **Trajectoire semver chart** : 0.6.0 (post-D18+A11) → 0.7.0
  (D21 ship) → 0.8.0 (D22 ship) → 0.9.0 (D27 ship) → **1.0.0**
  (D40 GA + OCI public publish).
* **Backward compatibility** : tous les nouveaux templates sont
  gated par `{{- if .Values.<feature>.enabled }}` block, default
  `false`. `helm upgrade` depuis une release pre-M12 ne casse rien
  sans opt-in explicite.
* **Polaris GREEN** : 0 dangers across default + staging + prod
  + tier2 values profiles. Validation : `polaris audit --severity
  danger --output-format pretty < /tmp/manifests.yaml` retourne
  exit 0.
* **Pattern récurrent "Helm hook idempotency"** — 1ère occurrence
  M12 (D27 bundle reload). Tout futur Helm hook doit être idempotent
  via fetch-current-state + skip-if-already-target. Pin test
  institutionnel : tout Job dans `templates/*-hook.yaml` doit avoir
  un check d'état avant l'action.
* **Pattern récurrent "Sprint-tagged template files"** — 1ère
  occurrence M12. Les templates M12 nouveaux utilisent le préfixe
  `dXX-` (e.g., `d22-mtls-cert.yaml`) pour faciliter l'audit
  trace + le rollback par sprint. Pattern à appliquer pour tout
  nouveau template chart.
