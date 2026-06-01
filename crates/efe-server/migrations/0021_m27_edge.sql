-- M27 Edge — per-tenant quotas + daily usage rollup + JWT revocation
--                + IP allowlist + 'QuotaExceeded' audit event.
--
-- Hard dependency : migration 0018 (tenant_feature M26 columns).
--
-- Idempotent : every ALTER / CREATE uses `IF NOT EXISTS` so re-running
-- the migration on an already-migrated DB is a no-op.
--
-- See `docs/EFE_M27_COMMERCIAL_EDGE_ARCHITECTURE.md` § 14.

BEGIN;

-- ── 1. tenant_feature additions for per-endpoint quotas ────────────────
ALTER TABLE tenant_feature
    ADD COLUMN IF NOT EXISTS classify_quota_per_min INTEGER NOT NULL DEFAULT 100,
    ADD COLUMN IF NOT EXISTS agent_run_quota_per_hour INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS embed_quota_per_min INTEGER NOT NULL DEFAULT 100,
    ADD COLUMN IF NOT EXISTS search_quota_per_min INTEGER NOT NULL DEFAULT 100,
    ADD COLUMN IF NOT EXISTS llm_tokens_quota_per_day BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS pii_strict BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS tier VARCHAR(32) NOT NULL DEFAULT 'free';

COMMENT ON COLUMN tenant_feature.classify_quota_per_min IS
    'M27 — Per-minute classify quota. Loaded by tenant_quota middleware.';
COMMENT ON COLUMN tenant_feature.agent_run_quota_per_hour IS
    'M27 — Per-hour cap on agent runs starts. 0 = disabled.';
COMMENT ON COLUMN tenant_feature.embed_quota_per_min IS
    'M27 — Per-minute embeddings quota.';
COMMENT ON COLUMN tenant_feature.search_quota_per_min IS
    'M27 — Per-minute vector search quota (M31 forward-compat).';
COMMENT ON COLUMN tenant_feature.llm_tokens_quota_per_day IS
    'M27 — Per-day cap on LLM tokens. 0 = unlimited (use carefully).';
COMMENT ON COLUMN tenant_feature.pii_strict IS
    'M27 — When TRUE, enable timing-jitter on cache responses + PII auto-redaction.';
COMMENT ON COLUMN tenant_feature.tier IS
    'M27 — Tier label : free|starter|pro|enterprise. Drives default quotas in onboarding.';

-- ── 2. daily usage rollup (populated by aggregator from Redis counters) ────
CREATE TABLE IF NOT EXISTS tenant_daily_usage (
    tenant_id     TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    day           DATE NOT NULL,
    endpoint      VARCHAR(64) NOT NULL,
    request_count BIGINT NOT NULL DEFAULT 0,
    llm_tokens    BIGINT NOT NULL DEFAULT 0,
    cost_cents    BIGINT NOT NULL DEFAULT 0,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, day, endpoint)
);

COMMENT ON TABLE tenant_daily_usage IS
    'M27 — Daily aggregated usage per tenant + endpoint. Populated by '
    'the usage aggregator worker which reads epoch-minute counters from '
    'Redis and rolls them up. Source of truth for billing.';

CREATE INDEX IF NOT EXISTS idx_tenant_daily_usage_day
    ON tenant_daily_usage (day);
CREATE INDEX IF NOT EXISTS idx_tenant_daily_usage_tenant_day
    ON tenant_daily_usage (tenant_id, day DESC);

-- ── 3. API key (JWT) revocation list ──────────────────────────────────
CREATE TABLE IF NOT EXISTS revoked_jwt (
    jti           UUID PRIMARY KEY,
    tenant_id     TEXT NOT NULL,
    sub           TEXT NOT NULL,
    revoked_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_by    TEXT NOT NULL,
    reason        TEXT,
    expires_at    TIMESTAMPTZ NOT NULL
);

COMMENT ON TABLE revoked_jwt IS
    'M27 — Revoked JWT list. Envoy Lua filter performs a Redis '
    'EXISTS check on revoked_jwt:{jti} ; this PG table is the '
    'source of truth (Redis cache repopulated via incremental sync).';
COMMENT ON COLUMN revoked_jwt.expires_at IS
    'When the token would have expired naturally. Used to auto-clean '
    'after expires_at + 7 days grace.';

CREATE INDEX IF NOT EXISTS idx_revoked_jwt_expires
    ON revoked_jwt (expires_at);
CREATE INDEX IF NOT EXISTS idx_revoked_jwt_tenant
    ON revoked_jwt (tenant_id);

-- ── 4. tenant API IP allowlist (optional, enterprise tier) ────────────
CREATE TABLE IF NOT EXISTS tenant_ip_allowlist (
    tenant_id     TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    cidr          CIDR NOT NULL,
    added_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    added_by      TEXT NOT NULL,
    note          TEXT,
    PRIMARY KEY (tenant_id, cidr)
);

COMMENT ON TABLE tenant_ip_allowlist IS
    'M27 — Enterprise-tier feature : restrict tenant API access to a '
    'CIDR list. Enforced at Cloudflare via Terraform-managed rule + '
    'at Envoy via lua filter as defense in depth.';

CREATE INDEX IF NOT EXISTS idx_tenant_ip_allowlist_tenant
    ON tenant_ip_allowlist (tenant_id);

-- ── 5. audit event for quota exceeded ────────────────────────────────
-- agent_audit_event is an ENUM in migration 0016. Adding a value
-- requires no-tx form (Postgres limitation). We wrap in DO block
-- so the migration script can stay in a transaction otherwise.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_type t
        JOIN pg_enum e ON e.enumtypid = t.oid
        WHERE t.typname = 'agent_audit_event'
          AND e.enumlabel = 'QuotaExceeded'
    ) THEN
        -- Note : ALTER TYPE ... ADD VALUE inside a transaction is
        -- supported on PG 12+ when the value is new.
        ALTER TYPE agent_audit_event ADD VALUE 'QuotaExceeded';
    END IF;
END$$;

-- ── 6. helper view for billing aggregation ───────────────────────────
CREATE OR REPLACE VIEW v_tenant_monthly_usage AS
SELECT
    tenant_id,
    DATE_TRUNC('month', day)::DATE AS month,
    endpoint,
    SUM(request_count) AS request_count,
    SUM(llm_tokens) AS llm_tokens,
    SUM(cost_cents) AS cost_cents
FROM tenant_daily_usage
GROUP BY tenant_id, DATE_TRUNC('month', day)::DATE, endpoint;

COMMENT ON VIEW v_tenant_monthly_usage IS
    'M27 — Monthly rollup of daily usage, used by Stripe Metered '
    'Billing aggregator (P3 sprint).';

-- ── 7. cleanup function : prune expired revoked JWTs ──────────────────
CREATE OR REPLACE FUNCTION fn_cleanup_expired_revoked_jwt()
RETURNS INTEGER
LANGUAGE plpgsql
AS $$
DECLARE
    deleted INTEGER;
BEGIN
    DELETE FROM revoked_jwt
    WHERE expires_at < NOW() - INTERVAL '7 days';
    GET DIAGNOSTICS deleted = ROW_COUNT;
    RETURN deleted;
END;
$$;

COMMENT ON FUNCTION fn_cleanup_expired_revoked_jwt() IS
    'M27 — Returns count of deleted rows. Schedule via pg_cron or '
    'external job once a day : SELECT fn_cleanup_expired_revoked_jwt().';

COMMIT;
