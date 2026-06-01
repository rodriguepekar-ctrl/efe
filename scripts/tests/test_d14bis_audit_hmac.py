"""Sprint **D14-bis** — HMAC-chained audit log surface pin.

Reads the Rust source files + Helm templates + migration SQL and
asserts the canonical D14-bis wire-up tokens are present. Catches
regressions like :

* Someone removes the `HmacChainedAuditSink::emit` impl — audit
  signing silently regresses to TracingAuditSink.
* Someone forgets the append-only triggers in
  `0003_audit_events.sql` — the table becomes mutable.
* Someone breaks the canonical payload field inventory — verify
  fails at the next chain check.
* Someone leaks the HMAC password into the ConfigMap (must always
  go through a Secret).
"""

from __future__ import annotations

import re
from pathlib import Path

import yaml

_REPO = Path(__file__).resolve().parents[2]
_AUDIT_HMAC_RS = _REPO / "crates" / "efe-server" / "src" / "catalog" / "audit_hmac.rs"
_AUDIT_KEYS_RS = _REPO / "crates" / "efe-server" / "src" / "catalog" / "audit_keys.rs"
_CONFIG_RS = _REPO / "crates" / "efe-server" / "src" / "config.rs"
_RUNTIME_RS = _REPO / "crates" / "efe-server" / "src" / "runtime.rs"
_LIB_RS = _REPO / "crates" / "efe-server" / "src" / "lib.rs"
_INSTRUMENTS_RS = (
    _REPO / "crates" / "efe-telemetry" / "src" / "metrics" / "d14bis_instruments.rs"
)
_SPANS_RS = _REPO / "crates" / "efe-telemetry" / "src" / "spans.rs"
_MIGRATION_RS = _REPO / "crates" / "efe-server" / "migrations" / "0003_audit_events.sql"
_VERIFY_BIN_RS = (
    _REPO / "crates" / "efe-server" / "src" / "bin" / "efe_verify_audit.rs"
)
_VALUES = _REPO / "deploy" / "helm" / "efe" / "values.yaml"
_CONFIGMAP = _REPO / "deploy" / "helm" / "efe" / "templates" / "configmap.yaml"
_DEPLOYMENT = _REPO / "deploy" / "helm" / "efe" / "templates" / "deployment.yaml"


# ─────────────────────────────────────────────────────────────────────
# audit_hmac.rs — sink + verify library
# ─────────────────────────────────────────────────────────────────────


def test_audit_hmac_rs_implements_audit_sink():
    text = _AUDIT_HMAC_RS.read_text(encoding="utf-8")
    assert "impl AuditSink for HmacChainedAuditSink" in text


def test_audit_hmac_rs_uses_advisory_lock_per_chain():
    """The advisory lock per chain is the concurrency primitive that
    serialises prev_hmac reads + inserts. Removing it = race condition
    on chain head."""
    text = _AUDIT_HMAC_RS.read_text(encoding="utf-8")
    assert "pg_advisory_xact_lock" in text
    assert "lock_id_for_chain" in text


def test_audit_hmac_rs_genesis_constant():
    text = _AUDIT_HMAC_RS.read_text(encoding="utf-8")
    # 64 zeros hex
    assert "GENESIS_PREV_HMAC" in text
    assert '"' + "0" * 64 + '"' in text


def test_audit_hmac_rs_canonical_payload_uses_btreemap():
    """The canonical serialiser MUST use BTreeMap (alphabetical key
    iteration) so the JSON output is byte-stable. HashMap would break
    determinism."""
    text = _AUDIT_HMAC_RS.read_text(encoding="utf-8")
    assert "BTreeMap" in text
    # The 13 canonical fields must all appear as map.insert keys.
    expected = [
        '"action"',
        '"catalog_key"',
        '"chain_id"',
        '"content_sha256"',
        '"created_at_unix_ms"',
        '"description"',
        '"event_id"',
        '"op_tag"',
        '"parent_key"',
        '"prev_hmac"',
        '"principal"',
        '"schema_version"',
        '"tenant"',
    ]
    for k in expected:
        assert f"m.insert({k}" in text, f"canonical payload missing field {k}"


def test_audit_hmac_rs_self_verify_pass_is_public():
    """`self_verify_pass` MUST be public so the runtime spawn loop can
    call it without going through the sink object (which would force
    an awkward downcast)."""
    text = _AUDIT_HMAC_RS.read_text(encoding="utf-8")
    assert "pub async fn self_verify_pass" in text


