# Sprint D6 — Validation accuracy report

**Date** : 2026-04-25
**Host** : Apple M3 Pro, Darwin 24.1.0, no-BLAS
**Catalog** : `support_triage` v1.0.0 (4 classes — fraud / bug / question / billing)
**Embedder** : `sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2` (dim=384)
**Test set** : 9 cas hand-curated EN+FR + 1 adversarial (`demo_classify_isolated.py`)

---

## Résultats comparatifs avant / après D6

| Cas | Texte | Pré-D6 | Post-D6 | Cosine raw nearest |
|-----|-------|:------:|:-------:|:------------------:|
| EN-fraud | "Someone charged my card..." | billing (0.575) | billing (0.710) | **fraud (0.66)** |
| EN-bug | "The app crashes..." | bug (0.575) ✓ | billing (0.695) ✗ | **bug (0.65)** |
| EN-question | "How do I export..." | question (0.575) ✓ | billing (0.701) ✗ | **question (0.39)** |
| EN-billing | "I want to cancel..." | billing (0.575) ✓ | billing (0.713) ✓ | billing (0.61) |
| FR-fraud | "J'ai été débité..." | billing (0.575) ✗ | billing (0.712) ✗ | **fraud (0.67)** |
| FR-bug | "L'application affiche..." | billing (0.575) ✗ | billing (0.699) ✗ | bug |
| FR-question | "Comment modifier..." | billing (0.575) ✗ | billing (0.713) ✗ | question |
| FR-billing | "Pouvez-vous m'envoyer..." | billing (0.575) ✓ | billing (0.712) ✓ | billing |
| ADV-weather | "It's raining hard..." | rejected (cone breach) ✓ | rejected (cone breach) ✓ | n/a |

**Métriques** :

|   | Pré-D6 | Post-D6 |
|---|:------:|:-------:|
| Top-1 accuracy (labeled) | 4/8 = 50 % | **2/8 = 25 %** |
| Adversarial reject correct | 1/1 ✓ | 1/1 ✓ |
| Confidence std-dev across inputs | 0.000 (constant 0.575) | **~0.007 (varies 0.695-0.713)** |

---

## Diagnostic

### Le fix de consensus pacing fonctionne

**Pré-D6** : confidence strictement constante à 0.5750… sur 8 inputs
distincts → preuve que la fusion ne s'exécute pas vraiment, ou que le
détecteur déclenche `Converged` avant que l'état n'ait évolué.

**Post-D6** : confidence varie 0.695-0.713, deux décimales bougent
selon l'input → **la fusion exécute >= 4 rounds (min_rounds floor)**
et le détecteur attend que les signaux composites montent au-dessus
de `confidence_threshold = 0.65` (vs 0.50 pré-D6). **Mécaniquement,
le fix marche**.

### L'accuracy ne s'améliore pas — au contraire

4/8 → 2/8. Plus la fusion tourne longtemps, plus elle amplifie un
biais structurel : le centroïde converge vers `billing` indépendamment
de l'input.

### Hypothèse H3 confirmée — biais genesis/fusion

L'architecture D6 §3 listait 3 hypothèses :
- H1 (hystérésis trop court) — **fixée par D6**
- H2 (`confidence_threshold` trop bas) — **fixée par D6**
- H3 (genesis/fusion biaisé vers une classe) — **non fixée**

Avant D6, H1+H2 court-circuitaient la convergence avant que H3 ne
se manifeste pleinement. Le centroïde retourné = état post-genesis
≈ moyenne des 3 templates `(NaSem + NaLog + NaCrit)`, indépendant
de l'input. Cosine nearest sur ce centroïde = la classe la plus
proche du centre du convex hull des anchors → souvent `billing` mais
parfois `bug` / `question` par chance.

