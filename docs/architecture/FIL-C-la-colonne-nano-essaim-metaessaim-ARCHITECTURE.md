# FIL-C — LA COLONNE : assembler nano-robots + essaim + méta-essaim (+ Fil B) pour qu'ils servent le plus

**Statut :** architecture / préparation (aucun code n'est modifié par ce document).
**Date :** 2026-07-17 · **Base :** monorepo `efe` @ HEAD `02eccdb` (Fil B complet B.0→B.4).
**Auteur :** R. Pekar — avec Claude Code (Opus 4.8).
**Règle cardinale :** NE JAMAIS SUR-VENDRE. Chaque affirmation est ancrée `fichier:ligne` quand elle
porte sur l'existant ; chaque capacité future est un jalon avec DoD, pas une promesse.

---

## 0 · Le but exact (et ce que Fil C EST / n'est PAS)

**But :** faire travailler ENSEMBLE les quatre choses que le dépôt sait déjà faire séparément —
les missions frontière (Couche 2 + Fil B), les nano-robots SLM, l'essaim latent (Couche 1) et le
méta-essaim (Couche 3) — dans UN pipeline où chacun fait ce qu'il sait prouver, relié par UN reçu.

**Fil C EST :** un ORGANISME à quatre étages — le cerveau (frontière) décide rarement et cher ;
les sens/membranes (nano-robots) traduisent et vérifient à chaque frontière de couche ; le muscle
(essaim/méta-essaim) exécute la charge lourde localement avec certificats ; la colonne vertébrale
(le reçu HMAC) scelle le tout en UNE racine re-vérifiable. Le livrable-roi : le **profil de
souveraineté par requête** — « cette réponse a coûté X appels frontière (egress, µ¢), Y hops SLM
locaux, Z pas latents locaux — tout scellé, re-vérifiable hors ligne ».

**Fil C N'EST PAS :** (a) rendre l'essaim « intelligent » — il calcule du latent sur un pipeline
FIXE (Φ = 1 ronde, population fixe, templates non câblés : garde-fous du doc vertical) ; (b) mettre
un LLM participant du méta-essaim (non-but assumé, Couche-3) ; (c) une agrégation robuste nouvelle
(β₁≈0.17 réfuté ; HB.3/HB.4 : ni vérification arithmétique ni synthèse ne rattrapent un écart de
modélisation) ; (d) un « nanobot de sécurité » — le verifier vérifie des claims TYPÉS, pas la
vérité du monde.

---

## 1 · Vérité-terrain : ce qui existe DÉJÀ (les coutures sont meilleures qu'attendu)

### 1.1 · La gateway exécute déjà les DEUX mondes, dans le MÊME store
- `POST /v1/swarm/run` exécute un essaim — sync ou job streamé (`routes/run.rs:1-9`), persiste via
  `StoredRun::from_outcome` (`store.rs:51`).
- Les missions frontière persistent via `StoredRun::from_mission` (`store.rs:71`).
- Le MÊME `GET /v1/swarm/verify/:id` re-prouve les deux (`routes/verify.rs`, prouvé pour les
  missions par `vote_e2e_seals_verifies_and_reports_r4`, `routes/llm.rs`).
⟹ **La moelle épinière existe** : un seul store, une seule crypto (SwarmChain HMAC par tenant,
`mission_graph.rs:196-199`), un seul vérificateur hors-ligne (`verify_run_offline`). Ce qui manque :
le LIEN entre un run de mission et un run d'essaim (aucune référence croisée aujourd'hui).

### 1.2 · La membrane typée existe (Fil B, complet)
- Trait `Reformulator` + moteurs Rule/SLM + estampilles par hop + raisons de dégradation
  (`reformulator_ext/`, B.0→B.4, commits 04759dd→6961f56).
- Le pattern d'extension de vocabulaire est ÉPROUVÉ : `SchemaId` +4 variants additifs, grammaire
  GBNF-union liant schema_id↔forme par construction, exemples JSON = contrat inter-crates testé
  (B.1). C'est exactement le pattern à réutiliser pour une 6ᵉ forme (la spec d'essaim).
- La discipline de résilience est ÉPROUVÉE : circuit à sonde unique `try_call` (B.2), deadline au
  reste de mission + entrée TIMEOUT synthétisée (B.4), `degrade_reason` scellé.

