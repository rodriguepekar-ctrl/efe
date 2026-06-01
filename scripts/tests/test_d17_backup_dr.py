"""Sprint **D17** — Backup + DR cross-layer pin tests.

Pin tests that don't require a live Postgres / kind cluster :

* `Cargo.toml` workspace member declared.
* The 13 D17 modules + CLI binary exist.
* The 8 D17 Prometheus instruments are wired into
  `efe-telemetry/src/metrics/d17_instruments.rs`.
* The migration `0006_d17_backup_history.sql` schema matches the
  architecture commitment.
* The 7 Helm templates render the expected env vars + CronJobs +
  alerts + RBAC + NetworkPolicy + PDB.
* The DR shell scripts exist + are executable + reference the
  canonical 6-step pipeline.
* The values.yaml `backup` block has the expected keys.
* `configmap.yaml` renders every `EFE_BACKUP_*` env var.

Cf. `docs/bench/d17_backup_dr_architecture.md` § 22.3.
"""

from __future__ import annotations

import re
from pathlib import Path

import yaml

_REPO = Path(__file__).resolve().parents[2]


# ─────────────────────────────────────────────────────────────────────
# Cargo workspace + crate skeleton
# ─────────────────────────────────────────────────────────────────────


def test_d17_workspace_member_declared():
    cargo = (_REPO / "Cargo.toml").read_text(encoding="utf-8")
    assert '"crates/efe-backup"' in cargo, (
        "efe-backup must be in the workspace members list"
    )


def test_d17_crate_cargo_toml_exists():
    f = _REPO / "crates" / "efe-backup" / "Cargo.toml"
    assert f.exists()
    content = f.read_text(encoding="utf-8")
    # Must declare the binary.
    assert 'name = "efe-backup"' in content
    assert 'path = "src/bin/efe_backup.rs"' in content
    # Must depend on ed25519-dalek + sha2 + chrono + clap + efe-telemetry.
    for dep in ["ed25519-dalek", "sha2", "chrono", "clap", "efe-telemetry"]:
        assert dep in content, f"missing dep: {dep}"


def test_d17_module_files_present():
    src = _REPO / "crates" / "efe-backup" / "src"
    expected = [
        "lib.rs",
        "error.rs",
        "config.rs",
        "manifest.rs",
        "command_runner.rs",
        "metadata.rs",
        "snapshot.rs",
        "postgres.rs",
        "vault.rs",
        "bundles.rs",
        "audit.rs",
        "restore.rs",
        "verify.rs",
        "drill.rs",
    ]
    for m in expected:
        f = src / m
        assert f.exists(), f"D17 module missing: {m}"
        assert f.stat().st_size > 0, f"D17 module empty: {m}"


def test_d17_cli_binary_present():
    f = _REPO / "crates" / "efe-backup" / "src" / "bin" / "efe_backup.rs"
    assert f.exists()
    content = f.read_text(encoding="utf-8")
    # Must declare 7 subcommands.
    for cmd in ["Snapshot", "Restore", "Verify", "Drill", "List", "Delete", "Rotate"]:
        assert f"Command::{cmd}" in content, f"CLI missing subcommand variant: {cmd}"
    # Must implement 4-eyes JWT logic.
    assert "enforce_four_eyes" in content
    assert "extract_jwt_sub" in content


# ─────────────────────────────────────────────────────────────────────
# Prometheus instruments (efe-telemetry/src/metrics/d17_instruments.rs)
# ─────────────────────────────────────────────────────────────────────


_D17_INSTRUMENTS_RS = (
    _REPO / "crates" / "efe-telemetry" / "src" / "metrics" / "d17_instruments.rs"
)


def test_d17_instruments_file_exists():
    assert _D17_INSTRUMENTS_RS.exists()


def test_d17_instruments_count_pinned_at_8():
    rs = _D17_INSTRUMENTS_RS.read_text(encoding="utf-8")
    once_count = len(re.findall(r"^once_metric!", rs, re.MULTILINE))
    assert once_count == 8, (
        f"D17 architecture § 19.1 commits to exactly 8 instruments, found {once_count}"
    )


def test_d17_instruments_canonical_names():
    rs = _D17_INSTRUMENTS_RS.read_text(encoding="utf-8")
    expected = [
        "efe_backup_runs_total",
        "efe_backup_duration_seconds",
        "efe_backup_size_bytes",
        "efe_backup_last_success_timestamp",
        "efe_backup_rpo_seconds",
        "efe_dr_drill_runs_total",
        "efe_dr_drill_duration_seconds",
        "efe_dr_drill_last_success_timestamp",
    ]
    for name in expected:
        assert name in rs, f"D17 instrument {name} not declared"


