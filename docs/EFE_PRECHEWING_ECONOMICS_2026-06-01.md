# EFE — Économie réelle du pré-mâchage (vrai LLM) · claude_claude-sonnet-4-6

> Roadmap **H0.1**. A/B contrôlé (baseline brut vs chewed compressé) sur modèle épinglé, coût **calculé** depuis tokens × pricing (in $3.0/Mtok · out $15.0/Mtok). Généré : 2026-06-01T14:40:45.963694+00:00. Données brutes : le `.json` à côté.

## Verdict par prompt

| Prompt | type | savings tokens | Δcoût (USD) | latence NETTE (ms) | overhead SLM (ms) | qualité (jaccard) | factuel base/chewed |
|---|---|---|---|---|---|---|---|
| fr_capital_1k | redundant | +16% (302→254 t) | +0.00017 ± 0.00024 | +2809 ± 817 | +2965 ± 653 | 0.72 ± 0.39 | 1.00/1.00 |
| syncasync_1k5 | redundant | +51% (430→212 t) | +0.00830 ± 0.00124 | -6846 ± 1566 | +2466 ± 42 | 0.25 ± 0.02 | 0.00/0.00 |
| rest_graphql_2k5 | redundant | +64% (658→234 t) | +0.01059 ± 0.00041 | -5004 ± 1209 | +7215 ± 657 | 0.24 ± 0.01 | 0.00/0.00 |
| varied_backend | varied | +16% (383→321 t) | +0.00122 ± 0.00133 | +6166 ± 1990 | +4953 ± 1201 | 0.26 ± 0.04 | 0.00/0.00 |

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

## Décision (H0.1 — VERDICT, post-R7 + post-campagne L1/C-B)

> **Le pré-mâchage paie sur le COÛT dès ~430 t d'entrée redondante (−51 à −64 % tokens IN,
> +$0.008–0.011/req) ; sur la LATENCE OUI sur les prompts redondants (chewed PLUS RAPIDE de
> 5–8 s end-to-end), neutre-à-négatif sur les prompts courts/propres ; qualité PRÉSERVÉE SOUS
> CONDITION (réponse correcte mais plus concise).**

**La vraie mécanique (révélée par la lecture des réponses + la décomposition du coût).** Le gain
vient surtout de la **réduction de SORTIE**, pas de l'entrée compressée. Un prompt propre/condensé
induit une réponse **correcte mais concise** (~2,5–2,8× moins de tokens de sortie). Sur
`rest_graphql`, **88 % de l'économie** vient de la sortie (1000→362 t × $15/Mtok) vs 12 % de
l'entrée (658→234 t × $3/Mtok). La sortie coûte 5× l'entrée et domine le temps → c'est là que
vivent le coût ET la latence.

**Lecture des réponses (la vraie preuve, pas le jaccard).** Les réponses chewed sont **sur le bon
sujet et correctes** (sync/async expliqué via code ; REST/GraphQL via endpoints/over-fetching),
simplement plus **courtes** que les baselines verbeuses (qui mirooirent la redondance du prompt
brut). Le jaccard bas (~0,25) = **style différent**, pas une erreur. Ce n'est PAS l'artefact pré-R7
(réponses fausses & courtes) : ici les réponses sont **justes** & courtes.

**Positionnement (RÉVISE H0.3).** Sur les prompts **verbeux/redondants**, le pré-mâchage est un
optimiseur de **COÛT + LATENCE** (via induction de brièveté correcte) ; sur les prompts déjà propres,
il reste **neutre** (l'overhead SLM amont n'est pas couvert). La brièveté est un **atout pour du
Q&A**, un **compromis si l'exhaustivité est requise** (décision produit).

⚠️ Rappel : ces prompts incluent des `redundant` (best-case compression → savings **optimistes**) ;
les `varied` bornent le réalisme (cf. H1.1 — élargir le corpus).
