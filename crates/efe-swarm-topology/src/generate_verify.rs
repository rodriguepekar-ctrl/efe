// SPDX-License-Identifier: UNLICENSED
//! **T1 — Generate-Verify** : K générateurs en parallèle + sélection a-posteriori.
//!
//! Le winner du gate Lot S2 (ADR-SWARM-002) sur les tâches vérifiables : la bonne
//! réponse est PRÉSENTE parmi K candidats bon marché ; un vérificateur a-posteriori
//! l'EXTRAIT (là où le vote majoritaire échoue).

use std::sync::Arc;

use serde_json::Value;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::traits::{Generator, Verifier};

/// Résultat d'un run [`GenerateVerify`].
#[derive(Debug, Clone)]
pub struct GvResult {
    /// Candidat sélectionné : le PASSANT de meilleur score ; sinon le meilleur score
    /// (fallback) ; `None` si aucun candidat n'a été produit.
    pub selected: Option<Value>,
    /// Tous les candidats générés (pour audit / provenance).
    pub candidates: Vec<Value>,
    /// Nombre de candidats ayant passé le vérificateur.
    pub passed: usize,
    /// Meilleur score observé (0.0 si aucun candidat).
    pub best_score: f64,
}

impl GvResult {
    /// True si au moins un candidat a passé le vérificateur.
    pub fn has_verified(&self) -> bool {
        self.passed > 0
    }
}

/// **T1** : génère K candidats EN PARALLÈLE (concurrence bornée) puis sélectionne
/// via le vérificateur a-posteriori.
#[derive(Debug, Clone)]
pub struct GenerateVerify {
    k: usize,
    max_concurrency: usize,
}

impl GenerateVerify {
    /// `k` = nombre de générateurs (taille essaim) ; `max_concurrency` borne les
    /// appels en vol (backpressure pour les SLM lents).
    pub fn new(k: usize, max_concurrency: usize) -> Self {
        Self {
            k: k.max(1),
            max_concurrency: max_concurrency.max(1),
        }
    }

    pub fn k(&self) -> usize {
        self.k
    }

    /// Exécute T1 sur une tâche : K candidats ∥, puis sélection a-posteriori.
    pub async fn run(&self, task: &Value, gen: Arc<dyn Generator>, ver: &dyn Verifier) -> GvResult {
        // ── Génération parallèle bornée ────────────────────────────────────────
        let sem = Arc::new(Semaphore::new(self.max_concurrency));
        let mut set: JoinSet<Value> = JoinSet::new();
        for variant in 0..self.k {
            let gen = gen.clone();
            let task = task.clone();
            let sem = sem.clone();
            set.spawn(async move {
                let _permit = sem.acquire_owned().await.expect("semaphore non fermé");
                gen.generate(&task, variant).await
            });
        }
        let mut candidates: Vec<Value> = Vec::with_capacity(self.k);
        while let Some(r) = set.join_next().await {
            if let Ok(c) = r {
                candidates.push(c);
            }
        }

        Self::select_from(task, candidates, ver)
    }

