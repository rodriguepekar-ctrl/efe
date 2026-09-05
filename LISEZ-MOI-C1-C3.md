# Branche `c1-c3` — l'espace dédié aux deux produits

Cette branche ne contient que **C1 et C3**. Tout le reste du dépôt — la recherche
sur l'essaim latent, le méta-essaim, les bancs d'essai, les SDK clients — a été
retiré. Rien n'est perdu : les autres branches le portent toujours.

## Ce qu'il y a dedans

**29 crates**, dérivés mécaniquement puis vérifiés par construction :

| produit | binaire | rôle |
|---|---|---|
| **C3 — Le Seuil** | `efe-swarm-gateway` | la passerelle souveraine à reçu |
| **C1 — Le Registre** | `audit-console` | le registre de preuve réglementaire |
| tiers | `verify-mini` | le vérifieur à **arbre de dépendances vide** |
| démo | `scella-preuve` | le pilote de `docker compose up` |

Les 25 premiers viennent de `cargo tree --edges normal` sur les binaires produit.
**Quatre s'y ajoutent, qu'aucune fermeture descendante ne pouvait voir** :

* `forensic-export` — dépendance **normale** de `audit-console`, `provenance-bridge`
  et `reg-evidence`. Ratée par un filtre de préfixes, pas par cargo.
* `efe-nano`, `efe-meta-verify` — **dev-dependencies**. `--edges normal` les exclut,
  et c'est fatal : cargo *résout* le manifeste d'une dev-dep sans la compiler, donc
  une dev-dep `path` absente fait sortir `cargo build` en **101**.
* `scella-mz` — invisible **par construction** : c'est un *dépendant*, pas une
  dépendance. `scripts/mz-check.sh` le pilote cinq fois.

## Trois couplages qui cassent la COMPILATION si on les rompt

Ils ne passent par aucune arête Cargo. Les déplacer, c'est casser le build.

| site | ce qu'il exige |
|---|---|
| `forensic-export/src/vendor.rs:17-23` | `include_str!` sur **7 fichiers** de `../../verify-mini/src/` — dans la lib, pas sous `cfg(test)` |
| `efe-swarm-gateway/src/routes/dashboard.rs:9` | `include_str!("../../assets/dashboard.html")` |
| `efe-multi-llm/tests/unit_slm_assets.rs:22` | `../efe-nano-app-coordinator/assets/schemas/*.json`, avec `panic!` si absent |

⚠ **Les crates se copient ENTIERS.** `efe-coordinator/Cargo.toml` déclare 29 sections
`[[test]]`/`[[example]]`/`[[bench]]` à chemins explicites ; un seul fichier déclaré
manquant est une erreur cargo dure.

## Le `Cargo.lock` : un seul ordre d'opérations fonctionne

1. **Sans lock** → `exit 101` : `spin 0.9.8` est **yanké** de crates.io
   (`flume ← sqlx-sqlite ← sqlx ← passerelle`). Une résolution neuve est *impossible*.
2. **Lock hérité + `--locked`** → `exit 101` : retirer 47 membres change le lock.
3. **Lock hérité, UNE passe sans `--locked`, puis `--locked`** → **exit 0**.

Fait ici : 10 217 → **6 274 lignes**. L'élagage est sûr — l'ensemble (nom, version)
obtenu est un sous-ensemble strict du précédent, zéro dérive de version.

## Ce qui est vérifié

```
cargo metadata --locked                          exit 0
cargo check --locked --workspace --all-targets   exit 0   (1 min 26)
```

Contrôles SANS démon : **10 sur 13 sortent 0** — `profile`, `m4`, `m5`, `m6`, `m7`,
`m8`, `m9`, `p2`, `p3`, `orphelins`.

## D1 — la porte d'entrée

Le `README.md` annonçait **un autre produit** : ligne 1 « EFE — Ephemeral Fluid
Swarm », ligne 3 « inférence multi-agent en espace latent partagé », et le mot
*Scella* n'apparaissait qu'à la **ligne 12**. Depuis le 4 septembre 2026, la ligne 1
nomme le produit et la commande est **ligne 6** — au-dessus du pli.

Et sa seconde moitié décrivait un dépôt qui n'existe plus. Mesuré avant réécriture :

| ce que le README affirmait | ce qui existe |
|---|---|
| `crates/efe-core`, `efe-kernels`, `efe-server` | **absents** (retirés par É0) |
| `crates/efe-ffi` « bindings PyO3 » | un seul fichier : un `.whl` construit |
| `git clone git@github.com:efe-ai/efe.git` | le dépôt réel est `github.com/rodriguepekar-ctrl/efe` |
| « Rust 1.85 » | `rust-toolchain.toml` **et** `Cargo.toml` disent **1.88** |

