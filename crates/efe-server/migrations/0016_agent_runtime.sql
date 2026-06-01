-- 0016_agent_runtime.sql
-- M25 — Agent runtime (experimental, EFE 2.0)
-- Runs on top of 0015_active_learning.sql (M24).
--
-- Adds 7 new tables, 2 ENUM types, 11 columns on tenant_feature, RLS,
-- 2 state-machine triggers, and HMAC-chained agent_audit table.

BEGIN;

-- ─── ENUM types ──────────────────────────────────────────────────────
--
-- Names match the Rust enums (`run.rs::AgentRunStatus`,
-- `approval.rs::ApprovalStatus`) under
-- `#[sqlx(rename_all = "SCREAMING_SNAKE_CASE")]`.

CREATE TYPE agent_run_status AS ENUM (
    'PENDING',
    'PROVISIONING',
    'RUNNING',
    'AWAITING_APPROVAL',
    'PAUSED',
    'RESUMING',
    'COMPLETED',
    'FAILED',
    'TERMINATED',
    'TIMED_OUT',
    'COST_CAPPED'
);

CREATE TYPE agent_approval_status AS ENUM (
    'PENDING',
    'GRANTED',
    'REJECTED',
    'EXPIRED',
    'CONSUMED'
);

-- ─── agent_run_template ──────────────────────────────────────────────
-- Reusable parameterized agent definition. Versioned per (tenant, slug).

CREATE TABLE agent_run_template (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    slug TEXT NOT NULL CHECK (slug ~ '^[A-Za-z0-9_-]{1,64}$'),
    display_name TEXT NOT NULL,
    description TEXT,
    profile TEXT NOT NULL,
    default_provider TEXT NOT NULL,
    default_model TEXT NOT NULL,
    system_prompt TEXT NOT NULL CHECK (length(system_prompt) BETWEEN 1 AND 32768),
    tool_whitelist_json JSONB NOT NULL,        -- array of tool names
    max_steps INT NOT NULL CHECK (max_steps BETWEEN 1 AND 200),
    default_timeout_seconds INT NOT NULL CHECK (default_timeout_seconds BETWEEN 60 AND 14400),
    default_max_cost_cents BIGINT NOT NULL CHECK (default_max_cost_cents >= 1),
    require_approval_for_writes BOOLEAN NOT NULL DEFAULT TRUE,
    created_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    version INT NOT NULL CHECK (version >= 1),
    deprecated BOOLEAN NOT NULL DEFAULT FALSE,
    -- Sigstore lineage (optional ; populated by promote pipeline).
    cosign_signature_url TEXT,
    cosign_cert_url TEXT,
    rekor_log_index BIGINT,
    UNIQUE(tenant_id, slug, version)
);

CREATE INDEX agent_template_tenant_slug_idx ON agent_run_template(tenant_id, slug);
CREATE INDEX agent_template_active_idx ON agent_run_template(tenant_id, slug, version DESC)
    WHERE deprecated = FALSE;

-- ─── agent_run ────────────────────────────────────────────────────────

CREATE TABLE agent_run (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    template_id TEXT NOT NULL,                -- template slug (FK enforced via app)
    profile TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    status agent_run_status NOT NULL DEFAULT 'PENDING',
    created_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    timeout_seconds INT NOT NULL CHECK (timeout_seconds BETWEEN 60 AND 14400),
    max_cost_cents BIGINT NOT NULL CHECK (max_cost_cents >= 1),
    current_cost_cents BIGINT NOT NULL DEFAULT 0 CHECK (current_cost_cents >= 0),
    current_step INT NOT NULL DEFAULT 0 CHECK (current_step >= 0),
    max_steps INT NOT NULL CHECK (max_steps BETWEEN 1 AND 200),
    vm_id TEXT,
    initial_prompt_json JSONB NOT NULL,
    tool_whitelist_json JSONB NOT NULL,
    final_output_json JSONB,
    failure_reason TEXT,
    -- Compliance / DPIA snapshot at submission time.
    dpia_addendum_hash TEXT,
    -- Optional links.
    k8s_pod_name TEXT,
    paused_at TIMESTAMPTZ,
    resumed_at TIMESTAMPTZ
);

