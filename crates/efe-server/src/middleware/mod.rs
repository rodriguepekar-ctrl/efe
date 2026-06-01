//! M27 — Tower middleware layers for the commercial edge.
//!
//! Three layers, applied in this order to the public router :
//!
//! ```text
//!     incoming request
//!         │
//!         ▼
//!   CorrelationIdLayer    (inject x-correlation-id if absent)
//!         │
//!         ▼
//!   BackpressureLayer     (concurrency cap + p99 shed)
//!         │
//!         ▼
//!   TenantQuotaLayer      (Redis-backed per-tenant counters)
//!         │
//!         ▼
//!     business logic
//! ```
//!
//! Each layer is independently feature-gated via env var so an operator
//! can disable any of them at runtime without re-deploying. Defaults
//! are sane for production (all ON).

pub mod backpressure;
/// Sprint **D19 (P1-Foundation)** — deadline propagation +
/// per-stage budget enforcement. Cf.
/// `docs/EFE_P1_SURFACE_ARCHITECTURE.md` § 7.2.
pub mod budget_tracker;
pub mod correlation_id;
pub mod tenant_quota;

pub use backpressure::{BackpressureConfig, BackpressureLayer};
pub use budget_tracker::{BudgetConfig, BudgetError, BudgetSnapshot, BudgetTracker};
pub use correlation_id::CorrelationIdLayer;
pub use tenant_quota::{TenantLimits, TenantQuotaConfig, TenantQuotaLayer};
