//! M29 — GDPR Art. 17 (erasure) + Art. 20 (portability) handlers.
//!
//! Sub-modules :
//! - [`erasure`] : `DELETE /v1/tenants/:slug` + MFA challenge + 7d grace.
//! - [`portability`] : `POST /v1/tenants/:slug/export` + status + download.
//! - [`state`] : strongly-typed state machine for both flows.

pub mod erasure;
pub mod portability;
pub mod state;
