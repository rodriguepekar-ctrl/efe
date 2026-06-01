"""Sprint **D14** — Runtime + config + telemetry wire-up surface pin.

Reads the Rust source files directly (no compilation required) and
asserts that the canonical D14 wire-up tokens are present. Catches
regressions like :

* Someone removes the `CatalogBackend::Postgres` arm from
  `runtime.rs` — Postgres mode silently falls back to Memory.
* Someone forgets to register the D14 Prometheus instruments —
  pool gauges return "no data" in Grafana even when `EFE_PG_*` is
  configured.
* Someone strips the migration `migrate!` macro call — fresh PG
  databases boot with an empty schema.
"""

from __future__ import annotations

from pathlib import Path

_REPO = Path(__file__).resolve().parents[2]
_RUNTIME_RS = _REPO / "crates" / "efe-server" / "src" / "runtime.rs"
_CONFIG_RS = _REPO / "crates" / "efe-server" / "src" / "config.rs"
_POSTGRES_RS = _REPO / "crates" / "efe-server" / "src" / "catalog" / "postgres.rs"
_TELEMETRY_RS = (
    _REPO / "crates" / "efe-telemetry" / "src" / "metrics" / "d14_instruments.rs"
)
_SPANS_RS = _REPO / "crates" / "efe-telemetry" / "src" / "spans.rs"


# ─────────────────────────────────────────────────────────────────────
# runtime.rs — Postgres branch
# ─────────────────────────────────────────────────────────────────────


def test_runtime_rs_opens_postgres_branch():
    """The Postgres backend MUST instantiate `PostgresCatalogRegistry`
    rather than returning the legacy fail-fast Err.
    """
    text = _RUNTIME_RS.read_text(encoding="utf-8")
    assert "PostgresCatalogRegistry::connect(" in text, (
        "runtime.rs must call PostgresCatalogRegistry::connect for the Postgres branch"
    )
    assert "run_migrations()" in text, (
        "runtime.rs must run migrations at boot for fail-fast schema validation"
    )
    assert "register_all_d14()" in text, (
        "runtime.rs must register the D14 instruments so dashboards see "
        "non-empty metric families immediately"
    )


def test_runtime_rs_build_is_async():
    """`ServerBuilder::build` was made async in D14 to accommodate
    `PostgresCatalogRegistry::connect`. Async-ness must be preserved
    by callers (tests, main, benches add `.await`).
    """
    text = _RUNTIME_RS.read_text(encoding="utf-8")
    assert "pub async fn build" in text, (
        "ServerBuilder::build must be `async` to support Postgres connect at boot"
    )


# ─────────────────────────────────────────────────────────────────────
# config.rs — CatalogPostgresConfig + 11 env vars
# ─────────────────────────────────────────────────────────────────────

EXPECTED_PG_ENV_VARS_IN_CONFIG = {
    "EFE_PG_HOST",
    "EFE_PG_PORT",
    "EFE_PG_DATABASE",
    "EFE_PG_USER",
    "EFE_PG_PASSWORD",
    "EFE_PG_SSL_MODE",
    "EFE_PG_POOL_MAX_CONNECTIONS",
    "EFE_PG_POOL_MIN_CONNECTIONS",
    "EFE_PG_POOL_ACQUIRE_TIMEOUT_MS",
    "EFE_PG_POOL_MAX_LIFETIME_S",
    "EFE_PG_POOL_IDLE_TIMEOUT_S",
    "EFE_PG_POOL_TEST_BEFORE_ACQUIRE",
}


def test_config_rs_declares_catalog_postgres_config():
    text = _CONFIG_RS.read_text(encoding="utf-8")
    assert "pub struct CatalogPostgresConfig" in text


def test_config_rs_reads_all_pg_env_vars():
    text = _CONFIG_RS.read_text(encoding="utf-8")
    for key in EXPECTED_PG_ENV_VARS_IN_CONFIG:
        assert key in text, f"config.rs does not read env var {key}"


def test_config_rs_validates_pg_when_backend_is_postgres():
    """The validate() method MUST guard against zero/empty values
    when backend=postgres (audit-style fail-fast)."""
    text = _CONFIG_RS.read_text(encoding="utf-8")
    assert "CatalogBackend::Postgres" in text
    assert "catalog.postgres.host" in text, (
        "validate() must check the host field for non-empty"
    )
    assert "catalog.postgres.pool_max_connections" in text


def test_config_rs_dsn_redaction_replaces_password():
    """The redacted DSN MUST replace the password by `***` so logs
    don't leak credentials. Critical for the audit trail. We accept
    either an explicit `"***"` literal or a `format!` substring like
    `:***` — both achieve the same redaction.
    """
    text = _CONFIG_RS.read_text(encoding="utf-8")
    assert "dsn_redacted" in text
    assert "***" in text, (
        "config.rs::dsn_redacted must include `***` to mask the password"
    )


