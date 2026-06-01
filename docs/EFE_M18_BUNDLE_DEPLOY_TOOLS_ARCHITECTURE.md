# Sprint M18 — Bundle & deploy tools : architecture

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : suit M17 (Advanced agent-native tools livré + audit pass
> 1 — 8 défauts corrigés, 160 pin tests M17 + 392 aggregate GREEN).
>
> **Pré-requis** : avoir lu :
> - [`EFE_M17_ADVANCED_AGENT_TOOLS_ARCHITECTURE.md`](EFE_M17_ADVANCED_AGENT_TOOLS_ARCHITECTURE.md)
>   (M17, blueprint MCP server avec 20 outils + 6 resources + 5 prompts
>   + sampling + streaming + 4 profiles `core`/`advanced`/`agent-native`/
>   `experimental`)
> - [`EFE_BUNDLES_EXTENSION_ARCHITECTURE.md`](EFE_BUNDLES_EXTENSION_ARCHITECTURE.md)
>   (M13, base bundle Ed25519 + SBOM CycloneDX 1.5 + OCI Artifacts +
>   bundle ID format §3.2 avec `manifestSha256`)
> - [`EFE_HELM_CHART_EXTENSION_ARCHITECTURE.md`](EFE_HELM_CHART_EXTENSION_ARCHITECTURE.md)
>   (M12, Helm chart 1.0 GA + D27 bundle-reload Helm hook)
> - [`EFE_COMPLIANCE_EXTENSION_ARCHITECTURE.md`](EFE_COMPLIANCE_EXTENSION_ARCHITECTURE.md)
>   (M14, threat model 86 STRIDE cumulés)
> - `mcp-server/` (M16+M17 livré, 160 pin tests GREEN)
> - **SLSA spec** : <https://slsa.dev/spec/v1.0/levels> (Levels 0-4)
> - **CycloneDX 1.5** : <https://cyclonedx.org/specification/overview/>
> - **Sigstore cosign** : <https://docs.sigstore.dev/cosign/overview/>
> - **OCI Distribution Spec v1.1** :
>   <https://github.com/opencontainers/distribution-spec/blob/v1.1.0/spec.md>
>
> **Mission M18** : exposer la **chaîne d'approvisionnement** EFE
> (bundles signés Ed25519 + SBOM + provenance SLSA + déploiement Helm)
> aux agents LLM via MCP, avec une **frontière de sécurité
> chirurgicale** : le serveur MCP **ne mute jamais l'état du cluster**.
> Pas de `kubectl`, pas de `helm`, pas de `cosign`, pas de subprocess.
> L'agent fait l'analyse + planification ; l'opérateur humain conserve
> l'autorité d'exécution avec ses propres credentials.
>
> Quatre sous-livraisons :
>
> | Sprint | Phase | Jour | Livrable |
> |---|:---:|:---:|---|
> | **D47** | Φ-A | J225 | **5 outils introspection** : `efe_list_available_bundles`, `efe_inspect_bundle`, `efe_verify_bundle_signature`, `efe_diff_bundles`, `efe_check_bundle_compatibility`. Lib pure-Python `cryptography` pour Ed25519 ; aucun subprocess. |
> | **D48** | Φ-B | J235 | **5 nouveaux MCP resources** (`efe://bundles/...`) + **3 prompts** (`verify_bundle_supply_chain`, `audit_bundle_provenance`, `weekly_bundle_health_report`). Parsing SBOM CycloneDX + SLSA provenance attestation. |
> | **D49** | Φ-C | J245 | **5 outils deployment helpers** : `efe_get_deployment_status`, `efe_get_deployment_history`, `efe_dry_run_deploy`, `efe_generate_helm_values`, `efe_generate_rollback_plan`. Tout en read-only ou text-generation. |
> | **D50** | Φ-D | J255 | **3 outils write avec confirm:true** : `efe_register_bundle`, `efe_promote_bundle`, `efe_request_deployment` (crée un ticket Jira/Linear/GitHub Issue, ne déploie pas). Nouveau profile `agent-native-deploy`. Audit pass M18 + freeze SLOs. |
>
> **Total livraison M18** : **13 nouveaux outils MCP** (de 20 → 33),
> **5 resource types** (`efe://bundles/...`), **5 prompt templates**.
> Volumétrie estimée : **~2 600 LOC code Python** + **~1 600 LOC tests**
> + **~300 LOC Rust** (3 nouveaux endpoints EFE 1.2). Additif sur
> `mcp-server/`.
>
> **Compliance posture EFE 1.0 GA toujours préservée** : zéro
> sub-processor ajouté ; aucun secret cryptographique manipulé par le
> MCP server (pas de signature, juste vérification de signatures
> existantes avec clés publiques). Conforme **EU Cyber Resilience Act
> 2024/2847** + **SLSA Level 3** (lecture only) + **NIS2 Article 21**
> (supply chain security).
>
> **Pain points M18 résolus** :
>
> 1. **Chaîne d'approvisionnement opaque pour l'agent** : un agent
>    voulant comprendre « ce bundle déployé est-il signé / d'où vient-
>    il / quand a-t-il été produit » doit aujourd'hui (M17) :
>    `efe_get_admin_bundle_info` (mirror M15-A5) → string `bundle_id`
>    → demander à l'humain de faire `cosign verify` manuellement →
>    coller le résultat dans la conversation. **6+ étapes humain-in-the-
>    loop**. M18 livre `efe_verify_bundle_signature` qui retourne
>    `{verified: true, signed_by: "ed25519/abc123", slsa_level: 3,
>    provenance: {...}}` en un seul appel.
> 2. **Pas de visibilité bundle-vs-deployed** : impossible aujourd'hui
>    de répondre « est-ce que la version v3.2.0 que je veux déployer
>    est compatible avec l'EFE server actuel (v1.1.5) ? ». M18 livre
>    `efe_check_bundle_compatibility` qui croise les contraintes
>    (`required_efe_server_version`, `required_dim`, `required_features`)
>    contre le `/info` du backend.
> 3. **Dry-run deployment manuel** : pour comprendre l'impact d'un
>    `helm upgrade --set bundle.slug=X`, l'opérateur doit `helm template`
>    + `kubectl diff` + lecture humaine. M18 livre `efe_dry_run_deploy`
>    qui retourne un **diff sémantique** (catalogues affectés, classes
>    avec drift, threshold changes, recommendations) sans exécuter helm
>    ni kubectl.
> 4. **Rollback risky** : un opérateur en panique fait un mauvais
>    `helm rollback` → mauvaise version → pire situation. M18 livre
>    `efe_generate_rollback_plan` qui retourne **les commandes
>    suggérées** (`helm rollback efe N`, `kubectl rollout status ...`)
>    + **les pré-vérifications** (vérifier le hash du bundle cible,
>    confirmer audit chain non-brisée), **mais n'exécute rien**.
>    L'opérateur copie-colle dans son terminal sous sa propre identité.
> 5. **SBOM cherry-picking impossible** : « ce bundle contient-il une
>    dépendance vulnérable CVE-2024-XXXX ? » nécessite de pull le bundle,
>    extraire le SBOM, grep CVE — workflow non-natif pour l'agent.
>    M18 livre `efe_get_sbom` qui retourne le CycloneDX 1.5 parsé +
>    une liste de CVEs détectées (intégration optionnelle OSV.dev).
> 6. **Bundle promotion sans peer review** : aujourd'hui pour
>    « promouvoir » un bundle vers un tenant, il faut un git PR + un
>    deploy. M18 livre `efe_promote_bundle` (write, confirm:true) qui
>    appelle un endpoint backend « tag a registered bundle as current ».
>    Mais **toujours pas un déploiement** — c'est juste de la métadonnée
>    backend qui dit « ce slug est ma référence pour ce tenant ». Le
>    déploiement reste manuel.
> 7. **Resources OCI inaccessibles** : les agents qui lisent
>    `efe://bundles/X` doivent aujourd'hui faire `oras pull` →
>    extraction → parsing. M18 livre des MCP resources qui font tout
>    côté serveur, retournent du JSON canonique cliquable.
>
> **Backward compat 100 %** : profile `core` (M16) + `advanced` (M17
> Phase A+B) + `agent-native` (M17 Phase C+D) tous inchangés. M18
> introduit `agent-native-deploy` qui ajoute les 3 outils write
> bundle/deploy. Profiles non-deploy = comportement identique à M17.

---

## Sommaire

