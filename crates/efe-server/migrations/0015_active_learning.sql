-- 0015_active_learning.sql
-- M24 — Active learning loop
-- Runs on top of 0014_taxonomy_curation.sql (M23).
--
-- Adds 7 new tables, 16 columns on tenant_feature, 3 columns on catalog,
-- RLS policies, and 2 state-machine triggers.

BEGIN;

-- ─── ENUM types ───────────────────────────────────────────────────────

CREATE TYPE gold_label_queue_status AS ENUM (
    'ENQUEUED',
    'LABELING',
    'LLM_LABELED',
    'AWAITING_REVIEW',
    'CONFIRMED',
    'REJECTED',
    'FAILED',
    'TOMBSTONED'
);

CREATE TYPE gold_label_review_decision AS ENUM (
    'CONFIRM',
    'EDIT_AND_CONFIRM',
    'REJECT'
);

CREATE TYPE retune_run_status AS ENUM (
    'TUNING',
    'EVAL_RUNNING',
    'EVAL_PASS',
    'EVAL_FAIL',
    'PROMOTED',
    'ROLLED_BACK',
    'ABORTED'
);

CREATE TYPE stakes_tier AS ENUM ('LOW', 'MEDIUM', 'HIGH');

-- ─── gold_label_queue ─────────────────────────────────────────────────

CREATE TABLE gold_label_queue (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    catalog_id TEXT NOT NULL,
    content_hash TEXT NOT NULL CHECK (content_hash ~ '^[a-f0-9]{64}$'),
    sentence_redacted TEXT NOT NULL,
    proposed_class_id TEXT,
    observed_confidence DOUBLE PRECISION CHECK (
        observed_confidence IS NULL
        OR (observed_confidence >= 0 AND observed_confidence <= 1)
    ),
    status gold_label_queue_status NOT NULL DEFAULT 'ENQUEUED',
    source_llm_call_id UUID,
    enqueued_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    labeled_at TIMESTAMPTZ,
    confirmed_at TIMESTAMPTZ,
    failure_count INT NOT NULL DEFAULT 0 CHECK (failure_count >= 0),
    last_error TEXT,
    UNIQUE(tenant_id, content_hash)
);

CREATE INDEX gold_label_queue_status_idx
    ON gold_label_queue(tenant_id, status, enqueued_at)
    WHERE status IN ('ENQUEUED', 'LABELING', 'LLM_LABELED', 'AWAITING_REVIEW');

CREATE INDEX gold_label_queue_catalog_idx
    ON gold_label_queue(tenant_id, catalog_id);

-- ─── gold_label_proposal ──────────────────────────────────────────────

