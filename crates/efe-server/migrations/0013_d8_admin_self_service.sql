-- 0013_d8_admin_self_service.sql — GLOSS Sprint A
--
-- Tables nécessaires à l'admin self-service :
--   - api_keys           : clés API par tenant, scopes, expiration, révocation
--   - sso_sessions       : sessions IdP côté portail admin
--   - ui_audit_log       : audit log des actions UI/CLI (HMAC chain D14-bis)
--   - tenant_compliance  : attestations versionnées AI Act Art. 14 + GDPR Art. 22
--
-- Backward compat strict :
--   - 4 NOUVELLES tables, isolées
--   - AUCUNE modification des tables M16-M22
--   - 1 trigger AFTER qui propage tenant_compliance → tenant_feature_mode3
--   - Toutes contraintes CHECK enforced niveau DB
--
-- Cf. `docs/EFE_GO_LIVE_ONBOARDING_ARCHITECTURE.md` § 9.

-- ─── api_keys ────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS api_keys (
    key_id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL,
    -- Argon2id hash of the raw plaintext key. NEVER store plaintext.
    key_hash            TEXT NOT NULL,
    -- Last 4 chars of the plaintext for UI display (« ...XYZW »).
    key_suffix          TEXT NOT NULL CHECK (length(key_suffix) = 4),
    scope               TEXT NOT NULL CHECK (scope IN ('read_only', 'classify', 'admin')),
    created_by          TEXT NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at          TIMESTAMPTZ,
    revoked_at          TIMESTAMPTZ,
    revoked_by          TEXT,
    revoked_reason      TEXT,
    last_used_at        TIMESTAMPTZ,
    use_count           BIGINT NOT NULL DEFAULT 0 CHECK (use_count >= 0),
    -- Either fully-revoked (3 cols set) or fully-active (3 cols null).
    CHECK (
        (revoked_at IS NULL AND revoked_by IS NULL AND revoked_reason IS NULL)
        OR (revoked_at IS NOT NULL AND revoked_by IS NOT NULL AND revoked_reason IS NOT NULL)
    )
);

-- Hot path : « given key_hash, find the active row in O(1) ».
CREATE INDEX IF NOT EXISTS ix_api_keys_hash_lookup ON api_keys(key_hash);

-- Hot path : « list active keys per tenant ».
-- Note : we intentionally do NOT include `expires_at > now()` in the
-- predicate ; Postgres requires index predicates to use IMMUTABLE
-- functions only, and now() is STABLE. The expiration check is done
-- at query time on this smaller (non-revoked) subset.
CREATE INDEX IF NOT EXISTS ix_api_keys_tenant_active
    ON api_keys(tenant_id)
    WHERE revoked_at IS NULL;

-- Cron path : « expire keys past their expires_at ».
CREATE INDEX IF NOT EXISTS ix_api_keys_expires
    ON api_keys(expires_at)
    WHERE revoked_at IS NULL AND expires_at IS NOT NULL;

-- ─── sso_sessions ─────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS sso_sessions (
    session_id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL,
    user_email          TEXT NOT NULL,
    user_subject        TEXT NOT NULL,           -- IdP `sub` claim (stable user id)
    idp_issuer          TEXT NOT NULL,           -- e.g. https://idp.acme.com/realms/efe
    refresh_token_hash  TEXT NOT NULL,           -- SHA256 of refresh token
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at          TIMESTAMPTZ NOT NULL,
    revoked_at          TIMESTAMPTZ,
    user_agent          TEXT,
    ip_address          INET
);

CREATE INDEX IF NOT EXISTS ix_sso_sessions_tenant_user
    ON sso_sessions(tenant_id, user_email)
    WHERE revoked_at IS NULL;

CREATE INDEX IF NOT EXISTS ix_sso_sessions_expires
    ON sso_sessions(expires_at)
    WHERE revoked_at IS NULL;

-- ─── ui_audit_log ─────────────────────────────────────────────────────
-- Audit log pour les actions UI/CLI ; complémentaire à llm_call_audit.
-- HMAC chain pattern (D14-bis) : prev_hmac + hmac chained per-tenant.
CREATE TABLE IF NOT EXISTS ui_audit_log (
    id              BIGSERIAL PRIMARY KEY,
    ts              TIMESTAMPTZ NOT NULL DEFAULT now(),
    tenant_id       TEXT NOT NULL,
    user_email      TEXT NOT NULL,
    -- e.g. 'tenant.create', 'compliance.attest', 'key.rotate', 'review.approve'
    action          TEXT NOT NULL CHECK (length(action) <= 64),
    target_type     TEXT,                        -- 'api_key', 'review', 'quota', etc.
    target_id       TEXT,
    -- JSONB payload : free-form, redacted (no secrets, no PII raw).
    payload_jsonb   JSONB,
    ip_address      INET,
    user_agent      TEXT,
    -- D14-bis HMAC chain.
    seq             BIGINT NOT NULL,
    prev_hmac       TEXT NOT NULL,
    hmac            TEXT NOT NULL,
    UNIQUE (tenant_id, seq)
);

CREATE INDEX IF NOT EXISTS ix_ui_audit_log_tenant_ts
    ON ui_audit_log(tenant_id, ts DESC);

CREATE INDEX IF NOT EXISTS ix_ui_audit_log_user
    ON ui_audit_log(user_email, ts DESC);

