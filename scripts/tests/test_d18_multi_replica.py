"""Sprint **D18** — Multi-réplica + bench D12 implementation pin tests.

Cf. `docs/bench/d18_multi_replica_bench_d12_architecture.md`.

Implementation phase pin tests : assert the structural invariants
of the D18 code surface so a future refactor that drops them
surfaces in CI.
"""

from __future__ import annotations

import re
from pathlib import Path

import yaml

_REPO = Path(__file__).resolve().parents[2]


# ─────────────────────────────────────────────────────────────────────
# Cargo workspace + crate skeleton
# ─────────────────────────────────────────────────────────────────────


def test_d18_efe_bench_workspace_member_declared():
    cargo = (_REPO / "Cargo.toml").read_text(encoding="utf-8")
    assert '"crates/efe-bench"' in cargo, (
        "efe-bench must be in the workspace members list"
    )


def test_d18_efe_bench_cargo_toml_exists():
    f = _REPO / "crates" / "efe-bench" / "Cargo.toml"
    assert f.exists()
    content = f.read_text(encoding="utf-8")
    assert 'name = "efe-bench"' in content
    assert 'path = "src/bin/efe_bench.rs"' in content


def test_d18_efe_bench_module_files_present():
    src = _REPO / "crates" / "efe-bench" / "src"
    expected = [
        "lib.rs",
        "error.rs",
        "workload.rs",
        "gen.rs",
        "metrics.rs",
        "client.rs",
        "runner.rs",
        "report.rs",
        "bin/efe_bench.rs",
    ]
    for m in expected:
        f = src / m
        assert f.exists(), f"D18 efe-bench module missing: {m}"


def test_d18_efe_bench_cli_subcommands():
    rs = (
        _REPO / "crates" / "efe-bench" / "src" / "bin" / "efe_bench.rs"
    ).read_text(encoding="utf-8")
    for cmd in ["Run", "Gen", "ListSlos", "ValidateTargetEnv", "Report"]:
        assert f"Command::{cmd}" in rs, f"CLI missing subcommand variant: {cmd}"


def test_d18_efe_bench_six_workload_patterns():
    rs = (_REPO / "crates" / "efe-bench" / "src" / "workload.rs").read_text(
        encoding="utf-8"
    )
    for w in ["read-heavy", "write-heavy", "balanced", "burst", "multi-tenant", "soak"]:
        assert f'"{w}"' in rs, f"workload pattern {w!r} missing"


def test_d18_efe_bench_refuses_prod():
    rs = (_REPO / "crates" / "efe-bench" / "src" / "workload.rs").read_text(
        encoding="utf-8"
    )
    assert "RefusedProdTarget" in rs
    assert "validate_target_env" in rs


def test_d18_efe_bench_seven_slos():
    rs = (_REPO / "crates" / "efe-bench" / "src" / "report.rs").read_text(
        encoding="utf-8"
    )
    for n in range(1, 8):
        assert f'"D18-bench-{n}"' in rs, f"SLO D18-bench-{n} missing"


# ─────────────────────────────────────────────────────────────────────
# efe-server multi_replica module
# ─────────────────────────────────────────────────────────────────────


def test_d18_multi_replica_module_present():
    mod = (
        _REPO / "crates" / "efe-server" / "src" / "multi_replica" / "mod.rs"
    )
    assert mod.exists()


def test_d18_multi_replica_five_modules():
    mr_dir = _REPO / "crates" / "efe-server" / "src" / "multi_replica"
    expected = [
        "idempotency_pg.rs",
        "ratelimit_pg.rs",
        "leader_election.rs",
        "shutdown_sequence.rs",
        "health_grpc.rs",
    ]
    for m in expected:
        f = mr_dir / m
        assert f.exists(), f"D18 multi_replica module missing: {m}"


