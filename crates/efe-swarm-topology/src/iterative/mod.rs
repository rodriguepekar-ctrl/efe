// SPDX-License-Identifier: UNLICENSED
//! # `iterative` — les topologies d'**influence mutuelle** (T4 Gossip · Debate).
//!
//! Les quatre topologies historiques sont *one-shot* : générer → juger → agréger,
//! personne ne révise sa copie à la lumière de celle du voisin. Ce module ajoute la
//! cinquième famille : des agents qui **se lisent et s'adaptent**, par tours.
//!
//! ## Statut & gates (ADR-SWARM-003 · ADR-SWARM-005)
//! Ces topologies étaient **gatées à 0 LOC** (priors défavorables chiffrés : Lot 1
//! Δ = −0,4 % ± 3,1 % ; S3 one-shot 0,716/0,871 ≈ oracle). La décision de build a été
//! prise par le propriétaire du projet ; la discipline est honorée *dans* le build :
//! - les **harnais de gate** S2-bis / S3-bis sont livrés avec le code
//!   (`efe-swarm-sdk/examples/lot_s2bis.rs` · `lot_s3bis.rs`) et écrivent leurs
//!   artefacts datés dans `scripts/swarm/` ;
//! - les bras-témoins « meilleur usage du même budget » y figurent (moyenne sans
//!   itération ; juges supplémentaires) — l'itératif doit les battre pour mériter
//!   son coût r×.
//!
//! ## Architecture (cf. `docs/reports/EFE_Archi_Topologies_Iteratives_2026-06-11`)
//! Une seule pièce neuve orchestre les deux topologies : le [`RoundDriver`]
//! (rounds → vague bornée → écritures **write-once** → barrière → arrêt sur **vérité**).
//! Tout le reste compose des primitives existantes : Blackboard CAS (`write_if`),
//! `new_epoch` (namespaces transients), `TopicRouter` (indice lossy), `snapshot()`
//! (la vérité), `agreement()` (la métrique catégorielle du sélecteur de consensus).
//!
//! ## Invariants (famille I-IT)
//! - **I-IT1** write-once par (tour, cellule) — CAS `write_if(…, 0, …)` ; un conflit
//!   est un bug compté dans le rapport.
//! - **I-IT2** terminaison inconditionnelle — `max_rounds` est une borne dure.
//! - **I-IT3** lecture figée du passé — la vague r ne voit que la **vérité** du tour
//!   r−1 (relue du snapshot), jamais les notifications.
//! - **I-IT4** budget multi-tours borné a priori — `estimated_calls` côté SDK compte
//!   K×r ; en vol, I13 (le `BudgetGuard`) reste souverain.
//! - **I-IT5** arrêt déterministe — le résiduel est une fonction pure des cellules.
//!
//! ## Provenance — le graphe d'influence, scellé
//! Chaque appel d'agent itératif passe par le Meter du SDK (capture + budget + N17) ;
//! chaque cellule de tour est **ancrée** via le `WriteSink` du contexte. L'influence
//! est prouvée à DEUX niveaux : le hash d'entrée commet les CONTENUS lus (les pairs
//! sont dans l'input), et la convention **`_reads`** — posée par les vagues
//! ([`RoundCell`] porte `pointer@version`), liftée par le Meter — scelle les
//! LECTURES cellule-par-cellule dans le record (`reads: Vec<CellRef>`), contrôlées
//! par le check DAG anti-version-fantôme de `verify_swarm`. « Qui a lu qui, à quel
//! tour » est re-prouvable hors ligne — l'argument §4.3 de l'architecture, réalisé.

mod debate;
mod gossip;
mod round_driver;

pub use debate::{builtin_arguers, DebateTopology};
pub use gossip::{builtin_refiner, GossipTopology, StageRefiner};
pub use round_driver::{RoundCell, RoundDriver, RoundsReport};

use async_trait::async_trait;
use serde_json::Value;

use crate::consensus::agreement;

/// **Refiner** — le rôle de l'influence mutuelle (T4 Gossip).
///
/// `own` = sa propre cellule au tour précédent (`None` au tour 0 : le Refiner est
/// alors le *générateur initial*) ; `peers` = les cellules des **autres** agents au
/// tour précédent (vérité figée, I-IT3). Le `task` transporte aussi le contexte de
/// vague injecté par la topologie : `{"gossip": {"agent_index": i, "round": r}}` —
/// ce qui rend les refiners Mode A déterministes par agent sans état interne.
#[async_trait]
pub trait Refiner: Send + Sync + 'static {
    async fn refine(&self, task: &Value, own: Option<&Value>, peers: &[Value]) -> Value;
}