CREATE INDEX agent_run_active_idx
    ON agent_run(tenant_id, status, created_at)
    WHERE status NOT IN ('COMPLETED', 'FAILED', 'TERMINATED', 'TIMED_OUT', 'COST_CAPPED');

CREATE INDEX agent_run_history_idx ON agent_run(tenant_id, created_at DESC);

-- ─── agent_step ──────────────────────────────────────────────────────

CREATE TABLE agent_step (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_run_id UUID NOT NULL REFERENCES agent_run(id) ON DELETE RESTRICT,
    step_index INT NOT NULL CHECK (step_index >= 0),
    kind TEXT NOT NULL CHECK (kind IN (
        'llm_call', 'tool_call', 'approval_request', 'approval_response',
        'final_output', 'pause', 'resume'
    )),
    tool_name TEXT,
    payload_json JSONB,
    cost_cents BIGINT NOT NULL DEFAULT 0 CHECK (cost_cents >= 0),
    duration_ms INT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    status TEXT NOT NULL DEFAULT 'IN_PROGRESS'
        CHECK (status IN ('IN_PROGRESS', 'OK', 'ERROR', 'AWAITING_APPROVAL')),
    error_message TEXT,
    UNIQUE(agent_run_id, step_index)
);

CREATE INDEX agent_step_run_idx ON agent_step(agent_run_id, step_index);

-- ─── agent_approval_request ──────────────────────────────────────────

CREATE TABLE agent_approval_request (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_run_id UUID NOT NULL REFERENCES agent_run(id) ON DELETE RESTRICT,
    tenant_id TEXT NOT NULL,
    step_index INT NOT NULL,
    tool_name TEXT NOT NULL,
    tool_args_json JSONB NOT NULL,
    justification TEXT,
    requested_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    status agent_approval_status NOT NULL DEFAULT 'PENDING',
    decided_at TIMESTAMPTZ,
    decided_by TEXT,
    decision_reason TEXT,
    expires_at TIMESTAMPTZ NOT NULL,
    -- Token (single-use HMAC). Stored as nonce only ; full token = HMAC(key, ...) computed on demand.
    token_nonce TEXT CHECK (token_nonce IS NULL OR token_nonce ~ '^[a-f0-9]{32}$'),
    consumed_at TIMESTAMPTZ,
    UNIQUE(agent_run_id, step_index)
);

CREATE INDEX agent_approval_pending_idx
    ON agent_approval_request(status, expires_at)
    WHERE status = 'PENDING';

CREATE INDEX agent_approval_run_idx
    ON agent_approval_request(agent_run_id);

CREATE INDEX agent_approval_tenant_idx
    ON agent_approval_request(tenant_id, status);

-- ─── agent_state (encrypted checkpoint) ──────────────────────────────
-- Composite key = (run_id, checkpoint_seq) so we keep history.

