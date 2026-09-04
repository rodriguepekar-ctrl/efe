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
* **La racine porte encore des artefacts de recherche** — `M*_ARCHITECTURE.md` de la
  série EFE (à ne pas confondre avec les modules Scella), `apps/`, `platform/`,
  `playground/`, `clients/`, `sdk/`, `terraform/`… Non retirés **délibérément** : le
  périmètre référence `docs/` **62 fois**, dont `audit-console/src/lib.rs` et le garde
  `p3`. Un élagage à l'aveugle casserait des gardes. À faire en second passage, mesuré.
* **13 signalements `gitleaks`** dans le périmètre — dont une vraie clé privée EC P-256
  en fixture (`efe-swarm-gateway/src/oidc.rs:119`, dupliquée en `routes/llm.rs:1386`) et
  `scripts/demo-consensus-receipt.sh:42` où la clé de forge du mode dégradé est en clair
  sous une variable `KEY=`. Aucune n'est une clé vive ; toutes bloqueraient un dépôt neuf.
* **`efe-nano-sdk` se déclare `MIT`** (décision `D-M50-CRATES-001`) et **aucun fichier
  `LICENSE`/`NOTICE`** n'existe dans le périmètre — la seule condition que MIT impose.
  Sur 413 paquets tiers : **zéro GPL/AGPL/SSPL** (mesuré), mais `webpki-roots 0.25.4`
  est MPL-2.0 en dépendance normale.

---

*Périmètre établi le 3 septembre 2026 par 18 agents sur 8 lentilles indépendantes,
chacune attaquée par un réfutateur, puis vérifié par construction réelle.*
