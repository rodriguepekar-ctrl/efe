//! Event sinks that consume M9 `RegistryEvent` streams and translate
//! them into metrics, logs, and audit entries.
//!
//! Sprint **D13** adds the [`d13_streams`] sub-module with strongly-
//! typed builders for the 5 D13 log streams (request, audit,
//! classify, evolution, security). Cf.
//! `docs/bench/d13_observability_dashboards_architecture.md` § 6.

pub mod d13_streams;
pub mod registry_sink;

pub use registry_sink::RegistrySink;
