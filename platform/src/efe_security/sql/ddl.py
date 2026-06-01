"""Postgres DDL for M11 — the canonical schema.

The strings below are importable and composable with any async driver
(``asyncpg``, ``psycopg``). They match §Annex E of the architecture
document exactly.

All multi-tenant tables enable Row Level Security. Every
``current_setting('efe.tenant_id', true)`` reference relies on
:class:`~efe_security.authz.rls_bridge.RlsBridge` to set the session
variable at transaction start.
"""

from __future__ import annotations

__all__ = [
    "AUDIT_LOG_DDL",
    "CONSENT_RECORDS_DDL",
    "DSAR_REQUESTS_DDL",
    "FULL_DDL",
    "KEY_REGISTRY_DDL",
    "PII_FIELD_REGISTRY_DDL",
    "RBAC_DDL",
    "SECURITY_INCIDENTS_DDL",
    "TENANTS_DDL",
]


TENANTS_DDL: str = """
CREATE TABLE IF NOT EXISTS tenants (
    tenant_id         TEXT PRIMARY KEY,
    display_name      TEXT NOT NULL,
    tier              TEXT NOT NULL CHECK (
        tier IN ('standard', 'hipaa', 'eu_sovereign')),
    region            TEXT NOT NULL,
    onboarded_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    offboarded_at     TIMESTAMPTZ,
    dek_wrapped       BYTEA NOT NULL,
    dek_key_version   INTEGER NOT NULL,
    labels            JSONB NOT NULL DEFAULT '{}'::jsonb,
    state             TEXT NOT NULL DEFAULT 'active',
    state_version     INTEGER NOT NULL DEFAULT 1
);
"""


RBAC_DDL: str = """
CREATE TABLE IF NOT EXISTS rbac_roles (
    role_id       TEXT PRIMARY KEY,
    display_name  TEXT NOT NULL,
    description   TEXT
);

CREATE TABLE IF NOT EXISTS rbac_role_scopes (
    role_id  TEXT NOT NULL REFERENCES rbac_roles(role_id),
    scope    TEXT NOT NULL,
    PRIMARY KEY (role_id, scope)
);

CREATE TABLE IF NOT EXISTS rbac_principal_roles (
    tenant_id     TEXT NOT NULL REFERENCES tenants(tenant_id),
    principal_id  TEXT NOT NULL,
    role_id       TEXT NOT NULL REFERENCES rbac_roles(role_id),
    granted_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    granted_by    TEXT NOT NULL,
    revoked_at    TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, principal_id, role_id)
);

ALTER TABLE rbac_principal_roles ENABLE ROW LEVEL SECURITY;
CREATE POLICY IF NOT EXISTS rbac_tenant_isolation ON rbac_principal_roles
  USING (tenant_id = current_setting('efe.tenant_id', true));
"""


PII_FIELD_REGISTRY_DDL: str = """
CREATE TABLE IF NOT EXISTS pii_field_registry (
    field_id        UUID PRIMARY KEY,
    table_name      TEXT NOT NULL,
    column_name     TEXT NOT NULL,
    classification  TEXT NOT NULL CHECK (
        classification IN ('public','internal','pii','phi','secret')),
    detector        TEXT NOT NULL,
    transform       TEXT NOT NULL,
    retention_days  INTEGER NOT NULL,
    notes           TEXT,
    UNIQUE (table_name, column_name)
);
"""


CONSENT_RECORDS_DDL: str = """
CREATE TABLE IF NOT EXISTS consent_records (
    consent_id           UUID PRIMARY KEY,
    tenant_id            TEXT NOT NULL REFERENCES tenants(tenant_id),
    subject_id           TEXT NOT NULL,
    purposes             TEXT[] NOT NULL,
    legal_basis          TEXT NOT NULL CHECK (
        legal_basis IN ('consent','contract','legal_obligation',
                        'vital_interests','public_task','legitimate_interests')),
    consent_text_digest  BYTEA NOT NULL,
    locale               TEXT NOT NULL,
    collected_at         TIMESTAMPTZ NOT NULL,
    collected_by         TEXT NOT NULL,
    expires_at           TIMESTAMPTZ,
    withdrawn_at         TIMESTAMPTZ,
    seal                 BYTEA NOT NULL,
    state                TEXT NOT NULL DEFAULT 'active'
);
CREATE INDEX IF NOT EXISTS idx_consent_subject
  ON consent_records (tenant_id, subject_id)
  WHERE withdrawn_at IS NULL;

ALTER TABLE consent_records ENABLE ROW LEVEL SECURITY;
CREATE POLICY IF NOT EXISTS consent_tenant_isolation ON consent_records
  USING (tenant_id = current_setting('efe.tenant_id', true));
"""


