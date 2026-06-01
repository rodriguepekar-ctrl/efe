//! M28 — Async batch classify subsystem.
//!
//! Customer submits an S3 URL → worker reads in 1000-row chunks →
//! writes Parquet result to S3 → publishes `batch.completed` event.
//!
//! Submodules :
//! - [`handlers`] : axum REST handlers.
//! - [`s3`] : S3 helpers (URL parsing, presigning, allow-list checks).
//! - [`job`] : DB row + lifecycle state machine.
//! - [`worker`] : worker entry point (called by `efe-batch-worker` bin).

pub mod handlers;
pub mod job;
pub mod s3;
pub mod worker;
