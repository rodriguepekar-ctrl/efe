-- Migration v009 — M50 Sprint 7 — GDPR tombstone (N10).
--
-- Extends `nano_agent_traces` (migration v004) with the tombstone
-- columns introduced by N10.  GDPR Article 17 (right to erasure)
-- previously had no audit-grade-compatible mechanism : either the
-- erasure destroyed the HMAC chain (illegal under audit-grade SLAs)
-- or the erasure was refused (illegal under EU GDPR).  N10 introduces
-- the **tombstone** : a structured erasure of PII bodies while the
-- cryptographically-frozen fields (`payload_hash`, `hmac`,
-- `chain_parent_id`, the hash columns) stay intact.
--
-- See `crates/efe-nano/src/audit/mod.rs` (`TombstoneMetadata`) for the
-- domain model and `audit/chain_store.rs` (`tombstone_trace`) for the
-- runtime contract.
--
-- Forward-only + idempotent.

-- ── Add the 5 tombstone columns ──────────────────────────────────────

ALTER TABLE nano_agent_traces
    ADD COLUMN IF NOT EXISTS tombstone_erased_at        TIMESTAMPTZ  NULL,
    ADD COLUMN IF NOT EXISTS tombstone_erasure_reason   TEXT         NULL
        CHECK (tombstone_erasure_reason IS NULL OR
               tombstone_erasure_reason IN (
                   'gdpr_article_17',
                   'retention_policy',
                   'operator_request',
                   'tenant_deleted'
               )),
    ADD COLUMN IF NOT EXISTS tombstone_erasure_request_id UUID       NULL,
    ADD COLUMN IF NOT EXISTS tombstone_erasure_actor    TEXT         NULL,
    ADD COLUMN IF NOT EXISTS tombstone_hmac             BYTEA        NULL
        CHECK (tombstone_hmac IS NULL OR octet_length(tombstone_hmac) = 32);

-- Consistency : all 5 columns must be NULL together (live trace) or
-- all 5 NOT NULL together (tombstoned trace).
ALTER TABLE nano_agent_traces
    ADD CONSTRAINT IF NOT EXISTS tombstone_consistency CHECK (
        (
            tombstone_erased_at IS NULL AND
            tombstone_erasure_reason IS NULL AND
            tombstone_erasure_request_id IS NULL AND
            tombstone_erasure_actor IS NULL AND
            tombstone_hmac IS NULL
        ) OR (
            tombstone_erased_at IS NOT NULL AND
            tombstone_erasure_reason IS NOT NULL AND
            tombstone_erasure_actor IS NOT NULL AND
            tombstone_hmac IS NOT NULL
            -- tombstone_erasure_request_id stays optional (None for
            -- retention / operator paths that don't have a request_id).
        )
    );

-- ── Indexes for ops dashboards + retention queries ───────────────────

-- Find all tombstoned traces for a tenant : ops "purged in last 24h"
-- dashboards.
CREATE INDEX IF NOT EXISTS nano_agent_traces_tombstoned
    ON nano_agent_traces (tenant_id, tombstone_erased_at)
    WHERE tombstone_erased_at IS NOT NULL;

-- Find all traces by erasure_request_id : trace a single GDPR request's
-- full footprint.
CREATE INDEX IF NOT EXISTS nano_agent_traces_erasure_request
    ON nano_agent_traces (tombstone_erasure_request_id)
    WHERE tombstone_erasure_request_id IS NOT NULL;

-- ── Audit cascade : the side-store body purge ────────────────────────
--
-- The body store lives in a separate table (Phase 4) typed roughly :
-- CREATE TABLE input_bodies (hash BYTEA PRIMARY KEY, body JSONB, refcount INT);
-- CREATE TABLE output_bodies (hash BYTEA PRIMARY KEY, body JSONB, refcount INT);
-- A trigger on UPDATE OF tombstone_erased_at decrements refcounts and
-- deletes the body row when refcount drops to zero.  That trigger is
-- Phase 4 deployment work ; the Rust application layer
-- (`InMemoryChainStore::tombstone_trace`) handles it for Sprint 7's
-- in-memory backend.

-- ── Helper view : live traces only ───────────────────────────────────

CREATE OR REPLACE VIEW nano_agent_traces_live AS
    SELECT * FROM nano_agent_traces WHERE tombstone_erased_at IS NULL;

-- ── Comment on the columns for SQL-side documentation ────────────────

COMMENT ON COLUMN nano_agent_traces.tombstone_erased_at IS
    'Sprint 7 — N10 : when the trace was tombstoned (GDPR Art. 17 / retention).
     NULL ⇒ live trace.';
COMMENT ON COLUMN nano_agent_traces.tombstone_erasure_reason IS
    'Sprint 7 — N10 : why the trace was tombstoned.  See ErasureReason enum.';
COMMENT ON COLUMN nano_agent_traces.tombstone_erasure_request_id IS
    'Sprint 7 — N10 : optional link to a higher-level erasure request id.
     Lets auditors trace which GDPR ask led to this row''s erasure.';
COMMENT ON COLUMN nano_agent_traces.tombstone_erasure_actor IS
    'Sprint 7 — N10 : free-form actor that triggered the tombstone.
     Examples : "gdpr_saga:request:{uuid}", "retention_policy:90d",
     "operator:{username}".';
COMMENT ON COLUMN nano_agent_traces.tombstone_hmac IS
    'Sprint 7 — N10 : HMAC-SHA256 over the tombstone canonical body
     (trace_id, erased_at, reason, request_id, actor) keyed by the
     tenant audit key.  Detects tampering with any tombstone field.';
