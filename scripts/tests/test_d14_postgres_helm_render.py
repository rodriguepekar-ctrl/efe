"""Sprint **D14** — Helm chart Postgres backend rendering.

Pin tests :

* ``values.yaml`` exposes a ``config.catalog.postgres`` block with
  the 13 expected fields.
* ``templates/configmap.yaml`` renders the 11 ``EFE_PG_*`` env vars
  conditionally on ``backend: "postgres"``.
* ``templates/deployment.yaml`` injects ``EFE_PG_PASSWORD`` from a
  Secret reference (one of three paths : existingSecretName /
  externalSecretName / inline password).
* ``crates/efe-server/migrations/0001_init.sql`` exists and matches
  the architecture doc § 6.1 schema (single ``catalogs`` table,
  composite PK, JSONB columns, partial parent_key index).

These tests do NOT require a live cluster — they read chart files
+ migration SQL and assert structural properties.
"""

from __future__ import annotations

import re
from pathlib import Path

import yaml

_REPO = Path(__file__).resolve().parents[2]
_CHART = _REPO / "deploy" / "helm" / "efe"
_VALUES = _CHART / "values.yaml"
_CONFIGMAP = _CHART / "templates" / "configmap.yaml"
_DEPLOYMENT = _CHART / "templates" / "deployment.yaml"
_MIGRATION = _REPO / "crates" / "efe-server" / "migrations" / "0001_init.sql"


# ─────────────────────────────────────────────────────────────────────
# values.yaml — postgres block structure
# ─────────────────────────────────────────────────────────────────────

EXPECTED_PG_KEYS = {
    "host",
    "port",
    "database",
    "user",
    "sslMode",
    "existingSecretName",
    "existingSecretKey",
    "externalSecretName",
    "password",
    "poolMaxConnections",
    "poolMinConnections",
    "poolAcquireTimeoutMs",
    "poolMaxLifetimeS",
    "poolIdleTimeoutS",
    "poolTestBeforeAcquire",
}


def test_values_yaml_has_postgres_block():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    pg = data["config"]["catalog"]["postgres"]
    missing = EXPECTED_PG_KEYS - set(pg.keys())
    assert not missing, f"values.yaml `config.catalog.postgres` missing keys: {missing}"


def test_values_yaml_postgres_ssl_mode_is_valid():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    mode = data["config"]["catalog"]["postgres"]["sslMode"]
    assert mode in {"disable", "prefer", "require", "verify-ca", "verify-full"}, (
        f"unexpected sslMode value: {mode!r}"
    )


def test_values_yaml_postgres_pool_defaults_match_arch_doc():
    """Defaults must match § 8.2 of d14_postgres_catalog_backend_architecture.md.

    Drift here = silent operator-vs-developer behaviour delta.
    """
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    pg = data["config"]["catalog"]["postgres"]
    expected = {
        "poolMaxConnections": 8,
        "poolMinConnections": 2,
        "poolAcquireTimeoutMs": 5000,
        "poolMaxLifetimeS": 1800,
        "poolIdleTimeoutS": 600,
        "poolTestBeforeAcquire": True,
    }
    for k, v in expected.items():
        assert pg[k] == v, f"config.catalog.postgres.{k} drift : {pg[k]} vs {v}"


def test_values_yaml_default_backend_remains_memory_for_safety():
    """The chart default MUST remain `memory` so a bare-bones
    `helm install` does not require a Postgres instance to boot.
    Postgres mode is opt-in.
    """
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    assert data["config"]["catalog"]["backend"] == "memory"


def test_values_yaml_password_inline_is_empty_by_default():
    """Inline password MUST default to empty so a `helm install`
    without an explicit Secret does not boot with a literal
    password committed to git history."""
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    assert data["config"]["catalog"]["postgres"]["password"] == ""


# ─────────────────────────────────────────────────────────────────────
# configmap.yaml — env var rendering
# ─────────────────────────────────────────────────────────────────────

EXPECTED_PG_ENV_VARS = {
    "EFE_PG_HOST",
    "EFE_PG_PORT",
    "EFE_PG_DATABASE",
    "EFE_PG_USER",
    "EFE_PG_SSL_MODE",
    "EFE_PG_POOL_MAX_CONNECTIONS",
    "EFE_PG_POOL_MIN_CONNECTIONS",
    "EFE_PG_POOL_ACQUIRE_TIMEOUT_MS",
    "EFE_PG_POOL_MAX_LIFETIME_S",
    "EFE_PG_POOL_IDLE_TIMEOUT_S",
    "EFE_PG_POOL_TEST_BEFORE_ACQUIRE",
}


def test_configmap_renders_11_pg_env_vars():
    text = _CONFIGMAP.read_text(encoding="utf-8")
    for key in EXPECTED_PG_ENV_VARS:
        assert f"{key}:" in text, f"configmap.yaml missing env var {key}"


