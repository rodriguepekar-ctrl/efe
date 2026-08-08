# Couche 3 · M1 — Vrais participants (architecture, préparation complète)

> Document d'architecture · 2026-07-13 · Issu d'une reconnaissance chirurgicale (lentilles moteur/seam/certification + **1 passe adverse soundness**) + lecture directe du seam mathématique. Prolonge [`COUCHE3-consensus-latent-meta-essaim-ARCHITECTURE.md`](./COUCHE3-consensus-latent-meta-essaim-ARCHITECTURE.md) §8 (jalon M1).
> Contexte : **dowry Kovant**. Règle absolue : **ne jamais sur-vendre** — et M1 en cache plusieurs.

---

## 0. Verdict & principe

Le jalon M1 « remplacer le `MockParticipant` synthétique par de vrais essaims efe-core » **paraît** un simple flip de feature (`core-integration`). **La reconnaissance a démonté cette illusion sur cinq axes**, dont un qui **renverse l'hypothèse de départ** :

1. **Le vrai danger n'est PAS l'attention.** Au round 1, la fusion efe-core opère sur un latent **seedé par les priors de template** (indépendant de `x`) ; `x` n'entre dans Φ **que** par `beta_mix` à poids **β₁ ≈ 0.17**. **Intra-cellule, Φ est donc analytiquement une ~0.17-CONTRACTION**, pas un opérateur expansif. Le vrai ennemi de H1 est la **discontinuité du routage génésique** (la classe de `x` → nombre entier d'agents → topologie **en escalier** → Φ **discontinu** aux frontières, Lipschitz localement non borné).
2. **La non-convergence est SILENCIEUSE.** Si Φ n'est pas contractant, la boucle rend `MaxItersHit` → un RPC **« réussi »**, jamais une erreur ; les projections de sûreté **bornent** les états, donc pas de `NaN`/divergence visible : le client reçoit un « consensus » = dernier itéré **non convergé**.
3. **Φ n'est pas déterministe.** Chaque `execute()` tire un `SwarmId = UUIDv4` qui seed le bruit genesis (`init_sigma`) ⟹ Φ(x) **varie d'un appel à l'autre**. Le mock, lui, est déterministe ; le descripteur sur le fil **annonce** `determinism` — il mentira après M1.
4. **Même câblé, ce ne sont pas des « SLM ».** Le seul câblage réel (`efe-server::mode1`) coordonne la fusion **NUMÉRIQUE** Mode1 sur des templates **STUB** : **aucun poids de LM, aucun tokenizer** ; `problem_embedding` est **écrasé** par `current` donc **aucun embedder** n'est sur le chemin du pas. Le sidecar chargeur-de-poids est un placeholder `pause:3.9` désactivé.
5. **Le garde-fou H1 est mort.** `verify_non_expansive()` renvoie `Stub` sur les DEUX participants ; la garde H1 (`coupled.rs:144`) ne peut **jamais** échouer.

> **Avertissement central** : **M1 fait naïvement est une RÉGRESSION DE SÛRETÉ par rapport au mock** — le mock `Φ(x)=κ·x` était **prouvablement contractant** ; un vrai participant efe-core, sans garde-fou, peut ne **jamais** converger, **en silence**, sur un point-fixe parasite manipulable. M1 = câbler le pont **ET** rétablir un garde-fou H1 réel **ET** cadrer honnêtement le périmètre.

---

## 1. La formule et le vrai point de rupture (le cœur mathématique)

Le pas d'un participant (trait `SwarmParticipant`, `participants/mod.rs:52-64`) :

$$ x_{t+1} = P_{C_m}\Big( (1-\alpha-\gamma)\,x_t + \alpha\,\Phi_m(x_t) + \gamma\,\mathrm{pull} \Big) $$

`mix_coupled` (`efe-core/swarm/coupled.rs:76`) = la combinaison convexe (`α+γ<1` imposé) ; `P_{C_m}` = projection de sûreté participant (fermement non-expansive). La **borne de Lipschitz composite** (pull fixé) :

