-- 0008_m21_tenant_llm_budget.sql — M21 Phase C
--
-- Per-tenant LLM cost cap (Postgres-backed, race-safe via SELECT FOR
-- UPDATE). Default `monthly_cap_usd = 0` so any tenant must explicitly
-- set a cap > 0 to enable Mode-2 (cf. roadmap § F.7 + arch § 4.6).
--
-- Race-safety : SELECT FOR UPDATE on the tenant row guarantees that
-- two concurrent replicas can't both pass with the same residual
-- budget.
--
-- Idempotency rollover : daily cron at 02:00 UTC resets
-- spent_this_month → 0 when EXTRACT(DAY FROM CURRENT_DATE)::int =
-- rollover_dom AND last_rollover < CURRENT_DATE - 25 days. The 25-day
-- guard ensures the cron is idempotent within the same day.

CREATE TABLE IF NOT EXISTS tenant_llm_budget (
    tenant_id           TEXT PRIMARY KEY,
    monthly_cap_usd     NUMERIC(10,4) NOT NULL DEFAULT 0
                        CHECK (monthly_cap_usd >= 0),
    soft_threshold_pct  INTEGER NOT NULL DEFAULT 80
                        CHECK (soft_threshold_pct BETWEEN 0 AND 100),
    fail_open           BOOLEAN NOT NULL DEFAULT FALSE,
    rollover_dom        INTEGER NOT NULL DEFAULT 1
                        CHECK (rollover_dom BETWEEN 1 AND 28),
    last_rollover       DATE NOT NULL DEFAULT CURRENT_DATE,
    spent_this_month    NUMERIC(10,4) NOT NULL DEFAULT 0
                        CHECK (spent_this_month >= 0),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS ix_tenant_llm_budget_rollover
    ON tenant_llm_budget(rollover_dom, last_rollover);

COMMENT ON TABLE tenant_llm_budget IS
    'M21 — Per-tenant LLM cost cap. Race-safe via SELECT FOR UPDATE.';
COMMENT ON COLUMN tenant_llm_budget.monthly_cap_usd IS
    'Hard monthly cap in USD. 0 = all calls rejected.';
COMMENT ON COLUMN tenant_llm_budget.fail_open IS
    'If true, calls over cap fall back to Mode-1 silently.';
COMMENT ON COLUMN tenant_llm_budget.rollover_dom IS
    'Day of month for budget reset (1-28 to avoid Feb edge cases).';