/// Métrique de convergence d'un tour : renvoie un **résiduel** (0 = convergé).
///
/// - [`ConvergenceMetric::Variance`] — variance de population des champs numériques
///   (`value` ou scalaire) : la famille « estimation continue » de l'ADR-003.
/// - [`ConvergenceMetric::Agreement`] — 1 − accord (fraction de paires identiques,
///   réutilise `consensus::selector::agreement`) : le catégoriel/textuel.
/// - [`ConvergenceMetric::Stability`] — 1 − accord entre le tour r−1 et le tour r
///   (par cellule) : « le débat se répète » (arrêt anticipé du Debate).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvergenceMetric {
    Variance,
    Agreement,
    Stability,
}

impl ConvergenceMetric {
    /// Résiduel du tour courant (fonction pure — I-IT5). `prev` n'est consulté que
    /// par `Stability` ; vide au tour 0 ⇒ résiduel 1.0 (jamais convergé d'office).
    pub fn residual(&self, prev: &[Value], cur: &[Value]) -> f64 {
        match self {
            ConvergenceMetric::Variance => variance(cur),
            ConvergenceMetric::Agreement => 1.0 - agreement(cur),
            ConvergenceMetric::Stability => {
                if prev.len() != cur.len() || cur.is_empty() {
                    return 1.0;
                }
                let same = prev.iter().zip(cur.iter()).filter(|(a, b)| a == b).count();
                1.0 - (same as f64 / cur.len() as f64)
            }
        }
    }

    /// L'agrégat final naturel de la métrique (la sortie du Gossip).
    pub fn aggregate(&self, cells: &[Value]) -> Value {
        match self {
            ConvergenceMetric::Variance => Value::from(mean(cells)),
            ConvergenceMetric::Agreement | ConvergenceMetric::Stability => majority(cells),
        }
    }
}

/// Valeur numérique d'une cellule : champ `value` numérique, sinon scalaire nu.
pub(crate) fn num_of(v: &Value) -> Option<f64> {
    v.get("value").and_then(Value::as_f64).or_else(|| v.as_f64())
}

fn mean(cells: &[Value]) -> f64 {
    let xs: Vec<f64> = cells.iter().filter_map(num_of).collect();
    if xs.is_empty() {
        return 0.0;
    }
    xs.iter().sum::<f64>() / xs.len() as f64
}

/// Variance de population des cellules numériques (< 2 valeurs ⇒ 0 : convergé).
fn variance(cells: &[Value]) -> f64 {
    let xs: Vec<f64> = cells.iter().filter_map(num_of).collect();
    if xs.len() < 2 {
        return 0.0;
    }
    let m = xs.iter().sum::<f64>() / xs.len() as f64;
    xs.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / xs.len() as f64
}

/// Valeur majoritaire (mode) — départage déterministe par forme canonique.
fn majority(cells: &[Value]) -> Value {
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<String, (usize, &Value)> = BTreeMap::new();
    for c in cells {
        let key = c.to_string();
        let e = counts.entry(key).or_insert((0, c));
        e.0 += 1;
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1 .0.cmp(&b.1 .0))
        .map(|(_, (_, v))| v.clone())
        .unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn variance_residual_and_aggregate() {
        let cells = vec![json!({"value": 1.0}), json!({"value": 3.0})];
        let m = ConvergenceMetric::Variance;
        assert!((m.residual(&[], &cells) - 1.0).abs() < 1e-9, "var([1,3]) = 1");
        assert!((m.aggregate(&cells).as_f64().unwrap() - 2.0).abs() < 1e-9, "mean = 2");
        // < 2 valeurs numériques → résiduel 0 (convergé par vacuité).
        assert_eq!(m.residual(&[], &[json!({"value": 5.0})]), 0.0);
    }

    #[test]
    fn agreement_residual_and_majority() {
        let m = ConvergenceMetric::Agreement;
        let all_same = vec![json!("a"), json!("a"), json!("a")];
        assert_eq!(m.residual(&[], &all_same), 0.0);
        let split = vec![json!("a"), json!("a"), json!("b")];
        assert!(m.residual(&[], &split) > 0.0);
        assert_eq!(m.aggregate(&split), json!("a"), "majorité = a");
    }

    #[test]
    fn stability_compares_rounds() {
        let m = ConvergenceMetric::Stability;
        let prev = vec![json!("x"), json!("y")];
        let cur_same = prev.clone();
        let cur_diff = vec![json!("x"), json!("z")];
        assert_eq!(m.residual(&prev, &cur_same), 0.0, "rien n'a bougé → stable");
        assert!((m.residual(&prev, &cur_diff) - 0.5).abs() < 1e-9, "1 cellule sur 2 a changé");
        assert_eq!(m.residual(&[], &cur_same), 1.0, "tour 0 : jamais stable d'office");
    }
}
