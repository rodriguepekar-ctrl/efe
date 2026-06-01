-- 0014_taxonomy_curation.sql
-- M23 — LLM-assisted taxonomy curation
--
-- Adds 7 new tables + 10 new columns on tenant_feature, plus RLS policies
-- and HMAC-chained audit trail. Runs on top of 0013_d8_admin_self_service.sql.
--
-- All tables are tenant-scoped via Postgres row-level security (RLS).
-- The tenant context is set per-session by efe-server middleware via
--   SELECT set_config('app.current_tenant_id', $1, true);

BEGIN;

-- ─────────────────────────────────────────────────────────────────────
-- 1. ENUM types
-- ─────────────────────────────────────────────────────────────────────

CREATE TYPE taxonomy_version_status AS ENUM (
  'DRAFT',
  'CANDIDATE',
  'ACTIVE',
  'DEPRECATED',
  'ROLLED_BACK'
);

CREATE TYPE proposal_status AS ENUM (
  'DRAFT',
  'PENDING',
  'APPROVED',
  'REJECTED',
  'PROMOTED',
  'ROLLED_BACK',
  'EXPIRED'
);

CREATE TYPE proposal_kind AS ENUM (
  'SPLIT',
  'MERGE',
  'EXPLAIN',
  'GENERATE_SYNTH',
  'AUDIT_VS_GOLD'
);

CREATE TYPE drift_severity AS ENUM ('LOW', 'MEDIUM', 'HIGH');

CREATE TYPE curation_review_decision AS ENUM ('APPROVE', 'REJECT');

-- ─────────────────────────────────────────────────────────────────────
-- 2. taxonomy_version
-- ─────────────────────────────────────────────────────────────────────

CREATE TABLE taxonomy_version (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id TEXT NOT NULL,
  catalog_id TEXT NOT NULL,
  semver TEXT NOT NULL CHECK (semver ~ '^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.-]+)?$'),
  parent_id UUID REFERENCES taxonomy_version(id),
  status taxonomy_version_status NOT NULL DEFAULT 'DRAFT',
  proposal_id UUID,                       -- forward FK ; added after proposal table
  anchors_json JSONB NOT NULL,
  cosign_signature_url TEXT,
  cosign_cert_url TEXT,
  rekor_log_index BIGINT,
  canary_started_at TIMESTAMPTZ,
  canary_completed_at TIMESTAMPTZ,
  canary_metrics_json JSONB,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  promoted_at TIMESTAMPTZ,
  rolled_back_at TIMESTAMPTZ,
  CONSTRAINT taxonomy_version_semver_unique UNIQUE(tenant_id, catalog_id, semver),
  -- only DRAFT/CANDIDATE versions may have NULL signatures
  CONSTRAINT taxonomy_version_signed_when_active CHECK (
    status NOT IN ('ACTIVE', 'DEPRECATED', 'ROLLED_BACK')
    OR cosign_signature_url IS NOT NULL
  )
);

-- Only ONE ACTIVE version allowed per (tenant, catalog) — partial unique index.
CREATE UNIQUE INDEX taxonomy_version_one_active_per_catalog
  ON taxonomy_version(tenant_id, catalog_id)
  WHERE status = 'ACTIVE';

CREATE INDEX taxonomy_version_parent_idx
  ON taxonomy_version(parent_id);

CREATE INDEX taxonomy_version_status_idx
  ON taxonomy_version(tenant_id, catalog_id, status);

CREATE INDEX taxonomy_version_promoted_idx
  ON taxonomy_version(tenant_id, promoted_at DESC)
  WHERE promoted_at IS NOT NULL;

-- ─────────────────────────────────────────────────────────────────────
-- 3. taxonomy_change_proposal
-- ─────────────────────────────────────────────────────────────────────

