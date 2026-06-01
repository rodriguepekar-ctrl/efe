//! M29 — Sub-processor list + change notification (GDPR Art. 28).
//!
//! Customers can :
//! - `GET /v1/subprocessors/list` (public-ish — also exposed unauth on
//!   trust center page).
//! - `POST /v1/subprocessors/subscribe` — webhook for change events.
//! - `GET /v1/subprocessors/changes` — paginated history.
//! - `POST /v1/subprocessors/ack/:event_id` — acknowledge a notification.

pub mod handlers;
pub use handlers::router;
