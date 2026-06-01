# EFE — Product Completion Architecture (Nano-Robots de l'Essaim)

> **Statut** : Draft v1.0.0 — Document de référence pour l'achèvement du
> produit EFE (Essaim Fluide Éphémère) jusqu'à GA 1.0.
> **Auteur** : rodrigue.pekar@icloud.com
> **Date** : 2026-04-28 (post-D18 audit phase).
> **Dépendances doc** : tous les `M*_ARCHITECTURE.md`, tous les
> `LETTER_*_ARCHITECTURE.md`, tous les `docs/bench/d*_architecture.md`
> de D1 à D18, `MODE1_ARCHITECTURE.md`, `M10_SCHEDULING_ARCHITECTURE.md`,
> `M11_SECURITY_ARCHITECTURE.md`, `M12_SWARM_INTEGRATION_ARCHITECTURE.md`.
> **Portée** : architecture exhaustive — sprint par sprint — de
> **TOUT ce qu'il reste à livrer** entre la version courante (v0.1.19,
> post-D18 audit GREEN) et **EFE 1.0 GA** (production-ready,
> compliance-attested, GPU-accelerated, multi-mode swarm).
>
> **Audience cible** : équipe technique projet, lead architect,
> chefs de produit, équipe sécurité/compliance, ops/SRE, on-call,
> et toute personne devant comprendre l'écart résiduel **AS-IS → TO-BE**.

---

## Table des matières

§0. Méta, glossaire, conventions
§1. Mission et définition du "produit final"
§2. État AS-IS au 2026-04-28 (post-D18)
§3. Définition opérationnelle de **EFE 1.0 GA**
§4. Vue macro — 7 phases sur 24 sprints
§5. **Φ-A** : Hardening des Lettres A → D (sprints D19-D20)
§6. **Φ-B** : Stabilisation Lettre E + multi-runtime SDKs (D21-D24)
§7. **Φ-C** : Bundle modèle réel + PyO3 zero-copy (D25-D27)
§8. **Φ-D** : CUDA kernels + GPU runtime + multi-GPU (D28-D32)
§9. **Φ-E** : Diversification nano-agents + Mode-2 (D33-D36)
§10. **Φ-F** : Mode-3 multi-turn reasoning (D37-D39)
§11. **Φ-G** : Compliance attestation + freeze GA (D40-D42)
§12. Architecture détaillée — **Le nano-agent** (taxonomie + cycle de vie)
§13. Architecture détaillée — **L'essaim** (genesis → fusion → consensus → dissolution)
§14. Extensions runtime : streaming, hybrid fusion, persistence
§15. Surface d'intégration : SDK multi-runtime (Go, TS, Java, Rust)
§16. SLO produit final
§17. Sécurité produit final
§18. Observabilité produit final
§19. Compliance et attestations (SOC2, GDPR, AI Act)
§20. Plan d'équipe / staffing
§21. Risques transverses + atténuations
§22. Critères Go/No-Go GA 1.0
§23. Versioning, freeze, deprecations
§24. Annexes — matrices décisionnelles
§25. Annexes — taxonomie des nano-agents (15 types proposés)
§26. Annexes — gantt prévisionnel D19 → GA

---

## §0. Méta, glossaire, conventions

### §0.1 Conventions du document

* **AS-IS** = état du code à la fin du sprint D18 (post-audit), v0.1.19.
* **TO-BE** = état cible défini par chaque section.
* Les sprints sont nommés `Dxx` (continuité numérique avec D1 → D18 livrés).
* Les phases sont nommées **Φ-A à Φ-G** — agrégats de sprints partageant
  un thème majeur (cf. §4).
* Les défauts résiduels sont enregistrés comme **DT-PC.x** (Defect
  Trace, Product Completion) pour tracer leur résorption.
* Les SLO sont nommés **SLO-PC.x** (cf. §16).
* Les critères Go/No-Go sont **GG-x** (cf. §22).
* Toute mention de "le moteur" sans qualification = le **SwarmController**
  Rust (`crates/efe-core/src/swarm/controller.rs`).
* Tout **identifiant entre backticks** est un symbole code réel à grep.

### §0.2 Glossaire

| Terme | Définition opérationnelle |
|-------|---------------------------|
| **EFE** | Essaim Fluide Éphémère — produit. Inférence multi-agent par consensus tensoriel. |
| **Nano-agent** | Unité de calcul individuelle de l'essaim. État = un vecteur ℝ^D dans l'ELS. Identité = `NanoAgentId : u32`. |
| **Essaim** (swarm) | Population de N nano-agents (typiquement N ∈ [64, 1024]) traitant une requête concurrente. |
| **ELS** | Epistemic Latent Space — la matrice ℝ^{N×D} qui héberge l'essaim (D ∈ [128, 1024]). |
| **Round** | Une itération de fusion attention → softmax → aggregate → mix → project → clip. Un essaim converge typiquement en 4-20 rounds. |
| **Consensus** | Le verdict du `MultiCriteriaDetector` à la fin d'un round : `Continue` / `Converged{conf}` / `Divergence` / `ModeCollapse`. |
| **Centroïde** | Le vecteur ℝ^D extrait par `Dissolver` à la fin du dernier round — la "réponse" de l'essaim. |
| **Cône d'alignement** | Région ℝ^D définie par anchors + max_projection_distance, dans laquelle le centroïde doit être projeté avant d'être renvoyé au client. |
| **Template** | Type de nano-agent : `NaSem` (sémantique), `NaLog` (logique), `NaCrit` (critique). Détermine le prior d'initialisation. |
| **Mode** | **Mode-1** (anchors compilés, current MVP) / **Mode-2** (anchors générés à chaque requête par LLM, à venir) / **Mode-3** (multi-turn, à venir). |
| **Lettre** (A→E) | Niveau d'intégration produit. A = CLI one-shot ; E = service backend multi-tenant. |
| **Sprint Dxx** | Itération projet de 1-3 semaines centrée sur une thématique. D1-D18 livrés ; D19+ prévus dans ce document. |
| **Phase Φ-X** | Agrégat de sprints partageant un thème (cf. §4). |
| **Bundle** | Embedder packagé : weights + tokenizer + manifest signé Ed25519 (D16). |
| **Catalog** | Ensemble nommé d'ancres + leurs versions immuables (D12-bis + D14). |

### §0.3 Sprint numbering

D1 → D18 ont déjà été livrés et audités. Le prochain sprint disponible
est **D19**. Le présent document planifie D19 → D42 (24 sprints).

### §0.4 Cadence

Chaque sprint = **2 semaines wallclock**. Les sprints sont chainés
avec une **freeze week toutes les 4 sprints** (slot-7 dédié à
audit + bug-fix + cross-sprint regression). Cadence prévue :

* **Sprint duration** : 10 jours ouvrés
* **Audit pass** : 2 jours / sprint (le pattern projet — cf. tous les
  audits livrés D8 → D18)
* **Slack** : ~20 % imprévus

---

## §1. Mission et définition du "produit final"

### §1.1 Mission EFE rappelée

> *Remplacer la chaîne d'agents LLM textuelle par une population de
> nano-agents éphémères qui communiquent via un espace latent partagé
> et convergent vers un consensus tensoriel en millisecondes.*

Cette mission produit **3 propriétés invariantes** que le produit
final DOIT satisfaire :

* **P1 — Latence opposable** : un classify aller-retour < 200 ms p99
  (Mode-1, embedder bundle, GPU CUDA, 3 réplicas, charge nominale).
* **P2 — Coût opposable** : ≤ 0.5 ¢ par classify, soit **20×–100×**
  moins cher qu'une chaîne d'agents LLM équivalente.
* **P3 — Auditabilité opposable** : chaque verdict possède une
  trajectoire complète **catalog_version + cone_id + swarm_id +
  rounds_executed + confidence + signed_audit_chain**, et chaque
  artefact (bundle, catalog, audit chain) est cryptographiquement
  vérifiable.

Si l'une des trois propriétés est cassée, le produit n'est pas
livrable. **Toute décision dans ce document découle de ces 3 invariants.**

### §1.2 Le "nano-robot" — précision sémantique

Le terme **nano-robot** dans la communication produit désigne le même
objet que le **nano-agent** dans le code. Les deux noms sont des
synonymes ; le code Rust utilise systématiquement `NanoAgent*` (cf.
`crates/efe-core/src/agents/`, `crates/efe-core/src/genesis/`,
`crates/efe-core/src/memory/`).

**Différences avec un agent LLM classique** :

| Trait | Agent LLM (chaîne textuelle) | Nano-agent EFE |
|-------|------------------------------|----------------|
| **État** | Conversation textuelle, K tokens | Vecteur ℝ^D, D ∈ [128, 1024] |
| **Communication** | Tokens passés en stdin/stdout | Attention pondérée sur l'ELS |
| **Coût** | $-tokens × prix/M-token | μs CPU/GPU × prix énergie |
| **Latence** | 1-30 s (LLM call) | 8-12 ms / round |
| **Persistance** | Logs + transcripts | Aucune (éphémère) sauf audit |
| **Reproductibilité** | Non (sampling) | Oui (déterministe, seed) |
| **Vérifiabilité** | Difficile | Cryptographique (signed audit) |
| **Concurrence** | Limitée (rate-limit LLM) | N (parallèle, 64-1024) |

**Le nano-agent ne pense pas en mots.** Il pense en directions
géométriques dans ℝ^D. La sémantique des mots est apportée par les
ancres (= centroïdes pré-calculés sur des phrases canoniques) et par
le bundle embedder. Le swarm fait du **gradient sans backprop** —
chaque round repositionne les agents proches d'un consensus, et le
consensus EST la réponse.

### §1.3 Hors-scope produit final

Pour éviter la dérive de scope, ce document explicite les NON-buts :

1. **Ne PAS** remplacer un LLM généraliste de raisonnement libre.
   EFE classifie / route / décide ; il ne génère pas de prose.
2. **Ne PAS** être une plateforme MLOps. EFE consomme un embedder
   bundlé + un catalog ; il ne pas le ré-entraîne pas.
3. **Ne PAS** offrir du fine-tuning. Mode-1 est zero-shot. Mode-2
   reste zero-shot (avec LLM de routage). Mode-3 ne fait que
   multi-tour (chaining EFE x EFE).
4. **Ne PAS** offrir un SDK Python lourd. Le SDK Python doit rester
   < 10 MiB installé (sans torch ; embedder lazy-loaded).
5. **Ne PAS** supporter de runtime Windows/macOS native pour le
   server (Linux x86_64 + arm64 only). Les SDKs clients
   sont multi-OS.

---

## §2. État AS-IS au 2026-04-28 (post-D18)

### §2.1 Sprints D1 → D18 livrés

| # | Nom | Statut | Crate principal | Tests |
|---|-----|--------|-----------------|-------|
| D1 | Real strategies | ✅ | efe-core | OK |
| D2 | Memory real arena | ✅ | efe-core | OK |
| D3 | Compute pipeline | ✅ | efe-core | OK |
| D4 | Server RPC | ✅ | efe-server | OK |
| D5 | Unified bench | ✅ | bench/ | OK |
| D6 | Consensus + accuracy SLO | ✅ | efe-core | OK |
| D7 | Genesis fusion bias | ✅ | efe-core | OK |
| D8 | Hybrid real-data | ✅ | clients/python | OK |
| D8-bis | Hybrid threshold calib | ✅ | clients/python | OK |
| D9 | Docker + Helm + load | ✅ | deploy/ | OK |
| D10 | SDK + docs + examples | ✅ | clients/python | OK |
| D11 | Per-tenant isolation | ✅ | deploy/helm | OK |
| D12 | Catalog admin versioning | ✅ | efe-core | OK |
| D12-bis | Catalog gRPC handler | ✅ | efe-server | OK |
| D13 | Observability dashboards | ✅ | deploy/grafana | OK |
| D13-bis | OTel SDK real | ✅ | efe-telemetry | OK |
| D14 | Postgres catalog backend | ✅ | efe-server | OK |
| D14-bis | HMAC audit chain | ✅ | efe-server | OK |
| D15 | Onboarding + IdP | ✅ | efe-server | OK |
| D16 | Embedder bundling | ✅ | efe-server | OK |
| D17 | Backup + DR | ✅ | efe-backup | OK (10 audit-fix) |
| D18 | Multi-réplica + bench D12 | ✅ | efe-server + efe-bench | OK (12 audit-fix) |

