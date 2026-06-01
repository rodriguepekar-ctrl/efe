//! M29 — Audit log export sink management.
//!
//! Customers register a destination (S3/GCS/Azure/webhook) ; an
//! export worker pushes daily Parquet rollups + optional live tap.

pub mod handlers;
pub mod sink;
// Sprint Φ-D-STREAM (D82) — AuditStream reader for AdminService.ListAuditEvents.
// Cf. `docs/EFE_STREAMING_RPC_COMPLETION_ARCHITECTURE.md` § 8.
pub mod stream;
pub use handlers::router;
