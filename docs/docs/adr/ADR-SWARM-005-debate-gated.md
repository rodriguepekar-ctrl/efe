# ADR-SWARM-005 : Debate (consensus itératif) — spécifié mais GATÉ (0 LOC)

- **Status** : Accepted · **Date** : 2026-06-09
- **Phase** : 3 (Qualité émergente) · clôture inc.4
- **Related** : Lot 1 (itération ⊘) ; ADR-SWARM-003 (Gossip gaté) ; ADR-SWARM-004 (consensus GO) ; Lot S3

## Contexte

`efe-multi-llm` propose 4 stratégies. Trois sont **construites et validées** (math prouvée par
oracles + `ConsensusTopology` sur le substrat + Lot S3 + `validate_consensus`) : **Vote** (plancher),
**Borda**, **Condorcet**. La quatrième — **Debate** (PRO/CONTRA argumentant sur plusieurs rounds +
jury) — est **ITÉRATIVE**. Comme le Gossip (ADR-SWARM-003), l'itération est suspecte par défaut.

## Décision

1. **Debate est SPÉCIFIÉ (ci-dessous) mais NON CONSTRUIT — 0 LOC.**
2. **Build conditionné à Lot S3-bis** : *le débat multi-rounds bat-il le consensus one-shot
   (Borda/Condorcet) ASSEZ pour payer son coût r× ?*
3. **Prior fort = NO**, pour DEUX raisons convergentes :
   - **Lot 1** : le consensus itératif sur ancres ≈ kNN@1-itération (l'itération n'apportait rien).
   - **Lot S3** : le consensus **one-shot** (Borda/Condorcet sur juges) crack DÉJÀ le non-vérifiable
     (0,716 à v=0,70, 0,855 à v=0,85 ≈ oracle 0,895). Le levier est **la qualité des juges (v)**,
     pas le nombre de tours. Ajouter des rounds = payer r× pour un gain probablement marginal.

## Spécification (gelée, pour le jour où le gate passe)

**Forme** : pour chaque candidat (ou paire), des agents PRO et CONTRA produisent des arguments sur
r rounds ; un agent JURY lit les arguments et score → ballot ; agrégation (Borda/Condorcet).

```
round 1..r :  PRO_i argumente POUR le candidat i (lit les arguments adverses du round précédent),
              CONTRA_i argumente CONTRE → écrits au Blackboard /debate/<round>/<role>/<i>.
fin :         JURY lit tous les arguments → score par candidat → ballot → agrégation consensus.
```

**Composition sur le substrat (aucune primitive nouvelle requise — réutilise Phase 1/3)** :
- État partagé : `Blackboard` (namespace transient `/debate/`, purgé par `new_epoch`).
- Réveil par round : `TopicRouter` (un topic par round) ; le jury collecte via `subscribe_prefix("debate.")`.
- Réactivité : `StreamingSwarm` (agents PRO/CONTRA long-vivants) + quiescence par round (crédit WS2).
- Juges = agents Mode C (le JURY) ; arguments = agents générateurs Mode C.

**API pressentie** (non implémentée, derrière `#[cfg(feature = "debate")]`) :
```rust
// pub struct DebateConsensus { rounds: usize, pro: Arc<dyn Generator>, contra: Arc<dyn Generator>,
//                              jury: Vec<Arc<dyn Judge>> }
// #[async_trait] impl Aggregator for DebateConsensus { ... }  // 0 LOC tant que non prouvé
```

## Le gate (Lot S3-bis) — AVANT tout build

**Question** : Debate(r rounds) bat-il Borda/Condorcet(one-shot) sur (qualité × coût) ?
- **Bras** : Borda/Condorcet (1 passe de jury) vs Debate (r rounds d'arguments + jury). Splits répétés ±σ.
- **Coût** : Debate ≈ r× le nombre d'appels → il doit gagner **≥ r×** en qualité pour payer.
- **GO** : Debate bat le one-shot **robustement** à coût justifié → on construit.
- **NO-GO** (prior) : Debate ≈ one-shot → l'argumentation itérative n'apporte rien au-delà de
  meilleurs juges → **abandon de Debate**, on investit dans la QUALITÉ DES JUGES (le vrai levier S3).

## Conséquences

### Positives
- **Discipline tenue** (3ᵉ topologie gatée après Gossip et Mode B) : 0 dette spéculative ; le
  substrat suffirait à bâtir Debate *si* prouvé — la spec le démontre.
- **Phase 3 close** : Vote/Borda/Condorcet construits et validés ; Debate gaté avec son test.
- **Le levier est clair** : améliorer **v** (juges) > ajouter des rounds.

### Négatives
- Si une niche exige vraiment de l'argumentation contradictoire (tâches très subjectives), il
  faudra d'abord passer Lot S3-bis (délai).

## Références
- `crates/efe-swarm-topology/src/consensus/` — Vote/Borda/Condorcet (construits, prouvés)
- `crates/efe-swarm-topology/examples/{lot_s3,validate_consensus,llm_judge}.rs`
- ADR-SWARM-004 (consensus GO, le levier = v) · ADR-SWARM-003 (Gossip gaté) · Lot 1 (itération ⊘)