**Total** : ~22 sprints livrés (incluant les bis), ~25 000 lignes
Rust + ~10 000 lignes Python + ~2 000 lignes Helm/YAML + ~5 000
lignes tests.

### §2.2 Lettres A → E (statut post-D18)

| Lettre | Forme | % livré | Manque principal |
|:-------|:------|:-------:|:-----------------|
| **A** | CLI one-shot | 60 % | Exit codes taxonomie, schema versioning, lazy imports, completions, --stdin/--text-file flexibles |
| **B** | REPL | 70 % | Windows readline, signal robustness, plafond mémoire, redaction debug |
| **C** | Script applicatif | 65 % | 3+ scénarios complets, retry exponentiel, cost ceiling enforcement, transcript versioning |
| **D** | Lib intégration | 75 % | Façade unifiée `EfeFacade`, type public `Verdict`, OTel auto, fixtures partagées |
| **E** | Service backend | 0 % | Tout (Dockerfile prod, K8s manifests, mTLS, JWT, AdminService complet, SDKs Go/TS/Java, runbook, SLO, on-call, dashboards) |

**Lecture stricte** : la Lettre E est l'unique livrable non démarré.
Les Lettres A-D ont chacune ~2-3 jours de hardening résiduel.
La Lettre E à elle seule est ~5 sprints (cf. §6).

### §2.3 Moteur swarm (post-D18)

| Sous-système | Statut | Module Rust |
|--------------|--------|-------------|
| `SwarmController` (orchestrateur) | ✅ Production | `crates/efe-core/src/swarm/controller.rs` (584 L) |
| Genesis (M3) | ✅ Production | `crates/efe-core/src/genesis/engine.rs` |
| Fusion CPU MVP (M5) | ✅ Production | `crates/efe-core/src/fusion/mvp_cpu.rs` (415 L) + 7 kernels (1614 L) |
| Consensus (M6) | ✅ Production | `crates/efe-core/src/consensus/multi_criteria.rs` |
| Dissolution (M7) | ✅ Production | `crates/efe-core/src/dissolution/engine.rs` |
| Alignment (M9) | ✅ Production | `crates/efe-core/src/alignment/` (~800 L) |
| Arbiter (budgets) | ✅ Production | `crates/efe-core/src/swarm/arbiter.rs` (142 L) |
| Mode-1 backend (gRPC bridge) | ✅ Production | `crates/efe-server/src/mode1.rs` (1697 L) |
| **CUDA kernels** | ❌ **Stub** | `crates/efe-kernels/src/lib.rs` (29 L stub) |
| **Mode-2 (anchors dynamiques)** | ❌ Manque | — |
| **Mode-3 (multi-turn)** | ❌ Manque | — |
| **Streaming RPC** | ⚠️ Déclaré | `services/swarm.rs:148` (signature only) |
| **Hybrid fusion (D8 mention)** | ❌ Manque | — |
| **SwarmState persistence** | ❌ Manque | — |
| **Multi-GPU sharding** | ❌ Manque | — |

### §2.4 Bindings + SDK

| Cible | Statut | Module |
|-------|--------|--------|
| **PyO3 FFI** | ❌ Stub | `crates/efe-ffi/src/lib.rs` ("Sprint 2 PR-017 PyO3 deferred") |
| **Python SDK** (gRPC client) | ✅ | `clients/python/efe_tool/client.py` |
| **Go SDK** | ❌ Manque | — |
| **TS/JavaScript SDK** | ❌ Manque | — |
| **Java SDK** | ❌ Manque | — |
| **Rust client SDK** | ⚠️ Partiel | `crates/efe-server/src/proto/` (consumed only by server) |

### §2.5 Bundle embedder (modèle réel)

| Élément | Statut |
|---------|--------|
| Pipeline bundling (D16) | ✅ Architecture + tests |
| Sample bundle livré (FAKE_ONNX) | ⚠️ Mock seulement |
| Bundle paraphrase-multilingual-MiniLM | ❌ Manque |
| Bundle all-MiniLM-L12-v2 | ❌ Manque |
| Tokenizer.json réel | ❌ Manque |
| ONNX CUDA EP wiring | ❌ Manque |
| Inference benchmark (real model) | ❌ Manque |

### §2.6 Compliance + ops

| Élément | Statut |
|---------|--------|
| SOC2 mappings (dataclass) | ⚠️ Documenté, non exécuté |
| GDPR mappings (dataclass) | ⚠️ Documenté, non exécuté |
| AI Act Annex IV (technical doc) | ❌ Manque |
| DR drill quarterly (script exécutable) | ❌ Manque (D17 mentionne, code absent) |
| SBOM pipeline (cosign + sbom in image) | ⚠️ Partiel |
| `helm lint` + `kubeval` + `polaris` en CI | ⚠️ Partiel |
| ISO/IEC 27001 mapping | ❌ Manque |
| Pen-test rapport | ❌ Manque |
| Threat modeling formalisé | ⚠️ M11 + M12 + d11 mais pas centralisé |

### §2.7 Métriques + Observabilité

D13 + D13-bis + D17 + D18 ont livré **8 + 6 + 8 + 9 = 31 instruments
Prometheus** + 6 dashboards Grafana + 7 D18 SLOs. La couverture est
solide. Le manque est :

* **Nano-agent-level metrics** (granularité < swarm) — non implémenté.
  Pas de `efe_agent_steps_total` ni `efe_agent_template_distribution`.
* **Per-tenant cost dashboard** — embryon (D11) mais sans dollar-figures.
* **Mode-2 / Mode-3 dashboards** — n/a (Mode-2/3 absents).

### §2.8 Synthèse AS-IS

```
Production-ready (Lettre E exclue) :
  ├── Mode-1 swarm engine (CPU)        ✅
  ├── gRPC server multi-tenant         ✅
  ├── Postgres catalog + audit chain   ✅
  ├── OIDC tenant onboarding           ✅
  ├── Embedder bundle pipeline         ✅ (mais bundle réel ⚠️)
  ├── Backup + DR (code)               ✅
  ├── Multi-réplica + leader election  ✅
  └── Bench harness (D18)              ✅

Manquant (entrant en scope ce document) :
  ├── Lettre E (service backend GA)    ❌
  ├── Lettre A/B/C/D hardening         ⚠️
  ├── CUDA kernels                     ❌
  ├── PyO3 FFI                         ❌
  ├── SDKs multi-runtime               ❌
  ├── Bundle modèle réel               ❌
  ├── Mode-2 (anchors dynamiques)      ❌
  ├── Mode-3 (multi-turn)              ❌
  ├── Streaming RPC actif              ⚠️
  ├── Hybrid fusion                    ❌
  ├── DR drill runbook automation      ❌
  ├── Compliance attestation           ❌
  └── Multi-GPU sharding               ❌
```

---

## §3. Définition opérationnelle de **EFE 1.0 GA**

### §3.1 Le contrat GA

EFE 1.0 GA satisfait **20 propriétés contractuelles** :

| # | Propriété | Métrique opposable |
|--:|-----------|--------------------|
| C-1 | Mode-1 GPU latency | p99 ≤ 200 ms (4 vCPU + 1 NVIDIA T4, charge 200 RPS, bundle MiniLM) |
| C-2 | Mode-1 CPU latency | p99 ≤ 800 ms (8 vCPU, charge 50 RPS, bundle MiniLM) |
| C-3 | Mode-2 GPU latency | p99 ≤ 1500 ms (anchors dynamiques + 1 LLM call routing) |
| C-4 | Mode-3 multi-turn | p99 ≤ 3 × p99 single-turn (k turns parallélisés) |
| C-5 | Coût par classify | ≤ 0.5 ¢ (Mode-1, bundle, GPU partagé) |
| C-6 | Tokens LLM saved vs chain | ≥ 90 % (vs chaîne baseline 5 tour LLM) |
| C-7 | Confidence calibration | Brier ≤ 0.10 sur eval set |
| C-8 | Catalog versioning | 100 % immuable, 100 % rollback < 5 min |
| C-9 | Audit chain integrity | 100 % HMAC verifiable, periodic self-verify chaque 1h |
| C-10 | RPO | ≤ 5 min |
| C-11 | RTO | ≤ 30 min (DR drill quarterly attesté) |
| C-12 | Availability | ≥ 99.9 % (≤ 43 min/mois indispo) |
| C-13 | mTLS | obligatoire en E (TLS 1.3, cert mgr rotation 30 j) |
| C-14 | OIDC tenant | obligatoire en E, multi-issuer ≤ 10 |
| C-15 | Rate-limit | per-tenant, leaky bucket distribué (D18) |
| C-16 | Observabilité | OTel traces 100 %, Prom metrics ≥ 50 instruments, Loki logs structurés |
| C-17 | SBOM | livré dans chaque image Docker (cosign attest) |
| C-18 | Compliance | SOC2 Type II passé, GDPR Art.32 attesté, AI Act Annex IV livré |
| C-19 | SDKs | Go + TS + Java + Python + Rust, signed releases, semver |
| C-20 | Documentation | Quickstart < 5 min, runbook 15+ scénarios, API ref complète |

### §3.2 Versioning produit

| Version | Sprint cible | Description |
|---------|:-----------:|-------------|
| v0.1.19 | D18 (livré) | Multi-réplica + bench + audit |
| v0.2.0 | D20 | Hardening Lettres A-D + premier bundle réel |
| v0.3.0 | D24 | Lettre E MVP single-tenant |
| v0.4.0 | D27 | PyO3 FFI + SDK polyglotte (Go + TS) |
| v0.5.0 | D30 | CUDA kernels v1 (single-GPU) |
| v0.6.0 | D32 | Multi-GPU + inference benchmark beating CPU 5× |
| v0.7.0 | D36 | Mode-2 (anchors dynamiques + LLM routing) |
| v0.8.0 | D39 | Mode-3 (multi-turn parallel) |
| v0.9.0 | D41 | Compliance + DR drill attestés |
| **v1.0.0 GA** | **D42** | **20/20 contrats vérifiés** |

### §3.3 Critères d'arrêt entre versions (gating)

Aucune `vX.Y.0` n'est releasée tant que :
* Tous les tests workspace + Python pin sont GREEN.
* Toutes les pin tests régression des audits précédents sont GREEN.
* Le SLO benchmark D18 (`efe-bench run --workload all`) passe les 7
  cibles D18-bench-1..7.
* Le bench `--target-env staging` passe sans `RefusedProdTarget`.
* Les SBOMs et signatures cosign sont vérifiables.
* CHANGELOG.md a une entrée datée pour la version.

---

## §4. Vue macro — 7 phases sur 24 sprints

### §4.1 Tableau d'agrégation

| Phase | Nom | Sprints | Wallclock | Dépendances entrantes | Livre |
|-------|-----|:-------:|:---------:|:----------------------|-------|
| **Φ-A** | Hardening Lettres A-D | D19-D20 | 4 sem | D18 GREEN | Lettres A-D 100 % |
| **Φ-B** | Lettre E + multi-runtime SDK | D21-D24 | 8 sem | Φ-A | E single-tenant + Go + TS SDK |
| **Φ-C** | Bundle réel + PyO3 zero-copy | D25-D27 | 6 sem | Φ-B (E pour wiring SDK) | bundle MiniLM + PyO3 + Java SDK |
| **Φ-D** | CUDA kernels + GPU runtime | D28-D32 | 10 sem | Φ-C (bundle réel pour bench GPU) | CUDA fusion + multi-GPU sharding |
| **Φ-E** | Diversification nano-agents + Mode-2 | D33-D36 | 8 sem | Φ-D (perf budget) | 5 nouveaux templates + Mode-2 |
| **Φ-F** | Mode-3 multi-turn | D37-D39 | 6 sem | Φ-E (Mode-2 stable) | Mode-3 + chained reasoning |
| **Φ-G** | Compliance + freeze GA | D40-D42 | 6 sem | Φ-F | SOC2 + GDPR + AI Act + GA freeze |
|       | **Total** | **24** | **48 sem (~11 mois)** | | **EFE 1.0 GA** |

