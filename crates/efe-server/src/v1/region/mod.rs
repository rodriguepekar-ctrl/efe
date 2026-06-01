//! M30 — REST gateway for the multi-region control plane under
//! `/v1/region/*`.
//!
//! Routes :
//!
//! - `GET  /v1/region/health`     — Cross-region health probe target.
//!   Returns this region's `up`, `replication_lag_secs`,
//!   `ready_for_promote`. Hit by peer regions.
//! - `GET  /v1/region/current`    — Returns this region's identity +
//!   topology (operator endpoint).
//! - `POST /v1/region/failover`   — Operator-triggered failover.
//!   Validates preconditions, calls `pg_promote()` on the target.
//! - `POST /v1/region/promote`    — Called by a peer's failover
//!   orchestrator on **this** region : performs the actual
//!   `pg_promote()`. Authenticated via the same operator token.
//! - `POST /v1/region/drill`      — Run the quarterly DR drill
//!   harness (idempotent + ratelimited).
//!
//! The handler invariants per ADR-M30-008 are :
//!
//! 1. **All flips are audit-logged.** Every state change writes a
//!    HMAC-chained `agent_audit` row with the `operator` identifier.
//! 2. **Failover is rate-limited.** ≤ 1 failover every 5 minutes per
//!    region pair.
//! 3. **Health probes are unauthenticated** but require the
//!    `X-EFE-Region-Peer` header set to a known peer URL.

pub mod errors;
pub mod handlers;
pub mod state;

pub use errors::RegionApiError;
pub use handlers::router;
pub use state::RegionApiState;
