# É0 — Le périmètre est arrêté

**Date :** 4 septembre 2026 · **Dossier :** `~/Downloads/C1-C3` · **Branche source :** `c1-c3` @ `23bb24e`

Ce document existe pour une raison : un périmètre n'est arrêté que si la décision est **écrite**.
Sans cela elle n'est pas prise, elle est reportée — et elle se rouvrira à la prochaine passe.

## La ligne

> **Retirer ce qu'un document diffère ou exclut, et ce qui appartient mesurablement à la surface
> cliente d'un autre produit. Garder ce qui est seulement inutilisé.**

Cette ligne est asymétrique **à dessein** : un fichier en trop qui compile coûte du bruit, un
fichier manquant coûte une pièce du produit. On ne retire donc que sur preuve, jamais sur
impression.

---

## Décision 1 — la cascade `examples` / `clients/python` / `catalogs` : **RETIRÉE** (171 fichiers)

### Ce qui a été retiré
| | |
|---|---|
| `clients/python/**` | 156 |
| `examples/*.py`, `examples/tests/`, `examples/tutorial-1-classify-text/`, `examples/README.md` | 12 |
| `catalogs/**` | 2 |
| `tests/stub_server.py` | 1 |

### Ce qui a été **gardé**, et pourquoi
* **`examples/run.json`** — réellement produit : `compose.yaml:84`, `scripts/m5-check.sh:56` et
  `scripts/p1-check.sh:256` le postent sur `/v1/swarm/run`.
* **`tests/fixtures/multilingual_eval.jsonl`** — justification **indépendante** de la cascade :
  `docs/compliance/ai_act_annex_iv.md` §2.3 le nomme comme **la preuve elle-même** du seuil
  « Cross-lingual eval F1 ≥ 0.80 ».

### Les deux preuves, et elles sont de natures différentes

**La mesure.** Les douze exemples font `from efe_tool import EfeClient`, et `efe_tool` vise
`efe.v1.catalog` (49 usages), `efe.v1.admin` (34), `efe.v1.SwarmService` (8) — en **gRPC**. Or,
dans ce dossier :

```
crates implémentant SwarmService : 0
fichiers .proto                  : 0
serveurs gRPC (tonic)            : 0
```

Ils appellent un service qui n'existe **nulle part** ici. Aucune route servie par la passerelle
(`/run`, `/runs/:id`, `/reg/:id`, `/replay/:id`, `/memory/:family`, `/events`, `/gaps`) ne recoupe
celles qu'ils appellent (`/v1/catalog`, `/v1/types`, `/v1/efe`, `/v1/admin`).

**Le document.** `Scella_Roadmap_C1_C3_2026-08-19.tex` §2 oppose deux voies :

| | (A) C1 en expansion de C3 | (B) C1 aussi en autonome |
|---|---|---|
| collecteur générique | **inutile** — la passerelle *est* le collecteur | **indispensable : HTTP + SDK Python/TS** |
| effort supplémentaire | — | **+3 à 4 semaines** |

et tranche : *« Je recommande (A) d'abord […] (B) reste ouvert : si trois prospects demandent le
registre sans vouloir router leur trafic, vous le construirez avec leur argent. »*

**Le SDK appartient donc à la voie (B), explicitement différée** — même règle que `MX`. Et ce
`clients/python` n'est même pas ce SDK-là : c'est le client de la **classification** par essaim
latent, un autre produit.

### La contre-épreuve, faite AVANT le retrait
```
références depuis crates/             : 0
références depuis scripts/*-check.sh  : 0
références depuis compose / Dockerfile: 0
membres du workspace concernés        : 0
```

### La vérification, faite APRÈS
```
fichiers   1230 − 171 = 1059          ✓ exact
membres    30                          ✓ inchangé
cargo metadata --locked                → 0
cargo check --locked --workspace --all-targets → 0
#[test]    3922                        ✓ INCHANGÉ — aucun test Rust n'est parti
contrôles  9/9                         → 0
SHA-256    1059/1059 identiques à la source · 0 différent
```

---

## Décision 2 — `verify-meta-mini` + `efe-meta-verify` : **GARDÉS**

C'est la décision **inverse** de celle que quatre passes d'analyse recommandaient, et elle repose
sur une mesure que ces passes n'avaient pas faite.

### Ce qui plaidait pour le retrait
Les deux se déclarent eux-mêmes « re-prouveur de consensus de **méta-essaim** », attendent
`meta_steps.jsonl` + `meta_manifest.json`, et **aucun crate du dossier ne produit ces fichiers**.
`SCELLA-TECHNOLOGIE-ARCHITECTURE.tex` ne les nomme pas une seule fois.

### Ce qui a renversé la décision
`Scella_OnePager_Entreprise_2026-08-26.tex` l. 63 vend :

> « **Deux vérifieurs autonomes, à l'arbre de dépendances vide**, que votre auditeur recompile
> lui-même : **aucune bibliothèque tierce**, aucun réseau, un code de sortie. »

Mesure des quatre candidats :

| crate | dépendances normales |
|---|---|
| `verify-mini` | **0 — littéralement vide** |
| `verify-meta-mini` | **0 — littéralement vide** |
| `provenance-verify` | **17** (serde, serde_json, provenance-dag, syn, quote…) |
| `efe-meta-verify` | 69 |

Pris **à la lettre** — et « aucune bibliothèque tierce » ne laisse pas de latitude — les deux
vérifieurs vendus ne peuvent être que `verify-mini` et `verify-meta-mini`. `provenance-verify`
tire dix-sept caisses.

⟹ **Retirer `verify-meta-mini` rendrait une affirmation commerciale fausse.** Le garder coûte deux
crates (arbre vide, 5 tests verts) et un défaut clippy trivial. L'asymétrie tranche.

`efe-meta-verify` suit : il est la dev-dep de `verify-meta-mini` pour son recoupement de fidélité.

### ⚠ Ce que cette mesure a mis au jour, et qui dépasse le code

Le OnePager promet **deux** vérifieurs à arbre vide. `SCELLA-TECHNOLOGIE-ARCHITECTURE.tex` §S4 en
décrit **un** — au singulier, et ne nomme que `verify-mini`. Et le second candidat,
`verify-meta-mini`, vérifie un artefact que **le produit ne produit pas**.

Les deux lectures possibles sont l'une et l'autre embarrassantes :

* si les deux vendus sont `verify-mini` + `provenance-verify`, alors « aucune bibliothèque
  tierce » est **faux** pour le second ;
* si ce sont `verify-mini` + `verify-meta-mini`, alors le second **ne vérifie rien de ce que le
  produit livre**.

**Ce n'est pas une décision de code : c'est une phrase commerciale à corriger avant de la dire à
un client.** Elle est signalée ici plutôt que laissée à découvrir.

---

## Ce qui reste délibérément dans le dossier, et pourquoi

* **`scripts/swarm/*.py`** (3 fichiers) — `crates/efe-swarm-sdk/tests/fewshot_modec.rs:245`
  **écrit** dans ce répertoire ; il ne lit pas ces scripts et ne les cite pas. Le répertoire est
  requis, ses scripts ne le sont pas — mais trois fichiers inutilisés ne justifient pas de rouvrir
  la chaîne `convert_routerbench.py` → `route_eval.py`. Gardés, et dit.
* **`ci.yml` et `Justfile`** — ils pilotent encore du code absent (`efe-ffi`, `efe-meta-*`,
  `Dockerfile.prod`, `poc.sh`). La réparation est d'en **retirer des jobs**, jamais de rapatrier
  ce code. C'est É3, pas É0.
