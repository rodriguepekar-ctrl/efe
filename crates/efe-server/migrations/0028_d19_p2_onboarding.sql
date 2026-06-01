-- Sprint **D19 (P2-Foundation)** — Self-service activation surface.
--
-- Cf. `docs/EFE_P2_SELF_SERVICE_ARCHITECTURE.md` § 4.1 (modèle de
-- données du funnel).
--
-- Adds two tables required by the activation funnel :
--   1. onboarding_sessions  — one row per activation attempt
--      (sandbox / poc / production). Holds the 11 timestamps that
--      compose the "time-to-X" measurement + the HMAC signature
--      for opposability (= chaîne D14-bis).
--   2. onboarding_events    — append-only log of every funnel
--      transition. Partitioned by day for retention 365 j.
--
-- Idempotent : safe to re-apply. Pair rollback in
-- `0028_d19_p2_onboarding_rollback.sql`.

BEGIN;

-- ─────────────────────────────────────────────────────────────────
-- onboarding_sessions
-- ─────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS onboarding_sessions (
    -- Identité
    session_id                  UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id                     UUID         NOT NULL,
    tenant_id                   TEXT         REFERENCES tenant_feature(id) ON DELETE SET NULL,

    -- Cible
    plan_target                 VARCHAR(16)  NOT NULL
                                CHECK (plan_target IN ('sandbox','poc','pro','enterprise')),
    cloud_target                VARCHAR(8)
                                CHECK (cloud_target IS NULL OR cloud_target IN ('aws','gcp','azure')),
    region                      TEXT,

    -- T = 0 et transitions (NULL = pas encore atteinte)
    started_at                  TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    email_verified_at           TIMESTAMPTZ,
    payment_authorized_at       TIMESTAMPTZ,
    provisioning_started_at     TIMESTAMPTZ,
    provisioning_done_at        TIMESTAMPTZ,
    helm_done_at                TIMESTAMPTZ,
    catalog_seeded_at           TIMESTAMPTZ,
    auth_initialized_at         TIMESTAMPTZ,
    first_classify_at           TIMESTAMPTZ,
    first_classify_correct_at   TIMESTAMPTZ,    -- = "aha moment"
    abandoned_at                TIMESTAMPTZ,
    abandoned_reason            TEXT,           -- error_class (cf. § 5.4)
    completed_at                TIMESTAMPTZ,    -- = first_classify_correct_at OU abandoned_at

    -- Métriques agrégées matérialisées (pour reporting + queries SLO)
    duration_to_first_classify_seconds  INTEGER,
    duration_to_aha_seconds             INTEGER,
    rollbacks_count                     INTEGER NOT NULL DEFAULT 0,
    errors_count                        INTEGER NOT NULL DEFAULT 0,

    -- Audit chain (opposabilité juridique cf. § 9)
    audit_chain_head_at_start           BYTEA,
    audit_chain_head_at_end             BYTEA,
    audit_signature                     BYTEA,         -- HMAC-SHA-256

    -- Cohérence forte : completed_at ne peut être set sans une raison
    CONSTRAINT onboarding_sessions_completed_consistent CHECK (
        completed_at IS NULL OR (
            first_classify_correct_at IS NOT NULL OR abandoned_at IS NOT NULL
        )
    ),
    -- Une session terminée a forcément une duration ≥ 0 sec
    CONSTRAINT onboarding_sessions_duration_nonneg CHECK (
        duration_to_first_classify_seconds IS NULL OR
        duration_to_first_classify_seconds >= 0
    )
);

COMMENT ON TABLE onboarding_sessions IS
    'D19 P2-Foundation — one row per activation attempt. Source of '
    'truth for SLO-P2-TTFC-{1,2,3} + SLO-P2-AHA-{1,2}. Signed HMAC '
    'in audit_signature for opposability (cf. § 9).';

COMMENT ON COLUMN onboarding_sessions.audit_signature IS
    'HMAC-SHA-256 over canonical-JSON serialization of the row + '
    'all onboarding_events for this session_id. Computed once when '
    'completed_at is set ; appended to the D14-bis chain.';