CREATE INDEX IF NOT EXISTS ix_ui_audit_log_action
    ON ui_audit_log(action, ts DESC);

-- ─── tenant_compliance ────────────────────────────────────────────────
-- Versioned attestations. The current attestation per tenant is the
-- most recent row where revoked_at IS NULL (use ix_tenant_compliance_current).
CREATE TABLE IF NOT EXISTS tenant_compliance (
    id                              BIGSERIAL PRIMARY KEY,
    tenant_id                       TEXT NOT NULL,
    -- AI Act Art. 14 (high-risk human oversight).
    ai_act_art14_acknowledged       BOOLEAN NOT NULL DEFAULT FALSE,
    ai_act_art14_acknowledged_at    TIMESTAMPTZ,
    ai_act_art14_signed_by          TEXT,
    -- GDPR Art. 22 (right-not-to-be-subject-to-solely-automated-decisions).
    gdpr_art22_acknowledged         BOOLEAN NOT NULL DEFAULT FALSE,
    gdpr_art22_acknowledged_at      TIMESTAMPTZ,
    gdpr_art22_dpo_contact          TEXT,
    -- SOC 2 readiness flag (no enforcement, informational).
    soc2_in_scope                   BOOLEAN NOT NULL DEFAULT FALSE,
    -- Attestation lifecycle.
    revoked_at                      TIMESTAMPTZ,
    revoked_reason                  TEXT,
    created_at                      TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Coherence : if acknowledged → ts and signer must be set.
    CHECK (
        NOT ai_act_art14_acknowledged
        OR (ai_act_art14_acknowledged_at IS NOT NULL AND ai_act_art14_signed_by IS NOT NULL)
    ),
    CHECK (
        NOT gdpr_art22_acknowledged
        OR (gdpr_art22_acknowledged_at IS NOT NULL AND gdpr_art22_dpo_contact IS NOT NULL)
    ),
    CHECK (
        (revoked_at IS NULL AND revoked_reason IS NULL)
        OR (revoked_at IS NOT NULL AND revoked_reason IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS ix_tenant_compliance_current
    ON tenant_compliance(tenant_id, created_at DESC)
    WHERE revoked_at IS NULL;

-- ─── Sync trigger : tenant_compliance → tenant_feature_mode3 ──────────
-- Quand une attestation AI Act Art. 14 est posée, propage l'horodatage
-- vers tenant_feature_mode3 (table M22). Cela débloque les tools Mode-3
-- via le trigger compliance_gate de la migration 0011.
CREATE OR REPLACE FUNCTION sync_tenant_compliance_to_mode3()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.ai_act_art14_acknowledged = TRUE
       AND NEW.ai_act_art14_acknowledged_at IS NOT NULL
       AND NEW.revoked_at IS NULL THEN
        INSERT INTO tenant_feature_mode3 (tenant_id, ai_act_art14_acknowledged_at)
             VALUES (NEW.tenant_id, NEW.ai_act_art14_acknowledged_at)
        ON CONFLICT (tenant_id)
        DO UPDATE SET
            ai_act_art14_acknowledged_at = NEW.ai_act_art14_acknowledged_at,
            updated_at = now();
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS sync_compliance_mode3 ON tenant_compliance;
CREATE TRIGGER sync_compliance_mode3
    AFTER INSERT OR UPDATE ON tenant_compliance
    FOR EACH ROW EXECUTE FUNCTION sync_tenant_compliance_to_mode3();

-- ─── Comments ─────────────────────────────────────────────────────────
COMMENT ON TABLE api_keys IS
    'GLOSS — API keys per tenant with scopes + expiration + revocation. Argon2id-hashed.';
COMMENT ON COLUMN api_keys.key_hash IS
    'Argon2id hash of the plaintext. Used for O(1) lookup at auth time.';
COMMENT ON COLUMN api_keys.key_suffix IS
    'Last 4 chars of plaintext for UI display only ; never used for auth.';
COMMENT ON COLUMN api_keys.scope IS
    'Authorization scope. read_only=GET; classify=GET+classify; admin=full.';

COMMENT ON TABLE sso_sessions IS
    'GLOSS — Browser sessions for portal-admin (NextAuth.js backed). 24h TTL default.';
COMMENT ON COLUMN sso_sessions.refresh_token_hash IS
    'SHA256 of OIDC refresh_token. Plaintext lives in browser cookie only.';

COMMENT ON TABLE ui_audit_log IS
    'GLOSS — UI+CLI action audit log. HMAC chain D14-bis pattern, per-tenant seq.';
COMMENT ON COLUMN ui_audit_log.action IS
    'Dotted action id (e.g. tenant.create, key.rotate). Max 64 chars.';
COMMENT ON COLUMN ui_audit_log.payload_jsonb IS
    'Redacted free-form JSONB. NEVER plaintext secrets or unredacted PII.';

COMMENT ON TABLE tenant_compliance IS
    'GLOSS — Versioned attestations AI Act Art. 14 + GDPR Art. 22. Trigger syncs to tenant_feature_mode3.';
COMMENT ON FUNCTION sync_tenant_compliance_to_mode3() IS
    'GLOSS — Propagates AI Act Art. 14 ack timestamp to tenant_feature_mode3 (unblocks Mode-3 compliance gate).';