CREATE TABLE taxonomy_change_proposal (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id TEXT NOT NULL,
  catalog_id TEXT NOT NULL,
  parent_taxonomy_version_id UUID NOT NULL REFERENCES taxonomy_version(id),
  candidate_taxonomy_version_id UUID REFERENCES taxonomy_version(id),
  kind proposal_kind NOT NULL,
  status proposal_status NOT NULL DEFAULT 'DRAFT',
  proposed_by TEXT NOT NULL,                -- OIDC sub claim
  proposed_via TEXT NOT NULL CHECK (proposed_via IN ('mcp_tool', 'cli', 'portal', 'auto_drift')),
  llm_call_audit_id UUID,                   -- FK to llm_call_audit if internal_llm
  prompt_template_name TEXT NOT NULL,
  prompt_template_version INT NOT NULL,
  execution_mode TEXT NOT NULL CHECK (execution_mode IN ('SAMPLING_DELEGATION', 'INTERNAL_LLM')),
  rationale_markdown TEXT NOT NULL,
  diff_json JSONB NOT NULL,
  estimated_cost_cents BIGINT,
  actual_cost_cents BIGINT,
  idempotency_key TEXT,
  expires_at TIMESTAMPTZ NOT NULL DEFAULT NOW() + INTERVAL '90 days',
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  CONSTRAINT proposal_idempotency_unique
    UNIQUE NULLS NOT DISTINCT (tenant_id, idempotency_key)
);

CREATE INDEX proposal_pending_idx
  ON taxonomy_change_proposal(tenant_id, status, created_at DESC)
  WHERE status IN ('PENDING', 'APPROVED');

CREATE INDEX proposal_catalog_idx
  ON taxonomy_change_proposal(tenant_id, catalog_id, status);

CREATE INDEX proposal_kind_idx
  ON taxonomy_change_proposal(tenant_id, kind);

CREATE INDEX proposal_expires_idx
  ON taxonomy_change_proposal(expires_at)
  WHERE status IN ('PENDING', 'APPROVED');

-- Now that proposal table exists, add back-FK on taxonomy_version.proposal_id
ALTER TABLE taxonomy_version
  ADD CONSTRAINT taxonomy_version_proposal_fk
  FOREIGN KEY (proposal_id) REFERENCES taxonomy_change_proposal(id);

-- ─────────────────────────────────────────────────────────────────────
-- 4. curation_review (human review record)
-- ─────────────────────────────────────────────────────────────────────

CREATE TABLE curation_review (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  proposal_id UUID NOT NULL REFERENCES taxonomy_change_proposal(id) ON DELETE RESTRICT,
  reviewer_oidc_sub TEXT NOT NULL,
  reviewer_role TEXT NOT NULL,
  decision curation_review_decision NOT NULL,
  rationale TEXT NOT NULL CHECK (length(rationale) >= 10),
  oidc_id_token_hash TEXT NOT NULL,
  oidc_token_iat TIMESTAMPTZ NOT NULL,
  reviewed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  CONSTRAINT review_one_per_reviewer UNIQUE(proposal_id, reviewer_oidc_sub),
  -- token freshness sanity : iat within last 24h of reviewed_at
  CONSTRAINT review_oidc_iat_fresh CHECK (
    reviewed_at - oidc_token_iat < INTERVAL '24 hours'
  )
);

CREATE INDEX curation_review_proposal_idx
  ON curation_review(proposal_id);

CREATE INDEX curation_review_reviewer_idx
  ON curation_review(reviewer_oidc_sub, reviewed_at DESC);

-- ─────────────────────────────────────────────────────────────────────
-- 5. taxonomy_drift_event
-- ─────────────────────────────────────────────────────────────────────

CREATE TABLE taxonomy_drift_event (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id TEXT NOT NULL,
  catalog_id TEXT NOT NULL,
  class_id TEXT NOT NULL,
  detected_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  window_old_start TIMESTAMPTZ NOT NULL,
  window_new_end TIMESTAMPTZ NOT NULL,
  kl_divergence DOUBLE PRECISION NOT NULL CHECK (kl_divergence >= 0),
  mean_confidence_old DOUBLE PRECISION NOT NULL,
  mean_confidence_new DOUBLE PRECISION NOT NULL,
  support_change_pct DOUBLE PRECISION NOT NULL,
  severity drift_severity NOT NULL,
  triggered_proposal_id UUID REFERENCES taxonomy_change_proposal(id),
  acknowledged_at TIMESTAMPTZ,
  acknowledged_by TEXT
);

