-- ===========================================================================
-- M31 — Rollback for 0025_m31_billing.sql.
--
-- Drops every artifact in reverse order. Does NOT drop the enum
-- values added to `agent_audit_event` (Postgres < 13 can't drop enum
-- values cleanly) — they remain additive.
-- ===========================================================================

BEGIN;

DROP VIEW IF EXISTS v_tenant_month_to_date;

ALTER TABLE tenant_feature
    DROP COLUMN IF EXISTS metering_enabled,
    DROP COLUMN IF EXISTS billing_enabled;

DROP TABLE IF EXISTS meter_event;
DROP TABLE IF EXISTS stripe_webhook_event;
DROP TABLE IF EXISTS payment_method;
DROP TABLE IF EXISTS budget_alert_event;
DROP TABLE IF EXISTS budget_alert;
DROP TABLE IF EXISTS billing_invoice_item;
DROP TABLE IF EXISTS billing_invoice;
DROP TABLE IF EXISTS billing_subscription;
DROP TABLE IF EXISTS usage_rollup_day;
DROP TABLE IF EXISTS usage_rollup_hour;
DROP TABLE IF EXISTS tenant_billing;
DROP TABLE IF EXISTS pricing_plan_meter;
DROP TABLE IF EXISTS pricing_plan;

COMMIT;
