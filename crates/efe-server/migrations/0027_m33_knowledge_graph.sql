-- ===========================================================================
-- M33 — Knowledge Graph + GraphRAG.
--
-- Adds :
--   1. kg_graph                — per-tenant root (one row per tenant).
--   2. kg_entity               — entities with kind + canonical name.
--   3. kg_entity_mention       — provenance link (entity → doc + span).
--   4. kg_relation             — directed edges.
--   5. kg_claim                — raw extraction claims (pre-promotion).
--   6. kg_predicate_catalog    — tenant-custom predicate registry.
--   7. kg_query_log            — analytics + audit.
--   + tenant_feature.kg_enabled column.
--   + 3 audit_event enum values.
-- ===========================================================================

BEGIN;

-- ── 1. kg_graph ───────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS kg_graph (
    tenant_id                   TEXT PRIMARY KEY REFERENCES tenant_feature(id) ON DELETE CASCADE,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_snapshot_at            TIMESTAMPTZ,
    last_snapshot_url           TEXT,
    last_snapshot_bytes         BIGINT
);
COMMENT ON TABLE kg_graph IS
    'M33 — Per-tenant graph root. 1:1 with tenant_feature.';

-- ── 2. kg_entity ──────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS kg_entity (
    id                          TEXT PRIMARY KEY,
    tenant_id                   TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    kind                        VARCHAR(16) NOT NULL
                                CHECK (kind IN ('person','org','location','product','event','misc')),
    canonical_name              VARCHAR(200) NOT NULL,
    confidence                  REAL NOT NULL DEFAULT 1.0
                                CHECK (confidence >= 0.0 AND confidence <= 1.0),
    embedding                   BYTEA,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
COMMENT ON TABLE kg_entity IS
    'M33 — Nodes in the per-tenant graph.';

CREATE INDEX IF NOT EXISTS idx_kg_entity_tenant_kind
    ON kg_entity (tenant_id, kind);
CREATE INDEX IF NOT EXISTS idx_kg_entity_tenant_name
    ON kg_entity (tenant_id, canonical_name);

-- ── 3. kg_entity_mention ──────────────────────────────────────────
CREATE TABLE IF NOT EXISTS kg_entity_mention (
    id                          BIGSERIAL PRIMARY KEY,
    entity_id                   TEXT NOT NULL REFERENCES kg_entity(id) ON DELETE CASCADE,
    tenant_id                   TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    doc_id                      TEXT NOT NULL,
    span_start                  INTEGER NOT NULL CHECK (span_start >= 0),
    span_end                    INTEGER NOT NULL CHECK (span_end > span_start),
    surface_form                TEXT NOT NULL,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
COMMENT ON TABLE kg_entity_mention IS
    'M33 — Provenance : which (doc, span) produced this entity.';

CREATE INDEX IF NOT EXISTS idx_kg_entity_mention_entity
    ON kg_entity_mention (entity_id);
CREATE INDEX IF NOT EXISTS idx_kg_entity_mention_doc
    ON kg_entity_mention (tenant_id, doc_id);

-- ── 4. kg_relation ────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS kg_relation (
    id                          TEXT PRIMARY KEY,
    tenant_id                   TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    subject_id                  TEXT NOT NULL REFERENCES kg_entity(id) ON DELETE CASCADE,
    predicate                   VARCHAR(32) NOT NULL
                                CHECK (predicate ~ '^[a-z0-9_]{1,32}$'),
    object_id                   TEXT NOT NULL REFERENCES kg_entity(id) ON DELETE CASCADE,
    confidence                  REAL NOT NULL DEFAULT 1.0
                                CHECK (confidence >= 0.0 AND confidence <= 1.0),
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
COMMENT ON TABLE kg_relation IS
    'M33 — Directed edges promoted from claims.';

CREATE INDEX IF NOT EXISTS idx_kg_relation_subject
    ON kg_relation (tenant_id, subject_id, predicate);
CREATE INDEX IF NOT EXISTS idx_kg_relation_object
    ON kg_relation (tenant_id, object_id, predicate);
CREATE INDEX IF NOT EXISTS idx_kg_relation_predicate
    ON kg_relation (tenant_id, predicate);

-- ── 5. kg_claim ───────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS kg_claim (
    id                          TEXT PRIMARY KEY,
    tenant_id                   TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    subject_id                  TEXT NOT NULL,
    predicate                   VARCHAR(32) NOT NULL
                                CHECK (predicate ~ '^[a-z0-9_]{1,32}$'),
    object_id                   TEXT NOT NULL,
    confidence                  REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
    provenance_doc_id           TEXT NOT NULL,
    provenance_span_start       INTEGER NOT NULL CHECK (provenance_span_start >= 0),
    provenance_span_end         INTEGER NOT NULL CHECK (provenance_span_end > provenance_span_start),
    promoted_to_relation_id     TEXT REFERENCES kg_relation(id) ON DELETE SET NULL,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
COMMENT ON TABLE kg_claim IS
    'M33 — Raw extraction claims. Promoted to kg_relation when supported by ≥ 2 claims or conf ≥ 0.9.';

CREATE INDEX IF NOT EXISTS idx_kg_claim_triple
    ON kg_claim (tenant_id, subject_id, predicate, object_id);
CREATE INDEX IF NOT EXISTS idx_kg_claim_promotion
    ON kg_claim (promoted_to_relation_id) WHERE promoted_to_relation_id IS NOT NULL;

-- ── 6. kg_predicate_catalog ───────────────────────────────────────
CREATE TABLE IF NOT EXISTS kg_predicate_catalog (
    tenant_id                   TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    predicate                   VARCHAR(32) NOT NULL
                                CHECK (predicate ~ '^[a-z0-9_]{1,32}$'),
    domain                      VARCHAR(16),
    range_                      VARCHAR(16),                 -- `range` is reserved in PG
    description                 TEXT,
    is_base                     BOOLEAN NOT NULL DEFAULT FALSE,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, predicate)
);
COMMENT ON TABLE kg_predicate_catalog IS
    'M33 — Per-tenant catalogue of allowed predicates (base + custom).';

-- ── 7. kg_query_log ───────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS kg_query_log (
    id                          BIGSERIAL PRIMARY KEY,
    tenant_id                   TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    op                          VARCHAR(24) NOT NULL
                                CHECK (op IN ('extract','traverse','graphrag_search','snapshot')),
    -- Fingerprint of the input (no raw text/vector ever logged).
    input_fingerprint           CHAR(16) NOT NULL,
    duration_ms                 INTEGER NOT NULL,
    result_count                INTEGER NOT NULL DEFAULT 0,
    issued_at                   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
COMMENT ON TABLE kg_query_log IS
    'M33 — Append-only KG-op log. Fingerprint only, no raw text.';

CREATE INDEX IF NOT EXISTS idx_kg_query_log_time
    ON kg_query_log (tenant_id, issued_at DESC);

-- ── 8. tenant_feature additions ───────────────────────────────────
ALTER TABLE tenant_feature
    ADD COLUMN IF NOT EXISTS kg_enabled BOOLEAN NOT NULL DEFAULT FALSE;

COMMENT ON COLUMN tenant_feature.kg_enabled IS
    'M33 — Master switch for the knowledge-graph plane.';

-- ── 9. agent_audit enum extensions ────────────────────────────────
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'KgGraphCreated') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'KgGraphCreated';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'KgGraphDropped') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'KgGraphDropped';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'KgEntityMerged') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'KgEntityMerged';
    END IF;
END$$;

-- ── 10. seed base predicates for the internal tenant ─────────────
-- Real tenants get their catalogue populated on first kg_enabled=TRUE flip.
-- This seed is for local dev only.

-- ── 11. View : per-tenant graph summary ──────────────────────────
CREATE OR REPLACE VIEW v_kg_graph_summary AS
SELECT g.tenant_id,
       g.last_snapshot_at,
       COALESCE(e.cnt, 0) AS entity_count,
       COALESCE(r.cnt, 0) AS relation_count
  FROM kg_graph g
  LEFT JOIN (
      SELECT tenant_id, COUNT(*)::BIGINT AS cnt FROM kg_entity GROUP BY tenant_id
  ) e ON e.tenant_id = g.tenant_id
  LEFT JOIN (
      SELECT tenant_id, COUNT(*)::BIGINT AS cnt FROM kg_relation GROUP BY tenant_id
  ) r ON r.tenant_id = g.tenant_id;

COMMENT ON VIEW v_kg_graph_summary IS
    'M33 — Surface for GET /v1/kg/graph/status.';

COMMIT;