-- Indices pour les queries SLO (cf. § 5.2)
CREATE INDEX IF NOT EXISTS idx_onboarding_sessions_started_at
    ON onboarding_sessions(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_onboarding_sessions_plan_target
    ON onboarding_sessions(plan_target, completed_at)
    WHERE completed_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_onboarding_sessions_cloud_target
    ON onboarding_sessions(cloud_target, completed_at)
    WHERE cloud_target IS NOT NULL AND completed_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_onboarding_sessions_tenant_id
    ON onboarding_sessions(tenant_id)
    WHERE tenant_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_onboarding_sessions_abandoned
    ON onboarding_sessions(abandoned_reason)
    WHERE abandoned_at IS NOT NULL;

-- ─────────────────────────────────────────────────────────────────
-- onboarding_events
-- ─────────────────────────────────────────────────────────────────
--
-- Append-only log de chaque transition d'état. Le portail Next.js
-- émet via WebSocket à `efe-server::v1::onboarding::funnel_events`,
-- le backend persiste ici (transaction unique avec update de
-- onboarding_sessions).
--
-- Partitionnée par jour pour rétention 365 j + drop cheap des
-- partitions âgées. La partition initiale couvre le mois en cours
-- + 1 mois (~ 60 j) ; un cron étend la fenêtre.

CREATE TABLE IF NOT EXISTS onboarding_events (
    event_id            UUID         NOT NULL DEFAULT gen_random_uuid(),
    session_id          UUID         NOT NULL,
    event_kind          VARCHAR(64)  NOT NULL,
    occurred_at         TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    metadata            JSONB        NOT NULL DEFAULT '{}'::jsonb,
    correlation_id      UUID         NOT NULL,
    actor               VARCHAR(16)  NOT NULL
                        CHECK (actor IN ('user','system','terraform','helm','auth','timer')),
    error_class         VARCHAR(40),

    PRIMARY KEY (occurred_at, event_id)
) PARTITION BY RANGE (occurred_at);

COMMENT ON TABLE onboarding_events IS
    'D19 P2-Foundation — append-only event log. Partitioned by day. '
    'Used for HMAC signature + audit trail. Retention 365 j.';

-- Partition par défaut pour absorber les inserts avant qu'on
-- explicitement crée des partitions journalières (= safety net).
CREATE TABLE IF NOT EXISTS onboarding_events_default
    PARTITION OF onboarding_events DEFAULT;

CREATE INDEX IF NOT EXISTS idx_onboarding_events_session_id
    ON onboarding_events(session_id, occurred_at);
CREATE INDEX IF NOT EXISTS idx_onboarding_events_event_kind
    ON onboarding_events(event_kind);
CREATE INDEX IF NOT EXISTS idx_onboarding_events_correlation_id
    ON onboarding_events(correlation_id);

-- Whitelist des `event_kind` connus (= 18 valeurs, alignée sur le
-- doc P2 § 4.4 + 4 events système). Toute nouvelle valeur requiert
-- update de cette CHECK constraint **et** des tests Rust qui pin
-- la liste (test `p2_event_kind_whitelist_complete`).
ALTER TABLE onboarding_events
    ADD CONSTRAINT onboarding_events_kind_whitelist
    CHECK (event_kind IN (
        'session.started',
        'step.signup_form_open',
        'step.signup_form_submit',
        'step.email_verified',
        'step.payment_authorized',
        'step.plan_selected',
        'step.provisioning_started',
        'step.provisioning_layer_1_done',
        'step.provisioning_layer_2_done',
        'step.provisioning_layer_3_done',
        'step.helm_install_done',
        'step.healthcheck_ok',
        'step.catalog_seeded',
        'step.auth_initialized',
        'step.first_classify_ok',
        'step.first_classify_correct',
        'session.abandoned',
        'session.completed',
        'system.session_expired',
        'system.rollback_triggered'
    ));

-- ─────────────────────────────────────────────────────────────────
-- Trigger : mise à jour automatique de duration_* dans
-- onboarding_sessions quand completed_at est set.
-- ─────────────────────────────────────────────────────────────────
--
-- Le backend Rust pourrait calculer ça, mais l'avoir en trigger
-- garantit que **toute** modification (CLI, console, replay) garde
-- les colonnes matérialisées cohérentes.

CREATE OR REPLACE FUNCTION onboarding_sessions_update_durations()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.completed_at IS NOT NULL AND OLD.completed_at IS NULL THEN
        IF NEW.first_classify_at IS NOT NULL THEN
            NEW.duration_to_first_classify_seconds :=
                EXTRACT(EPOCH FROM (NEW.first_classify_at - NEW.started_at))::INTEGER;
        END IF;
        IF NEW.first_classify_correct_at IS NOT NULL THEN
            NEW.duration_to_aha_seconds :=
                EXTRACT(EPOCH FROM (NEW.first_classify_correct_at - NEW.started_at))::INTEGER;
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_onboarding_sessions_update_durations
    ON onboarding_sessions;
CREATE TRIGGER trg_onboarding_sessions_update_durations
    BEFORE UPDATE ON onboarding_sessions
    FOR EACH ROW
    EXECUTE FUNCTION onboarding_sessions_update_durations();

COMMIT;
