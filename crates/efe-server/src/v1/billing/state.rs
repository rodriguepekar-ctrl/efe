//! M31 — Handler state trait for `/v1/billing/*`.

use async_trait::async_trait;
use efe_billing::{StripeInvoice, StripeSubscription};

use super::errors::BillingApiError;

/// Tenant's current billing plan.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlanView {
    /// Plan canonical name.
    pub plan_name: String,
    /// Plan display name.
    pub display_name: String,
    /// Monthly base fee in cents.
    pub monthly_base_cents: u64,
}

/// Update payment method body.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct UpdatePaymentMethodBody {
    /// Stripe `pm_...` identifier obtained via Stripe.js.
    pub payment_method_id: String,
}

/// Plan switch body.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PlanSwitchBody {
    /// Target plan name.
    pub plan_name: String,
}

/// Billing handler state trait.
#[async_trait]
pub trait BillingApiState: Clone + Send + Sync + 'static {
    /// Authenticate a request.
    fn auth_tenant_from_request(&self, parts: &axum::http::request::Parts) -> Option<String>;

    /// `true` iff the caller has the `billing:write` scope.
    fn caller_has_billing_scope(&self, parts: &axum::http::request::Parts) -> bool;

    /// Read the tenant's active plan.
    async fn current_plan(&self, tenant: &str) -> Result<PlanView, BillingApiError>;

    /// Switch the tenant to a different plan.
    async fn switch_plan(&self, tenant: &str, plan_name: &str)
        -> Result<PlanView, BillingApiError>;

    /// List tenant's subscriptions.
    async fn list_subscriptions(
        &self,
        tenant: &str,
    ) -> Result<Vec<StripeSubscription>, BillingApiError>;

    /// List tenant's invoices.
    async fn list_invoices(&self, tenant: &str) -> Result<Vec<StripeInvoice>, BillingApiError>;

    /// Fetch one invoice.
    async fn get_invoice(
        &self,
        tenant: &str,
        invoice_id: &str,
    ) -> Result<StripeInvoice, BillingApiError>;

    /// Update default payment method.
    async fn update_payment_method(
        &self,
        tenant: &str,
        payment_method_id: &str,
    ) -> Result<(), BillingApiError>;

    /// Create a Customer Portal session.
    async fn create_portal_session(
        &self,
        tenant: &str,
        return_url: &str,
    ) -> Result<String, BillingApiError>;
}
