# MZ — la mesure du Cadrage

> compresseur : `http://127.0.0.1:8081`  
> température : `0.0 (défaut, greedy)`  
> 29 mesures, 7 échec(s)

## Verdict

```
TUÉ — le Cadrage ne peut pas être employé tel quel :
  · 29 mesure(s) où le compresseur DÉCLARE préserver une entité absente du résumé — `validate_preservation` compare à la liste déclarée, jamais au texte : elle ne peut pas voir cela, et le pipeline scellerait la garantie
  · préservation vérifiée 10.5 % au ratio revendiqué 8.9:1 (exigé 99 %) — dans un produit de preuve, une entité perdue est une falsification scellée
```

## La courbe d'arbitrage

| cible | ratio médian | préservation **déclarée** | préservation **vérifiée** | écart | dégénérescence | menteurs |
|---|---|---|---|---|---|---|
| 20 mots | 8.9:1 | 100.0 % | **10.5 %** | +89.5 pp | 0.0 % | 5 |
| 30 mots | 6.2:1 | 100.0 % | **13.9 %** | +86.1 pp | 0.0 % | 5 |
| 50 mots | 3.2:1 | 100.0 % | **9.1 %** | +90.9 pp | 0.0 % | 5 |
| 80 mots | 2.6:1 | 100.0 % | **28.2 %** | +71.8 pp | 0.0 % | 4 |
| 120 mots | 2.6:1 | 100.0 % | **22.6 %** | +77.4 pp | 0.0 % | 5 |
| 200 mots | 2.6:1 | 100.0 % | **22.6 %** | +77.4 pp | 0.0 % | 5 |

## Ce qui a été perdu, nommé

| entité | perdue N fois |
|---|---|
| `Vertex Audit Partners` | 9 |
| `1 500 000 EUR` | 6 |
| `2024-06-30` | 6 |
| `2025-01-06` | 6 |
| `2025-01-15` | 6 |
| `2026-01-06` | 6 |
| `87 400 EUR` | 6 |
| `ACME Metallurgie SA` | 6 |
| `AV-2024-0311` | 6 |
| `OP-3391` | 6 |
| `SUP-2024-0417` | 6 |
| `2024-07-04` | 5 |
| `2025` | 5 |
| `2025-03-11` | 5 |
| `46 000 EUR` | 5 |
| `clause 12.1` | 5 |
| `clause 7.3` | 5 |
| `2024-11-19` | 4 |
| `2024-12-20` | 4 |
| `AUD-2024-0781` | 4 |

## Déterminisme

2/2 paires identiques. La campagne H1.1 avait mesuré une compression **non déterministe** à `temperature 0.2` (≈ 33 % de no-op) ; le passage au greedy (0.0) était censé la supprimer.

## Ce que cette mesure ne dit pas

- La **qualité de la réponse finale**. MZ mesure le compresseur, pas le pipeline.
- La **correction**. Ce qui est mesuré est ce qui a été *préservé*, jamais ce qui est *vrai*.
- Les **autres robots**. Analyseur, vérifieur et reformulateur ont leurs propres plafonds.

Et un fait qui encadre tout le reste : les deux sites de production du pipeline passent `must_preserve: vec![]`. Cette mesure demande la préservation ; **le service, jamais**.

