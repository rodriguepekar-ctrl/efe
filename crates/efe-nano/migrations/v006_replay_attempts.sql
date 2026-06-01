-- Migration v006 — M50 Sprint 6 — Replay attempt audit.
--
-- Each call to ReplayEngine::replay produces a row here, with the full
-- report stored as JSONB for offline analysis.

CREATE TABLE IF NOT EXISTS replay_attempts (
    replay_id           UUID         PRIMARY KEY,
    tenant_id           UUID         NOT NULL,
    original_request_id UUID         NOT NULL,

    -- Who triggered the replay.
    triggered_by        TEXT         NOT NULL CHECK (
        triggered_by IN ('admin', 'customer', 'regulator', 'ci')
    ),
    triggered_by_actor  TEXT         NULL,

    -- Replay outcome counters.
    chain_verified      BOOLEAN      NOT NULL,
    total_ops           INTEGER      NOT NULL,
    matched_ops         INTEGER      NOT NULL,
    mismatched_ops      INTEGER      NOT NULL,
    skipped_ops         INTEGER      NOT NULL,
    failed_ops          INTEGER      NOT NULL,
    fully_consistent    BOOLEAN      NOT NULL,

    started_at          TIMESTAMPTZ  NOT NULL,
    completed_at        TIMESTAMPTZ  NOT NULL,
    duration_ms         INTEGER      NOT NULL,

    -- Full report (cf crate::replay::cli::serialize_report).
    report_jsonb        JSONB        NOT NULL,

    inserted_at         TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS replay_attempts_tenant_started_idx
    ON replay_attempts (tenant_id, started_at DESC);
CREATE INDEX IF NOT EXISTS replay_attempts_original_idx
    ON replay_attempts (original_request_id);
