# Architecture — *Scout & Synthesize*

> Essaim de nano‑agents pour la recherche / analyse documentaire à faible latence, en déploiement hybride edge + cloud.

Document de référence. Lis‑le comme un *blueprint* implémentable : chaque section est conçue pour qu'un ingénieur puisse commencer à coder à partir d'elle sans avoir à re‑décider l'essentiel.

---

## Table des matières

0. [Cadre, contraintes, objectifs](#0-cadre-contraintes-et-objectifs-mesurables)
1. [Vue système globale](#1-vue-système-globale)
2. [Topologie de processus](#2-topologie-de-processus)
3. [Architecture par couche L0 → L5](#3-architecture-détaillée-par-couche)
4. [Architecture des données](#4-architecture-des-données)
5. [Plan de contrôle et streaming](#5-plan-de-contrôle-et-streaming)
6. [Spéculation pendant la frappe](#6-spéculation-pendant-la-frappe)
7. [Observabilité](#7-observabilité)
8. [Évaluation et boucle de distillation](#8-évaluation-et-boucle-de-distillation)
9. [Sécurité, confidentialité, conformité](#9-sécurité-confidentialité-conformité)
10. [Structure de fichiers du projet](#10-structure-de-fichiers-du-projet)
11. [Plan d'implémentation](#11-plan-dimplémentation-8-semaines)
12. [Stack avec versions](#12-stack-avec-versions-précises-recommandation-j0)
13. [Décisions ouvertes](#13-décisions-ouvertes-à-trancher)
14. [Risques et anti‑patterns](#14-risques-et-anti-patterns-à-surveiller-dès-le-j1)

---

## 0. Cadre, contraintes et objectifs mesurables

### Objectifs (par ordre de priorité)
1. **Latence perçue** : `time_to_first_useful_pixel` p50 < 250 ms, p99 < 500 ms
2. **Latence à la réponse** : `time_to_first_token` p50 < 1.0 s, p99 < 2.0 s
3. **Coût cloud** : ≤ 30 % d'un appel naïf (gros LLM seul avec tous les docs)
4. **Qualité** : *faithfulness* ≥ baseline naïve sur le golden set (cf. §8)

### Contraintes
- Hybride : nanos sur edge (Mac M‑series ou serveur Linux CPU/iGPU), gros LLM en API (Claude)
- Modèles edge ≤ 4 B paramètres, tous en mémoire simultanément (~12 Go RAM)
- Aucune sortie nano ne quitte la machine sans passer par L3 (filtre PII optionnel)
- Tout doit fonctionner en mode dégradé si le cloud tombe (failover local)

### Non‑objectifs (à acter explicitement)
- ❌ Multi‑utilisateurs concurrents massifs (cible : 1 user / 10 req·min en local)
- ❌ Indexation de corpus à chaud massif (préférer batch offline)
- ❌ Multi‑tour conversationnel complexe (V1 = QA, V2 = conversation)
- ❌ Génération de contenu créatif (cible : recherche/synthèse)

---

## 1. Vue système globale

```
┌────────────────────────────────────────────────────────────────────────────┐
│                              EDGE (machine locale)                         │
│                                                                            │
│  ┌──────────────┐   ┌──────────────────────────────────────────────────┐   │
│  │   UI (Web)   │◄──┤  Frontend SSE Stream                              │   │
│  └──────┬───────┘   └──────────────────────────────────────────────────┘   │
│         │ HTTP/SSE                                                         │
│  ┌──────▼─────────────────────────────────────────────────────────────┐    │
│  │            ORCHESTRATEUR  (Python asyncio, 1 process)              │    │
│  │  ┌──────┐ ┌──────┐ ┌──────────┐ ┌─────────────┐ ┌──────────┐       │    │
│  │  │ L0   │ │ L1   │ │ L2 swarm │ │ L3 aggreg.  │ │ L5 crit. │       │    │
│  │  │ Intent│ │ Plan│ │ N scouts │ │ Build prompt│ │ Fact-chk │       │    │
│  │  └──┬───┘ └──┬───┘ └─────┬────┘ └──────┬──────┘ └────┬─────┘       │    │
│  └─────┼────────┼───────────┼─────────────┼─────────────┼─────────────┘    │
│        │        │           │             │             │                  │
│  ┌─────▼────────▼───────────▼─────────────▼─────────────▼─────────────┐    │
│  │   INFERENCE PLANE                                                  │    │
│  │   ┌──────────────────────────┐  ┌──────────────────────────────┐   │    │
│  │   │ llama.cpp server         │  │  ONNX Runtime                │   │    │
│  │   │  • SmolLM2-360M  (slot0) │  │  • bge-reranker-base         │   │    │
│  │   │  • Qwen2.5-0.5B  (slot1) │  │  • bge-small (embeddings)    │   │    │
│  │   │  • Llama-3.2-1B  (slot2) │  │                              │   │    │
│  │   │  • Phi-3.5-mini  (slot3) │  └──────────────────────────────┘   │    │
│  │   └──────────────────────────┘                                     │    │
│  └────────────────────────────────────────────────────────────────────┘    │
│                                                                            │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │   DATA PLANE                                                        │   │
│  │   ┌──────────────┐  ┌────────────┐  ┌──────────┐  ┌─────────────┐   │   │
│  │   │ LanceDB      │  │ Redis      │  │ Trace DB │  │ Distill log │   │   │
│  │   │  - chunks    │  │  - L0 cache│  │ (Tempo)  │  │ (parquet)   │   │   │
│  │   │  - capsules  │  │  - L2 cache│  │          │  │             │   │   │
│  │   │  - embeddings│  │  - sessions│  │          │  │             │   │   │
│  │   └──────────────┘  └────────────┘  └──────────┘  └─────────────┘   │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
└──────────────────────────────────┬─────────────────────────────────────────┘
                                   │ HTTPS (Claude API)
                                   │ prompt caching enabled
                                   ▼
                          ┌────────────────────┐
                          │   CLAUDE API       │
                          │   (cloud)          │
                          └────────────────────┘
```

**Trois plans logiques** :
- **Control plane** : orchestrateur async + UI streaming
- **Inference plane** : llama.cpp server + ONNX Runtime (les "GPU/CPU workers")
- **Data plane** : LanceDB (vectoriel persistant) + Redis (cache éphémère) + Tempo (traces) + Parquet (logs distillation)

---

## 2. Topologie de processus

Décision clé : **3 processus**, pas plus.

| Process | Contenu | Pourquoi séparé |
|---|---|---|
| `app-orchestrator` | Python asyncio, HTTP+SSE, logique L0–L5 | Logique métier, hot‑reload aisé |
| `llama-server` | llama.cpp en mode serveur, 4 modèles loadés | Inférence partagée, batching multi‑slot |
| `redis` | cache L0/L2 + sessions | Persistance optionnelle, expiration TTL |

Tout le reste (LanceDB, ONNX) tourne **in‑process** dans `app-orchestrator` :
- LanceDB est *embedded* (pas de daemon)
- ONNX Runtime pour reranker/embeddings : in‑process, batch synchrone via `asyncio.to_thread`

**Pourquoi pas plus de processus ?** Chaque IPC ajoute 0.2–2 ms. Sur un budget de 200 ms côté edge, c'est précieux. La séparation `llama-server` est nécessaire (autre langage, gestion mémoire spécifique) ; le reste tient en Python.

### Configuration `llama-server`

```bash
# Démarrage typique — 1 serveur par modèle, sur ports 8080–8083
llama-server \
  -m /models/smollm2-360m-instruct.Q5_K_M.gguf \
  --parallel 4 \
  --ctx-size 4096 \
  --threads 6 \
  --port 8080 \
  --metrics
```

Avec 4 modèles, on lance 4 serveurs sur 4 ports (8080–8083) — plus simple que la multi‑model dans un seul serveur. Mémoire totale ~10–12 Go sur Mac (mmap + GGUF Q5).

---

## 3. Architecture détaillée par couche

### L0 — Compréhension de requête *(budget : 30 ms)*

**Rôle** : extraire l'*intention*, classer le type, vérifier le cache.

| Champ | Valeur |
|---|---|
| Modèle | `SmolLM2-360M-Instruct` (Q5_K_M, ~250 Mo) |
| Inférence | llama.cpp server port 8080, contexte 1024 |
| Entrée | `{"query": str, "history": list[Turn]}` |
| Sortie (JSON contraint) | `IntentResult` (cf. schéma) |
| Latence cible | p50 15 ms, p99 30 ms |
| Mode d'échec | timeout 50 ms → fallback `type=GENERAL, complexity=MEDIUM` |

**Schéma de sortie** (Pydantic, imposé par llama.cpp grammar JSON) :

```python
class IntentResult(BaseModel):
    type: Literal["FACTUAL", "ANALYTICAL", "COMPARATIVE", "TEMPORAL", "META"]
    complexity: Literal["TRIVIAL", "SIMPLE", "MEDIUM", "COMPLEX"]
    entities: list[str]              # named entities détectées
    time_scope: Optional[str]         # "recent", "historical", "all"
    requires_synthesis: bool          # true → besoin du gros LLM
    cache_key: str                    # hash sémantique normalisé
```

**Prompt** (court, en grammaire forcée, pas de chain‑of‑thought) :

```
Classifie cette requête en JSON. Pas de prose. Type, complexité, entités.
QUERY: "{{query}}"
JSON:
```

**Cache L0** : on hash l'embedding de la query (bge‑small) ; si similarité ≥ 0.95 avec une entrée < 5 min : on retourne la réponse cacheée → court‑circuit total.

---

### L1 — Planification du rétrieval *(budget : 30 ms)*

**Rôle** : décider *quelles sources* interroger et *comment décomposer* en K sous‑requêtes parallèles.

| Champ | Valeur |
|---|---|
| Modèle | `Qwen2.5-0.5B-Instruct` (Q5_K_M, ~400 Mo) |
| Inférence | port 8081 |
| Entrée | `IntentResult` + métadonnées corpus disponibles |
| Sortie | `RetrievalPlan` |
| Latence cible | p50 25 ms, p99 50 ms |

**Schéma** :

```python
class SubQuery(BaseModel):
    text: str                    # sous-question reformulée
    angle: str                   # "definition", "evidence", "counter", "history"
    source_filter: Optional[dict] # ex {"corpus": "papers", "year_gte": 2020}
    top_k: int = 5

class RetrievalPlan(BaseModel):
    sub_queries: list[SubQuery]  # 3 à 8 sous-queries
    fusion_strategy: Literal["rrf", "weighted", "diversity"]
    context_budget_tokens: int   # 2000–6000 selon complexité
```

**Décision clé** : K=5 par défaut. Si `complexity=COMPLEX` → K=8. Si `TRIVIAL` → K=1 (et on saute L2/L3 → direct cache template). L'angle (`evidence`/`counter`/etc.) permet une *diversité forcée* — chaque scout cherche un angle différent.

---

### L2 — Essaim d'éclaireurs *(budget : 80 ms en parallèle)*

**Rôle** : *N scouts indépendants* effectuent retrieval + rerank + extraction de "capsule".

#### Pipeline d'un scout (en parallèle des autres)

```
SubQuery
   │
   ▼
[Embedding bge-small]              (~2 ms)
   │
   ▼
[LanceDB ANN search, top 20]       (~5 ms)
   │
   ▼
[bge-reranker-base, top 20 → 5]    (~15 ms)
   │
   ▼
[Qwen2.5-0.5B: extract capsule]    (~40 ms, slot 1 batched)
   │
   ▼
ContextCapsule
```

**Schéma de capsule** (le format pivot du système) :

```python
class ContextCapsule(BaseModel):
    capsule_id: str               # uuid7
    sub_query: str                # la sous-question d'origine
    source_id: str                # doc identifier
    source_meta: dict             # title, date, url, page, …
    relevance_score: float        # 0..1, du reranker
    
    # 3 niveaux de compression (l'agrégateur choisit)
    level_1_oneline: str          # ≤ 20 mots
    level_2_summary: str          # ≤ 80 mots
    level_3_extracts: list[Extract]  # citations littérales avec offsets
    
    # support de stigmergie
    pheromone_weight: float       # incrémenté à chaque usage utile
    last_seen_ts: float
    
    # support pour critique L5
    factual_claims: list[str]     # liste d'affirmations extraites

class Extract(BaseModel):
    text: str
    char_start: int
    char_end: int
    confidence: float
```

**Détail critique** : la capsule est *l'unité de transport* dans le système. Tout ce qui passe entre L2 et L4 est une liste de capsules. Le format est figé — c'est ce qui permet à chaque composant d'évoluer indépendamment.

**Batching** : les 5 scouts appellent `Qwen2.5-0.5B` en parallèle. Avec llama.cpp `--parallel 4`, on traite 4 prompts en parallèle dans un seul forward pass — gain ~3× vs séquentiel.

**Stigmergie** : chaque capsule produite est écrite dans LanceDB avec une `pheromone_weight`. Si la même capsule est récupérée dans une future requête, son poids augmente. Les capsules à fort poids "remontent" naturellement dans les classements. Décroissance exponentielle avec λ=1/24h pour éviter la fossilisation.

---

### L3 — Agrégateur *(budget : 50 ms)*

**Rôle** : dédupliquer, ordonner, calibrer le budget de tokens, construire le prompt final pour le cloud.

| Champ | Valeur |
|---|---|
| Modèle | `Llama-3.2-1B-Instruct` (Q5_K_M, ~800 Mo) |
| Inférence | port 8082 |
| Entrée | `list[ContextCapsule]` + `IntentResult` + `RetrievalPlan` |
| Sortie | `CloudPrompt` |
| Latence cible | p50 40 ms, p99 80 ms |

**Étapes internes** :

1. **Déduplication sémantique** : embedding de chaque `level_2_summary` ; clusters par cosine > 0.88 ; on garde la capsule de plus haut `relevance_score` par cluster. *(~5 ms, pas de LLM)*
2. **Sélection de niveau** : selon `context_budget_tokens` et `complexity`, on choisit pour chaque capsule le niveau 1, 2 ou 3 (mix possible). Algorithme glouton : commencer en level_1, monter en level_2/3 pour les capsules de plus fort score jusqu'à saturation du budget.
3. **Ordonnancement** : tri par diversité d'angle + score décroissant.
4. **Construction du prompt** : template avec sections claires (system, sources, question). Le LLM 1B *valide* la cohérence et reformule légèrement, mais ne génère pas — c'est essentiellement une **fonction déterministe avec sanity check LLM**.

**Schéma** :

```python
class CloudPrompt(BaseModel):
    system: str                   # cacheable côté Anthropic
    sources_block: str            # markdown formaté, citations [^id]
    question: str                 # query reformulée si besoin
    expected_format: str          # "answer + citations"
    capsules_used: list[str]      # IDs pour traçabilité
    total_tokens_estimate: int
```

**Format du `sources_block`** (un détail qui change tout pour la qualité finale) :

```markdown
## Sources

[^src-a1] "Federal Reserve maintained rates at 5.25–5.50%..."  
  — *FOMC Statement*, 2024-03-20, p.2

[^src-a2] (résumé) The committee signaled three cuts in 2024...  
  — *FOMC Statement*, 2024-03-20

[^src-b1] (one-line) GDP grew 3.3% Q4 2023 — BEA, 2024-01-25
```

Le gros LLM cite via `[^src-a1]` ; le frontend rend les citations cliquables (popup avec capsule complète). **C'est ainsi qu'on ferme la boucle de provenance.**

---

### L4 — Gros LLM (cloud)

**Rôle** : raisonnement, synthèse, génération de la réponse avec citations.

| Champ | Valeur |
|---|---|
| Modèle | Claude (Opus pour qualité, Sonnet pour ratio coût/latence) |
| Mode | streaming via SSE |
| `system` | mis en **prompt caching** (TTL 5 min, gratuit après 1er hit) |
| Premier token cible | < 900 ms p50 |

**Stratégie de prompt caching** : tout ce qui est stable entre requêtes (system prompt, format instructions, parfois un corpus de "documents de référence" fréquemment cités) va dans le bloc cached. Pour ce use case, **le gros gain vient du cache si l'utilisateur enchaîne plusieurs questions sur le même corpus** — typique en mode "research session".

**Fallback local** : si `time_to_first_token` > 1500 ms ou erreur API, l'orchestrateur **bascule sur Llama‑3.2‑1B local** (port 8082) en utilisant le même `CloudPrompt`. Qualité moindre, mais réponse garantie. L'UI affiche un badge "📍 réponse locale — [régénérer avec Claude]".

---

### L5 — Critique factuelle *(budget : streamé en parallèle)*

**Rôle** : pour chaque affirmation dans la réponse streamée, vérifier qu'elle est supportée par les capsules.

| Champ | Valeur |
|---|---|
| Modèle | `Phi-3.5-mini-Instruct` (Q4_K_M, ~2.2 Go) |
| Inférence | port 8083 |
| Entrée | (chunk de réponse, capsules pertinentes) |
| Sortie | `CritiqueAnnotations` |

**Algorithme** :

1. Buffer la réponse par phrases (`.` + heuristiques)
2. Pour chaque phrase : extraire les claims factuelles (sub‑task rapide via Phi)
3. Pour chaque claim : embedding ; recherche dans les capsules ; si meilleur match < seuil → marquer "non sourcée"
4. Émettre événements SSE `critique` avec offsets ; le frontend surligne

**Schéma** :

```python
class CritiqueAnnotation(BaseModel):
    sentence_start: int          # offset dans la réponse complète
    sentence_end: int
    claims: list[Claim]

class Claim(BaseModel):
    text: str
    supported: bool
    best_capsule_id: Optional[str]
    similarity: float
    severity: Literal["INFO", "WARN", "ERROR"]
```

C'est *non bloquant* : la réponse s'affiche à vitesse normale, les annotations viennent par‑dessus comme des surlignages. Si Phi est trop lent, on dégrade gracieusement (sans annotations) plutôt que de bloquer.

---

## 4. Architecture des données

### 4.1 Pipeline d'ingestion (offline)

```
Documents bruts
   │
   ▼
[Parse]  ──► PDF / HTML / DOCX / MD → texte structuré
   │
   ▼
[Chunk hiérarchique]
   ├─ section (≤ 2000 tokens)
   ├─ paragraph (≤ 300 tokens)
   └─ sentence (≤ 50 tokens)
   │
   ▼
[Embed]  ──► bge-small/bge-m3 par niveau
   │
   ▼
[Index LanceDB]
   ├─ table `chunks_section`   (vec, text, doc_id, meta)
   ├─ table `chunks_paragraph` (vec, text, doc_id, section_id, meta)
   └─ table `chunks_sentence`  (vec, text, doc_id, paragraph_id, meta)
   │
   ▼
[Métadonnées]  ──► table `documents` (title, date, url, hash, ingest_ts)
```

**Chunking — stratégie détaillée** :

- **Section** : découpe sur titres `H1/H2/H3`, fallback sur 2000 tokens
- **Paragraph** : blocs séparés par double newline, fallback 300 tokens avec **overlap 50 tokens**
- **Sentence** : `nltk.sent_tokenize` ou `spaCy` (selon langue)

**Pourquoi 3 niveaux ?** Pour le retrieval initial on cherche au niveau *paragraph* (le sweet spot recall/précision). Pour la diversité, on remonte en *section* (un seul résultat par section, sinon redondance). Pour les citations littérales, on descend en *sentence*.

### 4.2 Schémas LanceDB

```python
# table: chunks_paragraph
{
    "chunk_id": "uuid7",
    "doc_id": "str",
    "section_id": "uuid7",
    "text": "str",                    # contenu
    "vector": "vector(384)",          # bge-small dim
    "token_count": "int",
    "char_start": "int",
    "char_end": "int",
    "meta": "json",                   # title, page, date, ...
    
    # stigmergie
    "pheromone": "float",             # default 0.0
    "last_useful_ts": "timestamp",    # màj quand une capsule de ce chunk est citée
    "use_count": "int",
}

# table: capsules_cache  (résultats L2 mémoïsés)
{
    "capsule_id": "uuid7",
    "sub_query_hash": "str",          # hash sémantique de la sub-query
    "sub_query_embedding": "vector(384)",
    "capsule_json": "str",            # ContextCapsule sérialisée
    "created_ts": "timestamp",
    "ttl_ts": "timestamp",
    "hit_count": "int",
}

# table: documents
{
    "doc_id": "uuid7",
    "title": "str",
    "url": "str",
    "content_hash": "str",            # sha256 du contenu brut
    "ingest_ts": "timestamp",
    "lang": "str",
    "n_sections": "int",
    "n_paragraphs": "int",
    "n_sentences": "int",
}
```

### 4.3 Architecture du cache

Trois étages de cache **distincts** :

| Cache | Stockage | Clé | TTL | Hit rate cible |
|---|---|---|---|---|
| **L0** (réponses complètes) | Redis | `hash(embedding(query))` arrondi | 5 min | 30–70 % en session |
| **L2** (capsules) | LanceDB `capsules_cache` | `hash(embedding(sub_query))` | 1 h | 40–60 % |
| **L4** (Anthropic prompt cache) | Côté API | system + sources stables | 5 min (extensible) | dépend usage |

**Clé sémantique** (pour L0 et L2) : on quantize l'embedding à 8 bits par dimension puis hash. Deux queries quasi‑identiques tombent sur la même clé. Si miss exact, on fait une *similarity search* sur les clés récentes (top‑1, seuil 0.95).

### 4.4 Logs et traces

```
~/vrai/data/
├── lance/                    # LanceDB tables
│   ├── chunks_section.lance
│   ├── chunks_paragraph.lance
│   ├── chunks_sentence.lance
│   ├── documents.lance
│   └── capsules_cache.lance
├── traces/                   # OpenTelemetry → fichier JSON ou Tempo
│   └── 2026-05-23/*.jsonl
└── distill/                  # paires (input, claude_correction)
    └── 2026-05-W21.parquet
```

---

## 5. Plan de contrôle et streaming

### 5.1 Protocole UI ↔ orchestrateur

**SSE** avec types d'événements sémantiques :

```typescript
type StreamEvent =
  | { type: "ack",       request_id: string, trace_id: string }
  | { type: "progress",  layer: "L0"|"L1"|"L2"|"L3"|"L4", status: string, t_ms: number }
  | { type: "intent",    payload: IntentResult }
  | { type: "plan",      payload: RetrievalPlan }
  | { type: "source",    payload: ContextCapsule }    // une par scout
  | { type: "prompt",    tokens_used: number }
  | { type: "token",     text: string }
  | { type: "critique",  payload: CritiqueAnnotation }
  | { type: "done",      stats: RequestStats }
  | { type: "error",     code: string, message: string, recoverable: boolean }
```

Le frontend peut afficher **chaque type d'événement** comme un signal de progression distinct. C'est ce qui crée l'illusion de vitesse.

### 5.2 Annulation et backpressure

- Chaque requête a un `request_id` et un `asyncio.CancelledError` propagé
- Si l'utilisateur soumet une nouvelle requête : `cancel(previous_request_id)` → fermeture du stream, abandon des appels llama.cpp via le endpoint `POST /slot/{id}/erase`, abandon HTTP Claude (timeout court)
- Backpressure naturelle : si le client SSE ralentit, l'orchestrateur ralentit son `yield`, qui ralentit la lecture du stream Claude — pas besoin de logique custom

### 5.3 Concurrence interne

Le DAG est :

```
L0 ──┐
     ▼
     L1 ──► [L2 × N] ──► L3 ──► L4 (stream) ──┐
                                              ├─► UI
                                       L5 ────┘
```

Implémentation (pseudocode) :

```python
async def handle_request(query: str, send: Callable):
    ctx = RequestContext(query)
    
    intent = await L0.run(ctx);                 await send("intent", intent)
    plan   = await L1.run(ctx, intent);         await send("plan", plan)
    
    # L2 en parallèle, streaming des sources dès qu'une capsule est prête
    capsules = []
    async for cap in L2.run_parallel(plan):
        capsules.append(cap)
        await send("source", cap)
    
    cloud_prompt = await L3.run(intent, plan, capsules)
    await send("prompt", {"tokens": cloud_prompt.total_tokens_estimate})
    
    # L4 + L5 en concurrence
    critique_task = asyncio.create_task(L5.run_alongside(cloud_prompt, send))
    
    async for tok in L4.stream(cloud_prompt):
        await send("token", tok)
    
    await critique_task
    await send("done", ctx.stats())
```

---

## 6. Spéculation pendant la frappe

### 6.1 Stratégie

L'UI émet un événement `query_change` à chaque caractère. Côté orchestrateur, un *speculator* gère ces événements :

```
keystroke
   │
   ▼
[Debounce 250 ms]
   │
   ▼
[Embedding rapide bge-small] (~3 ms)
   │
   ▼
[Diff vs dernière spéculation]
   │  si cos_sim > 0.85 ────► no-op (rien changé sémantiquement)
   │  sinon
   ▼
[Spawn task L0+L1] (en arrière-plan, faible priorité)
   │
   ▼
[Stocker résultat dans speculative_cache, clé = embedding]
```

À la soumission finale (Enter), on vérifie si la query soumise correspond à une spéculation en cache : si oui, on saute L0+L1 directement (~60 ms gagnés).

### 6.2 Coût et limite

- **Coût** : ~5 spéculations jetées par query soumise en moyenne. Sur edge, négligeable (CPU sinon idle).
- **Limite** : on n'exécute *jamais* L2 spéculativement par défaut (trop cher en LLM cycles). Option : activer L2 spéculatif après 800 ms d'inactivité de frappe, pour les power users.

### 6.3 Pré‑fetch en *session*

Au‑delà de la frappe, on pré‑fetch aussi en session :

- À la fin d'une réponse, le gros LLM propose 3 follow‑up questions (déjà commun).
- L'orchestrateur lance L1+L2 *en arrière‑plan* pour ces 3 questions, en faible priorité.
- Si l'utilisateur clique sur l'une → réponse perçue en ~600 ms (juste L4).

---

## 7. Observabilité

### 7.1 Modèle de traces

Une trace par requête, span par couche :

```
trace_id: 01H...
└─ span: request                            t=0, d=2.4s
   ├─ span: L0.intent                       t=0, d=18ms
   │  └─ span: llama.infer slot=0           t=2, d=14ms
   ├─ span: L0.cache_lookup                 t=18, d=2ms
   ├─ span: L1.plan                         t=20, d=26ms
   ├─ span: L2.swarm                        t=46, d=82ms
   │  ├─ span: L2.scout subq=0              t=46, d=78ms
   │  │  ├─ span: embed                     d=2ms
   │  │  ├─ span: lance.search              d=4ms
   │  │  ├─ span: rerank                    d=14ms
   │  │  └─ span: capsule.extract           d=58ms
   │  ├─ span: L2.scout subq=1              ...
   │  └─ ...
   ├─ span: L3.aggregate                    t=128, d=44ms
   ├─ span: L4.cloud                        t=172, d=2.1s
   │  ├─ event: first_token                 t=812
   │  └─ ...
   └─ span: L5.critique                     t=400, d=1.8s (parallel)
```

### 7.2 Métriques (Prometheus‑style)

```
# latence par couche (histogram)
nano_latency_ms{layer="L0|L1|L2|L3|L5"}

# pourcentage de cache hit
cache_hit_rate{cache="L0|L2|L4"}

# escalade : % de requêtes atteignant L4
escalation_rate

# coût cloud par requête
cloud_tokens_input_per_request
cloud_tokens_output_per_request
cloud_cost_usd_per_request

# qualité (depuis L5)
unsourced_claims_per_response
critique_severity{severity="INFO|WARN|ERROR"}

# infra
llama_slot_queue_depth{slot="0..3"}
llama_slot_utilization{slot="0..3"}
```

### 7.3 Tableau de bord cible

Une vue unique par requête, et une vue agrégée :

- **Par requête** : timeline horizontale des spans, capsules utilisées, critique
- **Agrégé** : p50/p95/p99 par couche, hit rate, coût/jour, distrib des escalades

---

## 8. Évaluation et boucle de distillation

### 8.1 Golden set

- 500 paires (question, réponse_de_référence, sources_attendues)
- Sourcé sur un corpus stable
- Catégorisé par : type (factuel/analytique/comparatif), difficulté, ambiguïté

### 8.2 Métriques d'évaluation

| Métrique | Définition | Calcul |
|---|---|---|
| **Faithfulness** | % de claims dans la réponse supportées par les sources | LLM judge (gros) sur sample |
| **Recall** | % de sources attendues présentes dans les capsules L3 | comparaison ID |
| **Precision** | % des capsules L3 qui sont effectivement utiles | LLM judge |
| **Latence_p50/p99** | quantiles `time_to_first_token` | mesure directe |
| **Cost_per_request** | tokens × prix | comptage |
| **Coverage** | % requêtes pour lesquelles on a répondu | comptage |

### 8.3 Pipeline d'évaluation

```bash
# CLI
vrai eval --golden ./eval/golden_v1.jsonl --variant swarm-v0.3 \
  --report ./eval/reports/swarm-v0.3.html
```

Compare automatiquement N variants côte‑à‑côte. Variants typiques :

- `naive_claude` : pas d'essaim, Claude lit tout
- `swarm_v1_no_speculation` : L0–L5 sans pré‑fetch
- `swarm_v1_full` : tout activé
- `swarm_v2_distilled_w21` : avec nanos distillés de la semaine 21

### 8.4 Boucle de distillation (continuous learning)

```
Production
   │
   ▼
Capture: (nano_input, nano_output_v_current, claude_correction_or_score)
   │
   ▼
Hebdomadaire (cron)
   │
   ▼
Curate: filtrer les corrections "fortes" (claude rejette / réécrit)
   │
   ▼
Pour chaque nano (L0, L1, L2-extract, L3, L5):
   ├─ Format en (input, target_output)
   ├─ LoRA fine-tune (rank=8, alpha=16, 1-3 epochs)
   └─ Save adapter
   │
   ▼
A/B test offline sur golden set
   ├─ si delta_quality > +1% ET delta_latency < +10% → promote
   └─ sinon → keep current
   │
   ▼
Hot-swap LoRA adapter en prod (sans restart)
```

Chaque nano s'améliore semaine après semaine, **spécialisé sur les patterns réels** de l'utilisateur ou de l'organisation. C'est l'avantage clé d'avoir des nanos vs un seul gros modèle : tu peux les *spécialiser à ton corpus*.

---

## 9. Sécurité, confidentialité, conformité

| Préoccupation | Réponse |
|---|---|
| Données docs ne doivent pas fuiter au cloud | Les capsules L3 ne contiennent que les *extraits cités*, jamais les docs complets. Optionnel : un nano "PII redactor" supplémentaire avant L4. |
| Clés API | OS keychain (macOS Keychain, Linux secret-tool). Jamais en clair dans `.env`. |
| Logs et traces | Les `traces/*.jsonl` peuvent contenir des queries → chiffrement at‑rest, purge > 30j. |
| Mode "air-gapped" | Mode `--offline` qui désactive L4 et utilise Llama‑3.2‑1B local pour synthèse. Qualité dégradée acceptée. |
| Audit | Chaque requête a un `trace_id` ; tous les events (capsules consultées, sources citées) sont loggés → audit complet possible. |

---

## 10. Structure de fichiers du projet

```
vrai/
├── pyproject.toml
├── README.md
├── ARCHITECTURE.md              # ce document
├── config/
│   ├── default.yaml             # config par défaut
│   └── local.yaml               # surcharges locales (gitignored)
├── src/vrai/
│   ├── __init__.py
│   ├── main.py                  # entrypoint FastAPI / uvicorn
│   ├── orchestrator.py          # pipeline L0→L5
│   ├── context.py               # RequestContext
│   ├── speculator.py            # spéculation pendant frappe
│   │
│   ├── layers/
│   │   ├── l0_intent.py
│   │   ├── l1_plan.py
│   │   ├── l2_scout.py
│   │   ├── l3_aggregate.py
│   │   ├── l4_cloud.py
│   │   ├── l5_critique.py
│   │   └── base.py              # Layer ABC
│   │
│   ├── models/                  # data models (Pydantic)
│   │   ├── intent.py
│   │   ├── plan.py
│   │   ├── capsule.py
│   │   ├── prompt.py
│   │   └── critique.py
│   │
│   ├── inference/
│   │   ├── llama_client.py      # client llama.cpp avec slots
│   │   ├── reranker.py          # ONNX wrapper bge-reranker
│   │   ├── embedder.py          # ONNX wrapper bge-small
│   │   └── anthropic_client.py  # SDK + caching
│   │
│   ├── data/
│   │   ├── lance_store.py       # interface LanceDB
│   │   ├── redis_cache.py
│   │   ├── ingestion.py         # offline doc → chunks
│   │   └── chunking.py
│   │
│   ├── stream/
│   │   ├── sse.py
│   │   └── events.py            # types d'événements
│   │
│   ├── telemetry/
│   │   ├── tracing.py           # OpenTelemetry setup
│   │   └── metrics.py           # Prometheus
│   │
│   └── eval/
│       ├── golden.py
│       ├── judge.py             # LLM-as-judge
│       └── runner.py
│
├── frontend/                    # React/Svelte/Solid, SSE client
│   └── ...
│
├── scripts/
│   ├── ingest.py                # CLI d'ingestion
│   ├── eval.py                  # CLI d'éval
│   ├── distill.py               # cron de distillation
│   └── benchmark.py
│
├── models/                      # gguf binaires (gitignored)
│   ├── smollm2-360m.gguf
│   ├── qwen2.5-0.5b.gguf
│   ├── llama-3.2-1b.gguf
│   ├── phi-3.5-mini.gguf
│   └── adapters/                # LoRA par nano
│
├── data/
│   ├── lance/                   # vectoriel
│   ├── traces/
│   └── distill/
│
└── tests/
    ├── unit/
    ├── integration/
    └── eval/
        └── golden_v1.jsonl
```

---

## 11. Plan d'implémentation (8 semaines)

| Sem | Livrable | Critère d'acceptation |
|---|---|---|
| **1** | Squelette : ingestion + retrieval + Claude (pas de nanos) | Une question → réponse en streaming, baseline mesurée |
| **2** | L2 (scouts + reranker) | Top 5 capsules dédupliquées arrivent au cloud, qualité ≥ baseline |
| **3** | L0 + L1 + L3 (les 3 nanos clés) | Pipeline complet, latence p50 ≤ 1.5s |
| **4** | Cache L0/L2 + prompt caching Anthropic | TTFB p50 < 1s sur 2e question |
| **5** | Spéculation pendant frappe + UI SSE complète | TTUPE (useful pixel) p50 < 250ms |
| **6** | L5 critique + annotations dans l'UI | Claims non‑sourcées surlignées |
| **7** | Observabilité complète + golden set + éval CLI | Dashboard fonctionnel, 500 cas testés |
| **8** | Distillation : capture + 1er cycle LoRA | Au moins 1 nano amélioré mesurablement |

**Règle d'or** : ne pas avancer à la semaine N+1 si les métriques de N ne sont pas mesurées et stables.

---

## 12. Stack avec versions précises (recommandation J‑0)

| Catégorie | Choix | Version | Notes |
|---|---|---|---|
| Langage principal | Python | 3.12 | asyncio mature, types |
| Web framework | FastAPI | ≥ 0.110 | SSE natif |
| Async | uvicorn + httpx | dernières | HTTP/2 pour Claude |
| Validation | Pydantic | v2 | perf + JSON schema |
| Inférence locale | llama.cpp | release récente | gguf, slots, JSON grammar |
| Embeddings/rerank | fastembed | dernière | ONNX, multi‑plateforme |
| Vectoriel | LanceDB | ≥ 0.10 | embedded, fast |
| Cache | Redis | 7.x | TTL, pubsub si besoin |
| Cloud LLM | anthropic SDK | dernière | prompt caching activé |
| Observabilité | opentelemetry-python | dernière | export OTLP |
| Tests | pytest + pytest‑asyncio | dernières | |
| Lint | ruff | dernière | |
| Frontend | Svelte ou React + SSE | au choix | léger préférable |

---

## 13. Décisions ouvertes à trancher

Voici les **8 décisions** qui restent à acter pour passer du blueprint au code :

1. **Quel corpus initial** ? Volume estimé (Go), types de docs (PDF, HTML, structuré ?), langue(s).
2. **UI à construire ou intégrer dans un existant** (Obsidian, VSCode, app standalone) ?
3. **Multi‑langue** ou un seul ? Influence le choix d'embeddings (`bge-small-en` vs `bge-m3`).
4. **Niveau de personnalisation** : un seul user (toi), équipe, ou public ?
5. **Type de gros LLM** confirmé : Claude Opus 4.x, Sonnet, ou choix dynamique ?
6. **Cible matérielle** : Mac M2/M3 (Metal), serveur Linux CPU, ou GPU NVIDIA ?
7. **Tolérance au lag de la distillation** : OK pour version "zero‑shot" 2–3 mois, ou besoin de distiller dès semaine 3 ?
8. **Frontend SSE custom** ou utiliser `langchain` / `vercel ai sdk` côté UI ?

---

## 14. Risques et anti‑patterns à surveiller dès le J1

| Risque | Symptôme | Mitigation |
|---|---|---|
| **L'orchestrateur Python devient le goulot** | CPU à 100 % sur 1 thread | Profiler `py-spy`, pousser le travail en `to_thread` agressivement, envisager Rust pour le hot path à long terme |
| **llama.cpp slot saturé** | queue depth > 2 sur un slot | Augmenter `--parallel`, ou dédoubler le serveur sur ce slot |
| **Drift de qualité après distillation** | métriques régressent | Toujours A/B sur golden set avant promote ; rollback automatique |
| **Cache empoisonné** | mauvaise réponse répétée | TTL court (5 min L0), validation par L5 même sur cache hit |
| **Stigmergie qui fossilise** | toujours les mêmes capsules remontent | Décroissance λ exponentielle + exploration aléatoire (ε=10 % de capsules nouvelles forcées) |
| **Phi-3.5 critique trop lent** | latence L5 > stream | Critique post‑stream (annotations apparaissent après) plutôt que blocking |
| **Anthropic timeout** | requêtes qui pendent | Timeout court 8 s + failover local automatique |

---

## En une phrase

> *Scout & Synthesize* = **un pipeline asynchrone à 6 couches, dont 5 tournent sur l'edge en parallèle et 1 en cloud**, où chaque couche est une fonction pure typée à entrée/sortie Pydantic, branchée sur un serveur d'inférence multi‑slot, avec une mémoire vectorielle hiérarchique servant à la fois de retrieval et de stigmergie — le tout streamé en SSE pour donner l'illusion d'instantanéité.