CREATE INDEX drift_event_unack_idx
  ON taxonomy_drift_event(tenant_id, catalog_id, severity, detected_at DESC)
  WHERE acknowledged_at IS NULL;

CREATE INDEX drift_event_recent_idx
  ON taxonomy_drift_event(tenant_id, detected_at DESC);

-- ─────────────────────────────────────────────────────────────────────
-- 6. taxonomy_change_audit (HMAC chained, mirrors llm_call_audit pattern)
-- ─────────────────────────────────────────────────────────────────────

CREATE TABLE taxonomy_change_audit (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id TEXT NOT NULL,
  parent_id UUID,                           -- previous row in chain ; NULL for genesis
  proposal_id UUID REFERENCES taxonomy_change_proposal(id),
  taxonomy_version_id UUID REFERENCES taxonomy_version(id),
  event_type TEXT NOT NULL CHECK (event_type IN (
    'genesis',
    'proposal_created',
    'proposal_validated',
    'proposal_pii_leak_blocked',
    'review_approve',
    'review_reject',
    'promote_started',
    'promote_canary_pass',
    'promote_canary_fail',
    'promote_completed',
    'rollback_manual',
    'rollback_auto',
    'gdpr_art17_redaction',
    'budget_exceeded'
  )),
  actor_id TEXT NOT NULL,                   -- OIDC sub OR 'system' for auto events
  actor_role TEXT,
  payload_json JSONB NOT NULL,
  payload_hash TEXT NOT NULL CHECK (payload_hash ~ '^[a-f0-9]{64}$'),
  hmac TEXT NOT NULL CHECK (hmac ~ '^[a-f0-9]{64}$'),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  -- chain integrity : if parent_id is set, it must point to a row of same tenant
  CONSTRAINT taxonomy_audit_parent_fk
    FOREIGN KEY (parent_id) REFERENCES taxonomy_change_audit(id) DEFERRABLE INITIALLY DEFERRED
);

CREATE INDEX taxonomy_audit_chain_idx
  ON taxonomy_change_audit(tenant_id, created_at);

CREATE INDEX taxonomy_audit_proposal_idx
  ON taxonomy_change_audit(proposal_id)
  WHERE proposal_id IS NOT NULL;

CREATE INDEX taxonomy_audit_version_idx
  ON taxonomy_change_audit(taxonomy_version_id)
  WHERE taxonomy_version_id IS NOT NULL;

-- ─────────────────────────────────────────────────────────────────────
-- 7. gold_dataset (versioned per tenant)
-- ─────────────────────────────────────────────────────────────────────

CREATE TABLE gold_dataset (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id TEXT NOT NULL,
  catalog_id TEXT NOT NULL,
  semver TEXT NOT NULL CHECK (semver ~ '^[0-9]+\.[0-9]+\.[0-9]+$'),
  parent_id UUID REFERENCES gold_dataset(id),
  description TEXT,
  examples_count INT NOT NULL CHECK (examples_count >= 0),
  per_class_counts_json JSONB NOT NULL,
  cosign_signature_url TEXT,
  rekor_log_index BIGINT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  CONSTRAINT gold_dataset_semver_unique UNIQUE(tenant_id, catalog_id, semver)
);

CREATE INDEX gold_dataset_tenant_idx ON gold_dataset(tenant_id, catalog_id, created_at DESC);

