-- ===========================================================================
-- M33 — Rollback for 0027_m33_knowledge_graph.sql.
--
-- Does NOT drop the enum values added to agent_audit_event
-- (PG <13 cannot drop enum values cleanly) — they remain additive.
-- ===========================================================================

BEGIN;

DROP VIEW IF EXISTS v_kg_graph_summary;

ALTER TABLE tenant_feature
    DROP COLUMN IF EXISTS kg_enabled;

DROP TABLE IF EXISTS kg_query_log;
DROP TABLE IF EXISTS kg_predicate_catalog;
DROP TABLE IF EXISTS kg_claim;
DROP TABLE IF EXISTS kg_relation;
DROP TABLE IF EXISTS kg_entity_mention;
DROP TABLE IF EXISTS kg_entity;
DROP TABLE IF EXISTS kg_graph;

COMMIT;
