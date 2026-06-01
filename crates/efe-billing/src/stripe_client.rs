// SPDX-License-Identifier: UNLICENSED
//! M31 — Thin async Stripe REST client.
//!
//! Wraps a handful of Stripe API endpoints needed by M31 :
//! Customers, Subscriptions, Invoices, Invoice Items, Customer
//! Portal sessions. The full Stripe surface is huge ; M31 only
//! touches the slice covered by this module.
//!
//! ## Idempotency
//!
//! All mutating calls accept an `idempotency_key` parameter ; if the
//! caller passes the same key, Stripe returns the original response
//! without re-executing. This is critical because cron retries are
//! a primary use case.
//!
//! ## Errors
//!
//! All variants are typed by `BillingError`. Stripe-specific error
//! codes (e.g. `card_declined`) are preserved verbatim in the
//! `code` field of `StripeApi`.

use std::time::{Duration, Instant};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::error::{BillingError, Result};
use crate::metrics::{STRIPE_API_DURATION_MS, STRIPE_API_ERRORS_TOTAL};

/// Stripe client configuration.
#[derive(Debug, Clone)]
pub struct StripeClientConfig {
    /// Stripe API key (`sk_live_...` or `sk_test_...`).
    pub api_key: String,
    /// Base URL ; default `https://api.stripe.com`.
    pub base_url: String,
    /// Per-request deadline.
    pub request_timeout_ms: u64,
    /// Connect timeout.
    pub connect_timeout_ms: u64,
    /// User agent string.
    pub user_agent: String,
}

impl Default for StripeClientConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: "https://api.stripe.com".into(),
            request_timeout_ms: 8_000,
            connect_timeout_ms: 1_000,
            user_agent: "efe-billing/0.0.1".into(),
        }
    }
}

/// Stripe Customer object (subset).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StripeCustomer {
    /// `cus_...` identifier.
    pub id: String,
    /// Customer email (echoed from the create request).
    pub email: Option<String>,
    /// Free-text name.
    pub name: Option<String>,
    /// Stripe-internal metadata blob (we set `tenant_id` here).
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Stripe Subscription object (subset).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StripeSubscription {
    /// `sub_...` identifier.
    pub id: String,
    /// Customer ID.
    pub customer: String,
    /// Status (`active`, `past_due`, `canceled`, etc.).
    pub status: String,
    /// Current period start (Unix timestamp).
    pub current_period_start: i64,
    /// Current period end (Unix timestamp).
    pub current_period_end: i64,
}

/// Stripe invoice status (textual).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StripeInvoiceStatus {
    /// Draft (not yet finalized).
    Draft,
    /// Finalized but unpaid.
    Open,
    /// Paid.
    Paid,
    /// Voided.
    Void,
    /// Uncollectible (written off).
    Uncollectible,
}

/// Stripe Invoice object (subset).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StripeInvoice {
    /// `in_...` identifier.
    pub id: String,
    /// Customer ID.
    pub customer: String,
    /// Invoice status.
    pub status: StripeInvoiceStatus,
    /// Total amount in cents.
    pub total: i64,
    /// Currency (e.g. `usd`).
    pub currency: String,
    /// Hosted-invoice URL (PDF + payment portal).
    pub hosted_invoice_url: Option<String>,
    /// Period start (Unix timestamp).
    pub period_start: i64,
    /// Period end (Unix timestamp).
    pub period_end: i64,
}

/// Stripe HTTP client.
#[derive(Debug, Clone)]
pub struct StripeClient {
    inner: Client,
    cfg: StripeClientConfig,
}

impl StripeClient {
    /// Construct from configuration.
    pub fn new(cfg: StripeClientConfig) -> Result<Self> {
        if cfg.api_key.is_empty() {
            return Err(BillingError::Internal {
                reason: "stripe api_key empty".into(),
            });
        }
        let inner = Client::builder()
            .user_agent(&cfg.user_agent)
            .connect_timeout(Duration::from_millis(cfg.connect_timeout_ms))
            .build()
            .map_err(|e| BillingError::Internal {
                reason: format!("reqwest build: {e}"),
            })?;
        Ok(Self { inner, cfg })
    }