    /// **Sélection a-posteriori** sur des candidats déjà produits (par INDEX → aucun
    /// emprunt à travers le move). Partagée entre [`GenerateVerify::run`] (K fixe) et
    /// le chemin **adaptatif** d'ADR-014 (`AdaptiveGvTopology` : vagues + quota vérifié) —
    /// I-AK5 : le sizer choisit COMBIEN de candidats, jamais comment on sélectionne.
    pub fn select_from(task: &Value, candidates: Vec<Value>, ver: &dyn Verifier) -> GvResult {
        let mut best_pass: Option<(f64, usize)> = None;
        let mut best_any: Option<(f64, usize)> = None;
        let mut passed = 0usize;
        for (i, c) in candidates.iter().enumerate() {
            let s = ver.score(task, c);
            if best_any.map(|(bs, _)| s > bs).unwrap_or(true) {
                best_any = Some((s, i));
            }
            if ver.passes(task, c) {
                passed += 1;
                if best_pass.map(|(bs, _)| s > bs).unwrap_or(true) {
                    best_pass = Some((s, i));
                }
            }
        }
        let best_score = best_any.map(|(s, _)| s).unwrap_or(0.0);
        let sel_idx = best_pass.or(best_any).map(|(_, i)| i);
        let selected = sel_idx.map(|i| candidates[i].clone());

        GvResult {
            selected,
            candidates,
            passed,
            best_score,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::json;

    struct MockGen {
        truth: i64,
    }
    #[async_trait]
    impl Generator for MockGen {
        async fn generate(&self, _task: &Value, variant: usize) -> Value {
            // variants 0,3,6,… → correct ; sinon leurre corrélé (truth+1).
            if variant % 3 == 0 {
                json!(self.truth)
            } else {
                json!(self.truth + 1)
            }
        }
    }
    struct AllWrongGen;
    #[async_trait]
    impl Generator for AllWrongGen {
        async fn generate(&self, _task: &Value, variant: usize) -> Value {
            json!(1000 + variant as i64)
        }
    }
    struct DetVer {
        truth: i64,
    }
    impl Verifier for DetVer {
        fn score(&self, _t: &Value, c: &Value) -> f64 {
            if c.as_i64() == Some(self.truth) {
                1.0
            } else {
                0.0
            }
        }
        fn passes(&self, _t: &Value, c: &Value) -> bool {
            c.as_i64() == Some(self.truth)
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn t1_selects_a_verified_correct_candidate() {
        let gv = GenerateVerify::new(7, 4);
        let r = gv
            .run(&json!({}), Arc::new(MockGen { truth: 42 }), &DetVer { truth: 42 })
            .await;
        assert_eq!(r.candidates.len(), 7);
        assert!(r.has_verified());
        assert_eq!(r.selected, Some(json!(42)), "T1 sélectionne le correct (a-posteriori)");
        assert_eq!(r.best_score, 1.0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn t1_no_correct_candidate_falls_back_no_pass() {
        let gv = GenerateVerify::new(5, 2);
        let r = gv
            .run(&json!({}), Arc::new(AllWrongGen), &DetVer { truth: 42 })
            .await;
        assert_eq!(r.passed, 0, "aucun candidat ne passe");
        assert!(r.selected.is_some(), "fallback : meilleur score même si aucun ne passe");
        assert_eq!(r.best_score, 0.0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn t1_beats_majority_vote_when_correct_is_minority() {
        // 7 candidats : 3 corrects (variants 0,3,6), 4 leurres → le VOTE choisirait le
        // leurre (majorité), mais T1 (vérificateur) choisit un correct.
        let gv = GenerateVerify::new(7, 7);
        let r = gv
            .run(&json!({}), Arc::new(MockGen { truth: 7 }), &DetVer { truth: 7 })
            .await;
        let lure_count = r.candidates.iter().filter(|c| c.as_i64() == Some(8)).count();
        assert!(lure_count > r.passed, "le leurre est MAJORITAIRE (le vote échouerait)");
        assert_eq!(r.selected, Some(json!(7)), "T1 extrait quand même le correct");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn t1_respects_k() {
        let gv = GenerateVerify::new(3, 1);
        let r = gv.run(&json!({}), Arc::new(AllWrongGen), &DetVer { truth: 0 }).await;
        assert_eq!(r.candidates.len(), 3);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn gv_topology_marks_unverified_run_unacceptable_despite_best_any() {
        // MEM-2.c (revue adversariale) : GV renvoie `best_any` (NON-nul) même quand AUCUN candidat
        // ne passe. La sortie est donc non-nulle, mais `produced_acceptable` doit être FAUX (sans
        // quoi le SDK, devinant l'échec depuis la forme du gagnant, manquerait cet échec réel →
        // sous-comptage de la mémoire d'échec). C'est exactement le bug HIGH que la revue a trouvé.
        use crate::fleet::AgentFleet;
        use crate::topology::{GvTopology, SwarmContext, Topology};
        let topo = GvTopology {
            gv: GenerateVerify::new(5, 2),
            gen: Arc::new(AllWrongGen),
            ver: Arc::new(DetVer { truth: 42 }),
        };
        let ctx = SwarmContext::new(Arc::new(AgentFleet::standard()));
        let out = topo.run(&json!({}), &ctx).await;
        assert!(!out.output.is_null(), "best_any : un gagnant NON-NUL est tout de même renvoyé");
        assert_eq!(out.quality.verified, 0, "aucun candidat vérifié");
        assert!(!out.produced_acceptable, "MEM-2.c : non vérifié → NON acceptable (→ failed=true en mémoire)");
    }

    #[test]
    fn answer_is_present_is_shape_aware() {
        // MEM-2.c (revue) : la présence d'une réponse est testée sur `value` | `text` | la valeur
        // NUE, avec la VACUITÉ (null, chaîne vide/blanche, objet/tableau vide) → absente. Corrige
        // le faux négatif des sorties `{text:""}` (pipeline/merger) que le test `value`-only manquait.
        use crate::topology::answer_is_present;
        assert!(answer_is_present(&json!({ "value": "tech" })));
        assert!(answer_is_present(&json!({ "text": "résumé" })));
        assert!(answer_is_present(&json!("réponse nue")));
        assert!(answer_is_present(&json!(42)));
        assert!(!answer_is_present(&json!(null)), "null → absente");
        assert!(!answer_is_present(&json!({ "text": "" })), "text vide → absente (LE faux négatif corrigé)");
        assert!(!answer_is_present(&json!({ "value": "  " })), "value blanche → absente");
        assert!(!answer_is_present(&json!("")), "chaîne nue vide → absente");
        assert!(!answer_is_present(&json!({})), "objet vide → absente");
    }
}
