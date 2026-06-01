-- Migration v004 — M50 Sprint 6 — Nano agent audit trace table.
--
-- Stores per-call traces with HMAC-chained integrity (cf
-- crates/efe-nano/src/audit/hmac_chain.rs).  Each row links to the
-- previous row for the same tenant via chain_parent_id ; the HMAC
-- covers (chain_parent_id ‖ payload_hash).
--
-- Forward-only + idempotent (uses IF NOT EXISTS).

CREATE TABLE IF NOT EXISTS nano_agent_traces (
    trace_id            UUID         PRIMARY KEY,
    tenant_id           UUID         NOT NULL,

    -- Per-op parent (DAG dependency).  Different from chain_parent_id.
    parent_trace_id     UUID         REFERENCES nano_agent_traces(trace_id),

    -- HMAC chain parent : the latest trace for this tenant when this
    -- row was inserted.  NULL for the first row of the tenant's chain.
    chain_parent_id     UUID         REFERENCES nano_agent_traces(trace_id),

    swarm_request_id    UUID         NOT NULL,
    operation_id        TEXT         NOT NULL,
    caller_agent_id     TEXT         NULL,

    nano_agent_id       TEXT         NOT NULL,
    nano_agent_version  TEXT         NOT NULL,
    execution_mode      TEXT         NOT NULL CHECK (
        execution_mode IN ('pure_rust', 'local_model', 'llm_augmented')
    ),

    input_hash          BYTEA        NOT NULL,       -- SHA-256, 32 bytes
    output_hash         BYTEA        NULL,           -- NULL on error
    error_kind          TEXT         NULL,

    started_at          TIMESTAMPTZ  NOT NULL,
    completed_at        TIMESTAMPTZ  NULL,
    latency_ms          INTEGER      NULL,
    cost_cents          DOUBLE PRECISION NOT NULL DEFAULT 0,
    llm_tokens_used     INTEGER      NULL,

    -- HMAC chain fields :
    payload_hash        BYTEA        NOT NULL,       -- SHA-256 of canonical row body
    hmac                BYTEA        NOT NULL,       -- HMAC-SHA-256(parent ‖ payload_hash, key)

    -- Server-side insertion timestamp.
    inserted_at         TIMESTAMPTZ  NOT NULL DEFAULT NOW(),

    -- Metadata.
    audit_tags          TEXT[]       NOT NULL DEFAULT '{}'::TEXT[]
);

-- Row-level security : per-tenant isolation (production deployment).
-- Tests use a connection with `SET LOCAL efe.tenant_id = '<uuid>'`.
ALTER TABLE nano_agent_traces ENABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS tenant_isolation_select ON nano_agent_traces;
CREATE POLICY tenant_isolation_select ON nano_agent_traces
    FOR SELECT
    USING (tenant_id::TEXT = current_setting('efe.tenant_id', true));

DROP POLICY IF EXISTS tenant_isolation_insert ON nano_agent_traces;
CREATE POLICY tenant_isolation_insert ON nano_agent_traces
    FOR INSERT
    WITH CHECK (tenant_id::TEXT = current_setting('efe.tenant_id', true));
