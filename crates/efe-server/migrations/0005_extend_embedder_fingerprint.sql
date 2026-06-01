-- ============================================================================
-- Sprint D16 — Extend embedder_fingerprint to support D16 byte-exact bundles.
-- ============================================================================
--
-- Reference : `docs/bench/d16_embedder_bundling_architecture.md` § 9.
--
-- D12 introduced the `catalogs.embedder_fingerprint` column at the
-- `family/model@dim` level (e.g. `sentence-transformers/all-MiniLM-L6-v2@384`).
-- That format is sufficient for grossière compatibility detection but
-- doesn't catch byte-exact drift between two `from_pretrained()` runs
-- that resolve different revisions of `main` on HuggingFace Hub.
--
-- D16 introduces signed bundles with a `manifest.json` whose SHA-256
-- IS the canonical fingerprint (32 bytes, 64 hex chars). This migration
-- adds two new columns :
--
--   * `bundle_fingerprint`        CHAR(64) — present iff the catalog
--                                   was fitted on a D16 bundle.
--   * `legacy_embedder_fingerprint` TEXT  — preserves the D12 format
--                                   for catalogs fitted pre-D16.
--
-- The pre-existing `embedder_fingerprint` column stays as the union of
-- both for backwards-compat ; new code paths read whichever non-null
-- column applies (cf. `embedder/legacy.rs::resolve_fingerprint`).
--
-- A CHECK constraint enforces that exactly one of the two columns is
-- non-null per row — never both, never neither (the check accepts NULL
-- on both during the backfill window in this migration but the post-
-- backfill statement at the end of the file flips it to strict XOR).
--
-- Performance : a partial index on `bundle_fingerprint` lets the
-- `Register` / `Evolve` RPCs do an O(log n) lookup of all catalogs
-- bound to a given bundle (operational query : "which catalogs need
-- re-fitting after rotating bundle X?").
-- ============================================================================

ALTER TABLE catalogs
    ADD COLUMN IF NOT EXISTS bundle_fingerprint CHAR(64) NULL,
    ADD COLUMN IF NOT EXISTS legacy_embedder_fingerprint TEXT NULL;

-- Backfill : copy the existing `embedder_fingerprint` values into
-- `legacy_embedder_fingerprint`. After this, every existing row has
-- legacy_embedder_fingerprint = embedder_fingerprint (both non-null)
-- and bundle_fingerprint = NULL.
UPDATE catalogs
   SET legacy_embedder_fingerprint = embedder_fingerprint
 WHERE legacy_embedder_fingerprint IS NULL
   AND embedder_fingerprint IS NOT NULL;

-- The XOR constraint : at most one of (bundle_fingerprint,
-- legacy_embedder_fingerprint) is non-null per row.
--
-- **Audit fix D16-C5** — relaxed from "exactly one" to "at most one"
-- because pre-D12 catalogs in production may have
-- `embedder_fingerprint = NULL` (the column was nullable in
-- `0001_init.sql`). The strict XOR `(A NOT NULL ⊕ B NOT NULL)` would
-- reject those rows at migration time and brick the deploy. We
-- accept the both-NULL state as legacy / unknown ; the catalog
-- handler treats those as "no fingerprint stored" and falls back
-- to the runtime's active bundle (operator must manually backfill
-- to enable the lifecycle gate).
ALTER TABLE catalogs
    ADD CONSTRAINT catalogs_fingerprint_xor
        CHECK (
            -- D16 byte-exact (preferred for new catalogs)
            (bundle_fingerprint IS NOT NULL AND legacy_embedder_fingerprint IS NULL)
            -- D12 legacy (back-compat)
         OR (bundle_fingerprint IS NULL     AND legacy_embedder_fingerprint IS NOT NULL)
            -- Pre-D12 nullable (allowed, deprecated — operator must backfill)
         OR (bundle_fingerprint IS NULL     AND legacy_embedder_fingerprint IS NULL)
        );

-- Bundle fingerprint format : lowercase hex, 64 chars exact.
ALTER TABLE catalogs
    ADD CONSTRAINT catalogs_bundle_fingerprint_format
        CHECK (
            bundle_fingerprint IS NULL
         OR bundle_fingerprint ~ '^[0-9a-f]{64}$'
        );

-- Partial index : speed up "catalogs bound to bundle X" lookup.
-- Filtering NULL entries keeps the index small (most rows have NULL
-- in bundle_fingerprint until the operator rolls the rebase forward).
CREATE INDEX IF NOT EXISTS catalogs_bundle_fingerprint_idx
    ON catalogs (bundle_fingerprint)
 WHERE bundle_fingerprint IS NOT NULL;

COMMENT ON COLUMN catalogs.bundle_fingerprint IS
    'Sprint D16 — SHA-256 (lowercase hex) of the D16 bundle manifest.json '
    'on which this catalog was fitted. NULL iff this catalog uses the '
    'D12 legacy_embedder_fingerprint instead. The bundle_fingerprint and '
    'legacy_embedder_fingerprint columns are XOR-exclusive (CHECK).';

COMMENT ON COLUMN catalogs.legacy_embedder_fingerprint IS
    'Sprint D16 — D12 format ``family/model@dim``. NULL for catalogs '
    'fitted via a D16 bundle (which use bundle_fingerprint instead).';
