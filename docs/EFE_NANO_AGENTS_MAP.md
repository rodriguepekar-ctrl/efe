# EFE — Cartographie des nano-agents

> **Source unique de vérité de la flotte nano-agents** + audit de santé daté.
> Ce doc existe parce que la mémoire de la flotte a fui (« il n'y avait pas des
> dizaines ? ») : la vraie réponse est **~33 nano-agents** sur 3 familles, pas
> les 6 robots du Preview Lab. Toute affirmation chiffrée ici porte sa **commande
> de régénération** (§12) → le doc est **vérifiable**, pas une photo qui pourrit.

**Audit du : 2026-06-01.** À re-vérifier avec les commandes du §12 après tout
sprint touchant `crates/efe-nano*`.

---

## 0. Comment lire ce doc

- Un **nano-agent** = une unité mono-tâche bon marché qui prépare/traite avant
  (ou autour) d'un LLM premium. C'est la mécanique du « pré-mâchage ».
- Statuts : 🟢 sain · 🟡 stub fonctionnel · ⚪ autre usage (hors pré-mâchage) ·
  🔴 cassé.
- 3 familles + 1 runtime + 1 couche d'orchestration au-dessus.

## 1. Le concept unificateur — 3 modes d'exécution

L'échelle coût/qualité qui sous-tend tout l'écosystème (= la thèse pré-mâchage :
faire le max en A/B avant de payer le C) :

| Mode | Alias code | Moyen | Coût |
|---|---|---|---|
| **A** | `PureRust` | code pur, zéro modèle | gratuit, instantané |
| **B** | `LocalModel` / SLM | petit modèle local (SLM/ONNX) | cheap |
| **C** | `LlmAugmented` | LLM premium (Opus/Haiku-class) | cher, haute qualité |

## 2. Vue d'ensemble

| Catégorie | Nombre | Crate |
|---|---|---|
| Nano-apps (starter pack) | **20** | `efe-nano` |
| Robots SLM (pré-mâchage) | **6** | `efe-nano-app-coordinator` |
| Composants de pré-analyse | **7** | `efe-nano-preanalysis` |
| **Total nano-agents** | **≈ 33** | |
| Runtime nano | 1 | `efe-nano` |
| Couche d'orchestration d'agents | 4 crates | `efe-{swarm-orchestrator, meta-swarm, agent-runner, agentic-bridge}` |
| Crates de support nano | 3 | `efe-nano-{sdk, inference-daemon, swarm-adapter}` |

> ⚠️ **Le Preview Lab (bridge SSE + frontend) n'exerce que 6 des ~33 nano-agents.**
> Les 20 du starter pack et les 7 de pré-analyse vivent ailleurs (runtime nano,
> gateway, orchestrateur).

---

## 3. Famille 1 — `efe-nano` : runtime + **20 nano-apps** (starter pack)

Un moteur d'exécution complet : `dispatcher` · `scheduler` · `budget` ·
`metering` · `replay` · `idempotency` · `audit` · `catalog`. Il enregistre 20
nano-apps mono-tâche (toutes TIER-1), chacune un **manifeste déclaratif**
(`execution_mode`, modèle, coût en tokens, `determinism`, stratégie PII) + sa
logique. Source : `crates/efe-nano/src/starter_pack/`.

### Mode A — PureRust (10) — code pur, zéro modèle
| App | Rôle |
|---|---|
| `csv_normalize` | parse CSV → headers + rows |
| `currency_normalize` | parse devises (US/EU/Indien) |
| `date_normalize` | date → ISO 8601 + timestamp |
| `html_extract_main_content` | texte principal d'un HTML |
| `json_validate` | valide JSON vs JSON Schema 2020-12 |
| `markdown_extract_headings` | extrait les titres markdown |
| `pdf_extract_text` | extrait le texte d'un PDF |
| `regex_extract` | regex → matches |
| `sql_parse_safe` | vérifie SQL = SELECT-only sûr |
| `xml_validate` | valide la bonne-formation XML |

### Mode B — LocalModel (5) — petit modèle local
| App | Rôle |
|---|---|
| `embed_text` | embedder déterministe 384-dim |
| `intent_classify_generic` | classif d'intention par mots-clés |
| `lang_detect` | détection langue (mots-fonction) |
| `ner_clinical` | NER biomédical (DRUG/DISEASE/GENE/PHASE) |
| `ner_legal` | NER juridique (CASE/CITATION/PARTY/CLAUSE) |

