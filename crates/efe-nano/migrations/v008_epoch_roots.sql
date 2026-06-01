-- Migration v008 — M50 Sprint 7 — Merkle epoch root table (N9).
--
-- Stores one row per sealed Merkle epoch of the per-tenant audit chain.
-- Each row commits to a contiguous slice of `nano_agent_traces` (the
-- Sprint 6 chain) via a Merkle root, with linkback to the previous
-- epoch's root.
--
-- See `crates/efe-nano/src/audit/merkle/epoch.rs` for the canonical
-- serialization + signature mechanism.
--
-- Forward-only + idempotent (uses IF NOT EXISTS).

CREATE TABLE IF NOT EXISTS epoch_roots (
    -- Tenant scope.  Each tenant has its own monotonic epoch_id sequence.
    tenant_id           UUID         NOT NULL,

    -- 1-indexed, monotonic per tenant.  PRIMARY KEY pair.
    epoch_id            BIGINT       NOT NULL CHECK (epoch_id >= 1),

    -- Boundaries within nano_agent_traces.  Both are inclusive.
    start_trace_id      UUID         NOT NULL,
    end_trace_id        UUID         NOT NULL,

    -- Number of traces covered (= end - start + 1 in chain order).
    leaf_count          INTEGER      NOT NULL CHECK (leaf_count >= 1),

    -- The Merkle root (SHA-256, RFC 6962-style domain-separated).
    root_hash           BYTEA        NOT NULL CHECK (octet_length(root_hash) = 32),

    -- Linkback to the previous epoch's root_hash.  NULL iff epoch_id == 1.
    -- Enforced by Rust application layer ; see
    -- `InMemoryEpochRootStore::append` for the reference check.
    previous_epoch_root BYTEA        NULL CHECK (
        previous_epoch_root IS NULL OR octet_length(previous_epoch_root) = 32
    ),

    -- When the sealer wrote this row.
    sealed_at           TIMESTAMPTZ  NOT NULL,

    -- HMAC-SHA256(canonical_root_body, tenant_audit_key).
    -- Lets auditors verify the row hasn't been mutated post-write.
    signature           BYTEA        NOT NULL CHECK (octet_length(signature) = 32),

    PRIMARY KEY (tenant_id, epoch_id)
);

-- Index : "find the epoch containing a given trace_id".
-- Used by `EpochSealer::inclusion_proof_for_trace`.
CREATE INDEX IF NOT EXISTS epoch_roots_by_start_trace
    ON epoch_roots (tenant_id, start_trace_id);

CREATE INDEX IF NOT EXISTS epoch_roots_by_end_trace
    ON epoch_roots (tenant_id, end_trace_id);

-- Index : "latest sealed epoch for tenant".  Used by the sealer to
-- know where to resume.
CREATE INDEX IF NOT EXISTS epoch_roots_latest_per_tenant
    ON epoch_roots (tenant_id, epoch_id DESC);

-- Row-level security : per-tenant isolation (production deployment).
ALTER TABLE epoch_roots ENABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS tenant_isolation_select ON epoch_roots;
CREATE POLICY tenant_isolation_select ON epoch_roots
    FOR SELECT
    USING (tenant_id::TEXT = current_setting('efe.tenant_id', true));

DROP POLICY IF EXISTS tenant_isolation_insert ON epoch_roots;
CREATE POLICY tenant_isolation_insert ON epoch_roots
    FOR INSERT
    WITH CHECK (tenant_id::TEXT = current_setting('efe.tenant_id', true));

-- Companion table : notary receipts.  One-to-many : an epoch root may
-- be re-notarized over time (e.g. Sigstore Rekor today, AWS QLDB
-- mirror tomorrow).  Each receipt independently verifies.
CREATE TABLE IF NOT EXISTS epoch_notary_receipts (
    receipt_id          UUID         PRIMARY KEY,
    tenant_id           UUID         NOT NULL,
    epoch_id            BIGINT       NOT NULL,
    notary_id           TEXT         NOT NULL,
    commitment_hash     BYTEA        NOT NULL CHECK (octet_length(commitment_hash) = 32),
    notarized_at        TIMESTAMPTZ  NOT NULL,
    -- Signature length is variable : HMAC-SHA256 = 32 bytes (Sprint 7
    -- InMemoryNotary) ; Ed25519 = 64 bytes (Phase 4 Sigstore via
    -- Fulcio) ; CT SCTs can be longer.  No fixed CHECK.
    signature           BYTEA        NOT NULL,
    FOREIGN KEY (tenant_id, epoch_id) REFERENCES epoch_roots (tenant_id, epoch_id)
);

CREATE INDEX IF NOT EXISTS epoch_notary_receipts_per_epoch
    ON epoch_notary_receipts (tenant_id, epoch_id);

ALTER TABLE epoch_notary_receipts ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation_select ON epoch_notary_receipts;
CREATE POLICY tenant_isolation_select ON epoch_notary_receipts
    FOR SELECT
    USING (tenant_id::TEXT = current_setting('efe.tenant_id', true));
DROP POLICY IF EXISTS tenant_isolation_insert ON epoch_notary_receipts;
CREATE POLICY tenant_isolation_insert ON epoch_notary_receipts
    FOR INSERT
    WITH CHECK (tenant_id::TEXT = current_setting('efe.tenant_id', true));
