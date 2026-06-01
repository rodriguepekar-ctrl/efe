-- Migration v005 — M50 Sprint 6 — Chain indexes.
--
-- Optimizes the four hot-path queries :
-- 1. Latest trace for a tenant (chain tip lookup on append).
-- 2. List traces for a swarm_request_id (replay).
-- 3. Chain walk for verification.
-- 4. Per-agent analytics (rollup).

CREATE INDEX IF NOT EXISTS traces_tenant_started_idx
    ON nano_agent_traces (tenant_id, started_at DESC);

CREATE INDEX IF NOT EXISTS traces_swarm_idx
    ON nano_agent_traces (swarm_request_id);

CREATE INDEX IF NOT EXISTS traces_chain_parent_idx
    ON nano_agent_traces (chain_parent_id)
    WHERE chain_parent_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS traces_nano_agent_started_idx
    ON nano_agent_traces (nano_agent_id, started_at DESC);

-- GIN index for tag-based search (e.g. WHERE audit_tags @> ARRAY['phase-2']).
CREATE INDEX IF NOT EXISTS traces_audit_tags_gin
    ON nano_agent_traces USING GIN (audit_tags);
