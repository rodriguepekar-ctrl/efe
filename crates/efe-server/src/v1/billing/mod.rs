//! M31 — `/v1/billing/*` Stripe-backed self-service endpoints.

pub mod errors;
pub mod handlers;
pub mod state;
pub mod webhook;

pub use errors::BillingApiError;
pub use handlers::router;
pub use state::BillingApiState;
pub use webhook::{stripe_webhook, HandleOutcome, WebhookHandler, WebhookState};
