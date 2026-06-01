"""SQL artefacts for M11 — DDL + migration helpers.

The in-memory stores remain the reference impl for tests ; this module
provides the canonical Postgres DDL so production deployments can
materialise the same schema. The DDL follows §Annex E of
``M11_SECURITY_ARCHITECTURE.md``.
"""

from __future__ import annotations

from efe_security.sql.ddl import (
    AUDIT_LOG_DDL,
    CONSENT_RECORDS_DDL,
    DSAR_REQUESTS_DDL,
    FULL_DDL,
    KEY_REGISTRY_DDL,
    PII_FIELD_REGISTRY_DDL,
    RBAC_DDL,
    SECURITY_INCIDENTS_DDL,
    TENANTS_DDL,
)

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
