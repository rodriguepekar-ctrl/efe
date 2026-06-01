-- Sprint **Φ-C D27 (Bundle embedder réel)** — `bundle_releases`
-- ledger (= source de vérité légale des bundles releasés).
--
-- Cf. `docs/EFE_BUNDLE_EMBEDDER_REAL_ARCHITECTURE.md` § 9.4 + Annexe E.
--
-- Cette table est l'append-only audit log de **chaque release de
-- bundle** (initial / rotation / revocation / emergency). C'est la
-- source que `efe-attest bundle verify` consulte pour valider
-- qu'un bundle déployé correspond à une release légitime, qu'elle
-- n'a pas été révoquée, et qu'elle est traçable dans le Sigstore
-- Rekor transparency log.
--
-- Compliance : satisfait SOC 2 CC8.1 (change management) +
-- partiellement EU AI Act Art. 53 (documentation technique).
--
-- Idempotent : safe to re-apply. Pair rollback in
-- `0030_bundle_releases_rollback.sql`.

BEGIN;

-- ─────────────────────────────────────────────────────────────────
-- bundle_releases
-- ─────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS bundle_releases (
    release_id          UUID         PRIMARY KEY DEFAULT gen_random_uuid(),

    -- Identité du bundle
    bundle_id           TEXT         NOT NULL,
    version             TEXT         NOT NULL,

    -- Type de release ; gating workflow différent par kind.
    release_kind        VARCHAR(16)  NOT NULL
                        CHECK (release_kind IN (
                            'initial',     -- première release de cette version
                            'rotation',    -- rotation 6 mois cadence
                            'revocation',  -- révocation d'urgence
                            'emergency'    -- patch security non-cadence
                        )),

    -- Hashes (= source d'opposabilité)
    manifest_sha256     BYTEA        NOT NULL,
    tar_sha256          BYTEA        NOT NULL,

    -- Timestamps
    released_at         TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    revoked_at          TIMESTAMPTZ,
    revoked_reason      TEXT,

    -- Auteur + reviewers
    released_by         TEXT         NOT NULL,
    approved_by         TEXT,         -- requis pour `emergency` + `revocation`

    -- Sigstore Rekor entry (= transparency log)
    rekor_log_index     BIGINT       NOT NULL,
    rekor_url           TEXT         NOT NULL,

    -- Quality metrics (= snapshot du build benchmark)
    recall_at_1         NUMERIC(5,4),
    recall_at_1_fr      NUMERIC(5,4),
    recall_at_1_en      NUMERIC(5,4),
    bench_p99_t4_fp16_seq128_ms  NUMERIC(8,2),
    bench_p99_cpu_fp32_seq128_ms NUMERIC(8,2),
    size_bytes          BIGINT       NOT NULL,

    -- Toolchain reproductibility (= ce qui était dans
    -- requirements-bundle.txt au moment du build)
    toolchain_fingerprint  TEXT      NOT NULL,

    -- Free-form notes
    notes               TEXT,

    -- Cohérence : revoked_at set ⇒ revoked_reason set
    CONSTRAINT bundle_releases_revocation_consistent CHECK (
        (revoked_at IS NULL AND revoked_reason IS NULL)
        OR (revoked_at IS NOT NULL AND revoked_reason IS NOT NULL)
    ),

    -- Unicité (bundle_id, version) — un seul release par version
    UNIQUE (bundle_id, version)
);

COMMENT ON TABLE bundle_releases IS
    'Φ-C D27 — append-only ledger des bundles releasés. Source de '
    'vérité légale pour SOC 2 CC8.1 + EU AI Act Art. 53. Joint '
    'avec Sigstore Rekor pour la vérification tiers.';

COMMENT ON COLUMN bundle_releases.manifest_sha256 IS
    'SHA-256 du manifest.json canonique (= bundle_fingerprint). '
    'C''est le hash que le bootloader vérifie au boot.';

COMMENT ON COLUMN bundle_releases.tar_sha256 IS
    'SHA-256 du .tar.gz distribué via OCI registry. Permet '
    'l''attestation reproductibility (= 3 runners doivent produire '
    'le même tar_sha256 sur les mêmes inputs).';

COMMENT ON COLUMN bundle_releases.rekor_log_index IS
    'Index dans le Sigstore Rekor transparency log. Permet à un '
    'tiers de vérifier indépendamment que cette release a bien '
    'été signée par EFE à la date released_at.';

-- Indices pour les queries opérationnelles
CREATE INDEX IF NOT EXISTS idx_bundle_releases_bundle_id_version
    ON bundle_releases(bundle_id, version);

CREATE INDEX IF NOT EXISTS idx_bundle_releases_released_at
    ON bundle_releases(released_at DESC);

CREATE INDEX IF NOT EXISTS idx_bundle_releases_revoked
    ON bundle_releases(revoked_at)
    WHERE revoked_at IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_bundle_releases_manifest_sha256
    ON bundle_releases(manifest_sha256);

CREATE INDEX IF NOT EXISTS idx_bundle_releases_kind
    ON bundle_releases(release_kind)
    WHERE release_kind IN ('revocation', 'emergency');

-- ─────────────────────────────────────────────────────────────────
-- bundle_release_attestations
-- ─────────────────────────────────────────────────────────────────
-- 1:N child table : chaque release peut avoir N attestations
-- (= audit firms, internal SRE sign-off, customer sign-off).
--
-- Pattern : chaque ligne = 1 attestation de qualité par 1 acteur.
-- Joint via release_id pour reconstruire l'historique complet.

CREATE TABLE IF NOT EXISTS bundle_release_attestations (
    attestation_id      UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    release_id          UUID         NOT NULL REFERENCES bundle_releases(release_id) ON DELETE CASCADE,

    attestor_kind       VARCHAR(20)  NOT NULL
                        CHECK (attestor_kind IN (
                            'internal_sre',      -- SRE on-call sign-off
                            'internal_security', -- security team sign-off
                            'audit_firm',        -- third-party audit firm
                            'customer',          -- customer who depends on this bundle
                            'automated_ci'       -- CI gates passed
                        )),
    attestor_identity   TEXT         NOT NULL,    -- email / company / CI workflow URL
    attestation_payload JSONB        NOT NULL,    -- free-form structured data
    attested_at         TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    signature_hmac      BYTEA                     -- HMAC du payload si signé
);

COMMENT ON TABLE bundle_release_attestations IS
    'Φ-C D27 — attestations multipartites par release bundle. '
    'Joint avec bundle_releases pour le rapport SOC 2 trimestriel.';

CREATE INDEX IF NOT EXISTS idx_bundle_release_attestations_release_id
    ON bundle_release_attestations(release_id);

CREATE INDEX IF NOT EXISTS idx_bundle_release_attestations_attestor_kind
    ON bundle_release_attestations(attestor_kind);

-- ─────────────────────────────────────────────────────────────────
-- Trigger : empêche la suppression d'une release (= append-only)
-- ─────────────────────────────────────────────────────────────────
--
-- Le ledger est append-only par contrat. Une révocation =
-- UPDATE de `revoked_at` + `revoked_reason`, **pas** un DELETE.
-- Le trigger force cette discipline.

CREATE OR REPLACE FUNCTION bundle_releases_no_delete()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION
        'bundle_releases is append-only ; use UPDATE to set revoked_at instead of DELETE';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_bundle_releases_no_delete ON bundle_releases;
CREATE TRIGGER trg_bundle_releases_no_delete
    BEFORE DELETE ON bundle_releases
    FOR EACH ROW
    EXECUTE FUNCTION bundle_releases_no_delete();

COMMIT;
