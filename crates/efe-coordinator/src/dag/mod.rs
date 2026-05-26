// SPDX-License-Identifier: UNLICENSED
//! Exécuteur DAG générique pour orchestrer les nano-robots.
//!
//! ## Modèle
//!
//! - Un **nœud** = une unité d'exécution async (boxed future).
//! - Une **arête** dirigée = `parent_id → child_id` indique que
//!   `child` ne peut démarrer qu'après que `parent` ait fini.
//! - Le DAG est exécuté en respectant les dépendances ; les nœuds
//!   indépendants s'exécutent en parallèle (limite : `max_concurrency`).
//!
//! ## Garanties
//!
//! - **I8** : parallélisme borné par `max_concurrency`.
//! - DAG cycle detected at submit (Kahn topological sort).
//! - Si un nœud échoue, ses enfants sont marqués `Skipped` (pas exécutés).

mod executor;
mod node;

pub use executor::{DagExecutor, DagRunReport};
pub use node::{DagNode, NodeId, NodeStatus, ParentOutputs};
