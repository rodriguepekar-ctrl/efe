# ADR-SWARM-001 : H-SWARM — l'essaim bon marché contient la réponse, l'AGRÉGATEUR est le pari

- **Status** : Accepted · **Date** : 2026-06-06
- **Phase** : 0 (Vérité & socle) · **Gate** : H-SWARM (Lot S1)
- **Related** : ADR-P6-001 (kNN > consensus tensoriel) ; R0 (routing = goulot) ; exemples `swarm_relay`/`swarm_slm` ; `scripts/swarm/swarm_eval.py`

## Contexte

La thèse fondatrice d'EFE — **« beaucoup de petits agents bon marché, en parallèle, battent un
gros modèle »** — n'avait jamais été testée. **Lot 1** avait tué la *méthode* (consensus
tensoriel ≈ kNN) ; **R0** avait montré que le *routage* est un goulot (kNN capte 28 % du prix
oracle). Aucun des deux n'avait tranché **H-SWARM** : un ESSAIM agrégé bat-il un MONOLITHE sur
le composite (qualité × coût) ?

Phase 0 exige le test **le moins cher** qui peut valider ou tuer la thèse avant d'investir le
flywheel (Phases 1-5). On réutilise la matrice RouterBench réelle (2499 prompts × 11 modèles,
qualité + coût par-prompt) et la méthodologie à **splits répétés ± σ** qui avait inversé Lot 1.

## Décision

1. **H-SWARM n'est ni un NO-GO ni un GO naïf : c'est un GO CONDITIONNEL.** La capacité brute de
   l'essaim bon marché **dépasse le monolithe à coût inférieur** — mais seulement si on sait
   **extraire** la bonne réponse parmi les candidats.
2. **Le pari est relocalisé sur l'AGRÉGATEUR** (sélecteur/vérificateur/jury), pas sur la capacité
   des modèles ni sur le substrat. C'est là qu'est la valeur ET le moat.
3. **La Phase 3 (qualité émergente / consensus) passe sur le chemin critique** ; le vote
   majoritaire naïf est insuffisant et NE doit PAS être le produit.
4. **Convergence avec la vision-noyau** : un agrégateur qui choisit parmi K candidats EST un
   problème de **passage d'infos inter-agent** (candidats + critiques → convergence) et **exige
   la provenance** (quel agent, quel candidat, pourquoi retenu). Substrat + consensus + provenance
   ne font qu'un seul pari cohérent.

## Évidence (Lot S1 — `swarm_eval.py`, 2499 prompts, 50 splits, moy ± σ)

| politique | accuracy | coût/prompt | vs monolithe |
|---|---|---|---|
| mono_cheap (1 modèle) | 0,291 ± 0,009 | 4,5e-05 | plancher |
| **mono_best (monolithe à battre)** | **0,851 ± 0,008** | **0,003318** | référence |
| vote3 / vote5 / vote7 (vote majoritaire) | 0,43 / 0,41 / 0,43 | 2,5e-4 / 6,1e-4 / 1,0e-3 | **échoue** |
| vote9 | 0,541 ± 0,008 | 0,003458 | coût du premium, qualité bien pire |
| **or5 (plafond, OR des 5 moins chers)** | **0,847 ± 0,007** | **6,1e-04** | **≈ égalité à 5,4× moins cher** |
| **or7 (plafond, OR des 7 moins chers)** | **0,908 ± 0,005** | **1,0e-03** | **BAT le monolithe à 3,2× moins cher** |
| or9 | 0,940 ± 0,005 | 0,003458 | +9 pts au coût du premium |

**Robustesse** : structure identique à **θ ∈ {0,3 ; 0,5 ; 0,7}**. À θ=0,7 (notation plus dure)
l'avantage de l'essaim **grandit** (or7 0,905 vs monolithe 0,795 → +11 pts).

**Lecture chirurgicale** :
- **La réponse correcte EST présente** dans les 5-7 modèles les moins chers ~85-91 % du temps,
  à une fraction du coût (or5 égale le monolithe à **5,4×** moins cher ; or7 le **dépasse** à
  **3,2×** moins cher).
- **Le vote majoritaire ne l'extrait PAS** (0,43) : les petits modèles sont individuellement
  faibles et **corrélés** dans leurs erreurs → le bon est souvent minoritaire.
- **L'écart vote→OR (0,43 → 0,91) = la valeur qu'un bon agrégateur débloque.** C'est le prix de
  la Phase 3.

→ **Verdict : GO conditionnel — le pari est l'agrégateur, pas la capacité.**

## Justification

1. **Parallèle exact avec R0** : même forme « le prix est réel, l'extracteur est le goulot ».
   R0 : oracle 0,92 mais routeur capte 28 %. Ici : plafond 0,91 mais vote capte ~0. La leçon est
   cohérente : **la valeur EFE n'est pas dans la capacité brute (commodité) mais dans l'extraction
   vérifiable** (sélecteur/jury/débat).
2. **Le plafond OR est ATTEIGNABLE pour les tâches vérifiables** : code qui compile/passe les
   tests, maths à réponse vérifiable, extraction structurée validée par schéma, QA ancrée avec
   citations. EFE a déjà les briques (`efe-gold-test-gate`, `output_validator`, `efe-multi-llm`
   jury). Le pari n'est donc pas spéculatif.
3. **Le seuil à battre est BAS** : un agrégateur n'a pas besoin d'être parfait, juste meilleur que
   le vote (0,43). L'amplitude (0,43 → 0,91) laisse un énorme marge.
4. **Cheap-first respecté** : ~1 jour de calcul, 0 modèle entraîné, réutilise une matrice déjà
   collectée. On a évité de bâtir les topologies (Phase 2) avant de savoir où est le goulot.