def test_d18_idempotency_traits_present():
    rs = (
        _REPO
        / "crates"
        / "efe-server"
        / "src"
        / "multi_replica"
        / "idempotency_pg.rs"
    ).read_text(encoding="utf-8")
    assert "pub trait IdempotencyStore" in rs
    assert "pub struct PgIdempotencyStore" in rs
    assert "pub struct MockIdempotencyStore" in rs
    # Must have the 3 outcomes : Leader / InFlight / Cached.
    for variant in ["Leader", "InFlight", "Cached"]:
        assert f"IdempotencyOutcome::{variant}" in rs or f"=> Self::{variant}" in rs or variant in rs


def test_d18_ratelimit_traits_present():
    rs = (
        _REPO
        / "crates"
        / "efe-server"
        / "src"
        / "multi_replica"
        / "ratelimit_pg.rs"
    ).read_text(encoding="utf-8")
    assert "pub trait RateLimitStore" in rs
    assert "pub struct PgRateLimitStore" in rs
    assert "pub struct MockRateLimitStore" in rs
    assert "RateLimitDecision::Allowed" in rs
    assert "RateLimitDecision::Throttled" in rs


def test_d18_leader_election_traits_present():
    rs = (
        _REPO
        / "crates"
        / "efe-server"
        / "src"
        / "multi_replica"
        / "leader_election.rs"
    ).read_text(encoding="utf-8")
    assert "pub trait LeaderLock" in rs
    assert "pub struct K8sLeaseLock" in rs
    assert "pub struct MockLeaderLock" in rs
    # Must reference K8s coordination.k8s.io.
    assert "coordination.k8s.io" in rs


def test_d18_shutdown_sequence_three_phases():
    rs = (
        _REPO
        / "crates"
        / "efe-server"
        / "src"
        / "multi_replica"
        / "shutdown_sequence.rs"
    ).read_text(encoding="utf-8")
    assert "DrainRpc" in rs
    assert "AuditFlush" in rs
    assert "PgClose" in rs
    # 3 distinct timeout fields.
    assert "drain_rpc:" in rs
    assert "audit_flush:" in rs
    assert "pg_close:" in rs


def test_d18_shutdown_validates_grace_period():
    rs = (
        _REPO
        / "crates"
        / "efe-server"
        / "src"
        / "multi_replica"
        / "shutdown_sequence.rs"
    ).read_text(encoding="utf-8")
    # Must validate that total ≤ k8s_grace_period.
    assert "k8s_grace_period" in rs


def test_d18_health_three_states():
    rs = (
        _REPO
        / "crates"
        / "efe-server"
        / "src"
        / "multi_replica"
        / "health_grpc.rs"
    ).read_text(encoding="utf-8")
    for state in ["NotServing", "Serving", "ShuttingDown"]:
        assert state in rs


def test_d18_health_three_preconditions():
    rs = (
        _REPO
        / "crates"
        / "efe-server"
        / "src"
        / "multi_replica"
        / "health_grpc.rs"
    ).read_text(encoding="utf-8")
    for pc in ["pg_pool_ok", "bundle_loaded", "jwks_prefetched"]:
        assert pc in rs


# ─────────────────────────────────────────────────────────────────────
# Migration 0007_d18_idempotency.sql
# ─────────────────────────────────────────────────────────────────────


_MIGRATION = (
    _REPO / "crates" / "efe-server" / "migrations" / "0007_d18_idempotency.sql"
)


def test_d18_migration_present():
    assert _MIGRATION.exists()


def test_d18_migration_creates_two_tables():
    sql = _MIGRATION.read_text(encoding="utf-8")
    assert "CREATE TABLE IF NOT EXISTS efe_idempotency" in sql
    assert "CREATE TABLE IF NOT EXISTS efe_ratelimit_buckets" in sql


def test_d18_migration_idempotency_check_constraints():
    sql = _MIGRATION.read_text(encoding="utf-8")
    # Cross-field invariant on completed/response.
    assert "chk_idem_completed_consistency" in sql
    # Status enum.
    assert "chk_idem_status_values" in sql
    # TTL invariant.
    assert "chk_idem_expires_after_started" in sql


