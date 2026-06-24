# HB.2 — la mesure de l'agrégation : le confondant attrapé, levé, et le vrai verdict (2026-06-24)

Ce document raconte l'arc complet de HB.2 sur de la vraie inférence : phi4-mini (petit, ~3.8 B,
K≤5 en chaîne-de-pensée) contre gemma3:12b (gros, K=1, même affordance), corpus T-MATH n=50, via le
driver `examples/hb_aggregation.rs`. Les K candidats sont captés **par rôle générateur** (I-HB-GENONLY,
un `SwarmObserver` — jamais des juges LLM, qui en mode vérifié contamineraient le décompte), chaque run
est scellé puis **R0 re-prouvé 50/50**. Trois temps : (1) un premier passage qui a attrapé son propre
confondant au lieu de cracher un faux verdict, (2) le correctif SDK qui l'a levé, (3) la première mesure
d'un **vrai** raisonnement agrégé — et le verdict pivot qu'elle a rendu.

## Temps 1 — le confondant DÉCISIF que l'instrument a attrapé

Premier passage. La courbe SC était **plate** (36 % → 32 % en montant K), la médiane à 0 %, gemma à
34 %, le verdict « indéterminé », sup-K (max-t) `∃K non-inférieur = false`. Pris au premier degré, on
aurait écrit « la thèse échoue : agréger n'apporte rien ». **C'eût été faux**, et l'instrument l'a vu.

Le signal qui a trahi le confondant : **~11 jetons de sortie moyens**. Aucun raisonnement ne tient en
11 jetons. La cause : le SDK plafonnait la génération à **`max_tokens = 16` codé en dur** (`llm.rs`,
`ask(.., 16, ..)`) *et* suffixait le prompt de « Answer with only the result, no preamble. ». Le
« step by step » de l'instruction était **tronqué à la racine, deux fois**. Ce run ne mesurait donc pas
le raisonnement agrégé mais l'arithmétique-en-un-coup (le régime de HA.3) — exactement la condition que
H-B devait *rouvrir*. Preuve indépendante : gemma3:12b lui-même restait à 34 % (≈ son score sans CoT),
alors que la sonde HA.3 l'avait vu calculer juste « step by step » quand il en avait la place (142 vs
122). La courbe plate, la médiane nulle, l'indéterminé : **tous ATTENDUS sous un plafond de 16 jetons**,
et **sans aucune information sur la vraie thèse**. Ce n'était pas un échec de la thèse, mais un plafond
du SDK qui défaisait sa prémisse — et le diagnostic, pas le verdict, est sorti. C'est la valeur d'EFE.

## Temps 2 — le correctif (l'unblocker de HB.3)

Changement SDK **ciblé, par défaut inchangé** (B0/B1/HA reposent sur des générateurs scriptés/echo qui
ignorent `max_tokens` ; octet-pour-octet identiques) :