La nouvelle structure est vérifiée ligne à ligne : 30 membres, 15 contrôles,
4 binaires dans l'image, 3 profils dans `compose.yaml`.

## La page qu'on tend

`docs/reports/Scella_Le_Seuil_En_Une_Commande_2026-09-04.tex` — **une page**, à lire
*avant* de lancer la commande : ce qui va défiler, ce qu'on peut recompter soi-même,
et ce que la démonstration ne prouve pas. Construite avec `tectonic` ; seule la
`.tex` est suivie (`docs/**/*.pdf` est ignoré). Vérifiée : **1 page exactement**,
**0 débordement**.

Chacun de ses gestes a été joué **verbatim** le jour de son écriture, et chaque
chiffre concorde :

```
docker compose cp preuve:/preuve ./p          → 3 fichiers
awk -F'\t' -v r=$RUN '$2==r {print $5}' …     → 3
grep -c '[^[:space:]]' p/bundle.sealed.jsonl  → 3
grep -c '[^[:space:]]' p/bundle.TRONQUE.jsonl → 2
grep -c $RACINE p/bundle.sealed.jsonl         → 1
grep -c $RACINE p/bundle.TRONQUE.jsonl        → 0
tailles annoncées 157/1340/893 = tailles réelles
```

⚠ **Un document dont le texte devient faux un module plus tard** est le défaut
récurrent de ce dépôt. Les chiffres de la page sont **datés** et l'en-tête le dit ;
ses **gestes**, eux, sont gardés : `e1-check.sh` vérifie qu'elle nomme les trois
fichiers que le pilote écrit et qu'elle enseigne la sélection d'ancre **par le run**.
Contre-prouvé : renommer un fichier dans la page, ou y réintroduire
`END{print $5}`, fait tomber le contrôle (`exit 1`).

Au passage, la règle réfutée par É2 — « la dernière ligne du journal » — était encore
enseignée à **trois** endroits : le pilote lui-même (étape 4), cette page, et
`m4-check.sh`. Les trois disent désormais la même chose. Un contrôle qui montre un
geste faux le propage, même là où son contexte le rend inoffensif.

## Les PORTES — une par profil réellement servi

`scripts/portes-check.sh` joue `clippy -D warnings` sur **chaque profil que
`compose.yaml` construit** : le coin (features par défaut, workspace entier), la
jonction (`reg-evidence`) et la production (`postgres,oidc,pii,frontier-llm`).

La liste des profils n'est **pas écrite dans le script** : elle est lue dans
`compose.yaml`, le fichier qui décide de ce qui est servi. Ajouter un profil au
compose sans le garder devient donc impossible — la porte suit le produit au lieu
d'être une liste parallèle qui diverge en silence.

Mesuré le 4 septembre 2026 : **3 portes, 0 échec, `exit 0`.** Les trois états ont été
exercés le même jour :

| sortie | déclencheur | résultat |
|---|---|---|
| `0` | l'arbre tel quel | 3 portes vertes |
| `1` | un défaut injecté sous `#[cfg(feature = "reg-evidence")]` | la porte du **coin reste verte**, celle de la jonction **tombe** en nommant la ligne |
| `3` | `CARGO_FEATURES` renommé dans compose.yaml | plancher anti-vacuité : « 0 jeu lu, 2 attendus » |

La contre-épreuve du milieu est la raison d'être du script en une image : un défaut
que la porte du dépôt ne peut pas voir, attrapé par le profil qui l'embarque.

⚠ **`cargo fmt` n'est pas parmi ces portes**, et c'est mesuré : ce dépôt est formaté
à la main, `cargo fmt --all -- --check` réécrirait **4 643 fichiers**. C'est aussi
pourquoi `ci.yml` ne pourrait pas passer même s'il se déclenchait — son job
`fast-checks` met `rustfmt` **avant** `clippy` et tomberait sans jamais atteindre la
porte.

⚠ **Rien n'exécute tout cela hors de la machine.** Le dépôt porte **90 workflows** ;
**aucun** ne lance un `*-check.sh`, et `ci.yml` ne se déclenche que sur
`main` / `release/**` — la branche de travail est `c1-c3`.

## Une feature qui ne compile pas