def test_d18_migration_ratelimit_check_constraints():
    sql = _MIGRATION.read_text(encoding="utf-8")
    assert "chk_rl_available_in_range" in sql
    assert "chk_rl_capacity_positive" in sql
    assert "chk_rl_refill_non_negative" in sql


def test_d18_migration_indexes():
    sql = _MIGRATION.read_text(encoding="utf-8")
    for idx in [
        "idx_idem_expires",
        "idx_idem_tenant_started",
        "idx_ratelimit_last_refill",
    ]:
        assert idx in sql


# ─────────────────────────────────────────────────────────────────────
# Helm chart D18 surface
# ─────────────────────────────────────────────────────────────────────


_VALUES = _REPO / "deploy" / "helm" / "efe" / "values.yaml"
_CONFIGMAP = _REPO / "deploy" / "helm" / "efe" / "templates" / "configmap.yaml"
_TEMPLATES_DIR = _REPO / "deploy" / "helm" / "efe" / "templates"


def test_d18_helm_templates_present():
    expected = [
        "d18-pdb.yaml",
        "d18-hpa.yaml",
        "d18-grpc-headless.yaml",
        "d18-leader-election-rbac.yaml",
        "d18-prometheus-rules.yaml",
    ]
    for f in expected:
        path = _TEMPLATES_DIR / f
        assert path.exists(), f"Helm template missing: {f}"


def test_d18_values_yaml_replicas_block():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    assert "replicas" in data
    assert data["replicas"]["min"] >= 1
    assert data["replicas"]["max"] >= data["replicas"]["min"]


def test_d18_values_yaml_default_replicas_min_3():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    assert data["replicas"]["min"] == 3, (
        "Default replicas.min should be 3 (HA target)"
    )


def test_d18_values_yaml_pdb_min_available_2():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    assert data["podDisruptionBudget"]["minAvailable"] == 2


def test_d18_values_yaml_hpa_cpu_target_70():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    assert data["hpa"]["cpuTarget"] == 70


def test_d18_values_yaml_headless_service():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    assert data["service"]["headless"] is True


def test_d18_values_yaml_termination_grace_60s():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    assert data["terminationGracePeriodSeconds"] == 60


def test_d18_values_yaml_leader_election_block():
    data = yaml.safe_load(_VALUES.read_text(encoding="utf-8"))
    le = data["leaderElection"]
    assert le["enabled"] is True
    assert le["leaseName"] == "efe-leader"


def test_d18_configmap_renders_envvars():
    cm = _CONFIGMAP.read_text(encoding="utf-8")
    expected = [
        "EFE_REPLICAS_MIN",
        "EFE_REPLICAS_MAX",
        "EFE_LEADER_ELECTION_ENABLED",
        "EFE_LEADER_LEASE_NAME",
        "EFE_LEADER_LEASE_DURATION_S",
        "EFE_LEADER_RENEW_DEADLINE_S",
        "EFE_SHUTDOWN_DRAIN_MS",
        "EFE_SHUTDOWN_AUDIT_FLUSH_MS",
        "EFE_SHUTDOWN_PG_CLOSE_MS",
    ]
    for var in expected:
        assert var in cm, f"configmap.yaml missing {var}"


def test_d18_prometheus_rules_alerts():
    rules = (_TEMPLATES_DIR / "d18-prometheus-rules.yaml").read_text(
        encoding="utf-8"
    )
    expected_alerts = [
        "D18ReplicasUnderTarget",
        "D18LeaderMissing",
        "D18LeaderSplit",
        "D18IdempotencyHitMiss",
        "D18RateLimitThrottleSpike",
        "D18ShutdownTimeout",
        "D18BenchSloRegression",
    ]
    for a in expected_alerts:
        assert a in rules, f"alert {a} missing from d18-prometheus-rules.yaml"


