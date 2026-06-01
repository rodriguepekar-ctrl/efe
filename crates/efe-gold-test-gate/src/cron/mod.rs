// SPDX-License-Identifier: UNLICENSED
//! **Sprint 12 — N20** : Cron-driven drift sweep — § 7.6 + G8 + G9.

pub mod drift_sweep;

pub use drift_sweep::{
    DriftSweepCron, DriftSweepCronConfig, DriftSweepState, DriftSweepStateEntry,
    InMemoryDriftSweepState,
};
