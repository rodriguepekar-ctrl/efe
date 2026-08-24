# EFE — Analyse chirurgicale et plan de finition pour le concept "véritables nano-agents"

> Audit du projet `/Users/rodrigue.pekar/Downloads/jjjj copie/efe` mis en relation avec le concept *Scout & Synthesize* (S&S) discuté dans [ARCHITECTURE.md](ARCHITECTURE.md) et [RESEARCH.md](RESEARCH.md).
>
> **Date** : 23 mai 2026

---

## 0. TL;DR — ce qu'il faut comprendre en 90 secondes

**EFE n'est pas un brouillon. C'est une plateforme nano-agent quasi-complète, plus avancée que tout ce que j'ai vu de comparable en open-source ou commercial.**

- **47 crates Rust, ~50 K LOC, 5 000+ tests passants, 0 failure** au dernier rapport
- **20 nano-agents starter pack déjà écrits** (10 Mode A pure Rust + 5 Mode B ML local + 5 Mode C LLM avec JSON schema strict)
- **Blackboard concurrent** avec CAS, versioning, audit log, cap 256 Ko — c'est la primitive de stigmergie que SwarmSys décrit théoriquement
- **HMAC audit chain + Merkle epochs + replay engine** — *non-retrofittable*, c'est le moat le plus défendable du projet
- **`efe-agentic-bridge`** branche déjà Claude avec streaming SSE + retry + provenance per-call
- **Phase 1-3 du sprint M50** essentiellement *terminée*

Ce qui manque pour activer le concept *"véritables nano-agents pour la recherche documentaire (S&S)"* tient en **3 chantiers bornés** :
1. **Wirings opérationnels** déjà identifiés dans l'investor brief (Firecracker, Postgres, Stripe, KYC, cosign)
2. **Câblage des vrais SLM locaux** (SmolLM2/Qwen2.5/Llama-3.2/Phi-3.5 via llama.cpp ou Candle) en remplacement des stubs Mode B/C
3. **L'application S&S elle-même** : un DAG préconçu + intelligence en essaim (phéromones, K-means clustering, spéculation frappe) qui *utilise* EFE comme plateforme

**Verdict** : EFE est la plateforme. S&S est une application qui tourne dessus. Le travail restant n'est pas de "compléter EFE" — il est de **monter S&S sur EFE** et de **finir les wirings prod**. Total estimé : 6–10 semaines avec un focus laser.

---

## 1. Inventaire surgical de ce qui EXISTE déjà dans EFE

### 1.1 Les 4 couches architecturales (toutes shippées et testées)

```
┌──────────────────────────────────────────────────────────────────────┐
│ COUCHE 4 — NANO AGENT SWARM (M50)                                    │
│  efe-nano-sdk + efe-nano + efe-nano-inference-daemon                 │
│  efe-nano-preanalysis (6 SwarmAgent réf.)                            │
│  efe-nano-swarm-adapter (NanoAgent ↔ SwarmAgent)                     │
│  efe-multi-region + efe-llm-provenance + efe-publisher-identity      │
│  efe-gold-test-gate + efe-blackboard + efe-swarm-orchestrator        │
│  efe-agentic-bridge (Claude tool-use + SSE)                          │
│  efe-gateway + efe-compliance-wiring                                  │
├──────────────────────────────────────────────────────────────────────┤
│ COUCHE 3 — AUDIT TRANSVERSE (HMAC chains + Merkle)                   │
│  audit::hmac_chain + audit::merkle + audit::verify + replay engine   │
├──────────────────────────────────────────────────────────────────────┤
│ COUCHE 2 — META-SWARM A1 (fédération inter-régions)                  │
│  efe-meta-swarm + efe-meta-swarm-proto + efe-meta-swarm-server       │
├──────────────────────────────────────────────────────────────────────┤
│ COUCHE 1 — INFÉRENCE LATENT-SPACE                                    │
│  efe-core + efe-kernels (CUDA) + efe-ffi + efe-server (gRPC)         │
└──────────────────────────────────────────────────────────────────────┘

+ M21–M33 : production hardening (prompts, llm-router, admin, saga,
  curation, active-learning, agent-runner, cache, events, pii,
  multimodal, region, metering, billing, vector, retrieval, kg, graphrag)
```