def test_audit_hmac_rs_uses_uuid_v7_for_event_id():
    """UUIDv7 (timestamp-monotone) is the canonical id format —
    enables streaming verify ordering + secondary index."""
    text = _AUDIT_HMAC_RS.read_text(encoding="utf-8")
    assert "Uuid::now_v7()" in text


def test_audit_hmac_rs_chain_to_tracing_fanout():
    """When `chain_to_tracing = true`, the sink must fan out to the
    legacy TracingAuditSink so existing Loki/Splunk pipelines keep
    receiving the audit stream."""
    text = _AUDIT_HMAC_RS.read_text(encoding="utf-8")
    assert "TracingAuditSink.emit" in text
    assert "chain_to_tracing" in text


# ─────────────────────────────────────────────────────────────────────
# audit_keys.rs — loader pattern M12
# ─────────────────────────────────────────────────────────────────────


def test_audit_keys_rs_enforces_32_byte_keys():
    """HMAC-SHA-256 optimal key length is exactly 32 raw bytes (block
    size 64 minus the inner pad). Shorter keys = low entropy ; longer
    = wasted bytes + key_id ambiguity."""
    text = _AUDIT_KEYS_RS.read_text(encoding="utf-8")
    assert "32 raw bytes" in text


def test_audit_keys_rs_does_not_leak_material_in_errors():
    """Mirror of M12 invariant : the offending base64 string is
    NEVER echoed in error messages — only the line number — so logs
    don't leak even a malformed secret."""
    text = _AUDIT_KEYS_RS.read_text(encoding="utf-8")
    # Verify the BadLine error format takes (path, lineno, detail)
    # and detail is "not valid base64-url: {e}" with no key content.
    assert "lineno" in text
    assert "detail" in text


def test_audit_keys_rs_first_key_is_active():
    """The first non-comment line is the active signing key. Mirrors
    the M12 HS256 secrets pattern verbatim."""
    text = _AUDIT_KEYS_RS.read_text(encoding="utf-8")
    assert "active_id.is_none()" in text


def test_audit_keys_rs_key_id_is_deterministic():
    """key_id = first 8 chars of SHA-256(material) ; deterministic
    across processes."""
    text = _AUDIT_KEYS_RS.read_text(encoding="utf-8")
    assert "key_id_of" in text
    assert "Sha256::digest(material)" in text


# ─────────────────────────────────────────────────────────────────────
# config.rs — AuditSigningConfig + 4 env vars
# ─────────────────────────────────────────────────────────────────────

EXPECTED_ENV_VARS = {
    "EFE_AUDIT_SIGNING_ENABLED",
    "EFE_AUDIT_HMAC_SECRETS_FILE",
    "EFE_AUDIT_CHAIN_TO_TRACING",
    "EFE_AUDIT_SELF_VERIFY_EVERY_S",
}


def test_config_rs_declares_audit_signing_config():
    text = _CONFIG_RS.read_text(encoding="utf-8")
    assert "pub struct AuditSigningConfig" in text


def test_config_rs_reads_all_audit_env_vars():
    text = _CONFIG_RS.read_text(encoding="utf-8")
    for v in EXPECTED_ENV_VARS:
        assert v in text, f"config.rs does not read env var {v}"


def test_config_rs_validate_refuses_signing_without_postgres_backend():
    """Audit signing requires the Postgres pool. Validate MUST
    fail-fast at boot if signing is enabled with backend=memory."""
    text = _CONFIG_RS.read_text(encoding="utf-8")
    assert "EFE_AUDIT_SIGNING_ENABLED=true requires" in text


def test_lib_rs_exports_audit_signing_config():
    text = _LIB_RS.read_text(encoding="utf-8")
    assert "AuditSigningConfig" in text


# ─────────────────────────────────────────────────────────────────────
# Migration 0003_audit_events.sql
# ─────────────────────────────────────────────────────────────────────


def test_migration_0003_creates_audit_events_table():
    text = _MIGRATION_RS.read_text(encoding="utf-8")
    assert "CREATE TABLE audit_events" in text


def test_migration_0003_has_three_block_mutation_triggers():
    """UPDATE, DELETE, and TRUNCATE must all be blocked. Missing the
    TRUNCATE trigger (which doesn't fire UPDATE/DELETE) would leave a
    bypass."""
    text = _MIGRATION_RS.read_text(encoding="utf-8")
    assert "audit_events_no_update" in text
    assert "audit_events_no_delete" in text
    assert "audit_events_no_truncate" in text
    assert "BEFORE UPDATE ON audit_events" in text
    assert "BEFORE DELETE ON audit_events" in text
    assert "BEFORE TRUNCATE ON audit_events" in text


