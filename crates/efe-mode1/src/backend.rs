// SPDX-License-Identifier: UNLICENSED
//! Le **dos** de l'assemblage Mode-1 : l'abstraction d'exécution, et l'adaptateur qui y branche un
//! contrôleur concret.
//!
//! ## Pourquoi ces deux items vivent ici et non dans `efe-server`
//!
//! Ils en viennent. `mode1.rs` faisait 1 979 lignes et ne contenait qu'**un seul** `use crate::` :
//! celui qui les importait. C'est cette unique ligne qui attachait tout l'assemblage Mode-1 à
//! `efe-server`, et par lui à dix-neuf crates d'une pile applicative que le méta-essaim n'a jamais
//! eu à connaître.
//!
//! Les déplacer coûte peu : ils ne dépendent que d'`efe-core` et d'`async-trait`. `efe-server` les
//! ré-exporte depuis `services`, si bien qu'aucun de ses quarante sites d'appel ne change.
//!
//! ## Une dette que ce déplacement RÉVÈLE sans la payer
//!
//! `efe-core` possède **déjà** un trait `ExecuteBackend` de signature identique
//! (`swarm/backend.rs`), et y implémente `ExecuteBackend for SwarmController` — ce qui rend
//! [`SwarmControllerBackend`] redondant. Le dépôt le sait et l'a écrit : *« l'adaptateur
//! `efe_server::services::SwarmControllerBackend` devient redondant ; sa suppression est
//! `DT-C-AUDIT-CLEANUP-1` »*.
//!
//! R1 **déplace** ; il n'unifie pas. Unifier voudrait dire remplacer le trait par un ré-export
//! d'`efe-core` et supprimer l'adaptateur, ce qui traverse une quarantaine de sites sur le chemin
//! de requête du serveur — un autre travail, avec son propre risque, et son ticket porte déjà un
//! nom. Le signaler ici plutôt que de le faire en passant, c'est la différence entre une dette
//! documentée et une dette qui se découvre deux fois.

use async_trait::async_trait;
use efe_core::{Request, Response, Result, SwarmController};

/// Minimal abstraction over the swarm controller. Present because
/// `SwarmController` is a concrete struct in `efe-core`; wrapping it in
/// a trait lets us inject a stub backend in tests without rebuilding
/// the entire strategy pipeline.
///
/// The trait intentionally does **not** require `Debug` — the real
/// [`SwarmController`] does not derive it. Components that need to
/// log the backend implement their own opaque `Debug`.
#[async_trait]
pub trait ExecuteBackend: Send + Sync {
    /// Execute a domain request end-to-end.
    async fn execute(&self, req: Request) -> Result<Response>;
}

/// Adapter around the concrete [`SwarmController`].
///
/// Use this when wiring a production server:
/// `Server::builder().backend(Arc::new(SwarmControllerBackend::new(controller)))`.
pub struct SwarmControllerBackend {
    controller: SwarmController,
}

impl SwarmControllerBackend {
    /// Wrap a pre-built controller.
    #[must_use]
    pub fn new(controller: SwarmController) -> Self {
        Self { controller }
    }
}

#[async_trait]
impl ExecuteBackend for SwarmControllerBackend {
    async fn execute(&self, req: Request) -> Result<Response> {
        self.controller.execute(req).await
    }
}