CREATE TABLE agent_state (
    agent_run_id UUID NOT NULL REFERENCES agent_run(id) ON DELETE CASCADE,
    checkpoint_seq INT NOT NULL CHECK (checkpoint_seq >= 1),
    state_blob_b64 TEXT NOT NULL,
    kms_key_id TEXT NOT NULL,
    bytes_size INT NOT NULL CHECK (bytes_size >= 0),
    digest_sha256 TEXT NOT NULL CHECK (digest_sha256 ~ '^[a-f0-9]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    schema_version INT NOT NULL DEFAULT 1,
    PRIMARY KEY (agent_run_id, checkpoint_seq)
);

CREATE INDEX agent_state_latest_idx
    ON agent_state(agent_run_id, checkpoint_seq DESC);

-- ─── agent_memory (cross-run, opt-in) ────────────────────────────────

CREATE TABLE agent_memory (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    namespace TEXT NOT NULL CHECK (length(namespace) BETWEEN 1 AND 64
        AND namespace ~ '^[a-z0-9][a-z0-9_-]*$'),
    created_by_run_id UUID REFERENCES agent_run(id) ON DELETE SET NULL,
    visibility TEXT NOT NULL DEFAULT 'private'
        CHECK (visibility IN ('private', 'team')),
    key TEXT NOT NULL CHECK (length(key) BETWEEN 1 AND 200),
    value_json JSONB NOT NULL,
    ttl_expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(tenant_id, namespace, visibility, key)
);

CREATE INDEX agent_memory_namespace_idx
    ON agent_memory(tenant_id, namespace);
CREATE INDEX agent_memory_ttl_idx
    ON agent_memory(ttl_expires_at)
    WHERE ttl_expires_at IS NOT NULL;

-- ─── agent_audit (HMAC chained, mirrors learning_audit) ──────────────

CREATE TABLE agent_audit (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    parent_id UUID,
    agent_run_id UUID REFERENCES agent_run(id),
    agent_step_id UUID REFERENCES agent_step(id),
    event_type TEXT NOT NULL CHECK (event_type IN (
        'run_created', 'microvm_provisioned', 'run_started',
        'llm_call_completed', 'tool_call_attempted', 'tool_call_completed',
        'tool_call_rejected_whitelist', 'tool_call_rejected_approval',
        'approval_requested', 'approval_granted', 'approval_rejected',
        'approval_expired', 'approval_consumed',
        'pause_requested', 'run_paused', 'run_resumed',
        'cost_warning_80pct', 'cost_cap_exceeded', 'budget_exceeded',
        'sandbox_escape_attempt_detected',
        'final_output', 'run_completed', 'run_terminated',
        'run_failed', 'run_timed_out',
        'gdpr_art17_cascade'
    )),
    actor_id TEXT NOT NULL,
    actor_role TEXT,
    payload_json JSONB NOT NULL,
    payload_hash TEXT NOT NULL CHECK (payload_hash ~ '^[a-f0-9]{64}$'),
    hmac TEXT NOT NULL CHECK (hmac ~ '^[a-f0-9]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT agent_audit_parent_fk
        FOREIGN KEY (parent_id) REFERENCES agent_audit(id)
        DEFERRABLE INITIALLY DEFERRED
);

CREATE INDEX agent_audit_chain_idx ON agent_audit(tenant_id, created_at);
CREATE INDEX agent_audit_run_idx ON agent_audit(agent_run_id) WHERE agent_run_id IS NOT NULL;
CREATE INDEX agent_audit_step_idx ON agent_audit(agent_step_id) WHERE agent_step_id IS NOT NULL;

-- ─── tenant_feature M25 columns ──────────────────────────────────────
--
-- Defensive : `tenant_feature` is assumed by M23/M24/M25 ; create it
-- here if no prior migration provided it. (The base shape is just a
-- per-tenant feature-flag bag keyed by tenant id ; columns accumulate
-- via ALTER across migrations.)

CREATE TABLE IF NOT EXISTS tenant_feature (
    id TEXT PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

ALTER TABLE tenant_feature
    ADD COLUMN IF NOT EXISTS m25_agent_runtime_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS m25_dpia_signed_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS m25_dpia_expires_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS m25_dpia_addendum_hash TEXT,
    ADD COLUMN IF NOT EXISTS m25_cost_cap_per_run_usd DOUBLE PRECISION DEFAULT 0,
    ADD COLUMN IF NOT EXISTS m25_timeout_minutes_max INT DEFAULT 60,
    ADD COLUMN IF NOT EXISTS m25_allowed_profiles TEXT[] DEFAULT '{}',
    ADD COLUMN IF NOT EXISTS m25_allowed_providers TEXT[] DEFAULT '{}',
    ADD COLUMN IF NOT EXISTS m25_memory_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS m25_microvm_pool_size INT DEFAULT 2,
    ADD COLUMN IF NOT EXISTS m25_max_concurrent_runs INT DEFAULT 5;

-- ─── RLS policies ────────────────────────────────────────────────────

ALTER TABLE agent_run_template ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_run ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_step ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_approval_request ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_state ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_memory ENABLE ROW LEVEL SECURITY;
ALTER TABLE agent_audit ENABLE ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation_agent_run_template ON agent_run_template
    USING (tenant_id = current_setting('app.current_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

CREATE POLICY tenant_isolation_agent_run ON agent_run
    USING (tenant_id = current_setting('app.current_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

CREATE POLICY tenant_isolation_agent_step ON agent_step
    USING (
        EXISTS (SELECT 1 FROM agent_run ar
                WHERE ar.id = agent_step.agent_run_id
                  AND ar.tenant_id = current_setting('app.current_tenant_id', true))
    );

CREATE POLICY tenant_isolation_agent_approval ON agent_approval_request
    USING (tenant_id = current_setting('app.current_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

CREATE POLICY tenant_isolation_agent_state ON agent_state
    USING (
        EXISTS (SELECT 1 FROM agent_run ar
                WHERE ar.id = agent_state.agent_run_id
                  AND ar.tenant_id = current_setting('app.current_tenant_id', true))
    );

CREATE POLICY tenant_isolation_agent_memory ON agent_memory
    USING (tenant_id = current_setting('app.current_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

CREATE POLICY tenant_isolation_agent_audit ON agent_audit
    USING (tenant_id = current_setting('app.current_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

-- ─── State machine triggers ──────────────────────────────────────────
--
-- Strict whitelist of allowed transitions. Mirrors
-- `run.rs::AgentRunStatus::can_transition_to` and
-- `approval.rs::ApprovalStatus`.

CREATE OR REPLACE FUNCTION fn_agent_run_state_machine()
RETURNS TRIGGER AS $$
BEGIN
    IF (OLD.status::text, NEW.status::text) NOT IN (
        ('PENDING',           'PROVISIONING'),
        ('PENDING',           'FAILED'),
        ('PENDING',           'TERMINATED'),
        ('PROVISIONING',      'RUNNING'),
        ('PROVISIONING',      'FAILED'),
        ('PROVISIONING',      'TERMINATED'),
        ('RUNNING',           'AWAITING_APPROVAL'),
        ('RUNNING',           'PAUSED'),
        ('RUNNING',           'COMPLETED'),
        ('RUNNING',           'FAILED'),
        ('RUNNING',           'TERMINATED'),
        ('RUNNING',           'TIMED_OUT'),
        ('RUNNING',           'COST_CAPPED'),
        ('AWAITING_APPROVAL', 'RUNNING'),
        ('AWAITING_APPROVAL', 'FAILED'),
        ('AWAITING_APPROVAL', 'TERMINATED'),
        ('AWAITING_APPROVAL', 'TIMED_OUT'),
        ('PAUSED',            'RESUMING'),
        ('PAUSED',            'TERMINATED'),
        ('RESUMING',          'RUNNING'),
        ('RESUMING',          'FAILED')
    ) THEN
        RAISE EXCEPTION 'invalid agent_run transition : % -> %', OLD.status, NEW.status;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_agent_run_state_machine
    BEFORE UPDATE ON agent_run
    FOR EACH ROW
    WHEN (OLD.status IS DISTINCT FROM NEW.status)
    EXECUTE FUNCTION fn_agent_run_state_machine();

CREATE OR REPLACE FUNCTION fn_agent_approval_state_machine()
RETURNS TRIGGER AS $$
BEGIN
    IF (OLD.status::text, NEW.status::text) NOT IN (
        ('PENDING',  'GRANTED'),
        ('PENDING',  'REJECTED'),
        ('PENDING',  'EXPIRED'),
        ('GRANTED',  'CONSUMED'),
        ('GRANTED',  'EXPIRED')   -- granted but never consumed → expires
    ) THEN
        RAISE EXCEPTION 'invalid agent_approval transition : % -> %', OLD.status, NEW.status;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_agent_approval_state_machine
    BEFORE UPDATE ON agent_approval_request
    FOR EACH ROW
    WHEN (OLD.status IS DISTINCT FROM NEW.status)
    EXECUTE FUNCTION fn_agent_approval_state_machine();

-- ─── Schema migration tracking ───────────────────────────────────────

INSERT INTO schema_migration (id, applied_at, description)
VALUES (
    '0016_agent_runtime',
    NOW(),
    'M25 — Agent runtime experimental : 7 tables + 11 tenant_feature columns + 2 ENUMs + RLS + state machine triggers'
)
ON CONFLICT (id) DO NOTHING;

COMMIT;
