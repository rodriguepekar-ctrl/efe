# Recherche d'état de l'art — *Scout & Synthesize*

> Audit complet de ce qui existe déjà sur **Hugging Face** et **GitHub** qui peut nous aider à construire le projet décrit dans [ARCHITECTURE.md](ARCHITECTURE.md). Cible : maximiser la réutilisation, minimiser le code écrit, identifier les angles morts.

**Date de l'audit** : 23 mai 2026
**Méthode** : recherche multi-sources sur HF papers, HF hub (modèles), Web (GitHub repos, blog posts, docs), avec ~25 requêtes ciblées sur 8 axes.

---

## TL;DR — la grande surprise

**Notre architecture n'est pas originale.** Trois publications très récentes (août–octobre 2025) décrivent quasi-exactement les patrons qu'on a inventés depuis zéro :

| Composant qu'on a "inventé" | Précédent récent qui le formalise | À retenir |
|---|---|---|
| Phéromones + capsules + scouts | **SwarmSys** (2025-10) avec Explorers/Workers/Validators et "pheromone-inspired reinforcement" | Adopter leur vocabulaire, leur boucle exploration/exploitation/validation |
| Edge LLMs + ledger de capacités + Beacon protocol | **Symphony** (2025-08, [code GitHub](https://github.com/GradientHQ/Symphony)) | Réutiliser leur structure de packages comme inspiration |
| Drafter/Verifier (L2 scouts → L4 cloud) | **Speculative RAG** (2024-07, [code](https://github.com/jjovalle99/Speculative-RAG)) | Adopter leur K-means clustering pour la diversité des subsets |
| Prompt caching agressif | **TurboRAG** (2024-10) précompute KV caches → **9.4× TTFT** | Activer prompt caching Anthropic + envisager KV cache local |

**Conséquence pour le projet** : on ne va pas réécrire ce qui existe. Ce document liste **ce qu'on adopte tel quel, ce qu'on adapte, ce qu'on doit encore écrire**.

---

## 1. Mapping ARCHITECTURE.md → composants existants

Vue d'ensemble de la réutilisation par couche :

| Couche | Composant prévu | Solution existante | Effort | Décision |
|---|---|---|---|---|
| **L0** Intent | SmolLM2-360M + JSON grammar | [HuggingFaceTB/SmolLM2-360M-Instruct](https://hf.co/HuggingFaceTB/SmolLM2-360M-Instruct) (GGUF dispo) + `llama-server --grammar-file` | 🟢 Faible | **Réutiliser** modèle, écrire prompt + grammar |
| **L1** Plan | Qwen2.5-0.5B | [Qwen2.5-0.5B-Instruct GGUF](https://hf.co/Qwen/Qwen2.5-0.5B-Instruct) + JSON schema | 🟢 Faible | **Réutiliser** modèle, écrire logique de décomposition |
| **L2** Scouts | Pipeline retrieval+rerank+extract en parallèle | [bge-m3](https://hf.co/BAAI/bge-m3) + [bge-reranker-v2-m3](https://hf.co/BAAI/bge-reranker-v2-m3) + LanceDB hybrid search | 🟡 Moyen | **Composer** existant + écrire orchestration |
| **Drafter/verifier** | Pattern L2→L4 | Inspirer de [Speculative RAG](https://github.com/jjovalle99/Speculative-RAG) (K-means clustering subsets) | 🟢 Faible | **Adopter** patron, adapter à nos capsules |
| **Stigmergie** | Phéromones LanceDB | Inspirer de **SwarmSys** (pheromone-inspired reinforcement) | 🟡 Moyen | **Adapter** à notre schéma de capsules |
| **L3** Aggregator | Llama-3.2-1B + dédup sémantique | [Llama-3.2-1B-Instruct GGUF](https://hf.co/bartowski/Llama-3.2-1B-Instruct-GGUF) + LanceDB cosine | 🟢 Faible | **Réutiliser** |
| **L4** Cloud | Claude API + prompt caching | SDK Anthropic + caching natif | 🟢 Faible | **Réutiliser** SDK direct |
| **L5** Critique | Phi-3.5-mini fact-check | [MaziyarPanahi/Phi-3.5-mini-instruct-GGUF](https://hf.co/MaziyarPanahi/Phi-3.5-mini-instruct-GGUF) | 🟢 Faible | **Réutiliser** modèle |
| **Inférence** | llama.cpp multi-slot | [llama.cpp server](https://github.com/ggml-org/llama.cpp/tree/master/tools/server) `--parallel`, continuous batching | 🟢 Faible | **Réutiliser** |
| **Storage** | LanceDB hiérarchique | [LanceDB](https://lancedb.com) avec hybrid search + rerankers built-in | 🟢 Faible | **Réutiliser** |
| **Streaming UI** | SSE | [sse-starlette](https://pypi.org/project/sse-starlette/) + FastAPI | 🟢 Faible | **Réutiliser** |
| **Evaluation** | Golden set + LLM judge | [Ragas](https://github.com/explodinggradients/ragas) faithfulness/context precision/answer relevancy + [DeepEval](https://deepeval.com) | 🟢 Faible | **Réutiliser** Ragas comme socle, DeepEval pour CI |
| **Orchestration** | Pipeline L0-L5 | Code maison (async Python) — pas de LangGraph | 🔴 Élevé | **Écrire** (~500 LOC) |
| **Speculator (frappe)** | Pre-fetch debounce | Code maison spécifique | 🔴 Élevé | **Écrire** |
| **Distillation loop** | LoRA + A/B | [unsloth](https://github.com/unslothai/unsloth) pour LoRA + Ragas pour A/B | 🟡 Moyen | **Composer** |

**Verdict** : ~70 % des composants existent déjà en open-source. Notre travail propre se concentre sur (a) l'orchestrateur asyncio, (b) la stigmergie sur LanceDB, (c) le speculator pendant frappe, (d) la boucle de distillation. C'est gérable.

---

## 2. Les 10 papiers les plus alignés (lecture obligatoire)

Classés par pertinence directe avec notre archi. Chaque papier doit être lu si on veut comprendre les choix de design.

### 🟢 Tier 1 — alignement direct, à étudier en profondeur

#### 1. SwarmSys: Decentralized Swarm-Inspired Agents for Scalable and Adaptive Reasoning
- **Date** : 11 oct. 2025 — [paper](https://hf.co/papers/2510.10047)
- **Pourquoi** : décrit littéralement notre intuition. Trois rôles (Explorers, Workers, Validators) cyclant exploration/exploitation/validation. **Phéromone-inspired reinforcement mechanism**. Embedding-based probabilistic matching pour l'allocation de tâches. Pas de supervision globale.
- **À reprendre** :
  - Vocabulaire **Explorer/Worker/Validator** plus parlant que L0/L2/L5
  - Mécanique de phéromones (renforcement + décroissance) — directement applicable à notre LanceDB pheromone field
  - Adaptive agent profiles : chaque nano peut avoir un "profil" qui évolue

#### 2. Symphony: A Decentralized Multi-Agent Framework for Scalable Collective Intelligence
- **Date** : 27 août 2025 — [paper](https://hf.co/papers/2508.20019) — **[code GitHub](https://github.com/GradientHQ/Symphony)** (MIT)
- **Pourquoi** : LLMs légers sur edge (Apple M-series cité explicitement), ledger décentralisé de capacités, **Beacon protocol** pour task allocation, weighted voting CoT.
- **À reprendre** :
  - Structure de packages (`agents/`, `core/`, `protocol/`, `runtime/`, `models/`, `infra/`) — bonne base
  - Beacon dataclass `(beacon_id, source_id, task_requirements, TTL, context)` — applicable à notre `SubQuery`
  - Capability vector + cosine matching — déjà notre design L1
  - Mistral-7B + Qwen2.5-7B + DeepSeek testés — confirme que des modèles "midi" peuvent jouer L3
  - Orchestration overhead < 5 % de la latence d'inférence — bon benchmark à viser
- **À ne pas reprendre** : leur cadre "decentralized" assume 3 serveurs RTX 4090, pas pertinent pour notre 1-machine. On reste centralisé sur un seul orchestrateur.

#### 3. Speculative RAG: Enhancing Retrieval Augmented Generation through Drafting
- **Date** : 11 juil. 2024 — [paper](https://hf.co/papers/2407.08223) — **[code repro](https://github.com/jjovalle99/Speculative-RAG)**
- **Pourquoi** : modèle parfait pour notre L2 (drafter) → L4 (verifier). Multi-perspective sampling via K-means clustering — résout le problème de diversité dans nos scouts.
- **À reprendre** :
  - **K-means clustering** des documents retrouvés, puis sample d'un par cluster pour générer N drafts diversifiés
  - 3 scores de confiance combinés : `ρ_Draft × ρ_Self-Consistency × ρ_Self-Reflection`
  - Hyperparamètres validés : m=5 drafts, k=2 docs/draft (TriviaQA, PopQA) ou m=10/k=6 (MuSiQue multi-hop)
  - Latence -50.83 % et qualité +12.97 % vs baseline sur PubHealth — preuve que le pattern fonctionne
- **Adaptation** : leur drafter est Mistral 7B fine-tuné ; nous voulons Qwen2.5-0.5B → besoin de distillation (cf. §6)

#### 4. TurboRAG: Accelerating RAG with Precomputed KV Caches
- **Date** : 10 oct. 2024 — [paper](https://hf.co/papers/2410.07590)
- **Pourquoi** : réduction **9.4× du TTFT** (8.6× en moyenne) en précalculant les KV caches des documents offline. Directement pertinent pour notre objectif latence.
- **À reprendre** :
  - Pour les **N capsules les plus fréquemment citées**, précalculer leur KV cache via Claude (prompt caching) ou localement (Llama-3.2-1B en fallback)
  - Le **mask matrix** et positional embedding hack pour concaténer des KV caches calculés indépendamment
- **Limite** : Claude prompt caching le fait automatiquement côté serveur. À explorer plus si on veut un mode 100% local.

#### 5. RAPID: Retrieval-Augmented Speculative Decoding for Long-Context
- **Date** : 27 fév. 2025 — [paper](https://hf.co/papers/2502.20330)
- **Pourquoi** : intègre RAG comme **drafter** pour speculative decoding du long-context LLM. Une autre lecture du pattern "petit propose, gros vérifie".

### 🟡 Tier 2 — patterns connexes à connaître

#### 6. RAPTOR: Recursive Abstractive Processing for Tree-Organized Retrieval
- **Date** : 31 janv. 2024 — [paper](https://hf.co/papers/2401.18059) (48 upvotes)
- **Pourquoi** : structure d'index hiérarchique par clustering + summarization récursive. Alternative ou complément à notre découpe section/paragraph/sentence.
- **À évaluer** : si notre découpe statique sous-performe sur multi-hop, basculer sur RAPTOR (LlamaIndex et LangChain ont des implémentations directes).

#### 7. RouteLLM: Learning to Route LLMs with Preference Data
- **Date** : 26 juin 2024 — [paper](https://hf.co/papers/2406.18665) (7 upvotes)
- **Pourquoi** : référence pour le **routage** (gros vs petit) — on l'utilise implicitement en L0 quand on décide `requires_synthesis: bool`.

#### 8. CARROT: Cost Aware Rate Optimal Router
- **Date** : 5 fév. 2025 — [paper](https://hf.co/papers/2502.03261)
- **Pourquoi** : formalise le tradeoff coût/qualité avec garantie de rate-optimality. Cadre théorique pour notre seuil d'escalade vers Claude.

#### 9. Online Speculative Decoding
- **Date** : 11 oct. 2023 — [paper](https://hf.co/papers/2310.07177)
- **Pourquoi** : retrainer continuellement les drafters sur les query patterns de production. **Directement** notre boucle de distillation.
- **À reprendre** : leur méthode "online knowledge distillation" pour le cycle hebdomadaire de fine-tuning LoRA.

#### 10. BookRAG / RT-RAG / SAGE — hiérarchique
- **Dates** : déc. 2025 / janv. 2026 / mars 2025 — [BookRAG](https://hf.co/papers/2512.03413), [RT-RAG](https://hf.co/papers/2601.11255), [SAGE](https://hf.co/papers/2503.01713)
- **Pourquoi** : trois variantes récentes du RAG hiérarchique. Décomposition en sous-questions, arbres de raisonnement, sémantique segmentation. Servent de comparaison qualitative pour notre design.

### 🔵 Tier 3 — à garder en bibliographie

- **MASS** ([paper](https://hf.co/papers/2502.02533)) : optimisation prompts/topologies de systèmes multi-agents
- **Scaling Small Agents Through Strategy Auctions** ([paper](https://hf.co/papers/2602.02751)) : SLMs coordonnés par enchères
- **MacNet** ([paper](https://hf.co/papers/2406.07155)) : scaling laws collaboratifs (loi log)
- **Speculative Streaming** ([paper](https://hf.co/papers/2402.11131)) : draft sans modèle auxiliaire
- **UFO^3** ([paper](https://hf.co/papers/2511.11332)) : Microsoft, DAG orchestration multi-device
- **DR.LLM** ([paper](https://hf.co/papers/2510.12773)) : routing dynamique de couches

---

## 3. Inventaire complet des modèles (HF)

Tous validés avec leur taille de téléchargement (popularité = robustesse) et la disponibilité GGUF/ONNX.

### 3.1 Nanos décisionnels (L0, L1, L3)

| Modèle | Params | Downloads | GGUF | ONNX | Multilingue | Usage |
|---|---|---|---|---|---|---|
| [SmolLM2-135M-Instruct](https://hf.co/HuggingFaceTB/SmolLM2-135M-Instruct) | 135 M | 1.4 M | ✅ ([bartowski](https://hf.co/bartowski/SmolLM2-135M-Instruct-GGUF)) | ✅ | EN | L0 minimal |
| [SmolLM2-360M-Instruct](https://hf.co/HuggingFaceTB/SmolLM2-360M-Instruct) | 360 M | 279 K | ✅ ([HuggingFaceTB](https://hf.co/HuggingFaceTB/SmolLM2-360M-Instruct-GGUF)) | ✅ | EN | **L0 recommandé** |
| [SmolLM2-1.7B-Instruct](https://hf.co/HuggingFaceTB/SmolLM2-1.7B-Instruct) | 1.7 B | 185 K | ✅ | ✅ | EN | L1 alternative |
| [Qwen2.5-0.5B-Instruct](https://hf.co/Qwen/Qwen2.5-0.5B-Instruct) | 0.5 B | — | ✅ | — | Multi | **L1, L2-extract** |
| [Qwen2.5-1.5B-Instruct](https://hf.co/Qwen/Qwen2.5-1.5B-Instruct) | 1.5 B | — | ✅ | — | Multi | L3 alternative |
| [Llama-3.2-1B-Instruct-GGUF](https://hf.co/bartowski/Llama-3.2-1B-Instruct-GGUF) | 1 B | 102 K | ✅ | — | EN/FR/DE/IT/ES/PT/HI/TH | **L3 recommandé** (multilingue natif !) |
| [hugging-quants Q4_K_M](https://hf.co/hugging-quants/Llama-3.2-1B-Instruct-Q4_K_M-GGUF) | 1 B | 40 K | ✅ (Q4) | — | Multi | Plus léger |

### 3.2 Critique (L5)

| Modèle | Params | Downloads | GGUF | Usage |
|---|---|---|---|---|
| [Phi-3.5-mini-instruct](https://hf.co/microsoft/Phi-3.5-mini-instruct) | 3.8 B | — | ✅ ([bartowski](https://hf.co/bartowski/Phi-3.5-mini-instruct-GGUF)) | **L5** |
| [MaziyarPanahi Phi-3.5-mini-instruct-GGUF](https://hf.co/MaziyarPanahi/Phi-3.5-mini-instruct-GGUF) | 3.8 B | 233 K | ✅ multi-quant | L5 prêt |

### 3.3 Embeddings (L1, L2, cache)

| Modèle | Dim | Downloads | ONNX | GGUF | Multilingue | Usage |
|---|---|---|---|---|---|---|
| [bge-m3](https://hf.co/BAAI/bge-m3) | 1024 | **28.7 M** | ✅ | — | 100+ langues | **Embeddings recommandé** |
| [mxbai-embed-large-v1](https://hf.co/mixedbread-ai/mxbai-embed-large-v1) | 1024 | 4.7 M | ✅ | ✅ ([ChristianAzinn](https://hf.co/ChristianAzinn/mxbai-embed-large-v1-gguf)) | EN | Alternative anglaise |
| [deepset-mxbai-embed-de-large-v1](https://hf.co/mixedbread-ai/deepset-mxbai-embed-de-large-v1) | 1024 | 258 K | ✅ | — | DE/EN | Si focus allemand |

**Décision** : adopter **bge-m3** comme défaut (multilingue, énorme adoption, ONNX). Pour FR pur, mxbai en backup.

### 3.4 Rerankers

| Modèle | Downloads | GGUF | ONNX | Multilingue | Usage |
|---|---|---|---|---|---|
| [bge-reranker-v2-m3](https://hf.co/BAAI/bge-reranker-v2-m3) | **12.5 M** | ✅ ([gpustack](https://hf.co/gpustack/bge-reranker-v2-m3-GGUF)) | ✅ ([onnx-community](https://hf.co/onnx-community/bge-reranker-v2-m3-ONNX)) | Oui | **Reranker recommandé** |
| [bge-reranker-v2-gemma](https://hf.co/BAAI/bge-reranker-v2-gemma) | 44.5 K | — | — | Oui | Plus précis mais plus lent |
| [bge-reranker-base](https://hf.co/BAAI/bge-reranker-base) | 4.5 M | — | ✅ | EN/ZH | Backup léger |
| [bge-reranker-v2.5-gemma2-lightweight](https://hf.co/BAAI/bge-reranker-v2.5-gemma2-lightweight) | 162 K | — | — | Oui | Récent, à tester |

**Décision** : **bge-reranker-v2-m3** (ONNX dispo, multilingue, rapide).

### 3.5 Récap mémoire (Mac M-series)

Avec quantization Q5_K_M par défaut :

| Modèle | RAM (Q5) | Slot |
|---|---|---|
| SmolLM2-360M | ~250 Mo | port 8080 (L0) |
| Qwen2.5-0.5B | ~400 Mo | port 8081 (L1, L2-extract) |
| Llama-3.2-1B | ~800 Mo | port 8082 (L3, fallback) |
| Phi-3.5-mini (Q4) | ~2.2 Go | port 8083 (L5) |
| bge-m3 (ONNX) | ~1.2 Go | in-process |
| bge-reranker-v2-m3 (ONNX) | ~1.2 Go | in-process |
| **Total** | **~6 Go** | ✅ tient sur 16 Go avec marge |

---

## 4. Frameworks et bases de code : ce qu'on utilise, ce qu'on évite

### 4.1 RAG frameworks — ce qu'on évite (et pourquoi)

| Framework | Forces | Pourquoi on ne l'utilise pas |
|---|---|---|
| [LangChain](https://github.com/langchain-ai/langchain) | écosystème énorme | trop d'abstractions, control flow opaque, overhead non négligeable |
| [LlamaIndex](https://github.com/run-llama/llama_index) | retrieval state-of-the-art, 150+ connectors | trop "framework", veut prendre l'orchestration |
| [Haystack](https://github.com/deepset-ai/haystack) | production-grade, structured pipelines | pipeline-centric mais lourd pour notre cas |
| [RAGFlow](https://github.com/infiniflow/ragflow) | UI visuelle, low-code | trop de dépendances (Elasticsearch + Infinity DB), non-light |
| [LightRAG (HKUDS)](https://github.com/HKUDS/LightRAG) | knowledge graph + RAG, EMNLP 2025, RAGAS intégré | overlap avec ce qu'on construit mais ajoute graph qu'on ne veut pas |
| [FlashRAG](https://github.com/RUC-NLPIR/FlashRAG) | toolkit recherche RAG efficient (WWW 2025) | orienté recherche, pas production |
| [UltraRAG (OpenBMB)](https://github.com/OpenBMB/UltraRAG) | low-code MCP framework | trop nouveau, écosystème immature |

**Décision globale** : on n'utilise **aucun framework RAG end-to-end**. On compose des briques (LanceDB, fastembed, bge-reranker, Anthropic SDK) avec ~500 LOC d'orchestration asyncio. Justifications :
- Notre DAG L0–L5 est **fixe** → un framework dynamique ajoute de l'overhead sans bénéfice
- Notre objectif **latence** demande un control flow optimisé, pas une abstraction qui empile des couches
- On veut maîtriser le **streaming SSE** finement, ce que les frameworks rendent compliqué

### 4.2 Frameworks d'agents — ce qu'on regarde de près

#### [smolagents](https://github.com/huggingface/smolagents) (Hugging Face)
- **~1000 LOC** au total, philosophie "agents que pensent en code"
- **CodeAgent** vs **ToolCallingAgent**
- Model-agnostic : transformers, ollama, providers HF, LiteLLM (OpenAI/Anthropic)
- Sandboxing intégré : Blaxel, E2B, Modal, Docker, Pyodide+Deno WASM
- **Pertinence pour nous** : limitée. Notre architecture n'a pas besoin d'exécuter du code arbitraire ni de boucler. À regarder uniquement pour la **simplicité de leur design**.

#### [DSPy](https://github.com/stanfordnlp/dspy) (Stanford NLP)
- Programmation déclarative + compilation vers métriques
- Compile vers des prompts/few-shots optimisés
- **Bench** : programmes DSPy avec T5-770M et Llama2-13b compétitifs avec GPT-3.5 + chains expert
- **Pertinence pour nous** : 🟡 utilisable pour **optimiser les prompts** de L0/L1/L3 *après* le MVP. Pas de dépendance critique.

#### [Symphony (GradientHQ)](https://github.com/GradientHQ/Symphony)
- Code MIT, structure inspirante (cf. §2 papier)
- **Pertinence** : on s'inspire de la structure de packages, mais on n'embarque pas le code (notre cas est centralisé, le leur décentralisé)

#### [LangGraph](https://github.com/langchain-ai/langgraph)
- DAG explicite, state management, supérieur à LangChain pour les flux complexes
- **Pertinence** : 🔴 trop d'overhead pour notre DAG figé. Garder en backup si on doit gérer des flux conditionnels complexes.

### 4.3 Inférence — confirmé

#### [llama.cpp server](https://github.com/ggml-org/llama.cpp/tree/master/tools/server)
Documentation officielle confirme **tout ce qu'on prévoyait** :

```bash
llama-server \
  -m model.gguf \
  --parallel 4 \              # 4 slots en parallèle
  --cont-batching \           # continuous batching (activé par défaut)
  --cache-ram 4096 \          # 4 Go de KV cache
  --cache-idle-slots \        # libère slot idle au nouveau task
  --json-schema schema.json \ # OU --grammar-file grammar.gbnf
  --port 8080
```

Et **`cache_prompt: true`** côté requête : réutilise le KV cache des préfixes communs entre requêtes. **Énorme** pour nos prompts L0/L1/L3 qui ont un préfixe fixe.

#### Alternatives écartées
- **vLLM** : excellent mais orienté GPU NVIDIA, moins bon sur Mac M-series
- **MLX** : Apple-only, écosystème plus jeune
- **TGI** (HuggingFace) : pas de gain sur llama.cpp pour notre cas

### 4.4 Stockage vectoriel — confirmé

[LanceDB](https://lancedb.com) coche toutes les cases :
- **Embedded** (pas de daemon)
- **Hybrid search** (vector + FTS BM25) avec reranker intégré
- **Rerankers built-in** : on peut brancher bge-reranker directement
- **Hierarchical retrieval** documenté (parent document / bigger chunk)
- **$30 M Série A en juin 2025** → maintenu activement
- **Recommandation** : LanceDB pour la V1, alternative Qdrant si on veut multi-tenant plus tard

### 4.5 Évaluation — stack recommandée

| Outil | Quand | Pourquoi |
|---|---|---|
| [Ragas](https://github.com/explodinggradients/ragas) | dev + CI | Métriques RAG spécialisées (faithfulness, context precision, answer relevancy) |
| [DeepEval](https://deepeval.com) | CI/CD | Unit tests style, intégration pytest |
| [Promptfoo](https://www.promptfoo.dev/) | regression detection | CLI rapide, cost tracking |
| LangSmith / TruLens | prod monitoring | Si besoin observabilité prod avancée |

**Décision** : commencer avec **Ragas** seul (covers 80 % des besoins), ajouter DeepEval si on a un CI pipeline.

### 4.6 Streaming UI — confirmé

- **FastAPI + [sse-starlette](https://pypi.org/project/sse-starlette/)** : implémentation SSE native, async generator, supports `EventSourceResponse` avec types d'événements personnalisés.
- Pattern validé : `yield {"event": "source", "data": json.dumps(capsule)}` → côté frontend, `EventSource` consume directement.

### 4.7 Fine-tuning / distillation

- **[unsloth](https://github.com/unslothai/unsloth)** : LoRA optimisé, 2× plus rapide que HuggingFace TRL, 50 % moins de RAM. Confirmé compatible avec Qwen2.5, Llama-3.2, Phi-3.5.
- Pipeline distillation : `unsloth` pour le fine-tuning, **Online Speculative Decoding** (paper §2.9) pour la méthode de retraining continu.

---

## 5. Décisions actionables : build vs reuse

### À adopter tel quel (réutilisation pure)

```
✅ HuggingFaceTB/SmolLM2-360M-Instruct        # L0
✅ Qwen/Qwen2.5-0.5B-Instruct                  # L1, L2-extract
✅ bartowski/Llama-3.2-1B-Instruct-GGUF        # L3 + fallback
✅ MaziyarPanahi/Phi-3.5-mini-instruct-GGUF    # L5
✅ BAAI/bge-m3                                  # embeddings
✅ BAAI/bge-reranker-v2-m3                     # rerank L2
✅ ggml-org/llama.cpp server                   # inférence
✅ LanceDB                                      # storage
✅ sse-starlette + FastAPI                     # streaming
✅ Ragas                                        # eval
✅ unsloth                                      # distillation
✅ Anthropic Python SDK                        # L4
```

### À adapter (inspiration code)

```
🔄 Symphony (GradientHQ) — structure de packages, Beacon dataclass
🔄 Speculative RAG (jjovalle99) — K-means subsets + 3-score verification
🔄 SwarmSys — vocabulaire Explorer/Worker/Validator + phéromones
🔄 LightRAG — patterns d'ingestion hiérarchique
```

### À écrire from scratch (cœur métier)

```
✍️ Orchestrateur async (handle_request avec L0→L5 streaming)
✍️ Speculator (debounce frappe → spec L0+L1 → cache)
✍️ Stigmergie (LanceDB pheromone field + décroissance + écriture par capsule)
✍️ Cancellation propagation (request_id + asyncio.CancelledError)
✍️ Critique L5 parallèle au stream (sentence buffer + claim extraction + similarity)
✍️ Distillation loop (capture (input, output, claude_correction) + LoRA hebdo)
```

**Estimation LOC propre** : ~1500–2000 lignes Python. Compatible avec le plan de 8 semaines.

---

## 6. Surprises et angles morts identifiés

### 6.1 Bonnes nouvelles
- **Le pattern drafter/verifier est validé** par Speculative RAG (-50 % latence sur PubHealth)
- **Le pattern phéromones est récemment formalisé** par SwarmSys
- **Prompt caching Anthropic + TurboRAG** prouvent un gain de 8–9× sur TTFT
- **bge-m3 + bge-reranker-v2-m3** couvrent le multilingue natif (incluant FR pour toi)
- **Symphony donne du code MIT** comme template

### 6.2 Angles morts à creuser

#### a) La spéculation pendant frappe n'a pas d'équivalent direct
- Aucun projet open-source ne fait du **pre-fetch sémantique** pendant la frappe d'une requête
- C'est notre **différenciateur principal** côté latence perçue
- À écrire complètement (~200 LOC)

#### b) La stigmergie sur capsules n'a pas d'implémentation publique
- SwarmSys décrit le concept mais ne publie pas le code de la phéromone
- À implémenter sur LanceDB (champ `pheromone`, mise à jour transactionnelle, décroissance par cron)

#### c) Le critique L5 en parallèle du stream est rare
- La plupart des frameworks RAG font la critique **après** la génération complète
- L5 streamé en parallèle (annotations sur le texte au fur et à mesure) est notre invention propre — à valider en UX

#### d) Le fallback local depuis Claude est rarement implémenté
- Beaucoup de systèmes "fallback" basculent sur **un autre cloud LLM**
- Notre fallback **local** (Llama-3.2-1B) est unique mais limité en qualité ; à mesurer si l'UX accepte la régression

### 6.3 Risques nouveaux identifiés par la recherche

1. **bge-m3 sur ONNX** : à vérifier la perf CPU vs PyTorch — certains rapports mentionnent une régression de 10–20 % avec ONNX
2. **Phi-3.5-mini en Q4** : qualité du critique peut souffrir, prévoir Q5 minimum si on a la RAM
3. **LanceDB write contention** : la stigmergie écrit *à chaque capsule consultée* → benchmarker la concurrence avant la prod
4. **llama.cpp slot saturation** : si 5 scouts L2 frappent le même modèle en même temps, augmenter `--parallel` à 8 mais profiler la mémoire

---

## 7. Plan d'implémentation mis à jour

Le plan de 8 semaines de [ARCHITECTURE.md §11](ARCHITECTURE.md#11-plan-dimplémentation-8-semaines) est confirmé. Ajustements basés sur la recherche :

| Sem | Action | Modèles/lib confirmés |
|---|---|---|
| **1** | Skeleton ingestion + retrieval + Claude | LanceDB + bge-m3 + Anthropic SDK + sse-starlette |
| **2** | L2 scouts + K-means clustering (inspiration Speculative RAG) | + bge-reranker-v2-m3 |
| **3** | L0 + L1 + L3 (Qwen2.5-0.5B + SmolLM2-360M + Llama-3.2-1B) | + llama-server multi-port |
| **4** | Cache L0/L2 + prompt caching Anthropic | Anthropic prompt cache config |
| **5** | Spéculation frappe + SSE UI complète | Code propre |
| **6** | L5 critique (Phi-3.5-mini) + stigmergie LanceDB | Code propre |
| **7** | Eval Ragas + golden set 500 cas | Ragas + LLM judge |
| **8** | Distillation LoRA hebdo (unsloth + Online Spec Dec method) | unsloth + script cron |

### Changements vs plan initial

- **Sem 2 enrichie** : adopter K-means clustering de Speculative RAG **dès** L2 (avant on l'avait laissé pour plus tard) — gain qualité immédiat
- **Sem 4 simplifiée** : prompt caching Anthropic suffit pour V1 ; précomputed KV caches local (TurboRAG style) reporté à V2
- **Sem 8 cadrée** : `Online Speculative Decoding` paper donne la méthode exacte — pas besoin d'inventer

---

## 8. Lectures recommandées par profil

### Si tu n'as 2 h à investir
1. [SwarmSys paper](https://hf.co/papers/2510.10047) (40 min lecture)
2. [Speculative RAG paper](https://hf.co/papers/2407.08223) (30 min)
3. [Symphony code](https://github.com/GradientHQ/Symphony) — survol des packages (30 min)
4. [llama.cpp server README](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md) — section parallel (20 min)

### Si tu veux comprendre la stigmergie en profondeur
1. SwarmSys → relire la section *pheromone-inspired reinforcement mechanism*
2. Stochastic Self-Organization in Multi-Agent Systems ([paper](https://hf.co/papers/2510.00685))
3. Beyond Self-Talk survey ([paper](https://hf.co/papers/2502.14321)) sections communication patterns

### Si tu veux maximiser la qualité (perf RAG)
1. RAPTOR ([paper](https://hf.co/papers/2401.18059))
2. BookRAG ([paper](https://hf.co/papers/2512.03413))
3. RT-RAG ([paper](https://hf.co/papers/2601.11255))
4. SAGE ([paper](https://hf.co/papers/2503.01713))

### Si tu veux maximiser la vitesse (latence)
1. TurboRAG ([paper](https://hf.co/papers/2410.07590))
2. Speculative Streaming ([paper](https://hf.co/papers/2402.11131))
3. Stream2LLM ([paper](https://hf.co/papers/2604.16395))
4. RAGServe ([paper](https://hf.co/papers/2412.10543))

---

## 9. Bibliographie complète (papiers + repos)

### Papiers Hugging Face (par ordre de pertinence)

#### Multi-agents et swarm intelligence
1. [SwarmSys: Decentralized Swarm-Inspired Agents](https://hf.co/papers/2510.10047) — *Li et al., 2025-10*
2. [Symphony: A Decentralized Multi-Agent Framework](https://hf.co/papers/2508.20019) — *Wang et al., 2025-08*
3. [SwarmAgentic: Towards Fully Automated Agentic System Generation via Swarm Intelligence](https://hf.co/papers/2506.15672) — *Zhang et al., 2025-06*
4. [Symphony-Coord: Emergent Coordination in Decentralized Agent Systems](https://hf.co/papers/2602.00966) — *Guan et al., 2026-02*
5. [Benchmarking LLMs' Swarm intelligence](https://hf.co/papers/2505.04364) — *Ruan et al., 2025-05*
6. [Multi-Agent Design: Optimizing Agents with Better Prompts and Topologies](https://hf.co/papers/2502.02533) — *Zhou et al., 2025-02*
7. [UFO^3: Weaving the Digital Agent Galaxy](https://hf.co/papers/2511.11332) — *Microsoft, 2025-11*
8. [Scaling Small Agents Through Strategy Auctions](https://hf.co/papers/2602.02751) — *Alazraki et al., 2026-02*
9. [Beyond Self-Talk: Communication-Centric Survey](https://hf.co/papers/2502.14321) — *Yan et al., 2025-02*
10. [Scaling Large-Language-Model-based Multi-Agent Collaboration](https://hf.co/papers/2406.07155) — *Qian et al., 2024-06*
11. [Stochastic Self-Organization in Multi-Agent Systems](https://hf.co/papers/2510.00685) — *Tastan et al., 2025-10*

#### Speculative decoding et drafter/verifier
12. [Speculative RAG](https://hf.co/papers/2407.08223) — *Wang et al., 2024-07*
13. [RAPID: Retrieval-Augmented Speculative Decoding](https://hf.co/papers/2502.20330) — *Chen et al., 2025-02*
14. [Speculative Streaming](https://hf.co/papers/2402.11131) — *Apple, 2024-02*
15. [Online Speculative Decoding](https://hf.co/papers/2310.07177) — *Liu et al., 2023-10*
16. [Ouroboros: Speculative Decoding with Large Model Enhanced Drafting](https://hf.co/papers/2402.13720) — *Zhao et al., 2024-02*
17. [DReSD: Dense Retrieval for Speculative Decoding](https://hf.co/papers/2502.15572) — *Gritta et al., 2025-02*

#### RAG hiérarchique et retrieval avancé
18. [RAPTOR: Recursive Abstractive Processing for Tree-Organized Retrieval](https://hf.co/papers/2401.18059) — *Sarthi et al., 2024-01*
19. [BookRAG: Hierarchical Structure-aware Index](https://hf.co/papers/2512.03413) — *Wang et al., 2025-12*
20. [RT-RAG: Reasoning in Trees](https://hf.co/papers/2601.11255) — *Shi et al., 2026-01*
21. [SAGE: Framework of Precise Retrieval for RAG](https://hf.co/papers/2503.01713) — *Zhang et al., 2025-03*
22. [Hierarchical Abstract Tree for Cross-Document RAG (Ψ-RAG)](https://hf.co/papers/2605.00529) — *Zhao et al., 2026-05*
23. [ReTreever: Tree-based Coarse-to-Fine Representations](https://hf.co/papers/2502.07971) — *Gupta et al., 2025-02*
24. [Augmenting QA with Hybrid RAG (Structured-Semantic RAG)](https://hf.co/papers/2601.12658) — *Yang et al., 2026-01*

#### Latence et performance
25. [TurboRAG: Precomputed KV Caches](https://hf.co/papers/2410.07590) — *Lu et al., 2024-10*
26. [Stream2LLM: Overlap Streaming Prefill](https://hf.co/papers/2604.16395) — *Bachkaniwala et al., 2026-04*
27. [RAGServe: Quality-Aware Configuration Adaptation](https://hf.co/papers/2412.10543) — *Ray et al., 2024-12*
28. [L-RAG: Entropy-Based Lazy Loading](https://hf.co/papers/2601.06551) — *Voloshyn, 2026-01*

#### Routage LLM (gros vs petit)
29. [RouteLLM: Learning to Route LLMs with Preference Data](https://hf.co/papers/2406.18665) — *Ong et al., 2024-06*
30. [CARROT: Cost Aware Rate Optimal Router](https://hf.co/papers/2502.03261) — *Somerstep et al., 2025-02*
31. [R2-Router: Routing with Reasoning](https://hf.co/papers/2602.02823) — *Xue et al., 2026-02*
32. [Router-R1: Multi-Round Routing via RL](https://hf.co/papers/2506.09033) — *Zhang et al., 2025-06*
33. [IRT-Router: Item Response Theory](https://hf.co/papers/2506.01048) — *Song et al., 2025-06*
34. [Universal Model Routing](https://hf.co/papers/2502.08773) — *Jitkrittum et al., 2025-02*
35. [Dr.LLM: Dynamic Layer Routing](https://hf.co/papers/2510.12773) — *Heakl et al., 2025-10*

#### Distillation et continual learning
36. [TinyLLM: Learning a Small Student from Multiple LLMs](https://hf.co/papers/2402.04616) — *Tian et al., 2024-02*
37. [GOLD: Generalized Knowledge Distillation via OOD Generation](https://hf.co/papers/2403.19754) — *Gholami et al., 2024-03*
38. [Knowledge Distillation of Large Language Models (MiniLLM)](https://hf.co/papers/2306.08543) — *Gu et al., 2023-06*
39. [ALAS: Autonomous Learning Agent for Self-Updating LLMs](https://hf.co/papers/2508.15805) — *Atreja, 2025-08*

### Repos GitHub clés

#### Frameworks utilisables directement
- [ggml-org/llama.cpp](https://github.com/ggml-org/llama.cpp) — inférence locale
- [GradientHQ/Symphony](https://github.com/GradientHQ/Symphony) — multi-agent decentralized (MIT)
- [explodinggradients/ragas](https://github.com/explodinggradients/ragas) — eval RAG
- [confident-ai/deepeval](https://github.com/confident-ai/deepeval) — eval unit-test
- [unslothai/unsloth](https://github.com/unslothai/unsloth) — LoRA fine-tuning rapide
- [huggingface/smolagents](https://github.com/huggingface/smolagents) — référence agents minimal

#### Repos d'inspiration code
- [jjovalle99/Speculative-RAG](https://github.com/jjovalle99/Speculative-RAG) — repro Speculative RAG
- [HKUDS/LightRAG](https://github.com/HKUDS/LightRAG) — RAG + graph
- [RUC-NLPIR/FlashRAG](https://github.com/RUC-NLPIR/FlashRAG) — toolkit recherche RAG
- [DEEP-PolyU/Awesome-GraphRAG](https://github.com/DEEP-PolyU/Awesome-GraphRAG) — curation graph RAG
- [stanfordnlp/dspy](https://github.com/stanfordnlp/dspy) — optimisation prompts

#### Modèles HF clés
- [HuggingFaceTB/SmolLM2-360M-Instruct](https://hf.co/HuggingFaceTB/SmolLM2-360M-Instruct)
- [Qwen/Qwen2.5-0.5B-Instruct](https://hf.co/Qwen/Qwen2.5-0.5B-Instruct)
- [meta-llama/Llama-3.2-1B-Instruct](https://hf.co/meta-llama/Llama-3.2-1B-Instruct)
- [microsoft/Phi-3.5-mini-instruct](https://hf.co/microsoft/Phi-3.5-mini-instruct)
- [BAAI/bge-m3](https://hf.co/BAAI/bge-m3)
- [BAAI/bge-reranker-v2-m3](https://hf.co/BAAI/bge-reranker-v2-m3)
- [mixedbread-ai/mxbai-embed-large-v1](https://hf.co/mixedbread-ai/mxbai-embed-large-v1)

#### Docs et ressources
- [llama.cpp server README](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md)
- [LanceDB hybrid search docs](https://lancedb.com/docs/search/hybrid-search/)
- [FastAPI SSE tutorial](https://fastapi.tiangolo.com/tutorial/server-sent-events/)
- [sse-starlette](https://pypi.org/project/sse-starlette/)
- [HuggingFace Agents Course](https://github.com/huggingface/agents-course)

---

## 10. Décisions prises grâce à cette recherche

| Avant la recherche | Après la recherche |
|---|---|
| L2 = 5 scouts indépendants | L2 = 5 scouts dont les inputs sont **K-means clusterisés** (Speculative RAG pattern) |
| Stigmergie inventée | Stigmergie **inspirée de SwarmSys** : exploration + exploitation + validation cycle |
| Modèle drafter à choisir | **Qwen2.5-0.5B confirmé** pour L1/L2, **SmolLM2-360M** pour L0 |
| Reranker à choisir | **bge-reranker-v2-m3** confirmé (12.5 M téléchargements, ONNX, multilingue) |
| Embedding à choisir | **bge-m3** confirmé (28.7 M téléchargements, multilingue natif) |
| TurboRAG-style KV cache à implémenter | Reporté V2 — **prompt caching Anthropic** suffit pour V1 |
| Distillation loop à inventer | **Online Speculative Decoding** donne la méthode, **unsloth** donne l'outil |
| Cadre à écrire en partant de zéro | **Symphony repo** donne la structure de packages (`agents/`, `core/`, `protocol/`...) |
| Eval à improviser | **Ragas** comme socle, **DeepEval** pour CI (couvre faithfulness, context precision, answer relevancy out-of-the-box) |
| Vocabulaire L0/L2/L5 | **Explorer/Worker/Validator** (vocabulaire SwarmSys, plus parlant) |

---

## En une phrase

> *Scout & Synthesize* repose à 70 % sur des briques open-source matures (modèles SmolLM2/Qwen2.5/Llama-3.2/Phi-3.5, embeddings bge-m3, reranker bge-reranker-v2-m3, llama.cpp server, LanceDB, Ragas, Anthropic SDK) et à 30 % sur du code propre (~1500 LOC) centré sur l'orchestration asyncio, la stigmergie sur capsules, la spéculation pendant frappe et la boucle de distillation hebdomadaire — patrons tous formalisés par des publications de 2024–2025 (Speculative RAG, SwarmSys, Symphony, Online Speculative Decoding).
