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
| **l'intégrité ne dit rien de la complétude** — un bundle tronqué se re-prouve PARFAITEMENT (`verify-mini` seul rend `0`) ; seule l'ancre, publiée hors du bundle, l'attrape, et par un simple COMPTAGE | [`banc-seuil.sh`](scripts/banc-seuil.sh) § 7, joué à chaque `push` |
| le discriminant bruyant/silencieux de la frontière, **isolé en fonction** pour être éprouvé — on crie sur un acte de l'opérateur, jamais sur une absence | [`frontier.rs`](crates/efe-swarm-gateway/src/frontier.rs) `bruyant()`, [`auth.rs`](crates/efe-coordinator/src/llm/providers/auth.rs) |
| refus de démarrer la frontière sans `postgres`, `oidc` et `pii` — `exit 78` (`EX_CONFIG`) | [`profile.rs:926`](crates/efe-swarm-gateway/src/profile.rs), [`main.rs:53`](crates/efe-swarm-gateway/src/main.rs) |
| analyseur d'autorité **unique** (5 copies réduites à 1 ; `127.0.0.1:8080@evil.com` coupait TLS vers `evil.com` sur 875 formes) | [`scella-authority`](crates/scella-authority/src/lib.rs) |
| les **quatre** destinations d'egress **validées** — y compris `EFE_LOCAL_SLM_URL`, qui n'avait aucun contrôle et dont le nom dit « local » — `https` partout, `http` seulement en local, jamais de `userinfo` (la destination est scellée dans le reçu) | [`common.rs`](crates/efe-coordinator/src/llm/providers/common.rs) ; `EFE_ANTHROPIC_URL`, `EFE_OPENAI_BASE_URL`, `EFE_GEMINI_BASE_URL` au [registre](crates/efe-swarm-gateway/src/profile.rs) |
| le reformulateur SLM **refuse le clair distant** ; le distant chiffré reste accepté **avec avertissement** — la frontière est fixée par des tests, pas par une ligne de journal | [`frontier.rs`](crates/efe-swarm-gateway/src/frontier.rs) |
| **ES256 et RS256** acceptés, l'algorithme **dérivé de la clé** et jamais de l'en-tête du jeton — un HS256 signé avec la clé publique est refusé | [`oidc.rs`](crates/efe-swarm-gateway/src/oidc.rs), [`p1-check.sh`](scripts/p1-check.sh) §7 |
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
- **Rotation de clé : la capacité existe, la CADENCE n'existe pas** — exactement
  la même forme que la sauvegarde ci-dessus, et cette ligne disait « zéro » à tort.
  `scella-conserver tourner` fait tourner une génération, `p2_rotation` éprouve
  qu'une archive **antérieure** se restaure encore après, et `p2-check.sh` rejoue
  les deux. Ce qui est réellement à zéro : **rien ne déclenche** cette rotation, et
  aucun document ne dit à quelle fréquence elle devrait avoir lieu. C'est une
  décision d'exploitation, pas du code.
- **Le jeton porteur reste un secret partagé** ([`audit-console/src/session.rs`](crates/audit-console/src/session.rs)).
  Une vraie non-répudiation demanderait que l'officier **signe** le tuple de
  l'acte. C'est nommé, et non produit.
- **La frontière ne démarre que sous contrainte, et c'est voulu.** Le garde du
  point 3 exige `postgres` + `oidc` + `pii` réunis, sinon `exit 78`.
  `scripts/p1-check.sh` la démarre pour de vrai contre un Postgres réel, la tue,
  la redémarre et re-prouve le reçu. Ce qui n'a jamais eu lieu, c'est un appel
  vers un **vrai** fournisseur : il faut une clé, et elle coûte de l'argent.
  L'aller-retour lui-même est désormais joué contre un faux fournisseur local.
- **M3 sous `oidc` : FERMÉ** le 6 septembre 2026. Les deux tests qui portent son
  critère étaient `#[cfg(not(feature = "oidc"))]` — ils ne tournaient donc sous
  **aucun** jeu de features servi. La cause n'était pas dans M3 : la paire de clés
  ES256 de test vivait dans un `mod tests` privé, donc les tests n'avaient d'autre
  choix que l'extracteur de développement. Fixture partagée, vrai jeton d'audience
  `efe-llm`, et `p1-check.sh` §7 joue désormais **les deux** jeux de features.
