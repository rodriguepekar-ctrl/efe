-- ============================================================================
-- Sprint D14 — Postgres-backed catalog registry, initial schema.
-- ============================================================================
--
-- Authoritative schema for the `efe.v1.catalog.CatalogService` six RPCs.
-- Kept identical to the design document
-- `docs/bench/d14_postgres_catalog_backend_architecture.md` § 6.1.
--
-- Forward-only — no `down.sql` per project convention. If a column needs
-- to be retired, write a new migration `00NN_drop_<column>.sql`.
--
-- Idempotent : `CREATE TABLE IF NOT EXISTS` is supported by Postgres ≥ 9.5
-- but the table primary keys + check constraints are NOT idempotent at
-- creation time. The `_sqlx_migrations` lock guarantees the migration runs
-- exactly once per deployment, so this trade-off is acceptable.
-- ============================================================================

-- ----------------------------------------------------------------------------
-- Table : catalogs
-- ----------------------------------------------------------------------------
-- One row per `(tenant, name, version)`. Append-only modulo explicit deletes
-- guarded by the `force` flag at the registry level.
--
-- Composite PK (tenant, name, version) :
--   * guarantees no two catalogs share the same canonical key,
--   * is dense for efficient (tenant, name)-prefix scans (list_versions),
--   * lets the (tenant) prefix scan support list_catalogs.
CREATE TABLE catalogs (
    tenant             TEXT      NOT NULL,
    name               TEXT      NOT NULL,
    version            INTEGER   NOT NULL,

    -- Content : full CatalogContent serialised as JSONB. JSONB picked over
    -- JSON for indexability and decompression cost ; over TEXT because
    -- the content has nested arrays we want the option to query (D15+).
    content_json       JSONB     NOT NULL,

    -- Metadata fields. Stored as separate columns rather than embedded
    -- in `content_json` so we can index / filter without a JSONB
    -- expression index.
    created_at_unix_ms BIGINT    NOT NULL,
    created_by         TEXT      NOT NULL,
    description        TEXT      NOT NULL DEFAULT '',
    parent_key         TEXT      NOT NULL DEFAULT '',
    evolution_op       TEXT      NOT NULL,
    labels             JSONB     NOT NULL DEFAULT '{}'::jsonb,
    content_sha256     TEXT      NOT NULL,

    -- Compiled anchors (NULL until embedding pool produces them).
    compiled_anchors   BYTEA,
    compiled_classes   JSONB,
    embedder_fingerprint TEXT,

    -- Audit trail (DB side ; orthogonal to created_at_unix_ms which is
    -- the *application's* notion of "registered at").
    db_inserted_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    db_updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (tenant, name, version),

    -- The chain integrity is enforced by parent_key matching the canonical
    -- key regex when non-empty. An audit trail forward-only table is
    -- left for D15.
    CONSTRAINT catalogs_parent_chain
        CHECK (
            parent_key = ''
            OR parent_key ~ '^[a-z0-9_-]+/[a-z0-9_-]+@v\d+$'
        ),
    CONSTRAINT catalogs_version_positive CHECK (version >= 1),
    CONSTRAINT catalogs_tenant_nonempty CHECK (length(tenant) >= 1),
    CONSTRAINT catalogs_name_nonempty CHECK (length(name) >= 1)
);

COMMENT ON TABLE catalogs IS
    'Sprint D14 — versioned catalogs. Composite PK (tenant, name, version) ; '
    'append-only modulo `delete` at the trait level. Parent chain integrity '
    'guarded by check constraint (regex on parent_key).';

-- ----------------------------------------------------------------------------
-- Indexes
-- ----------------------------------------------------------------------------

-- Reverse-version index for `latest_version` lookups in list_catalogs.
-- Also accelerates the SELECT MAX(version) inside the advisory-locked
-- register / evolve transactions.
CREATE INDEX catalogs_tenant_name_version_desc_idx
    ON catalogs (tenant, name, version DESC);

-- Reference check : "is this version pointed to by a successor ?".
-- Partial index to skip the (tenant, name@v1) rows where parent_key is
-- empty — those are the bulk of inserts in healthy chains.
CREATE INDEX catalogs_parent_key_idx
    ON catalogs (parent_key)
    WHERE parent_key <> '';

-- ----------------------------------------------------------------------------
-- Trigger : keep `db_updated_at` fresh on UPDATE.
-- ----------------------------------------------------------------------------
-- Catalogs are rarely UPDATEd at the trait level (registrations are
-- append-only, deletes are physical removal). The trigger gives us a
-- "last touched" timestamp for ops debugging when bulk operations
-- (e.g. cascade deletes) modify rows in flight.
CREATE OR REPLACE FUNCTION catalogs_set_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.db_updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER catalogs_updated_at_trg
    BEFORE UPDATE ON catalogs
    FOR EACH ROW
    EXECUTE FUNCTION catalogs_set_updated_at();