`efe-nano-app-coordinator/fastembed-real` échoue en `E0599` : `EmbeddingModel::BGEM3`
n'existe plus dans `fastembed 4.9.1`. La dépendance a été portée de 3.x à 4 sans que
le code suive. **Personne ne l'active** — aucun manifeste, aucun script, aucun
workflow — et aucune porte ne la compilait, d'où le silence.

Elle n'est pas réparée ici : choisir un modèle de remplacement change les vecteurs
produits, c'est une décision produit. Ce qui est corrigé, c'est la **promesse** — le
manifeste disait « Pour activer : `cargo test --features fastembed-real` », et cette
ligne a été retirée au profit du diagnostic.

## Le profil de PRODUCTION — et l'angle mort que la porte de qualité laissait

`scripts/p1-check.sh` est le seul contrôle qui éprouve `postgres`, `oidc`, `pii` et
`frontier-llm` **ensemble**, en démarrant un vrai binaire contre un vrai Postgres, en
le tuant, et en re-prouvant le reçu depuis un processus neuf.

Mesuré le 4 septembre 2026, première exécution sur ce périmètre : **17 contrôles,
`exit 0`.**

⚠ **`cargo clippy --workspace --all-targets` compile les features PAR DÉFAUT.** Les
quatre ci-dessus n'en sont pas : `routes/mod.rs:18` gate `pub mod llm` sur
`frontier-llm`, qui allume à son tour trois fournisseurs d'`efe-coordinator`. La porte
du dépôt pouvait donc être verte pendant que le chemin d'egress payant portait sa dette
intacte — et elle l'était : **sept défauts** y dormaient, dont

* une paire d'attributs `#[cfg]`/`#[test]` **orpheline** que le rapport de P1 citait
  déjà comme la preuve que ce chemin vivait hors de la rigueur du dépôt ;
* le garde de démarrage de l'egress payant, `frontier_boot_guard`, qui prenait **cinq
  booléens positionnels** — le cas de transposition silencieuse le plus pur du
  périmètre, invisible à clippy (cinq passe sous le plafond de six) ;
* deux tests M3 (`le SLM meurt, la mission passe, la raison remonte`) qui sont
  `#[cfg(not(feature = "oidc"))]` : **sous le profil de production, ils ne tournent
  pas**. Ce trou est écrit à l'endroit exact, dans `routes/llm.rs` ; il n'est pas comblé.

`p1-check.sh` porte désormais cette porte en tête d'étape 1. Un contrôle qui démarre le
profil de production sans en éprouver la qualité ne vérifie que la moitié de son sujet.

## R3 — la cause a disparu, personne ne l'a corrigée

`cargo test --locked --workspace` → **3 904 tests, 0 échec**. Mais R3 ne cherche pas
qu'un tour vert : il cherchait de l'**instabilité**, et la mesure du 27 août l'avait déjà
réfutée — les deux « instables » étaient parfaitement déterministes. La panique venait de
**deux fournisseurs cryptographiques rustls** liés dans un même binaire, par unification
des features sous `--workspace` (`efe-agent-runner → axum-server "tls-rustls"`).

Mesuré le 4 septembre : `axum-server` et `aws-lc-rs` ont **0 entrée** dans `Cargo.lock`,
`ring` en a une. `crates/efe-agent-runner` est un dossier **vide**, hors workspace. La
cause n'a pas été réparée — elle est **hors de portée**, emportée par l'élagage de É0.

C'est une propriété fragile et invisible : elle ne tient à aucune ligne de code, et une
seule dépendance TLS ajoutée la casse. D'où
`crates/efe-swarm-gateway/tests/r3_fournisseur_unique.rs` — un garde, trois
contre-épreuves, un plancher anti-vacuité. Contre-prouvé le jour de son écriture : un
`aws-lc-rs` injecté dans le verrou le fait tomber (`2 fournisseur(s) — ["aws-lc-rs",
"ring"] — sur 603 paquets`).

Le remède que R3 prescrivait — `serial_test` — est **inopérant** sur ce mode de
défaillance, et sa propre note le disait : il sérialise *dans* un binaire, quand la
panique naît de la **résolution des features**, avant qu'un test ne s'exécute.

⚠ **Et `r3-check.sh` ne pouvait pas aboutir sur une suite verte.** Une paire
`set +e` / `set -e` allumait `errexit` pour la boucle — un état que l'en-tête du
fichier exclut explicitement. Quinze lignes plus bas,
`grep '^test .* FAILED$'` rend **1 quand il ne trouve rien**, c'est-à-dire quand le
tour est vert : le script se tuait là, sans une ligne de sortie, et rendait `1` — que
son propre contrat documente comme « des échecs **instables** ». Une suite parfaitement
verte rapportée comme instable.