$$ L \le \underbrace{1}_{P_{C_m}} \cdot \big[(1-\alpha-\gamma) + \alpha\,L_\Phi\big] \quad\Rightarrow\quad L_\Phi \le 1 \Rightarrow L \le 1-\gamma < 1 \text{ (contraction)}. $$

**Toute la convergence de type Krasnoselskii-Mann dépend de H1 : `L_Φ ≤ 1`.** Le mock l'assure trivialement (`Φ(x)=κ·x`, `L_Φ=κ<1`). La question de M1 : **le Φ d'un vrai essaim efe-core l'assure-t-il ?**

### 1.1 Le renversement : Φ = Fuse₁, et `x` n'y entre qu'à ≈0.17

`SwarmController::step_with_pull` (`controller.rs:349-368`) calcule `Φ(x)` en **UN round** : clone du `base_spec`, `spec.problem_embedding = current`, `max_rounds = 1`, `execute()`, `phi_x = resp.output.as_raw()`, puis `mix_coupled`. Dans ce round (`mvp_cpu.rs:148-237`), l'ordre des kernels est **scores → softmax → aggregate → beta_mix → project → clip**, et :

- le latent est **seedé par les priors de template** + bruit `init_sigma` (`spawner.rs:356-384`), **PAS par `x`** ⟹ l'**attention/aggregate opèrent sur les priors, indépendants de `x`** ;
- `x` entre **uniquement** par `beta_mix` (`mvp_cpu.rs:208-213`) avec `β₁ = problem_anchor_init·e^{-decay} ≈ 0.2·e^{-0.15} ≈ 0.172` (`schedule.rs:20`, `config.rs:69`) ;
- `beta_mix` (Lipschitz β), `project` vers l'enveloppe des anchors (`(1-α)I+αΠ`, non-expansif), `clip` coordonnée (projection boîte, non-expansif), extraction du centroïde (moyenne, non-expansif) sont **tous non-expansifs**.

**Conséquence** : **intra-cellule, `L_Φ ≤ β₁ ≈ 0.17`** — Φ est analytiquement une **contraction**. La prémisse « attention expansive ⟹ Φ expansif » est **FAUSSE** pour ce câblage 1-round. *(Ne pas gaspiller l'effort à « dompter l'attention ».)*

### 1.2 Le vrai ennemi : la discontinuité du routage

