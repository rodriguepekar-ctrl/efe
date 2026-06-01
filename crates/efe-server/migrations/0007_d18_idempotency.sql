-- Sprint D18 — Multi-réplica + bench D12
--
-- Cf. `docs/bench/d18_multi_replica_bench_d12_architecture.md` § 6 + § 7.
--
-- Adds two tables required by the multi-replica runtime :
--   1. efe_idempotency : shared idempotency cache so a request
--      retried on a different replica returns the cached response
--      instead of re-executing (T-D18-2 mitigation).
--   2. efe_ratelimit_buckets : shared leaky-bucket state so a
--      tenant routed across N replicas sees a single effective
--      bucket (T-D18-3 mitigation).
--
-- Idempotent : safe to re-apply.

BEGIN;

-- ---------------------------------------------------------------------
-- efe_idempotency  (shared idempotency cache, D18 § 6)
-- ---------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS efe_idempotency (
    request_id      UUID         PRIMARY KEY,
    tenant_id       TEXT         NOT NULL,
    method          TEXT         NOT NULL,
    request_hash    BYTEA        NOT NULL,
    response_blob   BYTEA        NULL,
    response_status TEXT         NULL,
    schema_version  INTEGER      NOT NULL DEFAULT 1,
    started_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    completed_at    TIMESTAMPTZ  NULL,
    expires_at      TIMESTAMPTZ  NOT NULL,

    -- Cross-field invariant : a row is either in-flight (no response
    -- + no completed_at) OR completed (response + status + completed_at).
    CONSTRAINT chk_idem_completed_consistency CHECK (
        (completed_at IS NULL AND response_status IS NULL)
        OR
        (completed_at IS NOT NULL AND response_status IS NOT NULL)
    ),

    -- Sanity : expires_at must be strictly in the future of started_at.
    CONSTRAINT chk_idem_expires_after_started CHECK (expires_at > started_at),

    -- response_status is enum-like ; pin acceptable values.
    CONSTRAINT chk_idem_status_values CHECK (
        response_status IS NULL
        OR response_status IN ('ok', 'error', 'timeout')
    )
);

CREATE INDEX IF NOT EXISTS idx_idem_expires
    ON efe_idempotency (expires_at);

CREATE INDEX IF NOT EXISTS idx_idem_tenant_started
    ON efe_idempotency (tenant_id, started_at DESC);


-- ---------------------------------------------------------------------
-- efe_ratelimit_buckets  (shared leaky bucket state, D18 § 7)
-- ---------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS efe_ratelimit_buckets (
    tenant_id       TEXT         PRIMARY KEY,
    capacity        INTEGER      NOT NULL,
    refill_per_sec  REAL         NOT NULL,
    available       REAL         NOT NULL,
    last_refill_at  TIMESTAMPTZ  NOT NULL DEFAULT NOW(),

    -- Invariant : available always in [0, capacity].
    CONSTRAINT chk_rl_available_in_range
        CHECK (available >= 0 AND available <= capacity),
    CONSTRAINT chk_rl_capacity_positive CHECK (capacity > 0),
    CONSTRAINT chk_rl_refill_non_negative CHECK (refill_per_sec >= 0)
);

CREATE INDEX IF NOT EXISTS idx_ratelimit_last_refill
    ON efe_ratelimit_buckets (last_refill_at);


-- Marker row for migration tracking (D14 pattern).
INSERT INTO efe_migration_marker (sprint, applied_at)
VALUES ('D18', NOW())
ON CONFLICT DO NOTHING;

COMMIT;
