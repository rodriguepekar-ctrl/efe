-- ===========================================================================
-- M32 — Vector Search + Hybrid Retrieval.
--
-- Adds :
--   1. embedding_model          — global catalog of supported embedders.
--   2. vector_collection        — per-tenant named collections.
--   3. vector_index_segment     — HNSW shard descriptors.
--   4. vector_doc               — per-document mirror (PG source of truth).
--   5. retrieval_query_log      — analytics + audit of search calls.
--   6. vector_snapshot          — S3 snapshot history.
--   + tenant_feature.vector_search_enabled column.
--   + 3 audit_event enum values.
-- ===========================================================================

BEGIN;

-- ── 1. embedding_model ─────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS embedding_model (
    name                        VARCHAR(64) PRIMARY KEY,
    dim                         INTEGER NOT NULL CHECK (dim BETWEEN 1 AND 4096),
    normalized                  BOOLEAN NOT NULL DEFAULT TRUE,
    source                      VARCHAR(32) NOT NULL
                                CHECK (source IN ('internal','m30_vision','m30_audio','external','sparse_bm25')),
    description                 TEXT,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
COMMENT ON TABLE embedding_model IS
    'M32 — Global catalogue of supported embedding models. Validated at collection-create time.';

-- ── 2. vector_collection ───────────────────────────────────────────
CREATE TABLE IF NOT EXISTS vector_collection (
    id                          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id                   TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    name                        VARCHAR(32) NOT NULL
                                CHECK (name ~ '^[a-z0-9_-]{1,32}$'),
    embedding_model_name        VARCHAR(64) NOT NULL REFERENCES embedding_model(name),
    dim                         INTEGER NOT NULL CHECK (dim BETWEEN 1 AND 4096),
    metric                      VARCHAR(8) NOT NULL CHECK (metric IN ('cosine','dot','l2')),
    hnsw_m                      INTEGER NOT NULL DEFAULT 16 CHECK (hnsw_m BETWEEN 4 AND 64),
    hnsw_ef_construction        INTEGER NOT NULL DEFAULT 200 CHECK (hnsw_ef_construction BETWEEN 16 AND 1000),
    hnsw_ef_search              INTEGER NOT NULL DEFAULT 64 CHECK (hnsw_ef_search BETWEEN 16 AND 1000),
    force_hnsw                  BOOLEAN NOT NULL DEFAULT FALSE,
    max_docs                    BIGINT NOT NULL DEFAULT 10000000
                                CHECK (max_docs BETWEEN 1 AND 100000000),
    status                      VARCHAR(16) NOT NULL DEFAULT 'active'
                                CHECK (status IN ('active','building','snapshotting','dropping','dropped')),
    last_snapshot_at            TIMESTAMPTZ,
    last_snapshot_url           TEXT,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (tenant_id, name)
);
COMMENT ON TABLE vector_collection IS
    'M32 — Per-tenant named collection. (tenant_id, name) is unique.';

CREATE INDEX IF NOT EXISTS idx_vector_collection_tenant
    ON vector_collection (tenant_id, status);

-- ── 3. vector_index_segment ───────────────────────────────────────
CREATE TABLE IF NOT EXISTS vector_index_segment (
    id                          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    collection_id               UUID NOT NULL REFERENCES vector_collection(id) ON DELETE CASCADE,
    segment_index               INTEGER NOT NULL,
    cardinality                 BIGINT NOT NULL DEFAULT 0,
    backend                     VARCHAR(8) NOT NULL CHECK (backend IN ('flat','hnsw')),
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (collection_id, segment_index)
);
COMMENT ON TABLE vector_index_segment IS
    'M32 — HNSW shard descriptors. M32 uses 1 segment per collection.';

-- ── 4. vector_doc ─────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS vector_doc (
    id                          TEXT NOT NULL,
    collection_id               UUID NOT NULL REFERENCES vector_collection(id) ON DELETE CASCADE,
    tenant_id                   TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    vector                      BYTEA NOT NULL,           -- packed f32, little-endian
    dim                         INTEGER NOT NULL,
    text                        TEXT,                     -- for BM25 sparse index
    payload                     JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (collection_id, id)
);
COMMENT ON TABLE vector_doc IS
    'M32 — Per-document mirror (PG source of truth). Vector stored as BYTEA, dim x f32 little-endian.';

CREATE INDEX IF NOT EXISTS idx_vector_doc_tenant
    ON vector_doc (tenant_id);
CREATE INDEX IF NOT EXISTS idx_vector_doc_collection
    ON vector_doc (collection_id, created_at DESC);

-- ── 5. retrieval_query_log ────────────────────────────────────────
CREATE TABLE IF NOT EXISTS retrieval_query_log (
    id                          BIGSERIAL PRIMARY KEY,
    collection_id               UUID NOT NULL REFERENCES vector_collection(id) ON DELETE CASCADE,
    tenant_id                   TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    mode                        VARCHAR(16) NOT NULL
                                CHECK (mode IN ('dense','sparse','hybrid_rrf','hybrid_weighted')),
    top_k                       INTEGER NOT NULL CHECK (top_k > 0),
    -- We never log the raw query vector or text — only a fingerprint
    -- (SHA-256 prefix) so we can correlate without storing PII.
    query_fingerprint           CHAR(16) NOT NULL,
    candidates_per_side         INTEGER,
    rrf_k                       INTEGER,
    alpha                       REAL,
    duration_ms                 INTEGER NOT NULL,
    hits_returned               INTEGER NOT NULL,
    issued_at                   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
COMMENT ON TABLE retrieval_query_log IS
    'M32 — Append-only query log. Fingerprint only, no raw text/vector.';

CREATE INDEX IF NOT EXISTS idx_retrieval_query_log_time
    ON retrieval_query_log (tenant_id, issued_at DESC);

-- ── 6. vector_snapshot ────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS vector_snapshot (
    id                          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    collection_id               UUID NOT NULL REFERENCES vector_collection(id) ON DELETE CASCADE,
    started_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at                TIMESTAMPTZ,
    bytes_written               BIGINT,
    s3_url                      TEXT,
    sha256                      CHAR(64),
    status                      VARCHAR(16) NOT NULL DEFAULT 'in_progress'
                                CHECK (status IN ('in_progress','succeeded','failed'))
);
COMMENT ON TABLE vector_snapshot IS
    'M32 — Snapshot history (one row per snapshot attempt).';

CREATE INDEX IF NOT EXISTS idx_vector_snapshot_coll_time
    ON vector_snapshot (collection_id, started_at DESC);

-- ── 7. tenant_feature additions ───────────────────────────────────
ALTER TABLE tenant_feature
    ADD COLUMN IF NOT EXISTS vector_search_enabled BOOLEAN NOT NULL DEFAULT FALSE;

COMMENT ON COLUMN tenant_feature.vector_search_enabled IS
    'M32 — Master switch for the vector search plane.';

-- ── 8. agent_audit enum extensions ────────────────────────────────
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'VectorCollectionCreated') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'VectorCollectionCreated';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'VectorCollectionDropped') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'VectorCollectionDropped';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'VectorSnapshotTaken') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'VectorSnapshotTaken';
    END IF;
