// SPDX-License-Identifier: UNLICENSED
//! Types pour le DAG.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::CoordResult;

/// Identifiant d'un nœud (kebab-case court, unique au sein du DAG).
pub type NodeId = String;

/// État d'un nœud après exécution.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NodeStatus {
    /// Pas encore démarré.
    Pending,
    /// En cours.
    Running,
    /// Fini avec succès.
    Success,
    /// Fini en erreur (le message est dans `DagRunReport.errors`).
    Failed,
    /// Pas exécuté car au moins un parent a échoué.
    Skipped,
    /// Pas exécuté car son `with_timeout` a expiré avant la fin.
    TimedOut,
}

/// Map des sorties des parents d'un nœud (clé = `NodeId` du parent).
///
/// Passé à chaque closure construite via [`DagNode::with_inputs`] ou
/// [`DagNode::with_parents_and_inputs`] juste avant son exécution, une
/// fois tous ses parents terminés en succès.
pub type ParentOutputs = HashMap<NodeId, serde_json::Value>;

/// Une fonction async qui produit un résultat sérialisé JSON.
///
/// Reçoit en argument les sorties des parents (vide pour les racines
/// et pour les nœuds construits via `root`/`with_parents`).
///
/// Convention : la sortie est un `serde_json::Value` pour permettre une
/// chaîne hétérogène (chaque nœud peut produire un type différent).
pub(crate) type NodeFn = Box<
    dyn FnOnce(
            ParentOutputs,
        ) -> Pin<Box<dyn Future<Output = CoordResult<serde_json::Value>> + Send>>
        + Send
        + 'static,
>;

/// Un nœud du DAG : identifiant + parents + futur + (optionnel) timeout.
pub struct DagNode {
    /// Identifiant kebab-case unique.
    pub id: NodeId,
    /// IDs des parents (doivent être déjà ajoutés au DAG).
    pub parents: Vec<NodeId>,
    /// Closure qui produit le futur (consumed à l'exécution).
    pub work: NodeFn,
    /// Si Some, le nœud est annulé après cette durée et marqué `TimedOut`.
    pub timeout: Option<Duration>,
}

impl DagNode {
    /// Construit un nœud terminal (sans parent, sans timeout, sans inputs).
    pub fn root<F, Fut>(id: impl Into<NodeId>, f: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = CoordResult<serde_json::Value>> + Send + 'static,
    {
        Self {
            id: id.into(),
            parents: vec![],
            work: Box::new(|_| Box::pin(f())),
            timeout: None,
        }
    }

    /// Construit un nœud avec parents (sans accès aux outputs parents).
    ///
    /// Pour accéder aux outputs parents, utilisez [`Self::with_parents_and_inputs`].
    pub fn with_parents<F, Fut, I, S>(id: impl Into<NodeId>, parents: I, f: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = CoordResult<serde_json::Value>> + Send + 'static,
        I: IntoIterator<Item = S>,
        S: Into<NodeId>,
    {
        Self {
            id: id.into(),
            parents: parents.into_iter().map(Into::into).collect(),
            work: Box::new(|_| Box::pin(f())),
            timeout: None,
        }
    }

    /// Construit un nœud avec parents ET accès aux outputs parents
    /// — pattern DAG fondamental.
    ///
    /// La closure reçoit une [`ParentOutputs`] : map `parent_id → output`
    /// de tous les parents en succès.  Si un parent a échoué/skippé/timed-out,
    /// ce nœud sera marqué `Skipped` et la closure ne sera pas appelée.
    ///
    /// # Exemple
    /// ```ignore
    /// let analyze = DagNode::root("analyze", || async { Ok(json!({"intent": "FACTUAL"})) });
    /// let route = DagNode::with_parents_and_inputs("route", ["analyze"], |inputs| async move {
    ///     let intent = inputs["analyze"]["intent"].as_str().unwrap();
    ///     Ok(json!({"selected_llm": choose_llm(intent)}))
    /// });
    /// ```
    pub fn with_parents_and_inputs<F, Fut, I, S>(
        id: impl Into<NodeId>,
        parents: I,
        f: F,
    ) -> Self
    where
        F: FnOnce(ParentOutputs) -> Fut + Send + 'static,
        Fut: Future<Output = CoordResult<serde_json::Value>> + Send + 'static,
        I: IntoIterator<Item = S>,
        S: Into<NodeId>,
    {
        Self {
            id: id.into(),
            parents: parents.into_iter().map(Into::into).collect(),
            work: Box::new(|inputs| Box::pin(f(inputs))),
            timeout: None,
        }
    }

    /// Variante racine avec inputs explicitement vides (rare, mais utile
    /// pour uniformité d'API quand on génère des DAGs programmatiquement).
    pub fn with_inputs<F, Fut>(id: impl Into<NodeId>, f: F) -> Self
    where
        F: FnOnce(ParentOutputs) -> Fut + Send + 'static,
        Fut: Future<Output = CoordResult<serde_json::Value>> + Send + 'static,
    {
        Self {
            id: id.into(),
            parents: vec![],
            work: Box::new(|inputs| Box::pin(f(inputs))),
            timeout: None,
        }
    }

    /// Attache un timeout : le nœud sera annulé et marqué `TimedOut`
    /// si l'exécution dépasse cette durée.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

impl std::fmt::Debug for DagNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DagNode")
            .field("id", &self.id)
            .field("parents", &self.parents)
            .field("timeout", &self.timeout)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn root_node_runs() {
        let n = DagNode::root("a", || async { Ok(serde_json::json!({"x": 1})) });
        assert_eq!(n.id, "a");
        assert!(n.parents.is_empty());
        assert!(n.timeout.is_none());
        let v = (n.work)(HashMap::new()).await.unwrap();
        assert_eq!(v["x"], 1);
    }

    #[tokio::test]
    async fn with_parents_records_them() {
        let n = DagNode::with_parents("b", ["a", "z"], || async { Ok(serde_json::Value::Null) });
        assert_eq!(n.parents, vec!["a".to_string(), "z".to_string()]);
    }

    #[tokio::test]
    async fn with_parents_and_inputs_consumes_parent_outputs() {
        let n = DagNode::with_parents_and_inputs("c", ["a"], |inputs| async move {
            let v = inputs.get("a").cloned().unwrap_or(serde_json::Value::Null);
            Ok(serde_json::json!({"got": v}))
        });
        let mut inputs = HashMap::new();
        inputs.insert("a".to_string(), serde_json::json!({"x": 42}));
        let out = (n.work)(inputs).await.unwrap();
        assert_eq!(out["got"]["x"], 42);
    }

    #[test]
    fn with_timeout_attaches_duration() {
        let n = DagNode::root("a", || async { Ok(serde_json::Value::Null) })
            .with_timeout(Duration::from_millis(100));
        assert_eq!(n.timeout, Some(Duration::from_millis(100)));
    }

    #[test]
    fn status_roundtrip() {
        let s = NodeStatus::Skipped;
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(j, "\"SKIPPED\"");
        let back: NodeStatus = serde_json::from_str(&j).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn status_timed_out_roundtrip() {
        let s = NodeStatus::TimedOut;
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(j, "\"TIMED_OUT\"");
        let back: NodeStatus = serde_json::from_str(&j).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn with_inputs_root_constructor() {
        let n = DagNode::with_inputs("a", |_| async { Ok(serde_json::Value::Null) });
        assert!(n.parents.is_empty());
    }
}