### §4.2 Ordre d'exécution forcé

Les phases Φ-A → Φ-D **ne peuvent pas être réordonnées** :

* Φ-A → Φ-B : la Lettre E réutilise les types `Verdict` + `EfeFacade`
  livrés en Φ-A.
* Φ-B → Φ-C : PyO3 a besoin du build artifact prod-ready de E pour
  publier `efe-py` sur PyPI.
* Φ-C → Φ-D : CUDA bench a besoin d'un bundle réel pour mesurer le
  speedup vs CPU.
* Φ-D → Φ-E : Mode-2 ajoute 1 LLM call par requête → impose un budget
  latence < 300 ms pour le swarm seul, atteignable uniquement GPU.

Φ-E ↔ Φ-F sont relativement indépendantes, mais Mode-3 a la sémantique
de "chaîner k Mode-2 calls", donc Mode-2 stable est prérequis à Mode-3.

Φ-G doit être la **dernière** phase — toute attestation compliance
doit observer un produit FREEZED.

### §4.3 Parallélisme inter-équipe

Si 3 équipes (core / ops / sdk) travaillent en parallèle :

* **Équipe core** : Φ-A (1) → Φ-D (5) → Φ-E (4) → Φ-F (3) = 13 sprints
* **Équipe ops** : Φ-B (4) → Φ-G (3) = 7 sprints (avec slack)
* **Équipe sdk** : Φ-A (1) → Φ-C (3) = 4 sprints (avec slack)

Avec 3 équipes en parallèle, **9-10 mois wallclock** au lieu de 11
sprints séquentiels. Cf. §20 pour le staffing détaillé.

---

## §5. **Φ-A** — Hardening des Lettres A → D (D19-D20)

### §5.1 Objectif

Amener les Lettres A, B, C, D à **100 %** des critères Go/No-Go
définis dans leurs documents respectifs (`LETTER_*_ARCHITECTURE.md`).

### §5.2 Sprint **D19** — Lettres A + B

#### Lettre A — finalisation (3 jours)

* **DT-A-1** : exit codes distincts par famille d'erreur
  (`UsageError=1`, `CatalogError=4`, `NetworkError=5`, `TimeoutError=6`,
  `RejectedError=7`, `RateLimited=8`). Pin test `test_a_exit_codes_taxonomy`.
* **DT-A-2** : ajouter `schema_version: 1` dans le JSON `classify` output.
  Versioning contractuel. Pin test `test_a_output_carries_schema_version`.
* **DT-A-3** : inverser pretty/compact : compact par défaut,
  `--pretty` opt-in. Match docstring 1.4 du `LETTER_A` doc.
* **DT-A-4** : split `--verbose` → `--log-level {error|warn|info|debug}`
  + `--format {json|text|table}`.
* **DT-A-5** : ajouter `--text-file <path>` et `--stdin` (mutually
  exclusive avec text positional).
* **DT-A-6** : alias `efe` (cosmétique, `[project.scripts]`).
* **DT-A-7** : completions bash/zsh/fish via shtab.
* **DT-A-8** : lazy imports — torch + sentence-transformers chargés
  uniquement si la subcommand l'exige (cold start CLI < 500 ms).

**Tests à ajouter** : 8 pin tests dans `clients/python/tests/cli/`,
plus le `test_a_exit_codes_taxonomy_e2e` qui boote `efe-server` mock
+ exécute chaque code par fail injection.

#### Lettre B — finalisation (2 jours)

* **DT-B-1** : intégration readline Windows via `pyreadline3` extra
  (`efe-tool[repl-win]`).
* **DT-B-2** : tests pexpect : Ctrl-C pendant `:classify`, SIGTERM
  avec session sale, exception in :load (catalog corrompu).
* **DT-B-3** : plafonner historique à 1000 commandes max ; cache
  résultats `:last-result` borné à 100 entrées.
* **DT-B-4** : redaction automatique JWT/secrets dans logs `:debug`.
* **DT-B-5** : test automatisé `:help` ↔ `docs/repl.md` cohérence.

**Tests à ajouter** : 5 tests pexpect + 3 unit tests redaction.

### §5.3 Sprint **D20** — Lettres C + D + premier bundle

#### Lettre C — finalisation (3 jours)

* **DT-C-1** : 3 scénarios complets livrés en `clients/python/scenarios/` :
  `claude_billing_router.py`, `openai_intent_classifier.py`,
  `mock_a_b_decoder.py`. Chacun avec golden snapshot test.
* **DT-C-2** : retry exponentiel 3× avec backoff jitter sur 429 + 503.
  Abandon clean au 3e échec avec `RateLimitError`.
* **DT-C-3** : transcript JSON versionné `schema_version: 1` avec
  champs `total_cost_usd`, `llm_calls`, `efe_classifies`, `verdicts`.
* **DT-C-4** : cost ceiling enforcement — `--max-usd 0.10` abort à
  l'atteinte.
* **DT-C-5** : tests golden all-mock (sans API live) — déterministes.

#### Lettre D — finalisation (2 jours)

* **DT-D-1** : type public `Verdict` (`@dataclass(frozen=True)` avec
  `label: str`, `confidence: float`, `cone_id: str`,
  `rounds_executed: int`, `swarm_id: str`).
* **DT-D-2** : façade unifiée `efe_tool.EfeFacade` API stable :
  `classify(text) -> Verdict`, `classify_async(text) -> Awaitable[Verdict]`,
  `health() -> HealthStatus`, `close() -> None`.
* **DT-D-3** : OpenTelemetry tracing automatique (spans pour
  `preload`, `classify`, `cache_hit/miss`).
* **DT-D-4** : metrics Prometheus auto-exposées si `prometheus_client`
  installé : `efe_facade_classify_seconds`, `efe_facade_cache_hit_total`.
* **DT-D-5** : fixtures partagées : `MockEfeFacade`, `FakeCatalog`
  pour tests unitaires des apps utilisatrices.

#### Premier bundle "réel" tronc commun (2 jours)

* **DT-PC-1** : publier `bundles/all-MiniLM-L6-v2-en-1.0.0/` sample
  bundle dans `bundles/` du repo (poids quantizés INT8 si possible
  pour limiter la taille à ~30 MiB) signé Ed25519 par la CI release.
* **DT-PC-2** : doc `docs/bundles/sample.md` avec hash SHA-256 +
  signature attendus + reproductibilité bit-à-bit.

### §5.4 Critères Go/No-Go Φ-A

* GG-Φ-A.1 : Letters A-D 100 % critères de leurs LETTER_*_ARCHITECTURE.
* GG-Φ-A.2 : 1 sample bundle réel dans `bundles/` signé.
* GG-Φ-A.3 : 8 + 5 + 5 + 5 = 23 nouveaux pin tests (DT-A/B/C/D).
* GG-Φ-A.4 : Cargo workspace + Python pin tests GREEN.
* GG-Φ-A.5 : v0.2.0 release tag + CHANGELOG entry.

---

## §6. **Φ-B** — Lettre E + multi-runtime SDKs (D21-D24)

### §6.1 Objectif

Livrer la **Lettre E single-tenant production** + premiers SDKs
non-Python (Go, TypeScript). E reste single-tenant à la fin de Φ-B ;
le multi-tenant complet n'arrive qu'en D29 (post-D26 hardening).

### §6.2 Sprint **D21** — Dockerfile prod + K8s manifests + AdminService

#### Dockerfile production (2 jours)

* Multi-stage build : builder (rust:1.85 + cargo-chef cache) →
  runtime (gcr.io/distroless/cc:nonroot, ~50 MiB image).
* **CIS Docker benchmark compliance** : non-root user, no shell, no
  package manager, read-only root FS.
* SBOM generated by `syft` injecté dans `/usr/share/sbom.json`.
* `cosign attest` automatique en CI.

#### K8s manifests complets (2 jours)

* `Deployment` (déjà D9 + D18, finitions).
* `Service` headless (D18 ✅).
* `HorizontalPodAutoscaler` (D18 ✅).
* `PodDisruptionBudget` (D18 ✅).
* `NetworkPolicy` strict — tenant ingress + monitoring egress only.
* `ServiceMonitor` Prometheus (D13 ✅).
* `PodSecurityPolicy` / `SecurityContext` non-root + readOnlyRootFS
  + drop ALL capabilities + seccomp default.

#### AdminService gRPC complet (4 jours)

Proto `efe.admin.v1.AdminService` (cf. `LETTER_E_ARCHITECTURE.md` §5.2) :

```protobuf
service AdminService {
  rpc RegisterCatalog(RegisterCatalogRequest)    returns (RegisterCatalogResponse);
  rpc ListCatalogs(ListCatalogsRequest)          returns (ListCatalogsResponse);
  rpc GetCatalog(GetCatalogRequest)              returns (GetCatalogResponse);
  rpc DeleteCatalog(DeleteCatalogRequest)        returns (DeleteCatalogResponse);
  rpc ReloadCatalog(ReloadCatalogRequest)        returns (ReloadCatalogResponse);
  rpc PinCatalog(PinCatalogRequest)              returns (PinCatalogResponse);
  rpc EvolveCatalog(EvolveCatalogRequest)        returns (EvolveCatalogResponse);
  rpc ListAuditEvents(ListAuditEventsRequest)    returns (ListAuditEventsResponse);
}
```

* Le D12-bis a déjà livré le `CatalogService` ; AdminService est
  son équivalent **admin-scoped** avec auth-scope élevée
  (`scope: "admin"` requis dans le JWT).
* Pas de modification du data plane — AdminService est un overlay.

### §6.3 Sprint **D22** — mTLS + JWT scopes + rate-limit per-tenant

* **mTLS obligatoire** en E (TLS 1.3, cert-manager 30j rotation).
  Refus de connexion non-mTLS au boot.
* **JWT 3 scopes** : `data` (Mode1Service), `admin` (AdminService),
  `audit` (read-only events).
* **Rate-limit per-tenant** : déjà D18 ✅, mais ajout du contract
  `Quota` exposé via AdminService.
* **DT-PC-3** : ajout `efe_admin_api_calls_total{rpc, scope, outcome}`
  Prometheus metric.

### §6.4 Sprint **D23** — Go SDK

* Crate `clients/go/efe-sdk/` avec :
  * Client gRPC auto-généré depuis `protos/`.
  * Helper `EfeClient.Classify(ctx, text) (*Verdict, error)`.
  * Lib OTel auto-instrumentation.
  * Tests integration vs mock server.
  * Doc `clients/go/README.md` + 3 exemples.
* **Versioning** : SemVer aligné avec produit (v0.3.0 par convention).
* **Publication** : module Go privé d'abord, public à GA.

### §6.5 Sprint **D24** — TypeScript SDK + Lettre E v1 freeze

* Package `clients/typescript/@efe/sdk/` avec :
  * Client gRPC via `@grpc/grpc-js` + `protobufjs`.
  * Browser-friendly subset (uniquement RPCs idempotents) via
    `grpc-web`.
  * Types TypeScript exportés (Verdict, Catalog, Cone, …).
  * Tests Vitest + Playwright e2e.
* **Lettre E single-tenant freeze** : tag `v0.3.0`. Tous critères
  GG-E.x du `LETTER_E_ARCHITECTURE.md` sauf multi-tenant strict.

