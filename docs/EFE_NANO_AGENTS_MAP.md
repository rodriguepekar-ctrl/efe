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

**Les 3 bugs SLM corrigés cette session** (cf. CHANGELOG `[Unreleased]`) :
1. Compresseur qui **gonflait** les prompts (savings −0.71) → encadrement terse +
   gate tokens (`41d202b`).
2. Compresseur qui **ne compressait pas** (cap grammaire fixe) → brièveté forcée
   par grammaire (`9a9634e`).
3. Analyzer qui **échouait >1000 t** → entrée bornée (`6725336`).

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

## 11. Caveats & trous connus

- **Latence** (sweep d'économie, 2026-06-01) : le pré-mâchage est un optimiseur
  de **COÛT** (−56% à −91% de tokens), **pas de latence** (+5 à +19 s de SLM
  amont) aux vitesses CPU-SLM. Décision de positionnement à trancher (roadmap H0).
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

# Santé robots pré-mâchage (attendu : 118 passed)
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
```

## 13. Maintenance

**Règle** (alignée sur `docs/architecture/EFE_M_NUMBER_REGISTRY.md`) : tout
nouveau nano-agent, changement de modèle/port, ou nouveau câblage → **mettre à
jour ce doc** dans le même PR, et re-jouer le §12. Ce fichier est la **source
canonique** ; les arch docs de sprint le **référencent**, ne le dupliquent pas.