def test_d18_grpc_headless_uses_clusterip_none():
    h = (_TEMPLATES_DIR / "d18-grpc-headless.yaml").read_text(encoding="utf-8")
    assert "clusterIP: None" in h


def test_d18_leader_rbac_minimal_least_privilege():
    rbac = (_TEMPLATES_DIR / "d18-leader-election-rbac.yaml").read_text(
        encoding="utf-8"
    )
    # Must restrict to the named lease (not all leases cluster-wide).
    assert "resourceNames:" in rbac
    assert "efe-leader" in rbac


# ─────────────────────────────────────────────────────────────────────
# Setup script
# ─────────────────────────────────────────────────────────────────────


def test_d18_setup_kind_script_exists():
    f = _REPO / "scripts" / "setup-multi-replica-kind.sh"
    assert f.exists()


def test_d18_setup_kind_script_executable():
    f = _REPO / "scripts" / "setup-multi-replica-kind.sh"
    mode = f.stat().st_mode
    assert mode & 0o100, "setup script not executable for owner"


def test_d18_setup_kind_creates_3_workers():
    sh = (
        _REPO / "scripts" / "setup-multi-replica-kind.sh"
    ).read_text(encoding="utf-8")
    # Must spin a kind cluster with 3 worker nodes (zone-a/b/c).
    for zone in ["zone-a", "zone-b", "zone-c"]:
        assert zone in sh, f"setup script missing zone label: {zone}"


# ─────────────────────────────────────────────────────────────────────
# Architecture acceptance criterion
# ─────────────────────────────────────────────────────────────────────


def test_d18_architecture_doc_exists():
    """Implementation pin : the architecture doc must remain
    present alongside the code."""
    doc = (
        _REPO / "docs" / "bench" / "d18_multi_replica_bench_d12_architecture.md"
    )
    assert doc.exists()


def test_d18_changelog_entry_present():
    cl = (_REPO / "CHANGELOG.md").read_text(encoding="utf-8")
    assert "D18" in cl, "CHANGELOG should mention D18"
    # Either implementation phase OR architecture phase entry.
    assert "Multi-réplica" in cl or "Multi-replica" in cl


# ─────────────────────────────────────────────────────────────────────
# Library-level sanity (re-exports)
# ─────────────────────────────────────────────────────────────────────


def test_d18_efe_bench_lib_re_exports():
    rs = (_REPO / "crates" / "efe-bench" / "src" / "lib.rs").read_text(
        encoding="utf-8"
    )
    for sym in [
        "BenchClient",
        "MockBenchClient",
        "BenchError",
        "Workload",
        "WorkloadOp",
        "Runner",
        "RunnerConfig",
        "BenchReport",
        "validate_target_env",
    ]:
        assert sym in rs, f"efe-bench lib missing re-export: {sym}"


def test_d18_multi_replica_mod_re_exports_all_five():
    rs = (
        _REPO / "crates" / "efe-server" / "src" / "multi_replica" / "mod.rs"
    ).read_text(encoding="utf-8")
    for sym in [
        "IdempotencyStore",
        "PgIdempotencyStore",
        "RateLimitStore",
        "PgRateLimitStore",
        "LeaderLock",
        "K8sLeaseLock",
        "MockLeaderLock",
        "ShutdownSequence",
        "ShutdownTimeouts",
        "HealthService",
        "HealthStatus",
    ]:
        assert sym in rs, f"multi_replica mod.rs missing re-export: {sym}"


# ─────────────────────────────────────────────────────────────────────
# Audit-pass regression pin tests (D18-A1 .. D18-A12)
#
# These fire on a future refactor that drops one of the audit fixes,
# making the recurring "wiring forgotten" defect class impossible to
# regress silently.
# ─────────────────────────────────────────────────────────────────────


