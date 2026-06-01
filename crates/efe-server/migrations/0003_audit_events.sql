-- ============================================================================
-- Sprint D14-bis — HMAC-chained tamper-evident audit log.
-- ============================================================================
--
-- Reference : `docs/bench/d14bis_hmac_audit_chain_architecture.md` § 10.
--
-- This migration introduces the `audit_events` table that captures the
-- HMAC-chained signature for every catalog mutation. The table is
-- **append-only** : UPDATE / DELETE / TRUNCATE are blocked by triggers
-- so that even a privileged DBA cannot mutate the chain without first
-- explicitly DROP-ing the trigger (which is itself an audit-able DDL
-- event tracked by `pgaudit` outside of EFE's scope).
--
-- The chain itself is constructed by `HmacChainedAuditSink` in
-- `crates/efe-server/src/catalog/audit_hmac.rs` : each row's `hmac` is
-- HMAC-SHA-256 of the row's `canonical_payload` keyed by the active
-- HMAC key. The `prev_hmac` column points to the previous row in the
-- same `chain_id` ; the verify CLI replays the chain from the genesis
-- value (64 zeros hex) and any tampering / deletion / reordering
-- breaks the chain at a detectable position.
--
-- Forward-only — to retire the table later, write a new migration
-- `00NN_drop_audit_events.sql` (which itself is a destructive event that
-- ops should treat with care).
-- ============================================================================

CREATE TABLE audit_events (
    -- Monotonic insertion order (DB-assigned). Streaming verify scans
    -- by (chain_id, seq) ASC ; pagination is on `seq`.
    seq                BIGSERIAL PRIMARY KEY,

    -- UUIDv7 (timestamp-monotone). Carried in the canonical payload
    -- and the HMAC ; replay attacks (re-INSERT of an old event) fail
    -- on the UNIQUE constraint.
    event_id           UUID      NOT NULL UNIQUE,

    -- Chain identifier ("tenant:<X>" today, "global" reserved for D14-ter).
    -- The CHECK constraint mirrors `CatalogPolicy::validate_identifier`
    -- (post-D14 audit fix C1) : tenant identifiers are constrained to
    -- `^[a-z0-9_-]{1,63}$`.
    chain_id           TEXT      NOT NULL,

    -- Per-event metadata (mirror of `AuditEvent` in
    -- `crates/efe-server/src/catalog/audit.rs`).
    tenant             TEXT      NOT NULL,
    action             TEXT      NOT NULL,        -- "register"|"evolve"|"delete"
    op_tag             TEXT      NOT NULL,        -- "register"|"append_class"|...
    catalog_key        TEXT      NOT NULL,
    principal          TEXT      NOT NULL,
    description        TEXT      NOT NULL DEFAULT '',
    parent_key         TEXT      NOT NULL DEFAULT '',
    content_sha256     TEXT      NOT NULL,
    created_at_unix_ms BIGINT    NOT NULL,

    -- Exact bytes that fed the HMAC (debug + external verify). The
    -- canonical form is JSON sorted keys + minified UTF-8 ; we store
    -- it as JSONB to allow ad-hoc queries without breaking the byte
    -- equality (PG normalises JSONB at load time but our `verify_chain`
    -- re-serialises with `serde_json::to_vec` over a `BTreeMap` so the
    -- check is deterministic regardless of PG normalisation).
    canonical_payload  JSONB     NOT NULL,

    -- Chain pointers + signature.
    prev_hmac          TEXT      NOT NULL,        -- 64-char lowercase hex
    hmac               TEXT      NOT NULL,        -- 64-char lowercase hex
    key_id             TEXT      NOT NULL,        -- 8-char lowercase hex (first 8 of SHA-256(key))

    -- DB-side audit (orthogonal to created_at_unix_ms).
    db_inserted_at     TIMESTAMPTZ NOT NULL DEFAULT now(),

    -- Format invariants.
    CONSTRAINT audit_hmac_hex_format
        CHECK (hmac ~ '^[0-9a-f]{64}$'),
    CONSTRAINT audit_prev_hmac_hex_format
        CHECK (prev_hmac ~ '^[0-9a-f]{64}$'),
    CONSTRAINT audit_key_id_hex_format
        CHECK (key_id ~ '^[0-9a-f]{8}$'),
    CONSTRAINT audit_action_enum
        CHECK (action IN ('register', 'evolve', 'delete')),
    CONSTRAINT audit_chain_id_format
        CHECK (chain_id ~ '^(tenant:[a-z0-9_-]{1,63}|global)$')
);

COMMENT ON TABLE audit_events IS
    'Sprint D14-bis — HMAC-chained tamper-evident audit log. '
    'Append-only via the `audit_events_block_mutation` trigger. Each row''s '
    '`hmac` is HMAC-SHA-256 of the `canonical_payload` keyed by the active '
    'HMAC key (rotated via EFE_AUDIT_HMAC_SECRETS_FILE). The `prev_hmac` '
    'chains this row to the previous row in the same `chain_id` ; the '
    'verify CLI replays the chain from genesis (64 zeros) to detect '
    'tampering, deletion, or reordering.';

-- ============================================================================
-- Append-only enforcement.
-- ============================================================================
--
-- A single trigger function rejecting any mutation. We attach 3 triggers
-- (UPDATE, DELETE, TRUNCATE) because PG dispatches each event class to
-- a distinct trigger ; a single ON ALL trigger doesn't exist.
--
-- Note : a DBA with sufficient privileges can `DROP TRIGGER … ON
-- audit_events` and then mutate. That's a separate, auditable DDL event
-- which should be tracked outside EFE (e.g. via the `pgaudit` extension
-- or AWS RDS native audit). EFE's responsibility ends at "we made it
-- impossible to mutate without an explicit DDL change visible to ops".

CREATE OR REPLACE FUNCTION audit_events_block_mutation()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION
        'audit_events is append-only (D14-bis HMAC chain). '
        'Trigger % blocked the operation. '
        'If you really need to mutate, DROP this trigger first ; '
        'doing so will surface in pgaudit / RDS audit / file-system '
        'audit as a deliberate operator action.',
        TG_NAME;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER audit_events_no_update
    BEFORE UPDATE ON audit_events
    FOR EACH ROW EXECUTE FUNCTION audit_events_block_mutation();

CREATE TRIGGER audit_events_no_delete
    BEFORE DELETE ON audit_events
    FOR EACH ROW EXECUTE FUNCTION audit_events_block_mutation();

CREATE TRIGGER audit_events_no_truncate
    BEFORE TRUNCATE ON audit_events
    FOR EACH STATEMENT EXECUTE FUNCTION audit_events_block_mutation();

-- ============================================================================
-- Indexes.
-- ============================================================================
--
-- The chain replay query is `SELECT … FROM audit_events
-- WHERE chain_id = $1 ORDER BY seq` — one composite index covers both
-- the WHERE filter and the ORDER BY without a sort step.
CREATE INDEX audit_events_chain_seq_idx
    ON audit_events (chain_id, seq);

-- Time-range queries for compliance reports : "show all audit events
-- for tenant X between ts1 and ts2".
CREATE INDEX audit_events_tenant_created_at_idx
    ON audit_events (tenant, created_at_unix_ms);

-- event_id is already UNIQUE (implicit B-tree). No additional index
-- needed.
