// SPDX-License-Identifier: UNLICENSED
//! Speculative RAG (HF 2407.08223).
//!
//! Pendant qu'on attend la réponse du gros LLM, un petit drafter
//! génère une réponse anticipée.  Quand le gros LLM répond, il
//! vérifie/ré-écrit le draft : si le draft est OK, on rend la
//! réponse drafted (faster perceived latency).
//!
//! ## Composants
//!
//! - [`Speculator::draft`] : appel non-bloquant qui démarre le drafter.
//! - [`Speculator::verify`] : appel bloquant qui attend le gros LLM
//!   et arbitre.
//! - [`SpeculationOutcome`] : résultat final.

mod noise;
mod speculator;

pub use noise::{NoiseRejectionPolicy, NoiseSignal};
pub use speculator::{DraftToken, SpeculationOutcome, Speculator};