Le défaut était invisible tant qu'il y avait des échecs : le 27 août il y en avait,
`grep` trouvait, le script survivait. Il ne pouvait se manifester qu'au moment exact où
R3 allait enfin pouvoir conclure. **Ce n'est donc pas le disque qui a empêché R3
d'aboutir.**

## Le contrôle de la PORTE — le seul qui exige un démon Docker

`scripts/e1-check.sh` mesure les critères de **É1** (le coin) et de **É2** (la
jonction). Il est le seul du dépôt à exiger Docker, et c'est exactement la moitié
que `m4-check.sh` déclare, dans son propre en-tête, ne pas prétendre couvrir :
l'EMBALLAGE — l'image, le volume, le réseau de compose.

```
bash scripts/e1-check.sh                É1     · 17 contrôles
bash scripts/e1-check.sh jonction       É1+É2  · 35 contrôles
bash scripts/e1-check.sh --relire       relit le volume SANS rien exécuter
```

Trois états, et le troisième n'est pas un raffinement :

| sortie | sens |
|---|---|
| `0` | mesuré, et la porte s'ouvre |
| `1` | mesuré, et elle ne s'ouvre pas |
| `3` | **NON MESURÉ** — l'instrument n'a pas pu regarder |

Mesuré le 4 septembre 2026 : **É1 17/17**, **É1+É2 35/35**, `exit 0` dans les deux
cas (`SCELLA_PORT=18080 SCELLA_PORT_JONCTION=18081`). Les trois états ont été
exercés le même jour — non mesuré sur un port pris, rouge sur un volume abîmé
(4 échecs), vert sur une exécution réelle.

⚠ **Ne jamais lire `ExitCode` sans lire `State`.** Mesuré ce jour-là : 8080 était
pris, `up` s'est arrêté à la mise en réseau, **aucun conteneur n'a démarré** — et
les deux lignes de `docker compose ps -a --format json` portaient
`"State":"created"` avec `"ExitCode":0`. Zéro est la valeur par DÉFAUT d'un
conteneur qui n'a jamais tourné : un contrôle qui lit le code sans lire l'état
fabrique le succès qu'il prétend mesurer.

⚠ **`verify-mini` ne peut pas contredire une troncature**, avec ou sans clé. Sans
clé il ne rend aucun verdict (sortie 2, usage) ; avec la clé il VALIDERAIT le
bundle tronqué — « un préfixe de chaîne valide reste valide », dit l'aide de
`provenance-verify`. Ce qui contredit sans clé, c'est le COMPTAGE contre l'ancre ;
ce qui contredit avec la clé, c'est `provenance-verify --anchor --run`, le second
vérifieur que É1 a ajouté à l'image.

## Ce qui a été retiré, et pourquoi

* **47 membres de workspace** hors périmètre, dont les quatre hors `crates/`
  (`tools/loadgen`, `bench/harness`, `smoke/runner`, `slos/rollup`) — 1 566 fichiers.
* **`scripts/m1b-check.sh` et `scripts/r1-check.sh`** : mesurés inapplicables. Ils
  pilotent `efe-meta-swarm-server` et `efe-mode1`, qui ne sont pas ici — `cargo`
  répond « did not match any packages ». Un contrôle pour du code absent ne mesure rien.
* **Deux exemptions de `p3_aucune_cible_abandonnee.rs`** visant `efe-meta-swarm{,-server}`.
  Les retirer n'est pas contourner ce garde : c'est lui **obéir**. Sa règle est qu'une
  exemption dont la cible a disparu est périmée, et il est tombé pour cette raison exacte.

## Ce qui reste ouvert, et qu'il ne faut pas croire réglé

* **`scripts/r3-check.sh` n'a PAS été mesuré ici.** Il a échoué sur
  `No space left on device`, pas sur le code. Son applicabilité sur cette branche est
  **inconnue**, et le supprimer sur ce prétexte aurait été un mensonge de plus.
* **`cargo test --workspace` n'a jamais été exécuté** sur cette branche. `check` prouve
  que tout **compile** ; compiler n'est pas passer.
