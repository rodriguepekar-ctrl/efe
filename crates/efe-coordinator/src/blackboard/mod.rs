// SPDX-License-Identifier: UNLICENSED
//! Blackboard stigmergique borné — pattern SwarmSys (HF 2510.10047).
//!
//! Une **blackboard** est une mémoire partagée entre tous les nano-robots
//! et tous les agents LLM d'un même swarm.  Les robots y déposent des
//! "phéromones" (notes typées : entité extraite, fact vérifié, claim
//! contradicté…) ; les autres robots peuvent les lire pour orienter leur
//! propre comportement (stigmergie).
//!
//! ## Bornage (invariant I2)
//!
//! - **Mémoire** : 256 Ko max ([`MAX_BLACKBOARD_BYTES`]).  Dépassement = erreur.
//! - **Entrées** : 4096 max ([`MAX_ENTRIES`]).  Au-delà → décrochage LRU.
//! - **TTL** : chaque phéromone décroît par décrément exponentiel
//!   (invariant I7 — `intensity` ne ré-augmente jamais hors dépôt).
//!
//! ## Concurrence
//!
//! Read-heavy + write-bounded : `parking_lot::RwLock<BTreeMap>` pour O(log n)
//! garanti et fairness.  Pas de `dashmap` ici car on a besoin d'itérations
//! ordonnées + bornage atomique.

mod entry;
mod pheromone;

pub use entry::{BlackboardEntry, EntryKind};
pub use pheromone::{PheromoneBlackboard, BlackboardSnapshot, BlackboardStats};

/// Borne dure de la blackboard (256 Ko).
///
/// Justification : 256 Ko = ~64K tokens GPT — plus que ce qu'un LLM gros
/// peut absorber en contexte raisonnable.  Au-delà, on est dans le "trop
/// pour être utile, pas assez pour être un cache".
pub const MAX_BLACKBOARD_BYTES: usize = 256 * 1024;

/// Borne dure du nombre d'entrées vivantes simultanées.
///
/// 4096 = 16 robots × 256 dépôts/robot, scénario worst case observé en
/// benchmark.
pub const MAX_ENTRIES: usize = 4096;

/// Demi-vie par défaut d'une phéromone (60 secondes).
pub const DEFAULT_HALF_LIFE_SECS: u64 = 60;

/// Seuil sous lequel une phéromone est garbage-collectée (intensité < 0.05).
pub const PHEROMONE_GC_THRESHOLD: f32 = 0.05;
