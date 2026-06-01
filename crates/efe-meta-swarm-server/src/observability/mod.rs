//! Cross-cutting observability wiring.
//!
//!   * [`health`]   — shared `HealthState` (atomics, no Mutex).
//!   * [`metrics`]  — Prometheus registry (PR #12 adds the four
//!     `efe_meta_swarm_reload_*` series) + `/metrics` HTTP server.
//!   * [`tracing`]  — `tracing-subscriber` init.
//!   * [`shutdown`] — `await_termination` (SIGTERM / SIGINT) +
//!     PR #12's `install_sighup_handler` (SIGHUP, manifest reload,
//!     with `Notify`-based coalescing for concurrent signals).

pub mod health;
pub mod metrics;
pub mod shutdown;
pub mod tracing;