DSAR_REQUESTS_DDL: str = """
CREATE TABLE IF NOT EXISTS dsar_requests (
    request_id    UUID PRIMARY KEY,
    tenant_id     TEXT NOT NULL REFERENCES tenants(tenant_id),
    subject_id    TEXT NOT NULL,
    kind          TEXT NOT NULL CHECK (
        kind IN ('access','rectify','erase','portability')),
    state         TEXT NOT NULL CHECK (
        state IN ('received','in_progress','completed','rejected')),
    submitted_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at  TIMESTAMPTZ,
    receipt_seal  BYTEA,
    notes         TEXT
);
CREATE INDEX IF NOT EXISTS idx_dsar_tenant_subject
  ON dsar_requests (tenant_id, subject_id);

ALTER TABLE dsar_requests ENABLE ROW LEVEL SECURITY;
CREATE POLICY IF NOT EXISTS dsar_tenant_isolation ON dsar_requests
  USING (tenant_id = current_setting('efe.tenant_id', true));
"""


AUDIT_LOG_DDL: str = """
CREATE TABLE IF NOT EXISTS audit_log (
    audit_id        UUID PRIMARY KEY,
    occurred_at     TIMESTAMPTZ NOT NULL,
    tenant_id       TEXT,
    actor_kind      TEXT NOT NULL,
    actor_id        TEXT NOT NULL,
    action          TEXT NOT NULL,
    resource_kind   TEXT,
    resource_id     TEXT,
    outcome         TEXT NOT NULL,
    reasons         TEXT[] NOT NULL DEFAULT '{}',
    severity        TEXT NOT NULL,
    payload_digest  BYTEA NOT NULL,
    prev_seal       BYTEA,
    seal            BYTEA NOT NULL,
    key_version     INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_audit_tenant_time
  ON audit_log (tenant_id, occurred_at);
CREATE INDEX IF NOT EXISTS idx_audit_actor_time
  ON audit_log (actor_id, occurred_at);

ALTER TABLE audit_log ENABLE ROW LEVEL SECURITY;
-- Insert-only : UPDATE and DELETE have no matching policy and are
-- consequently denied.
CREATE POLICY IF NOT EXISTS audit_log_insert ON audit_log
  FOR INSERT WITH CHECK (true);
CREATE POLICY IF NOT EXISTS audit_log_select ON audit_log
  FOR SELECT USING (
    tenant_id IS NULL
    OR tenant_id = current_setting('efe.tenant_id', true)
  );
"""


SECURITY_INCIDENTS_DDL: str = """
CREATE TABLE IF NOT EXISTS security_incidents (
    incident_id   UUID PRIMARY KEY,
    tenant_id     TEXT,
    detected_at   TIMESTAMPTZ NOT NULL,
    severity      TEXT NOT NULL,
    category      TEXT NOT NULL,
    summary       TEXT NOT NULL,
    payload       JSONB NOT NULL,
    state         TEXT NOT NULL,
    notified_at   TIMESTAMPTZ,
    resolved_at   TIMESTAMPTZ,
    seal          BYTEA NOT NULL
);

ALTER TABLE security_incidents ENABLE ROW LEVEL SECURITY;
CREATE POLICY IF NOT EXISTS incident_tenant_isolation ON security_incidents
  USING (
    tenant_id IS NULL
    OR tenant_id = current_setting('efe.tenant_id', true)
  );
"""


KEY_REGISTRY_DDL: str = """
CREATE TABLE IF NOT EXISTS key_registry (
    key_id         TEXT PRIMARY KEY,
    alg_id         TEXT NOT NULL,
    purpose        TEXT NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL,
    rotated_from   TEXT REFERENCES key_registry(key_id),
    retired_at     TIMESTAMPTZ,
    state          TEXT NOT NULL CHECK (state IN ('active','retiring','retired')),
    metadata       JSONB NOT NULL DEFAULT '{}'::jsonb,
    state_version  INTEGER NOT NULL DEFAULT 1
);
"""


FULL_DDL: str = "\n\n".join(
    [
        TENANTS_DDL,
        RBAC_DDL,
        PII_FIELD_REGISTRY_DDL,
        CONSENT_RECORDS_DDL,
        DSAR_REQUESTS_DDL,
        AUDIT_LOG_DDL,
        SECURITY_INCIDENTS_DDL,
        KEY_REGISTRY_DDL,
    ]
)