### §6.6 Critères Go/No-Go Φ-B

* GG-Φ-B.1 : Image Docker `efe-server:v0.3.0` produite par CI signée.
* GG-Φ-B.2 : K8s manifests valident `kubeval` + `polaris` GREEN.
* GG-Φ-B.3 : `helm install` + `helm test` GREEN sur kind cluster.
* GG-Φ-B.4 : AdminService 8 RPCs GREEN + audit logged.
* GG-Φ-B.5 : Go SDK + TS SDK published, smoke tests GREEN.
* GG-Φ-B.6 : v0.3.0 tag + CHANGELOG.

---

## §7. **Φ-C** — Bundle modèle réel + PyO3 zero-copy + Java SDK (D25-D27)

### §7.1 Objectif

Amener un **vrai modèle d'embedding** dans le pipeline + livrer la
liaison **Rust ↔ Python zero-copy** + ajouter le SDK Java.

### §7.2 Sprint **D25** — PyO3 FFI complet

#### Implémentation `efe-ffi` PyO3 (5 jours)

```rust
// crates/efe-ffi/src/lib.rs
#[pymodule]
fn _native(_py: Python<'_>, m: &PyModule) -> PyResult<()> {
    m.add_class::<PySwarmController>()?;
    m.add_class::<PyVerdict>()?;
    m.add_class::<PyEpistemicLatentSpace>()?;
    m.add_function(wrap_pyfunction!(classify_zero_copy, m)?)?;
    m.add_function(wrap_pyfunction!(load_bundle, m)?)?;
    Ok(())
}
```

* `PySwarmController` : wrap `SwarmController` Rust avec ownership
  Python (drop ↔ Rust drop).
* `PyVerdict` : zero-copy view sur les bytes du verdict (centroïde +
  metadata).
* `PyEpistemicLatentSpace` : exposed comme `numpy.ndarray` zero-copy
  via `PyArray2<f32>`.
* `classify_zero_copy(text: str, catalog_id: str) -> Verdict` :
  bypass complet du gRPC stack pour le SDK in-process.

#### Migration `clients/python/efe_tool/client.py` (3 jours)

* Garder la classe `EfeClient` actuelle (gRPC-based, multi-process).
* Ajouter une classe `EfeNativeClient` qui utilise `efe-ffi` via PyO3.
* Bench `tests/bench/native_vs_grpc.py` : démontrer ≥ 5× speedup +
  zero-copy memory profile.

#### Wheel publishing (2 jours)

* Pipeline cibblio (manylinux2014 + macOS arm64 + macOS x86_64) avec
  `maturin` → `efe-py` wheel sur PyPI privé.
* Test install vide + smoke `python -c "from efe import classify; print(classify('hello', 'router-v1'))"`.

### §7.3 Sprint **D26** — Bundle réel + Java SDK

#### Bundle réel `paraphrase-multilingual-MiniLM` (5 jours)

* Pipeline `efe-bundle pack ./inputs/paraphrase-multilingual-MiniLM-L12-v2/`
  → `bundles/paraphrase-multilingual-MiniLM-L12-v2-1.0.0.efe`.
* Quantization INT8 (Olive ou ONNX Runtime tools) → ~80 MiB.
* SHA-256 manifests + Ed25519 signature.
* SBOM CycloneDX généré par `syft`.
* Hash check on load contre `bundles/manifests/<bundle_id>.expected.sha256`.

#### Java SDK (3 jours)

* `clients/java/efe-sdk/` Maven module avec :
  * gRPC client `io.grpc:grpc-netty-shaded`.
  * Maven Central publishing pipeline.
  * 3 exemples Spring Boot intégration.
  * JUnit5 tests integration vs mock server.

#### Bundle smoke benchmark (2 jours)

* `efe-bench run --workload balanced --bundle paraphrase-multilingual --target-env staging`
* Validate : p99 < 800 ms (CPU). Marquer SLO-PC.1 GREEN.

### §7.4 Sprint **D27** — Multi-bundle live reload

#### Live bundle reload (5 jours)

* Le `EmbedderRegistry` actuel charge le bundle au boot. Étendre avec
  `reload_bundle(new_path: PathBuf)` qui :
  * Charge le nouveau bundle dans un slot temporaire.
  * Vérifie hash + signature + smoke encode 10 phrases.
  * Atomic-swap le pointeur `Arc<Bundle>`.
  * Drain l'ancien bundle (Arc count → 0 quand RPCs en flight terminent).
* Trigger via `AdminService.ReloadBundle` (auth scope `admin`).

#### Pipeline CI auto-bundling (3 jours)

* GitHub Actions workflow `.github/workflows/bundle-release.yml`.
* Inputs : tag `bundle-vX.Y.Z`.
* Actions : pack + sign + sbom + upload to private S3 + cosign attest.
* Tests : reproducibilité bit-à-bit entre 2 runs.

#### Bundle distribution (2 jours)

* Endpoint OCI Artifacts `oci://registry.efe.ai/bundles/<id>:<version>`.
* Pull policy : opt-in via `EFE_BUNDLE_OCI_REF` env var.
* Auth : OIDC delegation depuis le `efe-server` ServiceAccount.

### §7.5 Critères Go/No-Go Φ-C

* GG-Φ-C.1 : `efe-py` wheel installable depuis PyPI privé.
* GG-Φ-C.2 : Bench `native_vs_grpc.py` ≥ 5× speedup démontré.
* GG-Φ-C.3 : 1 bundle réel `paraphrase-multilingual-MiniLM` produit
  + signé + smoke benchmark GREEN.
* GG-Φ-C.4 : Java SDK Maven Central pipeline GREEN.
* GG-Φ-C.5 : Live bundle reload GREEN sur kind cluster.
* GG-Φ-C.6 : v0.4.0 tag + CHANGELOG.

---

## §8. **Φ-D** — CUDA kernels + GPU runtime + multi-GPU (D28-D32)

### §8.1 Objectif

Remplacer le `MvpCpuFusion` par un **vrai runtime GPU CUDA** avec speedup
≥ 5× sur p99, multi-GPU sharding pour les essaims N > 256, et passe le
SLO C-1 (Mode-1 GPU p99 ≤ 200 ms).

### §8.2 Sprint **D28** — CUDA kernels MVP (single-GPU)

#### Kernels CUDA (8 jours)

Les 7 kernels CPU livrés en `crates/efe-core/src/fusion/kernels/`
sont portés en CUDA :

| Kernel | Implémentation CUDA | cuBLAS / custom |
|--------|---------------------|-----------------|
| `attention.cu` | `cublasSgemm` (matmul Q × Kᵀ) | cuBLAS |
| `softmax.cu` | Custom kernel, max-shift, warp-reduce | custom |
| `aggregate.cu` | `cublasSgemm` (scores × latent) | cuBLAS |
| `entropy.cu` | Custom warp-reduce | custom |
| `project.cu` | Custom (cone projection) | custom |
| `clip.cu` | Custom (vec ops) | custom |
| `mix.cu` | Custom (vec ops + scalar β) | custom |

* Bindings via `cudarc` crate (sécure, statiquement liée à libcuda).
* Build : opt-in `cargo build --features cuda` (Linux + arm64 jetson).
* Fallback automatique au `MvpCpuFusion` si pas de GPU détecté.

#### Choix runtime CUDA (2 jours)

* `cuda 12.4` (pinné par projet).
* `cuDNN 8.9` (optionnel, utilisé par ONNX EP).
* `cudarc 0.10+` (pinné dans Cargo.toml).
* `nvcc` au build time (CI runner avec GPU mock).

### §8.3 Sprint **D29** — CUDA fusion en path principal

#### Builder selection (3 jours)

```rust
// crates/efe-server/src/mode1.rs
let fusion: Box<dyn FusionStrategy> = if cfg.runtime.gpu_enabled
    && cudarc::driver::CudaDevice::new(0).is_ok()
{
    Box::new(CudaFusion::new(cfg.gpu)?)
} else {
    Box::new(MvpCpuFusion::new(cfg.cpu)?)
};
```

* Env `EFE_RUNTIME_GPU_ENABLED=true` (default `false` pour back-compat).
* Détection CUDA device au boot ; warn si requested mais absent.

#### Tests integration GPU (4 jours)

* `crates/efe-server/tests/d28_cuda_fusion_integration.rs` :
  * Smoke : execute 1 swarm CPU + 1 swarm GPU, asserter résultats
    identiques à 1e-3 (numerical equivalence).
  * Bench : 100 essaims, mesurer speedup ; gate à ≥ 5×.
  * Memory leak : 1000 swarms, asserter no leak via `cuda-memcheck`.

#### CI matrix (3 jours)

* Runner GitHub Actions `[self-hosted, gpu]` avec NVIDIA L4 ou T4.
* Workflow `.github/workflows/test-gpu.yml` runs nightly + on-demand.

### §8.4 Sprint **D30** — Multi-GPU sharding

#### Architecture sharding (3 jours)

Pour N > 256 (limite single-GPU mémoire), partitioner l'ELS en
**sub-swarms** :

```
ELS [N=1024 × D=512]
  ↓ shard
GPU-0: ELS_a [256 × D]
GPU-1: ELS_b [256 × D]
GPU-2: ELS_c [256 × D]
GPU-3: ELS_d [256 × D]
```

Chaque GPU exécute fusion locale ; **all-reduce** entre rounds via
NVLink ou PCIe. Bench attendu ≥ 3× sur N=1024 vs 1 GPU.

#### Implementation (5 jours)

* `MultiGpuFusion` : trait impl qui délègue à N `CudaFusion` avec
  sync barriers entre rounds.
* Communication : `nccl 2.18` ou tcp socket fallback.
* Auto-detection number of GPUs au boot ; partition selon
  `EFE_RUNTIME_GPU_SHARD_SIZE=256` env var.

#### Test harness (2 jours)

* `tests/d30_multi_gpu_sharding.rs` smoke test sur 1 GPU avec sharding
  forced (logical sharding sur même device for testing).
* Bench `efe-bench --workload soak --gpu-shards 4` 24h soak — assert
  no memory leak, no convergence drift.

### §8.5 Sprint **D31** — Optimisations + perf budget

#### Profiling + optimisations (5 jours)

* Profiler avec `nvprof` / `nsight-compute` :
  * Identifier les kernels bottleneck (typiquement softmax + entropy).
  * Fuse opérations (softmax + entropy → 1 kernel custom).
  * Tile sizes optimaux selon GPU type (T4 = 256, L4 = 512, A100 = 1024).

#### Bench budget (3 jours)

* Latence p99 par étape :
  * Embedder ONNX CUDA : ≤ 30 ms
  * Genesis : ≤ 5 ms
  * Fusion (10 rounds) : ≤ 100 ms (≤ 10 ms/round target)
  * Consensus + Dissolution : ≤ 2 ms
  * Alignment + projection : ≤ 5 ms
  * gRPC overhead + serialization : ≤ 50 ms
  * **Total p99 budget = 192 ms ≤ 200 ms (SLO C-1) ✅**

#### Tuning auto (2 jours)

* `efe-server tune` subcommand : runs benchmark, outputs optimal
  `EFE_RUNTIME_GPU_*` env vars pour le hardware détecté.

### §8.6 Sprint **D32** — Streaming RPC + Hybrid fusion

#### Streaming RPC (5 jours)

* Activer le stub `Mode1Service.ExecuteStreaming` :
  * Server-side streaming : 1 chunk par round.
  * Schéma `StreamChunk { round: uint32, partial_centroid: bytes,
    confidence: float, verdict: enum }`.
  * Use case : long-running essaims (Mode-3, soak workload).
* Test integration `tests/d32_streaming.rs`.

#### Hybrid fusion (5 jours)

* `HybridFusion` = combine CPU + GPU :
  * GPU run primary, CPU run shadow (validation).
  * Si divergence > seuil → log + alert (canary regression).