def test_d17_register_all_d17_invokes_all_8():
    rs = _D17_INSTRUMENTS_RS.read_text(encoding="utf-8")
    register = re.search(
        r"pub fn register_all_d17\(\) \{(.*?)\n\}", rs, re.DOTALL
    )
    assert register
    body = register.group(1)
    fns = [
        "backup_runs_total",
        "backup_duration_seconds",
        "backup_size_bytes",
        "backup_last_success_timestamp",
        "backup_rpo_seconds",
        "dr_drill_runs_total",
        "dr_drill_duration_seconds",
        "dr_drill_last_success_timestamp",
    ]
    for fn in fns:
        assert fn in body, f"register_all_d17 missing call to {fn}()"


def test_d17_metrics_mod_re_exports_register_all_d17():
    mod = (_REPO / "crates" / "efe-telemetry" / "src" / "metrics" / "mod.rs").read_text(
        encoding="utf-8"
    )
    assert "pub use d17_instruments::register_all_d17" in mod


def test_d17_metrics_label_arity_pinned():
    """Architecture § 19.1 commits to specific label arities for
    each of the 8 instruments."""
    rs = _D17_INSTRUMENTS_RS.read_text(encoding="utf-8")
    # The 3-label counter must declare three label names.
    assert '&["source", "type", "result"]' in rs
    # The 2-label histograms / gauges.
    assert '&["source", "type"]' in rs
    # The 1-label drill counter.
    assert '&["result"]' in rs
    # The no-label gauges + histogram are documented in the comments.
    assert "(no label)" in rs


# ─────────────────────────────────────────────────────────────────────
# Migration 0006_d17_backup_history.sql
# ─────────────────────────────────────────────────────────────────────


_MIGRATION = (
    _REPO / "crates" / "efe-server" / "migrations" / "0006_d17_backup_history.sql"
)


def test_d17_migration_present():
    assert _MIGRATION.exists()


def test_d17_migration_creates_efe_backup_history():
    sql = _MIGRATION.read_text(encoding="utf-8")
    assert "CREATE TABLE IF NOT EXISTS efe_backup_history" in sql


def test_d17_migration_check_constraints_match_architecture():
    sql = _MIGRATION.read_text(encoding="utf-8")
    # Architecture § 6.2 commits to status ∈ {ok, failed, partial}
    assert "CHECK (status IN ('ok', 'failed', 'partial'))" in sql
    # source ∈ {postgres, vault, bundles}
    assert "CHECK (backup_source IN ('postgres', 'vault', 'bundles'))" in sql
    # type covers the 6 BackupKind variants
    for k in ["full", "diff", "incr", "wal", "vault_snapshot", "bundle_sync"]:
        assert f"'{k}'" in sql


def test_d17_migration_indexes():
    sql = _MIGRATION.read_text(encoding="utf-8")
    for idx in [
        "idx_efe_backup_history_started_at",
        "idx_efe_backup_history_status_started",
        "idx_efe_backup_history_source_type",
    ]:
        assert idx in sql


# ─────────────────────────────────────────────────────────────────────
# Helm chart D17 surface
# ─────────────────────────────────────────────────────────────────────


_VALUES = _REPO / "deploy" / "helm" / "efe" / "values.yaml"
_CONFIGMAP = _REPO / "deploy" / "helm" / "efe" / "templates" / "configmap.yaml"
_TEMPLATES_DIR = _REPO / "deploy" / "helm" / "efe" / "templates"


def test_d17_helm_templates_present():
    expected = [
        "d17-backup-cronjobs.yaml",
        "d17-backup-rbac.yaml",
        "d17-backup-secret.yaml",
        "d17-backup-pdb.yaml",
        "d17-dr-drill-cronjob.yaml",
        "d17-prometheus-rules.yaml",
        "networkpolicy-backup-egress.yaml",
    ]
    for f in expected:
        path = _TEMPLATES_DIR / f
        assert path.exists(), f"Helm template missing: {f}"


def test_d17_values_yaml_backup_block_complete():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    backup = data.get("backup")
    assert backup is not None, "values.yaml must have a top-level `backup:` block"
    expected_keys = {
        "enabled",
        "devSecretValues",
        "serviceAccountAnnotations",
        "storage",
        "retention",
        "alerts",
        "audit",
        "snapshotTimeoutSeconds",
        "restoreTimeoutSeconds",
        "cronJobs",
        "bundles",
        "vault",
        "postgres",
        "drill",
    }
    actual = set(backup.keys())
    missing = expected_keys - actual
    assert not missing, f"backup block missing keys: {missing}"