### Mode C — LlmAugmented (5) — LLM premium
| App | Rôle |
|---|---|
| `classify_topic_llm` | classif de sujet libre |
| `extract_structured` | extraction de champs structurés |
| `qa_grounded` | Q&A sourcé avec citations |
| `sentiment_nuanced` | sentiment multi-aspect |
| `summarize_strict` | résumé à nombre de phrases capé |

---

## 4. Famille 2 — `efe-nano-app-coordinator` : **6 robots SLM** (swarm de pré-mâchage)

**La seule famille câblée dans le Preview Lab** — celle corrigée durant la
session du 2026-06. Source : `crates/efe-nano-app-coordinator/src/clients/`.

| Robot | Modèle | Port | Rôle | Preview Lab | Statut |
|---|---|---|---|---|---|
| `analyzer` | SmolLM2-360M | :8090 | extraction d'entités | ✓ | 🟢 corrigé (entrée bornée) |
| `reformulator` | Qwen3-0.6B | :8091 | **normalisation de SORTIE** (dialecte LLM → schéma canonique) | ✗ | ⚪ autre usage |
| `compressor` | Llama-3.2-1B | :8092 | compression de prompt | ✓ | 🟢 corrigé (+72%) |
| `verifier` | Phi-4-mini Q4 | :8093 | fact-check vs `candidate_sources` | ✓ | 🟢 verdicts |
| `filter` | bge-m3 (ONNX/fastembed) | in-proc | filtrage par embeddings + cosine | ✗ | ⚪ autre usage |
| `router` | bge-m3 + heuristiques Pareto | in-proc | sélection LLM | ✓ | 🟢 |

> ⚠️ **Pièges de lecture (erreurs commises et corrigées) :**
> - Le `reformulator` **n'est PAS** un reformulateur de requête d'entrée. Son
>   prompt le dit : il **canonise la SORTIE** d'un LLM (Claude XML / GPT-4 JSON /
>   Gemini Markdown / Mistral → `citation_v1` / `fact_v1` / `tool_call_v1` /
>   `claim_v1` / `source_ref_v1`). Le câbler dans Mission A/B serait une erreur.
> - Le `filter` est un robot **ONNX d'embeddings** (bge-m3), pas un SLM
>   llama-server (d'où l'absence de grammaire/prompt). MockEmbedder par défaut.
> - `reformulator` et `filter` ont des clients + tests complets (`live_smoke.rs`)
>   mais ne sont **pas** dans le pipeline Q&A du Preview — c'est **correct**.

Flux Mission A : `analyzer → router → compressor → LLM → verifier`.

---

## 5. Famille 3 — `efe-nano-preanalysis` : **7 composants** de pré-analyse

Couche de prétraitement (Sprint 13/15). Source :
`crates/efe-nano-preanalysis/src/`.

| Composant | Mode | Lignes | État |
|---|---|---|---|
| `language_detector` | A pur | 287 | 🟢 complet |
| `ner_extractor` | A pur | 326 | 🟢 complet |
| `pii_redactor` | A pur (125 réfs regex/redaction) | 354 | 🟢 complet — **sécurité-critique** |
| `schema_validator` | A pur | 359 | 🟢 complet |
| `catalog` | back-fill synthétique | 543 | 🟢 utilitaire dev |
| `sentiment_classifier` | B (mots-clés) | 256 | 🟡 **stub fonctionnel** → modèle local (Sprint 14) |
| `summary_writer` | C (N premières phrases) | 255 | 🟡 **stub fonctionnel** → Haiku (Sprint 14) |

**Consommée par 4 crates** (donc centrale, pas orpheline) :
`efe-gateway` · `efe-swarm-orchestrator` · `efe-compliance-wiring` ·
`efe-nano-swarm-adapter`.

> Les 2 « stubs » **ne sont pas du code cassé** : ils produisent des résultats
> déterministes corrects (sentiment par mots-clés ; résumé = N premières phrases
> tronquées), avec un chemin d'upgrade documenté vers modèle/LLM. Ils existent
> « pour que le swarm orchestrator puisse être testé maintenant ».

---

## 6. Crates de support nano

| Crate | Rôle |
|---|---|
| `efe-nano-sdk` | SDK pour écrire des nano-apps |
| `efe-nano-inference-daemon` | daemon d'inférence SLM |
| `efe-nano-swarm-adapter` | pont nano ↔ swarm |

