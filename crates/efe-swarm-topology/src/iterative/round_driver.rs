// SPDX-License-Identifier: UNLICENSED
//! **RoundDriver** — l'orchestrateur de tours, la seule pièce neuve de l'itératif.
//!
//! Contrat : *vagues → écritures write-once → barrière → arrêt sur vérité*.
//! ```text
//! tour r : prev = VÉRITÉ du tour r−1 (relue du snapshot — I-IT3, figée)
//!          wave.produce(r, prev)  →  cellules (∥, bornées par la vague)
//!          write_if({ns}/{r}/{sub}, 0, …)   — write-once (I-IT1, conflit = bug compté)
//!          publish "{topic}.r{r}"           — l'indice (lossy, jamais décisionnel)
//!          résiduel = métrique(prev, cur)   — pur (I-IT5) ; cur RELU du snapshot
//!          stop si résiduel ≤ ε  OU  r+1 == max_rounds (I-IT2)
//! ```
//! La **barrière** d'un tour est réalisée en joignant toutes les tâches de la vague
//! (le motif validé de `ConsensusTopology::generate`/`run_judges`) : sémantiquement
//! équivalente à la « quiescence de round » de l'ADR-003 pour des vagues de taille
//! fixe, et plus simple. Le driver ignore la sémantique des cellules — Gossip et
//! Debate injectent leur vague et leur métrique.

use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use efe_blackboard::{Blackboard, SwarmEvent, TopicRouter};
use efe_nano_sdk::{AgentId, SwarmId};
use serde_json::{json, Value};

use super::ConvergenceMetric;

/// Une cellule de tour, telle que la vague la REÇOIT : la valeur ET son adresse
/// exacte (`pointer@version`) — c'est ce qui permet aux topologies de déclarer
/// leurs lectures (`_reads`) et au DAG de la provenance de les contrôler.
#[derive(Debug, Clone)]
pub struct RoundCell {
    pub sub: String,
    pub pointer: String,
    pub version: u64,
    pub value: Value,
}

/// Une vague de tour : produit les cellules du tour `round` à partir de la vérité
/// figée du tour précédent (`prev`, adressée pointer@version, dans l'ordre de
/// production du tour r−1). Renvoie `(sous-chemin, auteur, valeur)` — le driver
/// écrit `{ns}/{round}/{sub}`.
#[async_trait]
pub(crate) trait RoundWave: Send + Sync {
    async fn produce(&self, round: usize, prev: &[RoundCell]) -> Vec<(String, AgentId, Value)>;
}

/// Rapport d'une exécution par tours.
#[derive(Debug, Clone)]
pub struct RoundsReport {
    /// Tours effectivement exécutés (≥ 1).
    pub rounds: usize,
    /// Résiduel du dernier tour (0 = parfaitement convergé).
    pub residual: f64,
    /// `residual ≤ ε` atteint avant `max_rounds` ?
    pub converged: bool,
    /// **I-IT1** : conflits CAS observés (devrait être 0 ; > 0 = bug de vague).
    pub write_conflicts: usize,
    /// Les cellules du dernier tour (la vérité finale, ordre de production).
    pub final_cells: Vec<Value>,
    /// Trajectoire des résiduels (diagnostic ; longueur == rounds).
    pub residual_history: Vec<f64>,
}

/// L'orchestrateur de tours (cf. doc de module).
pub struct RoundDriver {
    ns: String,
    topic: String,
    max_rounds: usize,
    epsilon: f64,
    metric: ConvergenceMetric,
}

impl RoundDriver {
    /// `ns` SANS slash final (ex. `"/gossip"`) ; `topic` la racine d'indice
    /// (ex. `"gossip"`) ; `max_rounds` est normalisé à ≥ 1 (I-IT2).
    pub fn new(ns: impl Into<String>, topic: impl Into<String>, max_rounds: usize, epsilon: f64, metric: ConvergenceMetric) -> Self {
        Self { ns: ns.into(), topic: topic.into(), max_rounds: max_rounds.max(1), epsilon, metric }
    }

