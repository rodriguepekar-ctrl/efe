# HB.4 — la synthèse : construite, relue, mesurée… elle ne paie pas non plus, et l'instrument dit pourquoi (2026-06-25)

Suite de HB.3 (le vérificateur de re-exécution ne paie pas ; l'écart résiduel est un écart de **modélisation**).
La seule lueur était la médiane à 96 % @K3 — mais à K **impair** la médiane *sélectionne* (un candidat),
elle ne *synthétise* pas. HB.4 a construit l'instrument honnête de la **synthèse** (fabriquer une valeur
absente des K), l'a passé au panel de méthodologie puis à la revue, et l'a mesuré aux K **pairs** (où la
synthèse a lieu). Verdict, constantes pré-enregistrées non touchées : **sur cette cellule, la synthèse ne
paie pas non plus — et le diagnostic explique pourquoi.**

## La mesure (n=20, R0 20/20, scellée ; puissance corrigée après revue)

| K | SC | RXW | pass@K │ méd | tronq | AWM | env │ fab% │ var | div% | ent |
|---|----|-----|--------|-----|-------|-----|-----|------|-----|------|-----|
| 1 | 85 | 85 | 85 │ 85 | 85 | 85 | 85 │ 0 │ 0 | 100 | 1.00 |
| 2 | 80 | 90 | 100 │ 80 | 80 | 90 | 100 │ 20 │ 1 252 | 60 | 0.86 |
| 3 | 90 | 85 | 100 │ 90 | 75 | 85 | 100 │ 0 │ 1 305 | 43 | 0.87 |
| 4 | 90 | 70 | 100 │ 85 | 85 | **70** | 100 │ 15 │ **1.2e16** | 46 | 0.89 |

référence gros (K=1) : **100 %**. genou ≈ K=2. Verdict @K\*=4 : SC indéterminé (90 %, puissance 1 %),
méd/tronq **indéterminé** (85 %, puissance ~0 %), AWM **inférieur** (70 %). sup-K parité-pure : ∃K
non-inférieur = **false** partout (sélection sur {1,3}, synthèse sur {2,4}).

## Lecture honnête

1. **Aucun mécanisme de synthèse n'atteint le gros.** La médiane et la moyenne tronquée plafonnent à
   **85 %** (= la sélection, **aucun gain de synthèse**) ; la fusion AWM est **inférieure** (70 %). Loin du
   gros (100 %), et la synthèse ne dépasse même pas la self-consistency.
2. **Pourquoi — le diagnostic, la vraie valeur de HB.4.** Le test de symétrie (label-using, cloisonné) :
   **asymétrie +8.72** (≫ 0,5 ; +6,12 à n=8 → +8,72 à n=20, le signal se renforce), biais signé +3,65. Les
   erreurs du petit sont **massivement systématiques et asymétriques**, PAS dispersées-symétriques — la
   condition même de réussite de la synthèse est violée. Moyenner/médianer **suit la mauvaise distribution**.
3. **Des outliers catastrophiques.** La variance à K=4 atteint **1,2 × 10¹⁶** : un candidat a émis un nombre
   astronomique. Un seul tel outlier gonfle la moyenne ET l'enveloppe ; la moyenne tronquée (TRIM_FRAC=0,25,
   **gelé, non retouché**) ne coupe pas assez (elle dégrade : 85 % @K2 → 75 % @K3). Et le plafond `env`=100 %
   est **lâche** (enveloppe gonflée par l'outlier) : « le label est entre min et max » sans que la synthèse
   le trouve.
4. **La fusion AWM HÉRITE de la limite de HB.3 — et l'aggrave ici** : à 70 %, elle est *sous* tout le reste.
   Les poids d'audit la tirent vers des candidats au bon calcul / mauvais montage, et les outliers déplacent
   la médiane pondérée. Confirmé : **le signal d'audit ne soigne pas l'erreur de modélisation**, fusionné ou non.
5. **La puissance, après correction.** La revue a trouvé que la reconstruction (p_b, p_c) pour
   `achieved_power` était **inversée de signe** — elle gonflait la puissance (62 % affiché à n=8). Corrigée,
   la puissance réelle est **~0–1 %** : à n=20 on ne peut formellement rien conclure (et de fait le petit
   *est* inférieur de &gt; δ, donc « pas de puissance pour déclarer non-inférieur » est le comportement JUSTE).
   Le diagnostic structurel (asymétrie, outliers), lui, est robuste et prédit que la synthèse ne paiera pas
   davantage à plus grand n.

## Le verdict, pour la thèse

L'arc se referme, honnêtement : **régime (b)** (« un vérificateur paierait ») → **HB.3** (le vérificateur ne
paie pas) → **HB.4** (la synthèse ne paie pas non plus, parce que les erreurs sont **systématiques**, pas
symétriques). Sur `phi4-mini × T-MATH`, **aucune agrégation déployable** — sélection (SC/pondéré/vérifié),
vérification (RXW), ni synthèse (médiane/tronquée/pondérée) — ne ferme l'écart de 6–15 points au gros. La
bonne réponse est *présente* (pass@K=100 %) mais **non récupérable** par agrégation : l'écart est un **écart
de modélisation**, irréductible par re-pondération, vote, ou moyenne. C'est un **négatif fort** qui **borne**
la thèse sur l'arithmétique — et la crédibilise (on ne vend pas ce qu'on n'a pas mesuré). La domination de
déploiement (GPU-s) reste suspendue à B2.

## Ce que HB.4 vaut

- **L'instrument a, encore, dit non — proprement.** Plutôt qu'un faux « la médiane à 96 % gagne » (le piège
  du ∃K à K impair sur un n sous-puissant), il a distingué sélection et synthèse, mesuré aux K pairs, rapporté
  le **taux de fabrication** (anti-mensonge), le **plafond d'enveloppe**, la **dispersion** et la **symétrie**,
  sous **multiplicité contrôlée** (K\* pré-spécifié, sup-K parité-pure) et **puissance** honnête. Le diagnostic
  de symétrie (asym +8,72) est la **cause-racine mesurée** : les erreurs ne sont pas du bruit moyennable.
- **Revue adversariale** (11 agents) : **1 bug** confirmé+corrigé (puissance inversée), tout le reste vérifié
  correct (plafonds parité, anti-mensonge, cloison label, no-arrondi, un-noyau, déterminisme, mécanismes).
  **109 tests, clippy propre.** Mesure sur code relu.
- **Caveat de puissance** : n=20 (foreground, pour survivre à l'environnement qui tue les jobs longs) au lieu
  du n=50 standard — mais le diagnostic structurel est cohérent de n=8 à n=20 et se **renforce**. Le verdict
  formel est « indéterminé/inférieur, puissance ~0 % » ; le verdict structurel est « non, et voici pourquoi ».