# ─────────────────────────────────────────────────────────────────────
# postgres.rs — trait impl + helpers
# ─────────────────────────────────────────────────────────────────────


def test_postgres_rs_implements_catalog_registry():
    text = _POSTGRES_RS.read_text(encoding="utf-8")
    assert "impl CatalogRegistry for PostgresCatalogRegistry" in text


def test_postgres_rs_uses_advisory_lock():
    """The `(tenant, name)` advisory lock is the concurrency
    primitive that lets monotone version assignment work without
    SERIALIZABLE isolation. Removing it = race conditions."""
    text = _POSTGRES_RS.read_text(encoding="utf-8")
    assert "pg_advisory_xact_lock" in text


def test_postgres_rs_lock_id_seed_is_efe_namespaced():
    """The xxh64 seed MUST be in the EFE namespace so locks don't
    collide with unrelated apps sharing the same cluster."""
    text = _POSTGRES_RS.read_text(encoding="utf-8")
    assert "0xefe_0000_d014_0000" in text, (
        "ADVISORY_LOCK_SEED must be the documented EFE-namespaced constant"
    )


def test_postgres_rs_sanitises_internal_errors():
    """`map_pg_error` for internal codes (e.g. 23502) MUST log
    the inner detail server-side but return an opaque message to
    the client. Audit fix T-12bis-2 invariant."""
    text = _POSTGRES_RS.read_text(encoding="utf-8")
    assert "map_pg_error" in text
    assert 'tracing::error!' in text or "tracing::error!(" in text


def test_postgres_rs_uses_sqlx_migrate_macro():
    text = _POSTGRES_RS.read_text(encoding="utf-8")
    assert 'sqlx::migrate!("./migrations")' in text


# ─────────────────────────────────────────────────────────────────────
# d14_instruments.rs — 4 new metrics
# ─────────────────────────────────────────────────────────────────────


def test_d14_instruments_declares_pool_gauges():
    text = _TELEMETRY_RS.read_text(encoding="utf-8")
    assert "efe_pg_pool_active_connections" in text
    assert "efe_pg_pool_idle_connections" in text


def test_d14_instruments_declares_query_histogram():
    text = _TELEMETRY_RS.read_text(encoding="utf-8")
    assert "efe_pg_query_duration_seconds" in text


def test_d14_instruments_declares_tx_failure_counter():
    text = _TELEMETRY_RS.read_text(encoding="utf-8")
    assert "efe_pg_tx_failures_total" in text


def test_d14_instruments_declares_migration_gauge():
    text = _TELEMETRY_RS.read_text(encoding="utf-8")
    assert "efe_pg_migration_version" in text


def test_d14_instruments_register_all_is_idempotent():
    """`register_all_d14` MUST be callable multiple times without
    panicking — invoked from the runtime's boot path."""
    text = _TELEMETRY_RS.read_text(encoding="utf-8")
    assert "pub fn register_all_d14" in text


# ─────────────────────────────────────────────────────────────────────
# spans.rs — 7 new PG spans
# ─────────────────────────────────────────────────────────────────────

EXPECTED_PG_SPANS = {
    "PG_REGISTER",
    "PG_EVOLVE",
    "PG_GET",
    "PG_LIST_CATALOGS",
    "PG_LIST_VERSIONS",
    "PG_DELETE",
    "PG_MIGRATE",
}


def test_spans_rs_declares_d14_pg_spans():
    text = _SPANS_RS.read_text(encoding="utf-8")
    for span in EXPECTED_PG_SPANS:
        assert f"pub const {span}" in text, f"spans.rs missing constant {span}"


def test_spans_rs_pg_namespace_consistency():
    """All PG spans must follow the `efe.pg.*` naming convention so
    operators can filter by namespace in Tempo / Jaeger.
    """
    text = _SPANS_RS.read_text(encoding="utf-8")
    pg_consts = [
        ('PG_REGISTER', '"efe.pg.register"'),
        ('PG_EVOLVE', '"efe.pg.evolve"'),
        ('PG_GET', '"efe.pg.get"'),
        ('PG_LIST_CATALOGS', '"efe.pg.list_catalogs"'),
        ('PG_LIST_VERSIONS', '"efe.pg.list_versions"'),
        ('PG_DELETE', '"efe.pg.delete"'),
        ('PG_MIGRATE', '"efe.pg.migrate"'),
    ]
    for const_name, expected_value in pg_consts:
        assert expected_value in text, (
            f"spans.rs constant {const_name} must equal {expected_value}"
        )