## 7. Couche AU-DESSUS — orchestration d'agents (≠ nano-SLM)

Au-delà des nano-agents, une couche d'**agents LLM orchestrés** (mentionnée pour
ne pas la reconfondre avec les nano-robots) :

| Crate | Rôle |
|---|---|
| `efe-swarm-orchestrator` | orchestration **DAG** d'agents (`agent.rs`, `dag.rs`, `compliance.rs`) |
| `efe-meta-swarm` | swarm de swarms (`builder`, `certification`, `iteration`) |
| `efe-agent-runner` | exécute des agents avec **approbation + contrôle de coût** |
| `efe-agentic-bridge` | pont agentic → Claude (`claude.rs`, `claude_stream`) |

---

## 8. Audit de santé (2026-06-01)

| Famille | Tests | Clippy | Bugs réels |
|---|---|---|---|
| Starter pack (`efe-nano`) | **940 ✓** | 0 warning | **0** |
| Pré-analyse (`efe-nano-preanalysis`) | **80 ✓** | 0 warning | **0** (2 stubs fonctionnels) |
| Pré-mâchage SLM (`efe-nano-app-coordinator`) | **118 ✓** | 0 warning | **3 trouvés + corrigés** (live) |
| Coordinator (`efe-coordinator`) | **341 ✓** | — | — |
| Bridge (`efe-preview-server`) | **66 ✓** (27+19+20) | 0 warning | — |

**Détails starter pack** : 20/20 apps testées, aucune app creuse (100→468 lignes,
4→11 tests chacune). Les « stubs » trouvés sont de l'**infra documentée**
(VmPool→Firecracker, audit→HMAC), pas de la logique d'app manquante.

**Les 4 bugs SLM corrigés cette session** (cf. CHANGELOG `[Unreleased]`) — tous
**invisibles aux unit-tests**, trouvés en live :
1. Compresseur qui **gonflait** les prompts (savings −0.71) → encadrement terse +
   gate tokens (`41d202b`).
2. Compresseur qui **ne compressait pas** (cap grammaire fixe) → brièveté forcée
   par grammaire (`9a9634e`).
3. Analyzer qui **échouait >1000 t** → entrée bornée (`6725336`).
4. Compresseur qui **larguait la question** (extrait le filler dominant) → le LLM
   premium répondait au bruit → ré-injection verbatim de la question (`fc7bc95`).
   **Révélé uniquement par un VRAI LLM** (H0.1) ; le mock le cachait.

## 9. 🔑 Insight — où vit le risque

> **La dette n'est PAS répartie uniformément.** Les couches **pur-code /
> déterministes** (starter pack + pré-analyse ≈ 27 des ~33 agents) sont **béton** :
> exhaustivement unit-testées, clippy-clean, **zéro bug**. **Tout** le risque réel
> se concentre sur la **frontière SLM live** — les 6 robots, là où les modèles
> 360M/1B/Phi-4 dégénèrent, expansent, timeoutent — **invisible aux unit-tests**.
> Corollaire opérationnel : **les tests live sont non négociables** sur cette
> frontière ; le durcissement futur va là, pas dans le code pur.

## 10. Matrice de câblage (qui sert où)

| Consommateur | Nano-agents utilisés |
|---|---|
| Preview Lab (bridge + frontend) | analyzer, compressor, verifier, router (4/6 robots) |
| `efe-gateway` | pré-analyse |
| `efe-swarm-orchestrator` | pré-analyse |
| `efe-compliance-wiring` | pré-analyse |
| `efe-nano-swarm-adapter` | pré-analyse |
| Runtime `efe-nano` | les 20 nano-apps (starter pack) |

## 11. Positionnement (FIGÉ — H0.1) & trous connus