1. [Executive summary](#1-executive-summary)
2. [Vision & rationale — pourquoi pas kubectl ?](#2-vision--rationale)
3. [Scope & non-goals (frontière de sécurité explicite)](#3-scope--non-goals)
4. [Catalogue M18 — vue d'ensemble](#4-catalogue-m18--vue-densemble)
5. [Architecture](#5-architecture)
6. [Tool taxonomy — 13 nouveaux outils](#6-tool-taxonomy)
   - 6.1 [Bundle introspection (5)](#61-bundle-introspection)
   - 6.2 [Bundle administration (3 write)](#62-bundle-administration)
   - 6.3 [Deployment helpers (5)](#63-deployment-helpers)
7. [MCP Resources `efe://bundles/...` (5 types)](#7-mcp-resources)
8. [MCP Prompts (5 templates)](#8-mcp-prompts)
9. [Bundle verification pipeline](#9-bundle-verification-pipeline)
10. [SBOM CycloneDX 1.5 integration](#10-sbom-integration)
11. [Ed25519 signature verification (Python-only)](#11-ed25519-verification)
12. [OCI Artifacts integration](#12-oci-artifacts-integration)
13. [Deployment dry-run mechanics](#13-deployment-dry-run-mechanics)
14. [Helm values.yaml generation](#14-helm-values-generation)
15. [JSON Schemas (forward-refs)](#15-json-schemas-forward-refs)
16. [Backend extensions (EFE 1.2 contract)](#16-backend-extensions)
17. [Threat model M18 (delta vs M17)](#17-threat-model-m18)
18. [Performance & SLOs M18](#18-performance--slos-m18)
19. [Compatibility matrix](#19-compatibility-matrix)
20. [Migration M17 → M18 (opt-in deploy profile)](#20-migration-m17--m18)
21. [Test plan (~75 nouveaux pin tests)](#21-test-plan)
22. [Deployment scenarios (delta vs M17)](#22-deployment-scenarios)
23. [Roadmap M19+](#23-roadmap-m19)
24. [Open questions](#24-open-questions)
25. [FAQ](#25-faq)
26. [Glossary M18](#26-glossary-m18)
27. [References](#27-references)

**Annexes**

- [A — Full JSON Schemas (13 nouveaux outils + 5 resources + 5 prompts)](#a--full-json-schemas)
- [B — Bundle URI grammar (formal ABNF)](#b--bundle-uri-grammar)
- [C — Prompt templates (full text)](#c--prompt-templates)
- [D — Pin tests inventory (75 tests)](#d--pin-tests-inventory)
- [E — Backend contract diff EFE 1.1 → 1.2](#e--backend-contract-diff)

---

## 1. Executive summary

| Item | Valeur |
|---|---|
| **Mission** | Exposer la chaîne d'approvisionnement EFE (bundles signés + SBOM + SLSA provenance + déploiement Helm) aux agents LLM via MCP avec une frontière `read+dry-run only` stricte. |
| **Durée** | 4 sprints (D47 → D50), ~38 jours-ingénieur. |
| **Ressources** | 1 senior Python (lead) + 0.75 mid Python (tools impl + tests) + 0.5 senior Rust (3 endpoints EFE 1.2) + 0.25 sécurité (threat-model review + cosign integration) + 0.25 doc. |
| **Budget** | ~$200 k (incluant audit pass post-livraison). |
| **Volumétrie code** | ~2 600 LOC Python `mcp-server/` + ~300 LOC Rust `crates/efe-server/` (3 endpoints) + ~1 600 LOC tests. |
| **Pin tests** | **+75 GREEN** (vs 160 M17 → 235 cumulés mcp-server). |
| **Backward compat** | 100 %. M16/M17 par défaut ; M18 deploy tools = opt-in via `EFE_MCP_TOOL_PROFILE=agent-native-deploy` (nouveau profile). Profiles existants inchangés. |
| **Risk profile** | **Medium** — manipulation crypto en pure Python (ed25519 via `cryptography` lib, déjà battle-tested) + parsing SBOM (JSON, simple) ; mitigation via Phase D audit pass. |
| **Compliance impact** | **Zéro nouveau sub-processor**. Aucune clé privée jamais sur le MCP server (vérification only avec clés publiques). Note additive prévue dans `gdpr_art32.md` §3.6, `soc2_evidence.md` CC9.2 (vendor management), et nouveau doc `compliance/cyber_resilience_act.md`. |
| **EFE 1.2 contract bump** | **Oui** — semver-mineur. 3 nouveaux endpoints REST + 1 RPC gRPC additifs. EFE 1.1 clients restent compatibles avec EFE 1.2 server. M18 deploy profile requiert EFE 1.2 server. |
| **Subprocess execution** | **JAMAIS**. `kubectl`, `helm`, `cosign`, `oras`, `docker` ne sont jamais invoqués. Vérification crypto en pur Python ; OCI access via `httpx` direct ; pas d'eval, pas d'os.system. |

---

## 2. Vision & rationale

### 2.1 Pourquoi M18 ne déploie jamais

EFE est un système de classification semantic critique pour des
workflows enterprise (support routing, content moderation,
compliance triage). Un mauvais déploiement = pannes downstream =
incidents production = pertes financières directes.

**Trois principes inviolables encadrent M18** :

1. **L'opérateur humain garde le contrôle de la mutation.** L'agent
   peut analyser, planifier, recommander ; mais le `helm upgrade`
   final est tapé par un humain dans son propre terminal sous ses
   propres credentials. Cela maintient la traçabilité d'audit
   (l'identité kubectl ≠ l'identité MCP) et donne un point d'arrêt
   psychologique avant changement irréversible.
2. **Aucun subprocess depuis le MCP server.** Pas de
   `subprocess.run(["kubectl", ...])`. Pas de `os.system`. Pas
   d'`eval`. Tout en pur Python : crypto via `cryptography`, OCI via
   `httpx`, parsing JSON. Cela élimine une classe entière d'attaques
   (command injection, escape via env vars) et facilite l'analyse
   statique.
3. **Le rapport généré est texte humain-relisable.** Quand l'agent
   produit un `efe_generate_helm_values`, la sortie est un
   `values.yaml` que l'opérateur lit, comprend, et applique. Pas une
   API magique opaque. Pas de "déploiement automatique sécurisé".
   La transparence est défense.

### 2.2 Pourquoi "MCP-natif" pour cette catégorie

On pourrait imaginer une CLI EFE séparée (`efe-cli inspect-bundle ...`).
Pourquoi tout passer par MCP ?

| Argument | MCP-natif (retenu) | CLI séparée |
|---|:---:|:---:|
| Découvrabilité agent | ✅ liste outils | ❌ doc à grep |
| Schemas typés validés | ✅ JSON Schema 2020-12 | ❌ argparse text |
| Audit chain D14-bis | ✅ chaque appel auditable | ⚠ logs CLI séparés |
| Streaming progress | ✅ `notifications/progress` | ❌ ANSI escape codes |
| Resources @-mention | ✅ `efe://bundles/X` | ❌ |
| Confirm-required UI | ✅ pattern carry-over | ❌ flag `--yes` |
| Multi-provider | ✅ Claude/GPT/Gemini/local via MCP | ❌ CLI Python only |

→ MCP-natif est un win clair pour l'orchestration agent. La CLI peut
exister en parallèle (et existe déjà partiellement via
`scripts/bundle-*.sh`) mais l'expérience-de-référence pour les agents
est MCP.

### 2.3 4 patterns d'usage observés (sources : entrevues 8 design partners
post-M17)

**Pattern A — « Pré-déploiement »** : un opérateur a un nouveau bundle
à déployer (e.g. `support-routing-multilang-v3.2.0`). Veut comprendre
ce qu'il y a dedans + l'impact. **M17** : impossible automatiquement
(nécessite oras pull + manual inspection). **M18** :
`efe_inspect_bundle` + `efe_dry_run_deploy`.

**Pattern B — « Compliance audit »** : un auditeur SOC 2 demande
« quelle est la provenance du bundle déployé ? Qui l'a signé ?
Est-ce SLSA Level 3 ? ». **M17** : nécessite contributor GitHub +
cosign verify manuel. **M18** : prompt
`audit_bundle_provenance` + `efe_verify_bundle_signature`.

**Pattern C — « Incident response »** : prod downtime soupçonné lié à
un bundle récemment promu. L'opérateur veut rollback rapide. **M17** :
nécessite mémoire de la version précédente + helm CLI. **M18** :
`efe_get_deployment_history` + `efe_generate_rollback_plan` →
commandes prêtes à coller.

**Pattern D — « Bundle change review »** : avant un déploiement,
revue de code-style des changements bundle entre v1 et v2. **M17** :
nécessite oras pull × 2 + diff manuel. **M18** : `efe_diff_bundles`
qui retourne un diff sémantique (classes added/removed, threshold
shifts, SBOM dependency diff).

### 2.4 Pourquoi pas un "GitOps agent" ?

Question stratégique tranchée pré-M18 : est-ce que M18 devrait
inclure un agent GitOps (FluxCD/ArgoCD-like) qui réconcilie l'état
désiré ? **Rejeté** parce que :

| Critère | Agent GitOps EFE-managed (rejeté) | M18 read+dry-run MCP (retenu) |
|---|---|---|
| Surface d'attaque | Énorme (cluster mutations) | Minimale (read-only + text gen) |
| Compliance | Demande revue SOC 2 dédiée | Note additive seulement |
| Latence panic-rollback | Incertaine (controller reconciliation) | Sub-second (humain copy-paste) |
| Ownership ops | Dilué (qui owns Argo ?) | Clair (l'opérateur, son terminal) |
| Multi-cluster | Architecture entièrement nouvelle | N/A |
| Vendor lock-in | Argo ou Flux à choisir | Aucun |

→ Si M18 customer veut GitOps il déploie ArgoCD/FluxCD à côté ; M18
les complète sans les remplacer. M18 reste **observabilité +
planification**, pas exécution.

---

## 3. Scope & non-goals

### 3.1 In scope (livraison M18)

| ID | Catégorie | Item | Phase | Profile requis |
|---|---|---|---|---|
| 3.1.1 | Tool | `efe_list_available_bundles` | D47 | agent-native |
| 3.1.2 | Tool | `efe_inspect_bundle` | D47 | agent-native |
| 3.1.3 | Tool | `efe_verify_bundle_signature` | D47 | agent-native |
| 3.1.4 | Tool | `efe_diff_bundles` | D47 | agent-native |
| 3.1.5 | Tool | `efe_check_bundle_compatibility` | D47 | agent-native |
| 3.1.6 | Tool | `efe_get_sbom` | D48 | agent-native |
| 3.1.7 | Tool | `efe_get_deployment_status` | D49 | agent-native |
| 3.1.8 | Tool | `efe_get_deployment_history` | D49 | agent-native |
| 3.1.9 | Tool | `efe_dry_run_deploy` | D49 | agent-native |
| 3.1.10 | Tool | `efe_generate_helm_values` | D49 | agent-native |
| 3.1.11 | Tool | `efe_generate_rollback_plan` | D49 | agent-native |
| 3.1.12 | Tool (W) | `efe_register_bundle` | D50 | agent-native-deploy |
| 3.1.13 | Tool (W) | `efe_promote_bundle` | D50 | agent-native-deploy |
| 3.1.14 | Tool (W) | `efe_request_deployment` | D50 | agent-native-deploy |
| 3.1.15 | Resources | `efe://bundles` | D48 | agent-native |
| 3.1.16 | Resources | `efe://bundles/{slug}/{version}` | D48 | agent-native |
| 3.1.17 | Resources | `efe://bundles/{slug}/{version}/sbom` | D48 | agent-native |
| 3.1.18 | Resources | `efe://bundles/{slug}/{version}/signature` | D48 | agent-native |
| 3.1.19 | Resources | `efe://deployments/current` | D49 | agent-native |
| 3.1.20 | Prompt | `verify_bundle_supply_chain` | D48 | agent-native |
| 3.1.21 | Prompt | `audit_bundle_provenance` | D48 | agent-native |
| 3.1.22 | Prompt | `weekly_bundle_health_report` | D48 | agent-native |
| 3.1.23 | Prompt | `deploy_new_bundle` | D49 | agent-native-deploy |
| 3.1.24 | Prompt | `rollback_to_previous_bundle` | D49 | agent-native |
| 3.1.25 | Backend | EFE 1.2 contract bump (3 REST + 1 gRPC additifs) | D49-D50 | — |
| 3.1.26 | Profile | `agent-native-deploy` (nouveau, opt-in) | D50 | — |
| 3.1.27 | Audit | M18 audit pass post-livraison | D50 | — |

### 3.2 Out of scope (forever)

| Item | Pourquoi exclu | Justification |
|---|---|---|
| `kubectl` exécution | Mutation cluster | Hors threat boundary M18 |
| `helm install/upgrade/rollback/uninstall` | Mutation cluster | Hors threat boundary M18 |
| `cosign sign` | Manipulation clé privée | Risque key-leak inacceptable |
| `cosign attest` | Génération attestation | Idem |
| `docker build/push` | Mutation registry | Hors threat boundary |
| `oras push` | Mutation OCI registry | Hors threat boundary |
| `argocd sync`, `flux reconcile` | Mutation cluster | Idem |
| Subprocess de toute commande | command injection risk | Boundary architecturale |
| Production secret rotation | Requires HSM | Out of MCP scope |
| Certificate signing | Requires CA | Out of MCP scope |
| Direct cluster access (kubeconfig) | Lateral movement risk | Backend gate only |
| HSM/KMS interactions | Privileged ops | M19 perhaps via dedicated audited tool |
| GitHub/Gitea write (PR creation) | Side-effect on git history | M19 perhaps |

### 3.3 Out of scope (deferred to M19+)

- Bundle authoring helpers (création de nouveau bundle from scratch)
- Multi-cluster deployment plans
- A/B deployment helpers (canary, blue-green)
- Chaos engineering helpers
- Cluster autoscaling recommendations

### 3.4 Hard constraints

- **Aucune dépendance Python critique nouvelle imposée** : `cryptography`
  (Ed25519 verify) est déjà commune ; sera dans extra `[bundle]`.
- **Aucune permission cluster requise** par le MCP server runtime
  (read-only via backend API ; pas de kubeconfig sur le serveur MCP).
- **OCI registry access** : peut être anonyme pour publics, ou via
  bearer token dans `EFE_OCI_REGISTRY_TOKEN` env var pour privés.
  Token injection préventive : registry URL whitelist enforced
  (`EFE_OCI_REGISTRY_ALLOWLIST=registry.example.com,ghcr.io`).
- **Profile par défaut = `core` (M16)**. M18 deploy = explicit opt-in
  via `agent-native-deploy`.

---

## 4. Catalogue M18 — vue d'ensemble

### 4.1 Outils (33 cumulés au final)

```
                       ┌──────────────────────────────────────┐
                       │  EFE MCP Server v0.3.0 (M18 GA)      │
                       │  Profile : core | advanced |         │
                       │            agent-native |            │
                       │            agent-native-deploy ← NEW │
                       └─────────────────┬────────────────────┘
                                         │
   ┌─────────────────────────────────────┼─────────────────────────────────────┐
   │                                                                            │
M16 core (6) ─ CRUD                  M17 advanced (8) ─ reasoning/diag        │
M17 agent-native (6) ─ sampling/streaming                                     │
                                                                              │
                M18 agent-native — read-only (10) — D47+D48+D49               │
                ──────────────────────────────────────────────                │
                • efe_list_available_bundles                                  │
                • efe_inspect_bundle                                          │
                • efe_verify_bundle_signature                                 │
                • efe_diff_bundles                                            │
                • efe_check_bundle_compatibility                              │
                • efe_get_sbom                                                │
                • efe_get_deployment_status                                   │
                • efe_get_deployment_history                                  │
                • efe_dry_run_deploy                                          │
                • efe_generate_helm_values                                    │
                • efe_generate_rollback_plan                                  │
                                                                              │
                M18 agent-native-deploy — write (3) — D50                     │
                ────────────────────────────────────────                      │
                • efe_register_bundle      (W)                                │
                • efe_promote_bundle       (W)                                │
                • efe_request_deployment   (W) — files a ticket, never deploys│

   Légende :
   W   = write tool (confirm:true requis)
   ※   = deploy profile only
```

### 4.2 Resources (5 nouveaux types, D48+D49)

| URI pattern | Lecture | Description |
|---|:---:|---|
| `efe://bundles` | R | Liste de tous les bundles connus du backend |
| `efe://bundles/{slug}/{version}` | R | Métadonnées bundle (manifest + signature status) |
| `efe://bundles/{slug}/{version}/sbom` | R | SBOM CycloneDX 1.5 parsé |
| `efe://bundles/{slug}/{version}/signature` | R | Artefact signature + provenance SLSA |
| `efe://deployments/current` | R | État du déploiement actuel (bundle déployé + helm chart version) |

### 4.3 Prompts (5 templates, D48+D49)

| ID | Variables | Profile | Phase |
|---|---|---|---|
| `verify_bundle_supply_chain` | `{slug}`, `{version}` | agent-native | D48 |
| `audit_bundle_provenance` | `{slug?}`, `{version?}` | agent-native | D48 |
| `weekly_bundle_health_report` | (no args) | agent-native | D48 |
| `deploy_new_bundle` | `{slug}`, `{version}`, `{environment?}` | agent-native-deploy | D49 |
| `rollback_to_previous_bundle` | `{environment?}`, `{reason}` | agent-native | D49 |

---

## 5. Architecture

### 5.1 Module map (delta vs M17)

```
mcp-server/
├── pyproject.toml                  ← UPDATE : new extras [bundle]
│                                              cryptography>=41.0
├── Dockerfile                      ← UPDATE : add cryptography in [bundle]
├── README.md                       ← UPDATE : profile section + bundle tools
├── mcp_server/
│   ├── __init__.py                 ← UPDATE : version 0.3.0
│   ├── efe_mcp_server.py           ← UPDATE : new profile
│   ├── dispatcher.py               ← UPDATE : weighted rate-limit for inspect+sbom
│   ├── backend_client.py           ← UPDATE : EFE 1.2 endpoints (deployment status, dry-run)
│   ├── auth.py                     ← UNCHANGED
│   ├── audit_emitter.py            ← UPDATE : bundle action codes
│   ├── rate_limiter.py             ← UNCHANGED
│   ├── profile.py                  ← UPDATE : Profile.AGENT_NATIVE_DEPLOY
│   ├── sampling.py                 ← UNCHANGED
│   ├── streaming.py                ← UNCHANGED
│   ├── resources/                  ← UPDATE : add bundle/deployment URIs
│   │   ├── __init__.py
│   │   ├── registry.py             ← UPDATE : 5 new resource types + URI grammar
│   │   ├── catalogs.py             ← UNCHANGED
│   │   ├── bundles.py              ← NEW : 4 bundle resource handlers
│   │   ├── deployments.py          ← NEW : 1 deployment resource handler
│   │   └── ...
│   ├── prompts/                    ← UPDATE : 5 new templates
│   │   ├── __init__.py
│   │   ├── registry.py             ← UPDATE : new templates registered
│   │   ├── verify_bundle_supply_chain.md  ← NEW
│   │   ├── audit_bundle_provenance.md     ← NEW
│   │   ├── weekly_bundle_health_report.md ← NEW
│   │   ├── deploy_new_bundle.md           ← NEW
│   │   └── rollback_to_previous_bundle.md ← NEW
│   ├── analysis/                   ← UNCHANGED
│   ├── state/                      ← UNCHANGED + add bundle_cache
│   │   ├── __init__.py
│   │   ├── embedding_cache.py
│   │   ├── sampling_cache.py
│   │   └── bundle_cache.py         ← NEW : LRU+TTL pour OCI artifacts
│   ├── bundle/                     ← NEW directory : pure-Python crypto + parsing
│   │   ├── __init__.py
│   │   ├── verify.py               ← Ed25519 verification
│   │   ├── sbom.py                 ← CycloneDX 1.5 parsing
│   │   ├── slsa.py                 ← SLSA provenance attestation parsing
│   │   ├── manifest.py             ← Bundle manifest D16 §3.2 parsing
│   │   ├── compat.py               ← Compatibility matrix logic
│   │   └── oci.py                  ← OCI registry HTTP client (read-only)
│   ├── deploy/                     ← NEW directory : dry-run + helm/rollback gen
│   │   ├── __init__.py
│   │   ├── dry_run.py              ← Compute deployment diff
│   │   ├── helm_values.py          ← Generate values.yaml from args
│   │   └── rollback.py             ← Generate rollback runbook
│   └── tools/                      ← UPDATE : 13 new tool handlers
│       ├── __init__.py
│       ├── registry.py             ← UPDATE : profile-gated registration
│       ├── ...                     ← (M16+M17 tools unchanged)
│       ├── list_available_bundles.py    ← NEW (D47)
│       ├── inspect_bundle.py            ← NEW (D47)
│       ├── verify_bundle_signature.py   ← NEW (D47)
│       ├── diff_bundles.py              ← NEW (D47)
│       ├── check_bundle_compatibility.py ← NEW (D47)
│       ├── get_sbom.py                  ← NEW (D48)
│       ├── get_deployment_status.py     ← NEW (D49)
│       ├── get_deployment_history.py    ← NEW (D49)
│       ├── dry_run_deploy.py            ← NEW (D49)
│       ├── generate_helm_values.py      ← NEW (D49)
│       ├── generate_rollback_plan.py    ← NEW (D49)
│       ├── register_bundle.py           ← NEW (D50, write)
│       ├── promote_bundle.py            ← NEW (D50, write)
│       └── request_deployment.py        ← NEW (D50, write)
└── tests/
    ├── conftest.py                 ← UPDATE : bundle fixtures + fake OCI
    ├── test_tool_schemas.py        ← UPDATE : +13 new tool schemas covered
    ├── test_tool_dispatch.py       ← UPDATE : +13 happy paths
    ├── ...
    ├── test_profile_gating.py      ← UPDATE : +agent-native-deploy
    ├── test_bundle_verify.py             ← NEW (D47, ~12 tests)
    ├── test_bundle_introspection.py      ← NEW (D47, ~10 tests)
    ├── test_bundle_diff.py               ← NEW (D47, ~6 tests)
    ├── test_bundle_compat.py             ← NEW (D47, ~5 tests)
    ├── test_sbom_parsing.py              ← NEW (D48, ~8 tests)
    ├── test_slsa_provenance.py           ← NEW (D48, ~6 tests)
    ├── test_oci_client.py                ← NEW (D47, ~7 tests)
    ├── test_deployment_dry_run.py        ← NEW (D49, ~8 tests)
    ├── test_helm_values_generation.py    ← NEW (D49, ~6 tests)
    ├── test_bundle_resources.py          ← NEW (D48, ~7 tests)
    └── test_audit_pass_m18.py            ← NEW post-livraison (D50)

crates/efe-server/                  ← UPDATE for EFE 1.2 contract
├── src/
│   ├── http/routes.rs              ← UPDATE : +3 endpoints
│   ├── grpc/service.rs             ← UPDATE : +1 RPC
│   └── deployment/                 ← NEW
│       ├── current_state.rs        ← Get current deployment metadata
│       ├── history.rs              ← Deployment history (last N)
│       └── dry_run.rs              ← Compute deployment diff
└── tests/
    └── m18_contract.rs             ← NEW (~20 tests EFE 1.2)
```

### 5.2 Process & threading model

Inchangé vs M17 (asyncio single-event-loop). Notes spécifiques M18 :

- **OCI registry HTTP calls** : `httpx.AsyncClient` réutilisé du
  backend client. Cache LRU `bundle_cache` (per-process) pour éviter
  re-pull du même bundle dans une session.
- **Crypto verification (Ed25519)** : pure Python via `cryptography`,
  ~1-2 ms par signature. Pas de subprocess. Si la lib n'est pas
  installée → outil retourne `verification_unavailable: true` et le
  user installe `pip install efe-mcp-server[bundle]`.
- **SBOM parsing** : JSON, instantané (< 5 ms pour SBOM 100 deps).
- **Pas de heavy compute thread offload** nécessaire ici (différent de
  M17 clustering).

### 5.3 New state caches (M18 D47)

`mcp_server/state/bundle_cache.py` — LRU+TTL pour les artefacts OCI :
- Capacity 32 bundles
- TTL 600 s (bundles sont immutables, longue vie)
- Per-tenant scoping (tenant_id dans la clé)
- Bypass via `EFE_MCP_DISABLE_CACHE=true`

Cache content : `{manifest, sbom, signature, slsa_provenance}` parsés.

---

## 6. Tool taxonomy

### 6.1 Bundle introspection

#### 6.1.1 `efe_list_available_bundles`

**Intent** : énumérer les bundles disponibles dans l'OCI registry
configuré (anonymous ou via token), filtrés par tenant scope si
applicable.

**Input** :
```json
{
  "registry": "ghcr.io/efe-project/bundles",   // optionnel, default = backend's
  "filter_slug": "support-routing*",            // glob pattern, optionnel
  "limit": 50                                    // 1..200
}
```

**Output** :
```json
{
  "registry": "ghcr.io/efe-project/bundles",
  "count": 12,
  "bundles": [
    {
      "slug": "support-routing-multilang",
      "version": "v3.2.0",
      "size_bytes": 1450000,
      "created_at": "2026-04-15T10:30:00Z",
      "manifest_sha256": "abc123...",
      "is_signed": true,
      "slsa_level": 3
    }
  ],
  "next_cursor": "opaque-token-or-null"
}
```

**Backend strategy** :
- EFE 1.2 endpoint `GET /admin/bundles?registry=...&filter=...` qui
  proxie l'OCI registry (backend a la connectivité, peut-être pas le
  client MCP).
- Fallback si endpoint absent : MCP server pull direct via `httpx`
  contre l'OCI registry (anonymous ou avec `EFE_OCI_REGISTRY_TOKEN`).

**Latence cible** : p50 200 ms (cached registry catalog), p99 2 s
(cold registry list).

#### 6.1.2 `efe_inspect_bundle`

**Intent** : retourner toutes les métadonnées d'un bundle : manifest
parsé, SBOM résumé (counts), signature status, SLSA provenance résumé.

**Input** :
```json
{
  "slug": "support-routing-multilang",
  "version": "v3.2.0",
  "include_full_sbom": false
}
```

**Output** :
```json
{
  "slug": "support-routing-multilang",
  "version": "v3.2.0",
  "manifest": {
    "manifestSha256": "abc123...",
    "framework": "sentence-transformers",
    "model_name": "paraphrase-multilingual-MiniLM-L12-v2",
    "dim": 384,
    "is_dev_unsigned": false,
    "schema_version": "1.0",
    "produced_at": "2026-04-15T10:30:00Z",
    "produced_by": "github.com/efe-project/efe-bundles@workflow:bundle-release.yml"
  },
  "signature": {
    "is_signed": true,
    "signature_algorithm": "ed25519",
    "verified": true,
    "signed_by_key_id": "ed25519:abc..."
  },
  "sbom_summary": {
    "format": "CycloneDX",
    "spec_version": "1.5",
    "components_count": 87,
    "vulnerabilities_count": 0,
    "dependencies_summary": {
      "python": 42,
      "system": 12,
      "ml_models": 1,
      "data_files": 32
    }
  },
  "slsa_provenance": {
    "level": 3,
    "builder": "github-actions",
    "build_type": "https://slsa-framework.github.io/github-actions-buildtypes/workflow/v1",
    "subject_digest": "sha256:abc..."
  },
  "compatibility": {
    "min_efe_server_version": "1.0.0",
    "max_efe_server_version": null,
    "required_features": ["multilingual"]
  }
}
```

**Algorithm** :
1. Cache lookup `bundle_cache.get(slug, version)`.
2. Si miss : call backend `/admin/bundles/<slug>/<version>` ou direct
   OCI pull via `mcp_server/bundle/oci.py`.
3. Parse manifest, SBOM, signature, SLSA — tout via `mcp_server/bundle/`
   modules.
4. Vérifie signature Ed25519 si clé publique disponible
   (`EFE_OCI_PUBLIC_KEY` env ou backend-served).
5. Cache + return.

**Latence cible** : p50 100 ms (cached), p99 3 s (cold + verify).

#### 6.1.3 `efe_verify_bundle_signature`

**Intent** : focus uniquement sur la vérification cryptographique.

**Input** :
```json
{
  "slug": "support-routing",
  "version": "v3.2.0",
  "expected_key_id": null                      // optionnel, vérifie qu'une clé spécifique a signé
}
```

**Output** :
```json
{
  "slug": "support-routing",
  "version": "v3.2.0",
  "verified": true,
  "signature_algorithm": "ed25519",
  "signed_by_key_id": "ed25519:abc...",
  "signed_at": "2026-04-15T10:30:00Z",
  "expected_key_id_match": null,                  // null si pas demandé
  "details": {
    "manifest_sha256": "abc123...",
    "manifest_sha256_matches": true,
    "signature_present_in_oci_referrers": true,
    "key_in_trust_store": true
  },
  "warnings": []
}
```

**Algorithm** :
1. Pull manifest + signature artifact (OCI Referrers API
   `/v2/<repo>/referrers/<digest>`).
2. Compute `manifest_sha256` localement, compare avec ce qui est signé.
3. Vérifie Ed25519 signature avec clé publique du trust store
   (`EFE_OCI_PUBLIC_KEY` env, ou backend-served `/admin/trust-store`).
4. Si `expected_key_id` fourni, vérifie que la clé correspond.

**Threat addressed** : TM-M18-S-01 (forged signature), TM-M18-T-01
(tampered manifest).

**Latence cible** : p50 80 ms, p99 1.5 s.

#### 6.1.4 `efe_diff_bundles`

**Intent** : diff sémantique entre deux versions d'un même bundle
(ou de deux bundles différents si compatibles).

**Input** :
```json
{
  "slug_a": "support-routing-multilang",
  "version_a": "v3.1.0",
  "slug_b": "support-routing-multilang",
  "version_b": "v3.2.0"
}
```

**Output** :
```json
{
  "slug_a": "support-routing-multilang",
  "version_a": "v3.1.0",
  "slug_b": "support-routing-multilang",
  "version_b": "v3.2.0",
  "manifest_diff": {
    "model_name_changed": false,
    "dim_changed": false,
    "framework_changed": false
  },
  "sbom_diff": {
    "components_added": [
      {"name": "transformers", "version": "4.35.0", "type": "library"}
    ],
    "components_removed": [],
    "components_upgraded": [
      {"name": "torch", "from": "2.0.1", "to": "2.1.0"}
    ],
    "vulnerabilities_added": [],
    "vulnerabilities_resolved": [
      {"cve": "CVE-2024-XXXX", "severity": "MEDIUM", "in_component": "old-lib"}
    ]
  },
  "size_diff_bytes": 12000,
  "size_diff_percent": 0.83,
  "compatibility_breaking": false,
  "recommendation": "Safe upgrade ; 1 CVE resolved, no breaking change."
}
```

**Latence cible** : p50 200 ms, p99 4 s (2 cold pulls).

#### 6.1.5 `efe_check_bundle_compatibility`

**Intent** : un bundle X est-il compatible avec mon EFE server actuel ?

**Input** :
```json
{
  "slug": "support-routing",
  "version": "v3.2.0",
  "target_environment": "production"            // optionnel, default = caller's
}
```

**Output** :
```json
{
  "slug": "support-routing",
  "version": "v3.2.0",
  "compatible": true,
  "checks": [
    {"name": "min_efe_server_version", "required": "1.0.0", "actual": "1.2.0", "ok": true},
    {"name": "max_efe_server_version", "required": null, "actual": "1.2.0", "ok": true},
    {"name": "required_dim", "required": 384, "actual": 384, "ok": true},
    {"name": "required_features", "required": ["multilingual"], "actual": ["multilingual", "drift_detection"], "ok": true},
    {"name": "schema_version", "required": "1.0", "actual": "1.0", "ok": true}
  ],
  "warnings": [],
  "blocking_issues": []
}
```

### 6.2 Bundle administration *(write, agent-native-deploy profile only)*

#### 6.2.1 `efe_register_bundle`

**Intent** : enregistrer un bundle URI dans la registry connue du
backend, sans le déployer. Permet ensuite de le référencer dans
`efe_promote_bundle` ou `efe_dry_run_deploy`.

**Input** :
```json
{
  "slug": "support-routing-multilang",
  "version": "v3.2.0",
  "registry_url": "ghcr.io/efe-project/bundles",
  "verify_before_register": true,
  "confirm": true
}
```

**Output** :
```json
{
  "registered": true,
  "bundle_id": "ghcr.io/efe-project/bundles/support-routing-multilang:v3.2.0",
  "manifest_sha256": "abc...",
  "verified_at_register": true,
  "audit_event_id": "evt_abc..."
}
```

**Backend strategy** : EFE 1.2 endpoint
`POST /admin/bundles/register`. Si endpoint absent → ToolError
`backend_unavailable` (no fallback for write operations).

#### 6.2.2 `efe_promote_bundle`

**Intent** : marquer un bundle enregistré comme « current » pour un
tenant. **N'est PAS un déploiement** — c'est de la métadonnée backend
qui dit « ce slug est ma référence pour ce tenant à partir de
maintenant ». Le déploiement Helm reste manuel (cf. §3.1, §3.2).

**Input** :
```json
{
  "slug": "support-routing-multilang",
  "version": "v3.2.0",
  "scope": "tenant",                             // tenant | global
  "confirm": true
}
```

**Output** :
```json
{
  "promoted": true,
  "scope": "tenant",
  "tenant_id": "acme",
  "previous_current_version": "v3.1.0",
  "new_current_version": "v3.2.0",
  "next_action_hint": (
    "Bundle metadata updated. To actually deploy, run:\n"
    "  helm upgrade efe ./charts/efe \\\n"
    "    --set bundle.slug=support-routing-multilang \\\n"
    "    --set bundle.version=v3.2.0\n"
    "Run efe_dry_run_deploy first to preview the change."
  ),
  "audit_event_id": "evt_def..."
}
```

#### 6.2.3 `efe_request_deployment`

**Intent** : créer un ticket (Jira / Linear / GitHub Issue selon
config backend) pour demander une révision de déploiement à un humain.

**Input** :
```json
{
  "slug": "support-routing-multilang",
  "version": "v3.2.0",
  "environment": "production",
  "rationale": "Adds Italian + Portuguese coverage requested by EU customer success",
  "tracker": "linear",                          // linear | jira | github | internal
  "confirm": true
}
```

**Output** :
```json
{
  "requested": true,
  "tracker": "linear",
  "ticket_id": "EFE-1234",
  "ticket_url": "https://linear.app/efe/issue/EFE-1234",
  "status": "open",
  "next_action_hint": (
    "Review and approval required from #efe-deployments team. "
    "Once approved, run helm upgrade with the values from "
    "efe_generate_helm_values."
  )
}
```

### 6.3 Deployment helpers

#### 6.3.1 `efe_get_sbom`

**Intent** : retourner le SBOM CycloneDX 1.5 d'un bundle, parsé.

**Input** :
```json
{
  "slug": "support-routing",
  "version": "v3.2.0",
  "filter_severity": "MEDIUM",                  // null|LOW|MEDIUM|HIGH|CRITICAL
  "max_components": 200
}
```

**Output** :
```json
{
  "slug": "support-routing",
  "version": "v3.2.0",
  "sbom_format": "CycloneDX",
  "spec_version": "1.5",
  "components": [
    {"name": "torch", "version": "2.1.0", "type": "library", "purl": "pkg:pypi/torch@2.1.0"}
  ],
  "vulnerabilities": [],
  "summary": {
    "total_components": 87,
    "components_with_vulns": 0,
    "highest_severity": null
  }
}
```

#### 6.3.2 `efe_get_deployment_status`

**Intent** : état actuel du déploiement EFE (bundle déployé, version
chart Helm, replicas, embedder).

**Input** : `{}`

**Output** :
```json
{
  "deployed_at": "2026-04-10T08:15:00Z",
  "deployed_by": "ops@efe-project.com",
  "bundle": {
    "slug": "support-routing-multilang",
    "version": "v3.1.0",
    "manifest_sha256": "old123..."
  },
  "helm": {
    "chart_version": "1.0.5",
    "release_name": "efe",
    "namespace": "efe-prod"
  },
  "replicas": {
    "desired": 3,
    "ready": 3,
    "available": 3
  },
  "health": "healthy",
  "uptime_seconds": 432000
}
```

#### 6.3.3 `efe_get_deployment_history`

**Intent** : last N deployments + their bundles + outcomes.

**Input** :
```json
{ "limit": 10 }
```

**Output** :
```json
{
  "count": 5,
  "deployments": [
    {
      "deployed_at": "2026-04-10T08:15:00Z",
      "deployed_by": "ops@efe-project.com",
      "bundle_slug": "support-routing-multilang",
      "bundle_version": "v3.1.0",
      "outcome": "success",
      "duration_seconds": 180
    }
  ]
}
```

#### 6.3.4 `efe_dry_run_deploy`

**Intent** : simuler un déploiement et retourner le diff sémantique
sans rien modifier.

**Input** :
```json
{
  "slug": "support-routing-multilang",
  "version": "v3.2.0",
  "environment": "production"                   // optionnel
}
```

**Output** :
```json
{
  "current": {
    "bundle": {"slug": "support-routing-multilang", "version": "v3.1.0"},
    "manifest_sha256": "old..."
  },
  "target": {
    "bundle": {"slug": "support-routing-multilang", "version": "v3.2.0"},
    "manifest_sha256": "new..."
  },
  "diff": {
    "manifest_changed": true,
    "model_name_changed": false,
    "dim_changed": false,
    "expected_replica_restarts": 3,
    "expected_downtime_seconds_p99": 0,            // 0 if rolling update
    "catalog_compatibility_check": [
      {
        "catalog_name": "support-routing",
        "compatible": true,
        "centroid_drift_estimate": 0.04
      }
    ]
  },
  "risks": [
    {
      "level": "LOW",
      "category": "centroid_drift",
      "message": "Catalog 'support-routing' will see ~4% centroid drift due to embedder change. Recommend running efe_calibrate_thresholds after deploy."
    }
  ],
  "blocking_issues": [],
  "recommended_actions": [
    "1. Run efe_verify_bundle_signature for support-routing-multilang:v3.2.0",
    "2. Run efe_check_bundle_compatibility",
    "3. Generate Helm values via efe_generate_helm_values",
    "4. Apply manually : helm upgrade efe ./charts/efe -f values-from-mcp.yaml",
    "5. Post-deploy : run efe_calibrate_thresholds for any catalog with reported drift"
  ]
}
```

#### 6.3.5 `efe_generate_helm_values`

**Intent** : produire un `values.yaml` Helm valide pour un déploiement
donné. **Retourne du texte ; n'apply jamais**.

**Input** :
```json
{
  "slug": "support-routing-multilang",
  "version": "v3.2.0",
  "replicas": 3,
  "include_resources_section": true,
  "include_pdb": true
}
```

**Output** :
```json
{
  "values_yaml": "# Generated by efe_generate_helm_values\n# bundle: support-routing-multilang:v3.2.0\nbundle:\n  slug: support-routing-multilang\n  version: v3.2.0\n  manifest_sha256: abc...\nreplicaCount: 3\nresources:\n  requests:\n    cpu: 500m\n    memory: 1Gi\n  limits:\n    cpu: 2000m\n    memory: 4Gi\npodDisruptionBudget:\n  enabled: true\n  minAvailable: 2\n",
  "size_bytes": 380,
  "next_action_hint": "Save to values-mcp.yaml and apply with `helm upgrade efe ./charts/efe -f values-mcp.yaml --dry-run` first to preview."
}
```

#### 6.3.6 `efe_generate_rollback_plan`

**Intent** : générer un plan de rollback (commandes + pré-checks)
sans rien exécuter.

**Input** :
```json
{
  "to_version": "previous",                     // previous | specific:v3.0.0
  "environment": "production",
  "reason": "p99 latency spike post-deploy"
}
```

**Output** :
```json
{
  "from_version": "v3.2.0",
  "to_version": "v3.1.0",
  "from_manifest_sha256": "new...",
  "to_manifest_sha256": "old...",
  "pre_checks": [
    {
      "name": "verify_target_bundle_signature",
      "command_hint": "(automatic) calling efe_verify_bundle_signature"
    },
    {
      "name": "verify_audit_chain_intact",
      "command_hint": "(automatic) calling efe_audit_health_check deep:true"
    },
    {
      "name": "snapshot_current_state",
      "command_hint": "kubectl get all -n efe-prod -o yaml > /tmp/efe-pre-rollback.yaml"
    }
  ],
  "rollback_commands": [
    "# 1. Verify bundle signature first",
    "# (run efe_verify_bundle_signature in your MCP client)",
    "",
    "# 2. Helm rollback",
    "helm history efe -n efe-prod          # find revision number for v3.1.0",
    "helm rollback efe <REV> -n efe-prod",
    "",
    "# 3. Confirm rollout",
    "kubectl rollout status statefulset/efe -n efe-prod --timeout=10m"
  ],
  "post_checks": [
    "Run efe_get_deployment_status to confirm bundle.version == v3.1.0",
    "Run efe_audit_health_check deep:true to verify chain intact"
  ],
  "estimated_duration_seconds": 300,
  "blast_radius": "single-namespace efe-prod"
}
```

---

## 7. MCP Resources

### 7.1 URI grammar (extension de M17)

```
efe-uri    = "efe://" path [ "?" query ]
path       = (M17 catalogs / audit / quotas / samples)
           / bundles                                    ; M18 NEW
           / deployments                                ; M18 NEW
bundles    = "bundles" [ "/" slug "/" version
                       [ "/" ( "sbom" / "signature" ) ]
                       ]
deployments = "deployments" "/" ( "current" / "history" )
slug       = name-start *99name-cont
version    = "v" 1*3DIGIT "." 1*3DIGIT "." 1*3DIGIT [ "-" tag ]
           ; e.g. v1.2.3, v1.2.3-rc1
tag        = 1*15( ALPHA / DIGIT / "-" )
```

ABNF formelle complète en Annexe B.

### 7.2 5 resource types M18

| URI pattern | Lecture | Description |
|---|:---:|---|
| `efe://bundles` | R | Liste de tous les bundles connus |
| `efe://bundles/{slug}/{version}` | R | Métadonnées bundle complètes |
| `efe://bundles/{slug}/{version}/sbom` | R | SBOM CycloneDX 1.5 (JSON parsé) |
| `efe://bundles/{slug}/{version}/signature` | R | Artefact signature + SLSA provenance |
| `efe://deployments/current` | R | État du déploiement actuel |

### 7.3 ETag + caching

Tous les bundle resources sont **immutables** (si la `version` change,
le bundle change → différent URI). Cela permet un caching agressif :
- ETag = `manifest_sha256`
- `Cache-Control: max-age=86400, immutable`

Resource `efe://deployments/current` est mutable :
- ETag = `deployed_at` timestamp + `manifest_sha256`
- `Cache-Control: max-age=30`

---

## 8. MCP Prompts

### 8.1 5 templates

#### 8.1.1 `verify_bundle_supply_chain` (D48)

```markdown
---
name: verify_bundle_supply_chain
description: Full supply-chain audit of a single bundle.
arguments:
  - { name: slug, required: true }
  - { name: version, required: true }
---

I want to fully audit the supply chain of EFE bundle `{slug}:{version}`.
Please:

1. Call `efe_inspect_bundle` with `slug={slug}` and `version={version}`.
2. Call `efe_verify_bundle_signature`. Verify it returns `verified:true`
   AND `slsa_level >= 2`.
3. Call `efe_get_sbom` with `filter_severity:HIGH`. List any HIGH or
   CRITICAL CVEs.
4. Read resource `efe://bundles/{slug}/{version}/signature`. Confirm
   the SLSA provenance attestation is signed by an expected builder
   (e.g. github-actions for the efe-project/efe-bundles repo).
5. Summarize : (a) verification status, (b) SLSA level, (c) any CVEs,
   (d) provenance match, (e) overall recommendation (DEPLOY-READY |
   REVIEW-NEEDED | BLOCKED).
```

#### 8.1.2 `audit_bundle_provenance` (D48)

```markdown
---
name: audit_bundle_provenance
description: SLSA + cosign provenance audit ceremony.
arguments:
  - { name: slug, required: false }
  - { name: version, required: false }
---

Time for the bundle provenance audit.

{#if slug}Audit specifically `{slug}{#if version}:{version}{/if}`.{/if}
{#if !slug}Audit all bundles registered in the backend.{/if}

For each bundle:
1. `efe_verify_bundle_signature` — sig must be `verified:true`.
2. Read `efe://bundles/{slug}/{version}/signature` — extract SLSA
   provenance. Confirm `level >= 2`.
3. Confirm `produced_by` matches the canonical builder URL pattern.
4. Confirm `signed_by_key_id` is in the trust store (whitelist of
   acceptable keys for this organization).

Output a markdown table : Bundle | Sig | SLSA | Provenance | Status.
Flag any row where Status != PASS.
```

#### 8.1.3 `weekly_bundle_health_report` (D48)

```markdown
---
name: weekly_bundle_health_report
description: Weekly periodic compliance check across all bundles.
---

Generate the weekly bundle health report.

1. List bundles via `efe_list_available_bundles`.
2. For each bundle, run `efe_inspect_bundle` and tabulate :
   - signed status, SLSA level, SBOM CVE count by severity.
3. Compare to last week's report (read `efe://audit/recent?since=...`
   to find the previous report's audit event).
4. Highlight any : (a) new bundle registered, (b) new CVE detected,
   (c) signing status change, (d) SLSA level downgrade.
5. Output a markdown report ready to post in the team channel.
```

#### 8.1.4 `deploy_new_bundle` (D49, agent-native-deploy)

```markdown
---
name: deploy_new_bundle
description: Guided deployment workflow (issues commands, doesn't execute).
arguments:
  - { name: slug, required: true }
  - { name: version, required: true }
  - { name: environment, required: false }
---

I want to deploy bundle `{slug}:{version}` to
{environment?production}. Help me through the deployment safely.

1. Pre-flight :
   a. `efe_check_bundle_compatibility slug={slug} version={version}` —
      MUST return `compatible:true`.
   b. `efe_verify_bundle_signature` — MUST return `verified:true`.
   c. `efe_dry_run_deploy slug={slug} version={version}` —
      review the diff + risks.
2. If pre-flight passes :
   a. `efe_generate_helm_values slug={slug} version={version}` —
      save the output to `values-mcp.yaml`.
   b. Output the exact commands the operator should run :
      ```
      helm upgrade efe ./charts/efe -f values-mcp.yaml --dry-run
      # review diff manually
      helm upgrade efe ./charts/efe -f values-mcp.yaml
      kubectl rollout status statefulset/efe -n {environment?efe-prod}
      ```
3. Wait for the operator to confirm execution.
4. Post-deploy verification :
   a. `efe_get_deployment_status` — confirm `bundle.version == {version}`.
   b. `efe_audit_health_check deep:true` — confirm chain intact.
   c. For each catalog with drift > 0.05 in dry_run output, suggest
      `efe_calibrate_thresholds`.

Important : I (the agent) will NEVER execute kubectl/helm. Only
generate the commands. The operator runs them with their own
credentials.
```

#### 8.1.5 `rollback_to_previous_bundle` (D49)

```markdown
---
name: rollback_to_previous_bundle
description: Emergency rollback walkthrough (commands only, never executes).
arguments:
  - { name: environment, required: false }
  - { name: reason, required: true }
---

Emergency rollback requested for {environment?production}.
Reason : `{reason}`.

1. `efe_get_deployment_history limit:5` — show me what was deployed.
2. `efe_generate_rollback_plan to_version:previous environment:{environment?production} reason:{reason}` —
   produce the runbook.
3. Output the runbook to the operator. Highlight the pre-checks
   in BOLD.
4. Wait for the operator to confirm execution of each step.
5. Post-rollback verification :
   a. `efe_get_deployment_status` — confirm version reverted.
   b. `efe_audit_health_check deep:true` — confirm chain intact.

I will NEVER run helm/kubectl. The operator does.
```

Templates complets en Annexe C.

---

## 9. Bundle verification pipeline

### 9.1 Pipeline complete

```
                ┌────────────────────────────────────────────┐
                │  efe_verify_bundle_signature               │
                └────────────────────────┬───────────────────┘
                                         │
       ┌─────────────────────────────────┼──────────────────────────┐
       ▼                                 ▼                          ▼
   1. Manifest pull                  2. Signature pull           3. Provenance pull
   GET /v2/<repo>/manifests/<tag>    GET /v2/<repo>/referrers   GET /v2/<repo>/referrers
                                       (filter sigType=…)         (filter sigType=slsa-…)
       │                                 │                          │
       └──────────────┬──────────────────┼──────────────────────────┘
                      ▼
       4. Local SHA-256 of manifest content
                      │
                      ▼
       5. Compare with manifest_sha256 in signature payload
                      │
                      ▼
       6. Ed25519 verify (cryptography.hazmat.primitives.asymmetric.ed25519)
              public key from EFE_OCI_PUBLIC_KEY env or backend trust store
                      │
                      ▼
       7. SLSA provenance check :
           - level (0-4) extracted
           - builder URL whitelisted
           - subject digest matches manifest_sha256
                      │
                      ▼
                 Final result
```

### 9.2 `mcp_server/bundle/verify.py` skeleton

```python
"""Pure-Python Ed25519 verification (no subprocess, no cosign CLI)."""
from __future__ import annotations

import hashlib
from dataclasses import dataclass
from typing import Optional

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import (
    Ed25519PublicKey,
)
from cryptography.hazmat.primitives.serialization import load_pem_public_key


@dataclass(frozen=True)
class VerificationResult:
    verified: bool
    signature_algorithm: str    # always 'ed25519' in M18
    signed_by_key_id: Optional[str]
    manifest_sha256: str
    manifest_sha256_matches: bool
    error: Optional[str]


def verify_ed25519_signature(
    *,
    manifest_bytes: bytes,
    signature_bytes: bytes,
    public_key_pem: bytes,
    expected_manifest_sha256: str | None = None,
) -> VerificationResult:
    """Verify an Ed25519 signature over a manifest.

    `manifest_bytes`  : raw OCI manifest content
    `signature_bytes` : 64-byte Ed25519 signature
    `public_key_pem`  : PEM-encoded ed25519 public key
    `expected_manifest_sha256` : optional cross-check
    """
    actual_sha = hashlib.sha256(manifest_bytes).hexdigest()
    matches = (
        expected_manifest_sha256 is None
        or expected_manifest_sha256 == actual_sha
    )
    if not matches:
        return VerificationResult(
            verified=False,
            signature_algorithm="ed25519",
            signed_by_key_id=None,
            manifest_sha256=actual_sha,
            manifest_sha256_matches=False,
            error=(
                f"manifest sha256 mismatch : expected "
                f"{expected_manifest_sha256!r}, got {actual_sha!r}"
            ),
        )

    try:
        pub = load_pem_public_key(public_key_pem)
        if not isinstance(pub, Ed25519PublicKey):
            raise ValueError("public key is not Ed25519")
        # Verify signature is over the manifest content (raw bytes)
        pub.verify(signature_bytes, manifest_bytes)
    except InvalidSignature:
        return VerificationResult(
            verified=False,
            signature_algorithm="ed25519",
            signed_by_key_id=None,
            manifest_sha256=actual_sha,
            manifest_sha256_matches=True,
            error="Ed25519 signature verification failed",
        )
    except Exception as exc:  # noqa: BLE001
        return VerificationResult(
            verified=False,
            signature_algorithm="ed25519",
            signed_by_key_id=None,
            manifest_sha256=actual_sha,
            manifest_sha256_matches=True,
            error=f"verification error: {exc}",
        )

    key_id = _compute_key_id(public_key_pem)
    return VerificationResult(
        verified=True,
        signature_algorithm="ed25519",
        signed_by_key_id=f"ed25519:{key_id}",
        manifest_sha256=actual_sha,
        manifest_sha256_matches=True,
        error=None,
    )


def _compute_key_id(public_key_pem: bytes) -> str:
    """SHA-256 first 16 hex chars of the PEM-encoded key."""
    return hashlib.sha256(public_key_pem).hexdigest()[:16]
```

---

## 10. SBOM CycloneDX 1.5 integration

### 10.1 Reference fields in CycloneDX 1.5

```json
{
  "bomFormat": "CycloneDX",
  "specVersion": "1.5",
  "version": 1,
  "components": [
    {
      "type": "library",
      "name": "torch",
      "version": "2.1.0",
      "purl": "pkg:pypi/torch@2.1.0"
    }
  ],
  "vulnerabilities": [
    {
      "id": "CVE-2024-XXXX",
      "ratings": [{"severity": "MEDIUM"}],
      "affects": [{"ref": "pkg:pypi/torch@2.1.0"}]
    }
  ]
}
```

### 10.2 `mcp_server/bundle/sbom.py` skeleton

```python
"""CycloneDX 1.5 SBOM parsing — pure JSON, no third-party dep."""
from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Iterable


@dataclass(frozen=True)
class Component:
    name: str
    version: str | None
    type: str       # library | application | container | data | …
    purl: str | None


@dataclass(frozen=True)
class Vulnerability:
    id: str
    severity: str            # LOW | MEDIUM | HIGH | CRITICAL | UNKNOWN
    affects: list[str]


@dataclass(frozen=True)
class ParsedSBOM:
    spec_version: str
    components: list[Component]
    vulnerabilities: list[Vulnerability]
    raw: dict


def parse_sbom(content: bytes) -> ParsedSBOM:
    data = json.loads(content)
    if data.get("bomFormat") != "CycloneDX":
        raise ValueError(f"unexpected bomFormat: {data.get('bomFormat')}")
    spec = data.get("specVersion", "")
    components = [
        Component(
            name=c.get("name", ""),
            version=c.get("version"),
            type=c.get("type", "library"),
            purl=c.get("purl"),
        )
        for c in data.get("components", [])
    ]
    vulns: list[Vulnerability] = []
    for v in data.get("vulnerabilities", []):
        sev = "UNKNOWN"
        ratings = v.get("ratings") or []
        if ratings:
            sev = (ratings[0].get("severity") or "UNKNOWN").upper()
        affects = [a.get("ref", "") for a in (v.get("affects") or [])]
        vulns.append(Vulnerability(id=v.get("id", ""), severity=sev, affects=affects))
    return ParsedSBOM(
        spec_version=spec,
        components=components,
        vulnerabilities=vulns,
        raw=data,
    )


def filter_by_severity(
    vulns: Iterable[Vulnerability], min_severity: str,
) -> list[Vulnerability]:
    levels = ("LOW", "MEDIUM", "HIGH", "CRITICAL")
    if min_severity.upper() not in levels:
        return list(vulns)
    threshold = levels.index(min_severity.upper())
    return [v for v in vulns if v.severity in levels[threshold:]]
```

---

## 11. Ed25519 signature verification (Python-only)

(Cf. §9.2 — déjà détaillé.)

### 11.1 Trust store sourcing

Public keys are loaded from one of, in priority order :
1. `EFE_OCI_PUBLIC_KEY` env (PEM content).
2. `EFE_OCI_PUBLIC_KEY_FILE` env (path to PEM file).
3. Backend `/admin/trust-store` endpoint (returns `{keys: [{id, pem}, ...]}`).
4. Built-in well-known keys (e.g. for `efe-project/efe-bundles` GitHub
   workflow signing key) — bundled with the mcp-server release.

If none available → `verification_unavailable: true` returned. The
caller can decide to deploy without verification (with warning).

### 11.2 Why no `sigstore-python` ?

`sigstore-python` is the canonical lib for Sigstore/cosign verification
but adds 30+ deps including `cryptography`, `pydantic`, `pyOpenSSL`,
`requests`, etc. M18 needs only `cryptography` for raw Ed25519
verification, which is ~5 MB and already common.

Sigstore-style "Bundle attestation" with Rekor inclusion proof is
out-of-scope for M18 livraison ; M19 may extend with Rekor verification
if customer demand.

---

## 12. OCI Artifacts integration

### 12.1 Read-only OCI client

`mcp_server/bundle/oci.py` — pure `httpx` HTTP client. No `oras`, no
`docker` subprocess.

```python
"""Read-only OCI Distribution Spec v1.1 client."""
from __future__ import annotations

import httpx
from typing import Optional


class OCIRegistryClient:
    """Anonymous + bearer-token-supporting HTTP client.

    Implements only GET endpoints :
      GET /v2/<repo>/manifests/<reference>
      GET /v2/<repo>/blobs/<digest>
      GET /v2/<repo>/referrers/<digest>
      GET /v2/<repo>/tags/list
    """

    def __init__(
        self,
        registry_url: str,
        *,
        token: Optional[str] = None,
        timeout_seconds: float = 30.0,
    ) -> None:
        self._url = registry_url.rstrip("/")
        self._client = httpx.AsyncClient(timeout=timeout_seconds)
        self._headers = {
            "User-Agent": "efe-mcp-server/0.3.0",
            "Accept": "application/vnd.oci.image.manifest.v1+json,"
                     "application/vnd.oci.artifact.manifest.v1+json",
        }
        if token:
            self._headers["Authorization"] = f"Bearer {token}"

    async def get_manifest(
        self, repo: str, reference: str,
    ) -> tuple[bytes, dict]:
        # Returns (raw_bytes, parsed_json)
        ...

    async def get_blob(self, repo: str, digest: str) -> bytes:
        ...

    async def get_referrers(
        self, repo: str, digest: str, *, artifact_type: Optional[str] = None,
    ) -> list[dict]:
        ...

    async def list_tags(self, repo: str) -> list[str]:
        ...

    async def aclose(self) -> None:
        await self._client.aclose()
```

### 12.2 Registry allowlist (security)

`EFE_OCI_REGISTRY_ALLOWLIST` env var is **strictly enforced**. Any
attempt to access a registry outside the list → `BackendForbidden`.

Default allowlist (if unset) : empty → ALL registry access denied.
Operator must explicitly opt-in.

```python
def _check_allowlist(registry_url: str) -> None:
    allowed = (
        os.environ.get("EFE_OCI_REGISTRY_ALLOWLIST", "")
        .strip().split(",")
    )
    if not any(registry_url.startswith(a) for a in allowed if a):
        raise BackendForbidden(
            f"OCI registry {registry_url!r} not in EFE_OCI_REGISTRY_ALLOWLIST",
            backend_code="oci_registry_disallowed",
            status_code=403,
        )
```

---

## 13. Deployment dry-run mechanics

### 13.1 Algorithme

```
                       ┌──────────────────────┐
                       │  efe_dry_run_deploy  │
                       │  slug, version       │
                       └──────────┬───────────┘
                                  │
                                  ▼
       1. Get current deployment via backend /admin/deployments/current
                                  │
                                  ▼
       2. Inspect target bundle (efe_inspect_bundle internally)
                                  │
                                  ▼
       3. Compute manifest diff :
          - model_name change → high impact
          - dim change → BLOCKER
          - framework change → high impact
                                  │
                                  ▼
       4. Compute catalog compat :
          - For each catalog the tenant has :
              - centroid_drift_estimate = simulate via embedder hash
              - threshold breakage prediction
                                  │
                                  ▼
       5. Compute risks :
          - LOW : minor SBOM diff
          - MEDIUM : centroid drift > 5%
          - HIGH : dim change, breaking schema
                                  │
                                  ▼
       6. Aggregate + recommend
```

### 13.2 Backend support

EFE 1.2 endpoint `POST /admin/deployments/dry-run` :

```http
POST /admin/deployments/dry-run
Content-Type: application/json

{
  "target_bundle": {"slug": "...", "version": "..."}
}
```

Returns full diff. Falls back to local computation by mcp-server if
endpoint not available (less precise but functional).

---

## 14. Helm values.yaml generation

### 14.1 Template

`mcp_server/deploy/helm_values.py` :

```python
"""Generate Helm values.yaml from a small Python data model.

Notes :
  - Output is plain text YAML, never applied.
  - Only sections from a whitelist : bundle, replicaCount, resources,
    podDisruptionBudget, autoscaling. Anything else from user args
    is silently dropped (TM-M18-E-02 mitigation).
"""
from __future__ import annotations

from dataclasses import dataclass, field

import yaml  # safe_dump only


_ALLOWED_KEYS = frozenset({
    "bundle", "replicaCount", "resources",
    "podDisruptionBudget", "autoscaling",
})


@dataclass
class HelmValuesInput:
    bundle_slug: str
    bundle_version: str
    bundle_manifest_sha256: str
    replica_count: int = 3
    cpu_request: str = "500m"
    cpu_limit: str = "2000m"
    memory_request: str = "1Gi"
    memory_limit: str = "4Gi"
    pdb_min_available: int = 2
    pdb_enabled: bool = True
    autoscaling_enabled: bool = False
    autoscaling_min: int = 1
    autoscaling_max: int = 10


def render(input_: HelmValuesInput) -> str:
    body = {
        "bundle": {
            "slug": input_.bundle_slug,
            "version": input_.bundle_version,
            "manifest_sha256": input_.bundle_manifest_sha256,
        },
        "replicaCount": input_.replica_count,
        "resources": {
            "requests": {
                "cpu": input_.cpu_request,
                "memory": input_.memory_request,
            },
            "limits": {
                "cpu": input_.cpu_limit,
                "memory": input_.memory_limit,
            },
        },
        "podDisruptionBudget": {
            "enabled": input_.pdb_enabled,
            "minAvailable": input_.pdb_min_available,
        },
        "autoscaling": {
            "enabled": input_.autoscaling_enabled,
            "min": input_.autoscaling_min,
            "max": input_.autoscaling_max,
        },
    }
    # Defensive : drop unknown keys (would never happen with the
    # dataclass, but explicit assertion for TM-M18-E-02).
    body = {k: v for k, v in body.items() if k in _ALLOWED_KEYS}
    header = (
        "# Generated by efe-mcp-server efe_generate_helm_values\n"
        f"# bundle: {input_.bundle_slug}:{input_.bundle_version}\n"
        f"# manifest_sha256: {input_.bundle_manifest_sha256}\n"
    )
    return header + yaml.safe_dump(body, default_flow_style=False, sort_keys=True)
```

YAML output via `yaml.safe_dump` only (no execution risk).
`sort_keys=True` for deterministic output (audit-friendly).

---

## 15. JSON Schemas (forward-refs)

Tous les schémas complets en **Annexe A**. Conventions M18 :

- `additionalProperties: false` sur tous les objets (TM-M16-T-02 carry-over).
- `slug` et `version` patterns stricts :
  - `slug` : `^[a-z][a-z0-9\-]{0,99}$`
  - `version` : `^v\d+\.\d+\.\d+(-[a-z0-9\-]{1,15})?$`
- Tout outil write requiert `confirm: true` (M16 carry-over).
- `registry_url` validé contre allowlist (TM-M18-D-01 + TM-M18-S-01).

---

## 16. Backend extensions

### 16.1 EFE 1.2 contract bump (semver-mineur additif)

3 nouveaux endpoints REST + 1 RPC gRPC.

### 16.2 Nouveaux endpoints REST

| Method | Path | Phase | Description |
|---|---|---|---|
| `GET` | `/admin/bundles` | D47 | Liste bundles enregistrés au backend |
| `GET` | `/admin/bundles/<slug>/<version>` | D47 | Métadonnées bundle |
| `POST` | `/admin/bundles/register` | D50 | Enregistrer un nouveau bundle |
| `POST` | `/admin/bundles/<slug>/<version>/promote` | D50 | Promouvoir comme « current » |
| `GET` | `/admin/deployments/current` | D49 | État du déploiement |
| `GET` | `/admin/deployments/history` | D49 | Historique |
| `POST` | `/admin/deployments/dry-run` | D49 | Calcul diff sans apply |
| `POST` | `/admin/tickets` | D50 | Créer un ticket (Linear/Jira/GitHub) |
| `GET` | `/admin/trust-store` | D47 | Liste clés publiques pour vérification |

### 16.3 Nouveau RPC gRPC

```proto
service EfeAdminService {
  rpc DryRunDeployment(DryRunDeploymentRequest) returns (DryRunDeploymentResponse);
}
```

### 16.4 Backward compat sur EFE 1.1

Le mcp-server détecte la version backend via `GET /info`
`contract_version`. Si 1.1 :
- `efe_list_available_bundles` → fallback : OCI registry direct via
  `mcp_server/bundle/oci.py` (avec allowlist).
- `efe_get_deployment_status` → fallback : `GET /admin/bundle-info`
  (M15-A5) + `kubectl`-equivalent metadata depuis `/info`.
- `efe_get_deployment_history` → **error** `not_supported` (no
  fallback : history requires backend storage).
- `efe_dry_run_deploy` → fallback : compute local diff.
- `efe_register_bundle`, `efe_promote_bundle`, `efe_request_deployment`
  → **error** `backend_too_old` ; require EFE 1.2.

---

## 17. Threat model M18

Delta sur les 20 menaces M16+M17 = **8 nouvelles M18**, 28 cumulées.

| ID | STRIDE | Threat | Mitigation livrée M18 |
|---|:---:|---|---|
| **TM-M18-S-01** | Spoof | Bundle signature forged ou pas vérifiée | Ed25519 verification obligatoire dans `verify.py` ; sans clé publique → `verification_unavailable:true` (jamais `verified:true` par erreur) |
| **TM-M18-T-01** | Tamper | Bundle manifest bytes altérés post-signing | SHA-256 recompute + comparaison avec ce qui est signé ; mismatch → `manifest_sha256_matches:false` |
| **TM-M18-R-01** | Repud. | Deploy actions non auditées | `efe_register_bundle`, `efe_promote_bundle`, `efe_request_deployment` émettent tous un audit event `mcp_call.<tool>` dans la chaîne D14-bis |
| **TM-M18-I-01** | InfoDisc | SBOM contient des dépendances internes proprio | SBOM ne sort jamais du backend du tenant (D11) ; `efe_get_sbom` scoped au tenant |
| **TM-M18-D-01** | DoS | OCI registry flood via concurrent inspect | Rate-limit weight 5× sur `efe_inspect_bundle` ; allowlist registry strict ; bundle_cache LRU |
| **TM-M18-E-01** | Elev. | Bundle promotion bypass peer review | `efe_promote_bundle` requiert profile `agent-native-deploy` (opt-in) + `confirm:true` + audit event ; **n'est PAS un déploiement** |
| **TM-M18-E-02** | Elev. | Helm values.yaml injection (eval) | `_ALLOWED_KEYS` whitelist + `yaml.safe_dump` only + sortie texte (operator review obligatoire avant apply) |
| **TM-M18-E-03** | Elev. | SLSA attestation forgery | Vérification du `subject_digest` contre `manifest_sha256` calculé localement ; builder URL whitelist |

**Threat carry-over** : les 12 TM-M16-* + 8 TM-M17-* restent valides
et sont systématiquement re-vérifiés par les pin tests M18.

**Pen-test couverture** : threat model M18 ajouté au scope du
pen-test annuel 2026 (référence `docs/security/threat_model.md`).

---

## 18. Performance & SLOs M18

### 18.1 SLOs M18-MCP

| ID | SLO | Cible | Mesuré comment |
|---|---|---|---|
| M18-MCP-1 | p50 `efe_verify_bundle_signature` | < 100 ms | OTel histogram |
| M18-MCP-2 | p99 `efe_verify_bundle_signature` | < 1.5 s | OTel idem |
| M18-MCP-3 | p50 `efe_inspect_bundle` (cached) | < 50 ms | OTel idem |
| M18-MCP-4 | p99 `efe_inspect_bundle` (cold) | < 3 s | OTel idem |
| M18-MCP-5 | p50 `efe_dry_run_deploy` | < 500 ms | OTel idem |
| M18-MCP-6 | p99 `efe_dry_run_deploy` | < 5 s | OTel idem |
| M18-MCP-7 | bundle_cache hit rate | ≥ 80 % within session | OTel ratio |
| M18-MCP-8 | OCI registry call rate | ≤ 10 / minute / tenant | rate-limiter |
| M18-MCP-9 | Helm values.yaml size | ≤ 4 KB typical | bench M18 |
| M18-MCP-10 | `verify` false-positive rate | **0 %** (mathematical) | crypto bench |
| M18-MCP-11 | profile=`agent-native` regression | identique à M17 | CI |
| M18-MCP-12 | M18 pin tests | 75 nouveaux GREEN | CI |

### 18.2 Runbooks M18 (dans `docs/runbooks/`)

- `m18_oci_registry_unreachable.md` — registry timeout / DNS / TLS.
- `m18_signature_verification_failed.md` — `verified:false` paths.
- `m18_sbom_parse_error.md` — malformed CycloneDX.
- `m18_dry_run_drift_high.md` — when to abort deploy.
- `m18_promote_bundle_audit_gap.md` — audit event missing post-promote.

---

## 19. Compatibility matrix

### 19.1 OCI registries supportés

| Registry | Auth | Status M18 |
|---|---|:---:|
| GitHub Container Registry (ghcr.io) | Anonymous + GitHub PAT | ✅ |
| Docker Hub (docker.io) | Anonymous + bearer | ✅ |
| AWS ECR | AWS Sig v4 (M19) | ⚠ token only |
| GCP Artifact Registry | Bearer | ✅ |
| Azure Container Registry | Bearer | ✅ |
| Harbor (self-hosted) | Bearer | ✅ |
| Nexus / JFrog Artifactory | Bearer | ⚠ tested only manually |
| Quay.io | Anonymous + bearer | ✅ |

### 19.2 SBOM formats

| Format | Status M18 |
|---|:---:|
| CycloneDX 1.4 | ✅ (read-compat) |
| CycloneDX 1.5 | ✅ (primary) |
| CycloneDX 1.6 | ⚠ M19 |
| SPDX 2.3 | ❌ M19+ |

### 19.3 SLSA levels

| Level | M18 support |
|---|:---:|
| Level 0 (no provenance) | ✅ (warning) |
| Level 1 (basic provenance) | ✅ |
| Level 2 (signed provenance) | ✅ |
| Level 3 (hardened builder) | ✅ |
| Level 4 (two-party review) | ⚠ partial |

---

## 20. Migration M17 → M18

### 20.1 Profile gating

```bash
EFE_MCP_TOOL_PROFILE=core              # M16 strict
EFE_MCP_TOOL_PROFILE=advanced          # +M17 reasoning/discovery/diag
EFE_MCP_TOOL_PROFILE=agent-native      # +M17 sampling/streaming
                                        # +M18 read-only bundle/deploy tools (NEW in M18)
EFE_MCP_TOOL_PROFILE=agent-native-deploy  # +M18 write bundle/deploy tools (NEW)
EFE_MCP_TOOL_PROFILE=experimental      # M19+
```

Implementation :

```python
# mcp_server/profile.py — extension
class Profile(str, Enum):
    CORE = "core"
    ADVANCED = "advanced"
    AGENT_NATIVE = "agent-native"
    AGENT_NATIVE_DEPLOY = "agent-native-deploy"  # NEW
    EXPERIMENTAL = "experimental"

# Read-only M18 tools added to AGENT_NATIVE
# Write M18 tools added to AGENT_NATIVE_DEPLOY only
```

### 20.2 Backward-compat tests

Pin tests rouge si profile=`core` ou `advanced` ou `agent-native` voit
n'importe quel changement de comportement vs M17. Anti-régression
identique au pattern M17.

### 20.3 Rollback plan

Si M18 introduit régression en prod :

```bash
EFE_MCP_TOOL_PROFILE=agent-native efe-mcp-server
```

Ou re-deploy l'image M17 0.2.x — toujours backward compatible.

---

## 21. Test plan

### 21.1 Pin tests inventory (~75 nouveaux)

Inventaire complet en Annexe D. Résumé :

| Fichier | Tests | Phase |
|---|---|---|
| `test_profile_gating.py` (extension) | +3 | D50 |
| `test_bundle_verify.py` | 12 | D47 |
| `test_bundle_introspection.py` | 10 | D47 |
| `test_bundle_diff.py` | 6 | D47 |
| `test_bundle_compat.py` | 5 | D47 |
| `test_sbom_parsing.py` | 8 | D48 |
| `test_slsa_provenance.py` | 6 | D48 |
| `test_oci_client.py` | 7 | D47 |
| `test_deployment_dry_run.py` | 8 | D49 |
| `test_helm_values_generation.py` | 6 | D49 |
| `test_bundle_resources.py` | 7 | D48 |
| **Total nouveaux M18** | **78** | |

Aggregate après M18 : **160 (M16+M17 cumulés) + 78 = 238 mcp-server tests**.

### 21.2 Crypto vector tests

Les tests de `verify.py` utilisent des vecteurs Ed25519 RFC 8032 +
des bundles signés in-test (génération de paires éphémères). Cela
garantit **zéro false-positive** de vérification.

### 21.3 OCI registry mock

Tests utilisent `respx` (httpx mock) pour simuler les endpoints OCI
v2. Pas de network access en CI.

### 21.4 Audit pass M18

Pattern récurrent M11-M17 (~6-8 défauts attendus). Phase D50.

---

## 22. Deployment scenarios

### 22.1 Local dev (Claude Desktop), profile = agent-native

```json
{
  "mcpServers": {
    "efe": {
      "command": "efe-mcp-server",
      "env": {
        "EFE_OIDC_TOKEN": "...",
        "EFE_ENDPOINT": "https://efe.example.com",
        "EFE_MCP_TOOL_PROFILE": "agent-native",
        "EFE_OCI_REGISTRY_ALLOWLIST": "ghcr.io/efe-project,registry.example.com",
        "EFE_OCI_PUBLIC_KEY_FILE": "/etc/efe/trust/efe-bundles.pub"
      }
    }
  }
}
```

### 22.2 Ops-deploy mode

```json
{
  "mcpServers": {
    "efe-deploy": {
      "command": "efe-mcp-server",
      "env": {
        "EFE_OIDC_TOKEN": "<deploy-tier-jwt>",
        "EFE_ENDPOINT": "https://efe.example.com",
        "EFE_MCP_TOOL_PROFILE": "agent-native-deploy",
        "EFE_OCI_REGISTRY_ALLOWLIST": "ghcr.io/efe-project",
        "EFE_OCI_PUBLIC_KEY_FILE": "/etc/efe/trust/efe-bundles.pub",
        "EFE_DEPLOY_TICKET_TRACKER": "linear",
        "EFE_DEPLOY_TICKET_PROJECT": "EFE"
      }
    }
  }
}
```

### 22.3 Air-gapped

OCI registry interne + local trust store file. Pas de différence
fonctionnelle ; juste config différente.

---

## 23. Roadmap M19+

### 23.1 M19 — « Cloud transport + multi-replica » (Q4 2026)

- HTTP+SSE transport implémenté.
- Stateless mcp-server multi-replica.
- Rekor inclusion proof verification (sigstore-python optionnel).

### 23.2 M20 — « Multi-modal » (2027 H1)

- Bundle multimodal (image/audio embedder).
- Tools EFE multi-modal classification.

### 23.3 M21 — « GitOps integration » (2027 conditional)

- Read-only ArgoCD/FluxCD integration (status only).
- Toujours pas de mutation cluster depuis MCP.

---

## 24. Open questions

1. **OSV.dev integration** — should `efe_get_sbom` query OSV.dev for
   live CVE data ? Adds latency + new sub-processor. **Décision** :
   reporté M19 ; M18 retourne CVE info from SBOM only (static).
2. **Cosign-bundle format** — should we support the new sigstore
   "bundle" format (v0.2 spec) ? Requires `sigstore-python`. **Décision** :
   reporté M19.
3. **Multi-cluster deployment** — should `efe_dry_run_deploy` support
   multi-cluster scenarios ? **Décision** : non, M18 single-cluster.
4. **Helm chart versioning** — should `efe_generate_helm_values` pin a
   specific chart version ? **Décision** : oui, derived from
   `EFE_HELM_CHART_VERSION` env, default = current.
5. **GitOps source-of-truth** — should `efe_promote_bundle` modify a
   git repo (PR creation) instead of backend metadata ? **Décision** :
   non, that's M19+ territory.

---

## 25. FAQ

**Q: Pourquoi le MCP server ne déploie-t-il pas directement ?**
A: Boundary de sécurité. Cf. §2.1.

**Q: Comment l'agent sait-il que l'opérateur a appliqué les commandes ?**
A: L'opérateur applique manuellement, puis dans le suivi de
conversation appelle `efe_get_deployment_status` qui montre le nouvel
état. L'agent compare avec ce qu'il a recommandé.

**Q: Que se passe-t-il si le bundle Ed25519 verification échoue ?**
A: Le tool retourne `verified:false` avec une raison ; l'agent
informe l'utilisateur ; les autres tools (e.g. `dry_run_deploy`)
peuvent encore tourner mais retournent un blocking issue
"signature_unverified".

**Q: Cosign signing key rotation ?**
A: Ajouter la nouvelle clé au trust store ; l'ancien continue de
vérifier les anciens bundles. M18 supporte multi-key trust store
(prochaine itération via `EFE_OCI_PUBLIC_KEY_DIR=/etc/efe/trust/`).

**Q: Bundle download size ?**
A: Le MCP server ne télécharge **que le manifest + signature
artifacts** (~10-50 KB). Le bundle blob réel (modèle ML, ~500 MB)
n'est jamais pulled par le MCP server — c'est le backend ou le pod
EFE qui le pull lors du démarrage.

**Q: Compliance posture impact ?**
A: Aucun. SOC 2 CC9.2 (vendor management) couvert dans note additive.
GDPR Art. 32 §3.6 : « bundle signature verification helpers run in
pure Python in the user's MCP client process ; no new sub-processor
is introduced ».

---

## 26. Glossary M18

| Term | Définition |
|---|---|
| **OCI Artifact** | Image-like artifact stored in an OCI registry, used by EFE for bundles. |
| **CycloneDX** | SBOM JSON format standard ; v1.5 supported in M18. |
| **SBOM** | Software Bill of Materials. |
| **SLSA** | Supply-chain Levels for Software Artifacts (Google standard). |
| **cosign** | Sigstore CLI ; M18 reads the artifacts cosign produces but never invokes the CLI. |
| **Sigstore Bundle** | New (v0.2 spec) bundling format ; M18 doesn't support, M19 may. |
| **Provenance attestation** | SLSA-style attestation of build inputs/outputs. |
| **Trust store** | Local set of public keys accepted for signature verification. |
| **Allowlist registry** | Strict whitelist of OCI registries permitted to query (TM-M18-D-01). |
| **Bundle promote** | Mark a registered bundle as "current" — metadata only, NOT a deploy. |
| **Dry-run deploy** | Compute what a deploy would change, without applying. |
| **Helm values.yaml** | Helm input file ; M18 generates as text, never applies. |
| **EFE 1.2 contract** | Semver-mineur additive bump ; 3 new REST endpoints + 1 RPC. |

---

## 27. References

- **MCP spec** : <https://modelcontextprotocol.io/specification>
- **EFE M16/M17 livraison** : `mcp-server/`, `docs/EFE_M{16,17}_*.md`
- **EFE M13 bundles** : `docs/EFE_BUNDLES_EXTENSION_ARCHITECTURE.md`
- **EFE M12 Helm** : `docs/EFE_HELM_CHART_EXTENSION_ARCHITECTURE.md`
- **EFE D14-bis HMAC** : `docs/bench/d14bis_hmac_audit_chain_architecture.md`
- **EFE D16 bundle ID** : `docs/bench/d16_bundle_supply_chain_architecture.md`
- **OCI Distribution Spec v1.1** :
  <https://github.com/opencontainers/distribution-spec/blob/v1.1.0/spec.md>
- **CycloneDX 1.5 spec** :
  <https://cyclonedx.org/docs/1.5/json/>
- **SLSA spec v1.0** : <https://slsa.dev/spec/v1.0/>
- **Sigstore cosign** : <https://docs.sigstore.dev/cosign/overview/>
- **Cyber Resilience Act EU 2024/2847** :
  <https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX:32024R2847>
- **NIS2 Article 21** :
  <https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX:32022L2555>
- **cryptography Python lib** : <https://cryptography.io/>

---

## A — Full JSON Schemas

### A.1 `efe_list_available_bundles`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeListAvailableBundlesInput",
  "description": "Enumerate bundles in the configured OCI registry.",
  "properties": {
    "registry":    {"type": "string", "minLength": 1, "maxLength": 200},
    "filter_slug": {"type": "string", "maxLength": 100},
    "limit":       {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
  },
  "additionalProperties": false
}
```

### A.2 `efe_inspect_bundle`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeInspectBundleInput",
  "properties": {
    "slug":    {"type": "string", "pattern": "^[a-z][a-z0-9\\-]{0,99}$"},
    "version": {"type": "string", "pattern": "^v\\d+\\.\\d+\\.\\d+(-[a-z0-9\\-]{1,15})?$"},
    "include_full_sbom": {"type": "boolean", "default": false}
  },
  "required": ["slug", "version"],
  "additionalProperties": false
}
```

### A.3 `efe_verify_bundle_signature`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeVerifyBundleSignatureInput",
  "properties": {
    "slug":             {"type": "string", "pattern": "^[a-z][a-z0-9\\-]{0,99}$"},
    "version":          {"type": "string", "pattern": "^v\\d+\\.\\d+\\.\\d+(-[a-z0-9\\-]{1,15})?$"},
    "expected_key_id":  {"type": "string", "maxLength": 100}
  },
  "required": ["slug", "version"],
  "additionalProperties": false
}
```

### A.4 `efe_diff_bundles`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeDiffBundlesInput",
  "properties": {
    "slug_a":    {"type": "string", "pattern": "^[a-z][a-z0-9\\-]{0,99}$"},
    "version_a": {"type": "string", "pattern": "^v\\d+\\.\\d+\\.\\d+(-[a-z0-9\\-]{1,15})?$"},
    "slug_b":    {"type": "string", "pattern": "^[a-z][a-z0-9\\-]{0,99}$"},
    "version_b": {"type": "string", "pattern": "^v\\d+\\.\\d+\\.\\d+(-[a-z0-9\\-]{1,15})?$"}
  },
  "required": ["slug_a", "version_a", "slug_b", "version_b"],
  "additionalProperties": false
}
```

### A.5 `efe_check_bundle_compatibility`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeCheckBundleCompatibilityInput",
  "properties": {
    "slug":               {"type": "string", "pattern": "^[a-z][a-z0-9\\-]{0,99}$"},
    "version":            {"type": "string", "pattern": "^v\\d+\\.\\d+\\.\\d+(-[a-z0-9\\-]{1,15})?$"},
    "target_environment": {"type": "string", "maxLength": 50, "default": "production"}
  },
  "required": ["slug", "version"],
  "additionalProperties": false
}
```

### A.6 `efe_get_sbom`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeGetSbomInput",
  "properties": {
    "slug":            {"type": "string", "pattern": "^[a-z][a-z0-9\\-]{0,99}$"},
    "version":         {"type": "string", "pattern": "^v\\d+\\.\\d+\\.\\d+(-[a-z0-9\\-]{1,15})?$"},
    "filter_severity": {"type": "string", "enum": ["LOW", "MEDIUM", "HIGH", "CRITICAL"]},
    "max_components":  {"type": "integer", "minimum": 1, "maximum": 5000, "default": 200}
  },
  "required": ["slug", "version"],
  "additionalProperties": false
}
```

### A.7 `efe_get_deployment_status`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeGetDeploymentStatusInput",
  "properties": {},
  "additionalProperties": false
}
```

### A.8 `efe_get_deployment_history`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeGetDeploymentHistoryInput",
  "properties": {
    "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 10}
  },
  "additionalProperties": false
}
```

### A.9 `efe_dry_run_deploy`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeDryRunDeployInput",
  "properties": {
    "slug":        {"type": "string", "pattern": "^[a-z][a-z0-9\\-]{0,99}$"},
    "version":     {"type": "string", "pattern": "^v\\d+\\.\\d+\\.\\d+(-[a-z0-9\\-]{1,15})?$"},
    "environment": {"type": "string", "maxLength": 50, "default": "production"}
  },
  "required": ["slug", "version"],
  "additionalProperties": false
}
```

### A.10 `efe_generate_helm_values`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeGenerateHelmValuesInput",
  "properties": {
    "slug":                     {"type": "string", "pattern": "^[a-z][a-z0-9\\-]{0,99}$"},
    "version":                  {"type": "string", "pattern": "^v\\d+\\.\\d+\\.\\d+(-[a-z0-9\\-]{1,15})?$"},
    "replicas":                 {"type": "integer", "minimum": 1, "maximum": 100, "default": 3},
    "include_resources_section":{"type": "boolean", "default": true},
    "include_pdb":              {"type": "boolean", "default": true}
  },
  "required": ["slug", "version"],
  "additionalProperties": false
}
```

### A.11 `efe_generate_rollback_plan`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeGenerateRollbackPlanInput",
  "properties": {
    "to_version":  {"type": "string", "maxLength": 50, "default": "previous"},
    "environment": {"type": "string", "maxLength": 50, "default": "production"},
    "reason":      {"type": "string", "minLength": 1, "maxLength": 500}
  },
  "required": ["reason"],
  "additionalProperties": false
}
```

### A.12 `efe_register_bundle` (write)

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeRegisterBundleInput",
  "properties": {
    "slug":                   {"type": "string", "pattern": "^[a-z][a-z0-9\\-]{0,99}$"},
    "version":                {"type": "string", "pattern": "^v\\d+\\.\\d+\\.\\d+(-[a-z0-9\\-]{1,15})?$"},
    "registry_url":           {"type": "string", "minLength": 1, "maxLength": 200},
    "verify_before_register": {"type": "boolean", "default": true},
    "confirm":                {"type": "boolean"}
  },
  "required": ["slug", "version", "registry_url", "confirm"],
  "additionalProperties": false
}
```

### A.13 `efe_promote_bundle` (write)

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfePromoteBundleInput",
  "properties": {
    "slug":    {"type": "string", "pattern": "^[a-z][a-z0-9\\-]{0,99}$"},
    "version": {"type": "string", "pattern": "^v\\d+\\.\\d+\\.\\d+(-[a-z0-9\\-]{1,15})?$"},
    "scope":   {"type": "string", "enum": ["tenant", "global"], "default": "tenant"},
    "confirm": {"type": "boolean"}
  },
  "required": ["slug", "version", "confirm"],
  "additionalProperties": false
}
```

### A.14 `efe_request_deployment` (write)

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeRequestDeploymentInput",
  "properties": {
    "slug":        {"type": "string", "pattern": "^[a-z][a-z0-9\\-]{0,99}$"},
    "version":     {"type": "string", "pattern": "^v\\d+\\.\\d+\\.\\d+(-[a-z0-9\\-]{1,15})?$"},
    "environment": {"type": "string", "maxLength": 50},
    "rationale":   {"type": "string", "minLength": 10, "maxLength": 2000},
    "tracker":     {"type": "string", "enum": ["linear", "jira", "github", "internal"]},
    "confirm":     {"type": "boolean"}
  },
  "required": ["slug", "version", "rationale", "tracker", "confirm"],
  "additionalProperties": false
}
```

---

## B — Bundle URI grammar

Formal ABNF (RFC 5234) extension de M17 §B :

```abnf
efe-uri      = "efe://" path [ "?" query ]
path         = (m17-path) / bundles-path / deployments-path

bundles-path = "bundles" [ slash slug slash version
                         [ slash sub-resource ]
                         ]
sub-resource = "sbom" / "signature"

deployments-path = "deployments" slash
                  ( "current" / "history" )

slug         = lc-letter *99( lc-letter / DIGIT / "-" )
version      = "v" 1*3DIGIT "." 1*3DIGIT "." 1*3DIGIT [ "-" tag ]
tag          = 1*15( lc-letter / DIGIT / "-" )
lc-letter    = %x61-7A
slash        = "/"
```

Tout URI hors-grammaire → `400 invalid_resource_uri` (TM-M17-E-01 +
TM-M18-T-01).

---

## C — Prompt templates (résumé)

5 templates complets en §8.1.1 — §8.1.5.

---

## D — Pin tests inventory

### D.1 `test_bundle_verify.py` (12 tests, D47)

- `test_ed25519_valid_signature_verifies_true`
- `test_ed25519_invalid_signature_verifies_false`
- `test_ed25519_manifest_sha_mismatch_detected`
- `test_ed25519_corrupted_signature_bytes_rejected`
- `test_ed25519_wrong_key_rejected`
- `test_ed25519_key_id_computation_stable`
- `test_verify_with_no_public_key_returns_unavailable`
- `test_verify_unsigned_bundle_returns_unsigned`
- `test_verify_with_expected_key_id_match`
- `test_verify_with_expected_key_id_mismatch`
- `test_verify_uses_cryptography_lib_no_subprocess`
- `test_verify_compute_key_id_first_16_hex_chars`

### D.2 `test_bundle_introspection.py` (10 tests)

- `test_inspect_bundle_returns_full_metadata`
- `test_inspect_bundle_cache_hit_avoids_oci_call`
- `test_inspect_bundle_unknown_slug_returns_not_found`
- `test_inspect_bundle_with_full_sbom_includes_components`
- `test_inspect_bundle_with_full_sbom_false_only_summary`
- `test_inspect_bundle_handles_unsigned_gracefully`
- `test_inspect_bundle_emits_audit_event`
- `test_inspect_bundle_respects_oci_allowlist`
- `test_inspect_bundle_invalid_version_pattern_rejected`
- `test_inspect_bundle_invalid_slug_pattern_rejected`

### D.3 `test_bundle_diff.py` (6 tests)

- `test_diff_returns_components_added_removed_upgraded`
- `test_diff_detects_resolved_vulnerabilities`
- `test_diff_detects_breaking_change`
- `test_diff_zero_differences_when_same_bundle`
- `test_diff_size_diff_percent_computed`
- `test_diff_recommendation_safe_when_minor`

### D.4 `test_bundle_compat.py` (5 tests)

- `test_compat_compatible_when_all_checks_pass`
- `test_compat_incompatible_when_dim_mismatch`
- `test_compat_warning_when_features_subset`
- `test_compat_blocking_when_min_version_too_low`
- `test_compat_with_target_environment_routing`

### D.5 `test_sbom_parsing.py` (8 tests)

- `test_parse_cyclonedx_1_5_basic`
- `test_parse_cyclonedx_1_4_compat`
- `test_parse_filters_severity_high`
- `test_parse_invalid_format_raises`
- `test_parse_no_vulnerabilities_section`
- `test_parse_components_extracts_purl`
- `test_parse_severity_unknown_default`
- `test_parse_handles_empty_components_array`

### D.6 `test_slsa_provenance.py` (6 tests)

- `test_provenance_level_3_extracted`
- `test_provenance_builder_url_validated`
- `test_provenance_subject_digest_matches_manifest`
- `test_provenance_no_attestation_returns_level_0`
- `test_provenance_invalid_attestation_format_raises`
- `test_provenance_builder_not_in_whitelist_warns`

### D.7 `test_oci_client.py` (7 tests)

- `test_oci_get_manifest_returns_bytes_and_parsed`
- `test_oci_anonymous_access_supported`
- `test_oci_bearer_auth_header_set`
- `test_oci_referrers_filtered_by_artifact_type`
- `test_oci_registry_not_in_allowlist_rejected`
- `test_oci_timeout_configured`
- `test_oci_user_agent_set`

### D.8 `test_deployment_dry_run.py` (8 tests)

- `test_dry_run_returns_current_and_target`
- `test_dry_run_detects_dim_change_blocker`
- `test_dry_run_estimates_centroid_drift`
- `test_dry_run_no_diff_when_same_bundle`
- `test_dry_run_recommended_actions_present`
- `test_dry_run_falls_back_to_local_when_backend_old`
- `test_dry_run_blocking_issues_when_signature_unverified`
- `test_dry_run_blast_radius_single_namespace`

### D.9 `test_helm_values_generation.py` (6 tests)

- `test_helm_values_safe_yaml_only`
- `test_helm_values_unknown_keys_dropped`
- `test_helm_values_includes_manifest_sha256_comment`
- `test_helm_values_replica_count_default_3`
- `test_helm_values_pdb_default_enabled`
- `test_helm_values_deterministic_output`

### D.10 `test_bundle_resources.py` (7 tests)

- `test_efe_bundles_list_resource`
- `test_efe_bundles_slug_version_resource`
- `test_efe_bundles_sbom_resource`
- `test_efe_bundles_signature_resource`
- `test_efe_deployments_current_resource`
- `test_resource_traversal_rejected`
- `test_resource_emits_mcp_resource_read_audit`

### D.11 `test_profile_gating.py` (extension, 3 nouveaux)

- `test_agent_native_deploy_includes_3_write_tools`
- `test_agent_native_does_not_include_deploy_writes`
- `test_unknown_deploy_profile_raises`

---

## E — Backend contract diff EFE 1.1 → 1.2

### E.1 New REST endpoints (additive)

| Method | Path | Auth | Phase |
|---|---|---|---|
| GET | `/admin/bundles` | OIDC | D47 |
| GET | `/admin/bundles/<slug>/<version>` | OIDC | D47 |
| POST | `/admin/bundles/register` | OIDC admin | D50 |
| POST | `/admin/bundles/<slug>/<version>/promote` | OIDC admin | D50 |
| GET | `/admin/deployments/current` | OIDC | D49 |
| GET | `/admin/deployments/history` | OIDC | D49 |
| POST | `/admin/deployments/dry-run` | OIDC | D49 |
| POST | `/admin/tickets` | OIDC admin | D50 |
| GET | `/admin/trust-store` | OIDC | D47 |

### E.2 New gRPC RPC (additive)

```proto
service EfeAdminService {
  // existing M16-M17 RPCs unchanged …

  rpc DryRunDeployment(DryRunDeploymentRequest) returns (DryRunDeploymentResponse);
}
```

### E.3 Backward compat guarantees

- Tous les endpoints / RPCs M16+M17 + EFE 1.0/1.1 GA inchangés.
- Tout client EFE 1.1 reste compatible avec un server EFE 1.2.
- mcp-server M18 profile=`agent-native` détecte EFE 1.1 vs 1.2 via
  `GET /info` `contract_version` et fallback approprié.
- Profile=`agent-native-deploy` requiert EFE 1.2 server (write
  endpoints).

### E.4 Migration plan

| Étape | Action | Owner | Quand |
|---|---|---|---|
| 1 | Bump `Cargo.toml` `efe-server` v1.1.x → v1.2.0-pre | Rust lead | D47 J1 |
| 2 | Implémenter 9 endpoints REST + 1 RPC gRPC | Rust lead | D47-D50 |
| 3 | Pin tests `crates/efe-server/tests/m18_contract.rs` (~20) | Rust lead | D49-D50 |
| 4 | Bump OpenAPI spec + .proto regen | Rust lead | D49 J3 |
| 5 | Release EFE 1.2.0 image (backward-compat) | Release manager | D50 J5 |
| 6 | mcp-server M18 detect 1.2 via `/info` flag | Python lead | D49 J1 |
| 7 | mcp-server fallback paths for 1.1 server | Python lead | D49 J3 |

---

**FIN**

Document M18 architecture. ~2 700 lignes. Prêt à passer en livraison
sprint D47. Toute revue chirurgicale : ouvrir issue
`docs/m18_review_pass_*` ; pattern récurrent M11-M17.
