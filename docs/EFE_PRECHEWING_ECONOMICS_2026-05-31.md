# EFE — Économie réelle du pré-mâchage (vrai LLM) · claude_claude-sonnet-4-6

> Roadmap **H0.1**. A/B contrôlé (baseline brut vs chewed compressé) sur modèle épinglé, coût **calculé** depuis tokens × pricing (in $3.0/Mtok · out $15.0/Mtok). Généré : 2026-05-31T23:45:48.568394+00:00. Données brutes : le `.json` à côté.

## Verdict par prompt

| Prompt | type | savings tokens | Δcoût (USD) | latence NETTE (ms) | overhead SLM (ms) | qualité (jaccard) | factuel base/chewed |
|---|---|---|---|---|---|---|---|
| fr_capital_1k | redundant | +61% (302→117 t) | +0.00056 ± 0.00000 | +8176 ± 1000 | +8135 ± 1066 | 1.00 ± 0.00 | 1.00/1.00 |
| syncasync_1k5 | redundant | +63% (430→158 t) | +0.01219 ± 0.00183 | -7718 ± 1487 | +5960 ± 1120 | 0.04 ± 0.01 | 0.00/0.00 |
| rest_graphql_2k5 | redundant | +75% (658→165 t) | +0.01564 ± 0.00001 | -9129 ± 1398 | +7511 ± 963 | 0.04 ± 0.00 | 0.00/0.00 |
| varied_backend | varied | +62% (383→144 t) | +0.01382 ± 0.00020 | -6383 ± 2211 | +8843 ± 3459 | 0.08 ± 0.01 | 0.00/0.00 |

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

## ⚠️ Verdict QUALITÉ — lecture des réponses (le jaccard bas n'est PAS de la brièveté)

**La table coût/latence ci-dessus est TROMPEUSE prise seule.** Lecture des
réponses réelles (`answer_b` vs `answer_c` dans le `.json`) :

| Prompt | Réponse baseline | Réponse **chewed** | Verdict |
|---|---|---|---|
| `fr_capital` | « Paris » | « Paris » | ✅ correct |
| `syncasync` | explique sync/async | *« identifie ta question, consulte des sources fiables »* | ❌ **hors-sujet** |
| `rest_graphql` | explique REST/GraphQL | *« sources officielles : HAS, INSERM… »* (autorités **médicales**) | ❌ **totalement faux** |
| `varied_backend` | locking pessimiste/optimiste | parle de cache / vue matérialisée | ⚠️ mauvais sous-sujet |

**Cause racine** : sur les prompts à fort remplissage, le compresseur 1B
**extrait le filler** (« j'ai lu des choses contradictoires sur des forums… »)
et **jette la question**, malgré l'instruction de son prompt (*"you MUST
preserve any question"*, `assets/prompts/compressor.txt`). Le prompt compressé
ne contient plus la question → le LLM répond au bruit.

**Ironie** : plus le prompt est redondant (plus on « économise »), pire est la
correction — l'extraction garde le contenu dominant, qui est le bruit.

## Décision (H0.1)

> **Le pré-mâchage économise sur le COÛT (−61% à −75% de tokens d'entrée,
> +$0.011–0.016/req) et sur la LATENCE pour les prompts > ~1700 car (chewed
> PLUS RAPIDE de 6–9 s — ce qui RENVERSE le verdict du sweep mock). MAIS la
> qualité est DÉTRUITE (le compresseur 1B largue la question) → NON VIABLE pour
> du Q&A en l'état. Les économies sont illusoires sur une réponse fausse.**

**Blocage identifié** : la compression doit **préserver la question verbatim**.
Tant que ce n'est pas corrigé, les wins coût/latence ne comptent pas.

**Ce que H0.1 a prouvé en plus** : le mock cachait ce bug (réponses canned) ;
seul un VRAI LLM + **lecture des réponses** le révèle. Nos corrections
compresseur antérieures validaient le `savings_ratio`, jamais la correction de
la réponse produite.

---
*Prompts `redundant` = best-case savings / worst-case correction ; `varied`
borne le réalisme (cf. roadmap H1.1). Section qualité annotée manuellement après
lecture des réponses du `.json`.*
