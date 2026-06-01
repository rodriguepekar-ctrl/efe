-- Sprint D17 — Backup + DR
--
-- Cf. `docs/bench/d17_backup_dr_architecture.md` § 6.2.
--
-- Adds the `efe_backup_history` table that records every backup
-- run (per source × kind). Read by the CLI `efe-backup list`,
-- by the `D17BackupStale` Prometheus alert pipeline, and by
-- the audit chain D14-bis emission.
--
-- Idempotent : safe to re-apply on a cluster that already has
-- the table (no-op).

BEGIN;

CREATE TABLE IF NOT EXISTS efe_backup_history (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    backup_label    TEXT        NOT NULL,
    backup_source   TEXT        NOT NULL CHECK (backup_source IN ('postgres', 'vault', 'bundles')),
    backup_type     TEXT        NOT NULL CHECK (backup_type IN ('full', 'diff', 'incr', 'wal', 'vault_snapshot', 'bundle_sync')),
    started_at      TIMESTAMPTZ NOT NULL,
    finished_at     TIMESTAMPTZ NOT NULL,
    status          TEXT        NOT NULL CHECK (status IN ('ok', 'failed', 'partial')),
    size_bytes      BIGINT      NOT NULL CHECK (size_bytes >= 0),
    files_count     INTEGER     NOT NULL CHECK (files_count >= 0),
    sha256_manifest TEXT        NOT NULL CHECK (length(sha256_manifest) = 64),
    ed25519_sig     TEXT        NOT NULL,
    repo_path       TEXT        NOT NULL,
    pg_lsn_start    TEXT        NULL,
    pg_lsn_stop     TEXT        NULL,
    error_message   TEXT        NULL,
    created_by      TEXT        NOT NULL DEFAULT 'cronjob',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Cross-field invariant : pg_lsn_* may only be set for the
    -- postgres source.
    CONSTRAINT chk_lsn_only_for_postgres CHECK (
        backup_source = 'postgres'
        OR (pg_lsn_start IS NULL AND pg_lsn_stop IS NULL)
    )
);

CREATE INDEX IF NOT EXISTS idx_efe_backup_history_started_at
    ON efe_backup_history (started_at DESC);

CREATE INDEX IF NOT EXISTS idx_efe_backup_history_status_started
    ON efe_backup_history (status, started_at DESC);

CREATE INDEX IF NOT EXISTS idx_efe_backup_history_source_type
    ON efe_backup_history (backup_source, backup_type, started_at DESC);

-- Marker row for migration tracking (D14 pattern).
INSERT INTO efe_migration_marker (sprint, applied_at)
VALUES ('D17', NOW())
ON CONFLICT DO NOTHING;

COMMIT;