CREATE TABLE gold_label_proposal (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    queue_id UUID NOT NULL REFERENCES gold_label_queue(id) ON DELETE RESTRICT,
    tenant_id TEXT NOT NULL,
    proposed_class_id TEXT NOT NULL,
    llm_confidence DOUBLE PRECISION NOT NULL CHECK (llm_confidence >= 0 AND llm_confidence <= 1),
    llm_rationale TEXT NOT NULL,
    llm_call_audit_id UUID,
    prompt_template_name TEXT NOT NULL,
    prompt_template_version INT NOT NULL,
    auto_confirmed BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX gold_label_proposal_queue_idx ON gold_label_proposal(queue_id);
CREATE INDEX gold_label_proposal_tenant_idx ON gold_label_proposal(tenant_id, created_at DESC);

-- ─── gold_label_review ────────────────────────────────────────────────

CREATE TABLE gold_label_review (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    proposal_id UUID NOT NULL REFERENCES gold_label_proposal(id) ON DELETE RESTRICT,
    reviewer_oidc_sub TEXT NOT NULL,
    reviewer_role TEXT NOT NULL,
    decision gold_label_review_decision NOT NULL,
    final_class_id TEXT,
    rationale TEXT NOT NULL CHECK (length(rationale) >= 10),
    quality_tag TEXT,                -- 'good', 'edited', 'unclear', 'llm_drift', NULL
    oidc_id_token_hash TEXT NOT NULL,
    oidc_token_iat TIMESTAMPTZ NOT NULL,
    reviewed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(proposal_id, reviewer_oidc_sub),
    CONSTRAINT review_oidc_iat_fresh CHECK (reviewed_at - oidc_token_iat < INTERVAL '24 hours')
);

CREATE INDEX gold_label_review_proposal_idx ON gold_label_review(proposal_id);

-- ─── bundle_retune_run ────────────────────────────────────────────────

CREATE TABLE bundle_retune_run (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    catalog_id TEXT NOT NULL,
    parent_taxonomy_version_id UUID NOT NULL REFERENCES taxonomy_version(id),
    new_taxonomy_version_id UUID REFERENCES taxonomy_version(id),
    new_bundle_semver TEXT NOT NULL,
    gold_dataset_id UUID NOT NULL REFERENCES gold_dataset(id),
    training_examples_count BIGINT NOT NULL CHECK (training_examples_count >= 0),
    holdout_examples_count BIGINT NOT NULL CHECK (holdout_examples_count >= 0),
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    status retune_run_status NOT NULL DEFAULT 'TUNING',
    eval_metrics_json JSONB,
    flagged_for_promotion BOOLEAN NOT NULL DEFAULT FALSE,
    cosign_signature_url TEXT,
    rekor_log_index BIGINT,
    triggered_by TEXT NOT NULL CHECK (triggered_by IN ('scheduler', 'on_demand', 'gdpr_cascade')),
    triggered_by_actor TEXT
);

CREATE INDEX retune_run_pending_idx
    ON bundle_retune_run(tenant_id, catalog_id, status)
    WHERE status IN ('TUNING', 'EVAL_RUNNING', 'EVAL_PASS');

CREATE INDEX retune_run_history_idx
    ON bundle_retune_run(tenant_id, catalog_id, started_at DESC);

-- ─── bundle_holdout_split ─────────────────────────────────────────────

CREATE TABLE bundle_holdout_split (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    retune_run_id UUID NOT NULL REFERENCES bundle_retune_run(id) ON DELETE CASCADE,
    gold_example_id UUID NOT NULL REFERENCES gold_example(id),
    UNIQUE(retune_run_id, gold_example_id)
);

CREATE INDEX bundle_holdout_split_run_idx ON bundle_holdout_split(retune_run_id);

-- ─── bundle_rebuild_request (GDPR Art. 17 cascade) ────────────────────

CREATE TABLE bundle_rebuild_request (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    catalog_id TEXT NOT NULL,
    reason TEXT NOT NULL CHECK (reason IN ('gdpr_art17', 'security_incident', 'manual')),
    urgency TEXT NOT NULL CHECK (urgency IN ('standard', 'regulatory', 'catastrophic')),
    affected_gold_example_ids UUID[] NOT NULL,
    requester_subject_id TEXT,
    request_received_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    sla_deadline TIMESTAMPTZ NOT NULL,
    completed_at TIMESTAMPTZ,
    completed_retune_run_id UUID REFERENCES bundle_retune_run(id),
    status TEXT NOT NULL DEFAULT 'PENDING'
        CHECK (status IN ('PENDING', 'IN_PROGRESS', 'COMPLETED', 'FAILED'))
);

CREATE INDEX bundle_rebuild_pending_idx
    ON bundle_rebuild_request(tenant_id, sla_deadline)
    WHERE status IN ('PENDING', 'IN_PROGRESS');

-- ─── learning_audit (HMAC chained) ───────────────────────────────────

CREATE TABLE learning_audit (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    parent_id UUID,
    queue_id UUID REFERENCES gold_label_queue(id),
    proposal_id UUID REFERENCES gold_label_proposal(id),
    retune_run_id UUID REFERENCES bundle_retune_run(id),
    taxonomy_version_id UUID REFERENCES taxonomy_version(id),
    event_type TEXT NOT NULL CHECK (event_type IN (
        'sample_enqueued', 'llm_label_proposed', 'auto_confirmed',
        'human_review_confirm', 'human_review_edit', 'human_review_reject',
        'gold_example_inserted', 'retune_started', 'retune_eval_pass',
        'retune_eval_fail', 'retune_aborted', 'promotion_eligibility_check',
        'promote_canary_started', 'promote_completed', 'promote_rolled_back',
        'gdpr_art17_redaction', 'gdpr_art17_force_promote',
        'budget_exceeded', 'pii_unredactable_skipped',
        'sample_failed_3x', 'centroid_drift_aborted'
    )),
    actor_id TEXT NOT NULL,
    actor_role TEXT,
    payload_json JSONB NOT NULL,
    payload_hash TEXT NOT NULL CHECK (payload_hash ~ '^[a-f0-9]{64}$'),
    hmac TEXT NOT NULL CHECK (hmac ~ '^[a-f0-9]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT learning_audit_parent_fk
        FOREIGN KEY (parent_id) REFERENCES learning_audit(id) DEFERRABLE INITIALLY DEFERRED
);

CREATE INDEX learning_audit_chain_idx ON learning_audit(tenant_id, created_at);
CREATE INDEX learning_audit_queue_idx ON learning_audit(queue_id) WHERE queue_id IS NOT NULL;
CREATE INDEX learning_audit_retune_idx ON learning_audit(retune_run_id) WHERE retune_run_id IS NOT NULL;

-- ─── tenant_feature M24 columns ──────────────────────────────────────

ALTER TABLE tenant_feature
    ADD COLUMN IF NOT EXISTS m24_active_learning_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS m24_auto_promotion_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS m24_human_review_required BOOLEAN NOT NULL DEFAULT TRUE,
    ADD COLUMN IF NOT EXISTS m24_dpo_consent_signed_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS m24_dpo_consent_expires_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS m24_uncertainty_threshold DOUBLE PRECISION DEFAULT 0.70,
    ADD COLUMN IF NOT EXISTS m24_auto_confirm_threshold DOUBLE PRECISION DEFAULT 0.90,
    ADD COLUMN IF NOT EXISTS m24_queue_max_size INT DEFAULT 10000,
    ADD COLUMN IF NOT EXISTS m24_gold_label_threshold INT DEFAULT 1000,
    ADD COLUMN IF NOT EXISTS m24_retune_interval_days INT DEFAULT 7,
    ADD COLUMN IF NOT EXISTS m24_anchor_weight_alpha DOUBLE PRECISION DEFAULT 0.70,
    ADD COLUMN IF NOT EXISTS m24_max_centroid_drift DOUBLE PRECISION DEFAULT 0.30,
    ADD COLUMN IF NOT EXISTS m24_promotion_threshold_pp DOUBLE PRECISION DEFAULT 1.00,
    ADD COLUMN IF NOT EXISTS m24_active_learning_budget_cents_per_day BIGINT DEFAULT 0,
    ADD COLUMN IF NOT EXISTS m24_review_sla_hours INT DEFAULT 48,
    ADD COLUMN IF NOT EXISTS m24_notify_after_hours INT DEFAULT 8;

-- ─── catalog M24 columns ─────────────────────────────────────────────

ALTER TABLE catalog
    ADD COLUMN IF NOT EXISTS m24_al_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS m24_stakes_tier stakes_tier NOT NULL DEFAULT 'MEDIUM',
    ADD COLUMN IF NOT EXISTS m24_drift_threshold_pp DOUBLE PRECISION DEFAULT 2.00;

-- ─── RLS policies ────────────────────────────────────────────────────

ALTER TABLE gold_label_queue ENABLE ROW LEVEL SECURITY;
ALTER TABLE gold_label_proposal ENABLE ROW LEVEL SECURITY;
ALTER TABLE gold_label_review ENABLE ROW LEVEL SECURITY;
ALTER TABLE bundle_retune_run ENABLE ROW LEVEL SECURITY;
ALTER TABLE bundle_holdout_split ENABLE ROW LEVEL SECURITY;
ALTER TABLE bundle_rebuild_request ENABLE ROW LEVEL SECURITY;
ALTER TABLE learning_audit ENABLE ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation_gold_label_queue ON gold_label_queue
    USING (tenant_id = current_setting('app.current_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

CREATE POLICY tenant_isolation_gold_label_proposal ON gold_label_proposal
    USING (tenant_id = current_setting('app.current_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

CREATE POLICY tenant_isolation_gold_label_review ON gold_label_review
    USING (
        EXISTS (SELECT 1 FROM gold_label_proposal p
                WHERE p.id = gold_label_review.proposal_id
                  AND p.tenant_id = current_setting('app.current_tenant_id', true))
    );

CREATE POLICY tenant_isolation_bundle_retune_run ON bundle_retune_run
    USING (tenant_id = current_setting('app.current_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

CREATE POLICY tenant_isolation_bundle_holdout_split ON bundle_holdout_split
    USING (
        EXISTS (SELECT 1 FROM bundle_retune_run br
                WHERE br.id = bundle_holdout_split.retune_run_id
                  AND br.tenant_id = current_setting('app.current_tenant_id', true))
    );

CREATE POLICY tenant_isolation_bundle_rebuild_request ON bundle_rebuild_request
    USING (tenant_id = current_setting('app.current_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

CREATE POLICY tenant_isolation_learning_audit ON learning_audit
    USING (tenant_id = current_setting('app.current_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

-- ─── State machine triggers ──────────────────────────────────────────

CREATE OR REPLACE FUNCTION fn_gold_label_queue_state_machine()
RETURNS TRIGGER AS $$
BEGIN
    IF (OLD.status::text, NEW.status::text) NOT IN (
        ('ENQUEUED', 'LABELING'),     ('ENQUEUED', 'FAILED'),    ('ENQUEUED', 'TOMBSTONED'),
        ('LABELING', 'LLM_LABELED'),  ('LABELING', 'ENQUEUED'),  ('LABELING', 'FAILED'),
        ('LLM_LABELED', 'AWAITING_REVIEW'), ('LLM_LABELED', 'CONFIRMED'), ('LLM_LABELED', 'REJECTED'),
        ('AWAITING_REVIEW', 'CONFIRMED'),   ('AWAITING_REVIEW', 'REJECTED'), ('AWAITING_REVIEW', 'TOMBSTONED'),
        ('CONFIRMED', 'TOMBSTONED')
    ) THEN
        RAISE EXCEPTION 'invalid gold_label_queue transition : % -> %', OLD.status, NEW.status;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_gold_label_queue_state_machine
    BEFORE UPDATE ON gold_label_queue
    FOR EACH ROW
    WHEN (OLD.status IS DISTINCT FROM NEW.status)
    EXECUTE FUNCTION fn_gold_label_queue_state_machine();

CREATE OR REPLACE FUNCTION fn_retune_run_state_machine()
RETURNS TRIGGER AS $$
BEGIN
    IF (OLD.status::text, NEW.status::text) NOT IN (
        ('TUNING', 'EVAL_RUNNING'),   ('TUNING', 'ABORTED'),
        ('EVAL_RUNNING', 'EVAL_PASS'),('EVAL_RUNNING', 'EVAL_FAIL'),('EVAL_RUNNING', 'ABORTED'),
        ('EVAL_PASS', 'PROMOTED'),    ('EVAL_PASS', 'ROLLED_BACK'),('EVAL_PASS', 'ABORTED'),
        ('EVAL_FAIL', 'ABORTED'),
        ('PROMOTED', 'ROLLED_BACK')
    ) THEN
        RAISE EXCEPTION 'invalid retune_run transition : % -> %', OLD.status, NEW.status;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_retune_run_state_machine
    BEFORE UPDATE ON bundle_retune_run
    FOR EACH ROW
    WHEN (OLD.status IS DISTINCT FROM NEW.status)
    EXECUTE FUNCTION fn_retune_run_state_machine();

-- ─── Holdout views (deterministic 10% split) ─────────────────────────
--
-- Per-(tenant, catalog, gold_example) deterministic split using
-- SHA-256(tenant || '|' || catalog || '|' || example_id) byte 31 mod 10
-- == 0 → holdout. Uses `get_byte(bytea, int)` which is unambiguous and
-- stable across Postgres versions (an earlier draft used a `('x'||hex)::bit(8)`
-- trick that had pgcrypto-version sensitivity ; replaced during M24 audit).
--
-- The Rust function `is_in_holdout` MUST stay byte-for-byte in sync.
-- Pin test : `test_holdout_view_matches_rust_function`.

CREATE OR REPLACE VIEW v_gold_examples_holdout AS
SELECT ge.*
FROM gold_example ge
JOIN gold_dataset gd ON gd.id = ge.gold_dataset_id
WHERE (
    get_byte(
        digest(gd.tenant_id || '|' || gd.catalog_id || '|' || ge.id::text, 'sha256'),
        31
    ) % 10
) = 0;

CREATE OR REPLACE VIEW v_gold_examples_training AS
SELECT ge.*
FROM gold_example ge
JOIN gold_dataset gd ON gd.id = ge.gold_dataset_id
WHERE (
    get_byte(
        digest(gd.tenant_id || '|' || gd.catalog_id || '|' || ge.id::text, 'sha256'),
        31
    ) % 10
) <> 0;

-- ─── Schema migration tracking ───────────────────────────────────────

INSERT INTO schema_migration (id, applied_at, description)
VALUES (
    '0015_active_learning',
    NOW(),
    'M24 — Active learning : 7 tables + 16 tenant_feature columns + 3 catalog columns + RLS + state machine triggers + holdout views'
)
ON CONFLICT (id) DO NOTHING;

COMMIT;
