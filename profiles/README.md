# Profils — M0 · le profil servi, et ce qu'il ne porte pas

Des configurations nommées. Chacune déclare **tout sauf les secrets** — et, depuis
**P1**, déclare aussi *lesquels* elle exige sans les porter.

> Ce fichier ne donne volontairement **aucun compte** de profils. La version
> précédente annonçait « trois configurations » alors qu'il y en avait neuf : un
> nombre figé dans une phrase vieillit à chaque ajout, et personne ne le voit.
> `ls profiles/*.env` répond, et il ne se trompe pas.

## Pourquoi aucun secret ici

Un profil qui embarquerait `EFE_NANO_AUDIT_HMAC_MASTER` ferait dire au rapport
« provenance : SERVIE » alors que la clé serait **publiquement connue** — un
*vert malhonnête* de manuel. `scripts/profile-check.sh` §2 le refuse, et le test
`aucun_profil_ne_contient_de_secret` aussi.

Les secrets s'injectent au déploiement :

```bash
export EFE_NANO_AUDIT_HMAC_MASTER=$(openssl rand -hex 32)   # ≥ 32 octets
```

Sans lui, le binaire **démarre quand même** et le rapport dit `DÉGRADÉE — repli
sur la clé de TEST PUBLIQUE`. C'est le comportement voulu : jamais de silence.

## `#!injection` — déclarer ce qu'on ne porte pas

Trois variables sont **exigées** par le garde de démarrage de la frontière et ne
peuvent vivre dans aucun `.env` : deux sont des secrets, la troisième
(`EFE_OIDC_PUBLIC_KEY_PEM`) est un blob multiligne qu'un fichier `.env` ne sait
pas représenter.

Un profil qui active la frontière les déclare donc :

```
#!injection EFE_NANO_AUDIT_HMAC_MASTER
#!injection EFE_OIDC_PUBLIC_KEY_PEM
#!injection DATABASE_URL
```

Ces lignes sont **un commentaire** pour tout parseur `.env` et **une
déclaration** pour `tests/p1_injections.rs`, qui exige qu'elles soient
*exactement* les préconditions du garde — ni moins (le profil mentirait par
omission), ni plus (il ferait poser une variable inutile).

Avant P1, `frontiere.env` annonçait `frontière : SERVIE` dans une configuration
où le binaire **sort en 78**. Rien ne pouvait l'attraper : le garde vit dans
`main.rs` sous `#[cfg(feature = "frontier-llm")]`, hors de portée du chemin qui
dérive un rapport depuis un fichier. Un profil qui active la frontière est
désormais toujours rendu **`REFUSÉE`** tant qu'on le lit seul — parce que c'est
vrai : un profil seul ne sert pas la frontière, il la **demande**.

## Les profils, par rôle

| profil | pour quoi |
|---|---|
| `demo`, `docker` | la démonstration en une commande — aucune clé, rien ne sort de la machine |
| `souverain` | l'essaim local, sans frontière |
| `frontiere` | l'essaim + les modèles frontière, en développement |
| `jonction` | le coin **et** l'expansion : chaque run produit son dossier réglementaire |
| `production` | **P1** — le coin en service : quatre features, trois injections, un store durable |
| `console`, `service`, `expose` | la console de conformité (binaire séparé, ses propres interrupteurs) |

## Usage

```bash
set -a && . ./profiles/souverain.env && set +a
export EFE_NANO_AUDIT_HMAC_MASTER=$(openssl rand -hex 32)
cargo run -p efe-swarm-gateway -- --profile-report | jq '.capabilities[] | select(.capability=="provenance")'
```

Pour le profil `production`, les quatre features sont obligatoires, et la
passerelle **refuse de démarrer** (`exit 78`) si l'une des trois injections
manque :

```bash
cargo build -p efe-swarm-gateway --features postgres,oidc,pii,frontier-llm
bash scripts/p1-check.sh          # démarre tout, tue, redémarre, re-prouve
```

## Vérification

* `tests/profile_files.rs` compare chaque `.env` à son `.expected.json`, et
  refuse qu'un profil livré échappe à tout test.
* `tests/p1_injections.rs` compare les `#!injection` aux préconditions du garde.
* `scripts/profile-check.sh` rejoue les deux, et vérifie qu'aucun profil ne pose
  de secret.

Modifier un profil sans mettre à jour son attendu casse le test — c'est le but.
