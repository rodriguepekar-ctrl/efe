# Mode 1 — EFE comme **outil de classification d'un agent LLM** — Architecture

> **Objet** : architecture d'intégration pour exploiter EFE depuis un agent LLM
> (Claude, GPT-4, Llama, Gemini…) sous la forme d'un *tool* /
> *function* de classification/routage par essaim consensuel sur
> **ancres pré-enregistrées**.
> **Crates impactés** : `efe-server` (M10), `efe-core::alignment` (M9),
> `efe-core::dissolution::decoders::nearest_anchor` (M7.5).
> **Nouveaux artefacts** : `clients/python/efe_tool`, `catalogs/*.yaml`,
> `agents/{claude,openai}_bridge.py`, `eval/golden/*.jsonl`.
> **Version du document** : 1.0 (Mode 1 MVP).
> **Date** : 19 avril 2026.
> **Niveau de précision** : chirurgicale, fichier-par-fichier.

---

## Sommaire

1. [Mission de Mode 1](#1-mission-de-mode-1)
2. [Positionnement dans le paysage EFE](#2-positionnement-dans-le-paysage-efe)
3. [Principes directeurs](#3-principes-directeurs)
4. [Acteurs et topologie](#4-acteurs-et-topologie)
5. [Flux d'une requête de bout en bout](#5-flux-dune-requête-de-bout-en-bout)
6. [Modèle de déploiement](#6-modèle-de-déploiement)
7. [Sous-système 1.1 — **Ancres sémantiques**](#7-sous-système-11--ancres-sémantiques)
8. [Sous-système 1.2 — **Catalogue d'ancres**](#8-sous-système-12--catalogue-dancres)
9. [Sous-système 1.3 — **Cycle de vie des cônes**](#9-sous-système-13--cycle-de-vie-des-cônes)
10. [Sous-système 1.4 — **Bridge client Python**](#10-sous-système-14--bridge-client-python)
11. [Sous-système 1.5 — **Choix de l'embedder**](#11-sous-système-15--choix-de-lembedder)
12. [Sous-système 1.6 — **Protocole gRPC utilisé**](#12-sous-système-16--protocole-grpc-utilisé)
13. [Sous-système 1.7 — **Post-traitement du verdict**](#13-sous-système-17--post-traitement-du-verdict)
14. [Sous-système 1.8 — **Intégration agent LLM (tool)**](#14-sous-système-18--intégration-agent-llm-tool)
15. [Sous-système 1.9 — **Cache et idempotence**](#15-sous-système-19--cache-et-idempotence)
16. [Sous-système 1.10 — **Observabilité côté client**](#16-sous-système-110--observabilité-côté-client)
17. [Sous-système 1.11 — **Erreurs et fallback**](#17-sous-système-111--erreurs-et-fallback)
18. [Sous-système 1.12 — **Sécurité et multi-tenant**](#18-sous-système-112--sécurité-et-multi-tenant)
19. [Budget de performances](#19-budget-de-performances)
20. [Jeu de test, golden dataset, métriques qualité](#20-jeu-de-test-golden-dataset-métriques-qualité)
21. [Stratégie de dérive de distribution](#21-stratégie-de-dérive-de-distribution)
22. [Plan de sprints](#22-plan-de-sprints)
23. [Critères d'acceptation globaux](#23-critères-dacceptation-globaux)
24. **Annexes**
    - A. [Schéma YAML d'un catalogue d'ancres](#annexe-a--schéma-yaml-dun-catalogue-dancres)
    - B. [Bridge Python — API publique](#annexe-b--bridge-python--api-publique)
    - C. [Schéma de tool Claude & OpenAI](#annexe-c--schéma-de-tool-claude--openai)
    - D. [Structure du golden dataset](#annexe-d--structure-du-golden-dataset)
    - E. [Modèles d'embedder recommandés](#annexe-e--modèles-dembedder-recommandés)
    - F. [Matrice de risques Mode 1](#annexe-f--matrice-de-risques-mode-1)
    - G. [Glossaire](#annexe-g--glossaire)

---

## 1. Mission de Mode 1

Mode 1 est **le cas d'usage canonique d'EFE en production**. Il transforme le
moteur d'essaim vectoriel (M1..M9) en un **classifier distribué robuste par
consensus** appelable comme une fonction pure depuis n'importe quel agent
LLM, sans que celui-ci ait conscience du mécanisme sous-jacent.

L'agent LLM, face à un texte libre de l'utilisateur, **délègue la décision
catégorielle** (quelle classe ? quel routage ? quelle politique ?) à l'essaim,
puis reprend son raisonnement une fois la classe et la confiance retournées.

### 1.1 Ce que Mode 1 **est**

- Un **point d'entrée asynchrone** exposé sous forme de tool/function, conforme
  aux standards Anthropic Tools, OpenAI Function Calling, LangChain Tools,
  MCP (Model Context Protocol).
- Un **classifier zero-shot** qui n'exige pas de fine-tuning : la qualité du
  discriminant est portée par (a) l'embedder, (b) la qualité des ancres,
  (c) la robustesse du consensus multi-agents.
- Un **service auditable** : chaque décision produit une trace dans l'audit
  JSONL (qui cône ? quelle version ? quel tenant ? quel verdict ? quelle
  marge de confiance ?) — conformité RGPD, AI Act, audit interne.
- Un **composant cacheable** : pour un `(texte, catalogue_version)` donné, la
  réponse est déterministe et peut être mémoïsée.

### 1.2 Ce que Mode 1 **n'est pas**

- **Pas un générateur** : ne produit pas de texte libre. Renvoie toujours un
  couple `(label, confidence)` + optionnellement le vecteur centroïde.
- **Pas un moteur de raisonnement multi-étapes** : une requête = une décision.
  Les agents LLM qui orchestrent plusieurs étapes (plan / execute / verify)
  appellent EFE ponctuellement à l'étape qui demande une décision
  catégorielle.
- **Pas un remplaçant de classifieur supervisé** quand celui-ci est possible :
  si on a 100 000 exemples étiquetés, un transformer fine-tuné battra EFE.
  Mode 1 brille sur le **zero-shot**, le **cold-start**, les domaines où les
  étiquettes changent fréquemment, les contextes où l'audit importe.

---

## 2. Positionnement dans le paysage EFE

```
 ┌─────────────────────────────────────────────────────────────────────┐
 │  AGENT LLM (Claude, GPT-4, Llama, Gemini)                           │
 │                                                                     │
 │    [user prompt]                                                    │
 │       │                                                             │
 │       ▼                                                             │
 │   ┌──────────────────────────┐                                      │
 │   │ reasoning LLM            │                                      │
 │   │ (analyse, planification) │                                      │
 │   └──────────────────────────┘                                      │
 │       │                                                             │
 │       │ « j'ai besoin de classer ce texte parmi {A,B,C,D} »         │
 │       ▼                                                             │
 │   ┌──────────────────────────────────────┐                          │
 │   │  tool: efe_classify                  │  ◄── Mode 1              │
 │   │   input = { text }                   │                          │
 │   └──────────────────────────────────────┘                          │
 │       │                                                             │
 └───────┼─────────────────────────────────────────────────────────────┘
         │
         ▼
 ┌─────────────────────────────────────────────────────────────────────┐
 │  BRIDGE CLIENT (Python / Rust / Go / Node)                          │
 │                                                                     │
 │   1. embed(text) → vecteur ℝ^D                                      │
 │   2. fabrique SwarmSpec { dim, pop, ancres, cône }                  │
 │   3. appel gRPC SwarmService.Execute                                │
 │   4. reçoit Response { output: AnchorLabel{idx, score}, conf }      │
 │   5. décode idx → label métier via catalogue                        │
 │   6. renvoie { label, confidence, alignment_delta }                 │
 └─────────────────────────────────────────────────────────────────────┘
         │
         ▼
 ┌─────────────────────────────────────────────────────────────────────┐
 │  EFE SERVER (gRPC) — M1..M12                                        │
 │                                                                     │
 │   • M10 : gRPC + auth + rate-limit + idempotence                    │
 │   • M8  : SwarmController.drive()                                   │
 │       ┌──────────────┐  ┌─────────┐  ┌──────────────┐  ┌─────────┐  │
 │       │ M3 Genesis   │→ │ M5 Fus. │→ │ M6 Consensus │→ │ M7 Diss.│  │
 │       └──────────────┘  └─────────┘  └──────────────┘  └─────────┘  │
 │   • M9  : registre des cônes (ancres auditées, versionnées)          │
 │   • M11 : /metrics + audit JSONL                                    │
 └─────────────────────────────────────────────────────────────────────┘
```

Mode 1 est donc une **couche d'adaptation** entre le monde LLM (textes,
tools, prompts) et le monde EFE (vecteurs latents, ancres, cônes,
verdicts). Toute la complexité de la conversion est concentrée dans le
bridge client.

---

## 3. Principes directeurs

### 3.1 Principes opposables en revue PR

1. **Zéro texte dans le flux interne EFE** — le bridge convertit le texte en
   vecteur au plus tôt et ne le retransmet jamais au serveur (sauf dans le
   champ optionnel `raw_prompt` utilisé uniquement pour l'audit,
   subordonné à la blocklist PII de M11).
2. **Déterminisme à deux niveaux** — (a) un même texte + même embedder +
   même catalogue = même vecteur problème ; (b) un même vecteur problème +
   même seed d'essaim = même verdict. Toute violation doit être justifiée
   par une fiche de dérogation.
3. **Idempotence sur le couple `(text_hash, catalog_version)`** — le cache
   d'idempotence côté serveur M10 garde 5 minutes la dernière réponse ;
   le bridge peut en plus mémoïser côté client sur une fenêtre plus
   longue pour les cas récurrents.
4. **Ancres **jamais** dérivées du texte utilisateur** — les ancres sont un
   **patrimoine stable** du système, conçues, validées et versionnées par
   l'équipe produit, jamais générées à la volée à partir d'un prompt
   utilisateur (cela ouvrirait un vecteur d'attaque par injection).
5. **Fail-closed sur seuil de confiance** — si `confidence < seuil` ou si
   `alignment_delta > max`, on retourne une réponse **`uncertain`** plutôt
   qu'un label faux. L'agent LLM reprend la main avec un raisonnement
   explicite.
6. **Audit obligatoire** — chaque appel laisse une ligne JSONL lisible par
   `jq`, incluant `(tenant, catalogue_id, catalogue_version, chosen_label,
   confidence, alignment_delta, rounds, duration_ms, swarm_id, request_id,
   text_hash)`. **Jamais le texte en clair**.

### 3.2 Ce que Mode 1 refuse d'implémenter

- **Ancres dynamiques générées par le LLM à chaque requête** → c'est Mode 2,
  explicitement hors de ce document.
- **Hot-reload d'ancres sans passer par `AdminService::EvolveCone`** →
  garantirait la cohérence avec l'audit et les 7 invariants I1..I7.
- **Sélection dynamique d'embedder** → un catalogue est lié à un embedder
  unique, fixé à la création ; changer d'embedder impose un nouveau
  catalogue.

---

## 4. Acteurs et topologie

### 4.1 Acteurs

| Acteur | Responsabilité | Déploiement |
|---|---|---|
| **Utilisateur final** | Pose un prompt libre | Client chat / API externe |
| **Agent LLM** | Décide, oriente, invoque le tool `efe_classify` | OpenAI, Anthropic, local vLLM |
| **Bridge client** | Embed + gRPC + post-traitement | Lib Python, container sidecar, serverless |
| **Embedder** | Transforme texte → vecteur | Local (sentence-transformers), API (OpenAI, Cohere) |
| **EFE Server** | Essaim + verdict | Kubernetes (voir M12) |
| **Catalogue d'ancres** | Définition versionnée des classes | Fichier YAML en repo + cônes M9 enregistrés |
| **Admin ops** | Évolution des ancres, rollback | CLI `efe-admin` + AdminService |

### 4.2 Topologie logique

```
 ┌──────────────┐      HTTPS        ┌──────────────────────┐
 │  Utilisateur │ ─────────────────►│   API de l'agent LLM │
 └──────────────┘                   │   (OpenAI / Claude)  │
                                    └──────────┬───────────┘
                                               │ tool_call
                                               ▼
 ┌──────────────────────────────────────────────────────────┐
 │  Bridge client (Python lib ou sidecar)                   │
 │                                                          │
 │   ┌─────────────┐   ┌──────────────┐   ┌─────────────┐  │
 │   │ Embedder    │   │ Catalogue    │   │ Cache LRU   │  │
 │   │ (local ou   │   │ loader +     │   │ (text_hash  │  │
 │   │  API)       │   │ validator    │   │  → verdict) │  │
 │   └─────────────┘   └──────────────┘   └─────────────┘  │
 │                                                          │
 │   ┌──────────────────────────────────────────────────┐   │
 │   │ gRPC client (tonic/grpcio)                        │   │
 │   └──────────────────────────────────────────────────┘   │
 └──────────┬───────────────────────────────────────────────┘
            │ gRPC TLS (ou plaintext intra-cluster)
            ▼
 ┌──────────────────────────────────────────────────────────┐
 │  EFE Server (service mesh, 3 replicas HA)                │
 └──────────────────────────────────────────────────────────┘
```

### 4.3 Variantes de déploiement

| Variante | Bridge | Embedder | Cas d'usage |
|---|---|---|---|
| **Library** | import python direct | local | intégration monolithique Python (agent + bridge même processus) |
| **Sidecar** | container dans le même pod que l'agent | local | scalabilité horizontale, isolation CPU |
| **Service partagé** | microservice dédié | local | plusieurs agents, mutualisation GPU embed |
| **Serverless** | Lambda / Cloud Run | API externe (OpenAI) | trafic irrégulier, coût à l'appel |

La variante **Library** est recommandée pour le MVP : elle minimise la
latence (pas de hop réseau supplémentaire entre l'agent et le bridge) et
simplifie l'observabilité (logs unifiés).

---

## 5. Flux d'une requête de bout en bout

Séquence canonique d'une classification zéro-shot via Mode 1 :

```
  t=0 ms   : Utilisateur envoie un prompt libre
  t=1 ms   : API agent LLM reçoit, invoque le LLM
  t=+200ms : LLM décide d'appeler tool `efe_classify(text="...")`
  t=+201ms : Bridge reçoit l'invocation, vérifie cache
            → miss → continuer
  t=+203ms : Embedder local encode le texte → vecteur 384-d (≈ 2 ms)
  t=+203ms : Catalogue chargé (mmap en mémoire), ancres obtenues
  t=+204ms : Bridge fabrique `SwarmSpec`:
              - dim_latent = 384
              - problem_embedding = <vecteur du prompt>
              - population = [("na-sem", 256)]
              - max_rounds = 16
              - alignment_cone = {anchors: <ancres catalogue>, max_d: 0.5}
  t=+204ms : Bridge appelle gRPC SwarmService/Execute
             (+ metadata: tenant, trace_id, request_id)
  t=+205ms : M10 server : auth OK, rate-limit OK, idempotence miss
  t=+206ms : M8 Controller : genesis (spawn 256 nano-agents) → 2 ms
  t=+208ms : Fusion round 1..N (attention, softmax, aggregate) ≈ 60 ms
             N ≈ 8 rounds en moyenne pour converger
  t=+270ms : Consensus détecté → Verdict::Converged { confidence: 0.89 }
  t=+271ms : Dissolution : extract_centroid → project on cone → decode
             NearestAnchor → DecodedOutput::AnchorLabel { idx=2, score=0.93 }
  t=+273ms : M11 audit : append JSONL { swarm_id, confidence, alignment_delta, ...}
  t=+274ms : gRPC response → bridge
  t=+275ms : Bridge : idx=2 → catalogue.labels[2] = "billing_question"
  t=+275ms : Bridge met en cache (text_hash, catalogue_version) → verdict
  t=+276ms : Bridge retourne { label: "billing_question", confidence: 0.89,
                               alignment_delta: 0.12, rounds: 8 } au LLM
  t=+280ms : LLM reprend le raisonnement avec la classe connue
```

Latence totale cible : **< 300 ms p95**, dont 60-80 ms d'essaim.

---

## 6. Modèle de déploiement

### 6.1 Surfaces de configuration

Trois surfaces de configuration coexistent :

1. **Le catalogue d'ancres** (YAML versionné en repo git) — définit les
   classes, leurs descriptions, leurs ancres éventuellement pré-calculées
   ou re-générées à chaque déploiement.
2. **La configuration du bridge** (TOML ou variables d'env) — embedder,
   endpoint EFE, credentials, budgets de latence, cache.
3. **La configuration du serveur EFE** (déjà couverte par M12) — rien de
   spécifique à Mode 1 hormis le pattern `population = na-sem ≥ 128`.

### 6.2 Matrice environnement → décisions

| Dev local | Staging | Prod |
|---|---|---|
| Embedder local CPU, 80 Mio, 384 d | Local ou API petit modèle | API premium ou local GPU |
| Catalogue en fichier YAML monté | Catalogue versionné par release | Catalogue + cônes M9 auditéss |
| `inline.enabled=true` pour JWT | ExternalSecret Vault | External Secret Vault + rotation trimestrielle |
| EFE in-process ou local docker | EFE staging cluster | EFE prod cluster 3 replicas |
| Cache local disque 1 Gio | Redis partagé | Redis partagé + RocksDB persistent |

### 6.3 Séquence de bootstrap du bridge

1. Lecture du fichier de config (`EFE_BRIDGE_CONFIG=/etc/efe-bridge/config.toml`).
2. Vérification que l'embedder déclaré est accessible (modèle téléchargé
   ou API authentifiée).
3. Lecture et validation du catalogue (schéma, dimension des ancres
   matchent celle de l'embedder, checksum des textes anchor).
4. Éventuellement : appel à `AdminService::GetCone` pour vérifier que le
   cône côté serveur est en phase avec le catalogue local (si la politique
   `strict_cone_sync=true`).
5. Ouverture du canal gRPC + ping santé.
6. Prêt à servir.

---

## 7. Sous-système 1.1 — **Ancres sémantiques**

### 7.1 Définition précise

Une **ancre sémantique** est un vecteur ℝ^D qui représente le **centre
sémantique d'une classe**. Formellement, c'est un point dans l'espace
latent de l'embedder qui est tel que tous les textes appartenant à la
classe sont *en moyenne proches* de ce point selon une distance cosinus.

```
anchor_class = E[embed(text) | text ∈ class]
```

Trois stratégies de construction (par ordre d'efficacité) :

#### 7.1.1 Stratégie A — Ancre par phrase canonique (MVP)

On choisit une ou quelques phrases qui **décrivent** la classe de façon
canonique, et on les encode. C'est la forme la plus simple.

```yaml
classes:
  - name: billing_question
    canonical_sentences:
      - "Where can I find my invoice?"
      - "I want to update my payment method."
      - "Why was I charged twice?"
```

Le catalogue stocke les phrases (auditables, diff-ables) et le bridge
calcule l'ancre comme `mean(embed(s_1), ..., embed(s_n)) / ||mean||`.

**Avantages** : simple, auditable, les humains comprennent ce qu'ils voient.
**Inconvénients** : dépend fortement de la qualité des phrases choisies ;
peu robuste aux formulations très différentes de l'anglais standard.

#### 7.1.2 Stratégie B — Ancre par centroïde d'exemples (recommandée quand disponible)

On recueille un petit corpus d'exemples étiquetés (10-100 par classe),
on les encode, et l'ancre est le centroïde.

```yaml
classes:
  - name: billing_question
    examples_file: examples/billing_question.txt
    # contient 50 formulations réelles ou synthétiques
```

**Avantages** : plus robuste à la diversité linguistique.
**Inconvénients** : demande un corpus minimal (peut être synthétique,
généré par un LLM sous supervision humaine).

#### 7.1.3 Stratégie C — Ancre apprise (post-MVP)

On apprend l'ancre par *contrastive learning* (InfoNCE) sur un corpus
étiqueté : ancre positive vs exemples négatifs. C'est plus puissant mais
exige un pipeline d'entraînement léger.

**Avantages** : discriminant maximal.
**Inconvénients** : pipeline ML en plus, moins interprétable.

### 7.2 Propriétés exigées d'une ancre de qualité

- **Unitaire** : `||anchor||_2 = 1 ± 1e-6` (l'essaim opère mieux en normalisé).
- **Cohérente avec l'embedder** : même dimension, même espace
  (conséquence : changer d'embedder = changer toutes les ancres).
- **Distincte des autres ancres** : la similarité cosinus inter-classes
  doit être **< 0.7** (heuristique). Au-delà, les classes se recouvrent
  et l'essaim aura du mal à discriminer.
- **Stable sur un jeu de validation** : sur un golden set interne, l'ancre
  doit donner `cosine(embed(ex_positif), anchor) > cosine(embed(ex_positif),
  anchor_d'une_autre_classe) + marge` pour la majorité des exemples.

### 7.3 Invariants statistiques du catalogue

Un catalogue sain satisfait :

| Invariant | Seuil MVP |
|---|---|
| **Nombre de classes** | 2 ≤ N ≤ 32 (au-delà, utiliser un arbre hiérarchique) |
| **Dimension** | 384 ≤ D ≤ 1536 |
| **Similarité inter-classe max** | `max(cos(a_i, a_j)) for i≠j` ≤ 0.75 |
| **Coverage recall** sur golden | ≥ 0.85 |
| **Confidence mean** sur golden | ≥ 0.80 |
| **Taux de rejet** (conf < 0.6) | ≤ 0.15 |

Ces invariants sont vérifiés par un script `eval_catalog.py` exécuté en CI
à chaque modification du catalogue.

---

## 8. Sous-système 1.2 — **Catalogue d'ancres**

### 8.1 Structure fichier

Un catalogue est un fichier YAML versionné dans le repo, à `catalogs/<nom>.yaml`.

```yaml
# catalogs/support_triage.yaml
apiVersion: efe.mode1/v1
kind: AnchorCatalog
metadata:
  name: support_triage
  version: "1.3.0"
  created_at: 2026-04-15T10:30:00Z
  author: product-team@example.com
  description: >
    Classification des tickets de support client en 4 catégories métier.

embedder:
  family: sentence-transformers
  model: all-MiniLM-L6-v2
  dim: 384
  normalize: l2        # l'embedder sort des vecteurs normalisés

cone_policy:
  max_projection_distance: 0.5
  registered_in_m9: true
  m9_key:
    tenant: support
    task: triage
    version: 3          # aligné avec metadata.version.major

quality_gates:
  min_inter_class_distance: 0.25   # équivalent à cos ≤ 0.75
  min_golden_recall: 0.85
  max_reject_rate: 0.15

classes:
  - name: fraud
    description: Suspicion de fraude (paiement, compte, abus)
    color: red
    canonical_sentences:
      - "Someone charged my card without my consent."
      - "I think my account has been hacked."
      - "There's a transaction I don't recognize."
    examples_file: catalogs/examples/fraud.txt

  - name: bug
    description: Problème technique (bug, panne, erreur système)
    color: orange
    canonical_sentences:
      - "The app keeps crashing when I try to log in."
      - "I'm getting a 500 error on checkout."
      - "The page is blank after the update."
    examples_file: catalogs/examples/bug.txt

  - name: question
    description: Question fonctionnelle (comment faire X, où trouver Y)
    color: blue
    canonical_sentences:
      - "How do I export my data?"
      - "Where can I change my notification preferences?"
      - "Is there a way to schedule a post?"
    examples_file: catalogs/examples/question.txt

  - name: billing
    description: Question de facturation (facture, paiement, abonnement)
    color: green
    canonical_sentences:
      - "When will I receive my invoice?"
      - "I want to cancel my subscription."
      - "Can I change my payment method?"
    examples_file: catalogs/examples/billing.txt

thresholds:
  confidence_high: 0.85       # ≥ → trusted label
  confidence_medium: 0.65     # [0.65, 0.85[ → label avec warning
  confidence_low: 0.65        # < → "uncertain"
```

### 8.2 Pipeline de compilation

```
     catalogs/support_triage.yaml
              │
              ▼
      ┌───────────────┐
      │ validator     │  ← schema + cohérence + golden gates
      └───────┬───────┘
              │
              ▼
      ┌───────────────┐
      │ embedder      │  ← encode canonical_sentences + examples
      └───────┬───────┘
              │
              ▼
      ┌───────────────┐
      │ compiler      │  ← mean + L2 norm + inter-class sanity
      └───────┬───────┘
              │
              ▼
     catalog.compiled.json  (ancres f32 + metadata)
              │
              ├────► mmap dans le bridge (local)
              │
              └────► RegisterCone vers M9 (audit, evolution, rollback)
```

Un catalogue **compilé** contient :

```json
{
  "schema_version": "1",
  "metadata": {...},
  "embedder_fingerprint": "sha256:abcd...",
  "classes": [
    {
      "name": "fraud",
      "index": 0,
      "anchor": [0.123, -0.456, ..., 0.789],   // 384 f32
      "anchor_hash": "sha256:efgh..."
    },
    ...
  ],
  "quality_report": {
    "inter_class_matrix": [[1.0, 0.21, ...], ...],
    "golden_recall": 0.92,
    "golden_confusion": {...}
  },
  "compile_epoch": 1760000000
}
```

### 8.3 Versioning et rétro-compatibilité

Le champ `metadata.version` suit SemVer 2 :

- **Patch** (1.3.0 → 1.3.1) : correction d'une phrase canonique, sans
  changement d'ancre (changement cosmétique documentaire).
- **Mineur** (1.3.x → 1.4.0) : ajout d'une classe ou enrichissement d'un
  exemple file. Les anciennes classes gardent leurs indices (append-only).
- **Majeur** (1.x → 2.0) : suppression ou re-indexation de classes,
  changement d'embedder. Breaking : les clients doivent être mis à jour.

Les clients connaissent le `catalog.version` de leur build ; ils refusent
un catalogue `major` différent.

---

## 9. Sous-système 1.3 — **Cycle de vie des cônes**

### 9.1 Pourquoi enregistrer dans M9 ?

Le catalogue local (YAML + JSON compilé) suffit au bridge pour appeler
`Execute`. **Mais** enregistrer le cône d'ancres côté serveur via
`AdminService::RegisterCone` apporte trois garanties :

1. **Audit auditable de la version déployée** : chaque requête peut porter
   la référence `alignment_cone_ref = {tenant, task, version}` plutôt
   que la cône inline, et le serveur reconstitue le cône côté lui. On
   voit alors dans `audit.jsonl` quelle version a servi.
2. **Évolution contrôlée** : on peut faire `EvolveCone` pour ajouter un
   anchor sans redéployer les clients (typique ajout d'une 5e classe).
3. **Rétention historique** : `ListVersions` montre l'historique de
   l'évolution, `BreachHistory` montre quand les requêtes sortent du cône.

### 9.2 Séquence d'enregistrement

```
   ┌──────────────────┐
   │ bridge/CLI:      │
   │ efe-admin        │
   │  register-cone   │
   └─────────┬────────┘
             │
             │ 1. lit catalog.compiled.json
             │ 2. construit AlignmentCone { anchors[], max_projection_distance }
             │ 3. AdminService::RegisterCone({
             │      tenant: "support",
             │      task: "triage",
             │      version: 3,
             │      cone: {...},
             │      created_by: "deploy@ci",
             │      description: "catalog 1.3.0 compiled on ...",
             │      labels: { git_sha: "abc...", catalog_version: "1.3.0" }
             │    })
             ▼
   ┌──────────────────┐
   │ EFE Server M9    │  ← vérifie les 7 invariants I1..I7
   │ InMemoryRegistry │  ← émet RegistryEvent::Registered
   └─────────┬────────┘
             │
             ▼
   ┌──────────────────┐
   │ M11 RegistrySink │  → compteur efe_cone_registrations_total{event="registered"}
   │                  │  → entrée JSONL audit
   └──────────────────┘
```

### 9.3 Règles d'évolution

Quand une classe évolue (phrase canonique modifiée, exemple ajouté) sans
changer son indice :

- **Cas 1** : cosinus entre nouvelle ancre et ancienne > 0.95
  → évolution mineure, on peut utiliser `EvolveCone` avec `op = replace_anchor{idx, new}`
  → invariant I3 (stabilité d'axe) respecté.
- **Cas 2** : cosinus < 0.95 → évolution **majeure** : bump major version
  du catalogue, appel à `RegisterCone` avec `version=N+1` comme nouvel enregistrement.

Quand une classe est ajoutée :
- `EvolveCone` avec `op = append_anchor{new}` → nouvelle version du cône.

Quand une classe est supprimée :
- Jamais supprimée dans un cône stable : elle est **dépréciée** (marquée
  `deprecated: true` dans le YAML, gardée pour la rétrocompatibilité
  jusqu'à la prochaine major).

### 9.4 Rollback

```bash
# Détection en prod : taux d'erreur augmente après déploiement v1.4.0
#  → rollback immédiat vers v1.3.x via Helm
helm rollback efe <revision>
# + côté catalogue : republier v1.3.0 et ré-enregistrer son cône
efe-admin register-cone --file catalogs/support_triage@1.3.0.compiled.json
# les nouveaux clients utilisent à nouveau v=3 du cône M9
```

---

## 10. Sous-système 1.4 — **Bridge client Python**

### 10.1 Architecture interne

```
efe_tool/
├── __init__.py           # exports publics: classify_with_swarm, AnchorCatalog
├── client.py             # EfeClient : pool gRPC + config
├── embedder.py           # EmbedderAdapter (local / API)
├── catalog.py            # AnchorCatalog : load, validate, compile
├── cache.py              # LRUCache + text_hash normalization
├── errors.py             # BridgeError, ConfigError, NetworkError, LowConfidenceWarn
├── metrics.py            # métriques Prometheus client-side
├── spec_builder.py       # construit SwarmSpec depuis un texte
├── bridges/
│   ├── claude.py         # tool def + bridge pour Anthropic SDK
│   ├── openai.py         # function def + bridge pour OpenAI SDK
│   ├── mcp.py            # exposition MCP-compatible
│   └── langchain.py      # wrapper Tool
├── cli.py                # efe-admin (register-cone, eval-catalog, etc.)
└── py.typed              # PEP 561 marker
```

### 10.2 API publique canonique

```python
from efe_tool import EfeClient, AnchorCatalog, ClassifyResult

catalog = AnchorCatalog.load("catalogs/support_triage.yaml")
client = EfeClient.from_env()  # lit EFE_ENDPOINT, EFE_JWT, etc.

result: ClassifyResult = client.classify(
    text="There's a charge I don't recognize",
    catalog=catalog,
)
# result.label         = "fraud"
# result.confidence    = 0.92
# result.alignment_delta = 0.11
# result.rounds        = 7
# result.reject        = False         # (conf > threshold_high)
# result.swarm_id      = "...uuid..."
```

### 10.3 Découpage en responsabilités

| Module | Responsabilité | Pur / Effect |
|---|---|---|
| `embedder.py` | texte → vecteur | I/O (CPU/GPU/API) |
| `catalog.py` | YAML → AnchorCatalog validé | Pur (sauf file IO) |
| `spec_builder.py` | texte_vecteur + catalog → SwarmSpec proto | Pur |
| `client.py` | SwarmSpec → gRPC → ExecuteResponse | I/O (réseau) |
| `cache.py` | (text_hash, catalog_ver) → verdict | État local |
| `metrics.py` | instrumente tous les appels | Effet side |
| `errors.py` | Normalisation d'erreurs | Pur |

### 10.4 Séquence interne d'un `classify`

```python
def classify(self, text: str, catalog: AnchorCatalog) -> ClassifyResult:
    # 1. Hash normalisé du texte
    text_hash = self._hash(text)

    # 2. Check cache
    cached = self.cache.get(text_hash, catalog.version)
    if cached is not None:
        self.metrics.cache_hit.inc()
        return cached

    # 3. Embed
    with self.metrics.embed_seconds.time():
        vec = self.embedder.encode(text)     # shape (D,)

    # 4. Build SwarmSpec proto
    spec = self.spec_builder.build(
        problem_embedding=vec,
        anchors=catalog.anchors,              # shape (N, D)
        max_projection_distance=catalog.max_proj_distance,
        population=[("na-sem", self.config.population)],
        max_rounds=self.config.max_rounds,
    )

    # 5. gRPC call
    req = ExecuteRequest(
        request_id=str(uuid.uuid4()),
        raw_prompt="",                        # PII-safe: jamais le texte
        spec=spec,
        decode_mode=DecodeMode(
            pass_through=PassThroughMode(),   # NearestAnchor côté serveur
        ),
    )
    with self.metrics.rpc_seconds.time():
        resp = self.client.Execute(req, metadata=self._auth_metadata())

    # 6. Décodage du résultat
    if resp.output.WhichOneof("kind") != "anchor_label":
        raise BridgeError(f"unexpected output kind: {resp.output}")
    idx = resp.output.anchor_label.index
    score = resp.output.anchor_label.score
    label = catalog.classes[idx].name

    # 7. Seuil de rejet
    reject = resp.confidence < catalog.thresholds.confidence_low

    result = ClassifyResult(
        label=label,
        confidence=resp.confidence,
        alignment_delta=resp.alignment_delta,
        rounds=resp.rounds_executed,
        reject=reject,
        swarm_id=resp.swarm_id,
        request_id=req.request_id,
        text_hash=text_hash,
        catalog_version=catalog.version,
    )

    # 8. Cache et return
    self.cache.put(text_hash, catalog.version, result)
    self.metrics.classifications_total.labels(
        label=label, reject=str(reject)
    ).inc()
    return result
```

### 10.5 Normalisation du hash texte

Le `text_hash` sert de clef de cache et d'audit. Normalisation :

- Strip leading/trailing whitespace.
- Collapse tout whitespace interne en espace unique.
- `NFC` Unicode normalization.
- Lowercase (optionnel, activable par config ; par défaut **non**,
  car la casse porte du signal sémantique dans certaines langues).

`text_hash = SHA-256(normalized)` — 64 hex chars.

### 10.6 Gestion de la concurrence

- `EfeClient` est **thread-safe** et partageable (pool gRPC interne).
- `AnchorCatalog` est **immuable** après `load` (via `@dataclass(frozen=True)`).
- Le cache utilise un `threading.Lock` fin (LRU maison ou `cachetools.LRUCache`).
- En async/await (FastAPI, asyncio), un wrapper `AsyncEfeClient` expose
  les mêmes méthodes avec `grpc.aio`.

---

## 11. Sous-système 1.5 — **Choix de l'embedder**

### 11.1 Contraintes

- **Dimension** : cohérente avec `dim_latent` du catalogue et du
  `SwarmSpec`. 384, 512, 768, 1024, 1536 sont les valeurs usuelles.
- **Normalisation** : l'embedder doit produire des vecteurs L2-normalisés,
  sinon le bridge doit les normaliser post-hoc.
- **Latence** : < 10 ms en p95 pour rester dans le budget total de < 300 ms.
- **Déterminisme** : même texte = même vecteur. Les embedders API peuvent
  introduire de légères variations (float truncation) ; on tolère 1e-4.
- **Langue** : si le produit est multilingue, choisir un embedder
  multilingue (ex. `paraphrase-multilingual-MiniLM-L12-v2`).

### 11.2 Candidats MVP

Voir [Annexe E](#annexe-e--modèles-dembedder-recommandés) pour la matrice
détaillée. Résumé :

- **Local CPU (MVP)** : `sentence-transformers/all-MiniLM-L6-v2`
  — 384 d, 80 Mio, < 5 ms/texte sur CPU, MIT-like. Excellent rapport
  qualité/prix.
- **Local GPU (v1.1)** : `BAAI/bge-large-en-v1.5`
  — 1024 d, meilleur sur benchmarks MTEB, exige GPU.
- **API managée (si politique le permet)** : OpenAI `text-embedding-3-small`
  — 1536 d, ~0,02 ¢ / 1k tokens, excellent multilingue.

### 11.3 Binding embedder → catalogue

Un catalogue **est lié** à son embedder par :
- `embedder.family` + `embedder.model` (humain)
- `embedder_fingerprint` (hash SHA-256 des poids, dans le compilé)

Le bridge vérifie l'empreinte au chargement : incompatibilité = refus.
Cela empêche d'utiliser des ancres calculées avec un embedder X contre
un prompt encodé avec un embedder Y (bug catastrophique silencieux sinon).

---

## 12. Sous-système 1.6 — **Protocole gRPC utilisé**

### 12.1 RPC(s) consommés

| RPC | Quand | Fréquence |
|---|---|---|
| `SwarmService.Execute` | chaque classification | chaque requête |
| `SwarmService.DescribeSpec` | au bootstrap (health, validation) | 1× au démarrage |
| `AdminService.RegisterCone` | déploiement d'un catalogue | rarement |
| `AdminService.EvolveCone` | ajout mineur | rarement |
| `AdminService.GetCone` | vérification de synchronisation | périodique |
| `AdminService.BreachHistory` | audit opérateur | manuel |
| `grpc.health.v1.Health/Check` | healthcheck du sidecar | périodique |

### 12.2 Métadonnées gRPC obligatoires

Chaque appel porte :

| Métadonnée | Valeur |
|---|---|
| `authorization` | `Bearer <JWT>` (si `EFE_AUTH_ENABLED=true`) |
| `x-request-id` | UUIDv4 unique par classification |
| `x-tenant-id` | identifiant tenant (= scope JWT) |
| `traceparent` | format W3C trace context |
| `user-agent` | `efe-tool-python/<version>` |

### 12.3 Stratégie d'idempotence

- Le bridge génère un `request_id` UUIDv4 **déterministe** à partir de
  `(text_hash, catalog_version, tenant)` si on veut l'idempotence longue
  (rejoue exactement le même résultat pendant 5 min même si déployé sur
  un autre pod EFE).
- Ou un UUIDv4 aléatoire si on veut forcer un nouvel essaim à chaque appel.
- Recommandation MVP : déterministe sur `(text_hash, catalog_version)` :
  le cache serveur agit en plus du cache client.

### 12.4 Choix de `DecodeMode`

- `pass_through` : le serveur renvoie le résultat brut du décodeur
  (dans notre cas `AnchorLabel{idx, score}`). **C'est le choix pour Mode 1.**
- `text` : génération de texte — inutilisé en Mode 1.

---

## 13. Sous-système 1.7 — **Post-traitement du verdict**

### 13.1 Du verdict EFE au verdict métier

EFE renvoie :
```
Response {
  output: { anchor_label: { index: 2, score: 0.93 } }
  confidence: 0.89
  alignment_delta: 0.11
  verdict: Converged { confidence: 0.89 }
  rounds_executed: 7
  total_ms: 73.4
}
```

Le bridge enrichit avec le catalogue et applique la politique de seuil :

```python
@dataclass(frozen=True)
class ClassifyResult:
    label: str                  # "fraud" via catalog.classes[idx].name
    confidence: float           # resp.confidence (0..1)
    alignment_delta: float      # resp.alignment_delta
    rounds: int
    reject: bool                # True si confiance trop basse
    sub_scores: dict[str, float]  # opt: cos vs chaque ancre
    swarm_id: str
    request_id: str
    text_hash: str
    catalog_version: str

    @property
    def trustworthy(self) -> bool:
        return (
            not self.reject
            and self.alignment_delta <= 0.5
            and self.confidence >= 0.65
        )
```

### 13.2 Politique de seuil à trois niveaux

| Zone | Confiance | Action par défaut |
|---|---|---|
| **Trusted** | ≥ `confidence_high` (0.85) | Utiliser le label tel quel |
| **Warn** | [`confidence_medium`, `confidence_high`[ | Label + `warning: low_confidence` ; l'agent LLM peut poser une question de clarification |
| **Reject** | < `confidence_medium` (0.65) | Renvoyer `label=uncertain` ; l'agent LLM raisonne par d'autres moyens |

### 13.3 Re-scoring facultatif contre le golden set

Pour les cas ambigus (zone Warn), le bridge peut (optionnellement) consulter
un **index de plus proche voisin** sur un échantillon d'exemples étiquetés
du golden set : si le vecteur problème est à `cos > 0.9` d'un exemple
connu, utiliser **le label de cet exemple** au lieu du verdict EFE.

Ce re-scoring est un filet de sécurité, pas un remplacement.

---

## 14. Sous-système 1.8 — **Intégration agent LLM (tool)**

### 14.1 Schéma de tool canonique

Tous les fournisseurs majeurs (Anthropic Claude, OpenAI, Gemini) acceptent
un schéma JSON-schema-like :

```json
{
  "name": "efe_classify",
  "description": "Classify a user utterance into one of the pre-defined categories using a fluid ephemeral swarm consensus. Use this when you need to route the user's intent into a known taxonomy. Returns (label, confidence). If confidence < 0.65, treat as uncertain and ask a clarifying question instead of acting on the label.",
  "input_schema": {
    "type": "object",
    "properties": {
      "text": {
        "type": "string",
        "description": "The raw user utterance to classify. Do not pre-process or translate."
      },
      "catalog": {
        "type": "string",
        "enum": ["support_triage", "intent_detection", "content_moderation"],
        "description": "Which pre-registered anchor catalog to use."
      }
    },
    "required": ["text", "catalog"]
  }
}
```

### 14.2 Règles d'usage par l'agent LLM (prompting)

Dans le *system prompt* de l'agent, il faut éduquer le LLM sur **quand** et
**comment** utiliser le tool :

```
You have access to a tool called `efe_classify`. It routes a user utterance
into one of the categories defined in the chosen catalog.

USE IT WHEN:
- The user message seems to fit a known category (fraud, bug, question, billing)
- You need a robust, auditable classification before taking action

DO NOT USE IT WHEN:
- The user asks an open-ended question with no categorical answer
- You have already classified this exact text in this conversation
- The classification task is not among the available catalogs

AFTER RECEIVING THE RESULT:
- If confidence ≥ 0.85 : act on the label confidently.
- If 0.65 ≤ confidence < 0.85 : act on the label but mention it to the user
  and ask if they want to provide more context.
- If confidence < 0.65 : do not act on the label ; instead, ask the user
  a clarifying question in your own words.

Never invent new categories. Never attempt to bypass the swarm by guessing.
```

### 14.3 Exemple de conversation

```
User : « hey mon compte a été débité de 200 € hier soir et j'ai jamais acheté ça »

LLM (raisonnement interne) : « Cela ressemble à de la fraude. Je classifie. »

LLM ──► efe_classify(text="hey mon compte a été débité...", catalog="support_triage")

Bridge ──► embed ──► SwarmSpec ──► EFE ──► AnchorLabel{idx=0, score=0.94}

Bridge ──► LLM : { label: "fraud", confidence: 0.91, alignment_delta: 0.08 }

LLM ──► User : « Je comprends votre inquiétude. Cela ressemble à une
                transaction non autorisée. Je vais ouvrir un ticket de
                fraude et transférer votre dossier à l'équipe dédiée. »
        [en arrière-plan : appel à create_fraud_ticket(user_id, amount=200, ...)]
```

### 14.4 Multi-catalogues

Un agent peut consommer plusieurs catalogues via le paramètre `catalog`.
Le bridge charge en mémoire tous les catalogues déclarés dans la config
au démarrage, chacun indexé par son nom. Chaque catalogue peut pointer
vers son propre cône M9 (`support_triage@v3`, `intent_detection@v7`).

---

## 15. Sous-système 1.9 — **Cache et idempotence**

### 15.1 Deux niveaux de cache

```
  [LLM agent]
      │
      ▼
  ┌────────────────────────┐
  │ Bridge local cache     │  ← niveau 1 : client, LRU mémoire ou Redis
  │ (text_hash, cat_ver)   │    TTL 1 heure, taille 10k entrées
  └─────────┬──────────────┘
            │ miss
            ▼
  ┌────────────────────────┐
  │ Serveur M10 idempotence│  ← niveau 2 : serveur, FIFO LRU dédupliqué
  │ cache                  │    TTL 5 min, 10k entrées par tenant
  └─────────┬──────────────┘
            │ miss
            ▼
       [essaim EFE]
```

### 15.2 Clef de cache

```python
key = sha256(f"{text_hash}|{catalog.version}|{catalog.id}").hexdigest()
```

Inclure `catalog.id` empêche qu'un même texte cache le résultat de deux
catalogues différents (bug subtil).

### 15.3 Invalidation

- **Expiration TTL** : 1 heure par défaut.
- **Invalidation explicite** : appel `EvolveCone` suivi de bump catalog
  version → les clients rafraîchissent leur cache sur changement de version.
- **Purge manuelle** : CLI `efe-admin cache-purge --tenant X --catalog Y`.

### 15.4 Quand **désactiver** le cache

- Tests unitaires (on veut toujours voir l'essaim bouger).
- Golden set evaluation (on veut le verdict réel, pas du cache).
- Période de dérive de distribution (les mêmes textes peuvent tendre vers
  des classes différentes, le cache devient toxique).

---

## 16. Sous-système 1.10 — **Observabilité côté client**

### 16.1 Métriques Prometheus côté bridge

Le bridge expose ses propres métriques (port `BRIDGE_METRICS_PORT`, défaut
9100) disjointes de celles du serveur EFE :

| Métrique | Type | Labels |
|---|---|---|
| `efe_bridge_classifications_total` | Counter | `label`, `catalog`, `reject` |
| `efe_bridge_classify_duration_seconds` | Histogram | `catalog`, `stage` ∈ {embed, rpc, decode} |
| `efe_bridge_cache_hits_total` | Counter | `catalog` |
| `efe_bridge_cache_misses_total` | Counter | `catalog` |
| `efe_bridge_rpc_errors_total` | Counter | `code`, `catalog` |
| `efe_bridge_low_confidence_total` | Counter | `catalog`, `zone` ∈ {warn, reject} |
| `efe_bridge_alignment_breach_total` | Counter | `catalog` |

### 16.2 Tracing

Le bridge ajoute des spans OpenTelemetry :

```
trace span: classify [catalog="support_triage", tenant="acme"]
  ├── embed               [embedder="all-MiniLM-L6-v2"]
  ├── cache_lookup        [hit=false]
  ├── grpc_execute        [swarm_id="..."]
  │     └── (downstream spans côté serveur EFE)
  └── decode              [label="fraud", confidence=0.91]
```

Le `traceparent` W3C est propagé au serveur.

### 16.3 Logs structurés

Au format JSON, un ligne par classification :

```json
{
  "ts": "2026-04-19T12:34:56.789Z",
  "level": "info",
  "event": "classify",
  "tenant": "acme",
  "catalog": "support_triage",
  "catalog_version": "1.3.0",
  "label": "fraud",
  "confidence": 0.91,
  "alignment_delta": 0.11,
  "rounds": 7,
  "reject": false,
  "duration_ms": 73.4,
  "swarm_id": "01930...",
  "request_id": "9f8e...",
  "text_hash": "a1b2c3d4e5f6..."
}
```

**Jamais** le texte en clair. Le `text_hash` permet de corréler des
requêtes identiques sans exposer le contenu.

---

## 17. Sous-système 1.11 — **Erreurs et fallback**

### 17.1 Taxonomie d'erreur

| Erreur | Origine | Action |
|---|---|---|
| `EmbedderNotAvailable` | Modèle manquant, API down | Fallback sur embedder B ou retour `uncertain` |
| `CatalogInvalid` | Validation au chargement | Crash au boot (fail-fast) |
| `CatalogVersionMismatch` | Client v1.3 vs cône M9 v4 | Warning + continuer si major match |
| `RpcUnavailable` | gRPC code 14 | Retry 2× avec backoff puis `uncertain` |
| `RpcDeadlineExceeded` | gRPC code 4 | Pas de retry, retour `uncertain` |
| `RpcInvalidArgument` | Spec malformé | Bug bridge → crash log + `uncertain` |
| `RpcFailedPrecondition` | Auth/scope/alignment breach | Log + `uncertain` |
| `LowConfidence` | conf < threshold | Pas une erreur ; zone Warn / Reject |
| `AlignmentBreach` | `alignment_delta > max` | Retour `uncertain` (le vecteur problème est hors du cône) |

### 17.2 Mode `uncertain`

Quand le bridge retourne `label="uncertain"`, l'agent LLM voit :

```python
ClassifyResult(
    label="uncertain",
    confidence=0.42,
    alignment_delta=0.65,
    rounds=16,
    reject=True,
    ...
)
```

**Contract avec le LLM** : face à `uncertain`, ne **pas** inventer une
classification. Poser une question de clarification à l'utilisateur ou
passer à une autre logique (escalade humain).

### 17.3 Circuit breaker

Si le taux d'erreur RPC dépasse 10 % sur 1 minute, le bridge ouvre un
**circuit breaker** et retourne directement `uncertain` pendant 30
secondes, sans solliciter EFE. Réduit la cascade d'erreurs en cas de
panne du serveur.

### 17.4 Dégradation gracieuse

Ordre de fallback quand EFE est indisponible (par priorité) :

1. Cache local encore chaud (dernière heure).
2. Embedder + **index local k-NN** sur un petit échantillon d'exemples
   étiquetés (fallback statique, perte de qualité de ~10 %).
3. `uncertain` forcé.

---

## 18. Sous-système 1.12 — **Sécurité et multi-tenant**

### 18.1 Modèle d'autorité

- Le **JWT** du bridge définit son `tenant` (scope JWT `tenant: acme`).
- Les **cônes M9** sont partitionnés par tenant : `tenant="acme", task="support-triage", version=N`.
- Un tenant ne peut **pas** voir ni utiliser les cônes d'un autre tenant.

### 18.2 PII et texte utilisateur

- Le bridge ne transmet **jamais** le texte utilisateur au serveur dans un
  champ loggé. `raw_prompt` reste vide ou contient uniquement le hash.
- Les embeddings eux-mêmes sont considérés **pseudonymisants** mais pas
  anonymes : ils peuvent être re-identifiants contre un corpus connu.
  En conséquence, l'audit JSONL ne contient pas les vecteurs (seulement
  les hashes + métadonnées).
- Les vraies chaînes originales restent dans les logs **de l'agent LLM**
  (soumis à la politique PII de l'éditeur du LLM), pas dans EFE.

### 18.3 Rate-limit et quotas

Le serveur EFE (M10) applique un rate-limit par tenant (`RateLimitConfig`).
Le bridge ne fait pas de rate-limit propre mais expose la métrique
`efe_bridge_rpc_errors_total{code="resource_exhausted"}` pour la
détection.

---

## 19. Budget de performances

| Étape | Budget p50 | Budget p95 | Budget p99 |
|---|---|---|---|
| `embed` (local MiniLM, 384d) | 3 ms | 8 ms | 15 ms |
| `cache_lookup` | 0,05 ms | 0,1 ms | 1 ms |
| `grpc_execute` (pop=256, N≈8 rounds) | 80 ms | 180 ms | 350 ms |
| `decode` (lookup idx → label) | 0,05 ms | 0,1 ms | 1 ms |
| **Total bridge** | **85 ms** | **200 ms** | **400 ms** |

Avec un embedder API (OpenAI), ajouter 100-200 ms p95 pour le call HTTP.

**Throughput cible** : 50 classifications / seconde / pod bridge sur
1 CPU, sans GPU. Au-delà, scaler horizontalement.

---

## 20. Jeu de test, golden dataset, métriques qualité

### 20.1 Structure du golden set

Un fichier `eval/golden/<catalog>.jsonl` :

```jsonl
{"id": "g001", "text": "Mon compte a été débité sans mon accord", "expected": "fraud"}
{"id": "g002", "text": "L'app plante quand je clique sur Valider", "expected": "bug"}
{"id": "g003", "text": "Comment télécharger ma facture de mars ?", "expected": "billing"}
...
```

Règles : au moins 50 exemples par classe, diversité linguistique (accents,
argot, erreurs de frappe), cas adversariaux (textes volontairement
ambigus).

### 20.2 Métriques qualité

| Métrique | Définition | Cible MVP |
|---|---|---|
| **Accuracy@top1** | % d'exemples dont le label prédit = attendu | ≥ 85 % |
| **Precision by class** | TP / (TP + FP) | ≥ 80 % par classe |
| **Recall by class** | TP / (TP + FN) | ≥ 80 % par classe |
| **F1 macro** | moyenne des F1 par classe | ≥ 82 % |
| **Useful reject rate** | % où `reject=True` ET le golden label est réellement hors catalogue | ≥ 70 % |
| **False reject rate** | % où `reject=True` ET le golden label est dans le catalogue | ≤ 15 % |
| **Mean confidence on correct** | moyenne de conf sur exemples correctement classifiés | ≥ 0,82 |
| **Mean confidence on incorrect** | idem sur faux | ≤ 0,65 |

La calibration (correct → haute conf, faux → basse conf) est essentielle
pour que la politique de seuil soit opérable.

### 20.3 Script d'évaluation

```bash
python -m efe_tool.eval \
    --catalog catalogs/support_triage.yaml \
    --golden eval/golden/support_triage.jsonl \
    --output eval/reports/support_triage_$(date +%s).json \
    --export-confusion-matrix
```

Produit un JSON + une matrice de confusion PNG + un rapport markdown
commitable au repo.

### 20.4 Gate CI

Toute PR qui touche `catalogs/` déclenche une évaluation sur le golden.
La merge est bloquée si :
- Accuracy < 85 %
- F1 macro < 82 %
- Une classe descend sous 80 % precision OU recall
- `useful_reject_rate` < 70 %

---

## 21. Stratégie de dérive de distribution

### 21.1 Détection

En production, on surveille trois signaux :

1. **Évolution de la distribution des labels** : si un label passe de 20 %
   à 40 % de la traffic sur 7 jours, soit un changement d'usage soit
   un bug.
2. **Baisse de la mean confidence par label** : signal de dérive de
   vocabulaire/style.
3. **Augmentation du `alignment_breach_total`** : les requêtes sortent
   du cône, l'espace sémantique a bougé.

### 21.2 Alerting

Règle Prometheus ajoutée à `PrometheusRule` (cf. M12.11) :

```yaml
- alert: EFEBridgeDistributionDrift
  expr: |
    abs(
      rate(efe_bridge_classifications_total{label="fraud"}[1d])
        -
      rate(efe_bridge_classifications_total{label="fraud"}[7d] offset 1d)
    ) / clamp_min(rate(efe_bridge_classifications_total{label="fraud"}[7d] offset 1d), 1) > 0.5
  for: 1h
  labels: { severity: warning }
```

### 21.3 Réponse opératoire

1. Collecte d'échantillons des requêtes en zone Warn/Reject (avec
   anonymisation manuelle si nécessaire).
2. Ré-étiquetage humain.
3. Mise à jour du catalogue (nouvelles phrases canoniques ou nouveaux exemples).
4. Bump version mineure + ré-évaluation golden.
5. Déploiement v1.3.x → v1.4.0 (ou hotfix 1.3.1 si le changement est assez petit).

---

## 22. Plan de sprints

Mode 1 comme livrable autonome se décompose en **4 sprints de 1 semaine** :

| Sprint | Livrables | Critère sortie |
|---|---|---|
| **S1** | Bridge Python skeleton + embedder local + gRPC client + catalog loader | `classify("hello")` retourne un label sur un catalog de démo |
| **S2** | Cache, métriques, erreurs, CLI `efe-admin`, intégration M9 RegisterCone | Catalogue déployé + 100 classifications en bout à bout |
| **S3** | Bridges Claude + OpenAI + LangChain + exemple d'agent complet | Démo live agent répond correctement sur 20 cas |
| **S4** | Golden dataset + éval script + gate CI + runbook + doc utilisateur | F1 macro ≥ 82 %, doc publique prête |

Effort estimé : **1 développeur backend + 0,5 développeur ML, 4 semaines**.

---

## 23. Critères d'acceptation globaux

### 23.1 Fonctionnels

| ID | Critère |
|---|---|
| CA-M1-01 | `classify(text, catalog)` retourne toujours un `ClassifyResult` (jamais d'exception non gérée côté consommateur) |
| CA-M1-02 | p95 latence totale ≤ 200 ms sur embedder local |
| CA-M1-03 | Accuracy ≥ 85 % sur golden set de 200 exemples / 4 classes |
| CA-M1-04 | F1 macro ≥ 82 % |
| CA-M1-05 | Cache hit rate ≥ 40 % sur un traffic réaliste (répétitions d'intentions communes) |
| CA-M1-06 | Zero leak de texte dans `/metrics` et audit JSONL |
| CA-M1-07 | Support de ≥ 3 catalogues chargés simultanément |
| CA-M1-08 | Reconnaissance d'au moins 2 langues (EN + FR minimum) |
| CA-M1-09 | Fallback `uncertain` correctement déclenché à conf < 0.65 |
| CA-M1-10 | Circuit breaker fonctionnel sur ≥ 10 % erreurs en 1 min |

### 23.2 Sécurité

| ID | Critère |
|---|---|
| CA-M1-SEC-01 | JWT validé sur chaque appel en mode auth on |
| CA-M1-SEC-02 | Le texte utilisateur n'apparaît dans aucun log du bridge ou du serveur |
| CA-M1-SEC-03 | Les catalogues sont signés par un membre de l'équipe produit (revue git) |
| CA-M1-SEC-04 | Les cônes M9 sont partitionnés par tenant strictement |

### 23.3 Opérationnels

| ID | Critère |
|---|---|
| CA-M1-OPS-01 | Runbook publié : déploiement catalogue, rollback, audit PII |
| CA-M1-OPS-02 | Dashboard Grafana dédié (latence, accuracy live, drift) |
| CA-M1-OPS-03 | Golden set versionné, évalué à chaque PR catalog |
| CA-M1-OPS-04 | Procédure de rotation d'embedder documentée |

---

## Annexe A — Schéma YAML d'un catalogue d'ancres

Voir §8.1 pour un exemple complet. Schéma formel (JSON Schema-like) :

```yaml
type: object
required: [apiVersion, kind, metadata, embedder, classes]
properties:
  apiVersion:       { const: "efe.mode1/v1" }
  kind:             { const: "AnchorCatalog" }
  metadata:
    type: object
    required: [name, version]
    properties:
      name:         { type: string, pattern: "^[a-z][a-z0-9_-]{2,32}$" }
      version:      { type: string, pattern: "^\\d+\\.\\d+\\.\\d+$" }
      created_at:   { type: string, format: date-time }
      author:       { type: string }
      description:  { type: string }
  embedder:
    type: object
    required: [family, model, dim]
    properties:
      family:       { enum: [sentence-transformers, openai, cohere, huggingface] }
      model:        { type: string }
      dim:          { type: integer, minimum: 64, maximum: 4096 }
      normalize:    { enum: [l2, none], default: l2 }
  cone_policy:
    type: object
    properties:
      max_projection_distance: { type: number, minimum: 0.0, maximum: 2.0 }
      registered_in_m9:        { type: boolean, default: true }
      m9_key:
        type: object
        required: [tenant, task, version]
        properties:
          tenant:   { type: string }
          task:     { type: string }
          version:  { type: integer, minimum: 1 }
  classes:
    type: array
    minItems: 2
    maxItems: 32
    items:
      type: object
      required: [name]
      properties:
        name:                 { type: string, pattern: "^[a-z][a-z0-9_]{1,32}$" }
        description:          { type: string }
        color:                { type: string }
        canonical_sentences:  { type: array, items: {type: string}, minItems: 1 }
        examples_file:        { type: string }
        deprecated:           { type: boolean, default: false }
  thresholds:
    type: object
    properties:
      confidence_high:    { type: number, minimum: 0.5, maximum: 1.0, default: 0.85 }
      confidence_medium:  { type: number, minimum: 0.3, maximum: 0.9, default: 0.65 }
```

---

## Annexe B — Bridge Python — API publique

```python
# efe_tool/__init__.py
from .client import EfeClient, AsyncEfeClient
from .catalog import AnchorCatalog, ClassDef
from .embedder import Embedder, LocalSTEmbedder, OpenAIEmbedder
from .errors import BridgeError, ConfigError, NetworkError, LowConfidenceWarning
from .types import ClassifyResult

__version__ = "0.1.0"
__all__ = [
    "EfeClient", "AsyncEfeClient",
    "AnchorCatalog", "ClassDef",
    "Embedder", "LocalSTEmbedder", "OpenAIEmbedder",
    "ClassifyResult",
    "BridgeError", "ConfigError", "NetworkError", "LowConfidenceWarning",
    "__version__",
]

# efe_tool/types.py
@dataclass(frozen=True)
class ClassifyResult:
    label: str
    confidence: float
    alignment_delta: float
    rounds: int
    reject: bool
    swarm_id: str
    request_id: str
    text_hash: str
    catalog_version: str
    sub_scores: Mapping[str, float] | None = None

    def as_tool_result(self) -> dict:
        """JSON-serializable dict suitable for returning to an LLM."""
        return {
            "label": "uncertain" if self.reject else self.label,
            "confidence": round(self.confidence, 4),
            "rounds": self.rounds,
            "alignment_delta": round(self.alignment_delta, 4),
        }

# efe_tool/client.py (extrait)
class EfeClient:
    def __init__(
        self,
        endpoint: str,
        *,
        embedder: Embedder,
        jwt: str | None = None,
        timeout_s: float = 5.0,
        cache_size: int = 10_000,
        cache_ttl_s: int = 3600,
    ) -> None: ...

    @classmethod
    def from_env(cls) -> "EfeClient":
        return cls(
            endpoint=os.environ["EFE_ENDPOINT"],
            embedder=cls._load_embedder_from_env(),
            jwt=os.environ.get("EFE_JWT"),
        )

    def classify(
        self,
        text: str,
        catalog: AnchorCatalog,
        *,
        tenant: str | None = None,
        bypass_cache: bool = False,
    ) -> ClassifyResult: ...

    def healthcheck(self) -> bool: ...
    def close(self) -> None: ...
    def __enter__(self): return self
    def __exit__(self, *exc): self.close()
```

---

## Annexe C — Schéma de tool Claude & OpenAI

### C.1 Anthropic Claude (tools)

```python
tools = [{
    "name": "efe_classify",
    "description": "Classify a user utterance using an ephemeral fluid swarm. "
                   "Returns { label, confidence, alignment_delta }. "
                   "Labels come from the chosen catalog. "
                   "Treat confidence < 0.65 as uncertain.",
    "input_schema": {
        "type": "object",
        "properties": {
            "text":    {"type": "string"},
            "catalog": {"type": "string",
                        "enum": ["support_triage", "intent_detection"]},
        },
        "required": ["text", "catalog"],
    },
}]

response = anthropic.messages.create(
    model="claude-opus-4",
    max_tokens=1024,
    tools=tools,
    messages=[{"role": "user", "content": user_prompt}],
)
```

### C.2 OpenAI (function calling)

```python
functions = [{
    "type": "function",
    "function": {
        "name": "efe_classify",
        "description": "(same as above)",
        "parameters": {
            "type": "object",
            "properties": {
                "text":    {"type": "string"},
                "catalog": {"type": "string",
                            "enum": ["support_triage", "intent_detection"]},
            },
            "required": ["text", "catalog"],
        },
    },
}]

response = openai.chat.completions.create(
    model="gpt-4o",
    tools=functions,
    messages=[{"role": "user", "content": user_prompt}],
)
```

### C.3 MCP (Model Context Protocol)

```python
# efe_tool/bridges/mcp.py
from mcp.server import Server
server = Server("efe-mode1")

@server.tool("efe_classify")
async def efe_classify(text: str, catalog: str) -> dict:
    result = bridge.classify(text, catalog_by_name[catalog])
    return result.as_tool_result()
```

---

## Annexe D — Structure du golden dataset

### D.1 Fichiers

```
eval/
├── golden/
│   ├── support_triage.jsonl          # 200+ lignes
│   ├── intent_detection.jsonl
│   └── content_moderation.jsonl
├── baselines/
│   ├── zero_shot_simple_argmax.py    # sans essaim
│   └── random_forest_on_embed.py
├── reports/
│   └── support_triage_2026-04-19.md  # généré par eval script
└── scripts/
    ├── build_golden.py               # génère synthétiques puis demande
    │                                 # validation humaine
    └── drift_probe.py                # échantillonne la prod, re-label
```

### D.2 Schéma d'une ligne

```jsonl
{
  "id": "g0042",
  "catalog": "support_triage",
  "text": "Mon app bug dès que je clique sur envoyer",
  "expected": "bug",
  "language": "fr",
  "source": "human_labelled",
  "adversarial": false,
  "notes": null
}
```

### D.3 Métriques exportées

Le script d'éval produit :

- `report.json` : par classe {precision, recall, f1, support}, global
  {accuracy, f1_macro, confusion_matrix}, latence {p50, p95, p99}.
- `confusion.png` : heatmap matplotlib.
- `report.md` : synthèse humaine commitable.

---

## Annexe E — Modèles d'embedder recommandés

| Rang | Famille | Modèle | Dim | Latence CPU | Mémoire | Licence | Commentaire |
|---|---|---|---|---|---|---|---|
| **1** | sentence-transformers | `all-MiniLM-L6-v2` | 384 | 3-5 ms | 80 Mio | Apache 2.0 | **MVP anglais**, MTEB ~60 |
| 2 | sentence-transformers | `paraphrase-multilingual-MiniLM-L12-v2` | 384 | 6-8 ms | 420 Mio | Apache 2.0 | Multilingue 50+ langues |
| 3 | BAAI | `bge-base-en-v1.5` | 768 | 15-25 ms | 420 Mio | MIT | MTEB ~63, exige GPU en prod |
| 4 | BAAI | `bge-large-en-v1.5` | 1024 | 30-50 ms | 1,3 Gio | MIT | MTEB ~64, GPU obligatoire |
| 5 | OpenAI API | `text-embedding-3-small` | 1536 | 50-150 ms net | — | API | Multilingue excellent, 0,02 ¢/1k tok |
| 6 | OpenAI API | `text-embedding-3-large` | 3072 | 80-200 ms net | — | API | Top-tier, 0,13 ¢/1k tok |
| 7 | Cohere API | `embed-multilingual-v3.0` | 1024 | 60-150 ms net | — | API | Supporte 100+ langues |

Pour le MVP : **#1 ou #2** selon multilingue ou non.

---

## Annexe F — Matrice de risques Mode 1

| Risque | Prob. | Impact | Mitigation |
|---|---|---|---|
| Ancres mal choisies → accuracy faible | Élevée | Élevé | Golden set obligatoire + CI gate |
| Embedder switch oublié → catalogue corrompu | Moyenne | Critique | `embedder_fingerprint` vérifié au chargement |
| Dérive de distribution | Moyenne | Élevé | Alerte Prometheus + drift_probe hebdo |
| Fuite PII via logs | Faible | Critique | Text hash only + revue de code + fuzz |
| Cache toxique pendant dérive | Moyenne | Moyen | TTL court + invalidation manuelle à chaque release |
| Circuit breaker déclenché à tort | Faible | Moyen | Seuil configurable + dashboard |
| Catalogue v2 incompatible oublié | Moyenne | Élevé | Refus au boot si major mismatch |
| Attaque par injection d'ancres | Faible | Critique | Ancres en repo signé + revue obligatoire |
| API embedder externe rate-limitée | Moyenne | Moyen | Fallback local + retry + quota |
| Latence embedder > 50 ms | Moyenne | Moyen | Warm-up + pré-allocation + monitoring |
| Threshold mal calibré | Moyenne | Moyen | Auto-calibration par binary search sur golden |
| Langue non supportée | Moyenne | Moyen | Détection via fastText + routing catalogue multi-langue |

---

## Annexe G — Glossaire

| Terme | Définition |
|---|---|
| **Ancre** | Vecteur ℝ^D représentant le centre sémantique d'une classe |
| **Catalogue** | Fichier YAML listant classes + métadonnées d'un domaine |
| **Cône d'alignement** | Ensemble {ancres + max_projection_distance} enregistré côté M9 |
| **Embedder** | Modèle ou API transformant texte → vecteur |
| **Bridge** | Librairie/sidecar qui fait le pont entre agent LLM et serveur EFE |
| **Golden set** | Dataset étiqueté humain pour évaluation qualité |
| **Confidence** | Score [0,1] de sortie de l'essaim ; basé sur 6 signaux (M6) |
| **Alignment delta** | Distance L2 entre centroïde et sa projection sur le cône |
| **Reject zone** | Confiance < threshold_medium → retour `uncertain` |
| **Drift** | Dérive de distribution des labels ou de la confiance |
| **Self-consistency** | Méthode LLM : k générations + vote (à distinguer de Mode 2) |
| **Tool / Function calling** | Mécanisme des SDK LLM pour invoquer des fonctions externes |
| **MCP** | Model Context Protocol, standard ouvert pour tools LLM |
| **Zero-shot** | Classification sans exemples d'entraînement pour le modèle de fond |
| **RegisterCone / EvolveCone** | RPCs M9 pour gérer le cycle de vie des cônes |
| **AnchorCatalog / ClassDef** | Types Python publics du bridge |
| **`NearestAnchorDecoder`** | Décodeur EFE qui renvoie l'index de l'ancre la plus proche |
| **Circuit breaker** | Pattern fail-fast temporaire à taux d'erreur élevé |
| **Canonical sentence** | Phrase humaine représentant prototypiquement une classe |

---

## Fin du document

**État** : approuvé pour implémentation.
**Version** : 1.0 (Mode 1 MVP).
**Prochaine révision prévue** : post-premier déploiement client. Changements attendus : hybride Mode 1 + 2 pour les cas mixtes, apprentissage continu des ancres par feedback loop.

Toute divergence entre ce document et l'implémentation doit être tracée dans `CHANGELOG.md` sous une section « Architectural decisions (Mode 1) ».
