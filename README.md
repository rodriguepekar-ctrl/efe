# Scella — Le Seuil & Le Registre

> Un reçu souverain, en **une** commande, sur votre machine.

```bash
docker compose up
```

Sur une machine où rien n'est installé, Docker excepté. Aucun compte, aucune clé
d'API, aucune inscription.

**Si rien ne démarre, c'est presque toujours le port.** `8080` est occupé sur
beaucoup de machines ; il se déplace sans éditer un fichier :

```bash
SCELLA_PORT=18080 docker compose up
```

La passerelle n'est publiée que sur la boucle locale `127.0.0.1`, jamais sur
toutes les interfaces de l'hôte.

## Ce que vous verrez

D'abord la passerelle **annoncer ce qu'elle sert réellement** — son rapport de
démarrage nomme chaque capacité et ce que son absence coûte, y compris
« provenance **DÉGRADÉE** », parce qu'aucune clé n'a été fournie. Puis une
démonstration en cinq étapes : un run, son reçu, son export, et une **troncature
démasquée par un simple comptage — sans aucune clé, sans cryptographie, sans
accorder sa confiance à qui que ce soit**.

Cette dernière propriété est la seule qui vaille d'être regardée. Le sceau d'une
chaîne a une faiblesse que rien, de l'intérieur, ne trahit : *le préfixe d'une
chaîne valide est lui-même valide*. Un bundle tronqué se re-prouve parfaitement.
Le seul témoin possible est **hors** du bundle — l'ancre publiée à l'export, qui
annonce le nombre de nœuds. La confronter est un comptage, que vous refaites
vous-même avec `grep -c` et `awk` sur les fichiers laissés dans le volume.

> **Ce que cette démonstration ne prouve PAS.** Aucune clé n'est fournie : la
> passerelle tourne sur la clé de **test publique**, connue de quiconque a lu ce
> dépôt. Le reçu produit démontre le **mécanisme**, jamais la **substance** — il
> n'oppose rien à personne. Pour cela il faut une clé que vous seul détenez :
> `EFE_NANO_AUDIT_HMAC_MASTER=$(openssl rand -hex 32) docker compose up`.
> Et ce qu'un reçu atteste, même avec une vraie clé, reste l'**intégrité** de ce
> qui a été enregistré — jamais la **correction** de la décision.

**La page à lire avant de lancer** —
[`docs/reports/Scella_Le_Seuil_En_Une_Commande_2026-09-04.tex`](docs/reports/Scella_Le_Seuil_En_Une_Commande_2026-09-04.tex)
tient sur une feuille : la commande, ce qui va défiler, les commandes exactes
pour tout recompter soi-même sur les fichiers du volume, et les quatre choses que
la démonstration ne prouve pas. Chacun de ses gestes a été joué verbatim.

## Le second produit, dans la même commande

```bash
docker compose --profile jonction up
```

**Le Registre** dérive, du **même** run, un dossier d'évidence réglementaire :
une vue de cadre daté, son mapping versionné, et son mode d'emploi de
re-dérivation. Le dossier porte le nom du run — c'est le lien, pas un rangement.

Il ne prononce **pas** la conformité : il mappe de l'évidence sur des exigences
et publie le compte de ce qui est couvert, partiel, ou hors périmètre. *Outillé
n'est pas conforme.*

## Les contrôles

Rien de ce qui précède n'est à croire sur parole. Quinze contrôles rejouent
chacun leur module, et refont eux-mêmes les comptages plutôt que de croire le
pilote :

| commande | ce qu'elle mesure | démon Docker |
|---|---|---|
| `bash scripts/m4-check.sh` | la charge utile — passerelle, reçu, ancre, troncature | non |
| `bash scripts/e1-check.sh` | l'emballage — image, volume, réseau ; `jonction` en argument y ajoute Le Registre | oui |
| `bash scripts/p1-check.sh` | le profil de production démarré pour de vrai, tué, redémarré, reçu re-prouvé | oui (PostgreSQL) |
| `bash scripts/portes-check.sh` | `clippy -D warnings` sur **chaque** profil que `compose.yaml` construit | non |
| `bash scripts/r3-check.sh` | la suite complète, rejouée (`R3_TOURS=3` pour le critère) | non |

Ils distinguent trois états, et le troisième compte autant que les deux autres :
`0` mesuré et vert · `1` mesuré et rouge · `3` **non mesuré** — l'instrument n'a
pas pu regarder. Un contrôle en panne doit dire qu'il est en panne, et ne jamais
emprunter le vocabulaire du sujet qu'il mesure.

## Structure

```
crates/                    30 membres de workspace
  efe-swarm-gateway/       la passerelle — Le Seuil
  reg-evidence/            l'évidence réglementaire — Le Registre
  scella-preuve/           le pilote de la démonstration (il conduit, il ne prouve pas)
  verify-mini/             le vérifieur tiers — arbre de dépendances VIDE
  provenance-verify/       le second vérifieur, le seul qui sache lire une ancre
  scella-conservation/     archivage, restauration, générations de clé
  audit-console/           la console opposable
compose.yaml               les trois profils : coin · jonction · production
profiles/                  les profils d'environnement, sous test
docker/                    l'image distroless (quatre binaires, aucun shell)
scripts/                   les seize contrôles
docs/reports/              la page qu'on tend, et les feuilles de route
LISEZ-MOI-C1-C3.md         l'inventaire mesuré du périmètre
```

## Construire et éprouver

```bash
cargo test  --locked --workspace              # 3 908 tests
cargo clippy --locked --workspace --all-targets -- -D warnings
bash scripts/portes-check.sh                  # la même porte, sur chaque profil servi
```

Rust **1.88**, épinglé par `rust-toolchain.toml` et `Cargo.toml`.

## Le projet de recherche qui vivait ici

Ce dépôt a d'abord porté **EFE — *Ephemeral Fluid Swarm***, un travail sur
l'inférence multi-agent par communication en espace latent partagé. Ses vingt et
un documents d'architecture occupaient la racine, et son README était cette page.

Ils ne sont **pas supprimés** : ils sont archivés, intacts et vérifiés fichier par
fichier, dans `Downloads/vrai/EFE-docs-racine-2026-09-04/`, avec une note qui dit
d'où ils viennent et ce que leur propre README affirmait de faux.

Le 5 septembre, **38 dossiers de la racine** puis **1 085 fichiers** de `docs/`,
`scripts/`, `docker/` et de la racine ont suivi — le portail, la plateforme
Python, les SDK polyglottes, les chartes Helm, Terraform, les bancs, les bacs à
sable : `Downloads/vrai/EFE-arbre-2026-09-05/`, 61 023 fichiers, ~1,5 Gio. Le dépôt
est passé de **48 dossiers à la racine à six**, et de 13 000 fichiers suivis à
**1 020**. Ce qui reste est cité par le code, par un contrôle, ou appartient à
Scella. Là encore rien n'est supprimé, et rien n'a bougé avant d'avoir été
vérifié : aucune référence depuis le produit, puis `cargo metadata --locked` et
`cargo check --locked --workspace` joués juste après les quatre dossiers les plus
risqués.

## Licence

Propriétaire — voir [LICENSE](LICENSE). ⚠ Un crate du périmètre,
`efe-nano-sdk`, déclare `MIT`, une licence que ce fichier ne mentionne pas :
la contradiction est relevée et non réglée (étape **D3** de la feuille de route).