def test_audit_d18_a3_d18_instruments_module_exists():
    """D18-A3 — `d18_instruments.rs` ships the 9 architecture-§17.1 metrics."""
    rs = (
        _REPO
        / "crates"
        / "efe-telemetry"
        / "src"
        / "metrics"
        / "d18_instruments.rs"
    )
    assert rs.exists(), "D18-A3 regression : d18_instruments.rs missing"
    text = rs.read_text(encoding="utf-8")
    for metric in [
        "efe_replica_id",
        "efe_idempotency_hits_total",
        "efe_idempotency_size",
        "efe_ratelimit_decisions_total",
        "efe_ratelimit_bucket_available",
        "efe_leader_is_leader",
        "efe_leader_acquire_attempts_total",
        "efe_shutdown_phase_seconds",
        "efe_bench_runs_total",
    ]:
        assert metric in text, (
            f"D18-A3 regression : d18_instruments.rs missing metric {metric}"
        )
    assert "register_all_d18" in text


def test_audit_d18_a3_register_all_d18_exported():
    """D18-A3 — `efe_telemetry::metrics` re-exports `register_all_d18`."""
    rs = (
        _REPO / "crates" / "efe-telemetry" / "src" / "metrics" / "mod.rs"
    ).read_text(encoding="utf-8")
    assert "pub mod d18_instruments" in rs
    assert "pub use d18_instruments::register_all_d18" in rs


def test_audit_d18_a3_idempotency_bump_wired():
    """D18-A3 — `idempotency_pg.rs` bumps `idempotency_hits_total`."""
    rs = (
        _REPO
        / "crates"
        / "efe-server"
        / "src"
        / "multi_replica"
        / "idempotency_pg.rs"
    ).read_text(encoding="utf-8")
    assert "idempotency_hits_total" in rs, (
        "D18-A3 regression : idempotency impl no longer bumps the metric"
    )
    assert "IdempotencyOutcomeLabel" in rs


def test_audit_d18_a3_ratelimit_bump_wired():
    """D18-A3 — `ratelimit_pg.rs` bumps `ratelimit_decisions_total`."""
    rs = (
        _REPO
        / "crates"
        / "efe-server"
        / "src"
        / "multi_replica"
        / "ratelimit_pg.rs"
    ).read_text(encoding="utf-8")
    assert "ratelimit_decisions_total" in rs, (
        "D18-A3 regression : ratelimit impl no longer bumps the metric"
    )
    assert "RateLimitOutcomeLabel" in rs


def test_audit_d18_a3_leader_bump_wired():
    """D18-A3 — `leader_election.rs` bumps `leader_acquire_attempts_total`."""
    rs = (
        _REPO
        / "crates"
        / "efe-server"
        / "src"
        / "multi_replica"
        / "leader_election.rs"
    ).read_text(encoding="utf-8")
    assert "leader_acquire_attempts_total" in rs
    assert "LeaderOutcomeLabel" in rs


def test_audit_d18_a3_shutdown_phase_observation():
    """D18-A3 — `shutdown_sequence.rs` observes `shutdown_phase_seconds`."""
    rs = (
        _REPO
        / "crates"
        / "efe-server"
        / "src"
        / "multi_replica"
        / "shutdown_sequence.rs"
    ).read_text(encoding="utf-8")
    assert "shutdown_phase_seconds" in rs


def test_audit_d18_a1_runtime_wires_multi_replica():
    """D18-A1 — `runtime.rs::ServerBuilder::build` instantiates multi_replica."""
    rs = (_REPO / "crates" / "efe-server" / "src" / "runtime.rs").read_text(
        encoding="utf-8"
    )
    assert "use crate::multi_replica::" in rs
    assert "build_multi_replica_handles" in rs
    assert "register_all_d18" in rs
    # The replica_id gauge MUST be set at build time so dashboards
    # have something to partition on.
    assert "replica_id()" in rs and ".set(1)" in rs


