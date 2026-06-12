// SPDX-License-Identifier: UNLICENSED
//! `TopologySpec` — choisir un pattern (T1/T2/T3/Consensus/Auto) + ses agents **par nom**,
//! sans connaître les internals. La résolution assemble la topologie depuis la flotte et
//! **enveloppe chaque agent** (budget + provenance).

use std::sync::Arc;

use efe_swarm_topology::{
    AdaptiveGvTopology, ConsensusTopology, ConvergenceMetric, DebateTopology, GenerateVerify,
    GossipTopology, GvTopology, HierTopology, Hierarchical, KPlan, Pattern, Pipeline,
    PipelineTopology, StopRule, Topology, DEFAULT_BATCH,
};
use serde_json::Value;

use crate::client::ClientMode;
use crate::resolver::Resolver;
use crate::result::SwarmError;

/// **Taille d'essaim** (ADR-014) : un nombre (le K historique, comportement inchangé)
/// **ou** une politique adaptative bornée — l'essaim échantillonne par vagues de
/// `batch` et s'arrête à la cible (`stop`), jamais avant `k_min`, jamais après `k_max`.
///
/// Le pré-vol budget chiffre toujours **k_max** (pire cas — I-AK4) ; la dégradation
/// budgétaire rabote k_max d'abord. `batch` est une grandeur de **spec** (défaut
/// [`DEFAULT_BATCH`] = 4, constante — jamais dérivée de la machine : architecture §2.3,
/// le déterminisme inter-machines ADR-007 prime sur le remplissage intra-run).
#[derive(Debug, Clone, PartialEq)]
pub enum KSpec {
    /// K fixe — `Auto`, tous les défauts et tous les usages historiques vivent ici.
    Fixed(usize),
    /// Échantillonnage séquentiel borné `[k_min, k_max]` (opt-in explicite).
    Adaptive { k_min: usize, k_max: usize, batch: usize, stop: StopRule },
}

impl From<usize> for KSpec {
    fn from(k: usize) -> Self {
        KSpec::Fixed(k.max(1))
    }
}

impl KSpec {
    /// Le **plafond** de génération — ce que chiffre le pré-vol (I-AK4) et ce que
    /// reçoit la topologie comme K de référence.
    pub fn max_k(&self) -> usize {
        match self {
            KSpec::Fixed(k) => (*k).max(1),
            KSpec::Adaptive { k_min, k_max, .. } => (*k_max).max(*k_min).max(1),
        }
    }

    /// Rabote le plafond pour tenir dans `cap` appels (la dégradation budgétaire
    /// rogne k_max d'abord ; k_min et batch suivent le plafond raboté).
    fn clamped_to(self, cap: usize) -> KSpec {
        let cap = cap.max(1);
        match self {
            KSpec::Fixed(k) => KSpec::Fixed(k.min(cap).max(1)),
            KSpec::Adaptive { k_min, k_max, batch, stop } => {
                let k_max = k_max.min(cap).max(1);
                KSpec::Adaptive { k_min: k_min.min(k_max), k_max, batch: batch.min(k_max).max(1), stop }
            }
        }
    }
}