- **La console SCELLE la supervision humaine, mais ne la PERSISTE pas** —
  trouvé le 6 septembre 2026 en construisant `scripts/banc-registre.sh`. Un
  officier qui scelle une revue voit `303`, puis la revue sur la page : elle est
  bien scellée. Le fichier de records, lui, reste **identique à l'octet près**
  (mesuré : 6 lignes / 2 125 octets avant, 6 lignes / 2 125 octets après un
  scellement réussi) — `Console::rehydrate` lit `EFE_CONSOLE_RECORDS`, rien ne
  le réécrit. La revue vit en mémoire et disparaît au redémarrage.
  Un registre de conformité dont la supervision scellée s'évapore n'est pas un
  registre ; c'est une session. Le banc en fait sa troisième épreuve, pour que
  ce soit constaté plutôt que découvert.
- **`EFE_CONSOLE_RECORDS` n'était alimentée par RIEN** — la variable qui
  réhydrate un tenant depuis un journal scellé est déclarée au registre de
  profil, lue par `main.rs`, et **zéro** script et **zéro** test ne l'écrivaient
  (mesuré par `grep -rl` sur `scripts/` et `crates/*/tests/`). M7 et M8
  démarraient bien la console et l'interrogeaient par HTTP, mais sur un tenant
  provisionné VIDE : le chemin de réhydratation compilait sans avoir jamais
  servi. `console-semence` l'alimente désormais avec la **même** dérivation de
  clé que le serveur, et le banc joue la chaîne entière — sceau réel, fichier,
  réhydratation, HTTP, badge. Le badge rouge avait son test unitaire
  (`badge_is_red_fail_loud_when_broken`, sur une enveloppe synthétique) ; ce
  qui n'existait pas, c'est un fichier réellement altéré traversant tout.
- **La frontière affirmait « ce n'est pas une clé manquante » sur une clé
  manquante** — trouvé le 6 septembre 2026 en construisant `scripts/banc-seuil.sh`.
  `frontier::enregistrer` distingue depuis toujours deux cas, et la distinction est
  juste : on CRIE sur une valeur fournie puis rejetée (l'opérateur a agi, il doit
  l'apprendre au boot), on se TAIT sur une absence (déploiement qui n'utilise pas
  ce fournisseur). Mais `ApiKey::from_env` typait **les trois** situations —
  variable absente, valeur vide, préfixe non conforme — en un seul
  `CoordError::Config`. Le bras bruyant s'exécutait donc sur une absence, et
  imprimait **trois fois** une phrase fausse, sur la posture la plus fréquente qui
  soit : frontière activée sans clé fournisseur.
  Un discriminant juste, rendu faux par le type de l'erreur qu'il discriminait —
  et qu'aucun test ne pouvait viser, parce qu'il vivait dans les bras d'un `match`
  plutôt que dans une fonction. Corrigé par une variante `CoordError::ConfigAbsent`
  et l'extraction du prédicat `bruyant()`, tous deux sous test — dont un test
  d'anti-vacuité qui refuse que les deux cas se remettent à rendre la même chose.
- **Les tests du module `frontier` ne tournaient nulle part** — ils n'existent que
  sous `frontier-llm`, or `cargo test --locked --workspace` (ce que joue la CI)
  compile cette feature à ZÉRO, et `p1-check.sh`, qui la joue, exige un démon
  Docker que le runner n'a pas. Même forme que M3 sous `oidc`, fermé le même mois.
  `banc-seuil.sh --production` les joue désormais, et la CI le joue.

## 5 · Ce qu'il faudra avant qu'une vraie politique existe

Dans cet ordre, et aucun ne s'invente à l'avance :

1. un service qui tourne, donc une surface à attaquer ;
2. une adresse que quelqu'un relève, et quelqu'un pour la relever ;
3. l'ouverture du dépôt, et avec elle le formulaire d'avis privé de GitHub ;
4. alors seulement des délais — parce qu'un délai est une promesse, et qu'une
   promesse sans personne derrière est ce que ce fichier remplace.