### 1.3 · Le muscle est réel et certifié
- Couche 1 : essaim latent servi par défaut (fusion efe-core dans efe-server), Φ déterministe
  (init_sigma=0, seed=compute_seed(spec.id) — M1.b), pureté par construction (centroïde porté par
  ELS — M1.c-bis).
- Couche 3 : participants efe-core RÉELS servis derrière `core-integration` (M1.d, c53a7f6) :
  manifest d'engine sur le fil (kappa/seed→Option), gate H1 fail-closed au boot, topologie fixe
  par défaut, `require_convergence→ABORTED`. Borne honnête MESURÉE : L̂_fixed=0.366 (le certificat
  analytique 0.17 est RÉFUTÉ — on ne cite que la mesure).
- Réserve à dire : le méta-essaim reste MOCK κ·x par défaut ; le vrai cœur = feature
  `core-integration`. La Colonne l'exigera et le reçu portera l'engine manifest (déjà sur le fil).

### 1.4 · Les sens existent, à moitié branchés
- `CoordinatorClients` porte DÉJÀ les slots : `analyzer`, `compressor`, `verifier`, `router`,
  `reformulator` (`coordinator.rs:37-62`) — tous `None` par défaut sur le chemin servi, SAUF le
  reformulator (câblé opt-in par Fil B.2, `frontier.rs:slm_clients_from`).
- Le pattern de wiring est donc PRÊT : câbler verifier/analyzer au chemin servi = répéter
  `slm_clients_from` (fonction pure, boot jamais cassé, warn de localité).

### 1.5 · Ce qui MANQUE (les quatre trous que Fil C bouche)
1. **Texte → spec d'essaim** : aucune forme canonique ne décrit un `RunRequest`
   (`dto.rs:34-46 : task + topology + budget + mode + width plafonnée serveur`) — le cerveau ne
   sait pas commander le muscle.
2. **Mission → essaim** : aucune mission n'exécute un run d'essaim (les deux mondes coexistent
   dans la gateway sans se parler).
3. **Reçu de reçus** : le reçu de mission ne peut pas référencer un run d'essaim (pas de nœud, pas
   de lien re-vérifiable).
4. **Résilience de couche** : aucun circuit/deadline/degrade_reason sur un appel mission→essaim.

---

## 2 · LE CONCEPT — « La Colonne » : un organisme, quatre étages, un reçu

```
        CERVEAU     missions frontière (Couche 2 + Fil B)     rare · cher · egress tracé
           │  plan (CanonicalPlan — existe)
        MEMBRANE    nano-robots (reformulator/analyzer/…)     typé · GBNF · estampillé
           │  spec (CanonicalSwarmTask — C.0)
        MUSCLE      essaim (C1) / méta-essaim (C3)            local · certifié · fail-closed
           │  outcome + certificats (L̂, convergence, manifest)
        MEMBRANE    verifier / reformulator (retour)          contre-vérifié · typé
           │  synthèse
        COLONNE     LE REÇU — une racine HMAC                 tout scellé · re-vérifiable
```