/// Spécification d'une topologie + ses agents (résolus depuis la flotte).
#[derive(Debug, Clone)]
pub enum TopologySpec {
    /// **T1 — Generate-Verify** : K générateurs + vérificateur a-posteriori (VÉRIFIABLE).
    /// `k` adaptatif + [`StopRule::VerifiedQuota`] = le cas-phare d'ADR-014 : coût
    /// attendu 1/p appels au lieu de K fixe sur la zone vérifiable.
    GenerateVerify { generator: String, verifier: String, k: KSpec },
    /// **T2 — Pipeline** : étages nommés chaînés.
    Pipeline { stages: Vec<String> },
    /// **T3 — Hiérarchique** : décompose → résout (via une *stage*) → fusionne.
    Hierarchical { decomposer: String, solver: String, merger: String },
    /// **T-Consensus** : K générateurs + panel de juges → Vote/Borda/Condorcet.
    /// `k` adaptatif (Agreement / StdError) ⇒ génération par vagues bornées (ADR-014).
    Consensus { generator: String, judges: Vec<String>, k: KSpec, pattern: Pattern, escalate: bool },
    /// **T4 — Gossip** (itératif, ADR-003) : K refiners × tours, arrêt à ε ou r_max.
    /// L'influence mutuelle — chaque agent lit les pairs du tour précédent et révise.
    Gossip { refiner: String, k: usize, max_rounds: usize, epsilon: f64, metric: ConvergenceMetric },
    /// **Debate** (itératif, ADR-005 · réalisé par ADR-013) : consensus à DOSSIER
    /// contradictoire — PRO/CONTRA argumentent sur r tours, le jury note sur dossier,
    /// l'agrégation est inchangée. `rounds = 0` dégrade en consensus pur (borne basse).
    /// Variante SŒUR du Consensus (déviation actée : additive, zéro churn sur le
    /// Consensus validé — cf. ADR-SWARM-013, tableau des déviations).
    Debate {
        generator: String,
        judges: Vec<String>,
        pro: String,
        contra: String,
        k: usize,
        pattern: Pattern,
        rounds: usize,
        /// Escalade à trois étages (Vote → jury → débat) — le débat n'est payé que
        /// sur les instances où l'essaim, déjà escaladé, hésite encore.
        escalate: bool,
    },
    /// **Auto** : choisit selon la tâche (vérifiable → T1 ; sinon défaut sûr).
    /// Les topologies ITÉRATIVES ne sont JAMAIS auto-sélectionnées (opt-in explicite).
    Auto,
}

impl TopologySpec {
    /// Raccourci : un gossip Variance standard (mean_pull-compatible).
    pub fn gossip(refiner: &str, k: usize, max_rounds: usize, epsilon: f64) -> Self {
        TopologySpec::Gossip {
            refiner: refiner.into(),
            k,
            max_rounds,
            epsilon,
            metric: ConvergenceMetric::Variance,
        }
    }

    /// Raccourci : un débat Borda ESCALADÉ (Vote → jury → débat) avec des
    /// instructions PRO/CONTRA génériques — le débat n'est payé qu'à l'hésitation.
    pub fn debate(generator: &str, judges: &[&str], k: usize, rounds: usize) -> Self {
        TopologySpec::Debate {
            generator: generator.into(),
            judges: judges.iter().map(|s| s.to_string()).collect(),
            pro: "Argue FOR the candidate: cite its strongest evidence.".into(),
            contra: "Argue AGAINST the candidate: cite its weakest point.".into(),
            k,
            pattern: Pattern::Borda,
            rounds,
            escalate: true,
        }
    }

    /// Raccourci : un consensus Borda standard.
    pub fn consensus(generator: &str, judges: &[&str], k: usize) -> Self {
        TopologySpec::Consensus {
            generator: generator.into(),
            judges: judges.iter().map(|s| s.to_string()).collect(),
            k: k.into(),
            pattern: Pattern::Borda,
            escalate: false,
        }
    }

    /// Raccourci ADR-014 : un consensus Borda à **K adaptatif** — vagues de
    /// [`DEFAULT_BATCH`], arrêt à l'accord `threshold` (0,9 = parité sélecteur/escalades).
    pub fn consensus_adaptive(generator: &str, judges: &[&str], k_min: usize, k_max: usize, threshold: f64) -> Self {
        TopologySpec::Consensus {
            generator: generator.into(),
            judges: judges.iter().map(|s| s.to_string()).collect(),
            k: KSpec::Adaptive {
                k_min,
                k_max,
                batch: DEFAULT_BATCH,
                stop: StopRule::Agreement { threshold },
            },
            pattern: Pattern::Borda,
            escalate: false,
        }
    }

    /// Raccourci ADR-014 : un T1 à **quota vérifié** — l'essaim s'arrête dès `quota`
    /// candidats passés au vérificateur (coût attendu quota/p au lieu de k_max).
    pub fn generate_verify_adaptive(generator: &str, verifier: &str, k_min: usize, k_max: usize, quota: usize) -> Self {
        TopologySpec::GenerateVerify {
            generator: generator.into(),
            verifier: verifier.into(),
            k: KSpec::Adaptive {
                k_min,
                k_max,
                batch: DEFAULT_BATCH,
                stop: StopRule::VerifiedQuota { quota },
            },
        }
    }