* ✅ **D2 est fait** (2026-09-05). La racine portait **48 dossiers** ; il en reste
  **six** — `crates/`, `docker/`, `docs/`, `examples/`, `profiles/`, `scripts/`. Les
  38 autres sont archivés dans `Downloads/vrai/EFE-arbre-2026-09-05/` : 61 023
  fichiers, ~1,5 Gio, **0 divergence**. Fichiers suivis : 13 000 → **2 117**.

  L'ordre a compté. Les quatre dossiers que l'architecture de É1 interdisait de
  toucher — `tools/`, `bench/`, `smoke/`, `slos/`, « membres du workspace dont
  `cargo --locked` a besoin pour résoudre » — sont partis **en premier**, après
  vérification que É0 avait retiré leurs manifestes (`find … -name Cargo.toml` → 0).
  L'épreuve a suivi immédiatement : `cargo metadata --locked` → 0, 30 membres ;
  `cargo check --locked --workspace --all-targets` → 0.

  ⚠ **`diff -r` a bloqué 28 minutes sur `clients/`** (666 fichiers, 5 Mo) : il suit
  les liens symboliques, et `.venv/bin/python3` pointe sur l'interpréteur système.
  Remplacé par un comparateur qui ne suit rien — SHA-256 des fichiers réguliers,
  cible des liens — contre-prouvé avant emploi. Les 27 dossiers restants : quelques
  secondes.

  Trois fichiers décrivaient aussitôt un arbre disparu et ont été corrigés :
  `.dockerignore`, `LICENSE` (sa seule échappée libre visait `/clients/<lang>/**`)
  et `.gitleaks.toml` — dont retirer `^mcp-server/` et `^platform/` **rouvre deux
  signalements dans l'historique**, mesuré. Seconde confirmation de la leçon de D3.

  **Reste mixte** : `docs/` (le produit cite une vingtaine de ses documents
  `EFE_*` depuis ses propres sources) et `scripts/` (les seize contrôles y voisinent
  avec les outils d'EFE). Les séparer demande de résoudre chaque citation.
* ✅ **D3 est fait** (2026-09-05). `gitleaks` sortait **13** puis **10** selon la
  commande employée — deux chiffres, deux mesures différentes, aucune juste. Mesuré
  proprement, comme la CI et le hook `pre-commit` le lancent :

  | commande | avant | après |
  |---|---|---|
  | `gitleaks detect` (historique git) | 21 | **0** |
  | `gitleaks detect --no-git` (arbre) | 100 | **0** |
  | dont fichiers **suivis** par git | 19 | **0** |

  **Aucun des 19 n'était un secret** : deux fixtures PKCS8 nommées `TEST_PRIV*`, huit
  jetons littéraux `jeton-acme-*`, des empreintes SHA-256 dans un exemple livré, des
  espaces réservés de documentation, un nom de fichier et une affectation
  `EFE_LLM_FRONTIER_ENABLED=true`. Les 81 autres étaient des artefacts de build —
  `rustls`, `pem`, `pkcs8` embarquent « BEGIN PRIVATE KEY » dans leur *documentation*,
  qui atterrit dans les `.rmeta` de `target/`.

  Trois des quatre exemptions de `.gitleaks.toml` visaient des crates que É0 a
  retirés. ⚠ **Les retirer a rouvert quatre signalements dans l'historique** : une
  allowlist sert deux publics — l'arbre et les commits — et un chemin peut être périmé
  pour l'un tout en restant nécessaire à l'autre. Elles sont restaurées sous une
  rubrique qui le dit.

  `bash scripts/d3-check.sh` rejoue les deux moitiés (6 contrôles, trois états).
  Contre-prouvé : retirer `LICENSE-MIT` → 2 échecs ; injecter un `ghp_…` → 1 échec.
* **`efe-nano-sdk` se déclare `MIT`** (décision `D-M50-CRATES-001`), et le `LICENSE` du dépôt
  **ne mentionne pas MIT**. ⚠ La version précédente de cette ligne disait « aucun fichier
  `LICENSE`/`NOTICE` n'existe » : **c'est faux** — `LICENSE` existe (64 lignes, double licence,
  propriétaire par défaut, Apache-2.0 pour `/clients/<lang>/**` seulement). Le défaut réel est une
  **contradiction** : le manifeste dit MIT, la licence dit « tout ce qui ne déclare pas Apache-2.0
  est propriétaire ». Mesuré le 2026-09-04.
  Sur 413 paquets tiers : **zéro GPL/AGPL/SSPL** (mesuré), mais `webpki-roots 0.25.4`
  est MPL-2.0 en dépendance normale.

---

*Périmètre établi le 3 septembre 2026 par 18 agents sur 8 lentilles indépendantes,
chacune attaquée par un réfutateur, puis vérifié par construction réelle.*
