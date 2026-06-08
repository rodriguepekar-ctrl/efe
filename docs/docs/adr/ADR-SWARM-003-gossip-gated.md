# ADR-SWARM-003 : T4 Gossip — spécifié mais GATÉ (0 LOC tant que non prouvé)

- **Status** : Accepted · **Date** : 2026-06-08
- **Phase** : 2 (Fleet & topologies) · clôture W6
- **Related** : ADR-SWARM-002 (T1/T2/T3) ; Lot 1 (l'itération ⊘) ; S1 (one-shot suffit) ; validation Phase 2

## Contexte

L'architecture de Phase 2 listait 4 topologies. Trois sont **construites et validées**
(ADR-SWARM-002 + harness `validate`) : **T1** Generate-Verify, **T2** Pipeline, **T3**
Hiérarchique. La quatrième — **T4 Gossip / agrégation P2P** (raffinement itératif décentralisé)
— **n'a AUCUNE preuve de gain**. La discipline de Phase 2 (« ne construire que ce que les gates
ont montré gagnant ») impose de la traiter à part.

## Décision

1. **T4 Gossip est SPÉCIFIÉE (ci-dessous) mais NON CONSTRUITE — 0 LOC.**
2. **Build conditionné à un gate cheap-first dédié — Lot S2-bis** : *le raffinement itératif
   bat-il le one-shot Generate-Verify (T1) sur le composite ?* Tant que ce gate n'est pas **GO
   robuste (±σ)**, aucune ligne de Gossip n'est écrite.
3. **Prior fort = NO** : Lot 1 a déjà montré que le **consensus itératif** sur ancres ≈
   **kNN@1-itération** (Δ −0,4 %±3,1 %). Le gossip est une forme d'itération ; construire sans
   preuve répèterait l'erreur de Lot 1.

## Spécification (gelée, pour le jour où le gate passe)

**Forme** : N agents partagent des estimations PARTIELLES via le Blackboard (stigmergie) et
convergent par tours.

```
round r :   chaque agent lit les estimations des pairs au round r−1 (/gossip/<r-1>/*),
            produit une estimation affinée → écrit /gossip/<r>/<agent>,
            publie une notif sur le topic "gossip.r<r>".
critère d'arrêt : variance inter-agents < ε  OU  r == r_max.
```

**Composition sur le substrat existant (aucune primitive nouvelle requise)** :
- État partagé : `Blackboard` (namespace transient `/gossip/`, purgé par `new_epoch`).
- Réveil par tour : `TopicRouter` (un topic par round) ; collecte des pairs via `subscribe_prefix("gossip.")`.
- Exécution : `StreamingSwarm` (agents long-vivants) + **détection de convergence** réutilisant le
  schéma de **quiescence par crédit** (WS2) — ici étendu à « quiescence de round ».
- Agents : générateurs/affineurs Mode A ou C du catalogue.

**API pressentie** (non implémentée) :
```rust
// pub struct Gossip { agents: Vec<Arc<dyn Refiner>>, epsilon: f64, max_rounds: usize }
// #[async_trait] pub trait Refiner { async fn refine(&self, own: &Value, peers: &[Value]) -> Value; }
// impl Gossip { pub async fn run(&self, task: &Value, ctx: &SwarmContext) -> Value }
```

## Le gate (Lot S2-bis) — à exécuter AVANT tout build

**Question** : sur une famille de tâches où le raffinement itératif POURRAIT aider (estimation
continue bruitée, p.ex. agrégation de scores, consensus numérique), **Gossip(r tours) bat-il
T1(one-shot) sur (qualité × coût) ?**
- **Bras** : T1 (K candidats, 1 tour) vs Gossip (K agents, r tours = r×K appels). Splits répétés ±σ.
- **Coût** : Gossip coûte **r×** plus cher que T1 → il doit gagner **≥ r×** en qualité pour payer.
- **GO** : un Gossip bat T1 **robustement** (borne basse > T1) à coût justifié → on construit.
- **NO-GO** (prior) : Gossip ≈ T1 → l'itération n'apporte rien (cohérent Lot 1/S1) → **abandon
  définitif de T4**, on capitalise sur T1/T2/T3.

## Conséquences

### Positives
- **Discipline tenue** : pas de dette spéculative (cf. ADR-P6-001 « binder efe-core pour un gain
  nul »). Le substrat suffirait à bâtir Gossip *si* prouvé — la spec le démontre.
- **Phase 2 close proprement** : 3 topologies validées (T1/T2/T3), la 4ᵉ gatée avec son test.

### Négatives
- Si une niche réelle exige du raffinement itératif, il faudra d'abord passer Lot S2-bis (délai).

## Références
- `crates/efe-swarm-topology/examples/validate.rs` — SLO Phase 2 (T1/T3 vs monolithe)
- ADR-SWARM-002 — T1/T2/T3 (le territoire gagnant)
- `scripts/latent/` (Lot 1) — pourquoi l'itération est suspecte par défaut
