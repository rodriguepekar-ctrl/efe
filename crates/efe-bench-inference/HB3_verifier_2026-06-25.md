# HB.3 — le vérificateur arithmétique déployable : construit, relu, mesuré… et il ne paie pas (2026-06-25)

Suite directe de HB.2. Le régime (b) disait : « la bonne réponse est là (pass@K=100 %) mais minoritaire —
**un vérificateur paierait** ». HB.3 a construit le vérificateur **le plus défendable** (re-exécution de
l'arithmétique écrite par le candidat → poids de vote → garde de non-infériorité), l'a passé au panel de
conception adversarial puis à la revue, et l'a **mesuré** sur la même cellule (`phi4-mini × T-MATH`, CoT
réel, référence du gros figée d'avance). Verdict honnête : **sur cette cellule, ce vérificateur ne paie pas.**

## La mesure (n=50, R0 50/50, scellée, constantes PRÉ-ENREGISTRÉES non touchées)

| K | SC | pond | vérif | **RXW** | méd | pass@K | couv% | override% | out/moy |
|---|----|------|-------|---------|-----|--------|-------|-----------|---------|
| 1 | 94.0 | 94.0 | 94.0 | 94.0 | 94.0 | 94.0 | 38 | 0 | 110 |
| 3 | 92.0 | 92.0 | 92.0 | **90.0** | **96.0** | 100.0 | 38 | 10 | 341 |
| 5 | 94.0 | 94.0 | 94.0 | **92.0** | 92.0 | 100.0 | 44 | 8 | 652 |

référence gros (gemma3:12b, K=1, CoT) : **100.0 %**. genou ≈ K=3. Tous les mécanismes → **indéterminé**
(non-infériorité), sup-K [RXW] borne inf **−7 pts → ∃K non-inférieur : false** (identique à SC).

## Lecture honnête

1. **RXW ne récupère PAS le headroom — il est légèrement SOUS la self-consistency** (92 % vs 94 % à K=5 ;
   90 % vs 92 % à K=3). Le mécanisme **fonctionne** (il dévie de SC sur 8–10 % des items, l'override est
   réel), mais ces déviations sont **net-négatives** : il choisit une réponse fausse-mais-proprement-auditée
   aussi souvent qu'une bonne-mais-minoritaire.
2. **Pourquoi — l'angle mort de la contrainte V, désormais MESURÉ.** La re-exécution valide le **calcul**,
   jamais la **modélisation**. Sur les ~6 % d'items durs de ce corpus, l'erreur dominante est le **montage**
   (le modèle additionne au lieu de soustraire, puis calcule juste). Son arithmétique **tient** → RXW le
   classe haut → mal sélectionné. Le signal d'audit ne sépare donc PAS le correct-minoritaire du
   confiant-faux-propre. C'est exactement la limite annoncée dans l'archi (§5, §8) — et elle se réalise.
3. **Le garde a tenu son rôle (mais pas plus).** Il garantissait « jamais pire que SC **sur les items à
   faible signal** » ; sur les items où l'override se déclenche (marge franchie), RXW prend un pari — qui, à
   n=8, gagnait (87,5 % vs 75 %, un artefact de petit échantillon), et à n=50, perd de peu. Le garde évite
   la régression catastrophique (92 %, pas 50 %), il ne fabrique pas un gain.
4. **L'honnêteté la plus dure : on n'a PAS retouché les constantes.** Un `MARGIN` plus conservateur
   ramènerait RXW vers SC (94 %) — mais le régler sur le jeu scellé n=50 serait `pass@K` fuyant par
   l'expérimentateur (le garde-fou de pré-enregistrement, archi §5). `MARGIN=2` donne 92 % : **c'est le
   chiffre honnête, on le rapporte tel quel.**
5. **La surprise — la SYNTHÈSE, pas la sélection.** La **médiane numérique** atteint **96 % à K=3** — le
   meilleur chiffre déployable de toute l'expérience, le plus proche du gros (100 %). Sur l'arithmétique,
   **moyenner** les réponses peut tomber juste là où aucune n'est majoritaire-correcte (une synthèse, qui
   peut dépasser une sélection). Instable cependant (92 % à K=5) : suggestif, pas concluant.

## Le verdict, pour la thèse

Sur la cellule `phi4-mini × T-MATH`, **l'écart de 6 points au gros n'est comblé par AUCUN mécanisme
déployable testé** — y compris le vérificateur arithmétique purpose-built. Le « un vérificateur paierait »
du régime (b) est, **pour le vérificateur le plus défendable, réfuté sur cette cellule** : la bonne réponse
est présente (pass@K=100 %) mais **non séparable** par re-exécution. Le diagnostic se raffine d'un cran :
l'écart résiduel est un **écart de modélisation**, pas de calcul — l'audit du calcul ne peut donc pas le
fermer, par construction. La piste qui reste ouverte est la **synthèse** (la médiane frôle le gros), pas la
vérification ; et la domination de déploiement (GPU-s) reste, comme toujours, suspendue à B2.

## Ce que HB.3 vaut quand même

- **L'instrument a fait son travail le plus précieux : il a dit non.** Plutôt qu'un faux « le vérificateur
  paie » (ce qu'aurait suggéré le smoke n=8), il a produit, re-prouvablement et à constantes gelées, le
  verdict honnête « ce vérificateur ne paie pas, et voici pourquoi (l'erreur est dans le montage, pas le
  calcul) ». C'est la valeur d'EFE : le diagnostic avant la conclusion, même quand la conclusion déçoit.
- **La revue adversariale a tenu** : 13 trouvailles confirmées (panic NaN, cluster négatifs/moins-unaire
  dont un faux-MISS introduit par mon propre correctif), toutes corrigées avant la mesure. 98 tests, clippy
  propre. La mesure tourne donc sur du code relu.
- **Un nouveau levier identifié et chiffré** : la médiane-synthèse (96 % @K3) est la prochaine brique
  naturelle si l'on veut fermer cette cellule — un agrégateur, pas un vérificateur.