    /// Create a Stripe `Customer`. The `tenant_id` is stored in
    /// metadata so the webhook handler can route back to PG.
    pub async fn create_customer(
        &self,
        tenant_id: &str,
        email: Option<&str>,
        idempotency_key: &str,
    ) -> Result<StripeCustomer> {
        let mut params: Vec<(&str, String)> = vec![
            ("metadata[tenant_id]", tenant_id.to_string()),
            ("name", format!("EFE tenant {tenant_id}")),
        ];
        if let Some(e) = email {
            params.push(("email", e.to_string()));
        }
        self.post("/v1/customers", &params, Some(idempotency_key), "create_customer")
            .await
    }

    /// Read a `Customer` by ID.
    pub async fn get_customer(&self, customer_id: &str) -> Result<StripeCustomer> {
        self.get(&format!("/v1/customers/{customer_id}"), "get_customer").await
    }

    /// Create an invoice item (line item attached to the next
    /// pending invoice). For M31 usage-based billing we add one
    /// `InvoiceItem` per meter at month-end then call
    /// `create_invoice`.
    pub async fn create_invoice_item(
        &self,
        customer_id: &str,
        amount_cents: i64,
        currency: &str,
        description: &str,
        idempotency_key: &str,
    ) -> Result<serde_json::Value> {
        let params: Vec<(&str, String)> = vec![
            ("customer", customer_id.to_string()),
            ("amount", amount_cents.to_string()),
            ("currency", currency.to_string()),
            ("description", description.to_string()),
        ];
        self.post(
            "/v1/invoiceitems",
            &params,
            Some(idempotency_key),
            "create_invoice_item",
        )
        .await
    }

    /// Create an invoice from pending items.
    pub async fn create_invoice(
        &self,
        customer_id: &str,
        idempotency_key: &str,
    ) -> Result<StripeInvoice> {
        let params: Vec<(&str, String)> = vec![
            ("customer", customer_id.to_string()),
            ("auto_advance", "true".to_string()),
            ("collection_method", "charge_automatically".to_string()),
        ];
        self.post("/v1/invoices", &params, Some(idempotency_key), "create_invoice")
            .await
    }

    /// Finalize a draft invoice (transition to `open`).
    pub async fn finalize_invoice(
        &self,
        invoice_id: &str,
        idempotency_key: &str,
    ) -> Result<StripeInvoice> {
        let params: Vec<(&str, String)> = vec![];
        self.post(
            &format!("/v1/invoices/{invoice_id}/finalize"),
            &params,
            Some(idempotency_key),
            "finalize_invoice",
        )
        .await
    }

    /// List invoices for a customer (most recent first, capped at
    /// `limit` ≤ 100).
    pub async fn list_invoices(
        &self,
        customer_id: &str,
        limit: u32,
    ) -> Result<Vec<StripeInvoice>> {
        #[derive(Deserialize)]
        struct ListResp {
            data: Vec<StripeInvoice>,
        }
        let limit = limit.clamp(1, 100);
        let url = format!("/v1/invoices?customer={customer_id}&limit={limit}");
        let resp: ListResp = self.get(&url, "list_invoices").await?;
        Ok(resp.data)
    }

    /// Create a Customer Portal session URL.
    pub async fn create_portal_session(
        &self,
        customer_id: &str,
        return_url: &str,
        idempotency_key: &str,
    ) -> Result<String> {
        #[derive(Deserialize)]
        struct PortalResp {
            url: String,
        }
        let params: Vec<(&str, String)> = vec![
            ("customer", customer_id.to_string()),
            ("return_url", return_url.to_string()),
        ];
        let resp: PortalResp = self
            .post(
                "/v1/billing_portal/sessions",
                &params,
                Some(idempotency_key),
                "create_portal_session",
            )
            .await?;
        Ok(resp.url)
    }

    // ─────────────────────── INTERNAL HELPERS ────────────────────────

