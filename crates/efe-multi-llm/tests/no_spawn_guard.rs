// SPDX-License-Identifier: UNLICENSED
//! **Couche 2 (M3, DoD #7)** — invariant de non-régression FIGÉ : aucun `tokio::spawn` dans
//! les `run_*` (workflows). La capture de provenance M1 (gateway) tient par un task-local
//! `MISSION` posé par `scoped()` ; or `tokio::spawn` détache la nouvelle tâche du task-local
//! ⟹ la mission perdrait **silencieusement** sa capture (sous-comptage → nœud `Missing` →
//! `verified=false` sur un run pourtant honnête). Ce test lit le source des workflows et
//! échoue si un futur refactor introduit `tokio::spawn` — transformant une propriété fragile
//! (spec §5, « Invariant à figer ») en gardien de CI.
//!
//! Purement additif : ne modifie aucun code de bibliothèque d'`efe-multi-llm`.

/// Source des 5 missions + le module (chemins relatifs STABLES à ce fichier de test).
const WORKFLOW_SRCS: &[(&str, &str)] = &[
    ("vote.rs", include_str!("../src/workflows/vote.rs")),
    ("relay.rs", include_str!("../src/workflows/relay.rs")),
    ("debate.rs", include_str!("../src/workflows/debate.rs")),
    ("specialist.rs", include_str!("../src/workflows/specialist.rs")),
    ("cascade.rs", include_str!("../src/workflows/cascade.rs")),
    ("mod.rs", include_str!("../src/workflows/mod.rs")),
];

#[test]
fn run_workflows_stay_task_local_no_tokio_spawn() {
    for (name, src) in WORKFLOW_SRCS {
        assert!(
            !src.contains("tokio::spawn"),
            "workflows/{name} contient `tokio::spawn` — la capture de provenance M1 (task-local \
             MISSION) serait perdue. Gardez les run_* task-locaux (join_all / FuturesUnordered / \
             tokio::join! INLINE, jamais spawn)."
        );
    }
}
