// SPDX-License-Identifier: UNLICENSED
//! M31 — Billing error taxonomy.

use thiserror::Error;

/// Canonical billing error type.
#[derive(Debug, Error)]
pub enum BillingError {
    /// Pricing plan unknown to the cache.
    #[error("unknown pricing plan: {0}")]
    UnknownPlan(String),

    /// No price configured for a meter under the active plan.
    #[error("meter {meter} not priced under plan {plan}")]
    MeterNotPriced {
        /// Meter name.
        meter: String,
        /// Plan name.
        plan: String,
    },

    /// Budget rule expression invalid.
    #[error("invalid budget: {0}")]
    InvalidBudget(String),

    /// Stripe API transport error.
    #[error("stripe transport: {source}")]
    StripeTransport {
        /// Underlying reqwest error.
        #[source]
        source: reqwest::Error,
    },

    /// Stripe API returned a non-2xx response.
    #[error("stripe api: status {status}, code {code}, msg {sanitized_msg}")]
    StripeApi {
        /// HTTP status code.
        status: u16,
        /// Stripe error code (e.g. `card_declined`).
        code: String,
        /// Sanitized error message (no card data).
        sanitized_msg: String,
    },

    /// Stripe API timed out.
    #[error("stripe timeout after {elapsed_ms} ms")]
    StripeTimeout {
        /// Elapsed wall-clock milliseconds.
        elapsed_ms: u64,
    },

    /// Webhook signature mismatch.
    #[error("webhook signature invalid")]
    WebhookSignatureInvalid,

    /// Webhook payload could not be decoded.
    #[error("webhook payload: {0}")]
    WebhookPayload(String),

    /// Webhook timestamp outside tolerance window.
    #[error("webhook timestamp drift: {drift_secs}s > tolerance {tolerance_secs}s")]
    WebhookTimestampDrift {
        /// Observed drift in seconds.
        drift_secs: i64,
        /// Configured tolerance.
        tolerance_secs: i64,
    },

    /// Reconciliation drift detected (local mirror vs Stripe).
    #[error("reconciliation drift: {0}")]
    Drift(String),

    /// Catch-all internal error.
    #[error("internal: {reason}")]
    Internal {
        /// Sanitized reason.
        reason: String,
    },
}

impl BillingError {
    /// Stable string identifier.
    #[must_use]
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::UnknownPlan(_) => "billing.unknown_plan",
            Self::MeterNotPriced { .. } => "billing.meter_not_priced",
            Self::InvalidBudget(_) => "billing.invalid_budget",
            Self::StripeTransport { .. } => "billing.stripe_transport",
            Self::StripeApi { .. } => "billing.stripe_api",
            Self::StripeTimeout { .. } => "billing.stripe_timeout",
            Self::WebhookSignatureInvalid => "billing.webhook_sig_invalid",
            Self::WebhookPayload(_) => "billing.webhook_payload",
            Self::WebhookTimestampDrift { .. } => "billing.webhook_drift",
            Self::Drift(_) => "billing.drift",
            Self::Internal { .. } => "billing.internal",
        }
    }

    /// Recommended HTTP status code.
    #[must_use]
    pub fn http_status(&self) -> u16 {
        match self {
            Self::UnknownPlan(_) | Self::MeterNotPriced { .. } | Self::InvalidBudget(_) => 422,
            Self::WebhookSignatureInvalid | Self::WebhookTimestampDrift { .. } => 400,
            Self::WebhookPayload(_) => 400,
            Self::StripeTransport { .. } => 503,
            Self::StripeTimeout { .. } => 504,
            Self::StripeApi { status, .. } => *status,
            Self::Drift(_) => 409,
            Self::Internal { .. } => 500,
        }
    }

    /// `true` iff retrying may succeed.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::StripeTransport { .. } | Self::StripeTimeout { .. }
        )
    }
}

/// Result alias for the crate.
pub type Result<T> = std::result::Result<T, BillingError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable() {
        assert_eq!(
            BillingError::UnknownPlan("x".into()).error_code(),
            "billing.unknown_plan"
        );
        assert_eq!(
            BillingError::WebhookSignatureInvalid.error_code(),
            "billing.webhook_sig_invalid"
        );
    }

    #[test]
    fn http_mapping() {
        assert_eq!(BillingError::WebhookSignatureInvalid.http_status(), 400);
        assert_eq!(BillingError::Drift("x".into()).http_status(), 409);
        assert_eq!(
            BillingError::StripeApi {
                status: 402,
                code: "card_declined".into(),
                sanitized_msg: "x".into(),
            }
            .http_status(),
            402
        );
    }

    #[test]
    fn retryable_set_minimal() {
        assert!(BillingError::StripeTimeout { elapsed_ms: 1 }.is_retryable());
        assert!(!BillingError::WebhookSignatureInvalid.is_retryable());
    }
}