- `llm.rs` : `LlmGen` porte désormais un `max_tokens`, et au-delà d'un seuil (`COT_TOKEN_THRESHOLD =
  96`) bascule sur `gen_prompt_cot` — **sans** le suffixe « only the result » : on demande le
  raisonnement *puis* la réponse finale, au lieu de le supprimer. Le `ask(.., 16, ..)` codé en dur
  devient `ask(.., self.max_tokens, ..)`.
- `resolver.rs` / `sdk.rs` : le budget se propage du builder (`Swarm::builder().max_gen_tokens(n)`)
  jusqu'aux générateurs et aux deux camps du débat. **Défaut conservé à 16** (rien d'existant ne bouge).

Validation du déblocage : sortie **8.8 → 86.5 jetons**, gemma sur le même corpus **34 % → 100 %**. Le
raisonnement a bien lieu, symétriquement petit/gros (I-HB-COT, I-HB-FAIR).

## Temps 3 — la première mesure du vrai raisonnement agrégé (`--cot-tokens 512`, n=50)

| K | SC | pondéré | vérifié | médiane | pass@K | div% | dégén% | out/moy |
|---|------|---------|---------|---------|--------|------|--------|---------|
| 1 | 94.0% | 94.0% | 94.0% | (bug→fix) | 94.0% | 100% | 0% | 110.3 |
| 3 | 92.0% | 92.0% | 92.0% | (bug→fix) | **100.0%** | 45% | 0% | 340.0 |
| 5 | 94.0% | 94.0% | 94.0% | (bug→fix) | **100.0%** | 47% | 0% | 651.0 |

référence gros (gemma3:12b, K=1, CoT) : **100.0 %** · R0 50/50 · petit : R0 50/50 (mesure scellée,
re-prouvable). genou (τ=2 pts) ≈ K=1.

**Lecture — régime (b) du tableau §7, « un vérificateur paierait ».** Trois faits se combinent :

1. **Le raisonnement rapproche radicalement la substitution.** phi4-mini 3.8 B passe de 40 % (sans CoT)
   à **94 %**, à ~6 points de gemma. Le petit *raisonne presque aussi bien* — la prémisse H-A tient
   beaucoup mieux qu'au K=1 brut.
2. **La self-consistency naïve n'ajoute rien** (plate à 94 %, vote pondéré et vérifié idem). La pure
   « quantité par le vote » ne comble pas l'écart : la majorité se trompe sur les mêmes items durs.
3. **Mais `pass@K` atteint 100 %** dès K=3. La bonne réponse **est présente** dans les K candidats sur
   ces ~6 % d'items durs — simplement **minoritaire**, donc invisible au vote. Un **sélecteur**
   (vérificateur déployable) qui la repère rejoindrait gemma, à coût-jetons moindre (le petit domine le
   coût-jetons partout, cache OFF/ON).

Le verdict de non-infériorité formel reste **indéterminé** (n=50 sous-puissance) et sup-K (max-t)
`∃K = false` — honnête : à ce n, on ne *prouve* pas l'égalité. Mais la structure est nette et chiffrée :
la thèse *naïve* (voter suffit) est réfutée, et H-B a fait son travail le plus précieux — **chiffrer la
voie** : il manque un vérificateur, pas du raisonnement.

## Le bug que la mesure corrigée a, à son tour, attrapé

La colonne médiane sortait à **0 %**. Cause : `parse_num` ne pelait pas l'enveloppe `{value: <texte>}`
des candidats réels du générateur (le test d'origine passait des nombres nus). **Corrigé** — on pèle
`value`/`text` avant extraction numérique — **+ test de régression** (`median_peels_value_wrapper_…`).
La médiane n'excède donc pas réellement pass@K ici ; sur re-run corrigé elle lirait ≈ SC (~94 %), pas
0 %. Même schéma qu'au Temps 1 : **c'est la mesure qui a trouvé le bug**, pas l'inverse.

## Ce qui est acquis, et la prochaine brique

- **L'instrument HB.2 est validé de bout en bout** : capture par rôle (diversité 100 % → 47 % = K
  candidats réellement distincts), courbe / verdict@K* / sup-K (FWER max-t) / coût-jetons / re-preuve
  R0 50/50, tous opérationnels sur du vrai CoT.
- **L'instrument a attrapé deux confondants** (le plafond de 16 jetons, puis le bug médiane) plutôt que
  d'émettre un faux verdict. À chaque fois, le diagnostic est sorti avant la conclusion.
- **Verdict pivot, régime (b)** : phi4×math, raisonnement réel — la bonne réponse est là mais
  minoritaire ; un **vérificateur arithmétique déployable** (plug-back, gratuit en maths) la
  sélectionnerait. C'est la cible **chiffrée** de la suite (HB.3) : mesurer si phi4-vérifié atteint
  gemma à coût-jetons moindre. La domination de **déploiement** (GPU-secondes) reste suspendue à B2.