`GenesisEngine::spawn` (`engine.rs:42-70`) : `meta_analyzer.decompose(x)` **classe `x`** par cosine-similarity + softmax vs 5 exemplaires de classe (`meta_analyzer.rs:150-175`) → `router.select` (distribution de templates) → `cardinality.estimate` (**nombre ENTIER d'agents** `N(x)`) → `spawner.spawn`. **`N(x)` et la composition de templates sont des fonctions EN ESCALIER de `x`** ⟹ **Φ est discontinu aux frontières de cellule**, où `‖Φ(x)−Φ(y)‖/‖x−y‖` peut **exploser**. C'est là — et seulement là — que H1 casse.

---

## 2. L'état des lieux (RÉEL / STUB / NON CÂBLÉ)

| Élément | Statut | Preuve |
|---|---|---|
| Pont réel `LocalParticipant` (efe-meta-swarm) | **codé, jamais exécuté** | `participants/local.rs` (delegate `runtime.block_on(controller.step_with_pull)` + `safety.project`) ; **aucun test e2e** (le `tests/integration_local_participant.rs` référencé `local.rs:264` **n'existe pas**) |
| `verify_non_expansive()` | **Stub** (les deux participants) | `local.rs:229`, `mock.rs:157` → `Stub` ; `certificate.rs:35` `Stub=>true` ⟹ garde H1 morte |
| Participant **servi** | **MockParticipant** (κ·x) | `efe-meta-swarm-server/participant/local.rs:91` `from_manifest_row` → `MockParticipant::with_contraction` ; **`core-integration` = 0 hit** dans le serveur |
| `Φ` semantics | **couplage faible ≈0.17, priors-dominé** | `x` n'entre qu'à β₁ (§1.1) ⟹ « les participants raisonnent ensemble sur le problème » est **largement fictif** |
| Manifeste participant | **mock-shaped** | `schema.rs:43-50` `{id,dim,kappa,seed}` — **aucun** champ de `SwarmSpec` (population/templates, convergence, alignment_cone, budget, max_rounds) |
| Builder `manifeste→SwarmController` | **absent** | n'existe QUE dans `efe-server::mode1:1149` (`build_mvp_backend`), qui rend un `Arc<dyn ExecuteBackend>`, **pas** un `SwarmController` |
| Déterminisme de Φ | **cassé** | `controller.rs:200` `SwarmId::new()=UUIDv4` par pas → seed genesis variable |
| « vrais SLM » | **stub** | `mode1.rs:1172` `StubTemplateRegistry` + `MvpCpuFusion` numérique ; sidecar poids = `pause:3.9` désactivé (`values.yaml:299`) |

---

## 3. Le garde-fou de contraction — les DEUX paliers (le make-or-break)

La question « comment rendre H1 vraie pour un vrai participant » a **deux réponses**, à ne jamais confondre dans le pitch :

### Palier-1 — non-expansivité MESURÉE (canary, livrable M1)
Un certificat **empirique** `NonExpansiveCertificate::Empirical{n_samples, seed, radius, l_hat}`, `is_valid() = l_hat < 1 − margin`, alimenté par un échantillonnage sur `LocalParticipant` via un nouvel accesseur **`controller.phi_once(x)`** (extraction de `controller.rs:349-366`).
> **Honnêteté impérative** : un `L̂` par échantillonnage de paires est une **BORNE INFÉRIEURE** (falsification-only) — il **détecte** l'expansivité grossière (`L̂ ≥ 1 ⟹ échec`), il ne la **certifie jamais**. Pire, ici il est **biaisé-optimiste** : des paires uniformes tombent presque sûrement dans la **même cellule** de routage (§1.2) où `L≈0.17`, et **n'enjambent quasi jamais une frontière** (mesure nulle) ⟹ `L̂` lira `≪1` alors que Φ est discontinu. **Donc l'échantillonnage DOIT être adversarial** : perturber `x` autour des exemplaires de `meta_analyzer`, montée de gradient/différences finies sur le ratio. Étiquette : « **non-expansivité property-testée, NON certifiée** ». À livrer comme **canary runtime + gate fail-closed**, jamais comme garantie.

### Palier-2 — non-expansivité PROUVÉE (la seule à revendiquer « prouvé »)
**Contraindre la topologie** : construire le `SwarmController` du participant avec un **`StaticRouter` + cardinalité fixe + masque figé**, indépendants de `x`. Alors la seule dépendance en `x` restante est `beta_mix` (continu) ⟹ **Φ devient continu et analytiquement non-expansif de module `≤ β₁ < 1`** par composition (§1.1) ⟹ un **certificat de contraction RÉEL** (forme fermée, pas empirique) ⟹ **H1 prouvée par construction**.

> **Décision M1** : viser le **Palier-2** (topologie fixe) pour obtenir une garantie **analytique** — c'est peu coûteux (désactiver le routage dynamique) et ça transforme la garde H1 morte en preuve. **Livrer le canary empirique EN PLUS** (Palier-1) pour attraper toute dérive. Le « Palier-1 seul » est insuffisant (falsification biaisée) ; le mélange (fixe + canary) est le seuil honnête.

---

## 4. Le trou de non-convergence silencieuse (blocker de soundness)

`run()` rend **`Ok(MaxItersHit)`** (`coupled.rs:180-189`), mappé en **métrique** `Convergence::MaxIters` sans `Err` gRPC (`iterate.rs:123-126`, `service.rs:331`). Le code porte même le commentaire assumé « *Phase-2's algorithm is contractive ; this counter is expected to stay at 0 in production* » (`iterate.rs:146`) — **hypothèse non vérifiée**. Combiné au `Stub` (garde H1 morte), un déploiement « vrais participants » peut tourner en prod **sans jamais converger et sans aucune erreur**, et rendre un **point-fixe parasite** (Brouwer : la map couplée envoie une boîte convexe compacte dans elle-même ⟹ ≥1 point-fixe, pas forcément unique ni juste) **manipulable** par un essaim adverse (la moyenne uniforme `UniformMetaOp` n'a **aucune** résistance aux outliers).

**Correctif (fail-closed, blocker)** : en profil « contraction requise », `stop_reason != Converged` ⟹ **RPC en échec** (`FAILED_PRECONDITION`), **jamais** étiqueté « consensus » ; exposer le `final_residual` ; et rendre la garde H1 (`coupled.rs:144`) **capable d'échouer** (`Stub.is_valid()=false` en strict).

> **AMENDÉ par l'addendum du 2026-08-08** (deux points, lire avant de citer ce §) : ce n'est plus un « profil » à activer — c'est le **défaut** dès qu'un participant efe-core est servi (`Manifest::require_convergence`) ; et le code d'erreur est **`ABORTED`**, pas `FAILED_PRECONDITION` (que `classify_grpc_status` range en `safety_violation`, ce qui confondrait une non-convergence avec une sortie de boîte).

---

## 5. Le trou de déterminisme (high)

`execute()` tire `SwarmId::new() = UUIDv4` (`controller.rs:200`) qui seed le bruit genesis (`spawner.rs:220`, `spawn_kernel.rs:16`, `sigma=init_sigma≈0.02`) ⟹ **Φ(x) non reproductible bit-à-bit**. Or le descripteur sur le fil (`state.rs:161`) annonce `features=[determinism, step_index]` + `kappa`/`seed` — concepts **mock**. **Après M1, le fil ment.**

**Correctif** : dériver `swarm_id`/`seed` **déterministiquement** depuis `(participant.seed ⊕ step_index)` + forcer `init_sigma = 0` sur le chemin servi + auditer l'ordre des réductions flottantes rayon (`coupled.rs:84-114`). **OU** retirer la revendication `determinism` et documenter « participant stochastique ». Remplacer `kappa`/`seed` sur le proto par un **`config_hash`** du controller (aligne avec `NonExpansivenessCertificate.config_hash`, COUCHE3 §5). *(Un vrai SLM — sampling + réductions GPU non-associatives — rendrait le déterminisme encore plus hors d'atteinte : à graver avant tout pitch.)*