**Répartition honnête des compétences** (chacun ne fait QUE ce qu'il prouve) :

| Étage | Fait | Ne fait PAS | Preuve portée au reçu |
|---|---|---|---|
| Missions frontière | jugement, langage, plan | le travail de masse | hops Fil B + arité déclarée + R4 egress |
| Nano-robots | traduction typée, pré-mâchage, contre-vérification de claims | décider | AuditEntry par invocation (B.3) + engine + raison |
| Essaim / méta-essaim | charge latente locale, déterministe, bornée | comprendre le texte | manifest engine + config_hash + convergence/ABORTED + L̂ mesuré |
| Le reçu | sceller, localiser la falsification | prouver la CORRECTION (intégrité ≠ vérité) | racine par tenant, verify hors ligne |

**Les deux glues (rien de neuf à inventer, tout à relier) :**
1. **La membrane universelle** : Fil B généralisé — le reformulateur ne traduit plus seulement
   entre gros modèles, mais entre COUCHES (texte→spec d'essaim, outcome→rapport typé), toujours
   GBNF-contraint, toujours validé typé, toujours estampillé, avec repli déterministe.
2. **La moelle** : le store/verify UNIQUE déjà partagé — il suffit d'un nœud de lien
   (mission→swarm_run) pour que UNE racine re-vérifie la colonne entière.

**Pourquoi cet assemblage « sert le plus » :** c'est littéralement la thèse Kovant (SLM-swarms
pour procurement/conformité) rendue démontrable : maximiser le calcul LOCAL prouvé, minimiser et
TRACER l'egress frontière, et vendre le reçu — pas la magie. Le profil de souveraineté par requête
est l'argument que ni un wrapper GPT ni un pipeline latent seul ne peut donner.

---

## 3 · Les trois assemblages pesés (et la décision)

**(a) La Colonne descendante — mission commande essaim.** Le cerveau planifie (1 appel frontière),
la membrane traduit plan→spec (hop SLM scellé), le muscle exécute (N pas latents locaux), le
verifier contre-vérifie, la synthèse remonte. Valeur : la masse du travail devient locale et
prouvée ; le frontière ne sert qu'au jugement. Risques : la traduction plan→spec est le point dur
(bornes, largeur plafonnée serveur) — mais c'est EXACTEMENT le problème que B.1 a déjà résolu pour
5 formes. **RETENU comme cœur de Fil C.**

**(b) La Colonne montante — l'essaim escalade vers le cerveau.** Un run qui n'aboutit pas demande
de l'aide. DANGER de sur-vente : « l'essaim sait quand il ne sait pas » est FAUX — il ne sait
rien. Version honnête : le déclencheur est MÉCANIQUE et déjà servi (`require_convergence→ABORTED`,
M1.d ; gate H1) — sur ABORTED, UNE escalade bornée (budget/deadline) vers une mission debate,
scellée comme telle. **Réduit à un jalon tardif (C.4), déclencheur mécanique uniquement.**

**(c) Le panneau latéral — nano partout.** Généraliser la membrane à chaque frontière de couche.
Ce n'est pas un assemblage, c'est LA glue de (a). **Absorbé dans C.0/C.3.**

Rejetés explicitement : LLM-participant du méta-essaim (non-but) ; agrégation robuste nouvelle
(réfutée HB.3/HB.4) ; essaim-de-petits-modèles en masse (pesé et écarté au choix du Fil B —
en-dessous du plancher de capacité pour la traduction ; GBNF garantit la forme, pas le contenu).

---

## 4 · Garde-fous anti-sur-vente (contrat d'honnêteté de Fil C)

1. L'essaim ne comprend RIEN au texte — toute prose qui le suggère est interdite ; le reçu dit
   « pas latents », jamais « raisonnement ».
2. Le méta-essaim par défaut est MOCK : la Colonne refuse de se dire « colonne » sans
   `core-integration` + manifest engine vérifié (fail-closed, pattern H1 boot gate M1.d) — sinon
   dégradation estampillée `column_degraded: mock_engine`.
3. La traduction plan→spec ÉCHOUE proprement : validation typée + bornes serveur (width plafonnée
   `EFE_GATEWAY_MAX_WIDTH`) ; échec ⟹ la mission continue SANS muscle, estampillée
   `column_degraded: spec_invalid` — le service ne casse jamais (pattern B.4).
4. ABORTED n'est jamais maquillé : un run non convergé est scellé ABORTED dans la colonne, et
   c'est le SEUL déclencheur d'escalade.
5. Intégrité ≠ correction : le reçu prouve ce qui a tourné, pas que c'est vrai (modèle de
   confiance clé-détenteur, hérité tel quel de Couche 2/B.3).
6. Chiffres mesurés ou rien : « egress évité », « latence », « coût » n'entrent dans aucun doc
   avant le bench C.4.

---

## 5 · Les jalons (chacun sûr, mergeable seul, DoD vérifiable)

- **C.0 — La couture : `CanonicalSwarmTask` + membrane.** Une 6ᵉ forme canonique, miroir TYPÉ et
  BORNÉ de `RunRequest` (task JSON contraint, topologie énumérée, budget, width ≤ plafond serveur ;
  `dto.rs:34-46`) ; `SchemaId::SwarmTaskV1` additif (pattern B.1 : le validateur provenance la
  REFUSE explicitement) ; enveloppe GBNF +1 dans la grammaire-union + prompt + exemple JSON
  (contrat testé) ; `supports()` suit le profil. **Invariant : rien de servi ne change.**
  DoD : hors-ligne, un texte de tâche se traduit en `CanonicalSwarmTask` valide (faux llama) ; une
  spec hors-borne est REJETÉE typée ; le repli règle produit une spec minimale déterministe.

