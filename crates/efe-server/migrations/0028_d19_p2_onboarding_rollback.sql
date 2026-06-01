-- Sprint **D19 (P2-Foundation)** rollback.

BEGIN;

DROP TRIGGER IF EXISTS trg_onboarding_sessions_update_durations
    ON onboarding_sessions;
DROP FUNCTION IF EXISTS onboarding_sessions_update_durations();

DROP TABLE IF EXISTS onboarding_events CASCADE;
DROP TABLE IF EXISTS onboarding_sessions CASCADE;

COMMIT;