* Use case : nouvelles versions GPU kernels en canary mode.

### §8.7 Critères Go/No-Go Φ-D

* GG-Φ-D.1 : Mode-1 GPU p99 ≤ 200 ms (SLO C-1) sur charge 200 RPS, T4.
* GG-Φ-D.2 : speedup CPU → GPU ≥ 5× sur p99 mesuré par `efe-bench`.
* GG-Φ-D.3 : multi-GPU 4× shard speedup ≥ 3× sur N=1024.
* GG-Φ-D.4 : Streaming RPC end-to-end smoke GREEN.
* GG-Φ-D.5 : 24h soak GPU sans memleak/divergence.
* GG-Φ-D.6 : v0.6.0 tag + CHANGELOG.

---

## §9. **Φ-E** — Diversification nano-agents + Mode-2 (D33-D36)

### §9.1 Objectif

Étendre l'**hétérogénéité de l'essaim** au-delà des 3 templates
actuels (NaSem/NaLog/NaCrit) + livrer **Mode-2** (anchors générées
dynamiquement par un LLM de routage à chaque requête).

### §9.2 Sprint **D33** — 5 nouveaux templates de nano-agents

Cf. §25 (annexe taxonomie) pour la justification de chaque template.
Les 5 nouveaux types ajoutés à la palette :

| Template | Code | Rôle |
|----------|------|------|
| `NaQual` | NQ | Qualité de l'évidence (consensus weight) |
| `NaCausal` | NC | Détection causalité vs corrélation |
| `NaTemp` | NT | Temporalité / urgence |
| `NaSafety` | NSF | Détection contenu sensible / refus |
| `NaConsist` | NCS | Cohérence intra-document (multi-paragraphe) |

#### Implémentation (8 jours)

* `crates/efe-core/src/agents/templates/` étendre l'enum :
  ```rust
  pub enum TemplateCode {
      NaSem, NaLog, NaCrit,                    // existants
      NaQual, NaCausal, NaTemp, NaSafety, NaConsist,  // nouveaux
  }
  ```
* Chaque template a son `prior_embedding` initialisé selon une
  stratégie spécifique (cf. §12 pour le détail).
* `TemplateRegistry` mis à jour pour gérer les 8 codes.
* Architecture : chaque essaim peut utiliser un sous-ensemble
  configurable via `SwarmSpec.template_mix: HashMap<TemplateCode, f32>`.

#### Tests + bench (2 jours)

* `tests/d33_template_diversity.rs` : asserter que les 8 templates
  ont des priors distincts (cosine < 0.8 entre paires).
* Bench accuracy : démontrer +2-5 % accuracy sur eval set vs mix
  3-template (NaSem/NaLog/NaCrit only).

### §9.3 Sprint **D34** — Mode-2 architecture + LLM routage

#### Mode-2 architecture (5 jours)

```
client → gRPC Mode2.Execute(text, llm_routing_config)
   ↓
LlmRouter::generate_anchors(text, top_k=5)
   → 5 anchor candidates (texte → embedding via bundle)
   ↓
SwarmController::execute(SwarmSpec {
    mode: Mode2,
    anchors: 5_dynamic_anchors,
    ...
})
   ↓ (idem Mode-1 à partir d'ici)
verdict
```

* Le LLM call est **fait avant** le swarm (bypass rate-limit, audit).
* Latence + p99 ≤ 1500 ms (SLO C-3) car +1 LLM call.
* Cost +1 LLM call par requête (typiquement +0.2 ¢).

#### LlmRouter trait (3 jours)

```rust
#[async_trait]
pub trait LlmRouter: Send + Sync {
    async fn generate_anchors(
        &self,
        query_text: &str,
        top_k: u32,
    ) -> Result<Vec<DynamicAnchor>, LlmRouterError>;
}
```

Impls : `OpenAIRouter`, `AnthropicRouter`, `MockRouter`. Stub pour
v0.7.0, real impls en D35.

#### Proto + RPC (2 jours)

```protobuf
service Mode2Service {
  rpc Execute(Mode2ExecuteRequest) returns (Mode2ExecuteResponse);
  rpc ExecuteStreaming(Mode2ExecuteRequest) returns (stream Mode2StreamChunk);
}

message Mode2ExecuteRequest {
  string query_text = 1;
  string llm_router_id = 2;  // identifies routing config
  uint32 top_k_anchors = 3;
  // ... idem Mode1
}
```

### §9.4 Sprint **D35** — Mode-2 LLM routers réels

* OpenAI router (3 jours) — async client, retry exponentiel,
  rate-limit awareness, cost accounting.
* Anthropic router (3 jours) — idem.
* Tests integration vs mock LLM (4 jours).

### §9.5 Sprint **D36** — Mode-2 stabilisation

* Bench Mode-2 accuracy + latence vs Mode-1 (3 jours).
* Audit Mode-2 : sécurité (LLM prompt injection ?), cost (≤ 0.5 ¢ ?),
  determinism (caching anchors par hash request) (4 jours).
* Documentation `docs/MODE2_ARCHITECTURE.md` (3 jours).

### §9.6 Critères Go/No-Go Φ-E

* GG-Φ-E.1 : 8 templates fonctionnels, accuracy +2-5 % vs 3-template.
* GG-Φ-E.2 : Mode-2 p99 ≤ 1500 ms (SLO C-3).
* GG-Φ-E.3 : LLM router OpenAI + Anthropic + mock GREEN.
* GG-Φ-E.4 : Audit Mode-2 0 critical findings.
* GG-Φ-E.5 : v0.7.0 tag + CHANGELOG + docs.

---

## §10. **Φ-F** — Mode-3 multi-turn reasoning (D37-D39)

### §10.1 Objectif

Mode-3 = **chaîner k Mode-1/Mode-2 calls en parallèle** avec un
**aggregator** qui produit un verdict final. Pattern self-consistency
mais en latent space (vs voting textuel des LLMs).

### §10.2 Architecture Mode-3

```
client → gRPC Mode3.Execute(text, k=5, mode_per_call=Mode1)
   ↓
Spawn k swarms in parallel (k Mode1 calls, different seeds)
   ↓
Each yields a centroid + confidence
   ↓
Mode3Aggregator::aggregate(k_centroids, weights)
   → final_centroid + final_confidence
   ↓
Alignment + verdict
```

### §10.3 Sprint **D37** — Mode-3 architecture + aggregator

* `Mode3Service.Execute` proto (2 jours).
* `Mode3Aggregator` trait + 3 impls :
  `MeanAggregator`, `WeightedAggregator`, `MedianAggregator` (4 jours).
* Tests : k=5, démontrer accuracy +1-3 % vs single Mode-1 (4 jours).

### §10.4 Sprint **D38** — Parallélisation + budget

* Tokio JoinSet pour spawn k swarms parallèles (3 jours).
* Budget enforcement : Mode-3 p99 ≤ 3 × Mode-1 p99 (4 jours).
* Streaming : 1 chunk par swarm complet (3 jours).

### §10.5 Sprint **D39** — Mode-3 stabilisation

* Bench Mode-3 vs Mode-1 vs Mode-2 (3 jours).
* Audit Mode-3 : determinism, cost, latence (4 jours).
* Documentation `docs/MODE3_ARCHITECTURE.md` (3 jours).

### §10.6 Critères Go/No-Go Φ-F

* GG-Φ-F.1 : Mode-3 p99 ≤ 3× Mode-1 p99 (SLO C-4).
* GG-Φ-F.2 : Accuracy +1-3 % vs Mode-1 sur eval set.
* GG-Φ-F.3 : Streaming GREEN.
* GG-Φ-F.4 : v0.8.0 tag + CHANGELOG + docs.

---

## §11. **Φ-G** — Compliance attestation + freeze GA (D40-D42)

### §11.1 Objectif

Atteindre **toutes** les attestations compliance + figer la version
v1.0.0 GA.

### §11.2 Sprint **D40** — DR drill automation + SBOM finale

#### DR drill automatique quarterly (5 jours)

* Helm `CronJob` `efe-dr-drill-quarterly` (cf. D17 architecture
  § 22 "DR drill hebdomadaire" — étendu en quarterly pour prod).
* Script `dr/run_quarterly_drill.sh` :
  1. Snapshot prod (read replica).
  2. Restore in isolated namespace `efe-dr-drill`.
  3. Smoke test (10 classifies).
  4. Compare outputs avec snapshot canonique.
  5. Tear down namespace.
  6. Email report to ops + compliance.
* Pin test `tests/test_dr_drill_runbook.py` qui vérifie les 6 étapes.

#### SBOM pipeline finale (3 jours)

* SBOM CycloneDX généré par `syft` pour chaque image Docker.
* Cosign attest chaque SBOM.
* Endpoint `https://efe.ai/sbom/<image-digest>.json` public.

#### ISO 27001 mapping (2 jours)

* Document `docs/compliance/iso27001.md` mappant chaque control
  ISO 27001 Annex A à un module EFE.

### §11.3 Sprint **D41** — SOC2 + GDPR + AI Act

#### SOC2 Type II preparation (4 jours)

* Évidence pack : audit logs, dr drills, vuln scans, access reviews.
* Document `docs/compliance/soc2_evidence.md` avec 8 controls
  (CC1-CC9) + lien vers évidence concrète.
* Tier 1 audit run (auditor mock). Pin test
  `test_soc2_evidence_pack_complete.py`.

#### GDPR Article 32 attestation (3 jours)

* Document `docs/compliance/gdpr_art32.md` avec :
  * Pseudonymization (hash texte, redaction).
  * Encryption at rest + in transit.
  * Confidentiality + integrity guarantees.
  * Backup + DR (cross-ref D17).
  * Vulnerability management (cosign + SBOM).

#### AI Act Annex IV documentation (3 jours)

* Document `docs/compliance/ai_act_annex_iv.md` :
  * Description du système.
  * Performance accuracy + biais.
  * Risques résiduels.
  * Logging + traçabilité.
  * Human oversight.
  * Cybersécurité.
  * Dataset documentation.

### §11.4 Sprint **D42** — GA freeze + release v1.0.0

#### Final audit pass (3 jours)

* Cross-sprint regression GREEN sur tous les sprints D1 → D42.
* Audit chirurgical complet (pattern projet) — recherche défauts
  résiduels classes "wiring forgotten", "dead counter", "duplicate
  YAML key", "missing helm consume".

#### Release v1.0.0 (4 jours)

* `git tag v1.0.0 -s` (signed tag).
* Build artifacts : Docker image signed + SBOM + Helm chart
  packagé + 5 SDKs published.
* CHANGELOG 1.0 : récapitulatif complet de toutes les phases.
* Docs : `docs/RELEASE_NOTES_1.0.md`.
* Communication : email équipe + blog post.

#### Freeze + handoff (3 jours)

* Code freeze sur `main` ; nouvelles features partent sur `main-1.x`.
* Handoff doc équipe ops : runbook, on-call rota, escalation,
  incident playbooks.

### §11.5 Critères Go/No-Go GA 1.0

(Cf. §22 pour le détail complet.)

* GG-GA.1 — GG-GA.20 : les 20 contrats (§3.1) tous GREEN.
* GG-GA.21 : SOC2 Type II passé (auditor sign-off).
* GG-GA.22 : GDPR Art.32 attesté.
* GG-GA.23 : AI Act Annex IV livré.
* GG-GA.24 : Pen-test rapport (no critical/high findings unresolved).
* GG-GA.25 : DR drill quarterly attesté.

---

## §12. Architecture détaillée — **Le nano-agent**

### §12.1 Définition formelle

Un nano-agent est un triplet `(id, template, state)` où :

* `id : NanoAgentId = u32` — identifiant dense, unique au sein d'un essaim.
* `template : TemplateCode` — type fixé à la naissance (cf. §12.2).
* `state : Vec<f32>` ou `&mut [f32; D]` — vecteur ℝ^D dans l'ELS, mutable
  durant les rounds.

