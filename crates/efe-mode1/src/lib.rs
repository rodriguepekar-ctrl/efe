// SPDX-License-Identifier: UNLICENSED
//! # `efe-mode1` — l'assemblage Mode-1, en crate **feuille**.
//!
//! ## Le fait mesuré qui a produit ce crate
//!
//! Servir de *vrais* participants dans le méta-essaim exige la feature `core-integration`. Mesurée,
//! elle faisait passer la fermeture d'`efe-meta-swarm-server` de **8 à 27 crates du dépôt** : elle
//! tirait `efe-server`, et avec lui `efe-billing`, `efe-graphrag`, `efe-kg`, `efe-llm-router`,
//! `efe-multimodal`, `efe-pii`, `efe-vector`, `efe-retrieval`, `efe-curation`, `efe-agent-runner`,
//! `efe-cache`, `efe-events`, `efe-kernels`, `efe-metering`, `efe-prompts`, `efe-region`,
//! `efe-telemetry` — une pile applicative entière, pour **une** fonction :
//! [`mode1::build_mvp_controller`].
//!
//! ## Pourquoi l'extraction était mécanique
//!
//! `mode1.rs` fait 1 979 lignes et ne contenait qu'**un seul** `use crate::` : les deux items de
//! `efe_server::services`, désormais dans [`backend`]. Tout le reste ne parle qu'à `efe-core`. Le
//! fichier a donc déménagé **verbatim** — une seule ligne modifiée, celle de cet import — ce qui
//! préserve intégralement ses liens de documentation internes et rend le déplacement relisible
//! d'un `git log --follow`.
//!
//! ## Ce que ce crate achète, et ce qu'il n'achète pas
//!
//! Ni performance, ni sécurité : la **cohérence entre le discours et l'artefact**. Le manifeste du
//! méta-essaim affirme que « le graphe de dépendances par défaut porte ZÉRO crate
//! `efe-core`/`efe-server` » — ce qui était vrai par défaut et cessait de l'être **dès qu'on
//! servait**. Un produit qui se présente comme un périmètre scellé ne peut pas avoir une feature de
//! production qui triple son périmètre.
//!
//! ## La forme : une feuille au sens strict
//!
//! Trois dépendances, dont **une seule** du dépôt — `efe-core`, dont la fermeture propre vaut 1
//! (elle ne traîne rien). C'est ce qui rend l'arithmétique du critère atteignable : 8 par défaut,
//! plus `efe-core`, plus ce crate = **10**, et le critère de R1 en demande « au plus dix ».
//!
//! ## Les deux portes
//!
//! * [`mode1::build_mvp_controller`] rend un `Arc<SwarmController>` nu — c'est ce que le
//!   méta-essaim consomme, et il ne touche aucun trait de `backend`.
//! * [`mode1::build_mvp_backend`] rend un `Arc<dyn ExecuteBackend>` — c'est le chemin de requête du
//!   serveur, et c'est la seule fonction qui ait jamais eu besoin de `efe_server::services`.
//!
//! Les deux partagent `build_mvp_parts`, qui est du pur `efe-core`.

pub mod backend;
pub mod mode1;

pub use backend::{ExecuteBackend, SwarmControllerBackend};