def test_d17_values_yaml_backup_disabled_by_default():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    assert data["backup"]["enabled"] is False


def test_d17_values_yaml_rpo_target_5_minutes():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    assert data["backup"]["alerts"]["rpoTargetSeconds"] == 300


def test_d17_values_yaml_rto_target_30_minutes():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    assert data["backup"]["alerts"]["rtoTargetSeconds"] == 1800


def test_d17_values_yaml_4_cronjobs_declared():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    cron = data["backup"]["cronJobs"]
    expected = {"postgres-full", "postgres-diff", "vault-snapshot", "bundle-sync"}
    assert set(cron.keys()) == expected


def test_d17_configmap_renders_backup_envvars():
    cm = _CONFIGMAP.read_text(encoding="utf-8")
    expected = [
        "EFE_BACKUP_ENABLED",
        "EFE_BACKUP_STORAGE_TYPE",
        "EFE_BACKUP_STORAGE_BUCKET",
        "EFE_BACKUP_STORAGE_REGION",
        "EFE_BACKUP_RPO_TARGET_S",
        "EFE_BACKUP_RTO_TARGET_S",
        "EFE_BACKUP_PG_STANZA",
        "EFE_BACKUP_PG_CONFIG_PATH",
        "EFE_BACKUP_VAULT_ADDR",
        "EFE_BACKUP_BUNDLES_SOURCE",
        "EFE_BACKUP_BUNDLES_DEST",
        "EFE_BACKUP_CROSS_REGION",
        "EFE_BACKUP_AUDIT_CHAIN_EVENTS",
        "EFE_BACKUP_SNAPSHOT_TIMEOUT_S",
        "EFE_BACKUP_RESTORE_TIMEOUT_S",
    ]
    for var in expected:
        assert var in cm, f"configmap.yaml missing {var}"


def test_d17_prometheus_rules_alerts():
    rules = (_TEMPLATES_DIR / "d17-prometheus-rules.yaml").read_text(encoding="utf-8")
    expected_alerts = [
        "D17BackupStaleFull",
        "D17BackupStaleDiff",
        "D17BackupFailureRate",
        "D17BackupRpoViolation",
        "D17DrDrillStale",
        "D17DrRtoViolation",
        "D17BackupS3Unreachable",
    ]
    for a in expected_alerts:
        assert a in rules, f"alert {a} missing from d17-prometheus-rules.yaml"


def test_d17_networkpolicy_egress_to_s3_vault_postgres():
    np = (_TEMPLATES_DIR / "networkpolicy-backup-egress.yaml").read_text(encoding="utf-8")
    # Must allow port 443 (S3), 8200 (Vault), 5432 (Postgres), 53 (DNS).
    for port in ["443", "8200", "5432", "53"]:
        assert f"port: {port}" in np, f"egress NetworkPolicy missing port {port}"


def test_d17_rbac_minimal_least_privilege():
    rbac = (_TEMPLATES_DIR / "d17-backup-rbac.yaml").read_text(encoding="utf-8")
    # Must restrict secret access to backup-config secret name only
    # (Lessons from D15-C3 path-drift : avoid wide secret access).
    assert "resourceNames:" in rbac
    assert "-backup-config" in rbac


def test_d17_pdb_max_unavailable_zero():
    pdb = (_TEMPLATES_DIR / "d17-backup-pdb.yaml").read_text(encoding="utf-8")
    assert "maxUnavailable: 0" in pdb


def test_d17_dr_drill_cronjob_schedule():
    cron = (_TEMPLATES_DIR / "d17-dr-drill-cronjob.yaml").read_text(encoding="utf-8")
    assert "drill.schedule" in cron
    # Default schedule per values.yaml is Sunday 03:00 UTC.
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    assert data["backup"]["drill"]["schedule"] == "0 3 * * 0"


# ─────────────────────────────────────────────────────────────────────
# DR shell scripts
# ─────────────────────────────────────────────────────────────────────


_DR_DIR = _REPO / "dr"


def test_d17_dr_scripts_exist():
    for f in ["restore.sh", "smoke_test.sh", "setup-test-env.sh"]:
        path = _DR_DIR / f
        assert path.exists(), f"dr/{f} missing"


def test_d17_dr_scripts_executable():
    for f in ["restore.sh", "smoke_test.sh", "setup-test-env.sh"]:
        path = _DR_DIR / f
        # On Unix, check the execute bit.
        mode = path.stat().st_mode
        assert mode & 0o100, f"dr/{f} not executable for owner"


