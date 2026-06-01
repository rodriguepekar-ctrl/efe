"""Postgres DDL for M12 — matches §Annex C of the architecture."""

from __future__ import annotations

__all__ = [
    "FULL_DDL",
    "SWARM_ARTEFACT_MANIFESTS_DDL",
    "SWARM_CALL_LOG_DDL",
    "SWARM_CONE_BINDINGS_DDL",
]


SWARM_CONE_BINDINGS_DDL: str = """
CREATE TABLE IF NOT EXISTS swarm_cone_bindings (
    binding_id         UUID PRIMARY KEY,
    tenant_id          TEXT NOT NULL,
    feature_id         TEXT NOT NULL,
    task_id            TEXT,
    cone_tenant        TEXT NOT NULL,
    cone_task          TEXT NOT NULL,
    cone_version       INTEGER NOT NULL,
    cone_key           TEXT NOT NULL,
    embedder_family    TEXT NOT NULL,
    embedder_version   TEXT NOT NULL,
    anchor_count       INTEGER NOT NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    disabled_at        TIMESTAMPTZ,
    state_version      INTEGER NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_cone_bindings_unique
    ON swarm_cone_bindings (tenant_id, feature_id, task_id)
    WHERE disabled_at IS NULL;

ALTER TABLE swarm_cone_bindings ENABLE ROW LEVEL SECURITY;
CREATE POLICY IF NOT EXISTS cone_bindings_tenant_isolation ON swarm_cone_bindings
    USING (tenant_id = current_setting('efe.tenant_id', true));
"""


SWARM_ARTEFACT_MANIFESTS_DDL: str = """
CREATE TABLE IF NOT EXISTS swarm_artefact_manifests (
    manifest_id        UUID PRIMARY KEY,
    tenant_id          TEXT NOT NULL,
    artefact_id        TEXT NOT NULL,
    digest             BYTEA NOT NULL,
    seal               BYTEA NOT NULL,
    synced_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    source_bucket      TEXT NOT NULL,
    target_bucket      TEXT NOT NULL,
    dim                INTEGER NOT NULL,
    tokenizer_family   TEXT,
    labels             JSONB NOT NULL DEFAULT '{}'::jsonb
);
CREATE INDEX IF NOT EXISTS idx_artefact_tenant
    ON swarm_artefact_manifests(tenant_id);

ALTER TABLE swarm_artefact_manifests ENABLE ROW LEVEL SECURITY;
CREATE POLICY IF NOT EXISTS artefact_manifest_tenant_isolation
    ON swarm_artefact_manifests
    USING (tenant_id = current_setting('efe.tenant_id', true));
"""


SWARM_CALL_LOG_DDL: str = """
CREATE TABLE IF NOT EXISTS swarm_call_log (
    call_id            UUID PRIMARY KEY,
    occurred_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    tenant_id          TEXT NOT NULL,
    actor_id           TEXT NOT NULL,
    method             TEXT NOT NULL,
    status_code        TEXT NOT NULL,
    latency_ms         REAL NOT NULL,
    swarm_id           TEXT,
    rounds_executed    INTEGER,
    verdict            TEXT,
    error_code         TEXT,
    attributes         JSONB NOT NULL DEFAULT '{}'::jsonb
);
CREATE INDEX IF NOT EXISTS idx_call_log_tenant_time
    ON swarm_call_log(tenant_id, occurred_at);

ALTER TABLE swarm_call_log ENABLE ROW LEVEL SECURITY;
CREATE POLICY IF NOT EXISTS call_log_tenant_isolation ON swarm_call_log
    USING (tenant_id = current_setting('efe.tenant_id', true));
"""


FULL_DDL: str = "\n\n".join(
    [
        SWARM_CONE_BINDINGS_DDL,
        SWARM_ARTEFACT_MANIFESTS_DDL,
        SWARM_CALL_LOG_DDL,
    ]
)