    /// Concrétise `Auto` selon la tâche **ET le mode**. Reflète H-SWARM (la valeur est
    /// dans la zone vérifiable) :
    /// - **Keyless** (Mode A déterministe) : T1 Generate-Verify avec des agents de la
    ///   flotte par nom (vérificateur déterministe a-posteriori).
    /// - **Self-host** (Mode C LLM) : un consensus N-juges robuste, avec des **instructions**
    ///   en langage naturel (les noms de flotte n'ont pas de sens comme prompts LLM).
    pub(crate) fn concretize(self, task: &Value, mode: &ClientMode) -> TopologySpec {
        match self {
            TopologySpec::Auto => {
                if matches!(mode, ClientMode::Keyless) {
                    let verifier = if task.get("expected").is_some() { "equals_expected" } else { "non_empty_text" };
                    // Auto reste K FIXE (politique ADR-014 : l'adaptatif est un opt-in
                    // explicite de spec — jamais une surprise de facturation).
                    TopologySpec::GenerateVerify { generator: "echo_generator".into(), verifier: verifier.into(), k: 3.into() }
                } else {
                    // Self-host : instructions LLM + consensus (les juges rendent le run robuste).
                    TopologySpec::Consensus {
                        generator: "Answer the task with a single short result, no preamble.".into(),
                        judges: vec![
                            "Judge how well the proposed answer fits the task.".into(),
                            "Assess the relevance and correctness of the answer.".into(),
                        ],
                        k: 3.into(),
                        pattern: Pattern::Borda,
                        escalate: false,
                    }
                }
            }
            other => other,
        }
    }

    /// Estimation conservatrice du nombre d'appels d'agents (pour le pré-vol budget).
    pub(crate) fn estimated_calls(&self, assumed_subtasks: usize) -> usize {
        match self {
            // ADR-014 / I-AK4 : l'adaptatif est chiffré à son PIRE CAS (k_max) —
            // l'admission budgétaire reste honnête ; l'économie vient à l'exécution.
            TopologySpec::GenerateVerify { k, .. } => k.max_k(),
            TopologySpec::Pipeline { stages } => stages.len().max(1),
            TopologySpec::Hierarchical { .. } => 1 + assumed_subtasks + 1,
            TopologySpec::Consensus { k, judges, .. } => k.max_k() + judges.len(),
            // I-IT4 : le multi-tours est compté AVANT le premier appel — en SATURANT
            // (revue ADR-014 : un k×rounds délirant doit saturer, jamais déborder).
            TopologySpec::Gossip { k, max_rounds, .. } => (*k).max(1).saturating_mul((*max_rounds).max(1)),
            TopologySpec::Debate { k, judges, rounds, escalate, .. } => {
                // pire cas escaladé : K + jury① + 2K×r + jury② (les DEUX jurys).
                let jury_passes = if *escalate { 2 } else { 1 };
                let k = (*k).max(1);
                k.saturating_add(k.saturating_mul(2).saturating_mul(*rounds))
                    .saturating_add(judges.len().saturating_mul(jury_passes))
            }
            TopologySpec::Auto => assumed_subtasks,
        }
    }