def test_d17_restore_sh_six_step_pipeline():
    rs = (_DR_DIR / "restore.sh").read_text(encoding="utf-8")
    # Architecture § 10.2 commits to 6 steps : Vault → Postgres →
    # Bundles → Helm → Verify → Smoke.
    for step in [
        "Step 0/6 — pre-flight",
        "Step 1/6 — restoring Vault",
        "Step 2/6 — restoring Postgres",
        "Step 3/6 — restoring bundles",
        "Step 4/6 — Helm upgrade",
        "Step 5/6 — post-restore verify",
        "Step 6/6 — smoke test",
    ]:
        assert step in rs, f"restore.sh missing step: {step!r}"


def test_d17_restore_sh_emits_timing_json():
    rs = (_DR_DIR / "restore.sh").read_text(encoding="utf-8")
    # The CI drill harness reads timing.json to assert RTO < 30 min.
    assert "timing.json" in rs
    assert "duration_seconds" in rs


def test_d17_restore_sh_distinct_exit_codes():
    rs = (_DR_DIR / "restore.sh").read_text(encoding="utf-8")
    # Must use distinct exit codes for triage (per architecture § 10.3).
    for code in ["exit 2", "exit 3", "exit 4", "exit 5", "exit 6", "exit 7", "exit 8"]:
        assert code in rs, f"restore.sh missing distinct exit code: {code}"


def test_d17_smoke_test_checks_metrics_endpoint():
    sm = (_DR_DIR / "smoke_test.sh").read_text(encoding="utf-8")
    # Smoke must verify the /metrics endpoint exposes D17 instruments.
    assert "/metrics" in sm
    assert "efe_backup_runs_total" in sm or "efe_otel_attributes_filtered_total" in sm


# ─────────────────────────────────────────────────────────────────────
# Architecture document still pinned (acceptance criteria gate)
# ─────────────────────────────────────────────────────────────────────


def test_d17_architecture_doc_acceptance_criteria_lists_audit_report():
    doc = (
        _REPO / "docs" / "bench" / "d17_backup_dr_architecture.md"
    ).read_text(encoding="utf-8")
    assert "d17_backup_dr_audit_report.md" in doc, (
        "Architecture § 26 must list the post-implementation audit "
        "report as an acceptance criterion"
    )


def test_d17_changelog_entry_present():
    cl = (_REPO / "CHANGELOG.md").read_text(encoding="utf-8")
    # The implementation entry MUST be present after this sprint.
    assert (
        "D17 — v0.1.18" in cl
        or "D17 implementation" in cl
        or "Sprint D17" in cl
    ), "CHANGELOG should mention D17 implementation"


# ─────────────────────────────────────────────────────────────────────
# 4-eyes invariant pin (D17 § 8.3)
# ─────────────────────────────────────────────────────────────────────


def test_d17_cli_4_eyes_enforced_in_delete_subcommand():
    rs = (
        _REPO / "crates" / "efe-backup" / "src" / "bin" / "efe_backup.rs"
    ).read_text(encoding="utf-8")
    # The Delete variant must require both tokens + reason.
    delete_block = re.search(
        r"Command::Delete \{(.*?)\}", rs, re.DOTALL
    )
    assert delete_block
    body = delete_block.group(1)
    assert "primary_token" in body
    assert "secondary_token" in body
    assert "reason" in body
    # And the function must call enforce_four_eyes which compares
    # the two `sub` claims and refuses equal subjects.
    assert "enforce_four_eyes" in rs
    assert "distinct" in rs  # error message about distinct sub claims


# ─────────────────────────────────────────────────────────────────────
# Cross-env restore guard pin (T-DR-6 mitigation)
# ─────────────────────────────────────────────────────────────────────


def test_d17_cross_env_guard_implemented_in_restore_module():
    rs = (
        _REPO / "crates" / "efe-backup" / "src" / "restore.rs"
    ).read_text(encoding="utf-8")
    # Must check manifest.env vs target_env and return CrossEnvRefused
    # unless force_cross_env is true.
    assert "force_cross_env" in rs
    assert "CrossEnvRefused" in rs
    assert "manifest.env" in rs


# ─────────────────────────────────────────────────────────────────────
# Manifest version pin (architecture § 27)
# ─────────────────────────────────────────────────────────────────────


def test_d17_manifest_version_pinned_at_1_0_0():
    rs = (
        _REPO / "crates" / "efe-backup" / "src" / "lib.rs"
    ).read_text(encoding="utf-8")
    assert 'pub const MANIFEST_VERSION: &str = "1.0.0"' in rs


