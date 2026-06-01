//! M28 — Customer-facing event log API.
//!
//! `GET /v1/events?since=<ts>&until=<ts>&topic=<filter>&limit=100&page_token=<...>`
//!
//! Returns paginated events from the tenant's recent event log
//! (mirrored from NATS into `customer_event_log` PG table, retention
//! 30 days). For older events the customer issues an export job
//! (M29 RGPD Art. 20).

pub mod handlers;
pub mod sse;
pub use handlers::{router, EventsHandlerState};