def test_audit_d18_a1_config_has_multi_replica_section():
    """D18-A1 — `ServerConfig` carries a `multi_replica` field."""
    cfg = (_REPO / "crates" / "efe-server" / "src" / "config.rs").read_text(
        encoding="utf-8"
    )
    assert "pub struct MultiReplicaConfig" in cfg
    assert "pub multi_replica: MultiReplicaConfig" in cfg
    # Each D18 env var MUST be read.
    for var in [
        "EFE_REPLICA_ID",
        "EFE_REPLICAS_MIN",
        "EFE_REPLICAS_MAX",
        "EFE_LEADER_ELECTION_ENABLED",
        "EFE_LEADER_LEASE_NAME",
        "EFE_LEADER_LEASE_DURATION_S",
        "EFE_LEADER_RENEW_DEADLINE_S",
        "EFE_SHUTDOWN_DRAIN_RPC_MS",
        "EFE_SHUTDOWN_AUDIT_FLUSH_MS",
        "EFE_SHUTDOWN_PG_CLOSE_MS",
    ]:
        assert var in cfg, (
            f"D18-A1 regression : config.rs no longer reads {var}"
        )


def test_audit_d18_a5_leader_election_loop_present():
    """D18-A5 — `runtime.rs` ships `run_leader_election_loop`."""
    rs = (_REPO / "crates" / "efe-server" / "src" / "runtime.rs").read_text(
        encoding="utf-8"
    )
    assert "run_leader_election_loop" in rs
    assert "lock.try_acquire().await" in rs
    assert "lock.renew().await" in rs


def test_audit_d18_a6_serve_drives_shutdown_sequence():
    """D18-A6 — `serve()` drives the 3-phase `ShutdownSequence`."""
    rs = (_REPO / "crates" / "efe-server" / "src" / "runtime.rs").read_text(
        encoding="utf-8"
    )
    assert "ShutdownSequence::new" in rs
    assert "run_drain_rpc" in rs
    assert "run_audit_flush" in rs
    assert "run_pg_close" in rs


def test_audit_d18_a2_deployment_consumes_pod_anti_affinity():
    """D18-A2 — `deployment.yaml` consumes `Values.podAntiAffinity`."""
    dep = (
        _REPO / "deploy" / "helm" / "efe" / "templates" / "deployment.yaml"
    ).read_text(encoding="utf-8")
    assert ".Values.podAntiAffinity" in dep, (
        "D18-A2 regression : deployment.yaml no longer reads "
        "Values.podAntiAffinity"
    )
    assert ".Values.topologySpreadConstraints" in dep, (
        "D18-A2 regression : deployment.yaml no longer reads "
        "Values.topologySpreadConstraints"
    )


def test_audit_d18_a8_runner_no_unused_imports():
    """D18-A8 — `runner.rs` imports cleaned of `CallOutcome`."""
    rs = (_REPO / "crates" / "efe-bench" / "src" / "runner.rs").read_text(
        encoding="utf-8"
    )
    # The line `use crate::client::{BenchClient, CallOutcome};` must
    # not exist any more (the audit fix dropped CallOutcome from the
    # import list).
    assert "use crate::client::{BenchClient, CallOutcome}" not in rs


def test_audit_d18_a9_runner_no_dead_op_tag_helper():
    """D18-A9 — dead `op_tag` helper removed from runner.rs.

    We look for the actual function definition (line starts with
    `fn op_tag(`), not for the explanatory comment which intentionally
    mentions the now-deleted helper to document the audit pass.
    """
    rs = (_REPO / "crates" / "efe-bench" / "src" / "runner.rs").read_text(
        encoding="utf-8"
    )
    for line in rs.splitlines():
        # A real function definition starts at column 0 (or with `pub`)
        # ; the comment lines all start with `//`. Skip comment lines.
        stripped = line.lstrip()
        if stripped.startswith("//"):
            continue
        assert "fn op_tag(" not in stripped, (
            "D18-A9 regression : the dead op_tag helper resurfaced "
            f"(line: {line!r})"
        )


