# Sprint M19 — Production hardening : architecture

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : suit M18 GA (Bundle & deploy tools livré + 4 sub-livraisons
> + 3 audit passes — 20 défauts attrapés + corrigés ; 452 pin tests
> mcp-server GREEN ; 490 aggregate).
>
> **Pré-requis** : avoir lu :
> - [`EFE_M18_BUNDLE_DEPLOY_TOOLS_ARCHITECTURE.md`](EFE_M18_BUNDLE_DEPLOY_TOOLS_ARCHITECTURE.md)
>   (M18, blueprint complet ; 14 outils bundle/deploy + 5 resources + 5
>   prompts + Ed25519 verify + SLSA parse + Helm values gen).
> - `mcp-server/` (M16+M17+M18 livré, **452 pin tests GREEN**, 34 outils
>   exposés, profiles `core`/`advanced`/`agent-native`/`agent-native-deploy`
>   /`experimental`).
> - [`EFE_BUNDLES_EXTENSION_ARCHITECTURE.md`](EFE_BUNDLES_EXTENSION_ARCHITECTURE.md)
>   (M13, base bundle Ed25519 + SBOM CycloneDX 1.5 + OCI Artifacts).
> - [`EFE_COMPLIANCE_EXTENSION_ARCHITECTURE.md`](EFE_COMPLIANCE_EXTENSION_ARCHITECTURE.md)
>   (M14, threat model 86 STRIDE cumulés ; SOC 2 / ISO 27001 / GDPR
>   Art. 32 / NIS2 / NIST 800-53 mappings).
> - **Spec OpenTelemetry v1.x** :
>   <https://opentelemetry.io/docs/specs/otel/>
> - **Spec Sigstore Bundle v0.3** :
>   <https://github.com/sigstore/protobuf-specs/blob/main/protos/sigstore_bundle.proto>
> - **Spec Rekor v2** :
>   <https://github.com/sigstore/rekor/tree/main/openapi.yaml>
> - **AWS KMS Asymmetric** :
>   <https://docs.aws.amazon.com/kms/latest/developerguide/symmetric-asymmetric.html>
> - **PKCS#11 v3.0** :
>   <https://docs.oasis-open.org/pkcs11/pkcs11-spec/v3.0/cs01/pkcs11-spec-v3.0-cs01.html>
> - **MCP HTTP+SSE transport spec** :
>   <https://modelcontextprotocol.io/specification#transports>
>
> **Mission M19** : transformer le mcp-server EFE d'un binaire **fonctionnel
> et sécurisé** (M16-M18 livrés) en un binaire **production-grade hardened
> et observable**. Quatre piliers couvrent les gaps qui séparent un
> système qui marche d'un système qui se déploie en prod enterprise :
>
> 1. **Observability** — chaque tool call, chaque OCI pull, chaque
>    sampling round, chaque audit event est traçable end-to-end (logs
>    structurés + traces OTel + métriques Prometheus). Aujourd'hui :
>    `logging.basicConfig(level=INFO)` produit des lignes texte
>    sans corrélation. SOC 2 CC7.1/CC7.2 (system monitoring + anomaly
>    detection) demande mieux.
> 2. **Cosign / Sigstore** — vérification des bundles via la chaîne
>    Sigstore complète (Rekor inclusion proof + Sigstore Bundle v0.3 +
>    keyless OIDC identity vs Ed25519 raw seul). Aujourd'hui : la
>    vérification utilise une clé publique unique stockée dans
>    `EFE_OCI_PUBLIC_KEY`. Pour une chaîne d'approvisionnement enterprise,
>    on veut Rekor pour avoir une **transparency log auditable**, et
>    keyless pour éliminer la gestion de clés long-lived.
> 3. **BYOK** (Bring Your Own Key) — les clés publiques de vérification
>    + l'OIDC token du mcp-server proviennent d'un **KMS / HSM / Vault**
>    plutôt que d'env-vars. Six backends : `env` (default, dev), `file`,
>    AWS KMS, GCP KMS, Azure Key Vault, HashiCorp Vault, PKCS#11.
>    SOC 2 CC6.1 (logical access controls) + ISO 27001 A.10.1.2 (key
>    management) demandent ça pour les déploiements bancaires / gov.
> 4. **HTTP+SSE transport + multi-replica** — implémenter
>    `_run_http()` (stub depuis M16) pour servir plusieurs clients
>    depuis un seul process ; rendre les caches distribués (Redis) pour
>    permettre le scale-out multi-replica. Aujourd'hui : un mcp-server
>    par client MCP local (subprocess stdio). Pour servir un parc de
>    clients via load balancer, il faut HTTP+SSE.
>
> Quatre sous-livraisons :
>
> | Sprint | Phase | Jour | Livrable |
> |---|:---:|:---:|---|
> | **D51** | Φ-A | J265 | **Observability** : structured logs (`structlog`), OTel traces (auto + manual), OTel metrics (`/metrics` Prometheus endpoint), trace propagation via JWT, log correlation. Pas de nouveaux tools — instrumentation only. |
> | **D52** | Φ-B | J280 | **Cosign / Sigstore** : `sigstore-python` extra ; nouveau `mcp_server/bundle/sigstore.py` avec Rekor inclusion proof + Sigstore Bundle v0.3 + keyless OIDC identity ; nouveau tool `efe_verify_rekor_inclusion` ; nouveau resource `efe://bundles/{slug}/{version}/transparency` ; extension `efe_verify_bundle_signature` avec `verification_method: ed25519|sigstore|auto`. |
> | **D53** | Φ-C | J295 | **BYOK** : pluggable `mcp_server/crypto/keystore.py` avec 7 backends (env, file, AWS KMS, GCP KMS, Azure KV, Vault, PKCS#11) ; nouveau tool `efe_check_keystore_health` ; nouveau resource `efe://crypto/keystore/status` ; rotation strategy (TTL + signal-driven reload). |
> | **D54** | Φ-D | J310 | **HTTP+SSE + multi-replica** : `_run_http()` implementation ; Redis-backed `RateLimiter` + `BundleCache` + `SamplingCache` ; `/health` + `/ready` probes ; graceful shutdown ; nouveau resource `efe://server/health`. **Audit pass M19 final**. |
>
> **Total livraison M19** : **3 nouveaux tools + 3 nouveaux resources +
> 1 nouveau prompt** (les 4 piliers sont surtout des **infrastructures
> transverses** plutôt que de nouvelles surfaces MCP). Volumétrie
> estimée : **~3 200 LOC code Python + ~2 000 LOC tests + ~250 LOC Rust**
> (3 nouveaux endpoints EFE 1.3 backend, additifs).
>
> **Pin tests cumulés post-M19** : **~520-540 dans `mcp-server/`** (vs
> 452 post-M18 GA = +70 environ).
>
> **Backward compat 100 %** :
>   - Toute la surface M16-M18 (34 tools, 11 resources, 10 prompts)
>     reste inchangée fonctionnellement.
>   - Observability : transparente quand `EFE_OTEL_EXPORTER` non-set.
>   - Cosign : `EFE_VERIFICATION_METHOD=ed25519` (default) garde le
>     comportement M18 ; `=sigstore` ou `=auto` opt-in.
>   - BYOK : `EFE_OCI_PUBLIC_KEY{,_FILE}` continuent à fonctionner ;
>     KMS via `EFE_KEYSTORE_BACKEND=aws-kms|gcp-kms|...` opt-in.
>   - HTTP+SSE : `EFE_MCP_TRANSPORT=http` opt-in ; `=stdio` (default)
>     inchangé.
>
> **Compliance posture EFE 1.0 GA toujours préservée** : zéro nouveau
> sub-processor obligatoire. Sigstore Public Good Instance (Rekor +
> Fulcio) reste **opt-in** (l'opérateur peut self-host Rekor sur prem
> via `EFE_REKOR_URL`). KMS providers (AWS/GCP/Azure) sont les choix
> standards de l'opérateur, pas imposés. M19 ouvre seulement
> l'**option** d'utiliser ces services ; l'opérateur garde le contrôle.
> Conforme **EU Cyber Resilience Act 2024/2847** (renforcé via
> transparency log) + **NIS2 Article 21(g)** (cryptography use,
> renforcé via KMS) + **SOC 2 CC6.1/CC7.1/CC7.2/CC9.1** (renforcés sur
> 4 axes).

---

## Sommaire

1. [Executive summary](#1-executive-summary)
2. [Vision & rationale — pourquoi production hardening](#2-vision--rationale)
3. [Scope & non-goals (delta vs M18 GA)](#3-scope--non-goals)
4. [Catalogue M19 — vue d'ensemble](#4-catalogue-m19--vue-densemble)
5. [Architecture](#5-architecture)
6. [Phase A (D51) — Observability deep-dive](#6-phase-a-d51--observability)
7. [Phase B (D52) — Cosign / Sigstore deep-dive](#7-phase-b-d52--cosign--sigstore)
8. [Phase C (D53) — BYOK deep-dive](#8-phase-c-d53--byok)
9. [Phase D (D54) — HTTP+SSE + multi-replica deep-dive](#9-phase-d-d54--http-sse--multi-replica)
10. [New tools + resources + prompts](#10-new-tools--resources--prompts)
11. [JSON Schemas (forward-refs)](#11-json-schemas-forward-refs)
12. [Backend extensions (EFE 1.3 contract bump)](#12-backend-extensions)
13. [Threat model M19 (delta vs M18)](#13-threat-model-m19)
14. [Performance & SLOs M19](#14-performance--slos-m19)
15. [Compatibility matrix](#15-compatibility-matrix)
16. [Migration M18 → M19 (opt-in everywhere)](#16-migration-m18--m19)
17. [Test plan (~80 nouveaux pin tests)](#17-test-plan)
18. [Deployment scenarios (delta vs M18)](#18-deployment-scenarios)
19. [Roadmap M20+](#19-roadmap-m20)
20. [Open questions](#20-open-questions)
21. [FAQ](#21-faq)
22. [Glossary M19](#22-glossary-m19)
23. [References](#23-references)

**Annexes**

- [A — Full JSON Schemas](#a--full-json-schemas)
- [B — KMS provider integration details](#b--kms-provider-integration-details)
- [C — OTel span semantic conventions](#c--otel-span-semantic-conventions)
- [D — Pin tests inventory](#d--pin-tests-inventory)
- [E — Backend contract diff EFE 1.2 → 1.3](#e--backend-contract-diff)

---

## 1. Executive summary

| Item | Valeur |
|---|---|
| **Mission** | Transformer le mcp-server EFE d'un binaire fonctionnel-et-sécurisé en un binaire production-grade hardened-et-observable. 4 piliers : observability, Sigstore, BYOK, HTTP+SSE multi-replica. |
| **Durée** | 4 sprints (D51 → D54), ~50 jours-ingénieur. |
| **Ressources** | 1 senior Python (lead) + 1 senior Python (sigstore + KMS) + 0.5 senior Rust (3 nouveaux endpoints EFE 1.3 + Helm chart updates) + 0.5 SRE (OTel collector + Prometheus + Grafana dashboards) + 0.25 sécurité (KMS threat model review) + 0.25 doc (operator guide). |
| **Budget** | ~$320 k incluant audit pass + Sigstore prod onboarding. |
| **Volumétrie code** | ~3 200 LOC Python + ~250 LOC Rust + ~2 000 LOC tests. |
| **Pin tests** | **+70 GREEN** (vs 452 M18 → ~522 cumulés mcp-server). |
| **Backward compat** | 100 %. Tout opt-in via env-vars ; defaults bit-for-bit M18. |
| **Risk profile** | **Medium-high** — `sigstore-python` est une nouvelle dépendance lourde (~30 transitive) ; KMS providers introduisent network calls + creds management ; HTTP+SSE est une nouvelle surface d'attaque. Mitigation : `[sigstore]` + `[byok]` extras opt-in, audit pass post-livraison, threat model M19 enrichi (8 nouvelles menaces). |
| **Compliance impact** | **Renforce sans modifier la posture** : zéro nouveau sub-processor obligatoire (Sigstore Public Good est opt-in, KMS providers sont customer-choice). Documents existants `gdpr_art32.md`, `soc2_evidence.md`, `iso27001.md` reçoivent des sections d'extension ; nouveau doc `compliance/cosign_sigstore_compliance.md` + extension `compliance/byok_keystore.md`. |
| **EFE 1.3 contract bump** | **Oui** — semver-mineur. 3 nouveaux endpoints REST + 1 RPC gRPC (additifs). EFE 1.2 clients restent compat avec EFE 1.3 server. mcp-server M19 features sont **toutes** opt-in côté backend (un EFE 1.2 backend continue à servir le mcp-server M19 en mode dégradé). |

---

## 2. Vision & rationale

### 2.1 Le saut "fonctionnel" → "production-grade"

M16-M18 ont livré le **fonctionnel** : 34 tools MCP, 11 resources, 10
prompts, threat model 28 STRIDE mitigés. Mais une feature peut
fonctionner et ne PAS être production-grade. Quatre dimensions
manquent :

1. **Observability** — quand un tool call échoue à 3am, l'opérateur
   doit pouvoir tracer la cause en < 5 min. Aujourd'hui le mcp-server
   produit des log lines texte non-corrélés. Pour une chaîne
   `LLM → MCP → backend EFE → OCI registry`, il faut un trace_id
   propagé end-to-end + des métriques agrégées (rate, latency, errors)
   exposées via Prometheus.
2. **Supply chain transparency** — Ed25519 raw verification (M18) protège
   contre la modification du bundle, mais **ne prouve PAS** que le
   bundle a été produit par un builder identifiable et reviewable.
   Sigstore + Rekor donnent ça : chaque signature est dans un
   transparency log public/privé, vérifiable en lecture par
   n'importe qui. SLSA Level 3 demande ça.
3. **Key management at scale** — un déploiement avec 50 tenants peut
   avoir 50 trust stores différents, des rotations trimestrielles,
   des HSM différents par compliance zone. `EFE_OCI_PUBLIC_KEY` env
   ne scale pas. KMS / HSM / Vault sont la convention enterprise.
4. **Multi-replica** — un mcp-server en stdio sert un seul client
   MCP. Pour un déploiement enterprise (10 000 utilisateurs avec
   Claude Desktop / Cursor), on veut HTTP+SSE derrière un load
   balancer + N replicas stateless.

### 2.2 4 patterns d'usage observés (sources : 5 design partners post-M18)

**Pattern A — « Incident response »** : prod down soupçonné ; opérateur
veut savoir si un tool call a échoué récemment et avec quel error code.
**M18** : grep `audit.recent` event log + correlate à logs serveur
manuellement. **M19** : Grafana dashboard avec
`efe_mcp.tool.errors{tool_name="*"}` agrégé par minute.

**Pattern B — « Compliance audit »** : auditeur SOC 2 demande "show me
how you protect crypto keys". **M18** : `EFE_OCI_PUBLIC_KEY` env var
documenté. **M19** : opérateur montre le KMS policy + accès logs +
rotation history dans CloudTrail.

**Pattern C — « Supply chain audit »** : security team demande "prove
that all bundles deployed last quarter came from your trusted builder".
**M18** : grep manifest annotations + cross-check avec GitHub
Actions logs. **M19** : agent suit le prompt
`audit_bundle_provenance` qui pull Rekor entries pour chaque bundle
+ vérifie OIDC identity claims.

**Pattern D — « Capacity planning »** : ops team veut savoir si le
mcp-server tient la charge prévue. **M18** : pas de métriques. **M19** :
`efe_mcp.tool.duration_ms` p99 + `efe_mcp.rate_limit.consumed` rate
+ Prometheus alerting.

### 2.3 Pourquoi 4 piliers en 1 sprint

Chaque pilier addresse un gap distinct mais ils s'**inter-renforcent** :
- Observability surface KMS calls latency / Sigstore cache hits.
- Sigstore intègre avec KMS (Sigstore Bundle peut être signé par
  un cert dérivé d'un KMS-stored key).
- Multi-replica nécessite des caches partagés (Redis) qui sont
  monitorés via OTel.
- HTTP+SSE expose `/metrics` qui consomme l'infra observability.

→ Faire ces 4 piliers ENSEMBLE évite des refactors successifs et
permet d'atteindre la "production-grade GA" en un seul cycle.

### 2.4 Pourquoi pas de nouveaux tools agent-native ?

M19 n'ajoute QUE 3 tools (vs 14 en M17, 14 en M18). C'est intentionnel :
**production hardening = renforcer, pas étendre**. Les 3 nouveaux
tools (`efe_verify_rekor_inclusion`, `efe_check_keystore_health`,
`efe_rotate_acknowledged`) sont des **diagnostics** que l'opérateur
voudra appeler post-incident, pas des features end-user.

---

## 3. Scope & non-goals

### 3.1 In scope (livraison M19)

| ID | Catégorie | Item | Phase |
|---|---|---|---|
| 3.1.1 | Infra | Structured logging (`structlog`) | D51 |
| 3.1.2 | Infra | OTel traces (auto-instrument httpx + manual dispatch) | D51 |
| 3.1.3 | Infra | OTel metrics (Prometheus exposition) | D51 |
| 3.1.4 | Infra | `/metrics` endpoint (HTTP transport prerequisite) | D51 |
| 3.1.5 | Infra | Trace propagation via JWT request_id | D51 |
| 3.1.6 | Module | `mcp_server/bundle/sigstore.py` (Rekor + Bundle v0.3) | D52 |
| 3.1.7 | Tool | `efe_verify_rekor_inclusion` | D52 |
| 3.1.8 | Resources | `efe://bundles/{slug}/{version}/transparency` | D52 |
| 3.1.9 | Schema | `efe_verify_bundle_signature.verification_method` arg | D52 |
| 3.1.10 | Module | `mcp_server/crypto/keystore.py` + 7 backends | D53 |
| 3.1.11 | Tool | `efe_check_keystore_health` | D53 |
| 3.1.12 | Tool (W) | `efe_rotate_acknowledged` (audit-only metadata) | D53 |
| 3.1.13 | Resources | `efe://crypto/keystore/status` | D53 |
| 3.1.14 | Infra | HTTP+SSE transport (`_run_http()` impl) | D54 |
| 3.1.15 | Infra | Redis-backed RateLimiter + BundleCache + SamplingCache | D54 |
| 3.1.16 | Resources | `efe://server/health` | D54 |
| 3.1.17 | Prompt | `incident_supply_chain_audit` | D52 |
| 3.1.18 | Backend | EFE 1.3 contract bump (3 REST + 1 gRPC additifs) | D52-D54 |
| 3.1.19 | Audit | M19 audit pass post-livraison | D54 |

### 3.2 Out of scope (explicitement)

| Item | Pourquoi exclu | Successeur |
|---|---|---|
| Signing operations (cosign sign) | M18 §2.1 invariant : MCP server ne tient JAMAIS de clés privées | Jamais (operator's CI) |
| HSM key generation | Idem — HSM ops via PKCS#11 sont read-only depuis MCP | Jamais |
| Direct Vault sealing/unsealing | Hors scope MCP ; ops domain | Jamais |
| TUF root-of-trust management | Out of MCP scope ; cosign CLI handles | Jamais |
| Custom OIDC identity providers | Use existing JWT auth (D15) | M21+ |
| Cross-region replication failover | Application-level concern, not MCP | M22+ |
| Cluster autoscaling | K8s HPA/VPA (Helm chart concern) | Helm chart M19 update |
| Multi-cluster federation | Scope creep ; KMS doesn't help | Never (Helm M21) |
| GraphQL transport | MCP spec is JSON-RPC ; non-standard | Jamais |
| WebSocket transport | MCP spec stdio + HTTP+SSE only | Jamais |
| Service mesh (Istio, Linkerd) sidecar | Deployment concern ; Helm M19 update OK | Helm chart M19 |
| Distributed tracing AT THE LLM LEVEL | LLM client out of MCP scope | Jamais |
| Custom metrics aggregation backend | Use Prometheus / OTel collector | Jamais |
| Tracker webhooks (M18 D50 callback) | Out of scope ; backend's concern | M21+ |

### 3.3 Constraints

- **Aucune dépendance Python obligatoire ajoutée** : `sigstore-python`,
  `boto3`, `google-cloud-kms`, `azure-keyvault-keys`, `hvac`,
  `python-pkcs11` sont tous dans des extras (`[sigstore]`, `[aws-kms]`,
  `[gcp-kms]`, `[azure-kv]`, `[vault]`, `[pkcs11]`).
- **Default behaviour bit-for-bit M18** : sans aucune env-var M19,
  le mcp-server se comporte comme M18.
- **OTel optionnel** : `opentelemetry-api` + `opentelemetry-sdk` sont
  des deps du tronc (instrumentation no-op si pas d'exporter).
- **Sigstore Public Good Instance opt-in** : default backend
  `EFE_REKOR_URL=` empty → Sigstore vérifications skipped. Customer
  peut self-host Rekor on-prem.
- **Audit pass obligatoire** : pattern récurrent M11-M18 (~6-8
  défauts attendus).

---

## 4. Catalogue M19 — vue d'ensemble

### 4.1 Outils (37 cumulés au final)

```
                       ┌─────────────────────────────────────┐
                       │  EFE MCP Server v0.4.0 (M19 GA)     │
                       │  Profile : core | advanced |        │
                       │            agent-native |           │
                       │            agent-native-deploy |    │
                       │            experimental             │
                       └─────────────────┬───────────────────┘
                                         │
       ┌─────────────────────────────────┼──────────────────────────┐
       │                                                             │
   M16-M18 (34 tools)                M19 (3 nouveaux tools)         │
   ─────────────                     ─────────────────────────       │
   [unchanged]                       D52 : efe_verify_rekor_inclusion │
                                     D53 : efe_check_keystore_health  │
                                     D53 : efe_rotate_acknowledged (W)│
```

### 4.2 Resources (14 types au final)

| URI pattern | Phase | Description |
|---|:---:|---|
| `efe://catalogs/...` | M17 | M17 inchangé |
| `efe://audit/recent` | M17 | M17 inchangé |
| `efe://quotas` | M17 | M17 inchangé |
| `efe://samples/{id}` | M17 | M17 inchangé |
| `efe://bundles[/{slug}/{version}[/sub]]` | M18 D48 | M18 inchangé |
| `efe://deployments/current` | M18 D49 | M18 inchangé |
| `efe://bundles/{slug}/{version}/transparency` | **D52** | **NEW** : Rekor entry + Sigstore Bundle v0.3 |
| `efe://crypto/keystore/status` | **D53** | **NEW** : keystore health |
| `efe://server/health` | **D54** | **NEW** : runtime health (replicas, cache stats, OTel status) |

### 4.3 Prompts (11 templates au final)

| ID | Variables | Phase |
|---|---|:---:|
| (10 M17/M18 prompts unchanged) | … | M17/M18 |
| `incident_supply_chain_audit` | `{since}`, `{tenant?}` | **D52** |

---

## 5. Architecture

### 5.1 Module map (delta vs M18 GA)

```
mcp-server/
├── pyproject.toml                  ← UPDATE : new extras
│                                              [sigstore]
│                                              [aws-kms], [gcp-kms]
│                                              [azure-kv], [vault]
│                                              [pkcs11]
│                                              [redis] (multi-replica)
├── Dockerfile                      ← UPDATE : numpy + cryptography
│                                              base ; extras built into
│                                              named tags
├── README.md                       ← UPDATE : full M19 ops guide
├── mcp_server/
│   ├── __init__.py                 ← UPDATE : version 0.4.0
│   ├── efe_mcp_server.py           ← UPDATE : OTel init, http transport,
│   │                                          /metrics endpoint
│   ├── dispatcher.py               ← UPDATE : OTel spans, trace_id propagation
│   ├── backend_client.py           ← UPDATE : OTel auto-instrument httpx
│   ├── auth.py                     ← UPDATE : trace_id extracted from JWT
│   ├── audit_emitter.py            ← UPDATE : OTel exporter for events
│   ├── rate_limiter.py             ← UPDATE : Redis backend optional
│   ├── profile.py                  ← UNCHANGED
│   ├── sampling.py                 ← UPDATE : OTel spans per round
│   ├── streaming.py                ← UPDATE : trace_id in progress events
│   ├── observability/              ← NEW directory
│   │   ├── __init__.py
│   │   ├── tracing.py              ← OTel tracer setup + decorators
│   │   ├── metrics.py              ← Prometheus exposition + OTel meter
│   │   ├── logging.py              ← structlog setup with trace_id binding
│   │   └── propagation.py          ← W3C TraceContext extract/inject
│   ├── crypto/                     ← NEW directory
│   │   ├── __init__.py
│   │   ├── keystore.py             ← Abstract KeyStore + factory
│   │   ├── env_keystore.py         ← Default (M18 carry-over)
│   │   ├── file_keystore.py        ← Multi-key directory load
│   │   ├── aws_kms.py              ← boto3.client('kms')
│   │   ├── gcp_kms.py              ← google.cloud.kms
│   │   ├── azure_kv.py             ← azure.keyvault.keys
│   │   ├── vault.py                ← hvac (HashiCorp Vault)
│   │   ├── pkcs11.py               ← python-pkcs11
│   │   └── rotation.py             ← TTL cache + signal-driven reload
│   ├── bundle/
│   │   ├── ...                     ← M18 unchanged
│   │   ├── sigstore.py             ← NEW : Sigstore Bundle v0.3 + Rekor
│   │   └── verify.py               ← UPDATE : verification_method dispatch
│   ├── transport/                  ← NEW directory (M19 D54)
│   │   ├── __init__.py
│   │   ├── stdio.py                ← extracted from efe_mcp_server.py
│   │   ├── http_sse.py             ← NEW : SSE + JSON-RPC over HTTP
│   │   ├── health.py               ← /health + /ready endpoints
│   │   └── shutdown.py             ← Graceful shutdown coordinator
│   ├── state/
│   │   ├── ...                     ← M16-M18 unchanged
│   │   └── redis_backed.py         ← NEW : Redis adapter for caches/limiter
│   ├── tools/
│   │   ├── ...                     ← M16-M18 unchanged
│   │   ├── verify_rekor_inclusion.py    ← NEW (D52)
│   │   ├── check_keystore_health.py     ← NEW (D53)
│   │   ├── rotate_acknowledged.py       ← NEW (D53, write)
│   │   └── verify_bundle_signature.py   ← UPDATE : verification_method
│   ├── resources/
│   │   ├── ...                     ← M16-M18 unchanged
│   │   ├── transparency.py         ← NEW : efe://bundles/.../transparency
│   │   ├── keystore.py             ← NEW : efe://crypto/keystore/status
│   │   └── server_health.py        ← NEW : efe://server/health
│   └── prompts/
│       ├── ...                     ← M16-M18 unchanged
│       ├── registry.py             ← UPDATE : +1 prompt
│       └── incident_supply_chain_audit.md  ← NEW (D52)
└── tests/
    ├── conftest.py                 ← UPDATE : OTel + KMS + sigstore fixtures
    ├── ...                         ← M16-M18 unchanged
    ├── test_observability.py            ← NEW (D51, ~12 tests)
    ├── test_sigstore_verify.py          ← NEW (D52, ~12 tests)
    ├── test_rekor_inclusion.py          ← NEW (D52, ~8 tests)
    ├── test_keystore_factory.py         ← NEW (D53, ~10 tests)
    ├── test_keystore_aws.py             ← NEW (D53, ~6 tests, conditional)
    ├── test_keystore_file.py            ← NEW (D53, ~6 tests)
    ├── test_keystore_health_tool.py     ← NEW (D53, ~6 tests)
    ├── test_http_transport.py           ← NEW (D54, ~10 tests)
    ├── test_redis_backed.py             ← NEW (D54, ~6 tests)
    └── test_audit_pass_m19.py           ← NEW post-livraison (D54)

crates/efe-server/                  ← UPDATE for EFE 1.3 contract
├── src/
│   ├── http/routes.rs              ← UPDATE : +3 endpoints
│   ├── grpc/service.rs             ← UPDATE : +1 RPC
│   └── observability/              ← NEW (Rust side)
│       ├── otel.rs                 ← Backend OTel exporter
│       └── metrics.rs              ← /metrics exposition
└── tests/
    └── m19_contract.rs             ← NEW (~15 tests EFE 1.3)
```

### 5.2 Process & threading model (delta vs M18)

M16-M18 utilisent asyncio single-event-loop ; M19 conserve ce modèle.
Notes spécifiques M19 :

- **HTTP+SSE transport (D54)** : `uvicorn` (or équivalent ASGI) wraps
  the dispatcher. Multiple SSE connections share the event loop ;
  no per-client thread. Backpressure via asyncio Semaphore.
- **Redis I/O (D54)** : `redis.asyncio.Redis` (already async). All
  cache/limiter calls non-blocking.
- **OTel exporter** (`opentelemetry-exporter-otlp`) : runs its own
  background thread (managed by SDK) — no event loop interference.
- **KMS calls (D53)** : `boto3.client('kms')` is sync ; we wrap in
  `asyncio.to_thread()` to avoid blocking. `google-cloud-kms` has
  async client → use directly.

### 5.3 Lifecycle (delta vs M18)

```
Boot
  │
  ├─ Load profile (env)
  ├─ Init structlog + OTel tracer + meter (D51)
  ├─ Init keystore via factory (D53)
  │   ├─ env / file : sync init
  │   ├─ AWS KMS : verify boto3 creds + cache initial pubkeys
  │   ├─ GCP KMS : verify gcloud creds + cache
  │   ├─ ... (other backends)
  │   └─ Schedule TTL refresh
  ├─ Init backend client (auto-instrumented)
  ├─ Init dispatcher with OTel-wrapped methods
  ├─ Init transport :
  │   ├─ stdio (default) → run_stdio()
  │   └─ http+sse → run_http() with /sse, /messages, /metrics, /health, /ready
  └─ Run

Each request
  │
  ├─ Extract trace_id (W3C TraceContext header OR JWT claim)
  ├─ Bind structlog context : trace_id, span_id, tenant_id_hash
  ├─ Dispatcher.dispatch(tool_name, args)
  │   ├─ OTel span "mcp.tool.dispatch"
  │   ├─ Validation
  │   ├─ Rate-limit (Redis-backed if configured)
  │   ├─ Handler (auto-instrumented httpx, sampling, OCI)
  │   └─ Audit emit (OTel exporter sends to collector)
  └─ Response

Shutdown (D54)
  │
  ├─ SIGTERM received
  ├─ Stop accepting new connections
  ├─ Wait for in-flight calls (max grace = 30s)
  ├─ Flush OTel exporter
  ├─ Close keystore connections
  ├─ Close Redis connections
  └─ Exit
```

---

## 6. Phase A (D51) — Observability deep-dive

### 6.1 Three pillars du SLO observability

OpenTelemetry distingue 3 signaux qui se complètent :

| Signal | Use case | Cost | Cardinality |
|---|---|---|---|
| **Traces** | Investigation specific request latency / error chain | High (1-10 KB / request) | Low (per-request UUID) |
| **Metrics** | Aggregate trends, alerts, dashboards | Low (~100 B / scrape interval) | Bounded (~100 series) |
| **Logs** | Detailed events, audit trail | Medium (1 KB / event) | High (free-text) |

M19 implémente les 3 + corrélation cross-signal (trace_id présent dans
chaque signal).

### 6.2 Structured logging (`structlog`)

`structlog` est déjà dans `pyproject.toml` deps (depuis M16). M19
l'active réellement.

Configuration (D51) :

```python
# mcp_server/observability/logging.py
import structlog
import logging
import sys

def setup_structured_logging(*, level: str = "INFO", json_output: bool = True):
    """Configure structlog with trace correlation + JSON for prod."""
    timestamper = structlog.processors.TimeStamper(fmt="iso", utc=True)
    
    processors = [
        structlog.contextvars.merge_contextvars,        # adds trace_id, span_id
        structlog.stdlib.add_log_level,
        structlog.stdlib.add_logger_name,
        timestamper,
        structlog.processors.StackInfoRenderer(),
        structlog.processors.format_exc_info,
        structlog.processors.UnicodeDecoder(),
    ]
    if json_output:
        processors.append(structlog.processors.JSONRenderer())
    else:
        processors.append(structlog.dev.ConsoleRenderer(colors=True))
    
    structlog.configure(
        processors=processors,
        wrapper_class=structlog.stdlib.BoundLogger,
        logger_factory=structlog.stdlib.LoggerFactory(),
        cache_logger_on_first_use=True,
    )
    # Standard logging → stderr (stdout reserved for stdio JSON-RPC)
    logging.basicConfig(
        format="%(message)s",
        stream=sys.stderr,
        level=getattr(logging, level.upper()),
    )
```

Usage in dispatcher :

```python
import structlog
log = structlog.get_logger("efe_mcp.dispatcher")

async def dispatch(self, *, tool_name, args, ...):
    log.info("tool_dispatch_start", tool=tool_name, tenant_hash=self.tenant_hash())
    try:
        result = await tool.handler(args, ctx)
        log.info("tool_dispatch_done", tool=tool_name, duration_ms=duration)
        return result
    except Exception as exc:
        log.exception("tool_dispatch_error", tool=tool_name, error=str(exc))
        raise
```

### 6.3 OpenTelemetry traces

Setup :

```python
# mcp_server/observability/tracing.py
import os
from opentelemetry import trace
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.resources import Resource
from opentelemetry.sdk.trace.export import BatchSpanProcessor
from opentelemetry.exporter.otlp.proto.http.trace_exporter import OTLPSpanExporter

def setup_tracing(*, service_name: str = "efe-mcp-server"):
    """Init OTel tracer. No-op if EFE_OTEL_EXPORTER unset."""
    exporter_url = os.environ.get("EFE_OTEL_EXPORTER")
    if not exporter_url:
        # Set a no-op tracer provider — instrumentation calls are free.
        return
    
    resource = Resource.create({
        "service.name": service_name,
        "service.version": _get_version(),
        "service.instance.id": _instance_id(),
        "deployment.environment": os.environ.get("EFE_DEPLOYMENT_ENV", "production"),
    })
    
    provider = TracerProvider(resource=resource)
    exporter = OTLPSpanExporter(endpoint=exporter_url)
    provider.add_span_processor(BatchSpanProcessor(exporter))
    trace.set_tracer_provider(provider)
    
    # Auto-instrument httpx (used by backend_client + OCI client)
    from opentelemetry.instrumentation.httpx import HTTPXClientInstrumentor
    HTTPXClientInstrumentor().instrument()
```

Manual span examples :

```python
# In dispatcher.py
from opentelemetry import trace
_tracer = trace.get_tracer("efe_mcp.dispatcher")

async def dispatch(self, *, tool_name, args, ...):
    with _tracer.start_as_current_span(
        "mcp.tool.dispatch",
        attributes={
            "mcp.tool.name": tool_name,
            "mcp.tenant.hash": self.tenant_hash(),
            "mcp.profile": self.profile.value,
        },
    ) as span:
        try:
            result = await tool.handler(args, ctx)
            span.set_attribute("mcp.tool.is_error", False)
            return result
        except Exception as exc:
            span.set_attribute("mcp.tool.is_error", True)
            span.set_attribute("mcp.tool.error_kind", type(exc).__name__)
            span.record_exception(exc)
            raise
```

Span semantic conventions in **Annexe C**.

### 6.4 OpenTelemetry metrics

```python
# mcp_server/observability/metrics.py
from opentelemetry.metrics import get_meter_provider, set_meter_provider
from opentelemetry.sdk.metrics import MeterProvider
from opentelemetry.sdk.metrics.export import PrometheusMetricReader

def setup_metrics():
    reader = PrometheusMetricReader()
    provider = MeterProvider(metric_readers=[reader])
    set_meter_provider(provider)
    
    meter = provider.get_meter("efe_mcp", version=_get_version())
    
    return {
        "tool_duration_ms": meter.create_histogram(
            "efe_mcp.tool.duration_ms",
            unit="ms",
            description="Latency of MCP tool dispatch (full pipeline)",
        ),
        "tool_errors": meter.create_counter(
            "efe_mcp.tool.errors",
            description="Tool calls ending in error, by tool_name + error_code",
        ),
        "rate_limit_consumed": meter.create_counter(
            "efe_mcp.rate_limit.consumed",
            description="Rate-limit weight consumed, by tool_name",
        ),
        "bundle_cache_hits": meter.create_counter(
            "efe_mcp.bundle_cache.hits",
            description="Bundle cache hits, by namespace",
        ),
        "bundle_cache_misses": meter.create_counter(
            "efe_mcp.bundle_cache.misses",
            description="Bundle cache misses, by namespace",
        ),
        "sampling_rounds": meter.create_counter(
            "efe_mcp.sampling.rounds",
            description="Sampling rounds consumed, by tool_name",
        ),
        "oci_requests": meter.create_counter(
            "efe_mcp.oci.requests",
            description="OCI registry HTTP requests, by registry_host + status",
        ),
        "audit_events_emitted": meter.create_counter(
            "efe_mcp.audit.events_emitted",
            description="D14-bis audit events emitted, by ok=true|false",
        ),
        "keystore_lookups": meter.create_counter(
            "efe_mcp.keystore.lookups",
            description="Keystore key lookups, by backend + status",
        ),
        "rekor_verifications": meter.create_counter(
            "efe_mcp.rekor.verifications",
            description="Rekor inclusion proof verifications, by ok=true|false",
        ),
    }
```

Cardinality control (TM-M19-I-02 mitigation) :

- `tool_name` : bounded enum (37 values)
- `error_code` : bounded enum (~15 values)
- `tenant_hash` : `sha256(tenant_id)[:8]` — stable but bounded by N tenants
- `registry_host` : bounded by `EFE_OCI_REGISTRY_ALLOWLIST` (~5 typical)
- `cache_namespace` : bounded enum (3 values)
- `keystore_backend` : bounded enum (7 values)

→ Total cardinality bounded at ~37 × 15 × 5 × 3 ≈ 8 K series, well within
Prometheus recommended limits (10 K - 100 K per metric).

### 6.5 Trace propagation

JWT-based : when a client sends a `X-Trace-Context: 00-{trace_id}-{span_id}-01`
header, we extract and bind to the current span. When using stdio
transport, the trace_id is generated per request.

W3C TraceContext spec : <https://www.w3.org/TR/trace-context/>

```python
# mcp_server/observability/propagation.py
from opentelemetry.propagate import extract, inject
from opentelemetry.trace import set_span_in_context

def extract_trace_context_from_request(meta: dict) -> dict:
    """Extract W3C TraceContext from MCP request `_meta` field."""
    headers = meta.get("trace_context") if isinstance(meta, dict) else None
    if not isinstance(headers, dict):
        return {}
    return extract(headers)
```

### 6.6 SLO M19-OBS-*

| ID | SLO | Cible |
|---|---|---|
| M19-OBS-1 | Trace export overhead | < 5 % p99 latency |
| M19-OBS-2 | Metrics scrape latency | < 50 ms |
| M19-OBS-3 | Log volume | ≤ 100 KB / minute / replica |
| M19-OBS-4 | Trace sampling rate | configurable (default 100 %, prod 10 %) |
| M19-OBS-5 | OTel exporter buffer | drop on backpressure (no blocking) |
| M19-OBS-6 | Cardinality | ≤ 10 K series total |

---

## 7. Phase B (D52) — Cosign / Sigstore deep-dive

### 7.1 Sigstore architecture overview

Sigstore est composé de 4 services :

| Composant | Rôle | M19 dependency |
|---|---|---|
| **Cosign** | CLI signer/verifier (used by operator CI) | NEVER invoked by MCP |
| **Fulcio** | Short-lived cert authority | Read public chain |
| **Rekor** | Append-only transparency log | **Read inclusion proofs** |
| **TUF** | Sigstore root of trust | Bundled in `sigstore-python` |

M19 utilise **uniquement** des opérations de lecture :
- Pull Sigstore Bundle v0.3 from OCI referrer
- Verify signature against Fulcio cert chain
- Verify Rekor inclusion proof (transparency log integrity)
- Optional : verify OIDC identity (e.g. "github.com/efe-project/efe-bundles")

### 7.2 Sigstore Bundle v0.3 format

Le format "Sigstore Bundle" (`application/vnd.dev.sigstore.bundle+json;version=0.3`)
est l'envelope all-in-one qui combine :
- Signature (Ed25519 ou ECDSA-P256)
- Cert chain (Fulcio short-lived)
- Rekor inclusion proof
- Timestamp

```json
{
  "mediaType": "application/vnd.dev.sigstore.bundle+json;version=0.3",
  "verificationMaterial": {
    "x509CertificateChain": {
      "certificates": [{"rawBytes": "..."}]
    },
    "tlogEntries": [{
      "logIndex": 12345,
      "logId": {"keyId": "..."},
      "kindVersion": {"kind": "intoto", "version": "0.0.2"},
      "integratedTime": 1735000000,
      "inclusionPromise": {"signedEntryTimestamp": "..."},
      "inclusionProof": {
        "logIndex": 12345,
        "rootHash": "...",
        "treeSize": 12346,
        "hashes": ["...", "..."],
        "checkpoint": {"envelope": "..."}
      },
      "canonicalizedBody": "..."
    }]
  },
  "messageSignature": {
    "messageDigest": {"algorithm": "SHA2_256", "digest": "..."},
    "signature": "..."
  }
}
```

### 7.3 `mcp_server/bundle/sigstore.py` skeleton

```python
"""Sigstore Bundle v0.3 + Rekor verification (M19 D52).

Pure-Python via `sigstore-python` library (v3.x). NEVER invokes
cosign CLI subprocess. Optional dep behind `[sigstore]` extra so
the default install stays slim.

Threats addressed :
  - TM-M19-S-04 (Rekor entry forgery) : multi-witness verification
    via TUF-signed Rekor public key.
  - TM-M19-T-03 (Sigstore Bundle tampering) : full schema validation
    + cert chain verification.
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Optional


# Lazy imports — sigstore-python is in [sigstore] extra.
try:
    from sigstore.verify import Verifier
    from sigstore.verify.policy import Identity, AnyOf
    from sigstore.models import Bundle as SigstoreBundle
    from sigstore.errors import VerificationError
    _SIGSTORE_AVAILABLE = True
except ImportError:
    _SIGSTORE_AVAILABLE = False


@dataclass(frozen=True)
class SigstoreVerificationResult:
    verified: bool
    method: str   # 'sigstore_bundle' | 'unavailable'
    rekor_log_index: Optional[int]
    rekor_inclusion_verified: bool
    cert_chain_verified: bool
    signed_by_identity: Optional[str]   # OIDC subject if keyless
    signed_by_issuer: Optional[str]     # OIDC issuer
    integrated_time_unix: Optional[int]
    error: Optional[str]


def verify_sigstore_bundle(
    *,
    bundle_bytes: bytes,
    artifact_bytes: bytes,
    expected_identity: Optional[str] = None,
    expected_issuer: Optional[str] = None,
    rekor_url: Optional[str] = None,
) -> SigstoreVerificationResult:
    """Verify a Sigstore Bundle v0.3 against the artifact it signs.
    
    `expected_identity` : OIDC subject (e.g. `https://github.com/efe-project/efe-bundles/.github/workflows/bundle-release.yml@refs/heads/main`)
    `expected_issuer`   : OIDC issuer (e.g. `https://token.actions.githubusercontent.com`)
    `rekor_url`         : self-hosted Rekor URL (default = Sigstore Public Good)
    """
    if not _SIGSTORE_AVAILABLE:
        return SigstoreVerificationResult(
            verified=False, method="unavailable",
            rekor_log_index=None, rekor_inclusion_verified=False,
            cert_chain_verified=False, signed_by_identity=None,
            signed_by_issuer=None, integrated_time_unix=None,
            error="sigstore extra not installed ; "
                  "install with `pip install efe-mcp-server[sigstore]`",
        )
    
    try:
        bundle = SigstoreBundle.from_json(bundle_bytes)
    except Exception as exc:
        return SigstoreVerificationResult(
            verified=False, method="sigstore_bundle",
            rekor_log_index=None, rekor_inclusion_verified=False,
            cert_chain_verified=False, signed_by_identity=None,
            signed_by_issuer=None, integrated_time_unix=None,
            error=f"bundle parse failed: {exc}",
        )
    
    verifier = Verifier.production()  # TUF-rooted Sigstore PGI
    if rekor_url:
        verifier = _verifier_with_custom_rekor(rekor_url)
    
    policy = (
        Identity(identity=expected_identity, issuer=expected_issuer)
        if expected_identity else None
    )
    
    try:
        verifier.verify_artifact(
            input_=artifact_bytes,
            bundle=bundle,
            policy=policy,
        )
    except VerificationError as exc:
        return _result(
            verified=False, bundle=bundle,
            error=f"verification failed: {exc}",
        )
    
    return _result(verified=True, bundle=bundle, error=None)
```

### 7.4 Tool extension : `efe_verify_bundle_signature`

```json
{
  ...
  "properties": {
    ...
    "verification_method": {
      "type": "string",
      "enum": ["ed25519", "sigstore", "auto"],
      "default": "auto"
    },
    "expected_identity": {
      "type": "string",
      "maxLength": 500
    },
    "expected_issuer": {
      "type": "string",
      "maxLength": 200
    }
  }
}
```

`auto` mode : try Sigstore Bundle first ; if absent, fall back to
Ed25519 raw signature (M18 carry-over). Surface `verification_method:
"sigstore"` or `"ed25519"` in the response so the agent knows which
chain succeeded.

### 7.5 New tool : `efe_verify_rekor_inclusion`

Standalone Rekor proof check, useful for incident investigation
when we want to confirm a specific log entry exists.

```json
{
  "type": "object",
  "title": "EfeVerifyRekorInclusionInput",
  "properties": {
    "log_index": {"type": "integer", "minimum": 0},
    "expected_artifact_digest": {
      "type": "string",
      "pattern": "^sha256:[a-f0-9]{64}$"
    },
    "rekor_url": {"type": "string", "maxLength": 200}
  },
  "required": ["log_index", "expected_artifact_digest"]
}
```

Output :
```json
{
  "verified": true,
  "log_index": 12345,
  "tree_size": 12346,
  "root_hash": "abc...",
  "checkpoint_signature_verified": true,
  "integrated_time_unix": 1735000000,
  "kind_version": {"kind": "intoto", "version": "0.0.2"},
  "rekor_url": "https://rekor.sigstore.dev"
}
```

### 7.6 New resource : `efe://bundles/{slug}/{version}/transparency`

Returns the parsed Rekor entry + Sigstore Bundle metadata for a bundle.

### 7.7 New prompt : `incident_supply_chain_audit`

Walks the agent through full chain audit during an incident :

```markdown
A supply chain incident is suspected. We need to audit every bundle
deployed since `{since}`{#if tenant} for tenant `{tenant}`{/if}.

1. List bundles via `efe_list_available_bundles` (or
   `efe://bundles` resource). Filter by registered_at ≥ {since}.
2. For each bundle :
   a. `efe_verify_bundle_signature` with `verification_method:auto`.
      Record the method used (sigstore or ed25519) + verified status.
   b. Read `efe://bundles/{slug}/{version}/transparency`. Confirm
      Rekor entry exists + log_index is monotonically increasing.
   c. `efe_verify_rekor_inclusion` for each log_index. Confirm
      checkpoint signature verifies.
3. Cross-reference signed_by_identity with the expected builder
   (`https://github.com/efe-project/efe-bundles/...`).
4. Output a markdown report : Bundle | Method | Verified | Rekor OK |
   Identity OK | Status. Any row with Status != PASS triggers
   `compliance/incident_response.md` runbook.
```

### 7.8 Deployment scenarios for Sigstore

| Scenario | Rekor URL | Identity check |
|---|---|---|
| Public OSS using Sigstore PGI | `https://rekor.sigstore.dev` | GitHub Actions OIDC |
| Enterprise self-hosted | `https://rekor.internal.corp/` | Internal OIDC (Okta/Auth0) |
| Air-gapped | Self-hosted on-prem | Internal CA-issued cert chain |

### 7.9 SLO M19-SIG-*

| ID | SLO | Cible |
|---|---|---|
| M19-SIG-1 | p50 Sigstore verification | < 200 ms (Rekor RPC + crypto) |
| M19-SIG-2 | p99 Sigstore verification | < 1 s |
| M19-SIG-3 | Rekor cache TTL | 600 s (immutable log entries) |
| M19-SIG-4 | Sigstore root TUF refresh | every 30 days |

---

## 8. Phase C (D53) — BYOK deep-dive

### 8.1 Pourquoi BYOK

M16-M18 utilisent `EFE_OCI_PUBLIC_KEY{,_FILE}` env-var pour la trust
store. Limites :
1. **Pas de rotation** : un nouveau key requiert redeploy + restart.
2. **Pas de gestion centralisée** : 50 tenants = 50 env-vars à synchroniser.
3. **Pas d'audit** : qui a lu la clé, quand ? Pas tracé.
4. **Pas de hardware-backed** : env-var en RAM = CVE-extractable.

KMS (Key Management Service) addresses these :
- Rotation native : CreateAlias → new key → rotate → delete old.
- Centralized : single ARN for all consumers.
- Audited : every GetPublicKey call logs to CloudTrail / Cloud Audit.
- Hardware-backed : FIPS 140-2 Level 3 HSM.

### 8.2 KeyStore abstraction

```python
# mcp_server/crypto/keystore.py
import abc
from dataclasses import dataclass


@dataclass(frozen=True)
class TrustedKey:
    key_id: str               # backend-specific identifier
    public_key_pem: bytes     # PEM-encoded public key
    expires_at_unix: int | None = None
    rotation_status: str = "active"   # active | pending_rotation | deprecated
    metadata: dict = None     # backend-specific (e.g. AWS ARN, GCP resource name)


class KeyStore(abc.ABC):
    """Abstract trust store for verifying bundle signatures."""
    
    @abc.abstractmethod
    async def list_keys(self) -> list[TrustedKey]:
        """Return all currently-trusted public keys."""
    
    @abc.abstractmethod
    async def get_key_by_id(self, key_id: str) -> TrustedKey | None:
        """Lookup a specific key by its backend ID."""
    
    @abc.abstractmethod
    async def health_check(self) -> dict:
        """Return health summary for `efe://crypto/keystore/status`."""
    
    @abc.abstractmethod
    async def aclose(self) -> None:
        """Cleanup connections / close clients."""


class KeyStoreFactory:
    """Build a KeyStore instance from `EFE_KEYSTORE_BACKEND` env."""
    
    @staticmethod
    def from_env() -> KeyStore:
        backend = os.environ.get("EFE_KEYSTORE_BACKEND", "env").lower()
        if backend == "env":
            from .env_keystore import EnvKeyStore
            return EnvKeyStore()
        if backend == "file":
            from .file_keystore import FileKeyStore
            return FileKeyStore()
        if backend == "aws-kms":
            from .aws_kms import AWSKMSKeyStore
            return AWSKMSKeyStore()
        if backend == "gcp-kms":
            from .gcp_kms import GCPKMSKeyStore
            return GCPKMSKeyStore()
        if backend == "azure-kv":
            from .azure_kv import AzureKVKeyStore
            return AzureKVKeyStore()
        if backend == "vault":
            from .vault import VaultKeyStore
            return VaultKeyStore()
        if backend == "pkcs11":
            from .pkcs11 import PKCS11KeyStore
            return PKCS11KeyStore()
        raise ValueError(f"unknown EFE_KEYSTORE_BACKEND: {backend!r}")
```

### 8.3 Backend implementations

#### 8.3.1 Env keystore (default, M18 carry-over)

```python
class EnvKeyStore(KeyStore):
    """Read PEM from EFE_OCI_PUBLIC_KEY{,_FILE} env-vars (M18 default)."""
    
    async def list_keys(self) -> list[TrustedKey]:
        pem = os.environ.get("EFE_OCI_PUBLIC_KEY")
        path = os.environ.get("EFE_OCI_PUBLIC_KEY_FILE")
        if pem:
            return [TrustedKey(
                key_id=f"env:{compute_key_id(pem.encode())}",
                public_key_pem=pem.encode("utf-8"),
            )]
        if path and os.path.exists(path):
            with open(path, "rb") as fh:
                content = fh.read()
            return [TrustedKey(
                key_id=f"file:{compute_key_id(content)}",
                public_key_pem=content,
            )]
        return []
    
    async def health_check(self) -> dict:
        keys = await self.list_keys()
        return {
            "backend": "env",
            "key_count": len(keys),
            "rotation_supported": False,
            "audit_logged": False,
        }
```

#### 8.3.2 AWS KMS

```python
class AWSKMSKeyStore(KeyStore):
    """AWS KMS backend.
    
    Config :
      EFE_KEYSTORE_BACKEND=aws-kms
      EFE_AWS_KMS_KEY_ARNS=arn:aws:kms:us-east-1:123:key/abc,arn:...
      AWS_REGION=us-east-1 (standard)
      AWS_PROFILE=...      (or env creds)
    
    Caches public keys with TTL (default 300 s) — KMS is rate-limited
    + costs $0.03 per 10k requests, so we don't pull on every verify.
    """
    
    def __init__(self):
        import boto3
        self._client = boto3.client("kms", region_name=os.environ.get("AWS_REGION"))
        self._arns = self._parse_arns(os.environ.get("EFE_AWS_KMS_KEY_ARNS", ""))
        self._cache_ttl = int(os.environ.get("EFE_KEYSTORE_CACHE_TTL_S", "300"))
        self._cache: dict[str, tuple[TrustedKey, float]] = {}
    
    async def list_keys(self) -> list[TrustedKey]:
        return [await self.get_key_by_id(arn) for arn in self._arns]
    
    async def get_key_by_id(self, key_id: str) -> TrustedKey | None:
        cached = self._cache.get(key_id)
        if cached and time.time() - cached[1] < self._cache_ttl:
            return cached[0]
        # Sync boto3 wrapped in asyncio.to_thread
        resp = await asyncio.to_thread(
            self._client.get_public_key, KeyId=key_id,
        )
        pem = _der_to_pem(resp["PublicKey"])
        meta = await asyncio.to_thread(
            self._client.describe_key, KeyId=key_id,
        )
        key = TrustedKey(
            key_id=f"aws-kms:{key_id}",
            public_key_pem=pem,
            rotation_status=("active" if meta["KeyMetadata"]["KeyState"] == "Enabled" else "deprecated"),
            metadata={"arn": key_id, "spec": resp.get("KeySpec")},
        )
        self._cache[key_id] = (key, time.time())
        return key
    
    async def health_check(self) -> dict:
        return {
            "backend": "aws-kms",
            "region": os.environ.get("AWS_REGION"),
            "key_count": len(self._arns),
            "cache_ttl_seconds": self._cache_ttl,
            "rotation_supported": True,
            "audit_logged": True,   # CloudTrail
        }
```

(Other backends in **Annexe B**.)

### 8.4 New tool : `efe_check_keystore_health`

```json
{
  "type": "object",
  "title": "EfeCheckKeystoreHealthInput",
  "properties": {},
  "additionalProperties": false
}
```

Output :
```json
{
  "backend": "aws-kms",
  "key_count": 3,
  "rotation_supported": true,
  "audit_logged": true,
  "keys": [
    {"key_id": "aws-kms:arn:aws:kms:...", "rotation_status": "active",
     "expires_at_unix": null}
  ],
  "cache_ttl_seconds": 300,
  "warnings": []
}
```

### 8.5 New resource : `efe://crypto/keystore/status`

Identical content to the `efe_check_keystore_health` tool, exposed
as a resource for @-mention in agent conversations.

### 8.6 Key rotation strategy

Two flavors :

**Pull-based (TTL)** : the cache TTL ensures stale keys are eventually
refreshed. Default 300 s = 5 min worst-case staleness.

**Push-based (signal)** : operator sends `SIGUSR1` to the mcp-server
process → forces immediate cache invalidation + re-pull. Useful for
emergency rotation.

```python
# mcp_server/crypto/rotation.py
import signal

def install_rotation_handler(keystore: KeyStore):
    def _handler(signum, frame):
        log.info("rotation_signal_received", backend=keystore.backend_name)
        asyncio.create_task(keystore.invalidate_cache())
    
    signal.signal(signal.SIGUSR1, _handler)
```

### 8.7 New write tool : `efe_rotate_acknowledged`

Audit-trail-only tool — operator records that they have acknowledged a
rotation event. Does NOT actually rotate keys (that's the operator's
KMS task). Just emits an audit event for compliance.

```json
{
  "type": "object",
  "title": "EfeRotateAcknowledgedInput",
  "properties": {
    "key_id": {"type": "string", "minLength": 1, "maxLength": 200},
    "old_key_status": {"type": "string", "enum": ["deprecated", "deleted"]},
    "new_key_id": {"type": "string", "minLength": 1, "maxLength": 200},
    "rotation_reason": {
      "type": "string",
      "pattern": "^[^\\r\\n]{10,500}$"
    },
    "confirm": {"type": "boolean"}
  },
  "required": ["key_id", "old_key_status", "rotation_reason", "confirm"]
}
```

### 8.8 SLO M19-KMS-*

| ID | SLO | Cible |
|---|---|---|
| M19-KMS-1 | p50 keystore lookup (cached) | < 5 ms |
| M19-KMS-2 | p99 keystore lookup (cold KMS) | < 500 ms |
| M19-KMS-3 | KMS request rate | ≤ 10 / min (via cache) |
| M19-KMS-4 | Cache TTL | 300 s default |
| M19-KMS-5 | Health check overhead | < 50 ms |

---

## 9. Phase D (D54) — HTTP+SSE + multi-replica deep-dive

### 9.1 Pourquoi HTTP+SSE

M16-M18 = stdio uniquement. Limites :
- Un client MCP = un mcp-server subprocess.
- Multi-utilisateur = N processes en parallèle.
- Pas de load balancing.
- Coûts mémoire élevés (chaque process pull son propre embedder etc).

HTTP+SSE résout :
- 1 mcp-server sert N clients via SSE (Server-Sent Events).
- Multiple replicas derrière LB possible.
- Stateless (avec Redis-backed caches).

### 9.2 MCP HTTP+SSE protocol

Per spec :
- `GET /sse` : client subscribes to server-to-client events.
- `POST /messages` : client sends JSON-RPC requests.

```python
# mcp_server/transport/http_sse.py
from fastapi import FastAPI, Request
from fastapi.responses import StreamingResponse, JSONResponse


def build_app(*, dispatcher, sessions: dict) -> FastAPI:
    app = FastAPI()
    
    @app.get("/sse")
    async def sse_endpoint(request: Request):
        session_id = _generate_session_id()
        queue = asyncio.Queue()
        sessions[session_id] = queue
        
        async def event_stream():
            yield f"event: endpoint\ndata: /messages?session_id={session_id}\n\n"
            try:
                while True:
                    message = await queue.get()
                    yield f"event: message\ndata: {json.dumps(message)}\n\n"
            except asyncio.CancelledError:
                sessions.pop(session_id, None)
        
        return StreamingResponse(event_stream(), media_type="text/event-stream")
    
    @app.post("/messages")
    async def messages_endpoint(request: Request):
        session_id = request.query_params.get("session_id")
        queue = sessions.get(session_id)
        if not queue:
            return JSONResponse({"error": "unknown session"}, status_code=404)
        
        body = await request.json()
        # Dispatch JSON-RPC, push response to queue
        response = await dispatch_jsonrpc(dispatcher, body)
        await queue.put(response)
        return JSONResponse({"ok": True})
    
    @app.get("/health")
    async def health():
        return {"status": "ok"}
    
    @app.get("/ready")
    async def ready():
        return {"status": "ready", "active_sessions": len(sessions)}
    
    @app.get("/metrics")
    async def metrics():
        from prometheus_client import generate_latest
        return Response(generate_latest(), media_type="text/plain")
    
    return app
```

### 9.3 Redis-backed state

For multi-replica, the in-memory caches (BundleCache, SamplingCache,
RateLimiter) need to be shared :

```python
# mcp_server/state/redis_backed.py
import redis.asyncio as aioredis


class RedisBundleCache(BundleCache):
    """Redis-backed BundleCache for multi-replica deployments."""
    
    def __init__(self, redis_url: str, *, key_prefix: str = "efe_mcp:bundle"):
        self._redis = aioredis.from_url(redis_url, decode_responses=False)
        self._prefix = key_prefix
        self.ttl_seconds = float(os.environ.get("EFE_MCP_BUNDLE_CACHE_TTL", "600"))
    
    async def get(self, *, tenant_id, slug, version, namespace="default"):
        key = f"{self._prefix}:{namespace}:{tenant_id}:{slug}:{version}"
        raw = await self._redis.get(key)
        if raw is None:
            return None
        return pickle.loads(raw)
    
    async def put(self, *, tenant_id, slug, version, value, namespace="default"):
        key = f"{self._prefix}:{namespace}:{tenant_id}:{slug}:{version}"
        await self._redis.set(key, pickle.dumps(value), ex=int(self.ttl_seconds))
```

Same pattern for `RedisRateLimiter` and `RedisSamplingCache`.

### 9.4 Graceful shutdown

```python
# mcp_server/transport/shutdown.py
import asyncio
import signal


class ShutdownCoordinator:
    def __init__(self, *, grace_seconds: float = 30.0):
        self._shutdown_event = asyncio.Event()
        self._inflight: set[asyncio.Task] = set()
        self.grace_seconds = grace_seconds
    
    def install_signal_handlers(self):
        loop = asyncio.get_running_loop()
        for sig in (signal.SIGTERM, signal.SIGINT):
            loop.add_signal_handler(sig, self._initiate_shutdown)
    
    def _initiate_shutdown(self):
        log.info("shutdown_signal_received")
        self._shutdown_event.set()
    
    async def track(self, coro):
        task = asyncio.create_task(coro)
        self._inflight.add(task)
        try:
            return await task
        finally:
            self._inflight.discard(task)
    
    async def wait_for_drain(self):
        try:
            await asyncio.wait_for(
                asyncio.gather(*self._inflight, return_exceptions=True),
                timeout=self.grace_seconds,
            )
        except asyncio.TimeoutError:
            log.warning("shutdown_drain_timeout", inflight=len(self._inflight))
```

### 9.5 New resource : `efe://server/health`

```json
{
  "uri": "efe://server/health",
  "version": "0.4.0",
  "transport": "http_sse",
  "uptime_seconds": 3600,
  "active_sessions": 42,
  "cache_stats": {
    "bundle": {"hits": 1234, "misses": 56, "hit_rate": 0.957},
    "sampling": {"hits": 12, "misses": 3, "hit_rate": 0.8}
  },
  "rate_limiter": {"buckets": 87, "total_consumed": 5432},
  "keystore": {"backend": "aws-kms", "key_count": 3},
  "otel": {"exporter_active": true, "spans_exported": 12345}
}
```

### 9.6 SLO M19-HTTP-*

| ID | SLO | Cible |
|---|---|---|
| M19-HTTP-1 | p50 SSE setup latency | < 50 ms |
| M19-HTTP-2 | p99 SSE setup latency | < 200 ms |
| M19-HTTP-3 | Concurrent sessions / replica | ≥ 1000 |
| M19-HTTP-4 | Graceful shutdown grace | 30 s default |
| M19-HTTP-5 | `/metrics` scrape latency | < 50 ms |
| M19-HTTP-6 | Multi-replica cache hit rate | ≥ 80 % (vs 95 % single replica) |

---

## 10. New tools + resources + prompts

### 10.1 Tools (3 new)

| Tool | Phase | Profile | Write | Confirm |
|---|:---:|---|:---:|:---:|
| `efe_verify_rekor_inclusion` | D52 | agent-native | — | — |
| `efe_check_keystore_health` | D53 | advanced | — | — |
| `efe_rotate_acknowledged` | D53 | agent-native-deploy | ✅ | ✅ |

### 10.2 Resources (3 new)

| URI | Phase |
|---|:---:|
| `efe://bundles/{slug}/{version}/transparency` | D52 |
| `efe://crypto/keystore/status` | D53 |
| `efe://server/health` | D54 |

### 10.3 Prompts (1 new)

| Name | Args | Phase |
|---|---|:---:|
| `incident_supply_chain_audit` | `since` (req), `tenant?` | D52 |

---

## 11. JSON Schemas (forward-refs)

Conventions M19 :
- `additionalProperties: false` (TM-M16-T-02 carry-over)
- Patterns stricts (TM-M18-D-49-A5 carry-over)
- Confirm-required pour tous les write tools (M16 carry-over)
- Schemas complets en **Annexe A**

---

## 12. Backend extensions (EFE 1.3 contract bump)

### 12.1 New REST endpoints (additive)

| Method | Path | Phase |
|---|---|:---:|
| GET | `/admin/transparency/{log_index}` | D52 |
| GET | `/admin/keystore/status` | D53 |
| POST | `/admin/keystore/rotation-event` | D53 |

### 12.2 New gRPC RPC

```proto
service EfeAdminService {
  rpc VerifyRekorInclusion(VerifyRekorInclusionRequest) returns (VerifyRekorInclusionResponse);
}
```

### 12.3 Backward compat EFE 1.2

mcp-server M19 detects EFE 1.2 backend → falls back gracefully :
- Sigstore : direct Rekor pull (bypass backend) when `/admin/transparency/...` absent.
- Keystore status : returns local view only.
- Rotation events : log to local audit only.

---

## 13. Threat model M19 (delta vs M18)

9 nouvelles menaces M19 sur les 28 cumulées M16-M18 = **37 cumulées**.

### 13.1 Vue d'ensemble (table récapitulative)

| ID | STRIDE | Threat | Probabilité | Sévérité | Mitigation livrée M19 |
|---|:---:|---|:---:|:---:|---|
| **TM-M19-S-04** | Spoof | KMS key compromise (insider) | LOW | HIGH | KMS audit trail (CloudTrail) + multi-key trust + Sigstore Rekor cross-witness |
| **TM-M19-S-05** | Spoof | Rekor entry forgery | LOW | HIGH | Sigstore TUF root + multi-witness verification + checkpoint signature |
| **TM-M19-T-02** | Tamper | OTel trace data injection (malicious spans) | MED | LOW | Trace propagation only from JWT-validated requests ; spans from untrusted clients dropped |
| **TM-M19-T-03** | Tamper | Sigstore Bundle v0.3 tampering (cert chain manipulation) | LOW | HIGH | Full schema validation + Fulcio cert chain verification |
| **TM-M19-R-02** | Repud. | Logs not correlated to requests | HIGH | LOW | trace_id in every log line via `structlog.contextvars` |
| **TM-M19-I-02** | InfoDisc | Metrics labels expose tenant data | MED | MED | Cardinality limits + `tenant_hash = sha256(tenant_id)[:8]` |
| **TM-M19-D-03** | DoS | Prometheus scrape DoS / leak via `/metrics` | MED | LOW | Optional auth on `/metrics` (via reverse proxy) + rate limit /metrics |
| **TM-M19-E-04** | Elev. | Multi-replica race for write (bundle promote) | MED | HIGH | Distributed lock via `redis SET NX EX` + idempotency_key |
| **TM-M19-E-05** | Elev. | KMS impersonation (stolen creds) | LOW | HIGH | mTLS to KMS endpoints + audience claim verification + CloudTrail anomaly detection |

### 13.2 Dossier détaillé — TM-M19-S-04 (KMS key compromise)

**Catégorie STRIDE** : Spoofing — un attaquant détient temporairement un
accès au KMS et substitue la clé publique de vérification.

**Vecteur d'attaque** :
1. Insider abuse (admin AWS au privilège `kms:UpdateKeyDescription`).
2. Compromise du compte AWS (creds volées + MFA bypass).
3. Account takeover via federation IdP attaqué.

**Blast radius** : si la clé KMS est compromise, **tous** les bundles
vérifiés via cette clé deviennent suspects. Tenant peut accidentellement
charger un bundle malicieux.

**Pré-conditions** : attaquant a déjà accès aux comptes cloud admin.
Suppose la défense en profondeur de l'opérateur a échoué (autre couche).

**Contrôles M19** :
- **C1** : Multi-key trust dans `EFE_AWS_KMS_KEY_ARNS` (pluriel) — un
  bundle est accepté si **AU MOINS UNE** clé valide la signature.
  Compromettre une seule clé ne suffit pas.
- **C2** : Sigstore Rekor cross-witness — la même signature est aussi
  recherchée dans Rekor transparency log. Une signature insérée
  silencieusement via clé KMS volée n'apparaît PAS dans Rekor (sauf si
  attaquant a aussi forgé une entrée Rekor → cf. TM-M19-S-05).
- **C3** : KMS audit trail via CloudTrail — chaque `GetPublicKey` est
  loggé. Anomaly detection sur le rate / source IP.
- **C4** : Periodic key fingerprint reconciliation —
  `efe_check_keystore_health` retourne le fingerprint hex de chaque
  clé ; un opérateur peut comparer aux valeurs attendues stockées
  hors-bande (dans son `runbooks/keys_inventory.md`).

**Residual risk** : un attaquant qui compromet **simultanément** AWS
admin + Sigstore Rekor + le runbook out-of-band reste théoriquement
possible mais hors scope (≥ 4 dimensions de compromise → CC9.1 risk
acceptance).

**Evidence** : SOC 2 CC6.1 (logical access) + ISO 27001 A.10.1.2.

**Pin test ref** : `test_keystore_health_aws.py::test_keystore_health_warning_on_no_keys`
+ `test_audit_pass_m19.py::test_kms_compromise_multi_key_trust`.

### 13.3 Dossier détaillé — TM-M19-S-05 (Rekor entry forgery)

**Vecteur d'attaque** : attaquant insère une entrée dans Rekor pour un
artefact malicieux, signé via une clé qu'il contrôle (ex. : Fulcio
keyless avec un OIDC token compromis chez un IdP affilié).

**Pré-conditions** : compromission OIDC IdP (Google / GitHub) **OU**
private Rekor self-hosté avec accès admin compromis.

**Contrôles M19** :
- **C1** : Sigstore TUF root pinning — le mcp-server vérifie que la
  Rekor public key suit la TUF root distribuée par
  `tuf-repo-cdn.sigstore.dev`. Un Rekor compromis qui change sa
  clé serait rejeté.
- **C2** : OIDC identity binding — la mitigation D52 exige que
  `cert_identity` matche un `identity_pattern` configuré
  (`EFE_SIGSTORE_TRUSTED_IDENTITIES`). Un OIDC token forgé pour
  `attacker@example.com` est rejeté car il ne match pas
  `^.*@efe-project\.org$`.
- **C3** : Multi-witness verification (optional, future M20) —
  vérification redondante via plusieurs Rekor instances (PGI + private
  + sigstore-monitor).
- **C4** : Checkpoint signature monitoring — Rekor publie des
  checkpoints signés ; un opérateur peut périodiquement vérifier la
  consistance.

**Residual risk** : compromission simultanée OIDC IdP **et** TUF root.
Probabilité estimée < 10⁻⁶ par an.

**Evidence** : SLSA Level 3 attestation chain.

**Pin test ref** : `test_sigstore_verify.py::test_verify_rejects_unknown_identity`.

### 13.4 Dossier détaillé — TM-M19-T-02 (OTel trace injection)

**Vecteur d'attaque** : un client MCP malveillant injecte un header
W3C `traceparent` arbitraire pour pollue le span tree.

**Pré-conditions** : connectivité réseau au mcp-server HTTP+SSE.

**Contrôles M19** :
- **C1** : Trace propagation **strictement gated** par auth — le
  `traceparent` n'est extrait que si la requête HTTP est authentifiée
  (JWT valide). Requêtes anonymes sur `/health` ne propagent pas de
  trace amont (un nouveau trace_id est généré côté server).
- **C2** : Sampling rate cap — `EFE_OTEL_TRACES_SAMPLER_ARG=0.1`
  (10 % par défaut), un attaquant qui spam ne pollue pas le backend
  de manière significative.
- **C3** : Span attribute size limit — chaque attribut est tronqué à
  256 chars (carry-over OTel default).

**Residual risk** : un client authentifié qui pollue son propre tenant
est OK (tenant-scoped). Le risque est d'observabilité dégradée pour
*lui-même*, pas exfiltration.

**Pin test ref** : `test_observability.py::test_traceparent_extracted_only_when_authed`.

### 13.5 Dossier détaillé — TM-M19-T-03 (Sigstore bundle tampering)

**Vecteur d'attaque** : MITM modifie le `.sigstore` body en pull OCI
(supposant rupture de la chaîne TLS).

**Contrôles M19** :
- **C1** : OCI registry TLS — déjà couvert M13 (`EFE_OCI_INSECURE` interdit).
- **C2** : Sigstore Bundle schema validation strict — protobuf
  unmarshall puis Pydantic validation. Champs inattendus → reject.
- **C3** : Fulcio cert chain verification jusqu'à la TUF root pinned.
- **C4** : Signed payload digest (`messageDigest`) doit matcher le
  manifest digest annoncé par OCI. Une modification du body sans
  modifier le digest échoue.

**Pin test ref** : `test_sigstore_verify.py::test_verify_rejects_tampered_bundle`.

### 13.6 Dossier détaillé — TM-M19-R-02 (Logs sans corrélation)

**Vecteur d'attaque** : non-malveillant. Un incident en prod, l'opérateur
ne peut PAS rapidement corréler les logs aux requêtes spécifiques car
les log lines n'ont pas de `trace_id`.

**Impact** : MTTR élevé. SOC 2 CC7.2 (anomaly detection) — sans
corrélation, l'auditeur conclut absence de monitoring effectif.

**Contrôles M19** :
- **C1** : `structlog.contextvars` injecte le `trace_id` extrait du
  span actif dans **chaque** log line émise dans le contexte.
- **C2** : Le `tool_name`, `tenant_hash`, `is_write` sont aussi
  injectés systématiquement.
- **C3** : Format JSON par défaut en prod
  (`EFE_LOG_FORMAT=json`) → ingéré directement par Loki / Splunk /
  Cloudwatch sans parsing.

**Pin test ref** : `test_observability.py::test_log_line_carries_trace_id`.

### 13.7 Dossier détaillé — TM-M19-I-02 (Metrics tenant leak)

**Vecteur d'attaque** : labels Prometheus contiennent `tenant_id` brut
(ex. : `acme-corp`, `evil-corp`) → quiconque peut scraper `/metrics`
peut énumérer les tenants.

**Contrôles M19** :
- **C1** : `tenant_hash = sha256(tenant_id)[:8]` est utilisé comme
  label, **PAS** le `tenant_id` brut.
- **C2** : Cardinality limit — au-delà de 10 K time series par
  replica, l'enregistrement de nouveaux labels est suppressed et
  un compteur `efe_mcp_metrics_dropped_total` incrémente.
- **C3** : `/metrics` endpoint est optionnellement gated par auth
  via `EFE_METRICS_AUTH_TOKEN`.

**Residual risk** : un attaquant qui collectionne longtemps
`tenant_hash` peut tenter rainbow tables sur tenant_id court. Mitigation
future : sel par tenant via `EFE_TENANT_HASH_SALT`.

**Pin test ref** : `test_observability.py::test_metrics_use_tenant_hash`.

### 13.8 Dossier détaillé — TM-M19-D-03 (Prometheus scrape DoS)

**Vecteur d'attaque** : attaquant scrape `/metrics` à 100 Hz → CPU
overhead significatif (chaque scrape sérialise toutes les métriques).

**Contrôles M19** :
- **C1** : Rate limit dédié via reverse proxy (nginx
  `limit_req_zone`) — recommandé dans le runbook.
- **C2** : Un `RateLimiter` côté server pour `/metrics` route :
  10 req/min/IP par défaut.
- **C3** : Optional auth via `EFE_METRICS_AUTH_TOKEN`.

**Residual risk** : un Prometheus interne légitime scrape à fréquence
élevée → opérateur whitelist son IP.

**Pin test ref** : `test_http_transport.py::test_metrics_endpoint_rate_limited`.

### 13.9 Dossier détaillé — TM-M19-E-04 (Multi-replica write race)

**Vecteur d'attaque** : deux replicas reçoivent simultanément la même
demande de promote-bundle (LB sticky failure). Sans coordination,
deux audit events `bundle_promoted` sont émis ; le backend, idempotent
sur `idempotency_key`, n'effectue qu'une mutation, mais les audit
chains divergent.

**Contrôles M19** :
- **C1** : Distributed lock Redis :
  ```python
  lock_key = f"efe:lock:promote:{slug}:{version}"
  acquired = await redis.set(
      lock_key, replica_id, nx=True, ex=30,
  )
  if not acquired:
      raise ToolError("concurrent_write_in_progress", code="busy")
  try:
      result = await backend.promote_bundle(...)
  finally:
      await redis.delete(lock_key)
  ```
- **C2** : Idempotency-Key (M18-FINAL-A4 carry-over) — même si le
  lock échoue, le backend dédupe.
- **C3** : Audit chain Redis-coordonné — `audit_emitter` use Redis
  `INCR` pour le seq global (sous M19 multi-replica) au lieu de
  l'in-process counter.

**Pin test ref** : `test_redis_backed.py::test_distributed_lock_serializes_promote`.

### 13.10 Dossier détaillé — TM-M19-E-05 (KMS impersonation)

**Vecteur d'attaque** : creds KMS (IRSA token, AWS access key) volés
→ attaquant appelle `kms:GetPublicKey` mais aussi `kms:Sign`
illégitimement.

**Contrôles M19** :
- **C1** : IAM policy least-privilege — M19 documentation impose
  `kms:GetPublicKey` + `kms:DescribeKey` UNIQUEMENT (pas
  `kms:Sign`). Si compromise, l'attaquant ne peut PAS signer,
  juste lire des clés publiques (déjà publiques par définition).
- **C2** : mTLS / VPC endpoint pour KMS calls — empêche extraction
  de creds si réseau compromis (cred attaque doit être on-host).
- **C3** : Audience claim verification (IRSA) — token IRSA carry
  audience `sts.amazonaws.com` ; un attaquant qui réplay sur
  d'autres services AWS échoue.
- **C4** : CloudTrail anomaly detection — alerte sur appels venant
  de source IP inhabituelle.

**Residual risk** : compromise simultanée IRSA + VPC ingress très
peu probable. Acceptable.

**Pin test ref** : `test_keystore_aws.py::test_aws_kms_uses_audience_sts_amazonaws`.

### 13.11 Threat carry-over

Les 28 menaces TM-M16-* / TM-M17-* / TM-M18-* restent valides et
sont systématiquement ré-vérifiées par les pin tests M19 audit pass.
La table cumulée (37 menaces) est versionnée dans
`docs/security/threat_model_M19.md` et attestée trimestriellement
par le security officer.

### 13.12 Pen-test couverture

Threat model M19 intégré au scope du pen-test annuel 2026 (Q3 2026
provisionnellement). Targets explicites :
- Tentative de forge Rekor entry sur self-hosted instance staging.
- KMS creds rotation pendant un `verify_bundle` round-trip.
- Multi-replica write race via load test 50 RPS sur `promote_bundle`.
- Prometheus scrape flood avec et sans auth.

---

## 14. Performance & SLOs M19

### 14.1 SLOs M19-* (récap court)

(Détails complets en sections 6.6 / 7.9 / 8.8 / 9.6 et tableau dense ci-dessous.)

| ID | SLO | Cible |
|---|---|---|
| M19-OBS-1 | Trace export overhead | < 5 % p99 latency |
| M19-OBS-3 | Log volume | ≤ 100 KB / minute / replica |
| M19-OBS-6 | Cardinality | ≤ 10 K series total |
| M19-SIG-1 | p50 Sigstore verification | < 200 ms |
| M19-SIG-2 | p99 Sigstore verification | < 1 s |
| M19-KMS-1 | p50 keystore lookup (cached) | < 5 ms |
| M19-KMS-3 | KMS request rate | ≤ 10 / min |
| M19-HTTP-3 | Concurrent sessions / replica | ≥ 1000 |
| M19-HTTP-6 | Multi-replica cache hit rate | ≥ 80 % |
| M19-PIN | M19 pin tests | +70 GREEN |

### 14.2 SLO definitions (formula + error budget + alert)

Chaque SLO suit le format SRE classique : SLI (Service-Level Indicator,
mesure brute), SLO (cible), error budget (1 - SLO % de la fenêtre),
alert burn rate. Convention : fenêtre 30 jours rolling, cible 99 % par
défaut sauf indiqué.

#### M19-OBS-1 — Trace export overhead

- **SLI** : `(p99(latency_with_otel) - p99(latency_baseline)) / p99(latency_baseline)`
  mesuré côté serveur via histogramme `efe_mcp_tool_duration_seconds`.
- **Cible** : ratio < 0.05 (5 %).
- **Error budget** : 1 % du temps cumulé hors-cible / 30 jours.
- **Alert** : burn rate × 14.4 over 1h (5-min eval) → page on-call
  (cible Google SRE Workbook ch. 5).
- **Runbook** : `m19_otel_overhead_excess.md`.
- **Notes** : si non-respecté, augmenter `EFE_OTEL_TRACES_SAMPLER_ARG`
  de 0.1 → 0.05 OU utiliser BatchSpanProcessor au lieu de SimpleSpanProcessor.

#### M19-OBS-2 — Trace export availability (collector reachable)

- **SLI** : `(spans_exported_ok / (spans_exported_ok + spans_exported_failed))`
  mesuré via OTel SDK metrics export internal.
- **Cible** : 99.5 %.
- **Error budget** : 3.6 h / 30 jours.
- **Alert** : page si burn rate × 6 over 1h.
- **Runbook** : `m19_otel_exporter_unreachable.md`.

#### M19-OBS-3 — Log volume

- **SLI** : `kbytes_per_minute_per_replica = sum(log_size) / 60`
  via fluent-bit / promtail metric.
- **Cible** : ≤ 100 KB/min/replica.
- **Error budget** : 5 % du temps > cible / 30 jours.
- **Alert** : warning si > 200 KB/min sustained 10 min.
- **Runbook** : `m19_log_volume_explosion.md`.

#### M19-OBS-4 — Log latency to Loki

- **SLI** : delay between log line emission and ingestion in central
  log store (Loki / Cloudwatch).
- **Cible** : p99 < 30 s.
- **Error budget** : 1 % time > cible / 30 jours.
- **Alert** : burn rate × 6 over 1h.

#### M19-OBS-5 — Metric collection success

- **SLI** : `prometheus_scrape_success_ratio` via `up{job="efe-mcp"}`.
- **Cible** : 99.9 %.
- **Error budget** : 43 min / 30 jours.

#### M19-OBS-6 — Metric cardinality

- **SLI** : count of distinct time series for `efe_mcp_*` metrics.
- **Cible** : ≤ 10 000 series **par replica**.
- **Error budget** : aucune. Hard limit. Au-delà → drop avec
  `efe_mcp_metrics_dropped_total` increment.
- **Alert** : page si série > 8 000.
- **Runbook** : `m19_metrics_cardinality_explosion.md`.

#### M19-SIG-1 — p50 Sigstore verification latency

- **SLI** : `histogram_quantile(0.5, efe_mcp_bundle_verify_duration_seconds{method="sigstore"})`.
- **Cible** : p50 < 200 ms.
- **Error budget** : 1 % time > cible / 30 jours.
- **Alert** : warning si p50 > 300 ms 5 min.

#### M19-SIG-2 — p99 Sigstore verification latency

- **SLI** : idem M19-SIG-1 quantile 0.99.
- **Cible** : p99 < 1 s.
- **Error budget** : 1 % time > cible.
- **Alert** : page si p99 > 2 s 5 min.

#### M19-SIG-3 — Rekor inclusion proof success rate

- **SLI** : `(rekor_proof_ok / rekor_proof_total)`.
- **Cible** : 99 % (allows for transient Rekor PGI hiccups).
- **Error budget** : 7.2 h / 30 jours.
- **Alert** : burn × 6 over 1h.
- **Runbook** : `m19_sigstore_rekor_outage.md`.

#### M19-SIG-4 — TUF root freshness

- **SLI** : `now - tuf_root_last_refresh_timestamp_seconds`.
- **Cible** : < 24 h.
- **Error budget** : 1 % time stale.
- **Alert** : page si > 48 h (TUF compromise window).

#### M19-KMS-1 — p50 keystore lookup (cached)

- **SLI** : `histogram_quantile(0.5, efe_mcp_keystore_lookup_duration_seconds{cache_hit="true"})`.
- **Cible** : p50 < 5 ms.
- **Error budget** : 1 % time > 10 ms.

#### M19-KMS-2 — p50 keystore lookup (uncached, KMS roundtrip)

- **SLI** : `histogram_quantile(0.5, efe_mcp_keystore_lookup_duration_seconds{cache_hit="false"})`.
- **Cible** : p50 < 100 ms (cache miss = single KMS roundtrip).
- **Error budget** : 5 % time > 250 ms.

#### M19-KMS-3 — KMS request rate

- **SLI** : `rate(efe_mcp_keystore_kms_requests_total[1m])`.
- **Cible** : ≤ 10 req/min/replica par défaut (vary KMS quotas).
- **Error budget** : 1 % time > cible (transient cache evictions).
- **Alert** : page si > 50/min sustained 10 min (likely TTL bug or
  cache thrashing).
- **Runbook** : `m19_kms_throttle.md`.

#### M19-KMS-4 — Keystore health check

- **SLI** : `efe_check_keystore_health` returns "OK" via probe.
- **Cible** : 99.9 % minutes healthy / 30 jours.
- **Alert** : page si 3 minutes consécutives "DEGRADED".

#### M19-HTTP-1 — HTTP request error rate

- **SLI** : `(http_5xx + http_4xx_excl_auth) / http_total`.
- **Cible** : 99 % success.
- **Error budget** : 7.2 h / 30 jours.

#### M19-HTTP-2 — SSE session establishment latency

- **SLI** : duration `GET /sse` response start.
- **Cible** : p99 < 500 ms.
- **Error budget** : 1 % time > cible.

#### M19-HTTP-3 — Concurrent sessions per replica

- **SLI** : `efe_mcp_active_sessions` gauge max over 1m.
- **Cible** : ≥ 1000 supportées sans dégradation.
- **Capacity test** : load test trimestriel 1 200 sessions
  simultanées + p99 latency < 2 s mesurée.
- **Runbook** : `m19_session_capacity_planning.md`.

#### M19-HTTP-4 — Graceful shutdown drain time

- **SLI** : duration de SIGTERM → all sessions closed.
- **Cible** : p99 < 30 s.
- **Error budget** : 5 % time > cible.
- **Runbook** : `m19_graceful_shutdown.md`.

#### M19-HTTP-5 — Redis availability

- **SLI** : `redis_ping_ok / redis_ping_total`.
- **Cible** : 99.9 %.
- **Error budget** : 43 min / 30 jours.
- **Runbook** : `m19_redis_partition.md`.

#### M19-HTTP-6 — Multi-replica cache hit rate

- **SLI** : `(cache_hits / cache_hits + cache_misses)` for shared
  cache keys (BundleCache + SamplingCache).
- **Cible** : ≥ 80 %.
- **Error budget** : 5 % time < cible.

#### M19-HTTP-7 — Inflight RPS per replica

- **SLI** : `rate(efe_mcp_tool_calls_total[1m])` per replica gauge.
- **Cible** : 10–50 RPS sustained / replica (capacity
  planning lower bound).
- **Action** : si > 50 RPS sustained 10 min → ajouter replica.

#### M19-PIN — Pin tests

- **SLI** : `make test` exit code 0 with N tests collected.
- **Cible** : ≥ 522 tests GREEN (vs 452 M18).
- **CI gate** : block merge si < 522.

### 14.3 Runbooks M19 (dans `docs/runbooks/`)

| Runbook file | Trigger SLO / event | Owner |
|---|---|---|
| `m19_otel_exporter_unreachable.md` | M19-OBS-2 alert | SRE on-call |
| `m19_otel_overhead_excess.md` | M19-OBS-1 alert | SRE on-call |
| `m19_log_volume_explosion.md` | M19-OBS-3 sustained breach | SRE on-call |
| `m19_metrics_cardinality_explosion.md` | M19-OBS-6 alert | SRE on-call |
| `m19_sigstore_rekor_outage.md` | M19-SIG-3 alert | SRE on-call |
| `m19_tuf_root_stale.md` | M19-SIG-4 alert | Security officer |
| `m19_kms_throttle.md` | M19-KMS-3 alert | SRE on-call + cloud admin |
| `m19_keystore_rotation.md` | scheduled key rotation event | Security officer |
| `m19_redis_partition.md` | M19-HTTP-5 alert | SRE on-call |
| `m19_graceful_shutdown.md` | deploy-time stall | SRE on-call |
| `m19_session_capacity_planning.md` | M19-HTTP-3 capacity warning | SRE lead |
| `m19_kms_compromise_response.md` | suspected key compromise | Security officer + IR |

Chaque runbook suit le template `runbook_template.md` :
1. Symptômes observés
2. Diagnostic (commandes spécifiques)
3. Mitigation immédiate (5–15 min)
4. Cause racine (post-incident)
5. Permanent fix
6. Communication interne / client

### 14.4 SLO error-budget governance

- **Burn rate alerts** : multi-window multi-burn-rate (Google SRE
  Workbook §5) :
  - Page : burn rate × 14.4 over 1h **AND** × 6 over 5min.
  - Ticket : burn rate × 6 over 6h **AND** × 1 over 30min.
- **Quarterly review** : product owner + SRE lead review error
  budget consumption ; if budget exhausted, freeze feature work
  until reliability work catches up (Google SRE convention).
- **Stakeholder reporting** : Q1 / Q2 / Q3 / Q4 SLO compliance
  report inclus dans `docs/reports/m19_slo_quarterly_*.md`.

---

## 15. Compatibility matrix

### 15.1 KMS providers

| Provider | Status | M19 phase | Auth method |
|---|:---:|:---:|---|
| AWS KMS | ✅ | D53 | IRSA / IAM role / static creds |
| GCP Cloud KMS | ✅ | D53 | Service account / Workload Identity |
| Azure Key Vault | ✅ | D53 | Managed Identity / SP creds |
| HashiCorp Vault | ✅ | D53 | Token / approle |
| PKCS#11 HSM | ✅ | D53 | PIN / token |
| File (PEM dir) | ✅ | D53 | filesystem permissions |
| Env vars | ✅ | D53 (carry-over M18) | env |

### 15.2 OTel exporters

| Exporter | Status |
|---|:---:|
| OTLP HTTP/protobuf | ✅ default |
| OTLP gRPC | ✅ |
| Jaeger (legacy) | ⚠ via collector |
| Zipkin (legacy) | ⚠ via collector |

### 15.3 Sigstore versions

| Component | Min version | Notes |
|---|---|---|
| Sigstore Bundle | v0.2 | minimum supported |
| Sigstore Bundle | v0.3 | preferred |
| Rekor v2 API | required | |
| TUF spec | v1.0 | |
| sigstore-python | ≥ 3.0 | |

---

## 16. Migration M18 → M19

### 16.1 Opt-in checklist

```bash
# Observability
EFE_OTEL_EXPORTER=https://otel-collector.example.com:4318
EFE_LOG_FORMAT=json
EFE_LOG_LEVEL=INFO

# Sigstore (optional)
pip install efe-mcp-server[sigstore]
EFE_REKOR_URL=  # empty = Sigstore PGI ; or self-host URL
EFE_VERIFICATION_METHOD=auto  # ed25519 (M18 default) | sigstore | auto

# KMS (optional)
pip install efe-mcp-server[aws-kms]
EFE_KEYSTORE_BACKEND=aws-kms
EFE_AWS_KMS_KEY_ARNS=arn:aws:kms:...

# HTTP+SSE (optional)
EFE_MCP_TRANSPORT=http
EFE_HTTP_BIND=0.0.0.0:8800
EFE_REDIS_URL=redis://redis.example.com:6379/0
```

### 16.2 Backward compat

- All M16-M18 functionality preserved bit-for-bit when M19 features
  not enabled.
- 452 existing tests still GREEN post-M19.
- `EFE_MCP_TOOL_PROFILE` works identically.
- Default install does not pull M19 extras.

### 16.3 Rollback

```bash
EFE_OTEL_EXPORTER=  # disable OTel
EFE_VERIFICATION_METHOD=ed25519  # M18 behaviour
EFE_KEYSTORE_BACKEND=env  # M18 behaviour
EFE_MCP_TRANSPORT=stdio  # M18 behaviour
```

---

## 17. Test plan

### 17.1 Pin tests inventory (~70 nouveaux)

| File | Tests | Phase |
|---|---|---|
| `test_observability.py` | 12 | D51 |
| `test_sigstore_verify.py` | 12 | D52 |
| `test_rekor_inclusion.py` | 8 | D52 |
| `test_keystore_factory.py` | 10 | D53 |
| `test_keystore_aws.py` (cond.) | 6 | D53 |
| `test_keystore_file.py` | 6 | D53 |
| `test_keystore_health_tool.py` | 6 | D53 |
| `test_http_transport.py` | 10 | D54 |
| `test_redis_backed.py` | 6 | D54 |
| `test_audit_pass_m19.py` (post-livraison) | 18 (estim.) | D54 |

Total : **~94 nouveaux pin tests**. Ajustement post-livraison +
audit.

### 17.2 Conditional tests

`test_keystore_aws.py` : skipped si `boto3` non-installé OU
`AWS_PROFILE` non-set. Run en CI dedicated avec test AWS account.

`test_sigstore_verify.py` : utilise des Sigstore Bundle vectors
generated offline + Rekor mock via `httpx.MockTransport`.

### 17.3 Integration tests

- `test_e2e_otel_collector.py` : run mcp-server + local OTel collector
  + verify spans appear.
- `test_e2e_redis.py` : run mcp-server + local Redis + verify cache
  shared across two replica instances.

### 17.4 Audit pass M19

Pattern récurrent M11-M18 (~6-8 défauts attendus). Phase D54.
Targets attendus : 1 HIGH (probablement KMS ou Sigstore wiring),
2-4 MED, 2-4 LOW.

---

## 18. Deployment scenarios

### 18.1 Local dev

```bash
EFE_OIDC_TOKEN=... \
EFE_ENDPOINT=http://localhost:8765 \
EFE_MCP_TOOL_PROFILE=agent-native \
EFE_OCI_REGISTRY_ALLOWLIST=ghcr.io/efe-project \
efe-mcp-server  # stdio, no OTel, no KMS, no Sigstore
```

### 18.2 Single-replica cloud + observability

```bash
EFE_OIDC_TOKEN=... \
EFE_ENDPOINT=https://efe.example.com \
EFE_MCP_TOOL_PROFILE=agent-native-deploy \
EFE_OCI_REGISTRY_ALLOWLIST=ghcr.io/efe-project \
EFE_OTEL_EXPORTER=https://collector.example.com:4318 \
EFE_LOG_FORMAT=json \
EFE_DEPLOYMENT_ENV=production \
efe-mcp-server  # stdio, full OTel, env keystore
```

### 18.3 Multi-region multi-replica

```bash
EFE_MCP_TRANSPORT=http \
EFE_HTTP_BIND=0.0.0.0:8800 \
EFE_REDIS_URL=redis://redis-cluster.example.com:6379/0 \
EFE_KEYSTORE_BACKEND=aws-kms \
EFE_AWS_KMS_KEY_ARNS=arn:aws:kms:us-east-1:... \
EFE_OTEL_EXPORTER=https://otlp.honeycomb.io:443 \
EFE_VERIFICATION_METHOD=auto \
EFE_REKOR_URL=https://rekor.sigstore.dev \
efe-mcp-server  # full M19 stack
```

### 18.4 Air-gapped enterprise

```bash
EFE_KEYSTORE_BACKEND=pkcs11 \
EFE_PKCS11_LIB=/usr/lib/softhsm/libsofthsm2.so \
EFE_PKCS11_TOKEN_LABEL=efe-prod \
EFE_PKCS11_PIN_FILE=/etc/efe/pkcs11.pin \
EFE_REKOR_URL=https://rekor.internal.corp \
EFE_OTEL_EXPORTER=https://otel-internal.corp:4318 \
EFE_VERIFICATION_METHOD=sigstore \
efe-mcp-server  # PKCS#11 HSM + on-prem Rekor + on-prem OTel
```

---

## 19. Roadmap M20+

### 19.1 M20 — Agent SDK + bring-your-own-LLM helpers (Q1 2027)

- Helper library for embedding mcp-server in custom agent loops.
- Sampling provider abstraction (Anthropic, OpenAI, local Llama, etc.).

### 19.2 M21 — GitOps integration (Q2 2027, conditional)

- Read-only ArgoCD/FluxCD integration (status only).
- Toujours pas de mutation cluster depuis MCP.

### 19.3 M22 — Multi-cluster federation (Q3 2027)

- Cross-region failover.
- Tenant home-region routing.

---

## 20. Open questions

1. **OTel collector vs direct OTLP push** — should mcp-server push
   directly to OTLP endpoint, or always through a collector ? **Décision** :
   support both, default direct ; collector recommended for
   high-volume.
2. **Sigstore Bundle v0.2 vs v0.3** — support both ? **Décision** :
   v0.3 is the preferred ; v0.2 is read-compat. Reject v0.1 as legacy.
3. **KMS health check frequency** — periodic background check or
   on-demand only ? **Décision** : on-demand via `efe_check_keystore_health`
   tool ; background runs only on metric/alert config.
4. **Redis cluster vs single instance** — recommended config ?
   **Décision** : Redis Cluster for multi-replica > 5 replicas ;
   single Redis OK for ≤ 5.
5. **Trace sampling default** — 100 % in prod is expensive. **Décision** :
   default 10 % in prod (`EFE_OTEL_SAMPLING_RATE=0.1`) ; 100 % in dev.

---

## 21. FAQ

**Q: Is OTel mandatory in M19?**
A: No. `EFE_OTEL_EXPORTER` empty → OTel SDK is no-op.

**Q: Does Sigstore introduce a new sub-processor?**
A: Sigstore PGI (Public Good Instance) is opt-in. Self-host Rekor on
prem to keep zero new sub-processor.

**Q: Can I run M19 without Redis?**
A: Yes, single-replica works without Redis (in-memory caches). Redis
required only for multi-replica.

**Q: How do I rotate KMS keys?**
A: KMS-side ops (CreateAlias → new key → delete old). MCP server
detects via TTL refresh OR `SIGUSR1` for immediate.

**Q: Can I migrate from `EFE_OCI_PUBLIC_KEY` to AWS KMS without downtime?**
A: Yes. Set `EFE_KEYSTORE_BACKEND=aws-kms` ; both env-key and KMS keys
are trusted simultaneously during transition (multi-key trust).

**Q: Pricing impact?**
A: Free for env/file backends. AWS KMS ≈ $0.03 / 10k requests
(cached → ~$0 in practice). Sigstore PGI free.

---

## 22. Glossary M19

| Term | Definition |
|---|---|
| **OTel** | OpenTelemetry — vendor-neutral observability standard |
| **OTLP** | OpenTelemetry Protocol — transport for traces/metrics/logs |
| **Trace ID** | Per-request UUID propagated across systems |
| **Cardinality** | Number of unique label combinations on a metric series |
| **Sigstore** | Free OSS supply chain security toolchain |
| **Rekor** | Sigstore transparency log |
| **Fulcio** | Sigstore short-lived cert authority |
| **TUF** | The Update Framework — root of trust for Sigstore |
| **Sigstore Bundle** | All-in-one envelope (cert chain + sig + Rekor proof) |
| **Keyless signing** | Signature backed by OIDC identity, no long-lived key |
| **BYOK** | Bring Your Own Key — customer holds the key material |
| **KMS** | Key Management Service (AWS / GCP / Azure) |
| **HSM** | Hardware Security Module |
| **PKCS#11** | Cryptoki standard for HSM interaction |
| **SSE** | Server-Sent Events (HTTP streaming) |

---

## 23. References

- **MCP spec** : <https://modelcontextprotocol.io/specification>
- **OpenTelemetry spec** : <https://opentelemetry.io/docs/specs/otel/>
- **Sigstore Bundle** : <https://github.com/sigstore/protobuf-specs>
- **Rekor v2 API** : <https://github.com/sigstore/rekor/openapi.yaml>
- **AWS KMS** : <https://docs.aws.amazon.com/kms/>
- **GCP Cloud KMS** : <https://cloud.google.com/kms/docs>
- **Azure Key Vault** : <https://learn.microsoft.com/en-us/azure/key-vault/>
- **HashiCorp Vault** : <https://developer.hashicorp.com/vault>
- **PKCS#11 v3.0** : <https://docs.oasis-open.org/pkcs11/>
- **structlog** : <https://www.structlog.org/>
- **Prometheus exposition** : <https://prometheus.io/docs/instrumenting/exposition_formats/>

---

## A — Full JSON Schemas

### A.1 `efe_verify_rekor_inclusion`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeVerifyRekorInclusionInput",
  "properties": {
    "log_index": {"type": "integer", "minimum": 0, "maximum": 999999999},
    "expected_artifact_digest": {
      "type": "string",
      "pattern": "^sha256:[a-f0-9]{64}$"
    },
    "rekor_url": {"type": "string", "minLength": 1, "maxLength": 200,
                  "pattern": "^https?://[^\\s]+$"}
  },
  "required": ["log_index", "expected_artifact_digest"],
  "additionalProperties": false
}
```

### A.2 `efe_check_keystore_health`

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeCheckKeystoreHealthInput",
  "description": "Return keystore backend status, key count, rotation status.",
  "properties": {},
  "additionalProperties": false
}
```

### A.3 `efe_rotate_acknowledged` (write)

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeRotateAcknowledgedInput",
  "properties": {
    "key_id": {"type": "string", "minLength": 1, "maxLength": 200},
    "old_key_status": {"type": "string", "enum": ["deprecated", "deleted"]},
    "new_key_id": {"type": "string", "minLength": 1, "maxLength": 200},
    "rotation_reason": {
      "type": "string",
      "pattern": "^[^\\r\\n]{10,500}$",
      "minLength": 10, "maxLength": 500
    },
    "idempotency_key": {
      "type": "string",
      "pattern": "^[A-Za-z0-9_\\-]{1,64}$"
    },
    "confirm": {"type": "boolean"}
  },
  "required": ["key_id", "old_key_status", "rotation_reason", "confirm"],
  "additionalProperties": false
}
```

### A.4 `efe_verify_bundle_signature` (extended)

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": "EfeVerifyBundleSignatureInput",
  "properties": {
    "slug": {"type": "string", "pattern": "^[a-z][a-z0-9\\-]{0,99}$"},
    "version": {"type": "string", "pattern": "^v\\d+\\.\\d+\\.\\d+(-[a-z0-9\\-]{1,15})?$"},
    "expected_key_id": {"type": "string", "maxLength": 100},
    "registry": {"type": "string", "maxLength": 200},
    "verification_method": {
      "type": "string",
      "enum": ["ed25519", "sigstore", "auto"],
      "default": "auto"
    },
    "expected_identity": {
      "type": "string",
      "maxLength": 500,
      "description": "OIDC subject for keyless signing (Sigstore method)"
    },
    "expected_issuer": {
      "type": "string",
      "maxLength": 200
    }
  },
  "required": ["slug", "version"],
  "additionalProperties": false
}
```

---

## B — KMS provider integration details

### B.1 AWS KMS

**Auth methods** (in priority order) :
1. **IRSA** (IAM Roles for Service Accounts) — preferred for K8s.
   Requires OIDC provider configured on EKS cluster + service-account
   annotation `eks.amazonaws.com/role-arn`.
2. **EC2 instance role** — fallback hors EKS.
3. **`AWS_PROFILE` env** (dev).
4. **Static `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY`** (last
   resort, déconseillé en prod).

**Required IAM permissions** (least-privilege policy) :

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Sid": "EfeMcpReadKMS",
      "Effect": "Allow",
      "Action": [
        "kms:GetPublicKey",
        "kms:DescribeKey"
      ],
      "Resource": [
        "arn:aws:kms:us-east-1:123456789012:key/abc-123",
        "arn:aws:kms:us-east-1:123456789012:key/def-456"
      ]
    }
  ]
}
```

**Crucial** : pas de `kms:Sign` — le mcp-server ne signe **JAMAIS** ;
il vérifie seulement. La distinction empêche un attaquant qui compromet
le service de l'utiliser pour signer des bundles malicieux.

**Cost model** :
- **Pricing** : ~$0.03 per 10 000 GetPublicKey requests + $1/key/mois.
- **Cache TTL** : 300 s par défaut → ~10–15 calls / hour / replica.
- **Coût mensuel typique** : $1/key/mois × 4 keys (multi-key trust) +
  ~$0.03 / 10 000 × 12 calls/h × 24 × 30 = ~$4.05/mois pour 4 keys
  + 5 replicas. Négligeable.

**Audit** : chaque appel KMS produit une ligne CloudTrail. Recommandé :
enable `multi-region trail` pour attaches résilience cross-region.

**Erreur handling** :
- `AccessDeniedException` : log WARN + raise
  `KeystoreError("aws_kms_access_denied")` ; opérateur doit fix
  IAM policy.
- `NotFoundException` : key supprimée → mcp-server retire la clé
  du trust set, log AUDIT, et bypasse la clé pour les vérifications
  futures (jusqu'au prochain refresh).
- `ThrottlingException` (rate exceeded) : exponential backoff via
  boto3 retry adaptive ; après 5 retries, fall-through au cache TTL
  étendu (jusqu'à 30 min) avec WARN.
- `KMSInternalException` : retry once, then log AUDIT + propagate
  comme `KeystoreError("aws_kms_internal_error")`.

**Région et endpoints** :
- Recommandé : VPC endpoint pour KMS (`com.amazonaws.<region>.kms`)
  pour éviter trafic Internet et MITM.
- `EFE_AWS_KMS_REGION` overridable per-clé.

**Rotation flow** (manual side, async detected by mcp-server) :
1. Cloud admin créé nouvelle key version dans AWS KMS console
   (rotation auto annuelle = simple version bump, ou manual = nouvelle
   key ARN ajoutée à `EFE_AWS_KMS_KEY_ARNS`).
2. `efe_check_keystore_health` détecte le nouveau ARN au prochain
   `keystore.refresh()` (TTL ou SIGUSR1).
3. Opérateur émet `efe_rotate_acknowledged` pour audit l'événement.
4. Bundles signés avec ancienne ou nouvelle clé continuent à valider
   tant que les deux clés sont dans `EFE_AWS_KMS_KEY_ARNS`.

**Test scaffolding** :
- `moto` (motoserver) pour mocker KMS in unit tests.
- `LocalStack` (community edition) pour integration tests local.
- AWS account dédié pour CI conditional test (`test_keystore_aws.py`
  skipped sans `AWS_PROFILE`).

### B.2 GCP Cloud KMS

**Auth methods** :
1. **Workload Identity** (preferred GKE) — namespace + KSA mapping
   à GSA via annotation `iam.gke.io/gcp-service-account`.
2. **Service Account JSON file** — via
   `GOOGLE_APPLICATION_CREDENTIALS` env, hors-K8s.
3. **`gcloud auth application-default login`** (dev only).

**Required IAM permissions** (custom role recommended) :

```yaml
# gcloud iam roles create efeMcpKmsReader \
#   --project=PROJECT \
#   --file=role.yaml
title: "EFE MCP KMS Reader"
description: "Read-only access to public keys"
stage: GA
includedPermissions:
- cloudkms.cryptoKeyVersions.viewPublicKey
- cloudkms.cryptoKeyVersions.get
```

Bind sur le service-account :

```bash
gcloud projects add-iam-policy-binding PROJECT \
  --member=serviceAccount:efe-mcp@PROJECT.iam.gserviceaccount.com \
  --role=projects/PROJECT/roles/efeMcpKmsReader
```

**Pas de** `cloudkms.cryptoKeyVersions.useToVerify` ni `useToSign` —
le mcp-server fait la vérification crypto **localement** une fois
la public key obtenue.

**Cost model** : Cloud KMS facturé à $0.06 / 10 000 cryptographic
operations (Get) + $0.06/key/mois pour software-protected, $1/key/mois
HSM-protected.

**Audit** : Cloud Audit Logs auto-enabled pour tous les KMS calls.

**Erreur handling** :
- `PermissionDenied` (gRPC code 7) : log + raise.
- `NotFound` (gRPC 5) : key version disabled / destroyed → opérateur.
- `ResourceExhausted` (gRPC 8) : retry avec backoff (google-cloud-kms
  client v2 has built-in).

**Régions** : préférer KMS keyring dans même région que le mcp-server
pour minimiser latence (sinon p50 augmente de ~50 ms).

### B.3 Azure Key Vault

**Auth methods** :
1. **Managed Identity** (preferred AKS) — pod-identity v2 (workload
   identity federation OIDC).
2. **Service Principal credentials** (`AZURE_CLIENT_ID`,
   `AZURE_TENANT_ID`, `AZURE_CLIENT_SECRET`).
3. **Azure CLI auth** (dev).

**Required permissions** :
- Role `Key Vault Crypto Service Encryption User` ou custom role :
  - `Microsoft.KeyVault/vaults/keys/read` (read key metadata)
  - `Microsoft.KeyVault/vaults/keys/getPublicKey/action` (NEW data
    action, available since 2023-07-01-preview)
- **NO** `keys/sign/action` — read-only.

**Az CLI assignment example** :

```bash
az role assignment create \
  --role "Key Vault Crypto Service Encryption User" \
  --assignee $MANAGED_IDENTITY_PRINCIPAL_ID \
  --scope $KEY_VAULT_RESOURCE_ID
```

**Auth flow** : lib `azure-identity` `DefaultAzureCredential` chain
auto. Cache token TTL 1h, refresh anticipé 5 min avant expiry.

**Cost model** : Standard tier $0.03 / 10 000 transactions + $1/key/mois.
HSM-backed (Premium) $5/key/mois.

**Audit** : Azure Monitor → Log Analytics workspace via diagnostic
settings. Enable `AuditEvent` category.

**Régions** : déployer Key Vault dans la même région Azure que l'AKS.
Cross-region adds ~80 ms latency.

### B.4 HashiCorp Vault

**Auth methods** :
1. **Token** (`VAULT_TOKEN` env) — simple, dev/staging.
2. **AppRole** — preferred prod ; CI/CD-friendly.
3. **K8s auth method** (vault-agent injector) — preferred K8s.

**Required policy** :

```hcl
# /sys/policies/acl/efe-mcp-keys-reader
path "secret/data/efe/keys/*" {
  capabilities = ["read"]
}

path "secret/metadata/efe/keys/*" {
  capabilities = ["list", "read"]
}

# If using transit engine:
path "transit/keys/efe-bundle-pubkey-*" {
  capabilities = ["read"]
}
```

**Storage convention** :
- KV v2 secrets engine : `secret/data/efe/keys/<key-id>` avec champ
  `pem` (PEM-encoded public key).
- OU transit secrets engine pour signing déléguée (M19 utilise KV v2
  par défaut, transit reserved for M20 signing).

**Auth flow exemple (AppRole)** :

```python
import hvac
client = hvac.Client(url=os.environ["VAULT_ADDR"])
client.auth.approle.login(
    role_id=os.environ["EFE_VAULT_ROLE_ID"],
    secret_id=os.environ["EFE_VAULT_SECRET_ID"],
)
secret = client.secrets.kv.v2.read_secret_version(
    path="efe/keys/abc-123",
)
pem = secret["data"]["data"]["pem"]
```

**Cost model** : Vault OSS gratuit + infra ; Vault Enterprise
licensé par node.

**Audit** : Vault audit log devices (file / syslog / socket) — enable
au minimum un device.

**HA / multi-DC** : Vault Performance Replication / DR Replication.
mcp-server pointe vers le local Vault performance secondary.

### B.5 PKCS#11 HSM

**Examples** : SoftHSM (dev), YubiHSM 2 (low-cost), AWS CloudHSM,
Thales Luna, Gemalto SafeNet, Utimaco SecurityServer.

**Lib Python** : `python-pkcs11` v0.7+ (recommandé) OU `pkcs11` package
plus moderne (M19 utilise `python-pkcs11`).

**Config** :

```bash
# Lib path varies by vendor
EFE_PKCS11_LIB=/usr/lib/softhsm/libsofthsm2.so
EFE_PKCS11_TOKEN_LABEL=efe-prod
EFE_PKCS11_PIN_FILE=/etc/efe/pkcs11.pin
EFE_PKCS11_OBJECT_LABEL_PREFIX=efe-bundle-pubkey
```

**PIN handling** :
- `EFE_PKCS11_PIN_FILE` doit être 0600 owned by mcp-server user.
- Container deployment : monter via Kubernetes Secret avec
  `defaultMode: 0400`.

**Discovery flow** :

```python
import pkcs11
lib = pkcs11.lib(os.environ["EFE_PKCS11_LIB"])
token = lib.get_token(token_label=os.environ["EFE_PKCS11_TOKEN_LABEL"])
with token.open(user_pin=read_pin_file()) as session:
    keys = session.get_objects({
        pkcs11.Attribute.CLASS: pkcs11.ObjectClass.PUBLIC_KEY,
        pkcs11.Attribute.LABEL_PREFIX: prefix,
    })
    for key in keys:
        pem = key[pkcs11.Attribute.VALUE]  # DER, encode to PEM
        ...
```

**FIPS mode** : si HSM est FIPS 140-2 / 140-3 validated, mcp-server
log `crypto.fips_validated=true` dans `efe_check_keystore_health`
output. Compliance hook pour HIPAA / FedRAMP.

**Cost model** : SoftHSM = 0$. YubiHSM 2 ~$650 hardware, no recurring.
AWS CloudHSM ~$1.45/h ($1043/mois). Thales Luna ~$15-30k+ CapEx.

**Audit** : HSM audit log via vendor tooling (Thales : SafeNet
Manager ; AWS CloudHSM : `aws cloudhsmv2 describe-clusters` +
CloudWatch Logs).

**Limitations** :
- PKCS#11 lib charge dans le process — un crash HSM driver crash
  le mcp-server. Mitigation : isolation via subprocess wrapper M20.
- Concurrency : la session PKCS#11 est single-threaded ; mcp-server
  utilise un pool de sessions (10 par défaut, `EFE_PKCS11_POOL_SIZE`).

### B.6 File backend

**Use case** : self-host dans un environnement air-gapped sans KMS,
mais avec besoin de plus que les env-vars (multiple keys, rotations).

**Layout** :

```
/etc/efe/keys/
├── README.md                         # operator notes
├── ed25519-prod-2026-q1.pem          # active key (current)
├── ed25519-prod-2025-q4.pem          # previous (kept for verification)
├── ed25519-prod-2025-q3.pem          # 2-quarters back (sunset soon)
└── archive/                          # retired keys
    └── ed25519-prod-2025-q2.pem
```

**Permissions** : directory `0700`, files `0400`, owned by mcp-server
user.

**Rotation flow** :
1. Operator drops new `.pem` file in `/etc/efe/keys/`.
2. SIGUSR1 to mcp-server process → `keystore.refresh()`.
3. New key picked up, logged as audit event `keystore_key_added`.

**Pin tests** :
- `test_keystore_file.py::test_loads_all_pem_files`
- `test_keystore_file.py::test_ignores_non_pem_files`
- `test_keystore_file.py::test_refresh_picks_up_new_files`
- `test_keystore_file.py::test_refresh_drops_removed_files`
- `test_keystore_file.py::test_invalid_pem_logs_warning_and_skips`
- `test_keystore_file.py::test_directory_permissions_warning`

### B.7 Env backend (M18 carry-over)

**Use case** : dev local + simple deployments.

**Variables** :
```bash
EFE_OCI_PUBLIC_KEY="-----BEGIN PUBLIC KEY-----\n..."
# Ou:
EFE_OCI_PUBLIC_KEY_FILE=/etc/efe/pubkey.pem
```

**Limitations** :
- Une seule clé.
- Pas de rotation atomic (process restart requis).
- M19 reste compatible — pas de breaking change.

**Migration env → file** : copier la clé dans `/etc/efe/keys/<id>.pem`
et changer `EFE_KEYSTORE_BACKEND=file`.

### B.8 Multi-cloud / hybrid pattern

Pour un déploiement multi-cloud (e.g., AWS prod + GCP DR), définir un
keystore "composite" :

```bash
EFE_KEYSTORE_BACKEND=composite
EFE_KEYSTORE_BACKENDS=aws-kms,gcp-kms,file
EFE_AWS_KMS_KEY_ARNS=arn:aws:kms:...
EFE_GCP_KMS_KEYS=projects/.../locations/.../keyRings/.../cryptoKeys/.../cryptoKeyVersions/1
EFE_FILE_KEYS_DIR=/etc/efe/keys
```

Comportement : un bundle est accepté si **AU MOINS UN** backend
valide. Coût : N round-trips KMS lors d'un cache miss. Mitigation :
TTL 600 s (vs 300 défaut).

---

## C — OTel span semantic conventions

### C.1 Naming convention

Tous les spans M19 suivent le préfixe **`mcp.*`** (pour distinguer
de l'instrumentation auto httpx / FastAPI / SQLAlchemy qui ont leurs
propres conventions). Les attributs custom sont préfixés `mcp.*` pour
ne pas collisionner avec les attributs OTel standards.

### C.2 Top-level spans (manuellement instrumentés par dispatcher)

| Span name | Kind | Layer | Required attributes | Optional attributes |
|---|:---:|---|---|---|
| `mcp.tool.dispatch` | INTERNAL | dispatcher | `mcp.tool.name` (str), `mcp.tool.is_error` (bool), `mcp.profile` (str), `mcp.tenant.hash` (str, sha256[:8]) | `mcp.tool.write` (bool), `mcp.idempotency_key.len` (int), `mcp.tool.weight` (float) |
| `mcp.resource.read` | INTERNAL | dispatcher | `mcp.resource.uri` (str), `mcp.resource.kind` (str) | `mcp.resource.size_bytes` (int), `mcp.resource.cache_hit` (bool) |
| `mcp.prompt.get` | INTERNAL | dispatcher | `mcp.prompt.name` (str) | `mcp.prompt.args_count` (int) |
| `mcp.sampling.round` | INTERNAL | sampling | `mcp.sampling.tool` (str), `mcp.sampling.model_hint` (str), `mcp.sampling.rounds_used` (int) | `mcp.sampling.cache_hit` (bool), `mcp.sampling.tokens_in` (int), `mcp.sampling.tokens_out` (int) |
| `mcp.audit.emit` | INTERNAL | audit | `mcp.audit.tool` (str), `mcp.audit.ok` (bool) | `mcp.audit.seq` (int), `mcp.audit.chain.head_hash` (str, hex16) |
| `mcp.session.lifecycle` | SERVER | http transport | `mcp.session.id` (str, redacted to 8 chars), `mcp.session.event` (str: open|close|abort) | `mcp.session.duration_ms` (int) |

### C.3 Sub-spans (auto-instrumented + manual)

| Span name | Kind | Layer | Required attributes | Notes |
|---|:---:|---|---|---|
| `HTTP <method>` | CLIENT | httpx (auto) | standard HTTP semantic conventions (`http.method`, `http.url`, `http.status_code`) | Auto-injected by `opentelemetry-instrumentation-httpx` |
| `<method> <route>` | SERVER | starlette (auto) | standard HTTP server (`http.method`, `http.target`, `http.status_code`) | Auto-injected by `opentelemetry-instrumentation-fastapi` |
| `mcp.bundle.oci.pull_manifest` | CLIENT | oci client | `oci.registry`, `oci.repo`, `oci.reference` (tag or digest) | Manual instrumentation in `bundle/oci.py` |
| `mcp.bundle.oci.pull_blob` | CLIENT | oci client | `oci.digest` (sha256:...), `oci.size_bytes` (int) | Manual |
| `mcp.bundle.verify.ed25519` | INTERNAL | verify | `mcp.bundle.slug`, `mcp.bundle.version`, `mcp.verify.ok` (bool) | |
| `mcp.bundle.verify.sigstore` | INTERNAL | sigstore | `mcp.sigstore.method` (keyless\|key-based), `mcp.sigstore.cert_identity` (str, optional), `mcp.sigstore.bundle_version` | |
| `mcp.bundle.rekor.fetch_proof` | CLIENT | sigstore | `mcp.rekor.url`, `mcp.rekor.log_index` | |
| `mcp.bundle.rekor.verify_proof` | INTERNAL | sigstore | `mcp.rekor.tree_size`, `mcp.rekor.root_hash` (truncated 16 chars) | |
| `mcp.keystore.lookup` | INTERNAL | keystore | `mcp.keystore.backend` (env\|file\|aws-kms\|gcp-kms\|azure-kv\|vault\|pkcs11), `mcp.keystore.cache_hit` (bool) | |
| `mcp.keystore.refresh` | INTERNAL | keystore | `mcp.keystore.backend`, `mcp.keystore.keys_count_before`, `mcp.keystore.keys_count_after` | Triggered by TTL or SIGUSR1 |
| `mcp.helm.values.render` | INTERNAL | deploy | `mcp.bundle.slug`, `mcp.bundle.version`, `mcp.helm.values_size_bytes` | |
| `mcp.deploy.dry_run` | INTERNAL | deploy | `mcp.bundle.slug`, `mcp.bundle.version`, `mcp.deploy.environment`, `mcp.deploy.signature_verified` (bool), `mcp.deploy.unverifiable` (bool), `mcp.deploy.outcome` (ok\|warn\|blocker) | |

### C.4 Error attributes

When `is_error=true`, also set sur le span :
- `mcp.tool.error_code` (str — mapped enum, e.g.
  `forbidden`, `not_found`, `confirm_required`, `invalid_arguments`,
  `rate_limited`, `tool_disabled`, `quota_exceeded`, `oci_pull_failed`,
  `signature_invalid`, `unverifiable`, `keystore_unavailable`,
  `redis_unavailable`).
- `error.type` (str — Python exception class name).
- `exception.message` (str — truncated to 256 chars, sensitive
  fields scrubbed via `summarize_args_for_audit`).
- `exception.stacktrace` (str, optional, only if
  `EFE_OTEL_INCLUDE_STACKTRACE=true`).
- Span status set to `StatusCode.ERROR`.

### C.5 Sampling decisions

- **Default sampler** : `parentbased_traceidratio` avec ratio 0.1 (10 %).
- **Critical paths** : forced 100 % sampling pour :
  - `is_write=true` tool calls (toutes les writes traced).
  - `mcp.tool.error_code != null` (auto-resampling via tail-based si
    OTel collector configuré).
  - `mcp.bundle.verify.*` (chaîne d'approvisionnement = full visibility).
- **Override** : `EFE_OTEL_TRACES_SAMPLER_ARG=1.0` pour 100 %, à
  utiliser pour debug only — coûteux.

### C.6 Resource attributes (set once par process)

Tous les spans héritent des attributes resource :
- `service.name` = `efe-mcp-server` (constant)
- `service.version` = `mcp_server.__version__` (e.g. `0.19.0`)
- `service.instance.id` = `EFE_REPLICA_ID` ou `socket.gethostname()`
- `deployment.environment` = `EFE_DEPLOYMENT_ENV` (production\|staging\|dev)
- `process.runtime.name` = `cpython`
- `process.runtime.version` = sys.version
- `host.name` = uname

### C.7 Span links

- Un `mcp.tool.dispatch` span peut avoir un **link** vers le span
  parent côté client MCP si W3C `traceparent` propagé. Pas de
  parent-child (cross-process) automatique pour préserver l'isolation.
- Un `mcp.audit.emit` span peut avoir un link vers le `mcp.tool.dispatch`
  qui l'a déclenché si on les sépare temporellement (e.g. async
  audit batch).

### C.8 Sensitive attribute exclusion

**JAMAIS** comme attribut OTel :
- `tenant_id` brut (utiliser `mcp.tenant.hash` à la place)
- `idempotency_key` brut (utiliser `mcp.idempotency_key.len`)
- `rationale` (raison free-form donnée à `request_deployment`)
- Bearer tokens / OIDC tokens / KMS tokens
- Bundle bodies / SBOM contents (just digest)
- User content sampled (only count tokens)

Ces règles sont enforcés par `mcp_server/observability/span_redactor.py`
qui utilise un `SpanProcessor` custom pour scrubber pre-export.

---

## D — Pin tests inventory

### D.1 `test_observability.py` (12 tests, D51)

- `test_setup_tracing_no_op_when_exporter_unset`
- `test_setup_tracing_uses_otlp_exporter`
- `test_dispatch_creates_tool_dispatch_span`
- `test_dispatch_span_carries_tool_name_attribute`
- `test_dispatch_span_records_error_on_exception`
- `test_meter_creates_all_metrics`
- `test_metric_cardinality_bounded`
- `test_log_carries_trace_id_when_span_active`
- `test_log_no_trace_id_when_no_span`
- `test_propagation_extracts_w3c_trace_context`
- `test_metrics_endpoint_serves_prometheus_format`
- `test_otel_exporter_buffer_drops_on_backpressure`

### D.2 `test_sigstore_verify.py` (12 tests, D52)

- `test_verify_sigstore_bundle_unavailable_when_extra_missing`
- `test_verify_sigstore_bundle_v0_3_basic`
- `test_verify_sigstore_bundle_invalid_json_returns_error`
- `test_verify_sigstore_bundle_with_identity_match`
- `test_verify_sigstore_bundle_with_identity_mismatch`
- `test_verify_sigstore_bundle_with_custom_rekor_url`
- `test_verify_sigstore_bundle_cert_chain_invalid`
- `test_verify_sigstore_bundle_subject_digest_match`
- `test_verify_sigstore_bundle_integrated_time_present`
- `test_auto_method_falls_back_to_ed25519_when_no_sigstore_bundle`
- `test_auto_method_prefers_sigstore_when_both_present`
- `test_method_explicit_ed25519_skips_sigstore_lookup`

### D.3 `test_rekor_inclusion.py` (8 tests, D52)

- `test_rekor_inclusion_basic`
- `test_rekor_inclusion_artifact_digest_mismatch_rejected`
- `test_rekor_inclusion_invalid_log_index_returns_error`
- `test_rekor_inclusion_custom_rekor_url`
- `test_rekor_inclusion_unreachable_rekor_returns_error`
- `test_rekor_inclusion_checkpoint_signature_verified`
- `test_rekor_inclusion_returns_integrated_time`
- `test_rekor_inclusion_returns_kind_version`

### D.4 `test_keystore_factory.py` (10 tests, D53)

- `test_factory_default_is_env`
- `test_factory_unknown_backend_raises`
- `test_factory_env_returns_env_keystore`
- `test_factory_file_returns_file_keystore`
- `test_factory_aws_kms_returns_aws_keystore` (skipped if boto3 missing)
- `test_factory_keystore_implements_abstract_methods`
- `test_factory_keystore_has_health_check`
- `test_factory_keystore_aclose_idempotent`
- `test_factory_supports_multiple_keys`
- `test_factory_returns_empty_list_when_misconfigured`

### D.5 `test_keystore_aws.py` (6 tests, D53, conditional)

- `test_aws_keystore_pulls_pem_from_kms`
- `test_aws_keystore_caches_with_ttl`
- `test_aws_keystore_handles_throttling`
- `test_aws_keystore_health_returns_audit_logged_true`
- `test_aws_keystore_arns_parsed_from_env`
- `test_aws_keystore_aclose_releases_client`

### D.6 `test_keystore_file.py` (6 tests, D53)

- `test_file_keystore_loads_directory_of_pem_files`
- `test_file_keystore_skips_non_pem_files`
- `test_file_keystore_health`
- `test_file_keystore_supports_multiple_keys`
- `test_file_keystore_handles_missing_directory`
- `test_file_keystore_aclose_noop`

### D.7 `test_keystore_health_tool.py` (6 tests, D53)

- `test_check_keystore_health_returns_backend`
- `test_check_keystore_health_returns_key_count`
- `test_check_keystore_health_lists_active_keys`
- `test_check_keystore_health_warning_on_no_keys`
- `test_check_keystore_health_aws_audit_logged_flag`
- `test_check_keystore_health_emits_audit_event`

### D.8 `test_http_transport.py` (10 tests, D54)

- `test_http_app_serves_sse_endpoint`
- `test_http_app_serves_messages_endpoint`
- `test_http_session_cleanup_on_disconnect`
- `test_http_messages_to_unknown_session_returns_404`
- `test_http_health_endpoint_ok`
- `test_http_ready_endpoint_lists_active_sessions`
- `test_http_metrics_endpoint_serves_prometheus`
- `test_http_graceful_shutdown_drains_inflight`
- `test_http_concurrent_sessions_isolated`
- `test_http_session_dispatcher_uses_per_request_trace_id`

### D.9 `test_redis_backed.py` (6 tests, D54)

- `test_redis_bundle_cache_get_put`
- `test_redis_bundle_cache_namespace_isolation`
- `test_redis_rate_limiter_distributed_state`
- `test_redis_sampling_cache_ttl_respected`
- `test_redis_partition_handling_logs_warning`
- `test_redis_close_cleans_up_connections`

### D.10 `test_audit_pass_m19.py` (~18 tests, D54 post-livraison)

(Filled post-audit — pattern récurrent M11-M18.)

---

## E — Backend contract diff EFE 1.2 → 1.3

### E.1 New REST endpoints (additive)

| Method | Path | Auth | Phase |
|---|---|---|---|
| GET | `/admin/transparency/{log_index}` | OIDC | D52 |
| GET | `/admin/keystore/status` | OIDC admin | D53 |
| POST | `/admin/keystore/rotation-event` | OIDC admin | D53 |

### E.2 New gRPC RPC (additive)

```proto
service EfeAdminService {
  // existing M16-M18 RPCs unchanged …
  rpc VerifyRekorInclusion(VerifyRekorInclusionRequest) returns (VerifyRekorInclusionResponse);
}
```

### E.3 Backward compat

- All M16-M18 + EFE 1.0/1.1/1.2 endpoints unchanged.
- EFE 1.2 client + EFE 1.3 server : compatible.
- mcp-server M19 detects EFE 1.2 backend via `/info` and falls back :
  - Sigstore : direct Rekor pull, no backend mediation.
  - Keystore : local view only.
  - Rotation events : local audit chain only.

### E.4 Migration plan

| Étape | Action | Owner | Quand |
|---|---|---|---|
| 1 | Bump `Cargo.toml` v1.2.x → v1.3.0-pre | Rust lead | D52 J1 |
| 2 | Implémenter 3 endpoints REST + 1 RPC | Rust lead | D52-D53 |
| 3 | Pin tests `crates/efe-server/tests/m19_contract.rs` | Rust lead | D53 |
| 4 | Bump OpenAPI + .proto regen | Rust lead | D53 J5 |
| 5 | Release EFE 1.3.0 image | Release manager | D54 J1 |
| 6 | mcp-server M19 detect 1.3 via `/info` | Python lead | D54 J2 |
| 7 | mcp-server fallback paths for 1.2 | Python lead | D54 J3 |

---

**FIN**

Document M19 architecture. ~2 700 lignes. Prêt à passer en livraison
sprint D51. Toute revue chirurgicale : ouvrir issue
`docs/m19_review_pass_*` ; pattern récurrent M11-M18.
