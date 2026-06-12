# ADR-SWARM-013 : Topologies itératives CONSTRUITES (levée des gates 003/005 sur décision propriétaire) — livrées **opt-in**, leurs gates exécutés rendent NO-GO

- **Status** : Accepted · **Date** : 2026-06-12
- **Phase** : post-rapport (extension itérative) · commit `167f81f`
- **Related** : ADR-SWARM-003 (Gossip gaté) ; ADR-SWARM-005 (Debate gaté) ; ADR-SWARM-002
  (l'ancre déterministe) ; ADR-SWARM-007 (rejeu sur version) ; Lot S2-bis · Lot S3-bis
  (artefacts `scripts/swarm/EFE_SWARM_S{2,3}BIS_2026-06-11.json`) ;
  architecture `docs/reports/EFE_Archi_Topologies_Iteratives_2026-06-11.html`

## Contexte

Les ADR-003/005 gardaient Gossip et Debate à **0 LOC** derrière des gates (S2-bis,
S3-bis) aux priors défavorables chiffrés. Le **propriétaire du projet a ordonné le
build** (« fais le code complet et massif ») — une levée de gate par décision
d'autorité, prévue nulle part : il faut donc l'acter, avec la façon dont la
discipline a été préservée *à l'intérieur* du build.

## Décision

1. **Construire la verticale itérative complète** (module
   `efe-swarm-topology/src/iterative/` : RoundDriver + invariants I-IT1..5,
   GossipTopology, DebateTopology avec escalade à trois étages ; SDK
   `TopologySpec::{Gossip, Debate}` ; DTO de fil ; verdicts au dashboard ;
   graphe d'influence scellé via la convention `_reads` → check DAG).
2. **Exécuter les gates AVEC le build, le même jour** — pas après, pas jamais :
   - **Lot S2-bis : NO-GO mesuré.** À iso-budget (12 appels), le bras-témoin
     `mean@12` (|err| 0,942 ± 0,036) bat le meilleur gossip 6×2 (1,294 ± 0,046).
     Le prior de l'ADR-003 (Lot 1 : itération Δ = −0,4 % ± 3,1 %) est **confirmé
     par la mesure** sur la famille « estimation continue bruitée ».
   - **Lot S3-bis : NO-GO conditionnel, δ\* = ∞** sur la famille S3. Même en
     créditant le débat d'un gain de compétence δ = +0,15 (acc 0,979), « payer
     plus de juges » au même budget domine (J = 19 : 0,999). Le levier reste v
     (ADR-004/005 reconfirmés). Un GO exigerait un δ réel mesuré en Mode C
     (juges avec vs sans dossier) supérieur au seuil — non atteint à ce jour.
3. **Politique de livraison : OPT-IN STRICT.** Les topologies itératives ne sont
   **jamais auto-sélectionnées** (`TopologySpec::Auto` ne les choisit pas) ; leur
   usage est un acte explicite de spec/DTO. Les verdicts NO-GO ci-dessus sont la
   *notice d'emploi* versionnée : sur les familles mesurées, le one-shot
   (T1/mean, Borda+v) reste le choix par défaut recommandé.

## Déviations assumées vs les specs gelées (003/005) — et leurs raisons

| Spec gelée | Réalisé | Raison |
|---|---|---|
| Debate = `impl Aggregator` **dans** le consensus, derrière `#[cfg(feature = "debate")]` | **variante sœur** `TopologySpec::Debate` (topologie composée), **sans** cargo-feature | additif (zéro churn sur le Consensus validé) ; le débat enrichit la *tâche du jury* (dossier), pas l'agrégation — la frontière naturelle est la topologie ; une feature ne gaterait aucune dépendance (contrairement à postgres/oidc) et fragmenterait la matrice de test — l'opt-in par spec suffit |
| réveil/collecte par `subscribe_prefix` + « quiescence de round » (crédits WS2) | **barrière par jointure** de la vague (JoinSet) ; le bus publie l'indice par cellule mais ne pilote rien | sémantiquement équivalent pour des vagues de taille fixe (architecture §3.1) ; c'est le motif **validé** de `ConsensusTopology::generate/run_judges` ; la décision d'arrêt lit la **vérité** (snapshot) — keystone strict |
| influence prouvée par le hash d'entrée | **renforcée** : convention `_reads` (cellules `pointer@version` déclarées par les vagues, liftées par le Meter, contrôlées par le check DAG anti-version-fantôme) | réalise intégralement la promesse §4.3 de l'architecture : « qui a lu qui, à quel tour » re-prouvable hors ligne |

## Conséquences

### Positives
- La capacité existe pour la niche qui l'exigerait (estimation collaborative,
  arbitrage contradictoire à valeur probatoire : le **transcript scellé** est la
  version du débat multi-agents qui a un sens d'audit) — à un coût marginal
  désormais connu et versionné.
- La discipline a survécu à la levée d'autorité : harnais livrés ET exécutés,
  bras-témoins « meilleur usage du même budget » inclus, verdicts archivés.
  Le projet sait dire de son propre code : « construit, prouvé, et voici quand
  ne pas s'en servir ».
- Escalade à trois étages : le débat ne se paie que sur les instances où
  l'essaim, déjà escaladé, hésite encore — le coût r× est confiné par design.

### Négatives / risques tenus
- +2 172 LoC de surface à maintenir pour des topologies hors chemin recommandé
  (mitigé : 33 tests dédiés, clippy 0, gardiens keyless intacts).
- Risque de chambre d'écho documenté (architecture §7) : l'ancre ADR-002 reste
  HORS de la boucle (le jury juge la fin, jamais le chemin) ; prompt Mode C
  anti-alignement (« révise seulement si les pairs te convainquent »).

## Références
- `crates/efe-swarm-topology/src/iterative/{mod,round_driver,gossip,debate}.rs`
- `crates/efe-swarm-sdk/tests/iterative_keyless.rs` · `crates/efe-swarm-gateway/tests/iterative_http.rs`
- `crates/efe-swarm-sdk/examples/{lot_s2bis,lot_s3bis,selfhost_iterative}.rs`
- `scripts/swarm/EFE_SWARM_S2BIS_2026-06-11.json` · `EFE_SWARM_S3BIS_2026-06-11.json`
