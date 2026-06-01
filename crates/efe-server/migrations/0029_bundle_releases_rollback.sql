-- Sprint **Φ-C D27 (Bundle embedder réel)** rollback.

BEGIN;

DROP TRIGGER IF EXISTS trg_bundle_releases_no_delete ON bundle_releases;
DROP FUNCTION IF EXISTS bundle_releases_no_delete();

DROP TABLE IF EXISTS bundle_release_attestations CASCADE;
DROP TABLE IF EXISTS bundle_releases CASCADE;

COMMIT;