L'identité ID est **éphémère** : elle disparaît avec l'essaim. Elle
n'est PAS persistée dans la base et ne traverse PAS les requêtes.

### §12.2 Templates (3 livrés + 5 prévus en D33)

| Template | Code | Rôle | Prior init strategy |
|----------|------|------|---------------------|
| `NaSem` | NS | **Sémantique native** : représente le sens lexical | Centroïde des ancres du catalog actif (anchor-aware) |
| `NaLog` | NL | **Logique native** : invariants logiques (e.g. negation) | Vecteur orthogonal aux centroïdes catalog |
| `NaCrit` | NC | **Critique native** : détection contradictions / outliers | Vecteur aléatoire normalisé, seed fixé |
| **`NaQual`** | NQ | **Qualité de l'évidence** : poids sur la fiabilité | Distribution gaussienne autour du centroïde catalog avec σ adaptative |
| **`NaCausal`** | NCA | **Causalité** : distingue corrélation vs cause | Initialisé sur des anchors causales (ex. "à cause de", "donc", "implique") |
| **`NaTemp`** | NT | **Temporalité** : urgence + horizon | Initialisé sur anchors temporelles (ex. "maintenant", "demain", "dans 1 mois") |
| **`NaSafety`** | NSF | **Sécurité contenu** : détection contenu sensible | Initialisé sur anchors safety (refus, escalade, rouge) |
| **`NaConsist`** | NCS | **Cohérence multi-doc** : intra-paragraphe | Vecteur orthogonal au centroïde 1er-niveau, capture variance |

Chaque template définit :

```rust
pub trait Template {
    fn code(&self) -> TemplateCode;
    /// Initial vector for an agent of this template.
    fn prior_embedding(&self, ctx: &PriorContext) -> Result<Vec<f32>>;
    /// How this template participates in fusion (weight + bias).
    fn fusion_weight(&self) -> f32;
    /// Whether this template is anchor-aware (modulated by catalog).
    fn is_anchor_aware(&self) -> bool;
}
```

### §12.3 Cycle de vie d'un nano-agent

```
Naissance (Genesis)
   ├── id = next_id (u32)
   ├── template = sampled from SwarmSpec.template_mix
   ├── state = template.prior_embedding(ctx) + noise(σ)
   └── enregistré dans NanoAgentBatch

Vie (Fusion rounds 1..R)
   ├── round k: state ← attention-weighted update
   ├── NaSem agents glissent vers leur ancre proche
   ├── NaLog agents maintiennent contraintes logiques
   ├── NaCrit agents s'écartent (variance) puis convergent
   └── ... (autres templates par leur loi)

Décision (Consensus)
   ├── Le détecteur multi-critère évalue l'essaim
   └── Si Converged → break

Mort (Dissolution)
   ├── Centroïde extrait (passthrough en Mode-1, weighted en post-D33)
   ├── État libre ; pas de persistance
   └── Audit row écrite avec swarm_id + verdict (D14-bis)
```

### §12.4 Hétérogénéité d'un essaim

Un essaim N=128 typique (post-D33) :

```
SwarmSpec.template_mix = {
    NaSem:  0.50,  // 64 agents sémantiques
    NaLog:  0.15,  // 19 agents logiques
    NaCrit: 0.15,  // 19 agents critiques
    NaQual: 0.10,  // 13 agents qualité
    NaSafety: 0.10, // 13 agents safety
    // NaCausal/NaTemp/NaConsist : 0.0 par défaut, opt-in
}
```

Le mix est **paramétrable par catalog version** — différents
catalogs peuvent avoir différents mixes selon le domaine.

### §12.5 Agent scheduling (M10)

Le M10 scheduler est responsable de :

* **Allocation de l'arène mémoire** (M2) : `NanoAgentBatch` est un slab
  contigu pour cache-locality (un seul memcpy hot).
* **Scheduling des kernels** : ordonnance les phases fusion → consensus
  → dissolution dans le bon ordre, sans réordonnement.
* **Backpressure** : si charge > seuil, refuse nouveaux essaims avec
  `Status::ResourceExhausted`.

Post-D31 (Φ-D), le scheduler GPU :

* Choisit `CudaDevice` selon round-robin ou GPU avec moins de RPS.
* Gère le NCCL communicator pour multi-GPU sharding.
* Backpressure GPU mémoire (refuse essaims si VRAM > 80 %).

### §12.6 Determinism + reproductibility

Un essaim est **déterministe** si :

* Seed fixé (`SwarmSpec.seed: u64`).
* Bundle fixé (hash SHA-256 du manifest).
* Catalog version fixée (immuable).
* Kernels CPU/GPU même version.

Cette déterminism est **CRUCIALE** pour :

* Audit replay (ré-exécuter un verdict exact pour vérifier).
* DR drill (snapshot + restore yields exact match).
* A/B testing (comparer 2 catalog versions byte-à-byte).

Pin test institutionnel : `test_swarm_determinism_seed_fixed.rs`
asserte 100 swarms avec même seed donnent même output bit-à-bit.

### §12.7 Métriques nano-agent (Φ-A nouveau)

DT-PC-15 (à livrer en D34) — 4 nouveaux instruments
`crates/efe-telemetry/src/metrics/nano_agent_instruments.rs` :

| Métrique | Type | Labels |
|----------|------|--------|
| `efe_agent_template_distribution` | Gauge | `template_code` |
| `efe_agent_steps_total` | Counter | `template_code` |
| `efe_agent_outlier_total` | Counter | `template_code, reason` |
| `efe_agent_lifetime_seconds` | Histogram | `template_code` |

Cardinalité estimée : 8 templates × 3 outcomes ≤ 24 séries — sous le
budget D13.

---

## §13. Architecture détaillée — **L'essaim**

### §13.1 Pipeline complet (Mode-1)

```
T+0 ms  ┌─ gRPC Execute ────────────────────────────┐
         │                                          │
         │  AuthGuard.verify(token)                 │
         │  RateLimiter.check(tenant)               │
         │  IdempotencyStore.try_acquire(req_id)    │
         │                                          │
T+5 ms  ├─ Embedder.encode(text) ───────────────────┤
         │  → vec ℝ^D                               │
         │                                          │
T+30 ms ├─ Genesis.spawn(spec) ────────────────────┤
         │  → NanoAgentBatch (N agents)             │
         │  → ELS [N × D]                           │
         │                                          │
T+35 ms ├─ Fusion loop ─────────────────────────────┤
         │  for round in 1..=max_rounds:            │
         │    fusion.run_round(els, batch, round)   │
         │      → FusionStats                       │
         │    detector.evaluate(round, stats)       │
         │      → Verdict                           │
         │    if Converged break                    │
         │  (typical: 4-12 rounds, 8-12 ms each)    │
         │                                          │
T+135ms ├─ Dissolution ────────────────────────────┤
         │  centroid = dissolver.extract(els)       │
         │                                          │
T+137ms ├─ Alignment ──────────────────────────────┤
         │  projected = align.project(centroid, cone)│
         │  if breach: refuse                       │
         │                                          │
T+142ms ├─ Audit + response ───────────────────────┤
         │  audit_emitter.emit(...)                 │
         │  IdempotencyStore.store_response(req_id) │
         │  → ExecuteResponse                       │
         └──────────────────────────────────────────┘
```

### §13.2 Mode-1 vs Mode-2 vs Mode-3

| Aspect | Mode-1 | Mode-2 | Mode-3 |
|--------|--------|--------|--------|
| **Anchors** | Compilés (catalog) | Générés par LLM par requête | Idem M1 ou M2 |
| **LLM call** | 0 | 1 (routage) | 0 ou k (selon mode_per_call) |
| **k swarms** | 1 | 1 | k (parallèle) |
| **Latence p99** | ≤ 200 ms | ≤ 1500 ms | ≤ 600 ms (3× Mode-1) |
| **Cost** | ≤ 0.5 ¢ | ≤ 0.7 ¢ | ≤ k × 0.5 ¢ |
| **Use case** | Routing rapide | Domaines hors-catalog | Décisions critiques |
| **Sprint cible** | D1-D18 ✅ | D34-D36 | D37-D39 |

### §13.3 Cône d'alignement

Le cône (M9, D33-D36 etend) est une **safety net** :

```
cone = AlignmentCone {
    anchors: Vec<[f32; D]>,    // ex. les centroïdes catalog
    max_projection_distance: f32,  // ex. 0.4 (cosine)
}
```

Toute requête dont le centroïde final tombe **hors du cône** → refus
(verdict `OutOfScope`). Ceci garantit que l'essaim ne dérive pas
hors du domaine pour lequel il a été calibré.

### §13.4 Convergence — multi-critères (D6 ✅)

Le `MultiCriteriaDetector` combine 6 signaux :

1. `s1_variance_level` — variance baisse
2. `s2_entropy_drop` — attention entropy baisse
3. `s3_centroid_stability` — centroïde stable round-à-round
4. `s4_outlier_ratio` — fraction d'agents > 2σ
5. `s5_variance_trend` — Theil-Sen slope
6. `s6_no_anomaly` — pas de NaN/Inf

Confidence finale = agrégation pondérée des 6 signaux ; hysteresis
de 2 rounds avant `Converged`.

### §13.5 Mode collapse + recovery (audit-B8 ✅)

Si `s4_outlier_ratio < 0.05` ET `s1_variance_level < ε`, le swarm
est en **mode collapse** (tous les agents ont fusionné en 1 point).
Recovery :

1. Réinjecte du bruit (σ × 2 sur 20 % des agents).
2. Respawn 20 % d'agents avec template aléatoire.
3. Continue jusqu'à 2 rounds supplémentaires max.
4. Si toujours collapsed → verdict `ModeCollapse`, pas de réponse.

### §13.6 Streaming (Φ-D D32)

Format `StreamChunk` :

```protobuf
message StreamChunk {
  uint32 round = 1;
  bytes partial_centroid = 2;  // ℝ^D quantizé int8
  float confidence = 3;
  Verdict verdict = 4;  // Continue / Converged / ...
  FusionStats stats = 5;
}
```

Le client peut early-abort dès que `confidence > seuil` ou afficher
en temps réel l'évolution du swarm.

### §13.7 Hybrid fusion (Φ-D D32)

Use cases :

* **Canary GPU kernels** : run CPU shadow en parallèle, alert si
  divergence > 1e-3 (cosine).
* **Embedded fallback** : si GPU indispo en milieu de requête, basculer
  en CPU sans drop la requête.
* **Verdict ensemble** : moyenne pondérée du verdict CPU + GPU pour
  réduire variance.

Trait :

```rust
pub trait HybridFusion: FusionStrategy {
    fn primary(&self) -> &dyn FusionStrategy;
    fn shadow(&self) -> Option<&dyn FusionStrategy>;
    fn ensemble_strategy(&self) -> EnsembleStrategy;
}
```

### §13.8 Persistence swarm state (post-GA)

Hors-scope GA. Si un client veut "reprendre" un swarm interrompu, on
livre une `SwarmCheckpoint` opt-in en v1.1+ :

```rust
struct SwarmCheckpoint {
    swarm_id: Uuid,
    round: u32,
    els_snapshot: Vec<f32>,  // [N × D] flat
    template_codes: Vec<TemplateCode>,
    created_at: DateTime<Utc>,
}
```

Stockée dans `efe_swarm_checkpoints` table + serialized in S3 si
> 1 MiB. TTL 1h (RGPD compliance).

---

## §14. Extensions runtime

### §14.1 Streaming RPC (D32)

Cf. §13.6.

### §14.2 Hybrid fusion (D32)

Cf. §13.7.

### §14.3 Persistence (post-GA)

Cf. §13.8.

### §14.4 Webhook subscriptions (post-GA)

Pour clients qui veulent être notifiés de verdicts asynchrones
(typiquement Mode-3 long-running) :

