// SPDX-License-Identifier: UNLICENSED
//! Algos de vote Phase 3 — Borda + Condorcet (extension du Plurality Phase 2).

pub mod borda;
pub mod condorcet;

pub use borda::{borda_tally, BordaBallot, BordaOutcome};
pub use condorcet::{condorcet_tally, CondorcetOutcome};