    /// Exécute les tours sur le substrat fourni. Le namespace est purgé d'entrée
    /// (`new_epoch` — état transient par contrat ADR-003/005).
    pub(crate) async fn run(
        &self,
        bb: &Arc<Blackboard>,
        router: &Arc<TopicRouter>,
        swarm_id: SwarmId,
        wave: &dyn RoundWave,
    ) -> RoundsReport {
        bb.new_epoch(&[self.ns.as_str()]);

        let mut prev: Vec<RoundCell> = Vec::new();
        let mut conflicts = 0usize;
        let mut history: Vec<f64> = Vec::with_capacity(self.max_rounds);

        for r in 0..self.max_rounds {
            // ── vague (∥ interne à la vague ; la jointure EST la barrière) ──────
            let produced = wave.produce(r, &prev).await;

            // ── écritures write-once (I-IT1) + indice par cellule ───────────────
            for (sub, who, val) in &produced {
                let ptr = format!("{}/{}/{}", self.ns, r, sub);
                if bb.write_if(&ptr, 0, val.clone(), who.clone()).is_err() {
                    conflicts += 1; // un conflit = la vague a produit deux fois la même cellule.
                }
                let _ = router.publish(SwarmEvent {
                    swarm_id,
                    published_by: who.clone(),
                    topic: format!("{}.r{}", self.topic, r),
                    payload: json!({ "round": r, "cell": sub }),
                    published_at: SystemTime::now(),
                });
            }

            // ── relecture de la VÉRITÉ du tour (I-IT3 pour le tour suivant),
            //    ADRESSÉE pointer@version (le graphe d'influence en aura besoin) ──
            let snap = bb.snapshot();
            let cur: Vec<RoundCell> = produced
                .iter()
                .map(|(sub, _, val)| {
                    let pointer = format!("{}/{}/{}", self.ns, r, sub);
                    match snap.fields.get(&pointer) {
                        Some(c) => RoundCell { sub: sub.clone(), pointer, version: c.version, value: c.value.clone() },
                        None => RoundCell { sub: sub.clone(), pointer, version: 0, value: val.clone() },
                    }
                })
                .collect();

            // ── arrêt (I-IT5 : pur ; I-IT2 : borne dure) ────────────────────────
            let prev_vals: Vec<Value> = prev.iter().map(|c| c.value.clone()).collect();
            let cur_vals: Vec<Value> = cur.iter().map(|c| c.value.clone()).collect();
            let residual = self.metric.residual(&prev_vals, &cur_vals);
            history.push(residual);
            if residual <= self.epsilon {
                return RoundsReport {
                    rounds: r + 1,
                    residual,
                    converged: true,
                    write_conflicts: conflicts,
                    final_cells: cur_vals,
                    residual_history: history,
                };
            }
            prev = cur;
        }

        let residual = *history.last().unwrap_or(&f64::INFINITY);
        RoundsReport {
            rounds: self.max_rounds,
            residual,
            converged: false,
            write_conflicts: conflicts,
            final_cells: prev.into_iter().map(|c| c.value).collect(),
            residual_history: history,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aid(s: &str) -> AgentId {
        AgentId::new(s).unwrap()
    }

    /// Vague numérique qui CONVERGE : k cellules, l'écart est divisé par 2 à chaque
    /// tour (valeurs 100 ± spread/2^r) — la variance fond géométriquement.
    struct Halving {
        k: usize,
        spread: f64,
    }
    #[async_trait]
    impl RoundWave for Halving {
        async fn produce(&self, round: usize, _prev: &[RoundCell]) -> Vec<(String, AgentId, Value)> {
            let s = self.spread / (1u64 << round) as f64;
            (0..self.k)
                .map(|i| {
                    let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
                    (i.to_string(), aid("refiner"), json!({ "value": 100.0 + sign * s / 2.0 }))
                })
                .collect()
        }
    }

    /// Vague qui NE converge jamais (valeurs alternées constantes).
    struct Stuck {
        k: usize,
    }
    #[async_trait]
    impl RoundWave for Stuck {
        async fn produce(&self, _round: usize, _prev: &[RoundCell]) -> Vec<(String, AgentId, Value)> {
            (0..self.k).map(|i| (i.to_string(), aid("refiner"), json!({ "value": i as f64 }))).collect()
        }
    }

    /// Vague BOGUÉE : produit deux fois la cellule "0" (viole I-IT1 → conflit compté).
    struct Dup;
    #[async_trait]
    impl RoundWave for Dup {
        async fn produce(&self, _round: usize, _prev: &[RoundCell]) -> Vec<(String, AgentId, Value)> {
            vec![
                ("0".into(), aid("refiner"), json!({ "value": 1.0 })),
                ("0".into(), aid("refiner"), json!({ "value": 2.0 })),
            ]
        }
    }

    fn substrate() -> (Arc<Blackboard>, Arc<TopicRouter>, SwarmId) {
        let sid = SwarmId::new();
        (Arc::new(Blackboard::new(sid)), Arc::new(TopicRouter::new(256)), sid)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn converges_then_stops_before_max_rounds() {
        let (bb, router, sid) = substrate();
        // spread 8 → variances 16, 4, 1, 0.25… ; ε = 0.5 → stop au tour 4 (var 0.25).
        let d = RoundDriver::new("/gossip", "gossip", 10, 0.5, ConvergenceMetric::Variance);
        let rep = d.run(&bb, &router, sid, &Halving { k: 4, spread: 8.0 }).await;
        assert!(rep.converged, "doit converger : {rep:?}");
        assert_eq!(rep.rounds, 4);
        assert!(rep.residual <= 0.5);
        assert_eq!(rep.write_conflicts, 0, "I-IT1 tenu");
        assert_eq!(rep.residual_history.len(), rep.rounds);
        assert!(rep.residual_history.windows(2).all(|w| w[1] <= w[0]), "résiduel décroissant ici");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn r_max_is_a_hard_bound() {
        let (bb, router, sid) = substrate();
        let d = RoundDriver::new("/gossip", "gossip", 3, 1e-9, ConvergenceMetric::Variance);
        let rep = d.run(&bb, &router, sid, &Stuck { k: 4 }).await;
        assert!(!rep.converged, "Stuck ne converge pas");
        assert_eq!(rep.rounds, 3, "I-IT2 : borne dure");
        assert!(rep.residual > 0.0);
        assert_eq!(rep.final_cells.len(), 4, "la vérité du dernier tour est rendue");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn duplicate_cell_is_counted_as_conflict() {
        let (bb, router, sid) = substrate();
        let d = RoundDriver::new("/gossip", "gossip", 1, 0.0, ConvergenceMetric::Variance);
        let rep = d.run(&bb, &router, sid, &Dup).await;
        assert_eq!(rep.write_conflicts, 1, "le 2ᵉ write_if sur la même cellule casse (write-once)");
        // la VÉRITÉ conserve la 1ʳᵉ écriture (le CAS a refusé la seconde).
        assert_eq!(rep.final_cells[0], json!({ "value": 1.0 }));
        assert_eq!(rep.final_cells[1], json!({ "value": 1.0 }), "relecture = cellule write-once");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn new_epoch_purges_between_runs_on_same_substrate() {
        let (bb, router, sid) = substrate();
        let d = RoundDriver::new("/gossip", "gossip", 2, 1e-12, ConvergenceMetric::Variance);
        let r1 = d.run(&bb, &router, sid, &Stuck { k: 3 }).await;
        // 2ᵉ run sur le MÊME Blackboard : sans purge, tous les write_if seraient en
        // conflit (cellules déjà présentes). new_epoch garantit 0 conflit.
        let r2 = d.run(&bb, &router, sid, &Stuck { k: 3 }).await;
        assert_eq!(r1.write_conflicts, 0);
        assert_eq!(r2.write_conflicts, 0, "namespace transient purgé (ADR-003)");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stability_metric_stops_a_repeating_wave() {
        let (bb, router, sid) = substrate();
        // Stuck répète EXACTEMENT les mêmes cellules → Stability = 0 dès le tour 2.
        let d = RoundDriver::new("/debate", "debate", 5, 0.0, ConvergenceMetric::Stability);
        let rep = d.run(&bb, &router, sid, &Stuck { k: 3 }).await;
        assert!(rep.converged);
        assert_eq!(rep.rounds, 2, "tour 0 jamais stable (résiduel 1), tour 1 identique → stop");
    }
}