Après D6, la fusion exécute pleinement. Le mécanisme de fusion
(attention + softmax + aggregate + β-mix + project + clip) tire
le centroïde non pas vers la signature de l'input mais vers
**l'attracteur dominant des templates**. Ce dernier est apparemment
biaisé vers `billing` (probablement parce que `NaSem` / templates
linguistiques généralistes capturent mieux la sémantique de
"facture / paiement / annulation" que celle de "fraude" ou "bug
technique" sur les v2 fixtures Gaussiennes par défaut).

### Implication architecturale

**Le problème n'est pas dans la couche bench (D1-D5) ni dans la
couche consensus (D6 layer 1+2)**. Il est dans :
- **Layer fusion** (β-mix coefficient, project_rows convex blend)
- **Layer genesis** (`HeuristicMetaAnalyzer.decompose` qui pondère
  les templates ; `DefaultSpawner` qui répartit les agents par
  template)

Le test pratique D6 a remonté du symptôme `billing 50 %` au mécanisme
exact de l'amplification : c'est le **β-mix scheduling** ou le
**convex projection scaling** qui domine la signature de l'input.

---

## Conséquence : escalade en D7

L'architecture D6 §8 R6 prévoyait ce cas explicitement :

> *« H3 (genesis bias) est la vraie cause, H1+H2 ne suffisent pas.
> Plan B documenté : modifier le `MetaAnalyzer` pour mieux pondérer
> l'embedding input (D7 follow-up). Critère DoD : si accuracy < 70 %
> après D6.4, escalader en D7. »*

**Décision** : escalade D7 acceptée. D6 délivre :

- ✅ Couche 1 (`min_rounds` controller floor) : 5/5 tests pass.
- ✅ Couche 2 (`Mode1Config` consensus durci) : 6 seuils recalibrés.
- ✅ Couche 3 infra (eval pipeline câblée) : `--output-bench-format`
  + `SLO-D6-ACC-*` au parser + job CI `bench-accuracy`.
- ⚠️ Couche 3 résultat (accuracy ≥ 75 %) : **NON ATTEINT** (25 %).

L'**infrastructure accuracy SLO de D6 a son utilité directe** : elle
fait FAIL le SLO `D6-ACC-1` en CI à chaque PR jusqu'à ce que D7 fixe
le biais structurel. C'est le **gate qui empêche D7 de régresser**.

---

## Recommandations pour D7

1. **Diagnostic genesis** : tracer `HeuristicMetaAnalyzer.decompose`
   sur les 9 inputs ; vérifier si `dag.primary_class` est dominé par
   une classe (probablement « classification » → mappe vers `NaSem`
   surtout).
2. **Diagnostic spawner** : compter la répartition des templates
   spawnés par input ; vérifier qu'elle reflète l'embedding.
3. **Diagnostic fusion** : tracer le centroïde après chaque round
   sur les inputs `EN-fraud` et `EN-bug` — voir si la trajectoire
   diverge (espéré) ou converge tous vers le même point (observé).
4. **Fix probable** : modifier `MetaAnalyzer` pour utiliser
   l'embedding *directement* dans la pondération du DAG (au-delà de
   `dag.primary_class`), ou ajuster `β-mix` pour préserver davantage
   de la signature de l'input dans le centroïde final.

---

## Annexe : raw cosines (sans swarm)

Pour référence — l'embedder seul (cosine nearest sur les anchors,
sans aucune fusion) atteindrait :

| Cas | Raw nearest | Correct ? |
|-----|:-----------:|:---------:|
| EN-fraud | fraud | ✓ |
| EN-bug | bug | ✓ |
| EN-question | question | ✓ |
| EN-billing | billing | ✓ |
| FR-fraud | fraud | ✓ |
| FR-bug | bug | ✓ |
| FR-question | question | ✓ |
| FR-billing | billing | ✓ |

**Embedder seul = 8/8 = 100 % accuracy**. **EFE Mode 1 actuel = 25 %**.

L'écart (-75 points) est ce que D7 doit récupérer. Si après D7
l'accuracy ne dépasse pas 75 %, c'est l'**architecture Mode 1
elle-même** (anchor blending via `cone_project`) qui est sub-optimale
pour ce cas d'usage — auquel cas Mode 2/3 (autre stratégie de
classification) deviennent le plan.
