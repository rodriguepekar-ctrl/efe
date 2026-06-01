-- Sprint Φ-D-LATACT D92 — Postgres migration : onboarding_sessions table.
-- Cf. docs/EFE_P1_LATENCE_P2_ACTIVATION_COMPLETION_ARCHITECTURE.md § 7.4.
--
-- Source of truth for the P2 activation funnel : signup → sandbox →
-- first_classify → first_correct (= Aha) → poc_active.
-- Used by p2_portal_funnel_query.py for the 9 SLOs.

BEGIN;

CREATE TABLE IF NOT EXISTS onboarding_sessions (
    tenant_id            TEXT PRIMARY KEY,
    plan_target          TEXT NOT NULL CHECK (plan_target IN ('sandbox', 'poc', 'prod')),
    cloud                TEXT             CHECK (cloud IS NULL OR cloud IN ('gcp', 'aws', 'azure')),

    -- Funnel timestamps (= 6 states from architecture § 7.2).
    signed_up_at         TIMESTAMPTZ NOT NULL,
    sandbox_created_at   TIMESTAMPTZ,
    first_classify_at    TIMESTAMPTZ,
    first_correct_at     TIMESTAMPTZ,  -- = Aha moment
    poc_requested_at     TIMESTAMPTZ,
    poc_active_at        TIMESTAMPTZ,
    abandoned_at         TIMESTAMPTZ,
    last_state           TEXT,         -- 'signup' / 'sandbox_created' / ...

    -- Consent (= 4-axis from Φ-D-COMPLIANCE D-COMP-9).
    consent_analytics            BOOLEAN NOT NULL DEFAULT FALSE,
    consent_model_training       BOOLEAN NOT NULL DEFAULT FALSE,
    consent_marketing            BOOLEAN NOT NULL DEFAULT FALSE,
    consent_third_party_sharing  BOOLEAN NOT NULL DEFAULT FALSE,

    -- Audit chain link (= HMAC-signed activation event ids).
    last_audit_event_id  TEXT,

    -- Metadata.
    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Indices for SLO queries (cf. P2 § 5).
CREATE INDEX IF NOT EXISTS idx_onboarding_plan_signup
    ON onboarding_sessions (plan_target, signed_up_at DESC);

CREATE INDEX IF NOT EXISTS idx_onboarding_first_classify
    ON onboarding_sessions (plan_target, first_classify_at)
    WHERE first_classify_at IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_onboarding_aha
    ON onboarding_sessions (plan_target, first_correct_at)
    WHERE first_correct_at IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_onboarding_abandoned
    ON onboarding_sessions (plan_target, abandoned_at)
    WHERE abandoned_at IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_onboarding_cloud
    ON onboarding_sessions (cloud, plan_target)
    WHERE cloud IS NOT NULL;

-- LISTEN/NOTIFY channel for the streaming audit replay (= cross-ref Φ-D-STREAM).
CREATE OR REPLACE FUNCTION notify_onboarding_change()
RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify('onboarding_events', json_build_object(
        'tenant_id', NEW.tenant_id,
        'plan_target', NEW.plan_target,
        'last_state', NEW.last_state,
        'ts', extract(epoch from NOW())
    )::text);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_onboarding_notify
AFTER INSERT OR UPDATE ON onboarding_sessions
FOR EACH ROW EXECUTE FUNCTION notify_onboarding_change();

COMMIT;