### 1.2 Les 20 nano-agents starter pack (`crates/efe-nano/src/starter_pack/`)

**Mode A — Pure Rust, 1–10 ms, déterminisme mathématique**

| Agent | Rôle |
|---|---|
| `json_validate` | Validation JSON schema |
| `xml_validate` | Validation XML |
| `regex_extract` | Extraction regex |
| `date_normalize` | Normalisation ISO 8601 |
| `currency_normalize` | Parsing devises |
| `csv_normalize` | Nettoyage CSV |
| `markdown_extract_headings` | H1–H6 extraction |
| `html_extract_main_content` | Extraction contenu principal |
| `pdf_extract_text` | Extraction texte PDF |
| `sql_parse_safe` | Parsing AST SQL safe |

**Mode B — ML local en Firecracker, 50–200 ms, déterminisme seeded**

| Agent | Rôle |
|---|---|
| `ner_clinical` | NER médical (médicaments, dosages, phases d'essai) |
| `ner_legal` | NER légal (parties, dates, montants) |
| `intent_classify_generic` | Classification d'intention |
| `lang_detect` | Détection de langue |
| `embed_text` | Embeddings de texte |

**Mode C — LLM + JSON schema strict, 500–2000 ms, replayable**

| Agent | Rôle |
|---|---|
| `summarize_strict` | Résumé schema-bound |
| `extract_structured` | Extraction structurée |
| `classify_topic_llm` | Classification topique |
| `sentiment_nuanced` | Sentiment nuancé |
| `qa_grounded` | QA grounded |

### 1.3 Les 6 SwarmAgents de pré-analyse (`crates/efe-nano-preanalysis/`)

Ceux-ci implémentent le **trait SwarmAgent** (différent du trait NanoAgent — c'est le pattern N23 d'adapter bidirectionnel) :

- `ner_extractor`
- `sentiment_classifier`
- `language_detector`
- `pii_redactor`
- `summary_writer`
- `schema_validator`

### 1.4 Maturité des tests (par sprint M50)

| Sprint | Test files trouvés | État |
|---|---|---|
| Phase 1 (Sprint 1–3) | `phase_1_integration.rs` | ✅ vert |
| Phase 2 N12 cache | `phase_2_n12_cache_hit.rs` | ✅ vert |
| Phase 2 N13 canary | `phase_2_n13_shadow_canary.rs` | ✅ vert |
| Phase 2 N14 envelope | `phase_2_n14_swarm_envelope.rs` | ✅ vert |
| Phase 2 Mode B | `phase_2_mode_b.rs` | ✅ vert |
| Phase 2 Mode C | `phase_2_mode_c.rs` | ✅ vert |
| Phase 2 HMAC | `phase_2_audit_chain.rs` | ✅ vert |
| Phase 2 Merkle | `phase_2_merkle_epochs.rs` | ✅ vert |
| Phase 2 GDPR | `phase_2_gdpr_tombstone.rs` | ✅ vert |
| Phase 2 Replay | `phase_2_replay.rs` | ✅ vert |
| Phase 3 N15 scheduling | `phase_3_n15_fairness_scheduling.rs` | ✅ vert |
| N21 swarm orchestration | `phase_6_n21_swarm_orchestration.rs` | ✅ vert |
| N23 adapter | `chaos_n23_nano_swarm_adapter.rs` + `audit_n23` | ✅ vert |
| N26 provenance wiring | `audit_n26_provenance_wiring.rs` | ✅ vert |
| N27 streaming retry | `phase_9_n27_streaming.rs` + chaos + audit | ✅ vert |

### 1.5 Ce qui est *réellement* stubbé (honnêteté de l'investor brief confirmée par lecture du code)

| Composant | État actuel | Pour passer en prod |
|---|---|---|
| Mode B VM pool | `StubVmPool` (in-process) | Firecracker microVMs sur fleet KVM |
| Catalog persistence | `InMemoryCatalog` | Postgres + migration v004-v007 prêtes |
| Chain store | `InMemoryChainStore` | Postgres backend |
| LLM client | `MockLlmClient` + `BuiltinLlmClient` | Provider réel via efe-llm-router (déjà câblé en partie) |
| Model loader Mode B | `InProcessModelLoader::with_phase_2_builtins` | Vrai chargement modèles (5 builtins stub) |
| Embeddings (`embed_text`) | Stub | bge-m3 ONNX via fastembed |
| Stripe billing | Code Rust écrit | Wiring vers vrai compte Stripe |
| KYC | Stubs permissifs | Onfido + OFAC |
| Cosign | Crypto code OK | Wiring KMS enterprise |
| ClickHouse aggregation | Mentionné dans archi, pas implémenté | À écrire (priorité basse, scale-driven) |

---

## 2. Comment EFE correspond exactement à notre concept S&S

Tableau de correspondance ligne à ligne avec [ARCHITECTURE.md §3](ARCHITECTURE.md) :

| Couche S&S | Composant EFE qui le couvre | État | Travail restant |
|---|---|---|---|
| **L0 Intent** (SmolLM2-360M, ~30 ms) | `intent_classify_generic` (Mode B) | 🟡 stub | Wirer SmolLM2-360M GGUF via llama.cpp |
| **L1 Plan** (Qwen2.5-0.5B, décomposition) | À écrire — bâti sur `extract_structured` (Mode C) | 🔴 manque | Écrire agent `retrieval_plan` (Mode C avec Qwen2.5-0.5B) |
| **L2 Scouts parallèles** | `Dispatcher` avec DAG + `max_parallelism` | ✅ | Composition uniquement |
| **L2 Embedding** | `embed_text` (Mode B) | 🟡 stub | Wirer bge-m3 ONNX |
| **L2 Retrieval** | `efe-vector` + `efe-retrieval` | ✅ | Compose avec LanceDB |
| **L2 Rerank** | À écrire | 🔴 manque | Nouvel agent Mode B `rerank_bge` |
| **L2 Capsule extract** | `extract_structured` + `summarize_strict` | ✅ | Wirer SLM réel + ajuster schemas pour ContextCapsule |
| **L3 Aggregator** | À écrire — orchestration via blackboard | 🟡 partial | Nouvel agent Mode B/C `aggregate_capsules` |
| **L4 Cloud LLM** (Claude streaming) | `efe-agentic-bridge::claude_stream` | ✅ | Aucun |
| **L5 Critique factuelle** | `qa_grounded` ou à customiser | 🟡 | Adapter `qa_grounded` ou nouvel agent `fact_check` |
| **Stigmergie phéromones** | `efe-blackboard` (versioning, audit) | ✅ très proche | Ajouter champ `pheromone: f32` + décroissance |
| **Cache L0 sémantique** | `efe-cache` (Redis) | ✅ | Composition |
| **Cache L4 prompt** | Pattern Anthropic SDK | 🟡 | Activer prompt caching dans agentic-bridge |
| **Spéculation frappe** | Aucun équivalent | 🔴 manque | Écrire dans `efe-gateway` (HTTP debounce + queue) |
| **Failover local** (Llama-3.2-1B si Claude down) | Aucun | 🔴 manque | Ajouter route alternative dans `agentic-bridge` |
| **Streaming SSE UI** | `efe-agentic-bridge::claude_stream` + `efe-gateway` | ✅ | Composition + ajouter événements `source`, `progress` |
| **Audit per-decision** | HMAC chain + Merkle + replay | ✅ **dépasse** S&S | Aucun |
| **Eval golden set** | `efe-gold-test-gate` | ✅ | Composer un golden set S&S |
| **Distillation LoRA** | À écrire | 🔴 manque | Pipeline `unsloth` externe, hors EFE |

**Verdict mapping** :
- ✅ **9 composants déjà en place** dans EFE (Dispatcher, retrieval, agentic-bridge, audit, cache, gateway, eval gate, etc.)
- 🟡 **7 composants stubbés** mais avec l'architecture en place (Mode B, embed_text, L4 caching, etc.)
- 🔴 **5 composants à écrire** (L1 plan, L2 rerank, L3 aggregator dédié, spéculation frappe, failover local, distillation)

Total à écrire pour l'application S&S : **5 nano-agents + 2 features gateway + 1 pipeline distillation** → ~3 000 LOC Rust + scripts Python.

---

## 3. Plan de finition en 3 phases (10 semaines)

### Phase A — Wiring opérationnel des stubs (4 semaines)

**Objectif** : faire passer EFE de "code complet et testé" à "déployable en prod avec un vrai client".

| Sem | Livrable | Crates touchés | Critère d'acceptation |
|---|---|---|---|
| 1 | Wire bge-m3 ONNX dans `embed_text` | `efe-nano`, nouveau `efe-embeddings` ou intégration in-crate | Mode B `embed_text` retourne 1024-dim réels en <50 ms |
| 1 | Wire SmolLM2-360M Q5 via llama.cpp dans `intent_classify_generic` | `efe-nano` | Latence <40 ms p99 |
| 2 | Wire Qwen2.5-0.5B Q5 dans Mode C runtime via vrai `LlmClient` (remplace `MockLlmClient`) | `efe-nano`, `efe-llm-router` | `summarize_strict` répond en <2 s avec JSON schema validé |
| 2 | Wire Llama-3.2-1B Q5 comme provider local dans `efe-llm-router` | `efe-llm-router` | Failover possible vers local |
| 3 | Postgres backend pour `Catalog` + `ChainStore` | `efe-nano`, migration v004-v007 | Tous les tests phase_2_* verts avec Postgres |
| 3 | Firecracker microVM réel (remplace `StubVmPool`) | `crates/efe-nano/src/runtime/pool/firecracker` | Mode B exécuté en VM isolée |
| 4 | Wire Stripe + KYC (Onfido) | `efe-billing`, `efe-publisher-identity` | Onboarding flow E2E vert sur stage |
| 4 | Wire cosign pipeline | `efe-publisher-identity` | Signature Ed25519 vérifiée par cosign |

**Effort** : 1 dev senior Rust + 1 dev senior infra, 4 semaines.

### Phase B — Application S&S sur EFE (4 semaines)

**Objectif** : transformer EFE en plateforme exécutant le DAG *Scout & Synthesize* pour la recherche documentaire.

| Sem | Livrable | Crates / fichiers | Critère |
|---|---|---|---|
| 5 | Nouveaux nano-agents starter S&S | `crates/efe-nano/src/starter_pack/` | 5 nouveaux agents `retrieval_plan` (C), `rerank_bge` (B), `aggregate_capsules` (B), `fact_check` (C), `intent_research` (B) |
| 5 | Type `ContextCapsule` dans SDK | `efe-nano-sdk` | Schema Pydantic-équivalent en Rust (serde) avec 3 niveaux de compression |
| 6 | Stigmergie phéromones | `efe-blackboard` extension | Champ `pheromone: f32`, méthode `decay(lambda)`, cron de décroissance λ=1/24h |
| 6 | DAG S&S préconçu | `crates/efe-nano/src/dispatcher/dag.rs` | DAG L0→L1→L2(×N)→L3→L4→L5 sérialisé en YAML |
| 7 | Spéculation pendant la frappe | `efe-gateway` | Endpoint `/v1/speculate` avec debounce 250 ms + cache d'embedding |
| 7 | Failover local Llama-3.2-1B | `efe-agentic-bridge` | Si Claude TTFB > 1.5 s, fallback automatique |
| 8 | Prompt caching Anthropic activé | `efe-agentic-bridge` | Section `system` + sources stables marquées cacheable |
| 8 | UI référence (Svelte ou React) | `apps/scout-and-synthesize-ui/` | Stream SSE consommé, capsules cliquables, critique surlignée |

**Effort** : 1 dev senior Rust + 1 frontend, 4 semaines.

### Phase C — Production & GTM (2 semaines)

**Objectif** : démos verticales + activation publisher ecosystem.

| Sem | Livrable | Critère |
|---|---|---|
| 9 | 3 démos verticales | Pharma (essais cliniques), Finance (KYC AML), Defense (sovereign docs) |
| 9 | Documentation publique | `docs.efe.ai` avec quickstart S&S |
| 10 | 1er publisher partner externe | Un cabinet juridique publie un nano-agent NER spécialisé sous Tier-1 |
| 10 | Pipeline de distillation | Script `scripts/distill_weekly.py` avec unsloth + LoRA hot-swap |

---

## 4. Risques nouveaux identifiés en lisant le code

Au-delà des risques listés dans l'investor brief :

### 4.1 Coût de complexité cognitive
**47 crates** est *énorme*. Un nouvel ingénieur prendra 4–6 semaines à comprendre la topologie. Conséquence : recruter prend plus de temps que prévu.
**Mitigation** : un `ONBOARDING.md` qui trace les flux de 3 use-cases canoniques bout-à-bout.

### 4.2 Drift architecture ↔ code
Les `M*_ARCHITECTURE.md` font 50–100 Ko chacun (~700 docs au total). Maintenir la cohérence entre les docs et le code shippé est une discipline coûteuse — déjà visible dans les renommages `M6_ARCHITECTURE.md` / `M06_ARCHITECTURE.md`.
**Mitigation** : script `scripts/arch-doc-coverage.sh` qui vérifie que chaque crate cite son architecture doc.

### 4.3 Le terme "nano-agent" est ambigu
Dans EFE, "nano-agent" = unité d'exécution Mode A/B/C atomique. Dans notre concept S&S, "nano-agent" = petit LLM spécialisé. **Ce ne sont pas exactement la même chose** :
- EFE nano-agent Mode A = pas un LLM du tout (Rust pur)
- EFE nano-agent Mode B = ML local (peut être un SLM)
- EFE nano-agent Mode C = LLM avec contraintes (peut être un SLM ou un gros LLM)

**Décision à acter** : dans la documentation publique, distinguer *EFE nano-agent* (l'unité d'exécution) du *small language model nano-agent* (le sous-ensemble Mode B/C utilisant un SLM).

### 4.4 Le moat audit-grade n'est utilisé que par 30 % du marché
L'investor brief table sur les industries régulées. Mais le concept S&S (recherche documentaire grand public) **n'a pas besoin** d'audit grade. Donc deux marchés distincts :
- **EFE-regulated** : pharma/finance/defense, audit-grade obligatoire, prix élevé
- **EFE-developer** : devs qui veulent juste un essaim de nano-agents rapide, audit lite

**Risque** : disperser l'énergie commerciale sur les deux marchés.
**Recommandation** : prioriser EFE-regulated pour les 12 prochains mois (le moat est là), exposer EFE-developer comme freemium pour générer du top-of-funnel.

### 4.5 La barre d'entrée pour un publisher externe est très haute
Ed25519 + KYC + 4 tiers + cooldown + cosign à TIER-audit : un publisher moyen sera intimidé. Le marketplace pourrait stagner.
**Mitigation** : un **starter tier "0+"** ultra-simple (sans KYC, sans signature, mais sans usage commercial autorisé) pour faciliter la contribution open-source.

---

## 5. Lien direct avec ce qu'on a écrit dans vrai/

Les documents [ARCHITECTURE.md](ARCHITECTURE.md), [RESEARCH.md](RESEARCH.md) et [EFE_ANALYSE_ET_PLAN_FINITION.md](EFE_ANALYSE_ET_PLAN_FINITION.md) (celui-ci) forment maintenant une trilogie cohérente :

- **ARCHITECTURE.md** = blueprint de S&S (l'application)
- **RESEARCH.md** = état de l'art (SwarmSys, Symphony, Speculative RAG)
- **EFE_ANALYSE_ET_PLAN_FINITION.md** = analyse de la plateforme EFE existante et plan pour faire tourner S&S dessus

**Synthèse** : S&S est l'application *grand public* qui *valide commercialement* la plateforme EFE auprès des développeurs et entreprises non-régulées, pendant que la vente régulée se construit en parallèle. C'est le bon ordre.

---

## 6. Décisions clés à acter immédiatement

| Décision | Recommandation |
|---|---|
| Construire S&S **dans le repo EFE** ou **dans `vrai/`** ? | **Dans EFE**, comme un crate `efe-nano-app-scout-synthesize` + une UI dans `apps/`. Évite la divergence et bénéficie de toute l'infra existante. |
| Renommer "nano-agent" vs "SLM nano-agent" ? | **Oui**, dans la doc publique. Garder "nano-agent" pour l'unité d'exécution, ajouter "SLM-driven nano-agent" pour le sous-ensemble Mode B/C avec un petit LLM. |
| Prioriser EFE-regulated ou EFE-developer ? | **EFE-regulated** pour 12 mois (le moat est là). EFE-developer en freemium pour générer la pipeline. |
| Wirer Postgres avant ou après Firecracker ? | **Postgres d'abord**. Plus simple, débloque la persistence avant de gérer l'isolation. |
| Faire tourner S&S avec stubs ou avec wirings complets ? | **Avec stubs d'abord** (Postgres in-memory, Mock LLM). Démonstrable en 1 semaine. Puis remplacer brique par brique. |
| Le golden set S&S doit être dans EFE ou séparé ? | **Dans EFE**, comme `eval/scout_synthesize_golden.jsonl`. Passe par `efe-gold-test-gate`. |

---

## 7. Le prochain mouvement concret (cette semaine)

Si je dois prescrire UNE action pour démarrer **lundi matin** :

```bash
# 1. Setup
cd ~/Downloads/jjjj\ copie/efe
git checkout -b feature/scout-synthesize-mvp

# 2. Créer le nouveau crate
mkdir -p crates/efe-nano-app-scout-synthesize/src
cd crates/efe-nano-app-scout-synthesize

# 3. Cargo.toml minimal — réutilise tout l'écosystème existant
cat > Cargo.toml <<EOF
[package]
name = "efe-nano-app-scout-synthesize"
version = "0.0.1"
edition = { workspace = true }
description = "S&S — Scout & Synthesize : DAG préconçu pour la recherche documentaire à faible latence."

[dependencies]
efe-nano                 = { path = "../efe-nano" }
efe-nano-sdk             = { path = "../efe-nano-sdk" }
efe-blackboard           = { path = "../efe-blackboard" }
efe-swarm-orchestrator   = { path = "../efe-swarm-orchestrator" }
efe-agentic-bridge       = { path = "../efe-agentic-bridge" }
efe-vector               = { path = "../efe-vector" }
efe-retrieval            = { path = "../efe-retrieval" }
efe-cache                = { path = "../efe-cache" }

tokio       = { workspace = true }
serde       = { workspace = true }
serde_json  = { workspace = true }
async-trait = { workspace = true }
EOF
```

À partir de là, écrire :
1. Le `ContextCapsule` Rust dans `efe-nano-sdk`
2. Le DAG S&S YAML dans le nouveau crate
3. Un test E2E qui dispatche le DAG sur une question simple
4. Itérer

Le rest suit.

---

## En une phrase

> EFE est **déjà** la plateforme nano-agent qu'on imaginait construire dans S&S — elle a 47 crates, 5K tests verts, un blackboard concurrent avec audit HMAC, 20 nano-agents starter, et un bridge Claude streaming. Le travail restant n'est pas de la concevoir, c'est de (a) remplacer les stubs Mode B/C par des vrais SLM (SmolLM2/Qwen2.5/Llama-3.2/Phi-3.5), (b) ajouter 5 nano-agents spécifiques à S&S (`retrieval_plan`, `rerank_bge`, `aggregate_capsules`, `fact_check`, `intent_research`), (c) ajouter les phéromones au blackboard et la spéculation pendant la frappe au gateway, et (d) finir les 6 wirings opérationnels que l'investor brief identifie. **Total : 10 semaines bien tenues, et le concept "véritables nano-agents" devient un produit qu'on peut vendre.**