- **🎯 POSITIONNEMENT (mesuré vrai LLM, A/B vs claude-sonnet-4-6, post-fix R7 —
  cf. [`EFE_PRECHEWING_ECONOMICS_2026-06-01.md`](EFE_PRECHEWING_ECONOMICS_2026-06-01.md))** :
  le pré-mâchage est un **OPTIMISEUR DE COÛT à correction préservée**, **PAS** un
  accélérateur.
  - **Coût** : win (−tokens d'entrée premium ; +$0.0005–0.010/req sur ces prompts).
  - **Correction** : préservée depuis `fc7bc95` (la question survit).
  - **Latence** : **neutre à négative** aux vitesses CPU-SLM (l'overhead SLM
    amont dépasse le temps LLM gagné). Le « win latence » observé au run **pré-fix**
    était un **artefact** de réponses fausses & courtes.
  - → **H0.2 (spike GPU, keyless, FAIT)** : le GPU est **nécessaire** (en CPU le
    compresseur timeoute, 15-25 s) mais **PAS suffisant** pour renverser la
    latence. Le mur n'est pas le débit GPU mais la **sérialisation** des appels
    map-reduce du compresseur (N appels SLM en série + cold-start). Le vrai levier
    est **architectural**, pas matériel.
  - → **L1 (FAIT, mesuré keyless)** : seuil map-reduce `SAFE_CHUNK_TOKENS`
    **300 → 512** (`compressor.rs`) → la bande 300-512 t passe de 3-4 appels série
    à **un seul**. Mesure (prompt 383 t, 5 reps, SLM Metal réels, LLM mock) :
    compresseur **6523 → 1819 ms (−72 %)**, SLM total **9972 → 5689 ms (−43 %)**,
    **savings préservés** (+24 %, sortie identique ~290 t). **Premier gain latence
    franc du projet.** ⚠ C'est la latence de l'**étage SLM** ; le Δlatence NETTE
    bout-en-bout vs baseline LLM reste à re-mesurer (vrai LLM). Le positionnement
    « optimiseur de coût » **tient** — L1 le complète, ne le contredit pas.
  - → **L2 (FAIT, mesuré keyless) — GARDÉ, mais ROI borné GPU.** MAP du
    compresseur parallélisé (`join_all` par fenêtres, `MAP_CONCURRENCY=4` +
    serveur `--parallel 4`/ctx 8192). Mesure (prompt 1922 t / ~4 chunks, même
    serveur, client `=1` vs `=4`) : compresseur **10637 → 8224 ms = ~1,3×**
    (jusqu'à ~1,5× reps propres), savings **+76,7 % préservés**. ⚠ **Ma projection
    (2,5×) était FAUSSE** : les N slots partagent **un seul GPU** → `--cont-batching`
    batche (speedup **sous-linéaire** ~1,4×), pas du compute indépendant. **Le ROI
    ne décolle qu'en MULTI-GPU** (1 instance compresseur/GPU). SLM total quasi
    immobile (−1,4 %) car l'**analyzer** domine l'étage SLM (inférence, PAS
    cold-start → cf. C-B). Code correct (test wiremock prouve la concurrence) ;
    c'est le **plafond matériel**, pas un bug. Réversible :
    `EFE_COMPRESSOR_MAP_CONCURRENCY=1` / `ROBOT_PARALLEL=(1 1 1 1)`.
  - → **L3 (FAIT, mesuré keyless) — NO-OP : le « cold-start ~5 s » était un
    MISDIAGNOSTIC.** A/B boot froid (`EFE_SLM_WARMUP=0`) vs chaud (`=1`) : 1ère
    requête **4808 vs 4851 ms** = Δ dans le bruit. Le ~4,7 s n'est pas une
    compilation de shaders amortissable mais la **latence d'inférence steady-state
    de l'analyzer** (lente à CHAQUE requête). Code warmup correct (chauffe au boot)
    mais sans effet ici → **non commité** (start.sh), disposition à trancher. Aurait
    pu aider sur un substrat à vrai cold-start (CUDA/serverless).
  - → **C-B (FAIT, mesuré, commité `d05c1a4`) — le VRAI levier.** L'analyzer = 4,7 s
    = **100 % décode** (682 t @ 179 tok/s) d'un output **non-gating** (entités →
    blackboard write-only, 0 lecture aval) et **hallucinées**. Cap grammaire
    `entities {0,19}→{0,4}` (+ schéma `maxItems 5` + `max_tokens 1024→512`). Mesuré :
    décode **3809 → 1369 ms (−64 %)**, **254 t** (−63 % coût GPU), 5 entités, JSON
    complet. Qualité réelle préservée. **Plus de gain que L1+L2 réunis** sur le
    chemin critique réel. ⚠ `max_entities` n'atteint pas le modèle → la **grammaire**
    est le seul cap.
- **Méthodo** : ces 4 bugs (cf. §8) n'étaient visibles qu'en **live + lecture des
  réponses** ; le mock LLM (réponses canned) les masquait tous. **L1 ET L2 ont été
  validés par le même principe** : mesure live (bridge SLM réel) — et la mesure L2
  a **corrigé une projection optimiste** (2,5× → 1,3×), comme H0.1/H0.2.
- **2 stubs pré-analyse** (`sentiment_classifier`, `summary_writer`) → upgrade
  modèle/LLM planifié (Sprint 14).
- **`reformulator` / `filter`** : construits + testés mais hors pré-mâchage Q&A
  (autre usage : canonisation de sortie / filtrage sémantique).

## 12. 🧪 Provenance & régénération (anti-pourrissement)

Toutes les commandes qui ont produit les chiffres de ce doc. **Re-jouer après
tout sprint touchant `efe-nano*`** ; si un résultat diffère, mettre à jour le doc.

```bash
# Nb de nano-apps starter pack (attendu : 20)
ls crates/efe-nano/src/starter_pack/*.rs | grep -v mod.rs | wc -l

# Répartition par mode (attendu : 10 PureRust / 5 LocalModel / 5 LlmAugmented)
# ⚠ PIÈGE : exclure mod.rs (sinon +1 PureRust parasite → faux total de 21), et
#   prendre la 1re occurrence par fichier (les tests mentionnent d'autres modes).
for f in crates/efe-nano/src/starter_pack/*.rs; do
  [ "$(basename "$f")" = "mod.rs" ] && continue
  grep -m1 -oE "ExecutionMode::(PureRust|LocalModel|LlmAugmented)" "$f"
done | sort | uniq -c

# Santé starter pack (attendu : 940 passed, clippy clean)
cargo test  -p efe-nano --lib
cargo clippy -p efe-nano --lib

# Santé pré-analyse (attendu : 80 passed)
cargo test  -p efe-nano-preanalysis --lib

# Santé robots pré-mâchage (attendu : 122 passed — 118 + 4 tests L1)
cargo test  -p efe-nano-app-coordinator --lib

# Santé aval (attendu : coordinator 341, bridge 27+19+20=66)
cargo test  -p efe-coordinator --lib
cargo test  -p efe-preview-server

# Modèles + ports des 6 robots (vérifie le §4 : SmolLM2-360M / Qwen3-0.6B /
# Llama-3.2-1B / Phi-4-mini / bge-m3 ; ports 8090-8093)
grep -h "Client du robot" crates/efe-nano-app-coordinator/src/clients/*.rs

# Inventaire des crates nano
ls -1 crates | grep -i nano

# Consommateurs de la pré-analyse (attendu : 4)
grep -rl "efe-nano-preanalysis" crates --include=Cargo.toml | grep -v "efe-nano-preanalysis/Cargo"

# L1 — seuil map-reduce (attendu : 512) + tests de frontière de dispatch
grep -n "const SAFE_CHUNK_TOKENS" crates/efe-nano-app-coordinator/src/clients/compressor.rs
cargo test -p efe-nano-app-coordinator --lib clients::compressor::chunking_tests::l1_

# L1 — repro de la mesure keyless (même binaire, l'env bascule le seuil ;
# bridge LLM mock + SLM Metal réels → coût LLM nul). Lancer 2 fois, comparer
# `compressor_latency_ms` dans l'event SSE PRE_CHEWING_METRICS :
#   pré-L1  : EFE_COMPRESSOR_CHUNK_TOKENS=300
#   post-L1 : EFE_COMPRESSOR_CHUNK_TOKENS=512  (= défaut)
# (SLM lancés via ./scripts/slm/start.sh ; un prompt de 300-512 t straddle la bande)
EFE_PREVIEW_MOCK=1 EFE_LIVE_SLM=1 EFE_COMPRESSOR_CHUNK_TOKENS=512 \
  EFE_PREVIEW_ADDR=127.0.0.1:8710 ./target/debug/efe-preview-server
```

## 13. Maintenance

**Règle** (alignée sur `docs/architecture/EFE_M_NUMBER_REGISTRY.md`) : tout
nouveau nano-agent, changement de modèle/port, ou nouveau câblage → **mettre à
jour ce doc** dans le même PR, et re-jouer le §12. Ce fichier est la **source
canonique** ; les arch docs de sprint le **référencent**, ne le dupliquent pas.