CREATE TABLE gold_example (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  gold_dataset_id UUID NOT NULL REFERENCES gold_dataset(id) ON DELETE CASCADE,
  text TEXT NOT NULL,
  expected_class_id TEXT NOT NULL,
  difficulty DOUBLE PRECISION CHECK (difficulty IS NULL OR (difficulty >= 0 AND difficulty <= 1)),
  source TEXT NOT NULL CHECK (source IN ('human_curated', 'llm_generated', 'production_overflow', 'imported')),
  pii_redacted_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX gold_example_dataset_idx
  ON gold_example(gold_dataset_id, expected_class_id);

CREATE INDEX gold_example_class_idx
  ON gold_example(expected_class_id);

-- ─────────────────────────────────────────────────────────────────────
-- 8. bundle_audit_scorecard
-- ─────────────────────────────────────────────────────────────────────

CREATE TABLE bundle_audit_scorecard (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id TEXT NOT NULL,
  catalog_id TEXT NOT NULL,
  bundle_id TEXT NOT NULL,
  gold_dataset_id UUID NOT NULL REFERENCES gold_dataset(id),
  taxonomy_version_id UUID NOT NULL REFERENCES taxonomy_version(id),
  accuracy DOUBLE PRECISION NOT NULL CHECK (accuracy >= 0 AND accuracy <= 1),
  macro_f1 DOUBLE PRECISION NOT NULL CHECK (macro_f1 >= 0 AND macro_f1 <= 1),
  weighted_f1 DOUBLE PRECISION NOT NULL CHECK (weighted_f1 >= 0 AND weighted_f1 <= 1),
  per_class_metrics_json JSONB NOT NULL,
  weak_spots_json JSONB,
  llm_explanation_markdown TEXT,
  computed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX scorecard_bundle_idx
  ON bundle_audit_scorecard(tenant_id, catalog_id, bundle_id, computed_at DESC);

CREATE INDEX scorecard_gold_idx
  ON bundle_audit_scorecard(gold_dataset_id);

-- ─────────────────────────────────────────────────────────────────────
-- 9. tenant_feature M23 columns
-- ─────────────────────────────────────────────────────────────────────

ALTER TABLE tenant_feature
  ADD COLUMN IF NOT EXISTS m23_curation_enabled BOOLEAN NOT NULL DEFAULT FALSE,
  ADD COLUMN IF NOT EXISTS m23_curation_allow_internal_llm BOOLEAN NOT NULL DEFAULT FALSE,
  ADD COLUMN IF NOT EXISTS m23_curation_require_4_eyes BOOLEAN NOT NULL DEFAULT FALSE,
  ADD COLUMN IF NOT EXISTS m23_drift_detection_enabled BOOLEAN NOT NULL DEFAULT FALSE,
  ADD COLUMN IF NOT EXISTS m23_drift_auto_trigger_proposal BOOLEAN NOT NULL DEFAULT FALSE,
  ADD COLUMN IF NOT EXISTS m23_drift_kl_threshold DOUBLE PRECISION DEFAULT 0.15,
  ADD COLUMN IF NOT EXISTS m23_drift_conf_drop_threshold DOUBLE PRECISION DEFAULT 0.10,
  ADD COLUMN IF NOT EXISTS m23_drift_support_threshold_pct DOUBLE PRECISION DEFAULT 0.40,
  ADD COLUMN IF NOT EXISTS m23_canary_traffic_percent DOUBLE PRECISION DEFAULT 0.05,
  ADD COLUMN IF NOT EXISTS m23_canary_duration_hours INT DEFAULT 24,
  ADD COLUMN IF NOT EXISTS m23_canary_min_samples INT DEFAULT 1000,
  ADD COLUMN IF NOT EXISTS m23_proposal_retention_days INT DEFAULT 90,
  ADD COLUMN IF NOT EXISTS m23_curation_budget_cents_per_day BIGINT DEFAULT 0;

-- ─────────────────────────────────────────────────────────────────────
-- 10. Row-level security (defense in depth)
-- ─────────────────────────────────────────────────────────────────────

ALTER TABLE taxonomy_version ENABLE ROW LEVEL SECURITY;
ALTER TABLE taxonomy_change_proposal ENABLE ROW LEVEL SECURITY;
ALTER TABLE curation_review ENABLE ROW LEVEL SECURITY;
ALTER TABLE taxonomy_drift_event ENABLE ROW LEVEL SECURITY;
ALTER TABLE taxonomy_change_audit ENABLE ROW LEVEL SECURITY;
ALTER TABLE gold_dataset ENABLE ROW LEVEL SECURITY;
ALTER TABLE gold_example ENABLE ROW LEVEL SECURITY;
ALTER TABLE bundle_audit_scorecard ENABLE ROW LEVEL SECURITY;

-- Tenant isolation policies (set per session via app.current_tenant_id)
CREATE POLICY tenant_isolation_taxonomy_version
  ON taxonomy_version
  USING (tenant_id = current_setting('app.current_tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

CREATE POLICY tenant_isolation_taxonomy_change_proposal
  ON taxonomy_change_proposal
  USING (tenant_id = current_setting('app.current_tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

CREATE POLICY tenant_isolation_curation_review
  ON curation_review
  USING (
    EXISTS (
      SELECT 1 FROM taxonomy_change_proposal p
      WHERE p.id = curation_review.proposal_id
        AND p.tenant_id = current_setting('app.current_tenant_id', true)
    )
  );

CREATE POLICY tenant_isolation_taxonomy_drift_event
  ON taxonomy_drift_event
  USING (tenant_id = current_setting('app.current_tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

CREATE POLICY tenant_isolation_taxonomy_change_audit
  ON taxonomy_change_audit
  USING (tenant_id = current_setting('app.current_tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

CREATE POLICY tenant_isolation_gold_dataset
  ON gold_dataset
  USING (tenant_id = current_setting('app.current_tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

CREATE POLICY tenant_isolation_gold_example
  ON gold_example
  USING (
    EXISTS (
      SELECT 1 FROM gold_dataset gd
      WHERE gd.id = gold_example.gold_dataset_id
        AND gd.tenant_id = current_setting('app.current_tenant_id', true)
    )
  );

CREATE POLICY tenant_isolation_bundle_audit_scorecard
  ON bundle_audit_scorecard
  USING (tenant_id = current_setting('app.current_tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

-- ─────────────────────────────────────────────────────────────────────
-- 11. Triggers — taxonomy_version immutability + updated_at
-- ─────────────────────────────────────────────────────────────────────

-- Prevent UPDATE on anchors_json once status is past CANDIDATE.
CREATE OR REPLACE FUNCTION fn_taxonomy_version_immutable_after_candidate()
RETURNS TRIGGER AS $$
BEGIN
  IF OLD.status IN ('ACTIVE', 'DEPRECATED', 'ROLLED_BACK') THEN
    IF OLD.anchors_json IS DISTINCT FROM NEW.anchors_json
       OR OLD.semver IS DISTINCT FROM NEW.semver
       OR OLD.parent_id IS DISTINCT FROM NEW.parent_id THEN
      RAISE EXCEPTION 'taxonomy_version is immutable after % status (id=%)',
                      OLD.status, OLD.id;
    END IF;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_taxonomy_version_immutable
  BEFORE UPDATE ON taxonomy_version
  FOR EACH ROW
  EXECUTE FUNCTION fn_taxonomy_version_immutable_after_candidate();

-- updated_at trigger on proposal
CREATE OR REPLACE FUNCTION fn_set_updated_at()
RETURNS TRIGGER AS $$
BEGIN
  NEW.updated_at = NOW();
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_proposal_updated_at
  BEFORE UPDATE ON taxonomy_change_proposal
  FOR EACH ROW
  EXECUTE FUNCTION fn_set_updated_at();

-- ─────────────────────────────────────────────────────────────────────
-- 12. Helper view : pending review queue
-- ─────────────────────────────────────────────────────────────────────

CREATE VIEW v_pending_review_queue AS
SELECT
  p.id AS proposal_id,
  p.tenant_id,
  p.catalog_id,
  p.kind,
  p.status,
  p.proposed_by,
  p.proposed_via,
  p.created_at,
  p.expires_at,
  p.actual_cost_cents,
  EXTRACT(EPOCH FROM (NOW() - p.created_at)) AS pending_seconds,
  (
    SELECT COUNT(*) FROM curation_review cr
    WHERE cr.proposal_id = p.id
  ) AS review_count
FROM taxonomy_change_proposal p
WHERE p.status IN ('PENDING', 'APPROVED');

-- ─────────────────────────────────────────────────────────────────────
-- 13. Schema version tracking
-- ─────────────────────────────────────────────────────────────────────

INSERT INTO schema_migration (id, applied_at, description)
VALUES (
  '0014_taxonomy_curation',
  NOW(),
  'M23 — LLM-assisted taxonomy curation : 7 tables + 13 tenant_feature columns + RLS + immutability triggers'
)
ON CONFLICT (id) DO NOTHING;

COMMIT;