def test_audit_d18_a10_cmd_run_requires_allow_mock():
    """D18-A10 — `efe-bench run` aborts without `--allow-mock`."""
    rs = (
        _REPO / "crates" / "efe-bench" / "src" / "bin" / "efe_bench.rs"
    ).read_text(encoding="utf-8")
    assert "allow_mock" in rs, (
        "D18-A10 regression : --allow-mock flag missing from CLI"
    )
    assert "MockBenchClient" in rs
    # The guard must be a hard error path.
    assert "if !allow_mock" in rs


def test_audit_d18_a11_values_yaml_no_duplicate_top_level_keys():
    """D18-A11 — `values.yaml` has no duplicate top-level keys.

    Pre-fix the chart shipped two ``service:``, two ``resources:``,
    and two ``terminationGracePeriodSeconds:`` mappings at the file
    root. YAML last-wins silently dropped the earlier blocks.
    """
    text = (
        _REPO / "deploy" / "helm" / "efe" / "values.yaml"
    ).read_text(encoding="utf-8")
    # We only count NON-INDENTED keys (top-level mapping). A line is
    # top-level when it starts at column 0 with `key:` followed by
    # whitespace or end-of-line. Comments + blank lines are skipped.
    top_level: dict[str, int] = {}
    for line in text.splitlines():
        if not line or line.startswith("#"):
            continue
        if line[0].isspace() or line[0] == "-":
            continue
        m = re.match(r"^([A-Za-z_][A-Za-z0-9_]*)\s*:", line)
        if m:
            top_level[m.group(1)] = top_level.get(m.group(1), 0) + 1
    duplicates = {k: c for k, c in top_level.items() if c > 1}
    assert not duplicates, (
        f"D18-A11 regression : values.yaml has duplicate top-level keys: "
        f"{duplicates}"
    )


def test_audit_d18_a12_configmap_no_duplicate_env_var():
    """D18-A12 — `configmap.yaml` no longer renders `EFE_SHUTDOWN_DRAIN_MS` twice.

    Pre-fix the D18 block at the bottom of the ConfigMap rendered
    `EFE_SHUTDOWN_DRAIN_MS` from `Values.shutdown.drainMs` while the
    legacy section near the top rendered the same env var from
    `Values.config.shutdown.drainMs`. YAML last-wins silently
    overrode the legacy value.

    Fix : rename the D18 phase-1 env var to
    `EFE_SHUTDOWN_DRAIN_RPC_MS` so the two paths no longer collide.
    """
    text = (
        _REPO / "deploy" / "helm" / "efe" / "templates" / "configmap.yaml"
    ).read_text(encoding="utf-8")
    # We only count actual render lines (`KEY:` at column 2 inside
    # the ConfigMap data block). Lines starting with `#` or `  #` are
    # comments and may legitimately mention the env var name.
    render_count = 0
    for line in text.splitlines():
        stripped = line.lstrip()
        if stripped.startswith("#"):
            continue
        if "EFE_SHUTDOWN_DRAIN_MS:" in line:
            render_count += 1
    assert render_count == 1, (
        f"D18-A12 regression : EFE_SHUTDOWN_DRAIN_MS rendered "
        f"{render_count} times as a key in configmap.yaml "
        f"(expected exactly 1)"
    )
    # And the new D18 phase-1 env var name is the one the runtime
    # reads.
    assert "EFE_SHUTDOWN_DRAIN_RPC_MS" in text


def test_audit_d18_a1_validate_rejects_bad_lease_pair():
    """D18-A1 — `validate()` rejects renew_deadline >= lease_duration."""
    cfg = (_REPO / "crates" / "efe-server" / "src" / "config.rs").read_text(
        encoding="utf-8"
    )
    # The validate() body must include the architecture-§8.2 invariant.
    assert "leader_renew_deadline_s" in cfg
    assert "leader_lease_duration_s" in cfg
    # The error message must reference the architecture section.
    assert "§ 8.2" in cfg or "architecture § 8.2" in cfg