    /// Dégrade la spec pour tenir dans `max_cents` à `price` par appel (réduit K, rogne
    /// les juges). Les topos sans paramètre de taille sont laissées telles quelles
    /// (la politique `Stop` les bornera à l'exécution).
    pub(crate) fn degraded_to_fit(self, max_cents: u32, price: u32, _assumed: usize) -> TopologySpec {
        if price == 0 {
            return self;
        }
        let budget_calls = (max_cents / price) as usize;
        match self {
            TopologySpec::GenerateVerify { generator, verifier, k } => {
                // ADR-014 : on rabote le PLAFOND (k_max) — k_min/batch suivent.
                TopologySpec::GenerateVerify { generator, verifier, k: k.clamped_to(budget_calls.max(1)) }
            }
            TopologySpec::Consensus { generator, judges, k, pattern, escalate } => {
                // réserve au moins 1 appel pour un juge (un consensus sans juge est invalide).
                let max_k = budget_calls.saturating_sub(1).max(1);
                let k = k.clamped_to(max_k);
                let remaining = budget_calls.saturating_sub(k.max_k()).max(1);
                let judges: Vec<String> = judges.into_iter().take(remaining).collect();
                TopologySpec::Consensus { generator, judges, k, pattern, escalate }
            }
            // Itératif : on rogne les TOURS d'abord (perdre un tour coûte moins que
            // perdre des voix — l'enseignement S1), puis K / les juges.
            TopologySpec::Gossip { refiner, k, max_rounds, epsilon, metric } => {
                let k2 = k.min(budget_calls.max(1)).max(1);
                let r2 = (budget_calls / k2).clamp(1, max_rounds.max(1));
                TopologySpec::Gossip { refiner, k: k2, max_rounds: r2, epsilon, metric }
            }
            TopologySpec::Debate { generator, judges, pro, contra, k, pattern, rounds, escalate } => {
                let jury_passes = if escalate { 2 } else { 1 };
                let k2 = k.min(budget_calls.saturating_sub(1).max(1)).max(1);
                let after_base = budget_calls.saturating_sub(k2);
                let judges2: Vec<String> =
                    judges.into_iter().take((after_base / jury_passes).max(1)).collect();
                let left = after_base.saturating_sub(judges2.len() * jury_passes);
                let rounds2 = if k2 > 0 { (left / (2 * k2)).min(rounds) } else { 0 };
                TopologySpec::Debate { generator, judges: judges2, pro, contra, k: k2, pattern, rounds: rounds2, escalate }
            }
            other => other,
        }
    }

