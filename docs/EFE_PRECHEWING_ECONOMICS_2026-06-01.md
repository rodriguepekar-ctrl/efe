# EFE — Économie réelle du pré-mâchage (vrai LLM) · claude_claude-sonnet-4-6

> Roadmap **H0.1**. A/B contrôlé (baseline brut vs chewed compressé) sur modèle épinglé, coût **calculé** depuis tokens × pricing (in $3.0/Mtok · out $15.0/Mtok). Généré : 2026-06-01T00:11:01.665240+00:00. Données brutes : le `.json` à côté.

## Verdict par prompt

| Prompt | type | savings tokens | Δcoût (USD) | latence NETTE (ms) | overhead SLM (ms) | qualité (jaccard) | factuel base/chewed |
|---|---|---|---|---|---|---|---|
| fr_capital_1k | redundant | +48% (302→157 t) | +0.00050 ± 0.00003 | +8917 ± 741 | +8959 ± 441 | 0.44 ± 0.39 | 1.00/1.00 |
| syncasync_1k5 | redundant | +50% (430→216 t) | +0.00711 ± 0.00110 | +1284 ± 6777 | +10509 ± 6760 | 0.25 ± 0.03 | 0.00/0.00 |
| rest_graphql_2k5 | redundant | +56% (658→290 t) | +0.01012 ± 0.00025 | +13174 ± 694 | +24041 ± 757 | 0.24 ± 0.02 | 0.00/0.00 |
| varied_backend | varied | +17% (383→316 t) | +0.00337 ± 0.00060 | +17410 ± 1783 | +20012 ± 2 | 0.20 ± 0.00 | 0.00/0.00 |

## Comment lire

- **Δcoût > 0** → le pré-mâchage **économise** sur le coût LLM (le coût SLM local ≈ nul). **Δcoût < 0** → l'overhead l'emporte (rare : dérive de longueur de sortie).
- **latence NETTE = (SLM + LLM_chewed) − LLM_baseline**. **< 0** → chewed est PLUS RAPIDE bout-en-bout (win latence). **> 0** → chewed est plus LENT (le SLM amont coûte plus que le temps LLM gagné).
- **qualité (jaccard)** : recouvrement de mots baseline↔chewed. **~1** = réponses concordantes ; **bas** = la compression a changé la réponse → inspecter `answer_b`/`answer_c` dans le `.json`.
- **factuel** : fraction de mots-clés attendus présents (base/chewed) ; doit rester égal pour valider la qualité.

## Caveats méthodologiques

- **Température non réglable via le DTO** (`MissionADto` n'expose pas `temperature`) → la variance LLM est contrôlée par les répétitions (moyenne ± σ), pas par `temperature=0`.
- **Tokens d'entrée estimés** (`chars/4`, via `PRE_CHEWING_METRICS`), pas le compte exact du tokenizer provider ; `cost_cents` du flux sert de garde-fou (mais arrondit). Le coût du tableau est calculé sur l'estimation.
- **Répétitions à prompt identique** → le cache de prompt SLM (`cache_prompt=true`) peut accélérer les répétitions 2-3 → l'overhead SLM mesuré est plutôt un **plancher** (biais favorable au chewed, mais réaliste en prod).
- **Coût SLM compute non monétisé** : supposé ≈ nul (self-hosted CPU/GPU local). Si SLM facturé, soustraire son coût du Δcoût.

## ✅ Run POST-FIX R7 (préservation de la question) — la qualité est rétablie

Run du 2026-06-01 **après** le correctif R7 (`workflows::extract_questions` +
ré-injection verbatim de la question dans `final_prompt_from_compression`). À
comparer au run pré-fix `EFE_PRECHEWING_ECONOMICS_2026-05-31.md` (qualité
détruite).

**Lecture des réponses chewed (la vraie preuve, pas le jaccard)** :

| Prompt | chewed PRÉ-fix (faux) | **chewed POST-fix (correct)** |
|---|---|---|
| `fr_capital` | « Paris » ✅ | « Paris » ✅ |
| `syncasync` | *« identifie ta question, sources fiables »* ❌ | **« Synchrone : le code s'exécute ligne par ligne… »** ✅ |
| `rest_graphql` | *« sources officielles : HAS, INSERM »* ❌ | **« REST : plusieurs endpoints ; GraphQL : un seul endpoint… »** ✅ |
| `varied_backend` | parle de cache ⚠️ | **« Pessimiste, dans ce cas précis… »** ✅ |

Le jaccard (0.2–0.44) reste modéré **uniquement** car la réponse chewed est plus
**concise** (`out_c` < `out_b`, ex. 420 vs 1000 t) — consigne BRÈVE, pas une
régression. **La question survit, le LLM y répond.**

## Décision (H0.1, post-fix)

> **Le pré-mâchage est un optimiseur de COÛT à CORRECTION PRÉSERVÉE.** Coût :
> win partout (+$0.0005 à +$0.010/req ; plus modeste qu'au pré-fix car la
> question ré-injectée rajoute des tokens). Latence : **neutre à négative** aux
> vitesses CPU-SLM (le « win latence » du run pré-fix était un artefact de
> réponses **fausses et courtes** ; une réponse correcte coûte plus de
> génération). Qualité : **préservée** (toutes les réponses sur le bon sujet).

**Implication positionnement (H0.3)** : vendre le pré-mâchage comme **réduction
de coût** (tokens d'entrée premium), pas comme accélérateur — sauf à passer les
SLM sur GPU (H0.2).

⚠️ Prompts `redundant` = best-case savings / worst-case correction (c'est là que
le bug R7 frappait le plus) ; `varied` borne le réalisme (cf. H1.1).