END$$;

-- ── 9. seed the default embedding-model catalog ───────────────────
INSERT INTO embedding_model (name, dim, normalized, source, description) VALUES
    ('efe-classifier-768', 768, TRUE,  'internal',   'M21 classifier head'),
    ('siglip-base-768',    768, TRUE,  'm30_vision', 'M30 SigLIP-base-patch16-224 visual encoder'),
    ('whisper-tiny-384',   384, TRUE,  'm30_audio',  'M30 Whisper-tiny pooled-encoder embeddings'),
    ('external-1536',      1536,TRUE,  'external',   'Customer-provided 1536-dim (OpenAI/Cohere)'),
    ('bm25-sparse',        1,   FALSE, 'sparse_bm25','Sparse BM25 inverted index (dim is sentinel)')
ON CONFLICT (name) DO NOTHING;

-- ── 10. View : per-tenant collection summary ──────────────────────
CREATE OR REPLACE VIEW v_vector_collection_summary AS
SELECT vc.tenant_id,
       vc.name,
       vc.embedding_model_name,
       vc.dim,
       vc.metric,
       vc.status,
       COUNT(vd.id)::BIGINT AS cardinality,
       vc.last_snapshot_at
  FROM vector_collection vc
  LEFT JOIN vector_doc vd ON vd.collection_id = vc.id
 GROUP BY vc.id;

COMMENT ON VIEW v_vector_collection_summary IS
    'M32 — Surface for `GET /v1/vector/collections`.';

COMMIT;
