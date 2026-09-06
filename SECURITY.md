# Sécurité — Scella

> **État** : dépôt **privé**, aucun service déployé, aucune équipe.
> **Dernière mesure** : 5 septembre 2026.

Ce fichier remplace une politique de sécurité qui promettait quatre adresses
courriel chez un domaine, une clé PGP, un programme de primes HackerOne, un
formulaire d'avis sur un dépôt qui n'est pas celui-ci, des délais de correction
contractuels (7 jours en critique, 30 en élevé), un *safe harbor*, un tableau
d'honneur, et sept documents de conformité **dont aucun n'existe**.

Elle s'adressait à *« security researchers, customers, regulators »*. **Ce dépôt
est privé** : ce public ne peut pas la lire. Une politique adressée à des gens
qui n'y ont pas accès n'est pas une politique, c'est un décor — et son danger
est réel : un chercheur qui aurait écrit à l'adresse annoncée aurait cru avoir
divulgué de façon responsable.

---

## 1 · Signaler une vulnérabilité

Tant que le dépôt est privé, **il n'y a pas de canal public, et nous n'en
inventons pas un**. Le seul chemin est le contact direct avec la personne qui
détient le dépôt.

Le formulaire d'avis privé de GitHub — le canal qu'un dépôt de ce genre devrait
offrir — **n'est pas disponible sur un dépôt privé** (l'API rend `404`, vérifié
le 5 septembre 2026). C'est le premier geste à faire le jour où le dépôt
s'ouvre, avant toute annonce.

**Aucun délai de correction n'est promis.** Il n'y a pas d'équipe pour le tenir.

---

## 2 · Licence — une contradiction, et sa correction

L'ancien fichier se terminait par : *« EFE is open source under the Apache
License 2.0 »*.

**C'est faux, et cette phrase est retirée.** [`LICENSE`](LICENSE) dit
`All rights reserved` ; le périmètre est **propriétaire**. Une seule exception,
et elle est écrite dans le fichier de licence : `crates/efe-nano-sdk/**` est
sous **MIT**, dont le texte est dans [`LICENSE-MIT`](LICENSE-MIT).

Cette incohérence est exactement celle que l'étape **D3** avait pour objet, et
elle avait survécu ici. `scripts/d3-check.sh` refuse désormais toute divergence
entre les licences déclarées par les manifestes et ce que le fichier de licence
accorde — mais il lit les manifestes, pas la prose. Ce paragraphe-ci n'est gardé
par rien d'autre que sa lecture.

---

## 3 · Ce qui est vrai, et où le vérifier

Sans certification, sans audit, sans « 97 % de la ligne de base NIST ». Ce qui
suit est du code qu'on peut ouvrir :

| propriété | où |
|---|---|
| chaîne HMAC scellée, ancre re-vérifiable, troncature détectable | `crates/verify-mini/`, `crates/provenance-verify/` |
| refus de démarrer la frontière sans `postgres`, `oidc` et `pii` — `exit 78` (`EX_CONFIG`) | [`profile.rs:926`](crates/efe-swarm-gateway/src/profile.rs), [`main.rs:53`](crates/efe-swarm-gateway/src/main.rs) |
| analyseur d'autorité **unique** (5 copies réduites à 1 ; `127.0.0.1:8080@evil.com` coupait TLS vers `evil.com` sur 875 formes) | [`scella-authority`](crates/scella-authority/src/lib.rs) |
| les **trois** destinations d'egress **configurables et validées** — `https` partout, `http` seulement en local, jamais de `userinfo` (la destination est scellée dans le reçu) | [`common.rs`](crates/efe-coordinator/src/llm/providers/common.rs) ; `EFE_ANTHROPIC_URL`, `EFE_OPENAI_BASE_URL`, `EFE_GEMINI_BASE_URL` au [registre](crates/efe-swarm-gateway/src/profile.rs) |
| le reçu annonce la destination **réelle**, jamais la constante | [`endpoint_configurable.rs`](crates/efe-coordinator/tests/endpoint_configurable.rs) |
| une destination **refusée** ne fait pas disparaître un fournisseur en silence : le boot le crie | [`frontier.rs`](crates/efe-swarm-gateway/src/frontier.rs) |
| les **trois** allers-retours frontière joués **sur une vraie socket** contre un faux fournisseur, en-têtes vérifiés depuis l'extérieur | [`aller_retour_frontiere.rs`](crates/efe-coordinator/tests/aller_retour_frontiere.rs) |
| le store durable **s'énumère** — le primitif sans lequel aucune sauvegarde n'est possible ; trait SÉPARÉ de `RunStore` pour qu'aucune route ne puisse lire entre tenants | [`inventaire.rs`](crates/efe-swarm-gateway/src/inventaire.rs) |
| sauvegarde **éprouvée par destruction** : table effacée, reçu disparu (404), import, reçu revenu et re-prouvé | [`p1-check.sh`](scripts/p1-check.sh) §6 |
| effacement cryptographique (la chaîne scelle le chiffré) | [`provenance-dag/src/shred.rs`](crates/provenance-dag/src/shred.rs) |
| recherche de secrets à chaque `push`, **arbre et historique** | [`scripts/d3-check.sh`](scripts/d3-check.sh), joué par `.github/workflows/portes.yml` |

