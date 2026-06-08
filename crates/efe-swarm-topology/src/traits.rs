// SPDX-License-Identifier: UNLICENSED
//! Traits de la couche topologie : [`Generator`], [`Verifier`], [`Stage`].

use async_trait::async_trait;
use serde_json::Value;

/// Produit un candidat pour une tâche.
///
/// `variant` permet la DIVERSITÉ entre les K générateurs d'un essaim (S1 :
/// exploiter la diversité, pas la majorité) — ex. température/persona/seed
/// différents par variant. Mode A (pur Rust, déterministe) ou Mode C (SLM async).
#[async_trait]
pub trait Generator: Send + Sync + 'static {
    async fn generate(&self, task: &Value, variant: usize) -> Value;
}

/// Vérificateur a-posteriori (le pari ADR-SWARM-001/002) : LIT un candidat et le
/// score. `passes` = critère binaire :
/// - tâche **vérifiable** → vérificateur DÉTERMINISTE (checke la réponse) → atteint OR ;
/// - tâche **non-vérifiable** → seuil sur un score approché (juge LLM, précision v<1).
pub trait Verifier: Send + Sync {
    /// Score du candidat dans `[0, 1]` (plus haut = meilleur).
    fn score(&self, task: &Value, candidate: &Value) -> f64;

    /// Le candidat passe-t-il ? Par défaut : `score >= 0.5`.
    fn passes(&self, task: &Value, candidate: &Value) -> bool {
        self.score(task, candidate) >= 0.5
    }
}

/// Une étape de pipeline (T2) : transforme un item en 0..N items pour l'étage
/// suivant (1→1 transform, 1→N fan-out, 1→0 filtre).
#[async_trait]
pub trait Stage: Send + Sync + 'static {
    async fn process(&self, input: &Value) -> Vec<Value>;
}

// ─── T3 Hiérarchique : décomposition / résolution / agrégation ───────────────

/// **T3** : décompose une tâche en sous-tâches (le superviseur).
#[async_trait]
pub trait Decomposer: Send + Sync + 'static {
    async fn decompose(&self, task: &Value) -> Vec<Value>;
}

/// **T3** : résout UNE sous-tâche. Point de RÉCURSION : un `Solver` peut être une
/// feuille (agent Mode A), un T1 (`GvSolver`), un T2, ou un autre T3 (`HierSolver`).
#[async_trait]
pub trait Solver: Send + Sync + 'static {
    async fn solve(&self, subtask: &Value) -> Value;
}

/// **T3** : agrège les résultats des sous-tâches en une sortie finale.
#[async_trait]
pub trait Merger: Send + Sync + 'static {
    async fn merge(&self, task: &Value, results: Vec<Value>) -> Value;
}