---

## 6. Le câblage serveur (l'effort « L »)

Le seam d'exécution est **à moitié prêt** : le handler `Step` tourne déjà en `spawn_blocking` (`participant/service.rs:152`) — exactement ce qu'exige le `block_on` du pont réel. Reste :

1. **`efe-core::swarm::phi_once(x) -> Vec<f32>`** : extraire les lignes `controller.rs:349-366` en accesseur public (mesure de `L_Φ` isolée + réutilisation).
2. **`efe-core::swarm::build_latent_controller(spec, config) -> SwarmController`** : un builder **réutilisable** (pas via `efe-server::mode1`, trop lourd — embedder/pools/catalog — et qui rend un `ExecuteBackend`). Il câble genesis (**`StaticRouter`** pour le Palier-2) / fusion (`MvpCpuFusion`) / dissolver / alignment sur `HostBackend`.
3. **Serveur** : activer `efe-meta-swarm = { features = ["core-integration"] }` dans `efe-meta-swarm-server/Cargo.toml` ; faire envelopper au type serveur `participant::LocalParticipant`, sous `#[cfg(feature="core-integration")]`, le pont réel `efe_meta_swarm::LocalParticipant` **en préservant la surface** (`id/dim/step/reset/current_state/step_index_last/descriptor`) dont dépendent `state.rs`/`service.rs`/`reload.rs`. Injecter le `runtime::Handle` (déjà dans `async_main`). *(L'invariant « efe-server byte-identique » du gate Phase-1 reste tenu : `efe-server` est un crate séparé.)*
4. **Manifeste** : ajouter un bloc versionné `participant.controller { templates/population, convergence, alignment_cone.anchors, budget, max_rounds, router: static }` **OU** référencer un profil Mode1 nommé ; câbler `meta.safety` (`Box/Poly/Hull`) → `SafetySet` (existent déjà) ; **déprécier `kappa`/`seed`** en champs descriptifs et faire échouer `validate_manifest` (fail-closed) si `core-integration` est actif et que les champs `SwarmSpec` manquent.

**Déploiement** : **1 Deployment+Service par participant** (déjà en place, `replicas=1` verrouillé), **efe-core IN-PROCESS**, **aucun sidecar requis** (Mode1 est numérique). Ne **pas** revendiquer « hot-reload de poids » ni « sidecar SLM » (`pause:3.9` placeholder).

---

## 7. Honnêteté & sur-ventes (cadrage Kovant)

| BANNIR | REMPLACER par |
|---|---|
| « le méta-essaim coordonne de **vrais SLM** / modèles de langage » | « coordonne de **vrais essaims latents efe-core** (fusion Mode1) » — la partie EFE est honnête |
| « les participants **raisonnent ensemble** sur le problème » | « couplage latent **faible (≈0.17)**, priors-dominé au round 1 » |
| « **convergence prouvée/garantie** » | « contraction **analytique** (module ≤ β₁) **sous topologie fixe** (Palier-2), sinon **property-testée non certifiée** (canary) » |
| « consensus **byzantin-robuste / tolérant aux fautes** » | « agrégation **vérifiable**, **aucune** résistance aux outliers (moyenne uniforme) » |
| « participant **déterministe** » (sur le fil) | soit rendu vrai (§5), soit retiré → « stochastique » |

> **« Vrais SLM »** est une **piste séparée NON livrée** : il faudrait des **poids de modèle + tokenizer sur le chemin du pas**, ce qui n'existe pas (`problem_embedding` écrasé, templates stub, sidecar désactivé).

---

## 8. Plan d'implémentation & DoD

**Sous-jalons** (l'ordre importe : le garde-fou AVANT la démo) :

| Sous-jalon | Livrable | Effort |
|---|---|---|
| **M1.a — seam d'exécution** | `phi_once(x)` + `build_latent_controller` (hors efe-server) + wiring `LocalParticipant` sous `core-integration` + **test d'intégration** (vrai `SwarmController`, N pas Step-sur-gRPC, **contenance** dans la boîte) | **L** |
| **M1.b — topologie fixe (Palier-2)** | `StaticRouter` + cardinalité fixe + masque figé ⟹ Φ continu, **certificat de contraction analytique** (`≤ β₁`) | **M** |
| **M1.c — garde-fou H1** | `verify_non_expansive` réel (`Empirical`, échantillonnage **adversarial** via `phi_once`) + `Stub.is_valid()=false` en strict + **gate fail-closed** (`MaxItersHit` ⟹ échec ; `L̂ ≥ 1−marge` ⟹ échec ; jamais « consensus » si non convergé) | **M** |
| **M1.d — déterminisme** | seed déterministe `(seed ⊕ step_index)` + `init_sigma=0` (ou retrait du claim) ; `config_hash` sur le proto à la place de `kappa/seed` | **M** |
| **M1.e — honnêteté** | schéma manifeste `controller`, dépréciation `kappa/seed`, bannissement des phrases (§7), redéfinition du drift de reload sur `config_hash` | **S** |

**DoD (acceptation)** :
1. **Un vrai `SwarmController` tourne une ronde servie** (Step-sur-gRPC) qui **converge** (`residual ≤ tol`, `stop_reason = Converged`), **contenue** dans la boîte de sûreté.
2. **H1 est un contrôle qui PEUT échouer** : un participant à topologie **dynamique** (donc potentiellement discontinu) OU un `L̂ ≥ 1−marge` ⟹ **RPC en échec**, jamais un consensus.
3. **Non-convergence ⟹ échec explicite** (`MaxItersHit` n'est plus un « succès »).
4. **Déterminisme** : soit reproductibilité bit-à-bit prouvée par un test de rerun, soit le claim est **retiré** du fil.
5. **Aucune phrase bannie** (§7) dans les docs ; `grep 'vrais SLM|convergence prouvée|byzantin'` = 0.
6. Le participant **servi** enveloppe le pont efe-core (pas `MockParticipant`) sous `core-integration` ; le mock reste disponible pour les tests.

---

## 9. Ce qu'on NE promet PAS

- **Pas de « vrais SLM »** : fusion numérique Mode1 sur templates stub ; aucun poids de LM ni tokenizer sur le chemin.
- **Pas de convergence garantie hors Palier-2** : sous topologie dynamique, H1 n'est pas prouvée — seulement mesurée (canary), et un `MaxItersHit` est fail-closed.
- **Pas de robustesse byzantine** : la moyenne uniforme cède à un participant adverse (cf. COUCHE3 §6, jalon M3).
- **Pas de déterminisme** sans le correctif §5 ; ne pas l'annoncer sur le fil tant qu'il n'est pas rétabli.
- **Pas de hot-reload de poids / sidecar SLM** (échafaudage PR#14/#15).

---

## 10. Corrections vs les docs / le code

| Revendication (doc/code) | Réalité (preuve) | Correctif M1 |
|---|---|---|
| « participant itère son opérateur Φ **sur `x_t`** » | `x_t` n'entre qu'à `β₁≈0.17` ; le latent vient des priors (`mvp_cpu.rs`, `spawner.rs`) | §1.1 + §7 (couplage faible documenté) |
| « garanties A5 §4 **transportées** » (docstring `local.rs:27`) | vrai **seulement si** Φ non-expansif ; jamais vérifié (Stub) | §3 (Palier-2 analytique) + §4 (garde réelle) |
| « algorithme **contractif**, compteur MaxIters = 0 » (`iterate.rs:146`) | hypothèse **assumée** ; expansif ⟹ non-convergence silencieuse | §4 (fail-closed) |
| descripteur `determinism` + `kappa/seed` sur le fil | Φ non déterministe (`UUIDv4`) ; `κ` sans sens pour un vrai controller | §5 (seed déterministe + `config_hash`) |
| roadmap « une ronde coordonne de **vrais essaims EFE/SLM** » | vrais essaims **efe-core** oui ; **SLM** non (templates stub) | §7 (re-libellé) |
| `verify_non_expansive` / garde H1 | `Stub` ⟹ garde morte (`certificate.rs:35`, `coupled.rs:144`) | §3/§4 (Empirical + strict) |

*Préparation M1 : « vrais participants » n'est pas un flip de flag — c'est un travail « L » qui, mal fait, **régresse la sûreté** sous le mock. Le seul chemin honnête : câbler le pont efe-core, **contraindre la topologie** pour retrouver une contraction **analytique** (`≤ β₁`), ajouter un **canary empirique adversarial** et un **gate fail-closed** sur la non-convergence, rétablir (ou retirer) le déterminisme — et dire « vrais essaims latents efe-core », **jamais** « vrais SLM ». Le dowry gagne en honnêteté ce qu'il perd en esbroufe : un méta-essaim qui coordonne de vrais essaims, converge de façon **vérifiable**, et **refuse** de mentir quand il ne converge pas.*

---

## Addendum · 2026-07-14 — Palier-2 livré, et ce que la mesure a tranché

**Livré** (complète le jalon M1.b du §8) : `FixedTemplateRouter` + `FixedCardinality`
(`efe-core/src/genesis/fixed.rs` — routage/cardinalité **constants**, le dag issu de `x`
est ignoré par construction, x-indépendance prouvée par tests) ; champ manifeste
`controller.topology: fixed | dynamic` (**défaut `fixed`**, l'opt-out `dynamic` est
loggé bruyamment au boot) ; assemblage à topologie fixe côté serveur
(`efe-meta-swarm-server::participant::core_engine`, miroir délibéré de la recette
latente de `mode1` — à consolider dans `mode1` quand la session concurrente aura posé
son rework du dissolver) ; token de fil `topology:fixed|dynamic` sur le descripteur.

**La mesure** (même boîte `[0.05, 0.30]^16`, même sonde, canari M1.c) :

| Topologie | `L̂` mesuré | Population effective | Verdict |
|---|---|---|---|
| dynamique (routage heuristique) | **0.691** | ≥128 agents, mix de templates | H1 tient (property-testé) |
| **fixe (Palier-2)** | **0.366** | 4 agents `na-sem` (plan du manifeste) | H1 tient (property-testé) |
| *prédiction §1.1 (forme fermée)* | *≤ β₁ ≈ 0.17* | — | **réfutée comme certificat** |

Deux enseignements — le premier **prudent** (l'expérience n'est PAS contrôlée), le
second **solide** (mesure d'un opérateur unique) :

1. **La comparaison fixe-vs-dynamique est CONFONDUE — ne pas en tirer de claim causal.**
   `0.366 < 0.691` va dans le sens attendu (retirer la discontinuité du routage *devrait*
   aider), mais ce n'est **pas** une expérience à variable unique : l'opérateur fixe
   honore le plan du manifeste (4 agents `na-sem` purs) tandis que l'opérateur dynamique,
   qui **ignore `spec.population`** (`GenesisEngine::spawn` dérive la cardinalité du dag,
   jamais du plan), fait éclore ≥128 agents de templates mixtes via
   `HeuristicEstimator` (`min_agents = 128`). La taille de population (~32×) ET le mix de
   templates changent EN MÊME TEMPS que la fixité du routage ; et les deux `L̂` sont des
   **bornes inférieures** issues d'une sonde qui cherche les discontinuités. Le ratio
   ~1.9 ne mesure donc PAS « la part du routage » — il faudrait une re-mesure à
   population/mix appariés (dynamique bridé à 4 `na-sem`) pour attribuer quoi que ce soit
   au routage. **Claim retiré : « fixer la topologie divise la sensibilité ».**
2. **Le §1.1 est réfuté comme certificat — ça, c'est solide** (mesure de l'opérateur
   fixe SEUL, sans comparaison) : même sans aucun routage, le vrai opérateur mesure
   `0.366 ≫ β₁ ≈ 0.17`. Il existe une sensibilité à `x` au-delà de `beta_mix` que la
   composition en forme fermée ne capture pas. **En conséquence, AUCUN certificat
   « analytique ≤ β₁ » n'est émis nulle part** : la topologie fixe apporte la
   **continuité par construction** (elle élimine LE mécanisme de rupture de H1 — les
   sauts entiers du routage), pas un module de contraction en forme fermée.

**Requalification honnête du Palier-2** (amende le §3) : « topologie fixe = Φ continu
par construction + borne **mesurée** (property-testée, non certifiée) ». Le seuil
honnête reste le mélange « fixe + canary » — inchangé ; seule la promesse « prouvé
analytiquement ≤ β₁ » est retirée tant que l'écart 0.366 vs 0.17 n'est pas réconcilié
(piste : mesurer les étages un à un via `phi_once` sur des variantes d'assemblage).

---

## Addendum · 2026-08-08 — audit de conformité : les deux écarts qui changeaient le comportement servi

L'audit du jalon M1 (6 critères du §8 + §3/§4/§5/§7 confrontés **au code**, suite de preuve
exécutée) a rendu 4 critères tenus et 2 écarts *de comportement*. Ils sont fermés ici ; les
écarts purement documentaires relevés au passage (§2 périmé, §7 prescrivant une phrase que
l'addendum du 14/07 a retirée, §8 sans marqueur de livraison et au lettrage divergent des
commits) restent ouverts et sont listés à la fin.

**Écart A — la preuve du DoD n'était défendue par aucune CI.** `ci.yml` exécute bien
`cargo nextest run --workspace`, donc `efe-meta-swarm-server` **est** testé — mais aux features
PAR DÉFAUT. Or les 5 tests qui prouvent le **DoD #1** (`m1d_core_serving_e2e.rs` : ronde servie
Step-sur-gRPC par un vrai `SwarmController`, gate H1 de boot, mesure Palier-2, `config_hash`)
vivent dans un fichier `#![cfg(feature = "core-integration")]` : **zéro test exécuté**. La preuve
existait et passait — je l'ai lancée à la main, 16/16 verte — mais rien ne la protégeait : une
dérive d'efe-core, du contrôleur latent ou de `core_engine::build` l'aurait cassée en silence.
*Correctif* : un job `meta-swarm-core-integration` dédié dans `ci.yml`, câblé dans `all-green`
(un job qui peut échouer sans bloquer serait lui-même un vert malhonnête). Job séparé plutôt
qu'une étape de `rust-tests` : la feature tire efe-core ET efe-server, un build qui ferait
sauter le timeout de 15 min. Il **compile toutes les cibles** sous la feature — c'est ce qui
attrape une rupture de `#[cfg]` n'importe où — mais n'en **exécute que la suite de preuve**, et
cette portée étroite est délibérée : le reste du crate tourne déjà dans `rust-tests`, et les
`integration_multiprocess_*` lancent de vrais binaires qui flakent sous charge parallèle (3
échecs observés en suite complète, les 3 verts isolés en 1,8 s). Les mettre dans une porte
bloquante apprendrait à relancer jusqu'au vert — exactement la pathologie visée.

**Écart B — le fail-closed de non-convergence était inerte là où il compte.** Le mécanisme
existait (§4 livré : gRPC `ABORTED`), mais `iteration.require_convergence` valait `false` par
défaut, sans aucun couplage à la présence d'un moteur réel — et un test épinglait ce défaut
permissif comme comportement correct. Le raisonnement qui manquait : un participant mock est
`Φ(x) = κ·x` avec `κ < 1`, il **converge par construction**, la garde y est sans objet ; c'est
le vrai opérateur efe-core, dont la contraction n'est que *property-testée* (canari M1.c, `L̂`
est une borne **inférieure** qui falsifie H1 sans jamais la certifier), qui peut rendre un
point-fixe parasite étiqueté « consensus ». La garde était donc désarmée exactement sur le seul
chemin qui en avait besoin — le trou que ce §4 appelle un *blocker de soundness* restait ouvert
pendant que la case était cochée.
*Correctif* : le champ devient `Option<bool>` (« absent » ≠ « à false ») et
`Manifest::require_convergence()` résout — **stipulé gagne**, non stipulé vaut `true` dès qu'une
ligne porte un bloc `controller`. On n'impose pas le choix, on exige qu'il soit **fait** :
l'opt-out explicite reste honoré et `load_and_validate` le journalise bruyamment quand un vrai
moteur est servi. Les manifestes mock v0.2.0 ne portent aucun bloc `controller` : leur
comportement est inchangé au bit près.

**Amendement au §4** : « en profil *contraction requise* » devient « **par défaut** dès qu'un
participant efe-core est servi ». Et le code est `ABORTED`, non `FAILED_PRECONDITION` comme
l'annonçait le §4 — écart délibéré et commenté au site : `classify_grpc_status` range
`FAILED_PRECONDITION` en `safety_violation`, ce qui aurait confondu une non-convergence avec une
sortie de boîte.

**Ce que l'audit a trouvé tenu**, et qu'il faut continuer à dire : DoD #1 (prouvé ET exécuté),
#4 (déterminisme prouvé par rerun + contrôle négatif ; fil honnête — `kappa`/`seed` sentinellés
et dépréciés, `config_hash`, `topology` annoncés), #6 (fail-closed dans les deux sens : un
binaire sans la feature refuse de servir un mock à la place d'un bloc `controller`, et le reload
rejette toute dérive du bloc — y compris la dérive de *présence* mock↔efe-core, qui aurait
permis de booter en mock puis de basculer à chaud sur un moteur jamais passé au canari).

**Écarts laissés ouverts** (documentaires, aucun effet servi) : (a) le §2 « état des lieux » est
un tableau au **présent** dont six affirmations sont désormais fausses (le test d'intégration
« n'existe pas » — il porte 11 tests ; `core-integration` « 0 hit dans le serveur » — 14 dans le
seul `participant/local.rs` ; `verify_non_expansive` « Stub » — `Empirical` réel ; etc.) ; (b) le
§7 prescrit de remplacer « convergence prouvée » par « contraction analytique ≤ β₁ », que
l'addendum du 14/07 a **retirée** — suivre le §7 produirait la sur-vente qu'il prétend éliminer,
alors que le code, lui, est honnête (aucune variante analytique dans l'enum de certificat) ;
(c) le §8 ne marque **aucun** jalon livré et son lettrage a divergé des commits (doc M1.b =
topologie fixe / commit M1.b = Φ déterministe ; doc M1.d = déterminisme / commit M1.d = vrais
participants servis) ; (d) le DoD #5 est littéralement inapplicable — `grep = 0` ne peut pas
être satisfait puisque les tables de bannissement contiennent les phrases ; sur le fond il est
tenu, aucune occurrence n'est une affirmation dans le périmètre.
