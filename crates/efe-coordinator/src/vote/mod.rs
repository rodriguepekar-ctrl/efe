// SPDX-License-Identifier: UNLICENSED
//! Vote engine — pattern Symphony (HF 2508.20019).
//!
//! N LLMs gros produisent chacun une réponse à la même question.
//! Chacun reçoit un poids `w_l` (par défaut uniforme).  Le `VoteEngine`
//! agrège leurs réponses et choisit le gagnant.
//!
//! ## Algorithmes supportés
//!
//! - **Weighted plurality** (Symphony) : la réponse qui a la plus grande
//!   somme de poids gagne ; tie-break déterministe.
//! - **Weighted quality** : on multiplie le poids par `self_quality` du
//!   provider.  Utile quand certains LLMs sont notoirement plus fiables
//!   sur certaines task_classes.
//!
//! ## Invariants
//!
//! - **I5** : pas de majorité ⇒ fallback sur le LLM ayant le plus haut
//!   `success_rate` historique (pas d'aléa).
//! - **I6** : `VoteEngine::tally` requiert ≥ 1 bulletin ; 0 ⇒ erreur.

mod engine;

pub use engine::{Ballot, VoteEngine, VoteOutcome};
