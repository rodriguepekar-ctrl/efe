"""Pin tests for Φ-D-LATACT (P1 latence + P2 activation completion).

Sprint **Φ-D-LATACT D94** — artefact #29.
Cf. ``docs/EFE_P1_LATENCE_P2_ACTIVATION_COMPLETION_ARCHITECTURE.md`` § 19.

36 pin tests (18 structurels + 18 sémantiques) + 8 realism stubs.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

import pytest

_REPO = Path(__file__).resolve().parents[2]
_EVAL = _REPO / "eval" / "scripts"


def _read_yaml(p: Path) -> dict:
    yaml = pytest.importorskip("yaml")
    return yaml.safe_load(p.read_text())


def _run_bench(script_name: str, mode: str = "synth_pass",
                extra: list[str] | None = None) -> dict:
    import tempfile
    script = _EVAL / script_name
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "out.json"
        cmd = ["python3", str(script), "--mode", mode, "--out", str(out)]
        if extra:
            cmd.extend(extra)
        r = subprocess.run(cmd, capture_output=True, timeout=30)
        # Some scripts return non-zero in synth_fail intentionally.
        assert r.returncode in (0, 1), (
            f"{script_name} unexpected exit {r.returncode}: "
            f"stderr={r.stderr.decode()[:300]}"
        )
        return json.loads(out.read_text())


# ═════════════════════════════════════════════════════════════════════
# 18 STRUCTURAL PIN TESTS
# ═════════════════════════════════════════════════════════════════════


def test_latact_slo_yaml_has_18_entries():
    p = _REPO / "bench_baselines" / "v0.8.0" / "latact_slos.yaml"
    assert p.exists()
    data = _read_yaml(p)
    assert data["domain"] == "latency_activation"
    ids = {s["id"] for s in data["slos"]}
    expected = {
        "SLO-C-1", "SLO-P1-BUDGET-5", "SLO-P1-BUDGET-7",
        "SLO-LATACT-TAIL-P9999", "SLO-LATACT-BURN-4x3",
        "SLO-P2-TTFC-1", "SLO-P2-TTFC-2", "SLO-P2-TTFC-3",
        "SLO-P2-AHA-1", "SLO-P2-AHA-2",
        "SLO-P2-CONV-1", "SLO-P2-CONV-2",
        "SLO-P2-ABANDON-1", "SLO-P2-ABANDON-2",
        "SLO-P2-CLOUD-PARITY", "SLO-P2-ROLLBACK", "SLO-P2-DELETE-GDPR",
        "SLO-LATACT-PENDING-AGE",
    }
    missing = expected - ids
    assert not missing, f"missing : {sorted(missing)}"
    assert len(ids) == 18


def test_latact_all_slos_are_active_in_yaml():
    """Le sprint Φ-D-LATACT doit livrer 18 SLOs en status ACTIVE (= 0 PENDING)."""
    data = _read_yaml(_REPO / "bench_baselines" / "v0.8.0" / "latact_slos.yaml")
    non_active = [s["id"] for s in data["slos"]
                   if s.get("status", "ACTIVE") != "ACTIVE"]
    assert not non_active, f"SLOs still PENDING in latact_slos.yaml : {non_active}"


def test_latact_terraform_gpu_runner_module_present():
    p = _REPO / "infra" / "terraform" / "gpu-runner" / "main.tf"
    assert p.exists()
    text = p.read_text()
    assert "aws_spot_instance_request" in text
    assert "g4dn.xlarge" in text
    assert "self-hosted" in text or "self_hosted" in text


def test_latact_terraform_azure_module_present():
    p = _REPO / "infra" / "terraform" / "azure" / "main.tf"
    assert p.exists()
    text = p.read_text()
    # Must declare AKS + PG Flexible + Redis + Storage + Key Vault.
    assert "azurerm_kubernetes_cluster" in text
    assert "azurerm_postgresql_flexible_server" in text
    assert "azurerm_redis_cache" in text
    assert "azurerm_storage_account" in text
    assert "azurerm_key_vault" in text


def test_latact_efe_saga_crate_present():
    p = _REPO / "crates" / "efe-saga" / "Cargo.toml"
    assert p.exists()
    lib = _REPO / "crates" / "efe-saga" / "src" / "lib.rs"
    assert lib.exists()
    src = lib.read_text()
    assert "pub trait SagaStep" in src
    assert "async fn execute" in src
    assert "async fn compensate" in src
    assert "pub struct Saga" in src
    assert "SagaOutcome" in src


def test_latact_efe_saga_5_default_steps():
    """efe-saga must provide 5 default tenant-deploy steps + 6 GDPR steps."""
    steps = _REPO / "crates" / "efe-saga" / "src" / "steps.rs"
    assert steps.exists()
    src = steps.read_text()
    # Tenant deploy steps.
    for s in ["ProvisionPostgresStep", "MigrateSchemaStep", "SeedFixturesStep",
              "NotifyTenantStep", "MarkActiveStep"]:
        assert s in src, f"missing step : {s}"
    # GDPR steps.
    for s in ["GdprAuditStartStep", "GdprDropSessionsStep",
              "GdprTombstoneAuditStep", "GdprPurgeRedisStep",
              "GdprPurgeS3Step", "GdprAuditCompleteStep"]:
        assert s in src, f"missing GDPR step : {s}"


def test_latact_onboarding_sessions_migration_present():
    migrations = list((_REPO / "migrations").glob("*onboarding_sessions*.sql"))
    assert migrations, "no onboarding_sessions migration found"
    text = migrations[0].read_text()
    # Required columns + 4-axis consent + funnel timestamps.
    for col in ["tenant_id", "plan_target", "signed_up_at", "sandbox_created_at",
                 "first_classify_at", "first_correct_at", "poc_active_at",
                 "consent_analytics", "consent_model_training",
                 "consent_marketing", "consent_third_party_sharing"]:
        assert col in text, f"migration missing column : {col}"
    # LISTEN/NOTIFY trigger for streaming audit replay.
    assert "pg_notify" in text


def test_latact_alerts_p1_burn_4x3_present():
    p = _REPO / "alerts" / "p1_burn_4x3.yaml"
    assert p.exists()


def test_latact_portal_telemetry_typescript_present():
    p = _REPO / "portal" / "src" / "lib" / "telemetry.ts"
    assert p.exists()
    text = p.read_text()
    assert "ActivationEvent" in text
    assert "emitActivationEvent" in text
    # 7 states (incl. abandoned).
    for state in ["signup", "sandbox_created", "first_classify",
                   "first_correct", "poc_requested", "poc_active", "abandoned"]:
        assert f"'{state}'" in text, f"state {state} missing"


def test_latact_portal_web_vitals_present():
    p = _REPO / "portal" / "src" / "lib" / "web_vitals.ts"
    assert p.exists()
    text = p.read_text()
    for fn in ["onLCP", "onINP", "onCLS", "onFCP", "onTTFB"]:
        assert fn in text, f"web-vitals function {fn} missing"


def test_latact_portal_signup_page_present():
    assert (_REPO / "portal" / "src" / "app" / "signup" / "page.tsx").exists()
    assert (_REPO / "portal" / "src" / "app" / "sandbox" / "page.tsx").exists()


def test_latact_phase2_scripts_exist():
    expected = [
        _EVAL / "p1_tail_multi_regime_bench.py",
        _EVAL / "p1_burn_4x3_coverage_audit.py",
        _EVAL / "p2_portal_funnel_query.py",
        _EVAL / "p2_cloud_parity_bench.py",
        _EVAL / "p2_saga_rollback_bench.py",
        _EVAL / "registry_age_scan.py",
    ]
    missing = [p for p in expected if not p.exists()]
    assert not missing, f"missing : {missing}"


def test_latact_phase2_scripts_invokable():
    for s in _EVAL.glob("p1_*.py"):
        r = subprocess.run(["python3", str(s), "--help"],
                            capture_output=True, timeout=10)
        assert r.returncode == 0, f"{s.name} --help failed"
    for s in _EVAL.glob("p2_*.py"):
        r = subprocess.run(["python3", str(s), "--help"],
                            capture_output=True, timeout=10)
        assert r.returncode == 0, f"{s.name} --help failed"
    for name in ["registry_age_scan.py"]:
        r = subprocess.run(["python3", str(_EVAL / name), "--help"],
                            capture_output=True, timeout=10)
        assert r.returncode == 0


def test_latact_phase2_scripts_emit_skip_in_live_mode():
    """Anti-Φ-D.A1 : every bench with --mode live emits skip_reason."""
    import tempfile
    scripts = [
        "p1_tail_multi_regime_bench.py", "p1_burn_4x3_coverage_audit.py",
        "p2_portal_funnel_query.py", "p2_cloud_parity_bench.py",
        "p2_saga_rollback_bench.py", "registry_age_scan.py",
    ]
    with tempfile.TemporaryDirectory() as td:
        for name in scripts:
            out = Path(td) / f"{name}.json"
            r = subprocess.run(
                ["python3", str(_EVAL / name), "--mode", "live", "--out", str(out)],
                capture_output=True, timeout=10,
            )
            assert r.returncode == 0, f"{name} live mode crashed"
            data = json.loads(out.read_text())
            slo = next(iter(data["slo_compliance"].values()))
            assert "skip_reason" in slo, f"{name} did not emit skip_reason in live"


def test_latact_runbooks_exist():
    runbook_dir = _REPO / "docs" / "runbook"
    expected = [
        "latact_gpu_runner_failure.md",
        "latact_portal_outage.md",
        "latact_saga_stuck.md",
        "latact_cloud_parity_violation.md",
        "latact_pending_overdue.md",
    ]
    missing = [n for n in expected if not (runbook_dir / n).exists()]
    assert not missing, f"missing runbooks : {missing}"


def test_latact_workflows_exist():
    wf = _REPO / ".github" / "workflows"
    expected = ["latact-contract.yml", "latact-bench.yml", "latact-portal-e2e.yml"]
    missing = [n for n in expected if not (wf / n).exists()]
    assert not missing, f"missing workflows : {missing}"


def test_latact_architecture_doc_exists():
    p = _REPO / "docs" / "EFE_P1_LATENCE_P2_ACTIVATION_COMPLETION_ARCHITECTURE.md"
    assert p.exists()
    text = p.read_text()
    assert "Φ-D-LATACT" in text
    assert "D-LATACT-1" in text and "D-LATACT-10" in text
    assert "SLO-LATACT-PENDING-AGE" in text


def test_latact_no_orphan_dt_references():
    """Every DT-LATACT-* in code MUST be in archi § 23."""
    archi = (_REPO / "docs" / "EFE_P1_LATENCE_P2_ACTIVATION_COMPLETION_ARCHITECTURE.md").read_text()
    archi_dts = set(re.findall(r"DT-LATACT-[A-Z0-9-]+", archi))
    code_dts = set()
    for f in [
        *_EVAL.glob("p1_*.py"), *_EVAL.glob("p2_*.py"),
        _EVAL / "registry_age_scan.py",
        _REPO / "scripts" / "tests" / "test_latact_completion.py",
    ]:
        if f.exists():
            code_dts.update(re.findall(r"DT-LATACT-[A-Z0-9-]+", f.read_text()))
    orphans = code_dts - archi_dts
    assert not orphans, f"DT-LATACT-* orphans : {sorted(orphans)}"


# ═════════════════════════════════════════════════════════════════════
# 18 SEMANTIC PIN TESTS
# ═════════════════════════════════════════════════════════════════════


def test_latact_tail_multi_regime_synth_pass_p9999_below_cap():
    d = _run_bench("p1_tail_multi_regime_bench.py", "synth_pass",
                    ["--n-samples", "100000"])
    slo = d["slo_compliance"]["SLO-LATACT-TAIL-P9999"]
    assert slo["measured_seconds"] <= 0.800, f"p99.99 = {slo['measured_seconds']:.3f}s > 0.8s"


def test_latact_tail_synth_fail_detects_outlier():
    """synth_fail injects 1.5s outlier → p99.99 > 0.8s → exit 1."""
    import tempfile
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "out.json"
        r = subprocess.run(
            ["python3", str(_EVAL / "p1_tail_multi_regime_bench.py"),
              "--mode", "synth_fail", "--n-samples", "100000", "--out", str(out)],
            capture_output=True, timeout=30,
        )
        assert r.returncode == 1
        slo = json.loads(out.read_text())["slo_compliance"]["SLO-LATACT-TAIL-P9999"]
        assert slo["measured_seconds"] > 0.800


def test_latact_tail_n0_skips():
    """n_samples ≤ 0 must SKIP, not crash."""
    import tempfile
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "out.json"
        r = subprocess.run(
            ["python3", str(_EVAL / "p1_tail_multi_regime_bench.py"),
              "--n-samples", "0", "--out", str(out)],
            capture_output=True, timeout=10,
        )
        assert r.returncode == 0
        slo = json.loads(out.read_text())["slo_compliance"]["SLO-LATACT-TAIL-P9999"]
        assert "skip_reason" in slo


def test_latact_burn_4x3_12_cells_defined():
    d = _run_bench("p1_burn_4x3_coverage_audit.py", "synth_pass")
    slo = d["slo_compliance"]["SLO-LATACT-BURN-4x3"]
    assert slo["measured_pass_count"] == 1
    assert slo["n_cells_found"] == 12
    assert slo["n_cells_expected"] == 12
    assert not slo["missing_cells"]


def test_latact_burn_4x3_synth_fail_detects_missing_cell():
    import tempfile
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "out.json"
        r = subprocess.run(
            ["python3", str(_EVAL / "p1_burn_4x3_coverage_audit.py"),
              "--mode", "synth_fail", "--out", str(out)],
            capture_output=True, timeout=10,
        )
        assert r.returncode == 1
        slo = json.loads(out.read_text())["slo_compliance"]["SLO-LATACT-BURN-4x3"]
        assert slo["measured_pass_count"] == 0


def test_latact_portal_funnel_emits_9_slos():
    d = _run_bench("p2_portal_funnel_query.py", "synth_pass")
    compl = d["slo_compliance"]
    for slo_id in ["SLO-P2-TTFC-1", "SLO-P2-TTFC-2", "SLO-P2-TTFC-3",
                    "SLO-P2-AHA-1", "SLO-P2-AHA-2",
                    "SLO-P2-CONV-1", "SLO-P2-CONV-2",
                    "SLO-P2-ABANDON-1", "SLO-P2-ABANDON-2"]:
        assert slo_id in compl, f"{slo_id} not emitted"


def test_latact_portal_ttfc_sandbox_below_5min():
    d = _run_bench("p2_portal_funnel_query.py", "synth_pass")
    sandbox = d["slo_compliance"]["SLO-P2-TTFC-2"]
    assert sandbox["measured_seconds"] is not None
    assert sandbox["measured_seconds"] <= 300.0, (
        f"Sandbox TTFC p90 = {sandbox['measured_seconds']:.1f}s > 300s")


def test_latact_portal_ttfc_poc_below_30min():
    d = _run_bench("p2_portal_funnel_query.py", "synth_pass")
    poc = d["slo_compliance"]["SLO-P2-TTFC-1"]
    assert poc["measured_seconds"] is not None
    assert poc["measured_seconds"] <= 1800.0


def test_latact_portal_conv_signup_above_60pct():
    d = _run_bench("p2_portal_funnel_query.py", "synth_pass")
    conv = d["slo_compliance"]["SLO-P2-CONV-1"]
    assert conv["measured_ratio"] >= 0.60, (
        f"signup→classify conversion = {conv['measured_ratio']:.4f} < 0.60")


def test_latact_portal_abandon_sandbox_below_40pct():
    d = _run_bench("p2_portal_funnel_query.py", "synth_pass")
    aba = d["slo_compliance"]["SLO-P2-ABANDON-2"]
    assert aba["measured_ratio"] <= 0.40


def test_latact_cloud_parity_under_20pct():
    d = _run_bench("p2_cloud_parity_bench.py", "synth_pass")
    slo = d["slo_compliance"]["SLO-P2-CLOUD-PARITY"]
    assert slo["measured_ratio"] <= 0.20
    assert len(slo["p90_per_cloud"]) == 3   # gcp + aws + azure


def test_latact_cloud_parity_synth_fail_detects_outlier():
    import tempfile
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "out.json"
        r = subprocess.run(
            ["python3", str(_EVAL / "p2_cloud_parity_bench.py"),
              "--mode", "synth_fail", "--out", str(out)],
            capture_output=True, timeout=10,
        )
        assert r.returncode == 1
        slo = json.loads(out.read_text())["slo_compliance"]["SLO-P2-CLOUD-PARITY"]
        # Azure was 3× → variance ≫ 0.20
        assert slo["measured_ratio"] > 0.20


def test_latact_saga_rollback_below_10min():
    d = _run_bench("p2_saga_rollback_bench.py", "synth_pass",
                    ["--n-injections", "50"])
    slo = d["slo_compliance"]["SLO-P2-ROLLBACK"]
    assert slo["measured_seconds"] <= 600.0


def test_latact_saga_rollback_n0_skips():
    import tempfile
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "out.json"
        r = subprocess.run(
            ["python3", str(_EVAL / "p2_saga_rollback_bench.py"),
              "--n-injections", "0", "--out", str(out)],
            capture_output=True, timeout=10,
        )
        assert r.returncode == 0
        slo = json.loads(out.read_text())["slo_compliance"]["SLO-P2-ROLLBACK"]
        assert "skip_reason" in slo


def test_latact_registry_age_scan_no_overdue():
    d = _run_bench("registry_age_scan.py", "synth_pass")
    slo = d["slo_compliance"]["SLO-LATACT-PENDING-AGE"]
    assert slo["n_pending_over_cap"] == 0


def test_latact_registry_age_scan_synth_fail_detects_overdue():
    import tempfile
    with tempfile.TemporaryDirectory() as td:
        out = Path(td) / "out.json"
        r = subprocess.run(
            ["python3", str(_EVAL / "registry_age_scan.py"),
              "--mode", "synth_fail", "--out", str(out)],
            capture_output=True, timeout=10,
        )
        assert r.returncode == 1
        slo = json.loads(out.read_text())["slo_compliance"]["SLO-LATACT-PENDING-AGE"]
        assert slo["n_pending_over_cap"] >= 1


def test_latact_funnel_event_states_typed_in_typescript():
    """Funnel telemetry MUST define all 7 states (= D-LATACT-9 granular)."""
    tele = (_REPO / "portal" / "src" / "lib" / "telemetry.ts").read_text()
    for state in ["'signup'", "'sandbox_created'", "'first_classify'",
                   "'first_correct'", "'poc_requested'",
                   "'poc_active'", "'abandoned'"]:
        assert state in tele


def test_latact_efe_saga_unit_tests_present():
    """The Rust crate must include unit tests for compensation order."""
    src = (_REPO / "crates" / "efe-saga" / "src" / "lib.rs").read_text()
    assert "#[tokio::test]" in src
    assert "saga_compensate_called_in_reverse_order" in src
    assert "saga_rolls_back_on_step_failure" in src


# ═════════════════════════════════════════════════════════════════════
# L1 AUDIT — anti-régression sur les 4 fixes critiques
# ═════════════════════════════════════════════════════════════════════


def test_latact_l1_efe_saga_in_workspace_cargo_toml():
    """L1 fix L1-1 : efe-saga MUST be in workspace Cargo.toml members.

    Pre-fix the crate was orphan : the file existed but `cargo build`
    would not pick it up → unit tests never compile.
    """
    workspace = (_REPO / "Cargo.toml").read_text()
    assert "members" in workspace
    assert '"crates/efe-saga"' in workspace, (
        "efe-saga not registered in workspace Cargo.toml → orphan crate"
    )


def test_latact_l1_signup_persists_ts_for_ttfc_timer():
    """L1 fix L1-2 : signup page persists signup_ts to sessionStorage.

    Pre-fix : SIGNUP_TS_KEY was read in /sandbox but never set in
    /signup → time_since_signup_ms = 0 → faussement-PASS on TTFC/AHA.
    """
    signup = (_REPO / "portal" / "src" / "app" / "signup" / "page.tsx").read_text()
    assert "SIGNUP_TS_KEY" in signup
    assert "sessionStorage.setItem" in signup
    assert "sessionStorage.setItem(SIGNUP_TS_KEY" in signup


def test_latact_l1_sandbox_reads_ttfc_timer_correctly():
    """L1 fix L1-2 : sandbox page reads SIGNUP_TS_KEY."""
    sandbox = (_REPO / "portal" / "src" / "app" / "sandbox" / "page.tsx").read_text()
    assert "SIGNUP_TS_KEY" in sandbox
    assert "sessionStorage.getItem(SIGNUP_TS_KEY)" in sandbox
    assert "time_since_signup_ms" in sandbox


def test_latact_l1_portal_tsconfig_present():
    """L1 fix L1-3 : portal/tsconfig.json with @/* alias."""
    p = _REPO / "portal" / "tsconfig.json"
    assert p.exists(), "portal/tsconfig.json missing"
    cfg = json.loads(p.read_text())
    paths = cfg.get("compilerOptions", {}).get("paths", {})
    assert "@/*" in paths, "tsconfig must declare @/* alias"
    assert paths["@/*"] == ["./src/*"]


def test_latact_l1_portal_next_config_present():
    """L1 fix L1-3 : portal/next.config.{js,mjs,ts} present."""
    candidates = [
        _REPO / "portal" / "next.config.mjs",
        _REPO / "portal" / "next.config.js",
        _REPO / "portal" / "next.config.ts",
    ]
    assert any(p.exists() for p in candidates), "no Next.js config in portal/"


def test_latact_l1_terraform_azure_random_provider_declared():
    """L1 fix L1-4 : Terraform Azure declares `random` in required_providers."""
    tf = (_REPO / "infra" / "terraform" / "azure" / "main.tf").read_text()
    assert 'source  = "hashicorp/azurerm"' in tf
    assert 'source  = "hashicorp/random"' in tf, (
        "Terraform Azure missing random provider → init fails"
    )
    assert "random_password" in tf


# ═════════════════════════════════════════════════════════════════════
# 8 REALISM TESTS (= sentinels for follow-up sprints)
# ═════════════════════════════════════════════════════════════════════


@pytest.mark.skip(reason="DT-LATACT-GPU-CAPACITY-PROD : requires GPU runner")
def test_realism_latact_gpu_bench_sustained_100k():
    pass


@pytest.mark.skip(reason="DT-LATACT-PORTAL-LIVE-FUNNEL : requires Next.js + PG")
def test_realism_latact_portal_signup_to_classify():
    pass


@pytest.mark.skip(reason="DT-LATACT-3CLOUDS-APPLY : requires AWS+GCP+Azure creds")
def test_realism_latact_terraform_3clouds_apply():
    pass


@pytest.mark.skip(reason="DT-LATACT-SAGA-INJECT-PROD : requires staging env")
def test_realism_latact_saga_failure_at_step_3():
    pass


@pytest.mark.skip(reason="DT-LATACT-GDPR-S3-GLACIER-RESIDUE : requires S3")
def test_realism_latact_gdpr_delete_cascade_30d():
    pass


@pytest.mark.skip(reason="DT-LATACT-BURN-PROD-ALERT : requires Prometheus")
def test_realism_latact_burn_4x3_alert_fires():
    pass


@pytest.mark.skip(reason="DT-LATACT-GPU-CAPACITY-PROD : requires sustained 100k req")
def test_realism_latact_tail_p9999_under_load():
    pass


@pytest.mark.skip(reason="DT-LATACT-PENDING-90D-PROD : requires live registry")
def test_realism_latact_pending_age_overdue_alert():
    pass