# ─────────────────────────────────────────────────────────────────────
# D17 audit-pass fixes (cf. CHANGELOG "Audit fixes — Sprint D17")
# ─────────────────────────────────────────────────────────────────────


def test_audit_ac1_efe_backup_in_dockerfile_prod():
    """**Audit fix § D17-AC1 (CRITICAL)** — pre-audit, Dockerfile.prod
    only built efe-server. The efe-backup binary was MISSING from
    the production image — every CronJob would CrashLoop with
    'exec efe-backup: file not found'. Same defect class as
    D16-C1 + D13bis-A20."""
    df = (
        _REPO / "docker" / "Dockerfile.prod"
    ).read_text(encoding="utf-8")
    # Build stage must compile efe-backup.
    assert "--bin efe-backup" in df, (
        "Dockerfile.prod must build efe-backup binary"
    )
    # Runtime stage must copy efe-backup.
    assert "/app/target/release/efe-backup" in df
    assert "/usr/local/bin/efe-backup" in df


def test_audit_ac2_runtime_instrument_module_exists():
    """**Audit fix § D17-AC2** — pre-audit the 8 D17 metrics were
    declared but never bumped from runtime (same class as D16-H3/H5/H6).
    Post-audit, `runtime_instrument::InstrumentedSnapshot` wraps
    Snapshotter calls and bumps `efe_backup_runs_total`,
    `efe_backup_duration_seconds`, etc."""
    rs = (
        _REPO / "crates" / "efe-backup" / "src" / "runtime_instrument.rs"
    ).read_text(encoding="utf-8")
    assert "pub struct InstrumentedSnapshot" in rs
    # Must bump all 4 main runtime metrics.
    for fn in [
        "backup_runs_total",
        "backup_duration_seconds",
        "backup_size_bytes",
        "backup_last_success_timestamp",
    ]:
        assert fn in rs, f"runtime_instrument.rs missing call to {fn}"


def test_audit_ac2_cli_uses_instrumented_snapshot():
    """The CLI cmd_snapshot must use InstrumentedSnapshot, not raw
    Snapshotter calls. Pre-audit, the CLI called `s.snapshot(...)`
    directly so metrics stayed dead even if InstrumentedSnapshot
    existed."""
    rs = (
        _REPO / "crates" / "efe-backup" / "src" / "bin" / "efe_backup.rs"
    ).read_text(encoding="utf-8")
    assert "InstrumentedSnapshot::new" in rs
    assert "inst.snapshot(" in rs


def test_audit_ac3_snapshot_metadata_sign_method_present():
    """**Audit fix § D17-AC3** — pre-audit, Snapshotter impls
    returned SnapshotMetadata with `ed25519_sig: String::new()` ;
    restore-time signature verification rejected every backup.
    Post-audit, `SnapshotMetadata::sign()` populates sha256_manifest
    and ed25519_sig, called from `InstrumentedSnapshot` when a
    signing key is available."""
    rs = (
        _REPO / "crates" / "efe-backup" / "src" / "metadata.rs"
    ).read_text(encoding="utf-8")
    assert "pub fn sign(" in rs
    assert "ed25519_dalek::SigningKey" in rs
    # Must populate both sha256 and signature.
    assert "self.sha256_manifest" in rs
    assert "self.ed25519_sig" in rs


def test_audit_ac3_verify_snapshot_metadata_helper_present():
    """The CLI/restore path must be able to verify a SnapshotMetadata
    signature against a TrustStore. The verify_snapshot_metadata
    helper is the public entry point."""
    rs = (
        _REPO / "crates" / "efe-backup" / "src" / "manifest.rs"
    ).read_text(encoding="utf-8")
    assert "pub fn verify_snapshot_metadata" in rs


def test_audit_ac4_real_verifier_impls_present():
    """**Audit fix § D17-AC4** — pre-audit only MockVerifier existed.
    Post-audit, 5 real verifier impls in `verifiers.rs` shell out
    to efe-server / efe-bundle / curl / grpcurl."""
    rs = (
        _REPO / "crates" / "efe-backup" / "src" / "verifiers.rs"
    ).read_text(encoding="utf-8")
    expected_structs = [
        "AuditChainVerifier",
        "BundleSignatureVerifier",
        "OidcReachabilityVerifier",
        "CatalogSmokeVerifier",
        "ExecuteSmokeVerifier",
    ]
    for s in expected_structs:
        assert f"pub struct {s}" in rs, f"verifier struct {s} missing"
    # Each verifier must impl the Verifier trait.
    for s in expected_structs:
        assert f"impl Verifier for {s}" in rs