- **C.1 — Le muscle branché : mission `run_column` (essaim local).** Nouvelle mission efe-multi-llm :
  plan frontière → traduction C.0 (hop scellé) → exécution d'un run d'essaim par le MOTEUR EXISTANT
  de la gateway (le chemin de `routes/run.rs` refactorisé en fonction interne réutilisable — PAS un
  second exécuteur) → synthèse frontière optionnelle. Circuit dédié `"column"` (SlmCircuitBreaker,
  clé partagée, pattern B.2) + deadline au reste de mission (pattern B.4). Feature-gatée, off par
  défaut, boot jamais cassé. DoD : mission verte avec mocks ; essaim indisponible ⟹ mission
  réussit dégradée-estampillée ; le run d'essaim apparaît dans le store.

- **C.2 — La moelle : le reçu de reçus.** Le graphe de mission gagne un nœud Mode-A `swarm_run`
  committant `{swarm_run_id, provenance_root_essaim, config_hash, engine_manifest}` ; la réponse
  porte le **profil de souveraineté** `{frontier_calls, egress_microcents, slm_hops, latent_steps}`
  (compteurs M4 existants + hops B.3 + outcome) ; le vérificateur croisé suit le lien et re-prouve
  le run d'essaim référencé (deux racines, un verdict). Falsifier le lien casse la re-preuve,
  localisée. DoD : test e2e sceau+verify croisé + test de falsification du lien.

- **C.3 — L'échelle : le méta-essaim en muscle.** L'exécution locale C.1 devient un client gRPC
  vers efe-meta-swarm-server (participants réels `core-integration`, manifest vérifié fail-closed
  — M1.d) ; verifier nano post-run optionnel (slot `CoordinatorClients.verifier` déjà présent,
  wiring pattern `slm_clients_from`) ; raisons de colonne scellées :
  `{spec_invalid, swarm_aborted, meta_unreachable, mock_engine, deadline_exhausted}`. DoD : e2e
  contre un serveur méta local ; ABORTED scellé tel quel ; mock refusé sauf opt-in explicite dit.

- **C.4 — L'escalade mécanique + les chiffres.** Sur ABORTED uniquement : UNE escalade bornée vers
  une mission debate (budget/deadline dédiés), scellée `escalated_from: swarm_run_id` ; bench
  mesuré de la Colonne (latence, µ¢ egress réels vs baseline tout-frontière, pas latents) — les
  chiffres honnêtes pour le dossier Kovant, mesurés pas déclarés. DoD : rapport de bench reproductible.

Ordre imposé : C.0 (sûr) → C.1 (utile seul) → C.2 (la valeur-preuve) → C.3 (l'échelle) → C.4
(la boucle + les chiffres). Chaque jalon suit le rituel maison : vérité-terrain → code → batterie
complète → revue adverse → doc/mémoire.

---

## 6 · Risques & questions ouvertes

1. **Que met-on dans `task` ?** Le JSON libre de `RunRequest.task` est côté essaim un contrat
   flou ; C.0 doit choisir un SOUS-ENSEMBLE typé honnête (ce que l'essaim consomme réellement) —
   à vérité-terrain au démarrage de C.0 (lire le consommateur de `task` dans le SDK).
2. **In-proc vs gRPC d'abord ?** C.1 réutilise l'exécuteur in-proc de la gateway (zéro nouvelle
   surface réseau) ; le gRPC n'arrive qu'en C.3 quand le reçu sait déjà porter le lien. Évite de
   déboguer réseau et provenance en même temps.
3. **Double comptage du budget** : le run d'essaim a son budget (`BudgetDto`), la mission le sien —
   C.1 doit dériver l'un de l'autre (reste de mission → budget d'essaim), jamais deux plafonds
   indépendants.
4. **Le verifier vérifie QUOI ?** Ses claims typés existants — pas la qualité du latent. À cadrer
   en C.3 pour ne pas sur-vendre « contre-vérifié ».
5. **Nom de la 6ᵉ forme** : `SwarmTaskV1` dans le `SchemaId` nano (additif, refusé par le
   validateur provenance — décision identique à B.1 §11.3).

---

## 7 · Definition of Done globale

- Une requête « colonne » servie produit : 1 plan frontière scellé + 1 hop de traduction estampillé
  + 1 run d'essaim (réel, manifest engine) + 1 reçu unique dont la re-preuve croisée passe hors
  ligne — et sa version dégradée (muscle absent) réussit, estampillée, jamais silencieuse.
- Le profil de souveraineté est exact (compteurs M4 + hops + pas latents) et re-dérivable du reçu.
- Aucune prose, aucun log, aucun champ ne prête à l'essaim une compréhension qu'il n'a pas.
