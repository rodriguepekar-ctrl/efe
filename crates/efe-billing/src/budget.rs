// SPDX-License-Identifier: UNLICENSED
//! M31 — Budget alert evaluation.
//!
//! A `BudgetAlert` describes a per-tenant cap on monthly cost. When
//! the running total crosses the threshold, the evaluator emits a
//! `BudgetEvaluation::Exceeded` outcome which the caller (the
//! billing cron) translates into a NATS event + email / webhook.
//!
//! ## Threshold types
//!
//! - **Soft cap** : informational warning ; service continues.
//! - **Hard cap** : ratelimit kicks in via the M27 quota plane.

use serde::{Deserialize, Serialize};

use crate::error::{BillingError, Result};
use crate::pricing::PriceCents;

/// Notification channel for an alert.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NotificationChannel {
    /// Send an email.
    Email {
        /// Destination email address.
        to: String,
    },
    /// POST to a webhook.
    Webhook {
        /// Webhook URL.
        url: String,
    },
    /// Trigger a PagerDuty incident.
    Pagerduty {
        /// PagerDuty service id.
        service_id: String,
    },
}

/// Severity classifier for a budget alert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetSeverity {
    /// Soft cap ; informational.
    Soft,
    /// Hard cap ; service throttled.
    Hard,
}

/// One configured budget alert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetAlert {
    /// Alert identifier (UUIDv4).
    pub id: String,
    /// Tenant identifier.
    pub tenant_id: String,
    /// Threshold in cents.
    pub threshold_cents: PriceCents,
    /// Severity.
    pub severity: BudgetSeverity,
    /// Notification channels.
    pub channels: Vec<NotificationChannel>,
    /// Optional dismissal-until timestamp (ISO-8601 UTC).
    pub dismissed_until: Option<String>,
}

impl BudgetAlert {
    /// Validate the alert configuration. Returns `InvalidBudget` for
    /// malformed inputs.
    pub fn validate(&self) -> Result<()> {
        if self.tenant_id.is_empty() {
            return Err(BillingError::InvalidBudget("empty tenant_id".into()));
        }
        if self.threshold_cents == 0 {
            return Err(BillingError::InvalidBudget(
                "threshold must be > 0 cents".into(),
            ));
        }
        if self.channels.is_empty() {
            return Err(BillingError::InvalidBudget(
                "at least one channel required".into(),
            ));
        }
        for c in &self.channels {
            match c {
                NotificationChannel::Email { to } if to.is_empty() => {
                    return Err(BillingError::InvalidBudget("empty email".into()));
                }
                NotificationChannel::Webhook { url } if !url.starts_with("https://") => {
                    return Err(BillingError::InvalidBudget(
                        "webhook url must be HTTPS".into(),
                    ));
                }
                NotificationChannel::Pagerduty { service_id } if service_id.is_empty() => {
                    return Err(BillingError::InvalidBudget(
                        "empty pagerduty service id".into(),
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Returns `true` if the alert is currently suppressed (within
    /// the dismissal window).
    pub fn is_suppressed(&self, now_utc_iso: &str) -> bool {
        match &self.dismissed_until {
            Some(t) => now_utc_iso <= t.as_str(),
            None => false,
        }
    }
}

/// Outcome of a single alert evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetEvaluation {
    /// Threshold not crossed.
    UnderThreshold {
        /// Observed cost in cents.
        observed_cents: PriceCents,
        /// Configured threshold.
        threshold_cents: PriceCents,
    },
    /// Threshold crossed but the alert is dismissed.
    Suppressed {
        /// Observed cost.
        observed_cents: PriceCents,
    },
    /// Threshold crossed — fire notifications.
    Exceeded {
        /// Observed cost.
        observed_cents: PriceCents,
        /// Configured threshold.
        threshold_cents: PriceCents,
        /// Severity (hard / soft).
        severity: BudgetSeverity,
        /// Channels to notify (echoed for convenience).
        channels: Vec<NotificationChannel>,
    },
}

/// Evaluate a budget alert against an observed cost. `now_utc_iso`
/// is used to honor `dismissed_until`.
pub fn evaluate_budget(
    alert: &BudgetAlert,
    observed_cents: PriceCents,
    now_utc_iso: &str,
) -> Result<BudgetEvaluation> {
    alert.validate()?;
    if observed_cents < alert.threshold_cents {
        return Ok(BudgetEvaluation::UnderThreshold {
            observed_cents,
            threshold_cents: alert.threshold_cents,
        });
    }
    if alert.is_suppressed(now_utc_iso) {
        return Ok(BudgetEvaluation::Suppressed { observed_cents });
    }
    Ok(BudgetEvaluation::Exceeded {
        observed_cents,
        threshold_cents: alert.threshold_cents,
        severity: alert.severity,
        channels: alert.channels.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn email_alert(threshold: u64) -> BudgetAlert {
        BudgetAlert {
            id: "abc".into(),
            tenant_id: "acme".into(),
            threshold_cents: threshold,
            severity: BudgetSeverity::Soft,
            channels: vec![NotificationChannel::Email {
                to: "billing@acme.com".into(),
            }],
            dismissed_until: None,
        }
    }

    #[test]
    fn validate_rejects_empty_tenant() {
        let mut a = email_alert(1000);
        a.tenant_id = String::new();
        assert!(matches!(a.validate(), Err(BillingError::InvalidBudget(_))));
    }

    #[test]
    fn validate_rejects_zero_threshold() {
        let a = email_alert(0);
        assert!(matches!(a.validate(), Err(BillingError::InvalidBudget(_))));
    }

    #[test]
    fn validate_rejects_http_webhook() {
        let mut a = email_alert(1000);
        a.channels = vec![NotificationChannel::Webhook {
            url: "http://example.com".into(),
        }];
        assert!(matches!(a.validate(), Err(BillingError::InvalidBudget(_))));
    }

    #[test]
    fn evaluate_under_threshold_returns_under() {
        let a = email_alert(1000);
        let r = evaluate_budget(&a, 500, "2026-05-11T00:00:00Z").unwrap();
        assert!(matches!(r, BudgetEvaluation::UnderThreshold { .. }));
    }

    #[test]
    fn evaluate_threshold_crossed_returns_exceeded() {
        let a = email_alert(1000);
        let r = evaluate_budget(&a, 1500, "2026-05-11T00:00:00Z").unwrap();
        match r {
            BudgetEvaluation::Exceeded { observed_cents, severity, .. } => {
                assert_eq!(observed_cents, 1500);
                assert_eq!(severity, BudgetSeverity::Soft);
            }
            other => panic!("expected Exceeded, got {:?}", other),
        }
    }

    #[test]
    fn evaluate_suppressed_when_dismissed_window_active() {
        let mut a = email_alert(1000);
        a.dismissed_until = Some("2026-05-12T00:00:00Z".into());
        let r = evaluate_budget(&a, 1500, "2026-05-11T00:00:00Z").unwrap();
        assert!(matches!(r, BudgetEvaluation::Suppressed { .. }));
    }

    #[test]
    fn evaluate_not_suppressed_after_dismissal_expired() {
        let mut a = email_alert(1000);
        a.dismissed_until = Some("2026-05-10T00:00:00Z".into());
        let r = evaluate_budget(&a, 1500, "2026-05-11T00:00:00Z").unwrap();
        assert!(matches!(r, BudgetEvaluation::Exceeded { .. }));
    }
}
