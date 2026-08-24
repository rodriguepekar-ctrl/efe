# MZ — la mesure du Cadrage

> compresseur : `http://127.0.0.1:8081`  
> température : `0`  
> délai par appel : `60 s` (défaut client : 5 s)  
> 65 mesures, 7 échec(s)

## Verdict

```
TUÉ — le Cadrage ne peut pas être employé tel quel :
  · 32 mesure(s) où la liste `preserved` porte une entité ABSENTE du résumé. Ce n'est pas une tromperie — la mesure des deux bras montre que le modèle recopie la demande — mais l'effet est le même et il est fatal : `validate_preservation` ne lit que cette liste, jamais le texte, donc elle ne peut pas détecter la perte, et le pipeline scellerait la garantie
  · préservation vérifiée 10.5 % au ratio revendiqué 8.9:1 (exigé 99 %) — dans un produit de preuve, une entité perdue est une falsification scellée
```

Le verdict porte sur le bras **exigé** — là où la promesse du contrat est éprouvée. Juger le compresseur sur une configuration où on ne lui demande rien l'acquitterait par construction.

## Les deux bras

### Bras `exigé` — la promesse

`must_preserve` = les entités du corpus. Le filtre du client (`validate_preservation`) est **actif** : il rejette l'appel quand la liste déclarée ne les couvre pas. Conséquence à garder en tête — ce bras ne peut enregistrer que des réponses où le modèle a **déclaré** tout préserver ; une omission avouée y devient une erreur, pas une mesure.

| cible | ratio médian | préservation **déclarée** | préservation **vérifiée** | écart | dégénérescence | déclarations infondées |
|---|---|---|---|---|---|---|
| 20 mots | 8.9:1 | 100.0 % | **10.5 %** | +89.5 pp | 0.0 % | 5 |
| 30 mots | 6.2:1 | 100.0 % | **13.9 %** | +86.1 pp | 0.0 % | 5 |
| 50 mots | 3.2:1 | 100.0 % | **9.1 %** | +90.9 pp | 0.0 % | 5 |
| 80 mots | 2.6:1 | 100.0 % | **22.6 %** | +77.4 pp | 0.0 % | 5 |
| 120 mots | 2.6:1 | 100.0 % | **18.8 %** | +81.2 pp | 0.0 % | 6 |
| 200 mots | 2.6:1 | 100.0 % | **18.8 %** | +81.2 pp | 0.0 % | 6 |

### Bras `servi` — ce que le pipeline obtient

`must_preserve` = `vec![]`, **la configuration réellement exécutée** : les deux sites de production passent une liste vide. Le filtre du client est alors une boucle sur rien, donc il n'écarte plus personne — et la vérification indépendante de MZ est le **seul** contrôle qui subsiste. C'est le chiffre à opposer à qui décrit le Cadrage tel qu'il tourne.

| cible | ratio médian | préservation **déclarée** | préservation **vérifiée** | écart | dégénérescence | déclarations infondées |
|---|---|---|---|---|---|---|
| 20 mots | 8.9:1 | 0.0 % | **10.5 %** | -10.5 pp | 0.0 % | 0 |
| 30 mots | 5.4:1 | 0.0 % | **13.9 %** | -13.9 pp | 0.0 % | 0 |
| 50 mots | 3.3:1 | 0.0 % | **19.1 %** | -19.1 pp | 0.0 % | 0 |
| 80 mots | 2.6:1 | 0.0 % | **18.8 %** | -18.8 pp | 0.0 % | 0 |
| 120 mots | 2.6:1 | 0.0 % | **18.8 %** | -18.8 pp | 0.0 % | 0 |
| 200 mots | 2.6:1 | 0.0 % | **18.8 %** | -18.8 pp | 0.0 % | 0 |


## L'effet de `must_preserve`, mesuré

Le contrat annonce « préservant des entités **spécifiées** ». Voici ce que le fait de les spécifier change réellement, à cible égale :

| cible | vérifié si l'on DEMANDE | vérifié si l'on ne demande PAS | effet |
|---|---|---|---|
| 20 mots | 10.5 % | 10.5 % | **+0.0 pp** |
| 30 mots | 13.9 % | 13.9 % | **+0.0 pp** |
| 50 mots | 9.1 % | 19.1 % | **-10.0 pp** |
| 80 mots | 22.6 % | 18.8 % | **+3.8 pp** |
| 120 mots | 18.8 % | 18.8 % | **+0.0 pp** |
| 200 mots | 18.8 % | 18.8 % | **+0.0 pp** |

Effet moyen de la demande : **-1.0 pp**.

Si cet effet est nul, la conclusion est plus dure que « le compresseur perd des entités » : le paramètre `must_preserve` est **inerte**. Le champ, l'instruction du prompt qui dit « keeping every entity from must_preserve », et la validation qui le relit — aucun des trois ne change ce qui sort. Et le modèle n'est pas malhonnête, il est **obéissant** : quand on lui demande X il déclare « j'ai gardé X », quand on ne demande rien il déclare 0 %. Le champ `preserved` **recopie la demande** ; il ne rapporte pas la réalité. C'est pourquoi `validate_preservation`, qui ne lit que lui, est structurellement incapable de détecter une perte.


## Ce qui a été perdu, nommé

| entité | perdue N fois |
|---|---|
| `Vertex Audit Partners` | 20 |
| `1 500 000 EUR` | 12 |
| `2024-06-30` | 12 |
| `2025-01-06` | 12 |
| `2025-01-15` | 12 |
| `2026-01-06` | 12 |
| `87 400 EUR` | 12 |
| `ACME Metallurgie SA` | 12 |
| `AV-2024-0311` | 12 |
| `OP-3391` | 12 |
| `SUP-2024-0417` | 12 |
| `2024-09-12` | 11 |
| `2024-10-01` | 11 |
| `2025-03-11` | 11 |
| `46 000 EUR` | 11 |
| `SCELLA-P3` | 11 |
| `clause 12.1` | 11 |
| `clause 7.3` | 11 |
| `2024-07-04` | 10 |
| `2025` | 10 |

## Déterminisme

5/5 paires identiques. La campagne H1.1 avait mesuré une compression **non déterministe** à `temperature 0.2` (≈ 33 % de no-op) ; le passage au greedy (0.0) était censé la supprimer. Rejouer avec `EFE_COMPRESSOR_TEMPERATURE=0.2` donne le bras de comparaison.

## Ce que cette mesure ne dit pas

- La **qualité de la réponse finale**. MZ mesure le compresseur, pas le pipeline.
- La **correction**. Ce qui est mesuré est ce qui a été *préservé*, jamais ce qui est *vrai*.
- Les **autres robots**. Analyseur, vérifieur et reformulateur ont leurs propres plafonds.

Et une limite de la mesure elle-même, qu'il faut dire : sur le bras **exigé**, le filtre du client écarte les réponses où le modèle avoue une omission. Ce bras est donc, par construction, biaisé vers les réponses qui déclarent tout préserver. C'est précisément pourquoi le bras **servi** existe — il n'a aucun filtre.

