-- ===========================================================================
-- M30 — Multi-modal inference + Multi-region active-passive.
--
-- Adds :
--   1. tenant_modality_policy           — per-tenant modality on/off
--      + caps + DPIA timestamp.
--   2. multimodal_inference_audit       — audit log of multimodal
--      inferences (input hash, modality, latency, outcome). The full
--      transcript / image is NEVER persisted ; only the SHA-256 prefix
--      and the embedding hash.
--   3. region                           — known regions in the
--      cluster topology + role (active / passive) + last_promotion.
--   4. region_health_history            — append-only log of peer
--      probe results (kept 90 days for SOC 2 evidence).
--   5. region_failover_event            — append-only log of every
--      failover initiation (manual or drill).
--   6. dr_drill_run                     — quarterly DR drill reports.
--   7. tenant_feature additions         — multimodal_enabled,
--      preferred_region.
--   8. agent_audit enum extensions       — Multimodal*, Region*.
--   9. View                             — v_region_active.
-- ===========================================================================

BEGIN;

-- ── 1. tenant_modality_policy ──────────────────────────────────────
CREATE TABLE IF NOT EXISTS tenant_modality_policy (
    tenant_id                   TEXT PRIMARY KEY REFERENCES tenant_feature(id) ON DELETE CASCADE,
    -- M30 audit fix : surface the DPIA signer in PG schema as well
    -- (Rust struct gained the field in efe-multimodal::policy).
    vision_enabled              BOOLEAN NOT NULL DEFAULT FALSE,
    audio_enabled               BOOLEAN NOT NULL DEFAULT FALSE,
    long_text_enabled           BOOLEAN NOT NULL DEFAULT TRUE,
    max_vision_bytes            BIGINT NOT NULL DEFAULT 10485760    -- 10 MB
                                CHECK (max_vision_bytes BETWEEN 1024 AND 104857600),
    max_audio_bytes             BIGINT NOT NULL DEFAULT 26214400    -- 25 MB
                                CHECK (max_audio_bytes BETWEEN 1024 AND 268435456),
    max_long_text_bytes         BIGINT NOT NULL DEFAULT 1048576     -- 1 MB
                                CHECK (max_long_text_bytes BETWEEN 1024 AND 10485760),
    max_audio_ms                BIGINT NOT NULL DEFAULT 60000        -- 60 s
                                CHECK (max_audio_ms BETWEEN 1000 AND 3600000),
    allowed_vision_mimes        TEXT[] NOT NULL DEFAULT ARRAY['image/jpeg','image/png','image/webp']::TEXT[],
    allowed_audio_mimes         TEXT[] NOT NULL DEFAULT ARRAY['audio/wav','audio/mpeg','audio/flac']::TEXT[],
    dpia_signed_at              TIMESTAMPTZ,
    dpia_signed_by              TEXT,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON TABLE tenant_modality_policy IS
    'M30 — Per-tenant policy for vision / audio / long-context modalities. '
    'DPIA signature required for vision/audio per GDPR Art. 35.';
COMMENT ON COLUMN tenant_modality_policy.dpia_signed_at IS
    'M30 — Null ⇒ vision/audio forbidden even if *_enabled = true.';
COMMENT ON COLUMN tenant_modality_policy.max_vision_bytes IS
    'M30 — Per-request cap. Soft limit 10 MB, hard cap 100 MB.';

-- ── 2. multimodal_inference_audit ──────────────────────────────────
CREATE TABLE IF NOT EXISTS multimodal_inference_audit (
    id                          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id                   TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    modality                    VARCHAR(16) NOT NULL
                                CHECK (modality IN ('vision','audio','long_text','fusion')),
    input_sha256_hex            CHAR(64) NOT NULL,
    detected_mime               VARCHAR(32),
    duration_ms_estimate        BIGINT NOT NULL DEFAULT 0,
    e2e_ms                      BIGINT NOT NULL,
    triton_ms                   BIGINT,
    outcome                     VARCHAR(16) NOT NULL
                                CHECK (outcome IN ('ok','err')),
    error_code                  VARCHAR(64),
    model_name                  VARCHAR(64),
    region                      VARCHAR(32) NOT NULL,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON TABLE multimodal_inference_audit IS
    'M30 — Audit log of multimodal inferences. Raw input bytes NEVER stored.';
COMMENT ON COLUMN multimodal_inference_audit.input_sha256_hex IS
    'M30 — SHA-256 of the raw input ; recomputable for forensics.';

CREATE INDEX IF NOT EXISTS idx_mm_audit_tenant_created
    ON multimodal_inference_audit (tenant_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_mm_audit_outcome_modality
    ON multimodal_inference_audit (outcome, modality, created_at DESC);

-- ── 3. region ─────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS region (
    name                        VARCHAR(32) PRIMARY KEY,
    role                        VARCHAR(8) NOT NULL CHECK (role IN ('active','passive')),
    cloud_provider              VARCHAR(16) NOT NULL DEFAULT 'aws',
    pg_endpoint                 TEXT NOT NULL,
    redis_endpoint              TEXT NOT NULL,
    nats_endpoint               TEXT NOT NULL,
    triton_endpoint             TEXT,
    last_promotion_at           TIMESTAMPTZ,
    last_promotion_reason       TEXT,
    last_promotion_operator     TEXT,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON TABLE region IS
    'M30 — Cluster topology. One row per known EFE region. '
    'role flips on failover.';

-- A logical UNIQUE constraint : exactly one region in role='active'.
-- Enforced via a partial unique index on a constant.
CREATE UNIQUE INDEX IF NOT EXISTS idx_region_one_active
    ON region ((true))
 WHERE role = 'active';

-- ── 4. region_health_history ──────────────────────────────────────
CREATE TABLE IF NOT EXISTS region_health_history (
    id                          BIGSERIAL PRIMARY KEY,
    observer_region             VARCHAR(32) NOT NULL,
    target_region               VARCHAR(32) NOT NULL,
    probed_at                   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    up                          BOOLEAN NOT NULL,
    rtt_ms                      INTEGER NOT NULL,
    replication_lag_secs        DOUBLE PRECISION,
    ready_for_promote           BOOLEAN NOT NULL DEFAULT FALSE,
    reason                      TEXT
);

COMMENT ON TABLE region_health_history IS
    'M30 — Append-only log of cross-region probe results. '
    'Retained 90 days for SOC 2 BCDR evidence.';

CREATE INDEX IF NOT EXISTS idx_region_health_target_time
    ON region_health_history (target_region, probed_at DESC);
CREATE INDEX IF NOT EXISTS idx_region_health_observer_time
    ON region_health_history (observer_region, probed_at DESC);

-- ── 5. region_failover_event ──────────────────────────────────────
CREATE TABLE IF NOT EXISTS region_failover_event (
    id                          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    initiated_at                TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at                TIMESTAMPTZ,
    source_region               VARCHAR(32) NOT NULL,
    target_region               VARCHAR(32) NOT NULL,
    reason                      TEXT NOT NULL,
    operator                    TEXT NOT NULL,
    force                       BOOLEAN NOT NULL DEFAULT FALSE,
    drill_run_id                UUID,
    status                      VARCHAR(16) NOT NULL DEFAULT 'in_progress'
                                CHECK (status IN ('in_progress','succeeded','failed','noop')),
    failure_reason              TEXT
);

COMMENT ON TABLE region_failover_event IS
    'M30 — Append-only log of failover initiations. Audit-chain mirror.';

CREATE INDEX IF NOT EXISTS idx_region_failover_target_time
    ON region_failover_event (target_region, initiated_at DESC);

-- ── 6. dr_drill_run ──────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS dr_drill_run (
    id                          UUID PRIMARY KEY,
    started_at                  TIMESTAMPTZ NOT NULL,
    completed_at                TIMESTAMPTZ,
    initial_active              VARCHAR(32) NOT NULL,
    failover_target             VARCHAR(32) NOT NULL,
    operator                    TEXT NOT NULL,
    verdict                     VARCHAR(16)
                                CHECK (verdict IN ('pass','fail','aborted')),
    total_duration_ms           BIGINT,
    report                      JSONB NOT NULL
);

COMMENT ON TABLE dr_drill_run IS
    'M30 — Quarterly DR drill reports. Archived for 7 years (SOC 2 BCDR).';

CREATE INDEX IF NOT EXISTS idx_dr_drill_started
    ON dr_drill_run (started_at DESC);

-- ── 6.bis FK from region_failover_event.drill_run_id → dr_drill_run.id ──
-- Declared as an ALTER (rather than inline) because `dr_drill_run` is
-- created in § 6, AFTER `region_failover_event` in § 5. The constraint
-- is NULLable (drills are optional). M30 audit fix : without this FK,
-- a deleted drill_run would orphan its failover events silently.
ALTER TABLE region_failover_event
    ADD CONSTRAINT fk_region_failover_event_drill_run
    FOREIGN KEY (drill_run_id) REFERENCES dr_drill_run (id) ON DELETE SET NULL;

-- ── 7. tenant_feature additions ───────────────────────────────────
ALTER TABLE tenant_feature
    ADD COLUMN IF NOT EXISTS multimodal_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS preferred_region VARCHAR(32);

COMMENT ON COLUMN tenant_feature.multimodal_enabled IS
    'M30 — Master switch ; ANDed with tenant_modality_policy.*_enabled.';
COMMENT ON COLUMN tenant_feature.preferred_region IS
    'M30 — Optional sticky region for tenant traffic (read-only).';

-- ── 8. agent_audit enum extensions ────────────────────────────────
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'MultimodalPolicyChanged') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'MultimodalPolicyChanged';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'MultimodalInferenceFailed') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'MultimodalInferenceFailed';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'RegionFailoverInitiated') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'RegionFailoverInitiated';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'RegionFailoverCompleted') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'RegionFailoverCompleted';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'DrDrillExecuted') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'DrDrillExecuted';
    END IF;
END$$;

-- ── 9. View : current active region ───────────────────────────────
CREATE OR REPLACE VIEW v_region_active AS
SELECT name, pg_endpoint, redis_endpoint, nats_endpoint, triton_endpoint,
       last_promotion_at, last_promotion_operator
  FROM region
 WHERE role = 'active'
 LIMIT 1;

COMMENT ON VIEW v_region_active IS
    'M30 — Trivial accessor for the single active region row.';

COMMIT;