def test_configmap_pg_block_is_gated_on_backend_postgres():
    """The `EFE_PG_*` block MUST be inside a
    `{{- if eq .Values.config.catalog.backend "postgres" }}`
    guard so Memory-mode deploys don't ship orphan env vars."""
    text = _CONFIGMAP.read_text(encoding="utf-8")
    assert '{{- if eq .Values.config.catalog.backend "postgres" }}' in text, (
        "configmap.yaml must gate the EFE_PG_* block on backend=postgres"
    )


def test_configmap_does_not_render_pg_password():
    """PG password MUST NEVER appear *as a rendered key* in the
    ConfigMap (which is plain text in etcd). It belongs in a Secret,
    referenced by the Deployment. We allow `EFE_PG_PASSWORD` to
    appear in template comments — the contract is "no rendered key
    of that name", not "no mention of the string".
    """
    text = _CONFIGMAP.read_text(encoding="utf-8")
    # A rendered key has the shape `  EFE_PG_PASSWORD:` at the start
    # of a YAML line (after some indentation). Comments embed the
    # name in prose like `the EFE_PG_PASSWORD env var is`.
    rendered_key_re = re.compile(r"^\s{2,}EFE_PG_PASSWORD:", re.MULTILINE)
    matches = rendered_key_re.findall(text)
    assert not matches, (
        f"PG password must not be rendered as a ConfigMap key ; "
        f"found {len(matches)} rendered occurrence(s)."
    )


# ─────────────────────────────────────────────────────────────────────
# deployment.yaml — Secret injection paths
# ─────────────────────────────────────────────────────────────────────


def test_deployment_injects_pg_password_from_secret():
    text = _DEPLOYMENT.read_text(encoding="utf-8")
    assert "EFE_PG_PASSWORD" in text, (
        "deployment.yaml must inject EFE_PG_PASSWORD when backend=postgres"
    )
    assert "secretKeyRef:" in text, (
        "deployment.yaml must use secretKeyRef to fetch the PG password"
    )


def test_deployment_supports_three_secret_paths():
    """The deployment must wire the password from one of three
    sources : existingSecretName, externalSecretName, or inline
    password (Tier 1 only)."""
    text = _DEPLOYMENT.read_text(encoding="utf-8")
    assert "existingSecretName" in text
    assert "externalSecretName" in text
    assert "postgres.password" in text


def test_deployment_pg_password_block_is_gated_on_backend_postgres():
    """The PG password env var block MUST be guarded so memory-mode
    deploys don't try to look up a Secret that doesn't exist."""
    text = _DEPLOYMENT.read_text(encoding="utf-8")
    # We expect at least one `eq .Values.config.catalog.backend "postgres"`
    # check around the PG password injection.
    assert re.search(
        r'{{-?\s*if\s+eq\s+\.Values\.config\.catalog\.backend\s+"postgres"\s*}}',
        text,
    ), "deployment.yaml must gate the EFE_PG_PASSWORD on backend=postgres"


# ─────────────────────────────────────────────────────────────────────
# migrations/0001_init.sql — schema invariants
# ─────────────────────────────────────────────────────────────────────


def test_migration_init_sql_exists():
    assert _MIGRATION.exists(), (
        f"missing initial migration : {_MIGRATION.relative_to(_REPO)}"
    )


def test_migration_creates_catalogs_table():
    text = _MIGRATION.read_text(encoding="utf-8")
    assert "CREATE TABLE catalogs" in text


def test_migration_has_composite_primary_key():
    text = _MIGRATION.read_text(encoding="utf-8")
    assert "PRIMARY KEY (tenant, name, version)" in text


def test_migration_uses_jsonb_for_content():
    text = _MIGRATION.read_text(encoding="utf-8")
    assert re.search(r"content_json\s+JSONB", text), (
        "content_json column must be typed JSONB (not JSON or TEXT)"
    )


def test_migration_has_parent_key_check_constraint():
    text = _MIGRATION.read_text(encoding="utf-8")
    assert "catalogs_parent_chain" in text, (
        "parent_key regex check constraint missing"
    )


def test_migration_has_partial_index_on_parent_key():
    text = _MIGRATION.read_text(encoding="utf-8")
    assert re.search(
        r"CREATE INDEX\s+catalogs_parent_key_idx\s+ON\s+catalogs\s*\(parent_key\)\s+WHERE\s+parent_key\s+<>\s+''",
        text,
    ), "partial index on parent_key (WHERE parent_key <> '') missing"


def test_migration_has_db_updated_at_trigger():
    text = _MIGRATION.read_text(encoding="utf-8")
    assert "catalogs_set_updated_at" in text
    assert "BEFORE UPDATE ON catalogs" in text