* `Mode3.SubscribeVerdict(swarm_id, callback_url)` enregistre un
  webhook.
* À la dissolution, EFE POST le verdict signé HMAC à `callback_url`.

Hors-scope GA.

---

## §15. Surface d'intégration : SDK multi-runtime

### §15.1 Matrice SDK

| SDK | Sprint cible | Lib/transport | Use case principal |
|-----|:-----------:|---------------|--------------------|
| **Python** (gRPC) | livré ✅ | `grpc + protobuf` | scripts, REPL, integration FastAPI |
| **Python** (native) | D25 | `efe-ffi PyO3` | in-process, zero-copy |
| **Go** | D23 | `grpc-go` | services Go (auth, billing) |
| **TypeScript/JS** | D24 | `grpc-js` ou `grpc-web` | front-end / Node services |
| **Java** | D26 | `grpc-netty-shaded` | enterprise / Spring Boot |
| **Rust** | livré (interne) | `tonic` | other Rust services |

### §15.2 Standards inter-SDK

Chaque SDK expose la même API :

```
Client.classify(text, catalog_id) -> Verdict
Client.health() -> HealthStatus
Client.close() -> void
```

Plus une méthode `Client.classify_batch(texts, catalog_id)` pour
bulk (côté serveur, exécuté en parallèle via `tokio::spawn`).

### §15.3 Versioning SDK

* SDK SemVer aligné sur produit. v1.0.0 SDK = compat avec server v1.0.0.
* Compat policy : SDK v(M).x compat avec server v(M).y.
* SDK v0.x.x peut breaker entre patches (pre-1.0).

### §15.4 Authentification

Tous les SDKs supportent :

* mTLS (via cert files).
* OIDC token (Auth0, Okta, Azure AD, Keycloak).
* HS256 token (legacy, deprecated en v1.0).

### §15.5 Observability

Tous les SDKs auto-instrumentent :

* OpenTelemetry traces (span par classify).
* Metrics Prometheus (si exporter présent).
* Logs structurés (JSON, level configurable).

---

## §16. SLO produit final

### §16.1 SLOs front-of-house

| ID | Description | Cible | Mesure |
|----|-------------|-------|--------|
| **SLO-PC.1** | Mode-1 GPU latency | p99 ≤ 200 ms | `efe_swarm_duration_seconds_bucket` (charge 200 RPS) |
| **SLO-PC.2** | Mode-1 CPU latency | p99 ≤ 800 ms | `efe_swarm_duration_seconds_bucket` (charge 50 RPS) |
| **SLO-PC.3** | Mode-2 latency | p99 ≤ 1500 ms | idem |
| **SLO-PC.4** | Mode-3 latency | p99 ≤ 3 × Mode-1 p99 | idem |
| **SLO-PC.5** | Availability | ≥ 99.9 % monthly | `1 - rate(error_5xx)/rate(total)` |
| **SLO-PC.6** | Error rate | < 0.5 % | `rate(error_5xx)/rate(total)` |
| **SLO-PC.7** | Confidence calibration (Brier) | ≤ 0.10 | offline eval set |
| **SLO-PC.8** | Cost per classify | ≤ 0.5 ¢ Mode-1 | billing dashboard |

### §16.2 SLOs back-of-house

| ID | Description | Cible |
|----|-------------|-------|
| **SLO-PC.9** | RPO | ≤ 5 min |
| **SLO-PC.10** | RTO | ≤ 30 min |
| **SLO-PC.11** | Backup success | ≥ 99 % daily |
| **SLO-PC.12** | DR drill quarterly | 100 % run, ≥ 95 % pass |
| **SLO-PC.13** | Audit chain integrity | 100 % HMAC verify |
| **SLO-PC.14** | Audit chain latency | p95 sign+persist ≤ 50 ms |

### §16.3 SLOs developer experience

| ID | Description | Cible |
|----|-------------|-------|
| **SLO-PC.15** | Quickstart time | ≤ 5 min (cold install → first classify) |
| **SLO-PC.16** | SDK install size | ≤ 10 MiB (Python sans torch) |
| **SLO-PC.17** | Cold start CLI | ≤ 500 ms |
| **SLO-PC.18** | Doc completeness | 100 % public APIs documented |

---

## §17. Sécurité produit final

Cf. `M11_SECURITY_ARCHITECTURE.md` pour le baseline. Ajouts post-D18 :

* **Threat model formalisé** dans `docs/security/threat_model.md` (D40).
* **Pen-test rapport** by external party (D41).
* **Bug bounty program** opt-in (post-GA).
* **Supply chain integrity** : cosign verify + SBOM in CI.
* **Secrets rotation** : automatic 30j via cert-manager / vault.
* **Audit log integrity** : HMAC-chained (D14-bis ✅).
* **mTLS partout** : obligatoire en E (D22).

---

## §18. Observabilité produit final

### §18.1 Inventaire métriques cible (post-GA)

| Catégorie | # métriques | Sprint livraison |
|-----------|:----------:|------------------|
| Core (D1-D18) | 31 | livré ✅ |
| Mode-2 | 5 | D34-D36 |
| Mode-3 | 4 | D37-D39 |
| Nano-agent | 4 | D34 |
| GPU | 6 | D29 |
| **Total GA** | **50** | |

### §18.2 Dashboards

| Dashboard | Sprint | Description |
|-----------|:------:|-------------|
| EFE Overview | D13 ✅ | latence, RPS, error rate |
| Catalog Lifecycle | D13 ✅ | register/evolve/resolve |
| Audit Chain | D14-bis ✅ | sign rate, chain head |
| OIDC + Tenants | D15 ✅ | issuers, tenants active |
| Embedder Bundle | D16 ✅ | bundles loaded, encode rate |
| Backup + DR | D17 ✅ | RPO, RTO, drill outcomes |
| Multi-replica | D18 ✅ | leader, idem hits, ratelimit |
| **GPU runtime** | D29 | utilization, mem, kernel time |
| **Mode-2** | D34 | LLM router latency, cost |
| **Mode-3** | D37 | k-fold latency, aggregator stats |
| **Nano-agent** | D34 | template distribution, lifetime |

11 dashboards à GA.

### §18.3 Alerting

Cf. `deploy/helm/efe/templates/d18-prometheus-rules.yaml` pour
baseline. Ajouts GA : 12 nouvelles règles (3 par phase Φ-D/E/F/G).

---

## §19. Compliance et attestations

### §19.1 SOC2 Type II

* Period d'observation : 6 mois post-D40.
* Auditor : CPA firm tier 1 (TBD).
* Évidences : audit logs (D14-bis), DR drills (D17), access reviews
  (OIDC + tenants), vuln scans (cosign + SBOM), incident response
  (runbook), change management (CI/CD).

### §19.2 GDPR Article 32

* Pseudonymisation : hash texte requête, redaction PII en logs.
* Encryption at rest (Postgres TDE, S3 SSE) + in transit (mTLS).
* Backup/DR : D17 + D40.
* Vulnerability management : cosign + SBOM + dependabot.

### §19.3 AI Act Annex IV

* Description système : `docs/concepts/architecture.md`.
* Performance : eval set + Brier ≤ 0.10.
* Risques résiduels : `docs/security/risks.md`.
* Logging : audit chain (D14-bis), trace ID, swarm ID.
* Human oversight : `Refuse` verdict + escalation.
* Cybersecurity : M11 + threat model.
* Dataset documentation : bundle SBOM + catalog SBOM.

### §19.4 ISO 27001

* Mapping `docs/compliance/iso27001.md` (D40).
* Pas d'audit certifié pour GA (deferred v1.1).

### §19.5 NIST 800-53

* Mapping `docs/compliance/nist_800_53.md` (D40).
* Controls priority : AC, AU, IA, SC.

---

## §20. Plan d'équipe / staffing

### §20.1 Équipes proposées

| Équipe | Headcount | Phase principale | Compétences |
|--------|:---------:|------------------|-------------|
| **Core** | 2-3 | Φ-A, Φ-D, Φ-E, Φ-F | Rust, CUDA, ML, perf |
| **Ops** | 2 | Φ-B, Φ-G | K8s, helm, mTLS, compliance |
| **SDK** | 2 | Φ-A, Φ-C | Python, Go, TS, Java |
| **Sec** | 1 | Φ-G | SOC2, GDPR, AI Act |
| **PM/Doc** | 1 | toutes | docs, runbook, communication |

**Total** : 8-9 personnes équivalent temps plein.

### §20.2 Cadencement

* Sprint = 2 semaines.
* Daily standups par équipe.
* Cross-team sync 1×/semaine.
* Audit pass dédié (2 jours / sprint).
* Freeze week 1×/4 sprints.

### §20.3 Responsabilités RACI

(Annexe §24.4)

---

## §21. Risques transverses + atténuations

### §21.1 Risques top-10

| # | Risque | Probabilité | Impact | Atténuation |
|--:|--------|:----------:|:------:|-------------|
| R-1 | CUDA kernels p99 ne tient pas SLO C-1 | Médium | Critique | Buffer 2 sprints supplémentaires Φ-D ; option ONNX Runtime CUDA EP en fallback |
| R-2 | Mode-2 LLM router rate-limit chez fournisseur | Médium | Hard | Cache anchors par hash (TTL 5 min) ; multi-fournisseur (OpenAI + Anthropic) ; Mode-1 fallback |
| R-3 | Compliance auditor reject SOC2 evidence | Faible | Critique | Pre-audit interne D40 ; pre-audit external (consulting) D41 |
| R-4 | Bundle réel licensing issue (paraphrase model) | Médium | Hard | Modèle alternatif licensé MIT/Apache (e.g. all-MiniLM-L6-v2 EN-only) |
| R-5 | DR drill quarterly révèle data loss | Faible | Critique | DR drill mensuel pre-GA ; cross-region replication (post-GA) |
| R-6 | Multi-GPU NCCL deadlock en production | Faible | Hard | Soak test 24h Φ-D D31 ; fallback single-GPU automatique |
| R-7 | PyO3 ABI break entre Python versions | Médium | Médium | Pin Python 3.12 ; abi3 wheels (compat 3.10+) |
| R-8 | Mode-3 cost out of budget | Médium | Hard | Cap k ≤ 5 dur ; alert sur cost > 2 × budget |
| R-9 | Pen-test révèle vuln high | Faible | Hard | Pre-pen-test interne D40 ; bug-bounty post-GA |
| R-10 | Équipe core indisponible | Médium | Hard | Documentation continue ; pair-programming ; ADRs |

### §21.2 Risques par phase

* **Φ-A** : faible — mostly polish.
* **Φ-B** : médium — K8s + multi-runtime SDK = beaucoup de surface.
* **Φ-C** : médium — bundle real + PyO3 = 2 nouvelles techno simultanées.
* **Φ-D** : haut — CUDA + multi-GPU = perf-critical.
* **Φ-E** : médium — Mode-2 = LLM dependency.
* **Φ-F** : médium — Mode-3 = parallélisme.
* **Φ-G** : médium — compliance = humain-dépendant.

---

## §22. Critères Go/No-Go GA 1.0

### §22.1 Liste exhaustive