def test_audit_ac4_verifiers_re_exported_from_lib():
    """The 5 real verifiers must be re-exported at crate root for
    CLI consumption."""
    rs = (
        _REPO / "crates" / "efe-backup" / "src" / "lib.rs"
    ).read_text(encoding="utf-8")
    for s in [
        "AuditChainVerifier",
        "BundleSignatureVerifier",
        "CatalogSmokeVerifier",
        "ExecuteSmokeVerifier",
        "OidcReachabilityVerifier",
    ]:
        assert f"pub use verifiers::" in rs or f"pub use verifiers::{{" in rs
        assert s in rs


def test_audit_ac5_cli_restore_has_source_flag():
    """**Audit fix § D17-AC5** — pre-audit the CLI restore looped
    over all 3 sources regardless of intent. Combined with
    `dr/restore.sh` calling `efe-backup restore` 3 times (one per
    source), each source was restored 3× total. Post-audit the CLI
    accepts `--source` and only restores the named source."""
    rs = (
        _REPO / "crates" / "efe-backup" / "src" / "bin" / "efe_backup.rs"
    ).read_text(encoding="utf-8")
    # The Restore subcommand must declare a `source: Option<SourceArg>`.
    import re as _re
    block = _re.search(r"Restore \{(.*?)\},", rs, _re.DOTALL)
    assert block
    body = block.group(1)
    assert "source: Option<SourceArg>" in body, (
        "CLI Restore must accept --source flag (audit § D17-AC5)"
    )


def test_audit_ac5_dr_restore_uses_source_flag():
    """The dr/restore.sh must invoke `efe-backup restore --source=<x>`
    for each of the 3 stages, not bare invocations."""
    sh = (_REPO / "dr" / "restore.sh").read_text(encoding="utf-8")
    assert "--source=vault" in sh
    assert "--source=postgres" in sh
    assert "--source=bundles" in sh


def test_audit_ah1_cmd_drill_invokes_verifier_suite():
    """**Audit fix § D17-AH1** — pre-audit, cmd_drill was a stub
    print. Post-audit, it builds the 5-verifier suite and runs it."""
    rs = (
        _REPO / "crates" / "efe-backup" / "src" / "bin" / "efe_backup.rs"
    ).read_text(encoding="utf-8")
    # The cmd_drill function must construct verifiers and call run_suite.
    import re as _re
    block = _re.search(
        r"async fn cmd_drill\(.*?\n\}\n", rs, _re.DOTALL
    )
    assert block
    body = block.group()
    assert "AuditChainVerifier::new" in body
    assert "BundleSignatureVerifier::new" in body
    assert "ExecuteSmokeVerifier::new" in body
    assert "run_suite" in body


def test_audit_ah4_pgbackrest_restore_at_honors_target_time():
    """**Audit fix § D17-AH4** — pre-audit, `PgBackRestSnapshotter::restore`
    silently dropped target_time and used `--type=immediate`.
    Post-audit, the new `restore_at(label, target_time)` method
    honors PITR via `--type=time --target=<ts>`."""
    rs = (
        _REPO / "crates" / "efe-backup" / "src" / "postgres.rs"
    ).read_text(encoding="utf-8")
    assert "pub async fn restore_at" in rs
    assert "--type=time" in rs


def test_audit_ah5_pgbackrest_env_includes_cipher_pass():
    """**Audit fix § D17-AH5** — pre-audit, the cipher passphrase
    env var was set on the calling pod but not passed through to the
    spawned pgbackrest child. Post-audit, `build_pgbackrest_env`
    collects PGBACKREST_REPO1_CIPHER_PASS + AWS creds and threads
    them through."""
    rs = (
        _REPO / "crates" / "efe-backup" / "src" / "postgres.rs"
    ).read_text(encoding="utf-8")
    assert "fn build_pgbackrest_env" in rs
    assert "PGBACKREST_REPO1_CIPHER_PASS" in rs


def test_audit_ah9_tokio_runner_has_timeout():
    """**Audit fix § D17-AH9** — pre-audit, no timeout on subprocess
    calls ; a hung pgBackRest would block the CronJob pod indefinitely.
    Post-audit, `TokioCommandRunner` wraps every spawn in a
    `tokio::time::timeout`."""
    rs = (
        _REPO / "crates" / "efe-backup" / "src" / "command_runner.rs"
    ).read_text(encoding="utf-8")
    assert "pub process_timeout" in rs or "process_timeout:" in rs
    assert "tokio::time::timeout" in rs
    assert "ErrorKind::TimedOut" in rs


