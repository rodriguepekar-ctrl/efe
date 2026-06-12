# ADR-SWARM-014 : Taille d'essaim adaptative (échantillonnage séquentiel borné) et largeur d'exécution dimensionnée machine — construits AVEC leurs gates

- **Status** : Accepted · **Date** : 2026-06-12
- **Phase** : fermeture des gaps §17 n°2 (« K adaptatif ») et n°5-étage-process (« admission-control »)
- **Related** : architecture `docs/reports/EFE_Archi_K_Adaptatif_Concurrence_CPU_2026-06-12.html` ;
  ADR-SWARM-007 (ordre canonique — le socle de I-CP1) ; ADR-SWARM-013 (la discipline de
  livraison opt-in + déviations actées) ; bench_scale 2026-06-11 (le plateau 22 k agents/s
  qui a chiffré le « 8 » câblé) ; Lot K-ADAPT · bench_width
  (artefacts `scripts/swarm/EFE_SWARM_KADAPT_2026-06-12.json` · `EFE_SWARM_WIDTH_2026-06-12.json`)

## Contexte

Avant ce build : **K était figé partout** (la spec le posait, rien ne l'adaptait en vol —
alors que juges, débat, tours et pattern s'adaptent déjà) ; et la **concurrence était un
« 8 » câblé en cinq exemplaires** (`ConsensusTopology`, `GossipTopology`, `DebateTopology`,
`GenerateVerify` via `k.min(8)`, plus le 16 du Hiérarchique), avec une molette
`with_concurrency` publique que **rien n'appelait** — sur une machine de référence à
11 cœurs dont le bench de scale portait la signature de la borne (latence affine à
~50 µs/agent, débit en plateau dès K=25). Le propriétaire a ordonné le build
(« fais le code complet et massif ») sur la base de l'architecture du 2026-06-12.

Contrairement aux ADR-003/005, le **prior est favorable** : l'arrêt séquentiel (Wald)
ne prétend pas faire MIEUX à budget égal — il prétend atteindre la même qualité en
dépensant moins quand les données le permettent. La discipline reste identique :
harnais construits AVEC le code, bras-témoins honnêtes, verdicts archivés quoi qu'ils disent.

## Décision

1. **`KSpec`** (`Fixed(usize)` | `Adaptive { k_min, k_max, batch, stop }`) sur
   `TopologySpec::{GenerateVerify, Consensus}`. Trois règles d'arrêt **pures**
   (`efe-swarm-topology/src/sizing.rs`) : `Agreement` (catégoriel — réutilise
   `consensus::selector::agreement`), `StdError` (numérique — s/√n), `VerifiedQuota`
   (T1, le cas-phare : coût attendu ~quota/p au lieu de K fixe). La boucle
   (`run_waves`) : plancher d'abord, puis vagues de `batch`, observation pure,
   arrêt sur cible / k_max / budget.
2. **`batch` ∈ SPEC, constante par défaut (4)** — jamais dérivé de la machine : le
   déterminisme inter-machines (ADR-007, prouvé au millième par la campagne du 12 juin)
   prime sur le remplissage intra-run. C'est la décision d'architecture centrale
   (§2.3) : *batch = sémantique ; width = exécution*.
3. **`WidthSpec`** (`Auto` | `Fixed`) au `SwarmBuilder` : `Auto` = cœurs mesurés en
   Mode A (`available_parallelism`), **capacité déclarée du backend** en Mode C
   (`LocalBackend::suggested_width()`, constante conservatrice 2). Le `Resolver`
   descend la largeur dans les SIX topologies (la molette morte revit) ; DTO `width`
   optionnel, **plafonné serveur** (`EFE_GATEWAY_MAX_WIDTH`, défaut 32).
   **Déviation actée vs l'architecture §3.1** (façon ADR-013) : la précédence prévue
   « env `EFE_SWARM_WIDTH` lue par le SDK » violait le garde-fou exécutable **G1**
   (`keyless_guardrails.rs` : AUCUN `env::var` dans les sources du SDK — l'env est
   l'endroit où se cachent les clés) ; l'override opérateur est donc lu **côté
   passerelle** (`state.rs::width_spec`, comme `EFE_SWARM_ROUTER_ENDPOINT`) et
   injecté en `Fixed` ; `EFE_OLLAMA_PARALLEL` est abandonné au profit d'une largeur
   explicite. Le garde-fou a attrapé le conflit pendant le build — keyless par
   construction prime.
4. **Admission process à la passerelle** : sémaphore FIFO
   (`EFE_GATEWAY_MAX_INFLIGHT_RUNS`, défaut max(4, cœurs)), `try_acquire` en tête de
   handler (sync ET jobs), refus = **429 + Retry-After** + corps stable — du
   backpressure visible. Trois métriques : `inflight_runs`, `inflight_max`,
   `admission_rejected_total`.
5. **Appariement de provenance rendu déterministe** (`recorder.rs::seal_with_key`) :
   les brouillons sont appariés aux ancres en ordre de **contenu**
   (agent, variant, input_hash, output_hash), plus jamais en ordre de capture —
   correction nécessaire à I-CP1 (l'ordre d'achèvement, que la largeur remanie,
   fuyait dans les records) ; pour les rôles à variant c'est AUSSI l'appariement
   sémantique exact (variant i ↔ /cand/i).
6. **Politique opt-in stricte** (parité ADR-013) : `Auto` reste `Fixed(3)` ;
   l'adaptatif est un acte explicite de spec/DTO ; l'itératif (Gossip/Debate) reste
   **hors périmètre** K-adaptatif (NO-GO S2-bis sur la famille + forme du graphe
   d'influence scellé) ; le jury garde son escalade — les deux adaptativités
   **composent** sans se voir (testé : une instance triviale traverse tout pour
   k_min appels).

## Invariants introduits (gardiens nominatifs)

| Inv. | Énoncé | Gardien |
|---|---|---|
| I-AK1 | k_min ≤ K_used ≤ k_max, bornes dures (exception tracée : 0 sur budget-première-vague) | `stops_at_k_min_when_target_is_immediate` · `runs_to_k_max_when_target_is_unreachable` · `kplan_normalizes_hard_bounds` (sizing) · `adaptive_k_stops_at_k_min_and_wraps_output_with_sizing` · `adaptive_k_runs_to_k_max_when_candidates_disagree` (consensus) · `first_wave_budget_refusal_yields_zero_sample_honestly` |
| I-AK2 | décision pure, re-dérivable hors ligne | `sizing_trace_matches_offline_rederivation` |
| I-AK3 | candidat i = f(task, i) — insensible au découpage ; trace rejouée identique | `runs_to_k_max_when_target_is_unreachable` (ordre des variants) + rejouabilité dans I-AK2 |
| I-AK4 | pré-vol au pire cas (k_max) ; aucune vague impayable, zéro sentinelle | `error_policy_prices_the_worst_case_kmax` · `no_partial_wave_under_budget` · `can_pay_is_a_whole_wave_probe` |
| I-AK5 | le sizer choisit COMBIEN, jamais QUOI | `adaptive_equals_fixed_when_target_is_unreachable` (même gagnant, même dépense) |
| I-CP1 | la largeur n'est pas une sémantique | `semantic_content_is_invariant_under_width` (w ∈ {1,4,11,32}) · `pairing_is_deterministic_under_capture_order` (racine à identités normalisées) · grille `bench_width` |
| I-CP2 | un seul étage de permits par run, pas d'interblocage | `width_one_completes_all_topologies` (les 6 topologies à w=1) |
| I-CP3 | résolution totale (≥ 1, plafonds) | `width_resolution_clamps_all_inputs` · `width_is_capped_server_side_and_admission_is_bounded` |

Précision d'honnêteté (I-CP1) : la **racine** porte aussi les identités par-run
(`record_id` v4, `swarm_id`) — l'invariance sous largeur porte sur **tout le contenu
sémantique** (empreinte : agent, rôle, variant, hashes, reads, writes) ; à identités
normalisées, la racine elle-même est invariante (test unitaire du recorder).

## Verdicts des gates (exécutés le jour du build — artefacts datés)

- **Lot K-ADAPT : GO** (le premier GO franc de la série de gates — prior favorable
  confirmé par la mesure). 3 familles × 3 difficultés × 3 bras, 200 instances/cellule,
  bras à candidats PARTAGÉS (splitmix64), témoin iso-budget recalibré **après coup**
  sur ⌈E[K_used]⌉. Iso-qualité vs k_max ET ≥ iso-budget sur les **9 cellules** :
  - F1 numérique (SE ≤ 0,6) : économie **14 % / 37 % / 57 %** pour π facile 0,2/0,5/0,8 ;
  - F2 catégorielle (accord ≥ 0,9) : **16 % / 30 % / 46 %** ;
  - F3 vérifiable (quota 1, le cas-phare) : **83 % / 79 % / 56 %** pour p 0,9/0,5/0,2 —
    et le contraste décisif : à p=0,5, le K fixe iso-budget (k=2) ne trouve un vérifié
    que 81,5 % du temps là où l'adaptatif fait **100 %** au même coût moyen (2,49) ;
    à p=0,2 : 95,0 % vs 67,0 %. L'adaptatif met ses appels là où la difficulté l'exige.
  `scripts/swarm/EFE_SWARM_KADAPT_2026-06-12.json`.
- **bench_width : GO** (I-CP1 tenu sur toute la grille) — et l'**hypothèse de débit
  RÉFUTÉE, honnêtement** : grille {1,2,4,8,11,16,32,auto} × K {50,200}, 20 runs/case,
  empreinte sémantique IDENTIQUE partout (l'appariement déterministe tient sous vraie
  concurrence). Mais 8 → 11 voies (K=200) : 11 726 → 11 847 agents/s, soit **+1,0 %**
  contre les +20-35 % écrits avant mesure. Lecture : le débit intra-run Mode A est
  borné par la **sérialisation aval** (recorder sous mutex, scellement), pas par les
  voies — la largeur 1 est même la plus rapide à K=50. La valeur de la largeur est
  donc ailleurs, exactement là où le §3 la plaçait par ailleurs : **Mode C** (aligner
  la pression sur un backend qui sérialise, éviter les timeouts), **admission process**
  (la rafale inter-runs, elle, scale — bench_scale : ~122 k agents/s), et le **p99**
  (w=32 à K=200 : p99 42,6 ms, le churn mesuré qui justifie le plafond serveur 32).
  Optimiser la sérialisation du recorder devient le levier de débit identifié — noté
  comme piste, non promis. `scripts/swarm/EFE_SWARM_WIDTH_2026-06-12.json`.

## Revue adversariale post-build (le jour même) — trouvailles confirmées et corrigées

Une revue multi-agents (4 lentilles indépendantes : concurrence, invariants, contrat
de fil, provenance ; chaque trouvaille attaquée par un sceptique avec reproduction sur
code réel) a confirmé **9 défauts** que les 329 tests ne couvraient pas — tous corrigés
dans ce lot, chacun avec son gardien de régression :

1. **(majeur)** La sonde coopérative I-AK4 ne marquait jamais `exhausted` → un run
   adaptatif tronqué par le budget se déclarait `budget_stopped: false` /
   `is_clean() == true` (régression vs le chemin K fixe). Corrigé :
   `BudgetGuard::mark_exhausted()` posé par la sonde du Resolver au refus ;
   gardiens : `no_partial_wave_under_budget` (étendu) +
   `t1_truncated_to_zero_sample_is_visibly_not_clean`.
2. **(majeur)** Pré-vol `Error` : cast `as u32` tronquant — un k×rounds délirant
   pouvait être ADMIS par débordement. Corrigé : arithmétique u64 saturante
   (sdk.rs + `estimated_calls` Gossip/Debate).
3. **(majeur)** Corps malformé (`"k": 3.5`, typo) → 422 text/plain opaque hors
   enveloppe. Corrigé : extracteur `GatewayJson` → 400 `{error, message}` stable ;
   gardien au fil.
4. **(majeur)** Champs inconnus avalés (`"se_targt"`, `"bacth"`, `"widht"`) —
   une typo changeait la politique ET la facture en silence. Corrigé :
   `deny_unknown_fields` (RunRequest, BudgetDto, AdaptiveKDto en struct dédiée) ;
   gardien `unknown_fields_are_rejected_not_swallowed`. (TopologyDto, tagué
   `kind`, reste permissif — limitation serde documentée.)
5. **(majeur)** Mode job : échec de persistance avalé (`let _ = store.put`) — preuve
   perdue en silence, statut menteur. Corrigé : `RunFailed` émis sur le bus.
6. **(mineur)** Le 429 d'admission inventait son code machine (`at_capacity`) —
   aligné sur l'enveloppe stable (`too_many_requests`), champs additifs conservés.
7. **(mineur)** Cibles hors domaine acceptées (agreement 1.7, se_target 0,
   quota > k_max) → facture k_max silencieuse garantie. Corrigé : validation de
   domaine au fil (400 net) ; gardien `dto_k_adaptive_rejects_out_of_domain_targets`.
8. **(mineur)** I-AK1 documenté absolu mais `k_used = 0` possible (budget refusant la
   première vague). Corrigé : exception documentée + gardien
   `first_wave_budget_refusal_yields_zero_sample_honestly` (combiné au fix n°1,
   le run vide est visiblement non-propre).
9. **(mineur)** `/metrics` sans auth contredisait la doc « toutes les routes » —
   doc corrigée : surface agrégée sans données tenant, assumée non authentifiée
   (probes k8s), à protéger au niveau réseau.

Précision de provenance (non vérifiée par la revue — limite de quota — analysée à la
conception) : pour les rôles **sans variant** (refiners gossip), l'appariement
contenu-ordonné attribue les cellules `writes` de façon **stable mais arbitraire**
entre tours — même niveau d'imprécision qu'avant (où il était non-déterministe),
le hash d'entrée commettant toujours l'influence réelle ; `verify_swarm` et le DAG
sont insensibles à cette attribution. Noté comme imprécision connue, pas une
régression.

Après correctifs : **335 tests passés · 0 échec · clippy 0** ; les deux gates
re-rendent GO (les correctifs ne touchent pas leurs chemins).

## Passe de complétude (audit architecture → disque, même jour)

L'audit symbole-par-symbole contre l'architecture (§3/§4/§9) a fermé trois derniers
écarts : **(a)** le test Mode C promis `n17_consistent_under_adaptive_k` (ignoré sans
Ollama — le pont par-appel reste 100 % cohérent quand K varie en vol) ; **(b)**
l'observabilité « exec_width » du §3.3, réalisée en champ **`Accounting.width`**
par-run (déviation actée : une jauge Prometheus par-run n'a pas de sens — le champ
additif sert l'API, le dashboard et les tests ; la jauge `efe_swarm_gateway_max_width`
expose le plafond de config) — gardée par `semantic_content_is_invariant_under_width`
(la largeur rapportée est exacte, et n'est toujours qu'informative — I-CP1) ;
**(c)** le `quickstart` enrichi d'une section ADR-014 (K adaptatif, quota T1,
largeur). Bilan final : **336 tests passés · 0 échec · 6 ignorés · clippy 0**.

## Conséquences

### Positives
- « Payer selon l'hésitation » couvre désormais la grandeur la plus chère : K.
  Sur la zone vérifiable (T1 + quota), le coût attendu passe de K fixe à ~quota/p.
- La largeur cesse d'être héréditaire : cœurs réels en Mode A, capacité déclarée en
  Mode C (plus de 8 curls contre un Ollama qui sérialise), admission bornée au process.
- L'appariement de provenance est désormais déterministe ET sémantiquement exact pour
  les rôles à variant — une imprécision latente (invisible à `verify`) corrigée.

### Négatives / risques tenus
- Biais d'arrêt séquentiel (cadré : cible de précision, pas de test d'hypothèse ;
  les gates mesurent la qualité RÉALISÉE contre témoins).
- Accord saturé à petit n : k_min ≥ 4 recommandé (F2 du gate balaie les taux de leurre).
- Mode C : trajectoire auditable (trace + N17), pas rejouable — le statut de tout le
  Mode C (ADR-007).
- `suggested_width` est déclaratif, pas mesuré — défaut conservateur 2, breaker G3 en filet.
- L'admission process ne fait ni équité par tenant ni files pondérées — le reste du
  gap §17 n°5 demeure ouvert, explicitement.
- Identités de record aléatoires (v4) : la racine reste par-run ; une dérivation
  d'identité par contenu (racines inter-runs comparables) est notée comme évolution,
  non promise (risque de collision sur contenus identiques à arbitrer).

## Références
- `crates/efe-swarm-topology/src/{sizing.rs, consensus_topology.rs, topology.rs (AdaptiveGvTopology), generate_verify.rs}`
- `crates/efe-swarm-sdk/src/{spec.rs (KSpec), client.rs (WidthSpec), budget.rs (can_pay), resolver.rs, sdk.rs, backend/}`
- `crates/efe-swarm-provenance/src/recorder.rs` (appariement déterministe)
- `crates/efe-swarm-gateway/src/{dto.rs (KDto, width), state.rs (admission), routes/run.rs, routes/metrics.rs}`
- tests : `efe-swarm-sdk/tests/{width_keyless,adaptive_keyless}.rs` ·
  `efe-swarm-gateway/tests/admission_http.rs` · unitaires sizing/recorder/budget/client/spec/dto/state
- harnais : `efe-swarm-sdk/examples/{lot_kadapt,bench_width}.rs`