    async fn post<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        params: &[(&str, String)],
        idempotency_key: Option<&str>,
        op: &str,
    ) -> Result<T> {
        let url = format!("{}{}", self.cfg.base_url, path);
        let started = Instant::now();
        let mut req = self
            .inner
            .post(&url)
            .basic_auth(&self.cfg.api_key, Some(""))
            .form(params);
        if let Some(k) = idempotency_key {
            req = req.header("Idempotency-Key", k);
        }
        self.dispatch::<T>(req, op, started).await
    }

    async fn get<T: for<'de> Deserialize<'de>>(&self, path: &str, op: &str) -> Result<T> {
        let url = format!("{}{}", self.cfg.base_url, path);
        let started = Instant::now();
        let req = self
            .inner
            .get(&url)
            .basic_auth(&self.cfg.api_key, Some(""));
        self.dispatch::<T>(req, op, started).await
    }

    async fn dispatch<T: for<'de> Deserialize<'de>>(
        &self,
        req: reqwest::RequestBuilder,
        op: &str,
        started: Instant,
    ) -> Result<T> {
        let resp = tokio::time::timeout(
            Duration::from_millis(self.cfg.request_timeout_ms),
            req.send(),
        )
        .await;
        let resp = match resp {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                STRIPE_API_ERRORS_TOTAL
                    .with_label_values(&[op, "transport"])
                    .inc();
                warn!(error = %e, op = %op, "stripe transport error");
                STRIPE_API_DURATION_MS
                    .with_label_values(&[op, "err"])
                    .observe(started.elapsed().as_millis() as f64);
                return Err(BillingError::StripeTransport { source: e });
            }
            Err(_elapsed) => {
                STRIPE_API_ERRORS_TOTAL
                    .with_label_values(&[op, "timeout"])
                    .inc();
                STRIPE_API_DURATION_MS
                    .with_label_values(&[op, "err"])
                    .observe(self.cfg.request_timeout_ms as f64);
                return Err(BillingError::StripeTimeout {
                    elapsed_ms: self.cfg.request_timeout_ms,
                });
            }
        };
        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            STRIPE_API_ERRORS_TOTAL
                .with_label_values(&[op, "api"])
                .inc();
            // Parse Stripe's structured error envelope.
            #[derive(Deserialize)]
            struct StripeErrorEnv {
                error: StripeErrorBody,
            }
            #[derive(Deserialize)]
            struct StripeErrorBody {
                #[serde(default)]
                code: String,
                #[serde(default)]
                message: String,
            }
            let body_txt = resp.text().await.unwrap_or_default();
            let (code, msg) = match serde_json::from_str::<StripeErrorEnv>(&body_txt) {
                Ok(env) => (env.error.code, env.error.message),
                Err(_) => (String::new(), body_txt.chars().take(256).collect()),
            };
            STRIPE_API_DURATION_MS
                .with_label_values(&[op, "err"])
                .observe(started.elapsed().as_millis() as f64);
            return Err(BillingError::StripeApi {
                status,
                code,
                sanitized_msg: msg,
            });
        }
        let body: T = resp.json().await.map_err(|e| BillingError::StripeApi {
            status,
            code: "decode_error".into(),
            sanitized_msg: format!("body decode: {e}"),
        })?;
        STRIPE_API_DURATION_MS
            .with_label_values(&[op, "ok"])
            .observe(started.elapsed().as_millis() as f64);
        debug!(op = %op, elapsed_ms = started.elapsed().as_millis() as u64, "stripe ok");
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_secure_defaults() {
        let c = StripeClientConfig::default();
        assert_eq!(c.base_url, "https://api.stripe.com");
        assert!(c.request_timeout_ms >= 1_000);
    }

    #[test]
    fn build_fails_with_empty_api_key() {
        let cfg = StripeClientConfig::default();
        let err = StripeClient::new(cfg).unwrap_err();
        assert!(matches!(err, BillingError::Internal { .. }));
    }

    #[test]
    fn build_succeeds_with_test_key() {
        let cfg = StripeClientConfig {
            api_key: "sk_test_abc".into(),
            ..Default::default()
        };
        let _ = StripeClient::new(cfg).unwrap();
    }

    #[tokio::test]
    async fn unreachable_stripe_returns_typed_error() {
        let cfg = StripeClientConfig {
            api_key: "sk_test_abc".into(),
            base_url: "http://127.0.0.1:1".into(),
            request_timeout_ms: 150,
            connect_timeout_ms: 50,
            ..Default::default()
        };
        let client = StripeClient::new(cfg).unwrap();
        let err = client
            .create_customer("acme", Some("a@b.c"), "idemp-1")
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            BillingError::StripeTransport { .. } | BillingError::StripeTimeout { .. }
        ));
    }

    #[test]
    fn invoice_status_serde_roundtrip() {
        let s = serde_json::to_string(&StripeInvoiceStatus::Paid).unwrap();
        assert_eq!(s, "\"paid\"");
        let parsed: StripeInvoiceStatus = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed, StripeInvoiceStatus::Paid);
    }
}