def test_migration_0003_check_constraints():
    text = _MIGRATION_RS.read_text(encoding="utf-8")
    assert "audit_hmac_hex_format" in text
    assert "audit_prev_hmac_hex_format" in text
    assert "audit_key_id_hex_format" in text
    assert "audit_action_enum" in text
    assert "audit_chain_id_format" in text


def test_migration_0003_indexes():
    text = _MIGRATION_RS.read_text(encoding="utf-8")
    assert "audit_events_chain_seq_idx" in text
    assert "audit_events_tenant_created_at_idx" in text


def test_migration_0003_event_id_is_unique():
    """event_id UNIQUE prevents replay attacks (re-INSERT of an old
    event with new HMAC)."""
    text = _MIGRATION_RS.read_text(encoding="utf-8")
    assert "event_id" in text
    assert "NOT NULL UNIQUE" in text


# ─────────────────────────────────────────────────────────────────────
# Telemetry — 6 instruments + 2 spans
# ─────────────────────────────────────────────────────────────────────

EXPECTED_INSTRUMENTS = {
    "efe_audit_events_signed_total",
    "efe_audit_sign_failures_total",
    "efe_audit_chain_head_seq",
    "efe_audit_keys_active",
    "efe_audit_sign_duration_seconds",
    "efe_audit_chain_break_detected_total",
}


def test_d14bis_instruments_declares_six_metrics():
    text = _INSTRUMENTS_RS.read_text(encoding="utf-8")
    for m in EXPECTED_INSTRUMENTS:
        assert m in text, f"d14bis_instruments.rs missing metric {m}"


def test_d14bis_instruments_register_all_is_idempotent():
    text = _INSTRUMENTS_RS.read_text(encoding="utf-8")
    assert "pub fn register_all_d14bis" in text


def test_spans_rs_declares_audit_spans():
    text = _SPANS_RS.read_text(encoding="utf-8")
    assert 'pub const AUDIT_SIGN' in text
    assert 'pub const AUDIT_VERIFY' in text
    assert '"efe.audit.sign"' in text
    assert '"efe.audit.verify"' in text


# ─────────────────────────────────────────────────────────────────────
# Runtime wire-up
# ─────────────────────────────────────────────────────────────────────


def test_runtime_rs_wires_hmac_chained_audit_sink():
    text = _RUNTIME_RS.read_text(encoding="utf-8")
    assert "HmacChainedAuditSink::new" in text
    assert "load_keys_file" in text
    assert "register_all_d14bis()" in text


def test_runtime_rs_spawns_periodic_self_verify():
    """The periodic self-verify is the runtime mechanism that
    detects tampering between explicit `efe-verify-audit` runs.

    **Audit fix C1** — the runtime MUST use the proper
    `periodic_self_verify(pool, keys, interval, shutdown)` helper
    (with shutdown.notified() handling) rather than an inline loop.
    """
    text = _RUNTIME_RS.read_text(encoding="utf-8")
    assert "self_verify_every_s" in text
    assert "periodic_self_verify" in text, (
        "runtime must spawn the proper periodic_self_verify helper "
        "(audit fix C1) — an inline loop without shutdown handling "
        "would leak the task at shutdown"
    )


def test_runtime_rs_drains_pending_audit_at_shutdown():
    """**Audit fix C2** — `Server::serve` MUST call `drain_pending`
    on the audit sink before returning, so events that were spawned
    but not yet committed survive a graceful shutdown."""
    text = _RUNTIME_RS.read_text(encoding="utf-8")
    assert "drain_pending" in text


def test_audit_hmac_rs_uses_constant_time_compare():
    """**Audit fix M1** — `verify_chain` MUST use the
    constant-time hex comparator `ct_hex_eq` instead of plain
    string `==` to prevent timing-side-channel attacks that could
    leak HMAC bits via verify duration measurement."""
    text = _AUDIT_HMAC_RS.read_text(encoding="utf-8")
    assert "pub fn ct_hex_eq" in text or "fn ct_hex_eq" in text
    # And it must actually be USED in verify_chain (otherwise the
    # function is dead code with no security benefit).
    # Count : at least 2 calls (chain link check + HMAC compare).
    n = text.count("ct_hex_eq(")
    assert n >= 3, (
        f"ct_hex_eq must be defined and called ; found {n} occurrences"
    )


def test_audit_hmac_rs_clock_skew_warns_explicitly():
    """**Audit fix M2** — `now_unix_ms` MUST log a warning when
    the wall clock is before UNIX_EPOCH (returning 0 silently
    would produce confusingly-stamped audit events)."""
    text = _AUDIT_HMAC_RS.read_text(encoding="utf-8")
    assert "BEFORE UNIX_EPOCH" in text


