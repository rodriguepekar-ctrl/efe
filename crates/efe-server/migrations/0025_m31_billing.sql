-- ===========================================================================
-- M31 — FinOps + Cost Governance + Stripe Billing.
--
-- Adds :
--   1. pricing_plan                — shared catalogue of plans.
--   2. pricing_plan_meter          — per-meter unit price under each plan.
--   3. tenant_billing              — per-tenant Stripe linkage + plan.
--   4. usage_rollup_hour           — hourly per-meter rollup.
--   5. usage_rollup_day            — daily per-meter rollup.
--   6. billing_subscription        — mirror of Stripe Subscription.
--   7. billing_invoice             — mirror of Stripe Invoice.
--   8. billing_invoice_item        — denormalized line items.
--   9. budget_alert                — per-tenant cap configuration.
--  10. budget_alert_event          — per-fire log (debounce + audit).
--  11. payment_method              — Stripe payment-method mirror (no PAN).
--  12. stripe_webhook_event        — dedup log of inbound webhooks.
--  13. meter_event                 — 30-day rolling event log (mirror).
--
--  + 1 column on `tenant_feature` (`billing_enabled`)
--  + 3 audit enum values.
-- ===========================================================================

BEGIN;

-- ── 1. pricing_plan ────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS pricing_plan (
    name                        VARCHAR(32) PRIMARY KEY,
    display_name                TEXT NOT NULL,
    monthly_base_cents          BIGINT NOT NULL DEFAULT 0
                                CHECK (monthly_base_cents >= 0),
    stripe_product_id           TEXT,
    is_active                   BOOLEAN NOT NULL DEFAULT TRUE,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
COMMENT ON TABLE pricing_plan IS
    'M31 — Canonical pricing plans (free / starter / pro / enterprise).';

-- ── 2. pricing_plan_meter ─────────────────────────────────────────
CREATE TABLE IF NOT EXISTS pricing_plan_meter (
    plan_name                   VARCHAR(32) NOT NULL REFERENCES pricing_plan (name) ON DELETE CASCADE,
    meter                       VARCHAR(40) NOT NULL,
    free_units                  BIGINT NOT NULL DEFAULT 0 CHECK (free_units >= 0),
    price_cents_per_unit        BIGINT NOT NULL DEFAULT 0 CHECK (price_cents_per_unit >= 0),
    stripe_price_id             TEXT,
    PRIMARY KEY (plan_name, meter)
);
COMMENT ON TABLE pricing_plan_meter IS
    'M31 — Unit price per meter under a plan.';

-- ── 3. tenant_billing ──────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS tenant_billing (
    tenant_id                   TEXT PRIMARY KEY REFERENCES tenant_feature (id) ON DELETE CASCADE,
    plan_name                   VARCHAR(32) NOT NULL REFERENCES pricing_plan (name),
    stripe_customer_id          TEXT UNIQUE,
    billing_email               TEXT,
    activated_at                TIMESTAMPTZ,
    updated_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
COMMENT ON TABLE tenant_billing IS
    'M31 — Per-tenant billing linkage. 1:1 with tenant_feature.';

CREATE INDEX IF NOT EXISTS idx_tenant_billing_customer
    ON tenant_billing (stripe_customer_id);

-- ── 4. usage_rollup_hour ───────────────────────────────────────────
CREATE TABLE IF NOT EXISTS usage_rollup_hour (
    tenant_id                   TEXT NOT NULL REFERENCES tenant_feature (id) ON DELETE CASCADE,
    period_start                TIMESTAMPTZ NOT NULL,
    meter                       VARCHAR(40) NOT NULL,
    value                       BIGINT NOT NULL CHECK (value >= 0),
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, period_start, meter)
);
COMMENT ON TABLE usage_rollup_hour IS
    'M31 — Per-tenant hourly rollups. Source of truth for billing.';

CREATE INDEX IF NOT EXISTS idx_usage_rollup_hour_tenant_time
    ON usage_rollup_hour (tenant_id, period_start DESC);

-- ── 5. usage_rollup_day ───────────────────────────────────────────
CREATE TABLE IF NOT EXISTS usage_rollup_day (
    tenant_id                   TEXT NOT NULL REFERENCES tenant_feature (id) ON DELETE CASCADE,
    period_start                DATE NOT NULL,
    meter                       VARCHAR(40) NOT NULL,
    value                       BIGINT NOT NULL CHECK (value >= 0),
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, period_start, meter)
);
COMMENT ON TABLE usage_rollup_day IS
    'M31 — Per-tenant daily rollups. Driven by usage_rollup_hour.';

CREATE INDEX IF NOT EXISTS idx_usage_rollup_day_tenant_time
    ON usage_rollup_day (tenant_id, period_start DESC);