| ID | Critère | Mesure |
|---:|---------|--------|
| GG-GA.1 | C-1 Mode-1 GPU p99 ≤ 200 ms | bench load 200 RPS |
| GG-GA.2 | C-2 Mode-1 CPU p99 ≤ 800 ms | bench load 50 RPS |
| GG-GA.3 | C-3 Mode-2 p99 ≤ 1500 ms | bench load 50 RPS |
| GG-GA.4 | C-4 Mode-3 p99 ≤ 3 × Mode-1 | bench k=5 |
| GG-GA.5 | C-5 cost per classify ≤ 0.5 ¢ | dashboard billing |
| GG-GA.6 | C-6 token saved ≥ 90 % | bench vs LLM chain |
| GG-GA.7 | C-7 Brier ≤ 0.10 | eval set |
| GG-GA.8 | C-8 Catalog rollback < 5 min | DR drill |
| GG-GA.9 | C-9 Audit chain integrity | self-verify hourly GREEN |
| GG-GA.10 | C-10 RPO ≤ 5 min | DR drill |
| GG-GA.11 | C-11 RTO ≤ 30 min | DR drill |
| GG-GA.12 | C-12 Availability ≥ 99.9 % | 30 j observation |
| GG-GA.13 | C-13 mTLS in E | helm test |
| GG-GA.14 | C-14 OIDC in E | tenant onboard test |
| GG-GA.15 | C-15 Rate-limit per-tenant | bench multi-tenant |
| GG-GA.16 | C-16 Observabilité | 50+ instruments + 11 dashboards |
| GG-GA.17 | C-17 SBOM | cosign verify GREEN |
| GG-GA.18 | C-18 Compliance | SOC2 + GDPR + AI Act docs livrés |
| GG-GA.19 | C-19 SDKs | 5 SDKs published + smoke GREEN |
| GG-GA.20 | C-20 Doc | quickstart + runbook + API ref |
| GG-GA.21 | SOC2 Type II passé | auditor report |
| GG-GA.22 | GDPR Art.32 attesté | doc + DPO sign-off |
| GG-GA.23 | AI Act Annex IV livré | doc revue legal |
| GG-GA.24 | Pen-test 0 critical/high open | rapport |
| GG-GA.25 | DR drill quarterly attesté | last 4 quarters logs |

### §22.2 No-Go conditions

GA est **NO-GO** si une seule des conditions suivantes :

* Tout test workspace ou Python pin RED.
* Tout SLO PC RED en 7 derniers jours.
* Pen-test révèle critique non-mitigé.
* Compliance auditor refuse SOC2 evidence.
* DR drill 2 dernières fois échouées.

---

## §23. Versioning, freeze, deprecations

### §23.1 SemVer policy

* `MAJOR` (1.0 → 2.0) = break gRPC proto, break SDK API.
* `MINOR` (1.0 → 1.1) = nouvelles features compat ascendantes.
* `PATCH` (1.0.0 → 1.0.1) = bug-fix only, no new features.

### §23.2 Deprecation policy

* Annonce dans CHANGELOG MINOR avant.
* Deprecated 2 MINOR versions consecutive.
* Removed at next MAJOR.

### §23.3 Freeze GA

`v1.0.0` est **immutable**. Tout fix part en `v1.0.1` (patch
release). Nouvelles features partent en `v1.1.0` avec dev branch
`main-1.x`.

### §23.4 Roadmap post-GA (v1.1+)

* **v1.1** : multi-region, webhook subscriptions, SwarmCheckpoint.
* **v1.2** : ISO 27001 certified, fine-tuning embedder pipeline.
* **v2.0** : Mode-4 (en R&D — possibly online learning ?).

---

## §24. Annexes — matrices décisionnelles

### §24.1 Choix technologique CUDA vs OpenCL vs Metal

* **CUDA** retenu — écosystème mûr (cudarc, ONNX EP, NCCL).
* **OpenCL** rejeté — fragmentation, perf inférieure.
* **Metal** rejeté — Apple-only, pas de target serveur.
* **ROCm (AMD)** déféré post-GA (v1.2).

### §24.2 Choix LLM router

* **OpenAI gpt-4o-mini** : default (low cost, fast).
* **Anthropic claude-haiku** : alternative (lower latency parfois).
* **Mistral / Cohere** : déférés post-GA.

### §24.3 Choix bundle modèle réel

* **paraphrase-multilingual-MiniLM-L12-v2** : default (multilingual,
  ~120 MiB, MIT-like license).
* **all-MiniLM-L6-v2** : EN-only fallback (~80 MiB).
* **e5-small** : alternative (Microsoft, MIT).

### §24.4 RACI matrix par phase

| Activité | Core | Ops | SDK | Sec | PM |
|----------|:----:|:---:|:---:|:---:|:--:|
| Φ-A polish A-D | I | I | **R** | I | A |
| Φ-B Lettre E + SDKs | I | **R** | **R** | C | A |
| Φ-C bundle + PyO3 | C | I | **R** | I | A |
| Φ-D CUDA | **R** | I | I | I | A |
| Φ-E Mode-2 | **R** | I | C | C | A |
| Φ-F Mode-3 | **R** | I | C | I | A |
| Φ-G compliance | C | C | I | **R** | A |

R = Responsible, A = Accountable, C = Consulted, I = Informed.

---

## §25. Annexe — taxonomie complète des nano-agents (15 types proposés)

### §25.1 Templates "core" (existants ou D33)

| # | Code | Nom | Statut | Rôle | Initialization strategy |
|--:|------|-----|--------|------|-------------------------|
| 1 | NaSem | Sémantique native | ✅ | Sens lexical | Centroïde des ancres catalog (anchor-aware) |
| 2 | NaLog | Logique native | ✅ | Invariants logiques | Vecteur orthogonal aux centroïdes catalog |
| 3 | NaCrit | Critique native | ✅ | Détection contradictions | Aléatoire normalisé, seed fixé |
| 4 | NaQual | Qualité évidence | D33 | Poids fiabilité | Gaussienne autour centroïde, σ adaptative |
| 5 | NaCausal | Causalité | D33 | Cause vs corrélation | Anchors causales |
| 6 | NaTemp | Temporalité | D33 | Urgence + horizon | Anchors temporelles |
| 7 | NaSafety | Sécurité contenu | D33 | Refus / escalade | Anchors safety |
| 8 | NaConsist | Cohérence multi-doc | D33 | Variance intra-doc | Vecteur orthogonal centroïde 1er-niveau |

### §25.2 Templates "extension" (post-GA)

| # | Code | Nom | Statut | Rôle |
|--:|------|-----|:------:|------|
| 9 | NaSent | Sentiment | v1.1 | Polarité émotionnelle |
| 10 | NaToxic | Toxicité | v1.1 | Détection contenu toxique |
| 11 | NaIdent | Identité | v1.1 | Détection PII |
| 12 | NaLang | Langue | v1.1 | Détection langue + traduction |
| 13 | NaCode | Code | v1.2 | Détection langage de programmation |
| 14 | NaMath | Mathématique | v1.2 | Reasoning numérique |
| 15 | NaSpatial | Spatial | v2.0 | Reasoning géographique / multimodal |

### §25.3 Stratégies de mix

| Mix profile | Distribution | Use case |
|-------------|--------------|----------|
| `default` | NaSem 50, NaLog 25, NaCrit 25 | générique (post-D18) |
| `accuracy` | + NaQual 10, NaConsist 10 | high-stakes routing |
| `safety` | + NaSafety 20, NaToxic 10 | contenu user-facing |
| `multilingual` | + NaLang 15 | translation routing |
| `causal` | + NaCausal 15 | analyse cause-effet |

### §25.4 Cardinalité scaling

| Mix | N agents | D dim | Memory ELS | Compute / round (CPU) | / round (GPU T4) |
|-----|:--------:|:-----:|:----------:|:--------------------:|:----------------:|
| smoke | 32 | 128 | 16 KiB | 0.5 ms | 0.05 ms |
| default | 128 | 384 | 192 KiB | 4 ms | 0.4 ms |
| balanced | 256 | 512 | 512 KiB | 12 ms | 1.2 ms |
| stress | 1024 | 768 | 3 MiB | 80 ms | 8 ms |
| max | 4096 | 1024 | 16 MiB | n/a | 50 ms (multi-GPU) |

### §25.5 Hyper-paramètres de fusion par mix

| Mix | Temperature | β_anchor | Clip | Max rounds |
|-----|:----------:|:--------:|:----:|:----------:|
| default | 1.0 | 0.05 | 1.0 | 20 |
| accuracy | 0.7 | 0.10 | 1.0 | 30 |
| safety | 1.2 | 0.15 | 0.8 | 25 |
| fast | 1.5 | 0.02 | 1.0 | 10 |

---

## §26. Annexe — Gantt prévisionnel D19 → GA

```
                        D19 D20 D21 D22 D23 D24 D25 D26 D27 D28 D29 D30 D31 D32 D33 D34 D35 D36 D37 D38 D39 D40 D41 D42
Phase Φ-A (A-D polish)   ██  ██  
Phase Φ-B (Lettre E)             ██  ██  ██  ██
Phase Φ-C (bundle+PyO3)                          ██  ██  ██
Phase Φ-D (CUDA + GPU)                                       ██  ██  ██  ██  ██
Phase Φ-E (Mode-2 + 5 templates)                                                 ██  ██  ██  ██
Phase Φ-F (Mode-3)                                                                                ██  ██  ██
Phase Φ-G (compliance + GA)                                                                                       ██  ██  ██

Releases :
  v0.2.0 — D20
  v0.3.0 — D24
  v0.4.0 — D27
  v0.5.0 — D30
  v0.6.0 — D32
  v0.7.0 — D36
  v0.8.0 — D39
  v0.9.0 — D41
  v1.0.0 GA — D42  ▲
```

### §26.1 Découpage par mois (cible 11 mois wallclock)

| Mois | Sprints | Phase | Milestone |
|:----:|---------|-------|-----------|
| M1 | D19 | Φ-A | A polish |
| M1.5 | D20 | Φ-A | B/C/D polish + bundle real |
| M2 | D21 | Φ-B | Dockerfile + K8s |
| M2.5 | D22 | Φ-B | mTLS + JWT scopes |
| M3 | D23 | Φ-B | Go SDK |
| M3.5 | D24 | Φ-B | TS SDK + E v1 freeze |
| M4 | D25 | Φ-C | PyO3 |
| M4.5 | D26 | Φ-C | Bundle réel + Java SDK |
| M5 | D27 | Φ-C | Live reload |
| M5.5 | D28 | Φ-D | CUDA kernels MVP |
| M6 | D29 | Φ-D | CUDA fusion path |
| M6.5 | D30 | Φ-D | Multi-GPU sharding |
| M7 | D31 | Φ-D | Optimisations + perf budget |
| M7.5 | D32 | Φ-D | Streaming + Hybrid |
| M8 | D33 | Φ-E | 5 nouveaux templates |
| M8.5 | D34 | Φ-E | Mode-2 architecture |
| M9 | D35 | Φ-E | LLM routers réels |
| M9.5 | D36 | Φ-E | Mode-2 stabilisation |
| M10 | D37 | Φ-F | Mode-3 architecture |
| M10.5 | D38 | Φ-F | Parallélisation |
| M11 | D39 | Φ-F | Mode-3 stabilisation |
| M11 | D40 | Φ-G | DR + SBOM |
| M11.5 | D41 | Φ-G | SOC2 + GDPR + AI Act |
| M12 | **D42** | **Φ-G** | **GA 1.0.0** ▲ |

(11 mois wallclock avec 3 équipes en parallèle ; 12 mois si 2 équipes
seulement.)

### §26.2 Critical path

Le chemin critique passe par :

```
Φ-A (D19-D20) → Φ-B Lettre E (D21-D24) → Φ-C bundle (D26)
              → Φ-D CUDA (D28-D32) → Φ-E Mode-2 (D33-D36)
              → Φ-F Mode-3 (D37-D39) → Φ-G GA (D40-D42)
```

Aucune phase ne peut être skippée. Φ-D est le slot le plus risqué
(CUDA = nouvelle techno + perf-critical) — buffer 2 sprints prévu.

---

## §27. Mises à jour du document

Ce document est **vivant**. Toute découverte audit-time, défaut
résiduel, ou changement de scope doit déclencher une révision avec :

* Nouvelle entrée dans CHANGELOG.md (`docs/PRODUCT_COMPLETION_ARCHITECTURE.md`).
* Bump du Statut en haut.
* Pin test de garde dans `scripts/tests/test_product_completion_doc_drift.py`
  qui valide le sommaire + les SLOs cibles.

**Fin du document.**