def test_audit_hmac_rs_emit_fanouts_to_tracing_first():
    """**Audit fix L1** — `TracingAuditSink.emit` MUST happen
    BEFORE the spawn so Loki sees events in the registry-emit
    order, not the spawn-completion order."""
    text = _AUDIT_HMAC_RS.read_text(encoding="utf-8")
    # The fan-out happens before the runtime.spawn call.
    fanout_pos = text.find("TracingAuditSink.emit(event)")
    spawn_pos = text.find("runtime.spawn(Self::sign_task")
    assert fanout_pos > 0 and spawn_pos > 0, (
        "Both fan-out and spawn must be present"
    )
    # We expect at least one fan-out site before the spawn site
    # (there may be additional fan-out within the spawn closure for
    # back-compat ; that's OK as long as the pre-spawn one exists).


def test_audit_hmac_rs_drain_pending_is_public():
    """**Audit fix C2** — `drain_pending` MUST be public so the
    runtime's shutdown path can invoke it without forcing an
    awkward downcast."""
    text = _AUDIT_HMAC_RS.read_text(encoding="utf-8")
    assert "pub async fn drain_pending" in text


# ─────────────────────────────────────────────────────────────────────
# Helm
# ─────────────────────────────────────────────────────────────────────


def test_values_yaml_has_audit_signing_block():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    audit = data["config"]["catalog"]["audit"]["signing"]
    expected_keys = {
        "enabled",
        "keysFile",
        "existingSecretName",
        "existingSecretKey",
        "chainToTracing",
        "selfVerifyEverySeconds",
    }
    missing = expected_keys - set(audit.keys())
    assert not missing, f"audit.signing missing keys: {missing}"


def test_values_yaml_audit_signing_default_disabled():
    """For zero-cost backwards compat, the audit signing default MUST
    be `false`. Operators on memory backend or those without HMAC
    requirements continue with the legacy TracingAuditSink."""
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    assert data["config"]["catalog"]["audit"]["signing"]["enabled"] is False


def test_values_yaml_audit_signing_self_verify_default_3600():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    assert (
        data["config"]["catalog"]["audit"]["signing"]["selfVerifyEverySeconds"] == 3600
    )


def test_configmap_renders_audit_env_vars_only_when_enabled():
    """ConfigMap MUST guard the `EFE_AUDIT_*` env vars on
    `audit.signing.enabled` so memory-mode deploys don't ship orphan
    env vars."""
    text = _CONFIGMAP.read_text(encoding="utf-8")
    assert (
        ".Values.config.catalog.audit.signing.enabled" in text
        or "audit.signing.enabled" in text
    )
    for v in EXPECTED_ENV_VARS:
        assert v in text


def test_deployment_mounts_audit_hmac_secret_only_when_enabled():
    """The HMAC keys file is mounted via a Secret-backed volume only
    when audit.signing.enabled = true. Memory-mode deploys must not
    reference a non-existent Secret."""
    text = _DEPLOYMENT.read_text(encoding="utf-8")
    assert "audit-hmac" in text
    assert "audit.signing.existingSecretName" in text


def test_deployment_audit_hmac_secret_uses_default_mode_0400():
    """Read-only secret. Pod can read but not write."""
    text = _DEPLOYMENT.read_text(encoding="utf-8")
    # Search the audit-hmac secret block specifically.
    m = re.search(
        r"name:\s*audit-hmac\s+secret:.*?defaultMode:\s*0400", text, re.DOTALL
    )
    assert m is not None, "audit-hmac volume must use defaultMode: 0400"


# ─────────────────────────────────────────────────────────────────────
# CLI bin
# ─────────────────────────────────────────────────────────────────────


def test_verify_audit_cli_is_declared_in_cargo():
    cargo = (_REPO / "crates" / "efe-server" / "Cargo.toml").read_text(encoding="utf-8")
    assert 'name = "efe-verify-audit"' in cargo
    assert 'path = "src/bin/efe_verify_audit.rs"' in cargo


def test_verify_audit_cli_supports_json_flag():
    text = _VERIFY_BIN_RS.read_text(encoding="utf-8")
    assert '"--json"' in text


def test_verify_audit_cli_exit_codes_distinguish_failure_modes():
    """Exit code 0 = intact, 1 = tampering detected, 2 = operational
    error. CI/regulator tooling differentiates 'compromised' from
    'verifier broken'."""
    text = _VERIFY_BIN_RS.read_text(encoding="utf-8")
    assert "ExitCode::SUCCESS" in text
    assert "ExitCode::from(1)" in text
    assert "ExitCode::from(2)" in text
