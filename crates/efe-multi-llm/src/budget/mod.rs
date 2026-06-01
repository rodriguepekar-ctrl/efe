// SPDX-License-Identifier: UNLICENSED
//! `BudgetTracker` — invariant I13 : try_charge atomique + revert.

mod tracker;
pub use tracker::{BudgetSnapshot, BudgetTracker};
