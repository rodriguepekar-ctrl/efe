-- ============================================================================
-- Sprint D15 — Tenant registry (onboarding source of truth).
-- ============================================================================
--
-- Reference : `docs/bench/d15_onboarding_idp_architecture.md` § 15.
--
-- The `tenants` table is the **single source of truth** for which tenants
-- have been onboarded onto this EFE deployment. It feeds :
--
--   * `efe-onboard` CLI (`preflight`, `activate`, `suspend`, `list`).
--   * The OIDC `IssuerEntry` lookup when `tenant_strategy = 'issuer'`
--     (the issuer URL is mapped to a tenant via this table).
--   * The Grafana dashboard `d15_tenant_onboarding` (status histogram).
--   * Compliance reports : "list of tenants active during 2026-Q1".
--
-- Format conventions :
--   * `tenant_id`     — same regex as catalogs.tenant (D14 audit fix C1).
--   * `idp_issuer`    — issuer URL (HTTPS), free-form (any IdP).
--   * `idp_strategy`  — 'claim' (read tenant_id from JWT custom claim) or
--                       'issuer' (the issuer URL maps directly to this
--                       tenant_id ; useful for Auth0 organizations or
--                       Keycloak realms).
--   * `idp_meta`      — JSONB with vendor-specific extras (e.g. Auth0
--                       organization_id, Okta authorization_server_id).
-- ============================================================================

CREATE TABLE tenants (
    tenant_id     TEXT PRIMARY KEY,
    display_name  TEXT NOT NULL,
    status        TEXT NOT NULL,                 -- 'pending' | 'active' | 'suspended'
    idp_issuer    TEXT NOT NULL,
    idp_strategy  TEXT NOT NULL,                 -- 'claim' | 'issuer'
    idp_meta      JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    activated_at  TIMESTAMPTZ,
    suspended_at  TIMESTAMPTZ,

    CONSTRAINT tenants_id_format
        CHECK (tenant_id ~ '^[a-z0-9_-]{1,63}$'),
    CONSTRAINT tenants_status_enum
        CHECK (status IN ('pending', 'active', 'suspended')),
    CONSTRAINT tenants_idp_strategy_enum
        CHECK (idp_strategy IN ('claim', 'issuer')),
    CONSTRAINT tenants_display_name_nonempty
        CHECK (length(display_name) >= 1),
    CONSTRAINT tenants_idp_issuer_nonempty
        CHECK (length(idp_issuer) >= 1)
);

CREATE INDEX tenants_idp_issuer_idx ON tenants (idp_issuer);
CREATE INDEX tenants_status_idx ON tenants (status);

COMMENT ON TABLE tenants IS
    'Sprint D15 — Tenant registry. Source of truth for the onboarding '
    'workflow (`efe-onboard` CLI) and for the OIDC `tenant_strategy = '
    '''issuer''` lookup. Status transitions : pending → active → '
    'suspended (or pending → suspended directly for cancelled '
    'onboardings). The activated_at / suspended_at columns are set by '
    'the CLI ; the trigger emits an audit_events row on each transition.';

COMMENT ON COLUMN tenants.idp_meta IS
    'Vendor-specific metadata. Examples : '
    '{"auth0_org_id":"org_abc123"}, '
    '{"keycloak_realm":"efe","keycloak_client_id":"efe-resource-server"}, '
    '{"okta_authorization_server_id":"aus..."}, '
    '{"azure_tenant_id":"abcd-..."}';