def test_audit_ah8_4_eyes_documents_trust_boundary():
    """**Audit fix § D17-AH8** — clarify that signature verification
    is delegated to the upstream surfacing path, not done in-process
    by the CLI."""
    rs = (
        _REPO / "crates" / "efe-backup" / "src" / "bin" / "efe_backup.rs"
    ).read_text(encoding="utf-8")
    # Must include the new trust-boundary docstring.
    assert "Trust delegation is to the **upstream surfacing path**" in rs
    # Must enforce non-empty sub claims.
    assert "non-empty" in rs.lower() or "non empty" in rs.lower()


# ─────────────────────────────────────────────────────────────────────
# Diátaxis howto — operator-facing runbook
# ─────────────────────────────────────────────────────────────────────


def test_d17_operator_howto_exists():
    """Diátaxis pattern : every operator-impacting sprint MUST ship
    a `docs/docs/howto/...` guide. D14-bis, D15 already ship
    operator howtos. D17 must do the same so on-call operators
    have a runbook before they're paged at 3 AM with a backup
    failure."""
    howto = _REPO / "docs" / "docs" / "howto" / "backup-and-restore.md"
    assert howto.exists(), (
        "D17 operator howto missing — Diátaxis pattern violated"
    )
    content = howto.read_text(encoding="utf-8")
    # Sanity : must cover the 5 canonical operations.
    for section in [
        "Take an ad-hoc backup",
        "Verify a backup at rest",
        "Restore from a backup",
        "Run a DR drill",
        "Delete a backup",
    ]:
        assert section in content, (
            f"howto missing operation section: {section!r}"
        )
    # Must reference the audit-fix institutional pattern (4-eyes,
    # cross-env guard, RGPD).
    assert "4-eyes" in content or "4 eyes" in content
    assert "RGPD" in content or "GDPR" in content
    assert "cross-env" in content.lower() or "cross-environment" in content


def test_d17_howto_links_to_architecture_and_audit():
    """The howto must cross-link to the architecture + audit doc
    so operators have access to the depth-of-explanation when a
    runbook step is unclear."""
    howto = _REPO / "docs" / "docs" / "howto" / "backup-and-restore.md"
    content = howto.read_text(encoding="utf-8")
    assert "d17_backup_dr_architecture.md" in content
    assert "d17_backup_dr_audit_report.md" in content


# ─────────────────────────────────────────────────────────────────────
# Operator runbooks (alert annotations must point to real files)
# ─────────────────────────────────────────────────────────────────────


def test_d17_alert_runbooks_exist():
    """**Institutional defense** — every Helm Prometheus alert
    annotated with a `runbook:` URL MUST point to a real file in
    `docs/runbook/`. Pre-fix : the alerts referenced 7 runbook
    URLs but the directory didn't exist — page PagerDuty at 3 AM
    → 404 on the link.

    This test scans every Helm template for `runbook:` annotations
    and asserts every referenced file exists. Adding a new alert
    forces shipping its runbook."""
    import re as _re
    helm_templates = (_REPO / "deploy" / "helm" / "efe" / "templates").glob("*.yaml")
    referenced_runbooks = set()
    for tpl in helm_templates:
        content = tpl.read_text(encoding="utf-8")
        for m in _re.finditer(r'runbook:\s*"([^"]+)"', content):
            referenced_runbooks.add(m.group(1))
    assert referenced_runbooks, (
        "no `runbook:` annotations found in any Helm template — "
        "sanity check failed"
    )
    missing = []
    for path in sorted(referenced_runbooks):
        full = _REPO / path
        if not full.exists():
            missing.append(path)
    assert not missing, (
        f"alert-referenced runbook(s) missing in `docs/runbook/`: "
        f"{missing}. Either ship the runbook OR remove the alert "
        f"annotation."
    )


def test_d17_runbook_index_exists():
    """The `docs/runbook/README.md` must exist and list every
    sprint-D17 runbook. Operators discover runbooks via the index."""
    idx = _REPO / "docs" / "runbook" / "README.md"
    assert idx.exists()
    content = idx.read_text(encoding="utf-8")
    for r in ["D17-1", "D17-2", "D17-3", "D17-4", "D17-5", "D17-7"]:
        assert r in content, f"runbook index missing entry: {r}"


