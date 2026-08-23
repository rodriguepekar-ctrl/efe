// SPDX-License-Identifier: UNLICENSED
//! # `provenance-capture` — Provenance P1 / S1 : la couche de **capture** (taps + store)
//!
//! Produit un **vrai DAG scellé** sur le runtime, via le contrat de S0 ([`provenance_dag`]). Trois
//! frontières y vivent :
//! - **Observation** ([`tap`]) — quatre adaptateurs convertissent les événements observés (appel modèle,
//!   message inter-agents, consultation outil/retrieval, revue humaine) en **nœuds scellables**. Les
//!   entrées sensibles sont **hachées** (minimisation GDPR) ; le **cas** est porté dans l'id de nœud
//!   (`{case}::{étape}`) — donc **scellé** (auditable) et **requêtable** — **sans** modifier le contrat S0.
//! - **Souveraineté** ([`guard`]) — egress refusé : seuls les endpoints **loopback** (durci : IPv4
//!   `127.0.0.0/8` littérale, pas un nom préfixé `127.`) sont permis ; les records persistent **localement**.
//! - **Provenance** ([`store`]) — un [`CaptureStore`] scelle au fil de l'eau (intégrité causale imposée),
//!   re-prouve, persiste local, et **requête** par cas / rôle.
//!
//! Dépend **uniquement** de `provenance-dag` (zéro dépendance crypto) → la chaîne de confiance reste mince.
//!
//! ```
//! use provenance_capture::{CaptureStore, LlmCall};
//! use provenance_dag::TenantKey;
//!
//! let mut store = CaptureStore::new(TenantKey::derive(b"root", "acme-corp"));
//! store.record_llm(&LlmCall {
//!     case_id: "SUP-2024", step: "sanction", agent_role: "sanction_check",
//!     inputs: b"nom: ACME", model: "phi4-mini", version: "1", output: "PASS",
//!     evidence_steps: &[], timestamp: 1,
//! }).unwrap();
//! assert!(store.reprove().ok);                 // le DAG capté est re-prouvable
//! assert_eq!(store.by_case("SUP-2024").len(), 1); // requêtable par cas
//! ```

/// **M6** — ce qu'on chiffre à la capture, et l'inventaire de ce qui survit à un broyage.
pub mod conservation;
pub mod guard;
pub mod store;
pub mod tap;

// `host_of` est exporté depuis P1 : un appelant qui JOURNALISE un endpoint doit pouvoir n'en nommer
// que l'hôte. Imprimer l'URL brute recopie son `userinfo` (`http://user:pass@hôte/`) dans la sortie
// de démarrage, où il sera collecté et archivé. Sans cet export, un appelant se rabat sur l'URL
// entière — ou réécrit un analyseur, ce qui est le défaut que P1 a fermé.
pub use conservation::{a_du_contenu_effacable, cas_de, enrober, enrober_par, lire, Balayage, Conservation, SansCle};
pub use guard::{assert_local, authority_of, host_of, is_local_endpoint, Authority, SovereigntyError};
pub use store::{CaptureError, CaptureStore, LoadError};
pub use tap::{hash_hex, node_id, HumanReview, LlmCall, MessageEvent, ToolCall};
