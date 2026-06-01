-- ===========================================================================
-- M32 — Rollback for 0026_m32_vector_search.sql.
--
-- Does NOT drop the enum values added to `agent_audit_event` (PG <13
-- can't drop enum values cleanly) — they remain additive.
-- ===========================================================================

BEGIN;

DROP VIEW IF EXISTS v_vector_collection_summary;

ALTER TABLE tenant_feature
    DROP COLUMN IF EXISTS vector_search_enabled;

DROP TABLE IF EXISTS vector_snapshot;
DROP TABLE IF EXISTS retrieval_query_log;
DROP TABLE IF EXISTS vector_doc;
DROP TABLE IF EXISTS vector_index_segment;
DROP TABLE IF EXISTS vector_collection;
DROP TABLE IF EXISTS embedding_model;

COMMIT;
