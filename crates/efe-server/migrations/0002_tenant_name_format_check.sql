-- ============================================================================
-- Sprint D14 — audit fix C1 : tenant + name format CHECK constraints.
-- ============================================================================
--
-- Audit C1 finding : the `catalogs_parent_chain` CHECK enforces
-- `parent_key ~ '^[a-z0-9_-]+/[a-z0-9_-]+@v\d+$'` (lowercase ASCII)
-- but neither `tenant` nor `name` columns had any character-class
-- check. Symptom : an operator could `register tenant="ACME" name="billing"`
-- (parent_key='' → CHECK passes), then `evolve to v2` failed with an
-- opaque `check_violation` because the synthesised
-- `parent_key="ACME/billing@v1"` violated the regex.
--
-- This migration adds CHECK constraints that mirror the parent_key regex
-- on `tenant` and `name` themselves, so the inconsistency cannot arise.
-- The constraint is **defense in depth** — the registry-level
-- `CatalogPolicy::validate_identifier` (in `policy.rs`) is the first
-- line of defence and produces user-friendly `InvalidArgument` errors ;
-- the SQL CHECK guards against any path that would bypass the trait
-- (e.g. a future direct-SQL admin tool).
--
-- This migration is **idempotent-safe** because PostgreSQL refuses to
-- ADD CONSTRAINT with a duplicate name ; a re-run on an already-migrated
-- DB is a no-op via `_sqlx_migrations` tracking.
-- ============================================================================

ALTER TABLE catalogs
    ADD CONSTRAINT catalogs_tenant_format
        CHECK (tenant ~ '^[a-z0-9_-]{1,63}$');

ALTER TABLE catalogs
    ADD CONSTRAINT catalogs_name_format
        CHECK (name ~ '^[a-z0-9_-]{1,63}$');

COMMENT ON CONSTRAINT catalogs_tenant_format ON catalogs IS
    'D14 audit fix C1 — tenant must match the canonical regex '
    '`^[a-z0-9_-]{1,63}$`. Mirrors `CatalogPolicy::validate_identifier`.';

COMMENT ON CONSTRAINT catalogs_name_format ON catalogs IS
    'D14 audit fix C1 — name must match the canonical regex '
    '`^[a-z0-9_-]{1,63}$`. Mirrors `CatalogPolicy::validate_identifier`.';
