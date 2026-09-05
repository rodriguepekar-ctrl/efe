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

## Quand c'est fini — et ça ne se voit pas tout seul

La démonstration s'arrête sur `═══ DÉMONSTRATION OK ═══`, et le conteneur
`preuve` sort en `0`. **La commande, elle, ne rend pas la main** : la passerelle
est un serveur, elle continue de servir. Rien à l'écran ne dit que c'est
terminé — c'était, jusqu'au 5 septembre 2026, le seul endroit de ce parcours que
ce README ne décrivait pas.

```bash
# quitter : Ctrl-C
docker compose down -v     # et ne rien laisser derrière : conteneurs, réseau, volume
```

Tant que la passerelle tourne, elle répond sur `127.0.0.1:8080` (ou le port que
vous lui avez donné). Sans le `-v`, le volume `scella_preuve` survit — c'est ce
qu'il faut si vous voulez recompter les fichiers plus tard, et ce qu'il ne faut
pas si vous voulez repartir de zéro.

**Combien de temps.** Mesuré le 5 septembre 2026, image `scella-coin:local`
supprimée au préalable : **75 s** entre `docker compose up` et la dernière ligne
de la démonstration. Le cache de construction de Docker était chaud (seules les
couches applicatives ont été refaites) ; une machine vraiment neuve reconstruit
les dépendances **et** télécharge les images de base, donc davantage.

> Ce chiffre a d'abord été écrit à **201 s**, et c'était faux : 201 s était le
> plafond de la boucle qui attendait la fin — une boucle qui ne pouvait pas la
> voir, puisque la commande ne rend pas la main. C'est la ligne de fin ajoutée
> ci-dessus qui a rendu la mesure possible. Une durée sans point d'arrêt
> observable n'est pas une mesure, c'est un délai d'attente.

**Si le port est pris**, voici le message exact — il vaut d'être lu de près :

```
Error response from daemon: failed to set up container networking:
Bind for 0.0.0.0:8080 failed: port is already allocated
```

Deux choses qu'il ne dit pas. D'abord **`SCELLA_PORT` n'y apparaît pas** : le
remède est ici, dans ce fichier, pas dans l'erreur. Ensuite le `0.0.0.0` n'est
**pas** notre liaison — `compose.yaml` demande `127.0.0.1:${SCELLA_PORT:-8080}`,
et Docker rapporte la liaison de *celui qui détient déjà le port*, pas celle
qu'on réclamait. La passerelle n'est jamais publiée sur toutes les interfaces.

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

Rien de ce qui précède n'est à croire sur parole. Seize contrôles rejouent
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

### Ce qu'une machine en joue

Un `push` ou une `pull request` sur `c1-c3`, `main` ou `release/**` déclenche
[`.github/workflows/portes.yml`](.github/workflows/portes.yml) : les trois portes
`clippy -D warnings`, puis **les onze contrôles restants qui n'exigent
pas de démon Docker — chacun sa propre étape**, pour qu'un échec nomme le contrôle
plutôt que la boucle — puis `cargo test --locked --workspace`. Quatre contrôles restent à la
main, et c'est écrit dans l'en-tête du fichier : `e1` et `p1` veulent un démon (et
un vrai PostgreSQL), `mz` veut un modèle de plusieurs gigaoctets, `r3` veut trois
tours d'affilée là où la CI n'en joue qu'un.

Le dépôt portait **90** workflows. Aucun ne lançait un `*-check.sh` ; le seul qui
nommait la porte ne partait jamais sur la branche de travail, et sa première étape
— `cargo fmt --all -- --check` — serait tombée sur 4 643 fichiers, ce dépôt étant
formaté à la main. Les 90 sont archivés hors du produit, pas supprimés.

**Il est vert chez GitHub** : run `33977178927`, 10 min 32 s — trois portes,
onze contrôles, **3 941 tests passés · 0 échoué · 191 binaires**. Le passage
précédent était tombé, non sur le code mais sur `No space left on device` : ce
workspace écrit 189 binaires de test *plus* trois jeux d'artefacts clippy dans un
même `target/`, et le runner n'offre que 14 Gio. Le workflow retire donc
l'outillage préinstallé (**22 Gio** rendus, mesurés avant/après) et construit sans
tables de débogage — `target/` tombe à **3,1 Gio**, et une étape finale
`if: always()` imprime la marge restante (**31 Gio**) pour qu'un manque de place
ne se relise jamais comme un défaut du code.

Le workflow avait d'abord été **joué sur Linux avant d'être écrit dans le dépôt**,
dans `rust:1.88-bookworm` sous un utilisateur non privilégié. Le premier passage était
rouge : un trait jamais importé, trois gardes de plateforme écrites deux fois,
deux `if let Err(_)`. Ces cinq défauts vivaient tous sous
`#[cfg(target_os = "linux")]` — jamais compilés, donc jamais lintés, sur aucune
machine de développement. C'est le seul argument qui compte pour cette étape : un
contrôle qui ne tourne que là où l'on travaille ne garde que ce qu'on y voit.

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
sable : `Downloads/vrai/EFE-arbre-2026-09-05/`, 62 361 fichiers, ~1,6 Gio. Le dépôt
est passé de **48 dossiers à la racine à six** (plus `.github/` et `.sigstore/`),
et de 13 000 fichiers suivis à **926** — 925 après le
retrait, plus le seul workflow que D4 ajoute. Ce qui reste est cité par le code, par un contrôle, ou appartient à
Scella. Là encore rien n'est supprimé, et rien n'a bougé avant d'avoir été
vérifié : aucune référence depuis le produit, puis `cargo metadata --locked` et
`cargo check --locked --workspace` joués juste après les quatre dossiers les plus
risqués.

## Licence

Propriétaire — voir [LICENSE](LICENSE). Le crate `efe-nano-sdk` déclare `MIT` :
la contradiction avec le fichier de licence est **réglée** (étape **D3**) — le crate
y a sa ligne, et le texte MIT qu'il promettait existe enfin, dans
[LICENSE-MIT](LICENSE-MIT). `scripts/d3-check.sh` refuse toute divergence entre les
licences déclarées par les manifestes et ce que le fichier de licence accorde.

## Les deux autres portes

- [`CONTRIBUTING.md`](CONTRIBUTING.md) — la chaîne d'outils, la règle qu'on
  enfreint en premier (**jamais `cargo fmt`**), et ce qui remplace ici la revue
  par un pair : seize contrôles, trois états, des planchers anti-vacuité.
- [`SECURITY.md`](SECURITY.md) — ce qui est vrai de la posture et **où le
  vérifier**, ce qui manque (sauvegarde : zéro ; rotation de clé : zéro), et
  pourquoi aucun canal de divulgation n'est publié tant que le dépôt est privé.

Les deux décrivaient encore, le 5 septembre 2026, un dépôt disparu — un ADR, un
`CHANGELOG`, un `CODEOWNERS`, une chaîne Go, sept documents de conformité, quatre
adresses courriel et une licence Apache-2.0 que [LICENSE](LICENSE) contredit. Un
garde exécuté par `scripts/d3-check.sh` tient désormais la moitié mécanisable du
problème : **tout lien relatif de ces quatre documents doit résoudre**.
