# EFE — Economie stratifiee du pre-machage (vrai LLM) · mock-claude

> Roadmap **H1.1**. A/B controle (baseline brut vs chewed compresse), corpus **stratifie par famille**, cout **calcule** depuis tokens × pricing (in $3.0/Mtok · out $15.0/Mtok). Corpus : scripts/bench/corpus/prechew_corpus_v1.json (version v1, 26 prompts)  [filtre familles : clean_concise_long]. Juge LLM : OFF. Genere : 2026-06-02T14:53:55.642959+00:00. Donnees brutes : le `.json` a cote.

## 1. Verdict par famille (le rollup)

| Famille | n | savings IN | reduction SORTIE | part sortie | Δcout (USD) | latence NETTE (ms) | qualite (Δrubric) | mesure | regle |
|---|---|---|---|---|---|---|---|---|---|
| F2 propre/concis long | 7 | +39% | +0% | +0% | +0.00038 ± 0.00026 | +2577 ± 911 | +0.00 (n=7) | cout ~0 · plus lent | **NE PAS chewer** |

## 2. Decomposition du cout — ou vit le gain (entree vs SORTIE)

> Le verdict H0.1 affirmait que ~88 % du gain vient de la **sortie** (induction de brievete), pas de l'entree compressee. Cette section le **chiffre** par famille. `part sortie` = Δcout_out / Δcout_total.

| Famille | Δcout entree (USD) | Δcout SORTIE (USD) | part sortie | reduction sortie |
|---|---|---|---|---|
| F2 propre/concis long | +0.00038 ± 0.00026 | +0.00000 ± 0.00000 | +0% | +0% |

## 3. Detail par prompt

| Prompt | famille | savings IN | red. sortie | Δcout (USD) | latence NETTE (ms) | overhead SLM (ms) | rubric b/c | style (jaccard) |
|---|---|---|---|---|---|---|---|---|
| f2_idempotency | clean_concise_long | +0% (336→336 t) | +0% (7→7 t) | +0.00000 ± 0.00000 | +1390 ± 491 | +1390 ± 491 | 0.00/0.00 | 1.00 |
| f2_cap_theorem | clean_concise_long | +0% (333→333 t) | +0% (7→7 t) | +0.00000 ± 0.00000 | +2120 ± 2015 | +2120 ± 2015 | 0.00/0.00 | 1.00 |
| f2_index_btree | clean_concise_long | +36% (325→208 t) | +0% (7→7 t) | +0.00035 ± 0.00029 | +1422 ± 475 | +1422 ± 475 | 0.00/0.00 | 1.00 |
| f2_oauth_pkce | clean_concise_long | +60% (322→130 t) | +0% (7→7 t) | +0.00058 ± 0.00000 | +3369 ± 89 | +3369 ± 89 | 0.00/0.00 | 1.00 |
| f2_gc_generational | clean_concise_long | +60% (321→130 t) | +0% (7→7 t) | +0.00057 ± 0.00000 | +2556 ± 69 | +2556 ± 69 | 0.00/0.00 | 1.00 |
| f2_vector_hnsw | clean_concise_long | +60% (332→132 t) | +0% (7→7 t) | +0.00060 ± 0.00000 | +3891 ± 257 | +3891 ± 257 | 0.00/0.00 | 1.00 |
| f2_rate_limit | clean_concise_long | +60% (326→130 t) | +0% (7→7 t) | +0.00059 ± 0.00000 | +3289 ± 140 | +3289 ± 140 | 0.00/0.00 | 1.00 |

## 4. Qualite (honnete)

- **rubric b/c** : fraction des concepts attendus (`expect`) presents dans la reponse baseline (b) puis chewed (c). **Critere de garde : rubric_c ≥ rubric_b** (la brievete ne doit pas perdre de concept). `—` = prompt sans `expect` (le juge prend le relais).
- **juge LLM** : DESACTIVE (`--judge` off). Pour le verdict rigoureux « correct mais concis », relancer avec `--judge` (ajoute ~1 appel/prompt).
- **style (jaccard)** : recouvrement de mots baseline↔chewed. **RETROGRADE** → mesure une **divergence de STYLE**, PAS la qualite. Un jaccard bas (~0,25) = reponse plus concise, pas une erreur (cf. verdict H0.1).

## 5. Regle de routage (LIVRABLE — entree d'une future politique de routeur)

| Famille | chewer ? | justification (mesuree) |
|---|---|---|
| F2 propre/concis long | **NE PAS chewer** | cout ~0 · plus lent (Δcout +0.00038 $, net_lat +2577 ms) |

**Logique de routage** : sous le gate (F3) → BYPASS (compression NO-OP, overhead analyzer pur) ; redondant/verbeux (F1) → CHEWER (gain coût+latence via reduction de sortie) ; propre/concis (F2) → NE PAS chewer (overhead SLM non couvert) ; long technique (F4) → CONDITIONNEL (chewer si Q&A, pas si exhaustivite requise — arbitrer avec la colonne juge).

## Comment lire

- **Δcout > 0** → le pre-machage **economise** (cout SLM local ≈ nul). **part sortie** ≈ fraction du gain venant de la reduction de SORTIE.
- **latence NETTE = (SLM + LLM_chewed) − LLM_baseline**. **< 0** → chewed PLUS RAPIDE bout-en-bout.
- **rollup par famille** = moyenne ± σ des moyennes par-prompt de la famille.

## Caveats methodologiques

- **Tokens d'entree estimes** (`chars/4`, via `PRE_CHEWING_METRICS`), pas le compte exact du tokenizer provider ; `cost_cents` du flux sert de garde-fou.
- **Temperature non reglable via le DTO** → variance geree par les repetitions (moyenne ± σ), pas par `temperature=0`.
- **Famille redundant_verbose = best-case** (padding) → savings optimistes ; les familles propre/court/long bornent le realisme.
- **Cout compute SLM non monetise** (self-host local ≈ nul). Si SLM facture, soustraire son cout du Δcout.
- **Juge LLM** (si ON) : un evaluateur est lui-meme bruite ; il sert d'**aide a la decision** sur F4, pas de verite absolue.

## Decision (a completer apres lecture)

> Phrase-titre attendue : « Le pre-machage se route par FAMILLE : CHEWER les prompts redondants (−X% cout, −Y ms), BYPASSER les courts, NE PAS chewer les propres, et CONDITIONNER les longs techniques a [Q&A / exhaustivite]. »