-- ── 6. billing_subscription ───────────────────────────────────────
CREATE TABLE IF NOT EXISTS billing_subscription (
    id                          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id                   TEXT NOT NULL REFERENCES tenant_feature (id) ON DELETE CASCADE,
    stripe_subscription_id      TEXT UNIQUE NOT NULL,
    status                      VARCHAR(16) NOT NULL
                                CHECK (status IN (
                                    'active','past_due','canceled','incomplete',
                                    'incomplete_expired','trialing','unpaid','paused'
                                )),
    current_period_start        TIMESTAMPTZ NOT NULL,
    current_period_end          TIMESTAMPTZ NOT NULL,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
COMMENT ON TABLE billing_subscription IS
    'M31 — Local mirror of Stripe Subscription objects.';

CREATE INDEX IF NOT EXISTS idx_billing_subscription_tenant
    ON billing_subscription (tenant_id, status);

-- ── 7. billing_invoice ────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS billing_invoice (
    id                          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id                   TEXT NOT NULL REFERENCES tenant_feature (id) ON DELETE CASCADE,
    stripe_invoice_id           TEXT UNIQUE NOT NULL,
    status                      VARCHAR(16) NOT NULL
                                CHECK (status IN ('draft','open','paid','void','uncollectible','pending_stripe')),
    total_cents                 BIGINT NOT NULL CHECK (total_cents >= 0),
    currency                    VARCHAR(3) NOT NULL DEFAULT 'usd',
    period_start                TIMESTAMPTZ NOT NULL,
    period_end                  TIMESTAMPTZ NOT NULL,
    hosted_invoice_url          TEXT,
    finalized_at                TIMESTAMPTZ,
    paid_at                     TIMESTAMPTZ,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
COMMENT ON TABLE billing_invoice IS
    'M31 — Local mirror of Stripe Invoice objects.';

CREATE INDEX IF NOT EXISTS idx_billing_invoice_tenant_time
    ON billing_invoice (tenant_id, period_end DESC);
CREATE INDEX IF NOT EXISTS idx_billing_invoice_status
    ON billing_invoice (status, finalized_at DESC);

-- ── 8. billing_invoice_item ───────────────────────────────────────
CREATE TABLE IF NOT EXISTS billing_invoice_item (
    id                          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    invoice_id                  UUID NOT NULL REFERENCES billing_invoice (id) ON DELETE CASCADE,
    meter                       VARCHAR(40) NOT NULL,
    units                       BIGINT NOT NULL CHECK (units >= 0),
    amount_cents                BIGINT NOT NULL CHECK (amount_cents >= 0),
    description                 TEXT NOT NULL,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
COMMENT ON TABLE billing_invoice_item IS
    'M31 — Denormalized invoice line items (one row per meter).';

CREATE INDEX IF NOT EXISTS idx_billing_invoice_item_invoice
    ON billing_invoice_item (invoice_id);

-- ── 9. budget_alert ───────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS budget_alert (
    id                          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id                   TEXT NOT NULL REFERENCES tenant_feature (id) ON DELETE CASCADE,
    threshold_cents             BIGINT NOT NULL CHECK (threshold_cents > 0),
    severity                    VARCHAR(8) NOT NULL CHECK (severity IN ('soft','hard')),
    channels                    JSONB NOT NULL,
    dismissed_until             TIMESTAMPTZ,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
COMMENT ON TABLE budget_alert IS
    'M31 — Per-tenant soft/hard caps with notification channels.';

CREATE INDEX IF NOT EXISTS idx_budget_alert_tenant
    ON budget_alert (tenant_id);

-- ── 10. budget_alert_event ────────────────────────────────────────
CREATE TABLE IF NOT EXISTS budget_alert_event (
    id                          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    alert_id                    UUID NOT NULL REFERENCES budget_alert (id) ON DELETE CASCADE,
    observed_cents              BIGINT NOT NULL CHECK (observed_cents >= 0),
    severity                    VARCHAR(8) NOT NULL CHECK (severity IN ('soft','hard')),
    fired_at                    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    delivery_status             VARCHAR(16) NOT NULL DEFAULT 'queued'
                                CHECK (delivery_status IN ('queued','delivered','failed'))
);
COMMENT ON TABLE budget_alert_event IS
    'M31 — Append-only log of budget alert firings.';

CREATE INDEX IF NOT EXISTS idx_budget_alert_event_fired
    ON budget_alert_event (alert_id, fired_at DESC);

-- ── 11. payment_method ────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS payment_method (
    id                          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id                   TEXT NOT NULL REFERENCES tenant_feature (id) ON DELETE CASCADE,
    stripe_payment_method_id    TEXT UNIQUE NOT NULL,
    kind                        VARCHAR(16) NOT NULL
                                CHECK (kind IN ('card','sepa','bacs','ach')),
    -- Card-specific fields ; we NEVER store PAN. last4 + brand only.
    card_brand                  VARCHAR(16),
    card_last4                  VARCHAR(4),
    card_exp_month              SMALLINT CHECK (card_exp_month BETWEEN 1 AND 12),
    card_exp_year               SMALLINT CHECK (card_exp_year BETWEEN 2024 AND 2100),
    is_default                  BOOLEAN NOT NULL DEFAULT FALSE,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    detached_at                 TIMESTAMPTZ
);
COMMENT ON TABLE payment_method IS
    'M31 — Stripe payment method mirror. NEVER stores PAN.';

CREATE INDEX IF NOT EXISTS idx_payment_method_tenant
    ON payment_method (tenant_id, is_default);

-- ── 12. stripe_webhook_event ──────────────────────────────────────
CREATE TABLE IF NOT EXISTS stripe_webhook_event (
    stripe_event_id             TEXT PRIMARY KEY,           -- `evt_...`
    event_type                  VARCHAR(64) NOT NULL,
    received_at                 TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    processed_at                TIMESTAMPTZ,
    processing_status           VARCHAR(16) NOT NULL DEFAULT 'received'
                                CHECK (processing_status IN ('received','processed','failed','skipped')),
    error_msg                   TEXT,
    raw_payload                 JSONB NOT NULL
);
COMMENT ON TABLE stripe_webhook_event IS
    'M31 — Dedup log of inbound Stripe webhook events.';

CREATE INDEX IF NOT EXISTS idx_stripe_webhook_event_type_time
    ON stripe_webhook_event (event_type, received_at DESC);

-- ── 13. meter_event ───────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS meter_event (
    id                          BIGSERIAL PRIMARY KEY,
    tenant_id                   TEXT NOT NULL REFERENCES tenant_feature (id) ON DELETE CASCADE,
    meter                       VARCHAR(40) NOT NULL,
    delta                       BIGINT NOT NULL CHECK (delta > 0),
    bucket                      VARCHAR(10) NOT NULL,        -- yyyymmddhh
    observed_at                 TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    nats_subject                VARCHAR(64)
);
COMMENT ON TABLE meter_event IS
    'M31 — 30-day rolling event log mirrored from NATS meter.bumped.';

CREATE INDEX IF NOT EXISTS idx_meter_event_tenant_time
    ON meter_event (tenant_id, observed_at DESC);

-- ── 14. tenant_feature additions ───────────────────────────────────
ALTER TABLE tenant_feature
    ADD COLUMN IF NOT EXISTS billing_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS metering_enabled BOOLEAN NOT NULL DEFAULT FALSE;
COMMENT ON COLUMN tenant_feature.billing_enabled IS
    'M31 — Master switch ; tenants with this OFF stay on hand-rolled invoicing.';
COMMENT ON COLUMN tenant_feature.metering_enabled IS
    'M31 — When OFF, meter_inc calls become silent no-ops (used during pilot phases).';

-- ── 15. agent_audit enum extensions ────────────────────────────────
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'BillingPlanChanged') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'BillingPlanChanged';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'BudgetExceeded') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'BudgetExceeded';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'InvoiceFinalized') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'InvoiceFinalized';
    END IF;
END$$;

-- ── 16. seed the default pricing plans ─────────────────────────────
INSERT INTO pricing_plan (name, display_name, monthly_base_cents) VALUES
    ('free',       'Free',       0),
    ('starter',    'Starter',    4900),
    ('pro',        'Pro',        49900),
    ('enterprise', 'Enterprise', 0)  -- per-customer quote
ON CONFLICT (name) DO NOTHING;

INSERT INTO pricing_plan_meter (plan_name, meter, free_units, price_cents_per_unit) VALUES
    -- Free plan : 10k classify free, rest gated by quota (M27).
    ('free',    'classify_requests',           10000, 0),
    -- Starter : 50k classify free, $0.001/req after.
    ('starter', 'classify_requests',           50000, 1),
    ('starter', 'agent_run_seconds',               0, 1),
    ('starter', 'multimodal_vision_requests',      0, 5),
    -- Pro : 500k free, same per-unit but bigger base.
    ('pro',     'classify_requests',          500000, 1),
    ('pro',     'agent_run_seconds',               0, 1),
    ('pro',     'multimodal_vision_requests',      0, 3)
ON CONFLICT (plan_name, meter) DO NOTHING;

-- ── 17. View : per-tenant current-month rollup ────────────────────
CREATE OR REPLACE VIEW v_tenant_month_to_date AS
SELECT urd.tenant_id,
       date_trunc('month', urd.period_start)::date AS month_start,
       urd.meter,
       SUM(urd.value) AS units
  FROM usage_rollup_day urd
 WHERE urd.period_start >= date_trunc('month', CURRENT_DATE)
 GROUP BY 1, 2, 3
 ORDER BY 1, 3;

COMMENT ON VIEW v_tenant_month_to_date IS
    'M31 — Current-month rollups per tenant + meter (used by projection).';

COMMIT;