**Outillé n'est pas conforme.** `crates/reg-evidence/` produit des artefacts qui
répondent à des exigences nommées (AI Act art. 12, CSDDD/LkSG, RGPD). Produire
l'artefact n'est pas être conforme, et ce dépôt ne prétend ni l'un ni l'autre au
nom d'un tiers.

---

## 4 · Ce qui manque, et qui n'était écrit nulle part

Le plus utile de ce fichier :

- **Sauvegarde : faite, et ÉPROUVÉE par destruction** — c'était « zéro » jusqu'au
  6 septembre 2026, et la cause était plus basse qu'un script manquant :
  `RunStore` n'avait **aucune méthode qui énumère**. Un registre à conserver dix
  ans qu'aucun code ne sait parcourir ne peut être ni exporté, ni sauvegardé, ni
  migré. `--exporter-runs` / `--importer-runs` le parcourent désormais, et
  `p1-check.sh` §6 **détruit la table** puis remonte : le reçu revient et se
  re-prouve. Rejouer l'import n'écrase rien (write-once).
  ⚠ Ce qui reste : **rien ne PLANIFIE** cette sauvegarde. La capacité existe, la
  cadence n'existe pas — c'est une décision d'exploitation, pas du code.
- **Rotation de clé : zéro.** `crates/scella-conservation/` sait produire des
  générations de clé ; rien ne les fait tourner.
- **Le jeton porteur reste un secret partagé** ([`audit-console/src/session.rs`](crates/audit-console/src/session.rs)).
  Une vraie non-répudiation demanderait que l'officier **signe** le tuple de
  l'acte. C'est nommé, et non produit.
- **La frontière ne démarre que sous contrainte, et c'est voulu.** Le garde du
  point 3 exige `postgres` + `oidc` + `pii` réunis, sinon `exit 78`.
  `scripts/p1-check.sh` la démarre pour de vrai contre un Postgres réel, la tue,
  la redémarre et re-prouve le reçu. Ce qui n'a jamais eu lieu, c'est un appel
  vers un **vrai** fournisseur : il faut une clé, et elle coûte de l'argent.
  L'aller-retour lui-même est désormais joué contre un faux fournisseur local.
- **M3 sous `oidc`** : les deux tests qui portent son critère ne tournent que
  sous `frontier-llm` *sans* `oidc`. L'argument qui rendrait ce trou acceptable
  est écrit dans `routes/llm.rs` comme une **revendication**, pas comme une
  mesure.

---

## 5 · Ce qu'il faudra avant qu'une vraie politique existe

Dans cet ordre, et aucun ne s'invente à l'avance :

1. un service qui tourne, donc une surface à attaquer ;
2. une adresse que quelqu'un relève, et quelqu'un pour la relever ;
3. l'ouverture du dépôt, et avec elle le formulaire d'avis privé de GitHub ;
4. alors seulement des délais — parce qu'un délai est une promesse, et qu'une
   promesse sans personne derrière est ce que ce fichier remplace.