## Conséquences

### Positives
- La thèse de l'essaim **survit** avec une cible précise et chiffrée (or7 = +6 pts à 3,2× moins cher).
- Le **chemin critique est clarifié** : Phase 3 (agrégateur) avant Phase 2 (topologies) à grande échelle.
- **Moat cohérent** : agrégateur vérifiable + provenance par-candidat = ce que les providers ne
  peuvent pas être (vérificateur indépendant), et ce qui **ne s'érode pas** avec le prix des tokens.

### Négatives
- **Le vote majoritaire (la baseline gratuite) est mort** : pas de produit « self-consistency » trivial.
- **Dépendance à la vérifiabilité** : le gain OR n'est récupérable que là où l'on peut scorer un
  candidat. Sur les tâches non-vérifiables, le plafond réel est plus bas (à mesurer).
- Le substrat parallèle (Phases 1-2), bien que réel et prouvé (test CAS), n'est **pas suffisant
  seul** : sans agrégateur, l'essaim ne bat pas le monolithe.

## Portée & Caveats (ce qui rouvrirait/affinerait la décision)
- **Forme la plus FAIBLE d'essaim** : ensemble indépendant, MÊME prompt, SANS passage d'infos
  inter-agent. Les topologies riches (décomposition, débat) pourraient faire mieux — ou pire.
- **OR suppose un vérificateur/sélecteur parfait** : c'est un PLAFOND, pas un résultat. Le vrai
  test (ci-dessous) doit mesurer un agrégateur RÉEL.
- **Tâches RouterBench** (QA/raisonnement gradés) ; **pas de latence** (RouterBench ne la fournit
  pas — en réel l'essaim s'exécute en parallèle, ≈ latence du plus lent des K, souvent < monolithe).
- **Corrélation des erreurs** : les petits modèles partagent des erreurs → borne le gain du vote ;
  un agrégateur doit exploiter la DIVERSITÉ, pas la majorité.

## Suite — agrégateur : a-priori MESURÉ insuffisant, spec a-posteriori v* ≥ 87 % (2026-06-06)

Lot S1 a dit « capacité oui, vote non ». Le test d'agrégateur (`swarm_aggregator_eval.py`,
30 splits) tranche **quelle famille d'agrégateur** :

| K | selector a-priori (kNN, **mesuré** ±σ) | rand | OR (plafond) | **v\* requis** | moins cher |
|---|---|---|---|---|---|
| 5 | 0,487 ± 0,027 | 0,437 | 0,848 | 101 % | ↓ |
| **7** | **0,485 ± 0,026** | 0,455 | **0,909** | **87 %** | **↓ 3,2×** |
| 9 | 0,484 ± 0,027 | 0,504 | 0,940 | 80 % | ≈ coût monolithe |

1. **L'agrégateur A-PRIORI (routage sur features) ÉCHOUE — mesuré.** Le sélecteur kNN appris
   atteint 0,485 ≈ `rand` 0,455 : il ne capte que **~7 %** du gisement (OR−rand). C'est le
   **pattern R0 confirmé** : router sur le prompt ne capte pas la valeur. → **on ne route pas.**
2. **L'agrégateur A-POSTERIORI a une cible chiffrée** (forme close `v* = (mono−rand)/(OR−rand)`) :
   **un vérificateur qui LIT les réponses et identifie un candidat correct avec précision ≥ 87 %
   sur K=7 candidats bat le monolithe (0,851) à 3,2× moins cher.** Plancher à battre (vote 0,43,
   a-priori 0,49) bas ; marge énorme.
3. **Mécanisme codé + démontré** (`swarm_verifier.rs`, sur le substrat réel) : le vérificateur
   a-posteriori interpole `verifier(v) ≈ v·OR + (1−v)·vote` ; à v=1,0 il atteint le plafond OR,
   le vote stagne au plancher (faute corrélée). Trait `CandidateVerifier` réutilisable
   (style gold-test-gate), backend-agnostique.

**Verdict raffiné : SPEC** — le pari n'est ni la capacité (suffisante) ni le routage (échoue),
mais un **vérificateur a-posteriori ≥ 87 % sur les réponses**. Atteignable par les briques EFE
sur tâches **vérifiables** (`efe-gold-test-gate`, `output_validator`) ; à **mesurer** par jury
`efe-multi-llm` sur tâches non-vérifiables.

### Reste à trancher (le dernier gate, le moins cher)
**Un vérificateur RÉEL (gold-test-gate / jury / débat) sur des RÉPONSES réelles atteint-il
v ≥ 87 % ?** RouterBench ne fournit pas le texte des réponses → ce test exige un **run modèles
réels** (`swarm_slm.rs` + agrégateur branché, backend llama-server). Si oui → GO ferme, c'est le
produit. Si non → l'agrégation est intrinsèquement dure ; repli sur « infra + provenance ».

## Références
- `scripts/swarm/swarm_eval.py` — harness H-SWARM (politiques mono/vote/OR, splits répétés)
- `scripts/swarm/swarm_aggregator_eval.py` — gate agrégateur (selector a-priori mesuré + spec v*)
- `scripts/swarm/EFE_SWARM_S1_2026-06-06.{md,json}` · `EFE_SWARM_AGG_2026-06-06.{md,json}` — résultats
- `crates/efe-blackboard/tests/swarm_cas_invariant.rs` — invariant de coordination gelé (gagnés+perdus==N×R)
- `crates/efe-blackboard/examples/{swarm_relay,swarm_slm,swarm_verifier}.rs` — substrat · essaim SLM · vérificateur a-posteriori