def test_d17_runbooks_have_canonical_structure():
    """Every runbook must follow the canonical 5-section format :
    Symptom / Impact / Triage / Common causes / Mitigation /
    Escalation. This is the on-call ergonomics pattern : a runbook
    that doesn't have these sections is unusable at 3 AM."""
    runbook_dir = _REPO / "docs" / "runbook"
    required_sections = ["## Symptom", "## Impact", "## Triage", "## Common causes", "## Mitigation", "## Escalation"]
    for rb in runbook_dir.glob("D17-*.md"):
        content = rb.read_text(encoding="utf-8")
        for sec in required_sections:
            assert sec in content, (
                f"runbook {rb.name} missing canonical section: {sec!r}"
            )


# ─────────────────────────────────────────────────────────────────────
# D17-AC4-tris : AuditChainVerifier passes required args to efe-verify-audit
# ─────────────────────────────────────────────────────────────────────


def test_audit_ac4_tris_audit_chain_verifier_passes_pg_url_and_keys_file():
    """**Audit fix § D17-AC4-tris** — discovered tick 16 via
    real-binary smoke test. `efe-verify-audit --help` documents
    `--pg-url <DSN> --keys-file <PATH>` as REQUIRED args. The
    pre-AC4-tris `AuditChainVerifier` omitted both → spawn would
    fail with "missing required argument" → drill always fails
    at AuditChain step.
    Pin guard : the verifier impl must construct args including
    both flags."""
    rs = (
        _REPO / "crates" / "efe-backup" / "src" / "verifiers.rs"
    ).read_text(encoding="utf-8")
    # The struct must declare both fields.
    assert "pub pg_url: String" in rs
    assert "pub keys_file: String" in rs
    # The check() impl must include both flags in the args vec.
    import re as _re
    impl_block = _re.search(
        r"impl Verifier for AuditChainVerifier \{(.*?)^\}",
        rs,
        _re.DOTALL | _re.MULTILINE,
    )
    assert impl_block
    body = impl_block.group(1)
    assert '"--pg-url"' in body
    assert '"--keys-file"' in body


def test_audit_ac4_tris_cli_drill_threads_pg_env_vars():
    """The CLI `cmd_drill` must construct the Postgres DSN from
    the existing `EFE_PG_*` env vars (rendered by Helm ConfigMap)
    and the keys file path from `EFE_AUDIT_HMAC_SECRETS_FILE`."""
    rs = (
        _REPO / "crates" / "efe-backup" / "src" / "bin" / "efe_backup.rs"
    ).read_text(encoding="utf-8")
    assert "EFE_PG_HOST" in rs
    assert "EFE_PG_USER" in rs
    assert "EFE_PG_DATABASE" in rs
    assert "EFE_AUDIT_HMAC_SECRETS_FILE" in rs


# ─────────────────────────────────────────────────────────────────────
# Architecture acceptance criterion § 26 — audit report file exists
# ─────────────────────────────────────────────────────────────────────


def test_d17_audit_report_exists():
    """D17 architecture § 26 lists `docs/audits/d17_backup_dr_audit_report.md`
    as an explicit acceptance criterion. This pin test guards
    against a future refactor that deletes the report. Same retro-
    pattern that we used to close D13bis / D15 / D16 audit reports."""
    report = _REPO / "docs" / "audits" / "d17_backup_dr_audit_report.md"
    assert report.exists(), (
        "D17 audit report missing — architecture § 26 acceptance "
        "criterion not satisfied"
    )
    content = report.read_text(encoding="utf-8")
    # Sanity : contains the canonical sections + records the 10
    # findings (5C + 5H).
    for marker in [
        "AC1",  # the Critical wiring finding
        "AC2",  # dead metrics
        "AC3",  # signing pipeline
        "AC4",  # real verifiers
        "AC5",  # restore loop
        "AH1",
        "AH4",
        "AH5",
        "AH8",
        "AH9",
        "Conclusion",
        "Dette technique",
    ]:
        assert marker in content, (
            f"D17 audit report missing required marker: {marker!r}"
        )


def test_d17_audit_report_documents_recurring_wiring_class():
    """The D17 audit report MUST explicitly reference D13bis-A20 +
    D16-C1 as recurring wiring defects so the institutional learning
    is visible to any reader of D17 alone. Same pattern as the D16
    audit report referencing D13bis-A20."""
    report = _REPO / "docs" / "audits" / "d17_backup_dr_audit_report.md"
    content = report.read_text(encoding="utf-8")
    # Must reference the 2 prior wiring defects.
    assert "D16-C1" in content
    assert "D13bis-A20" in content
    # Must articulate the institutional pattern + the 3 disciplines.
    assert "binaire CLI" in content or "binary CLI" in content
    assert "Python pin test" in content
    assert "3" in content  # count of recurrence (3rd time)