    /// Assemble la topologie : agents résolus **par nom** depuis la flotte, chacun
    /// enveloppé par le métrage (budget + provenance). Les `agent_id` des
    /// générateurs/juges du consensus matchent les écritures internes
    /// (`/cand/*`, `/ballot/*`) pour l'ancrage.
    pub(crate) fn resolve(self, r: &Resolver<'_>) -> Result<Box<dyn Topology>, SwarmError> {
        // **ADR-014 (CP-1)** : la LARGEUR d'exécution, résolue une fois par le run
        // (cœurs en Mode A, backend déclaré en Mode C), descend ici dans CHAQUE
        // topologie — les « 8 » câblés des constructeurs ne sont plus jamais empruntés
        // par le chemin SDK/HTTP. I-CP1 : ce paramètre ne touche aucune sémantique.
        let width = r.width.max(1);
        match self {
            TopologySpec::GenerateVerify { generator, verifier, k } => {
                let gen = r.generator(&generator, "t1_generator")?;
                let ver = r.verifier(&verifier)?; // toujours déterministe (a-posteriori)
                match k {
                    KSpec::Fixed(k) => {
                        let k = k.max(1);
                        Ok(Box::new(GvTopology { gv: GenerateVerify::new(k, width.min(k)), gen, ver }))
                    }
                    KSpec::Adaptive { k_min, k_max, batch, stop } => Ok(Box::new(AdaptiveGvTopology {
                        gen,
                        ver,
                        plan: KPlan { k_min, k_max, batch, stop },
                        width,
                        budget_probe: Some(r.budget_probe()),
                    })),
                }
            }
            TopologySpec::Pipeline { stages } => {
                if stages.is_empty() {
                    return Err(SwarmError::EmptySpec("pipeline sans étage"));
                }
                let mut pipe = Pipeline::new();
                for (i, name) in stages.iter().enumerate() {
                    pipe = pipe.stage(name.clone(), r.stage(name, &format!("t2_stage_{i}"))?);
                }
                Ok(Box::new(PipelineTopology::new(pipe)))
            }
            TopologySpec::Hierarchical { decomposer, solver, merger } => {
                let dec = r.decomposer(&decomposer, "t3_decomposer")?;
                let solv = r.solver(&solver, "t3_solver")?; // flotte : stage adaptée ; self-host : LLM
                let mer = r.merger(&merger, "t3_merger")?;
                let hier = Hierarchical::new(dec, solv, mer).with_concurrency(width);
                Ok(Box::new(HierTopology::new(Arc::new(hier), 3.0)))
            }
            TopologySpec::Consensus { generator, judges, k, pattern, escalate } => {
                if judges.is_empty() {
                    return Err(SwarmError::EmptySpec("consensus sans juge"));
                }
                // ADR-014 : verified_quota exige un vérificateur — le consensus n'en a pas.
                if let KSpec::Adaptive { stop, .. } = &k {
                    if stop.needs_verifier() {
                        return Err(SwarmError::EmptySpec(
                            "consensus adaptatif : verified_quota requiert un vérificateur (utilisez generate_verify)",
                        ));
                    }
                }
                // agent_id "generator" / "judge_{i:03}" → matchent /cand/* + /ballot/* (ancrage).
                let gen = r.generator(&generator, "generator")?;
                let mut jv = Vec::with_capacity(judges.len());
                for (i, name) in judges.iter().enumerate() {
                    jv.push(r.judge(name, &format!("judge_{i:03}"))?);
                }
                let mut topo =
                    ConsensusTopology::new(gen, k.max_k(), jv).with_pattern(pattern).with_concurrency(width);
                if escalate {
                    topo = topo.with_escalation(0.9, pattern);
                }
                if let KSpec::Adaptive { k_min, k_max, batch, stop } = k {
                    topo = topo
                        .with_k_plan(KPlan { k_min, k_max, batch, stop })
                        .with_budget_probe(r.budget_probe());
                }
                Ok(Box::new(topo))
            }
            TopologySpec::Gossip { refiner, k, max_rounds, epsilon, metric } => {
                // agent_id "refiner" → matche les écritures /gossip/<r>/<i> (ancrage).
                let rf = r.refiner(&refiner, "refiner")?;
                Ok(Box::new(
                    GossipTopology::new(rf, k.max(1), max_rounds.max(1), epsilon, metric)
                        .with_concurrency(width),
                ))
            }
            TopologySpec::Debate { generator, judges, pro, contra, k, pattern, rounds, escalate } => {
                if judges.is_empty() {
                    return Err(SwarmError::EmptySpec("debate sans juge"));
                }
                let gen = r.generator(&generator, "generator")?;
                let mut jv = Vec::with_capacity(judges.len());
                for (i, name) in judges.iter().enumerate() {
                    jv.push(r.judge(name, &format!("judge_{i:03}"))?);
                }
                let (p, c) = r.arguers(&pro, &contra);
                let mut topo =
                    DebateTopology::new(gen, k.max(1), p, c, jv, pattern, rounds).with_concurrency(width);
                if escalate {
                    topo = topo.with_escalation(0.9, 0.9); // parité consensus (escalade 0.9)
                }
                Ok(Box::new(topo))
            }
            TopologySpec::Auto => unreachable!("Auto doit être concrétisé avant resolve"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn auto_is_mode_aware() {
        let task = json!({ "text": "x", "expected": "x" });
        // Keyless : flotte Mode A déterministe → T1.
        match TopologySpec::Auto.concretize(&task, &ClientMode::Keyless) {
            TopologySpec::GenerateVerify { generator, verifier, .. } => {
                assert_eq!(generator, "echo_generator");
                assert_eq!(verifier, "equals_expected"); // vérifiable
            }
            other => panic!("Keyless Auto attendu T1, eu {other:?}"),
        }
        // Self-host : instructions LLM + consensus (pas de noms de flotte).
        let sh = ClientMode::SelfHost { endpoint: "x".into(), model: "m".into() };
        match TopologySpec::Auto.concretize(&task, &sh) {
            TopologySpec::Consensus { generator, judges, k, .. } => {
                assert!(generator.contains(' '), "instruction LLM, pas un nom de flotte");
                assert_eq!(judges.len(), 2);
                assert_eq!(k, KSpec::Fixed(3), "Auto reste K FIXE (l'adaptatif est opt-in — ADR-014)");
            }
            other => panic!("Self-host Auto attendu Consensus, eu {other:?}"),
        }
    }

    #[test]
    fn estimated_calls_uses_kmax_and_degrade_clamps_kmax_first() {
        // I-AK4 : le pré-vol chiffre le PIRE CAS (k_max), jamais l'espérance.
        let spec = TopologySpec::consensus_adaptive("echo_generator", &["consistency", "length_band"], 4, 24, 0.9);
        assert_eq!(spec.estimated_calls(8), 24 + 2, "k_max + juges");
        // Dégradation : k_max rabote d'abord ; k_min et batch suivent le plafond.
        match spec.degraded_to_fit(10, 1, 8) {
            TopologySpec::Consensus { k: KSpec::Adaptive { k_min, k_max, batch, .. }, judges, .. } => {
                assert_eq!(k_max, 9, "plafond raboté : 10¢ − 1 juge réservé");
                assert_eq!(k_min, 4, "plancher conservé tant qu'il tient sous le plafond");
                assert!(batch <= k_max && batch >= 1);
                assert_eq!(judges.len(), 1, "le budget restant ne paie qu'un juge");
            }
            other => panic!("attendu Consensus adaptatif, eu {other:?}"),
        }
        // Rabotage SOUS le plancher : k_min suit le plafond (I-AK1 : k_min ≤ k_max).
        let tight = TopologySpec::generate_verify_adaptive("echo_generator", "non_empty_text", 4, 16, 1);
        assert_eq!(tight.estimated_calls(8), 16);
        match tight.degraded_to_fit(2, 1, 8) {
            TopologySpec::GenerateVerify { k: KSpec::Adaptive { k_min, k_max, .. }, .. } => {
                assert_eq!((k_min, k_max), (2, 2));
            }
            other => panic!("attendu T1 adaptatif, eu {other:?}"),
        }
    }

    #[test]
    fn non_auto_specs_pass_through_concretize() {
        let spec = TopologySpec::Pipeline { stages: vec!["uppercase".into()] };
        match spec.concretize(&json!({}), &ClientMode::Keyless) {
            TopologySpec::Pipeline { stages } => assert_eq!(stages, vec!["uppercase".to_string()]),
            other => panic!("attendu Pipeline inchangé, eu {other:?}"),
        }
    }

    #[test]
    fn iterative_specs_estimate_and_degrade_rounds_first() {
        // Gossip : K×r ; la dégradation rogne les TOURS d'abord (I-IT4).
        let g = TopologySpec::gossip("mean_pull", 4, 10, 0.1);
        assert_eq!(g.estimated_calls(8), 40);
        match g.degraded_to_fit(12, 1, 8) {
            TopologySpec::Gossip { k, max_rounds, .. } => {
                assert_eq!((k, max_rounds), (4, 3), "K conservé, r rogné : 4×3 = 12 ¢");
            }
            other => panic!("attendu Gossip, eu {other:?}"),
        }
        // Debate : K + 2K×r + J×(1|2 jurys selon escalade) — le pire cas est compté.
        let d = TopologySpec::Debate {
            generator: "g".into(), judges: vec!["a".into(), "b".into()],
            pro: "p".into(), contra: "c".into(), k: 3, pattern: Pattern::Borda,
            rounds: 2, escalate: false,
        };
        assert_eq!(d.estimated_calls(8), 3 + 12 + 2);
        let d_esc = TopologySpec::Debate {
            generator: "g".into(), judges: vec!["a".into(), "b".into()],
            pro: "p".into(), contra: "c".into(), k: 3, pattern: Pattern::Borda,
            rounds: 2, escalate: true,
        };
        assert_eq!(d_esc.estimated_calls(8), 3 + 12 + 4, "escalade : les DEUX jurys comptés");
        // Dégradation : les tours sautent avant les juges.
        match d.degraded_to_fit(9, 1, 8) {
            TopologySpec::Debate { k, judges, rounds, .. } => {
                assert_eq!((k, judges.len(), rounds), (3, 2, 0), "r→0 (consensus pur), juges gardés");
            }
            other => panic!("attendu Debate, eu {other:?}"),
        }
    }
}
