# Contribuer à Scella

Ce fichier remplace un document qui décrivait **une organisation** — deux
approbations par fusion, un `CODEOWNERS`, des canaux Slack, une adresse de
sécurité chez un domaine, une réponse « sous 4 heures ouvrées ». Rien de tout
cela n'existe. Le dépôt est **privé**, il n'a **qu'une paire de mains**, et la
discipline qui le tient réellement n'y était pas écrite.

Ce qui suit est mesuré, pas souhaité.

---

## 1 · La chaîne d'outils

`rust-toolchain.toml` et `Cargo.toml` épinglent tous deux **1.88**, et
`.github/workflows/portes.yml` joue la même. Les trois sont d'accord.

```bash
rustup install 1.88
rustup component add --toolchain 1.88 rustfmt clippy rust-src rust-analyzer
```

Entrer dans le dépôt suffit : `rust-toolchain.toml` déclenche l'installation à
la première commande `cargo`.

> Le commentaire de `rust-toolchain.toml` renvoie à
> `docs/docs/adr/ADR-D93-001-rustc-1.88-bump.md`. **Ce fichier n'existe pas** —
> plus aucun ADR n'est dans ce dépôt. La *raison* du choix est heureusement
> recopiée dans le commentaire lui-même : `jsonwebtoken 9.3.1` tire
> `simple_asn1 0.6.4`, qui verrouille `time ^0.3.47`, qui exige rustc ≥ 1.88.

Il n'y a **aucun fichier Go** dans ce dépôt. L'ancien document épinglait une
chaîne Go 1.22 pour un `cmd/vsock-relayer` qui n'existe plus.

---

## 2 · La règle qu'on enfreint en premier

> **On ne lance jamais `cargo fmt` sur ce dépôt.**

Il est formaté **à la main**. Mesuré le 5 septembre 2026 : `cargo fmt --all`
réécrirait **4 647 fichiers**. C'est aussi pourquoi la porte de qualité ne
contient pas `cargo fmt --check` — une porte qui exige le contraire du dépôt ne
garde rien, elle interdit simplement de passer.

> Le chiffre **bouge**, et c'est l'argument. Il valait 4 643 le 4 septembre ; il
> a pris quatre unités parce que quatre fichiers Rust ont été touchés depuis. Une
> porte `fmt` sur ce dépôt ne serait pas rouge une fois, elle le serait
> définitivement, et davantage à chaque contribution.

La règle est répétée dans [`README.md`](README.md),
[`LISEZ-MOI-C1-C3.md`](LISEZ-MOI-C1-C3.md),
[`scripts/portes-check.sh`](scripts/portes-check.sh) et
[`.github/workflows/portes.yml`](.github/workflows/portes.yml) — cinq fichiers
avec celui-ci, parce que c'est la première chose qu'un nouvel arrivant fait.

`clippy`, lui, est **bloquant** : `-D warnings`, sur chacun des trois profils
que `compose.yaml` construit.

---

## 3 · Ce qui remplace la revue par un pair

Il n'y a pas de second relecteur. Ce qui tient sa place, ce sont **seize
contrôles** qui rejouent chacun leur module et **refont eux-mêmes les
comptages** au lieu de croire le pilote.

```bash
bash scripts/portes-check.sh     # clippy -D warnings, un profil à la fois
bash scripts/d3-check.sh         # licences cohérentes + gitleaks (arbre ET historique)
bash scripts/orphelins-check.sh  # aucun membre du workspace sans dépendant ni porte d'entrée
```

Un contrôle de ce dépôt obéit à trois règles, et la troisième est la plus
importante :

| état | sens |
|---|---|
| `0` | mesuré, et vert |
| `1` | mesuré, et rouge |
| `3` | **non mesuré** — l'instrument n'a pas pu regarder |

Un contrôle en panne doit **dire qu'il est en panne**, et ne jamais emprunter le
vocabulaire du sujet qu'il mesure. Plusieurs portent en plus un **plancher
anti-vacuité** : ils échouent si le nombre de choses observées tombe sous un
minimum, parce qu'un contrôle qui n'observe rien rend `0` avec le même
enthousiasme qu'un contrôle qui observe tout.

Si tu ajoutes un contrôle, ajoute-lui aussi sa **contre-épreuve** : la
manipulation qui doit le faire tomber. Un garde incapable de tomber n'est pas un
garde, et ce dépôt en a déjà attrapé plusieurs de ce genre.

---

## 4 · Les commits

L'ancien document imposait [Conventional Commits](https://www.conventionalcommits.org/).
Mesure sur les 40 derniers commits : **0 le respecte**. La convention réelle est
descriptive, en français, et dit **ce qui a été mesuré** :

```
D4 · les portes s'exécutent, et elles ont trouvé cinq défauts invisibles sur macOS
D3 — la licence était l'intrus, et « 10 signalements » était deux fois faux
```

Le corps du message porte les chiffres et, quand il y en a une, la chose qui a
été **réfutée**. Un commit qui ne dit que ce qu'il ajoute est incomplet.

---

## 5 · Ce qui n'existe pas, et qu'il ne faut pas chercher

Nommé pour que personne ne perde une heure à en chercher la trace :

- pas de `CHANGELOG.md` — l'ancien « definition of done » demandait d'y écrire ;
- pas de `.github/CODEOWNERS` — archivé le 5 septembre 2026 avec 94 autres
  fichiers, dans `~/Downloads/vrai/EFE-arbre-2026-09-05/github/` ;
- pas de canal Slack, pas d'adresse `@efe.ai`, pas d'équipe ;
- pas de seuil de couverture de tests — les « ≥ 85 % de branches » de l'ancien
  document n'étaient mesurés nulle part ;
- pas de banc de non-régression obligatoire : `efe-bench-inference` existe mais
  aucune porte ne l'exige.

Voir [`SECURITY.md`](SECURITY.md) pour ce qui concerne la sécurité, et
[`docs/reports/`](docs/reports/) pour les feuilles de route.
