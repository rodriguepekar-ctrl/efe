"""Sprint **D38 (Φ-F)** — pin tests for Mode-3 workloads.

Anti-regression : guards M6 extension §11 :

  * `WorkloadOp::Mode3Execute { k_swarm: u32 }` declared.
  * `Workload::canonical("mode3-k5")` and `mode3-k10` constructors
    return the right shape (mix=100% Mode3Execute, k=5/10).
  * `canonical_labels()` lists the 8 patterns (6 D18 + 2 D38).
  * `K_SWARM_SOFT_CAP=5` and `K_SWARM_HARD_CAP=10` constants.
  * `validate_k_swarm()` enforces the soft + hard caps with
    opt-in via `allow_above_max`.
  * `BenchError::KSwarmAboveMax { got: u32 }` variant.
  * CLI `--k-swarm` and `--allow-k-above-max` flags.
  * CLI `compare` subcommand for C-3 + C-9 SLO evaluation.
  * `evaluate_c3()` and `evaluate_c9()` functions exist.
  * `BenchReport.k_swarm: Option<u32>` field.
  * Tonic client `dispatch_mode3_execute` method.
"""
from __future__ import annotations

from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

BENCH_CRATE = REPO_ROOT / "crates" / "efe-bench"
WORKLOAD_RS = BENCH_CRATE / "src" / "workload.rs"
ERROR_RS = BENCH_CRATE / "src" / "error.rs"
REPORT_RS = BENCH_CRATE / "src" / "report.rs"
CLIENT_TONIC = BENCH_CRATE / "src" / "client" / "tonic_client.rs"
CLI_BIN = BENCH_CRATE / "src" / "bin" / "efe_bench.rs"
LIB_RS = BENCH_CRATE / "src" / "lib.rs"


def _read(p: Path) -> str:
    return p.read_text(encoding="utf-8")


# ---------------------------------------------------------------------
# §11.2 — WorkloadOp::Mode3Execute
# ---------------------------------------------------------------------


def test_workload_op_mode3_execute_declared() -> None:
    rs = _read(WORKLOAD_RS)
    assert "Mode3Execute {" in rs
    assert "k_swarm: u32" in rs


def test_workload_op_as_str_specialised_for_k5_k10() -> None:
    rs = _read(WORKLOAD_RS)
    assert '"mode3_execute_k5"' in rs
    assert '"mode3_execute_k10"' in rs
    assert '"mode3_execute"' in rs  # generic fallback


def test_workload_op_is_mode3_helper_exists() -> None:
    rs = _read(WORKLOAD_RS)
    assert "pub fn is_mode3(&self) -> bool" in rs


# ---------------------------------------------------------------------
# §11.3 — canonical mode3-k5 + mode3-k10 patterns
# ---------------------------------------------------------------------


def test_workload_canonical_mode3_k5_declared() -> None:
    rs = _read(WORKLOAD_RS)
    assert '"mode3-k5" => Ok(Self {' in rs


def test_workload_canonical_mode3_k10_declared() -> None:
    rs = _read(WORKLOAD_RS)
    assert '"mode3-k10" => Ok(Self {' in rs


def test_workload_canonical_labels_lists_eight() -> None:
    """6 D18 + 2 D38 patterns = 8 canonical labels."""
    rs = _read(WORKLOAD_RS)
    labels = ["read-heavy", "write-heavy", "balanced", "burst",
              "multi-tenant", "soak", "mode3-k5", "mode3-k10"]
    for lbl in labels:
        assert f'"{lbl}"' in rs, f"canonical label {lbl} missing"


def test_workload_is_mode3_method_exists() -> None:
    rs = _read(WORKLOAD_RS)
    assert "pub fn is_mode3(&self) -> bool" in rs


def test_workload_k_swarm_extractor_method_exists() -> None:
    rs = _read(WORKLOAD_RS)
    assert "pub fn k_swarm(&self) -> Option<u32>" in rs


# ---------------------------------------------------------------------
# §11 — K_SWARM_SOFT_CAP + K_SWARM_HARD_CAP constants
# ---------------------------------------------------------------------


def test_k_swarm_caps_constants_declared() -> None:
    rs = _read(WORKLOAD_RS)
    assert "pub const K_SWARM_HARD_CAP: u32 = 10;" in rs
    assert "pub const K_SWARM_SOFT_CAP: u32 = 5;" in rs


def test_k_swarm_caps_re_exported_from_lib() -> None:
    rs = _read(LIB_RS)
    assert "K_SWARM_HARD_CAP" in rs
    assert "K_SWARM_SOFT_CAP" in rs


# ---------------------------------------------------------------------
# §11 — validate_k_swarm function
# ---------------------------------------------------------------------


def test_validate_k_swarm_function_exists() -> None:
    rs = _read(WORKLOAD_RS)
    assert "pub fn validate_k_swarm(k: u32, allow_above_max: bool)" in rs


def test_validate_k_swarm_rejects_zero() -> None:
    rs = _read(WORKLOAD_RS)
    assert 'k_swarm must be > 0' in rs


def test_validate_k_swarm_above_soft_cap_requires_opt_in() -> None:
    rs = _read(WORKLOAD_RS)
    assert "k > K_SWARM_SOFT_CAP && !allow_above_max" in rs
    assert "KSwarmAboveMax" in rs


def test_validate_k_swarm_above_hard_cap_always_rejected() -> None:
    rs = _read(WORKLOAD_RS)
    assert "k > K_SWARM_HARD_CAP" in rs


def test_workload_validate_rejects_mode3_zero_k() -> None:
    rs = _read(WORKLOAD_RS)
    assert "Mode3Execute k_swarm must be > 0" in rs


# ---------------------------------------------------------------------
# §11 — BenchError::KSwarmAboveMax
# ---------------------------------------------------------------------


def test_bench_error_has_k_swarm_above_max() -> None:
    rs = _read(ERROR_RS)
    assert "KSwarmAboveMax {" in rs
    assert "got: u32" in rs


def test_k_swarm_above_max_message_mentions_opt_in_flag() -> None:
    rs = _read(ERROR_RS)
    assert "--allow-k-above-max" in rs


def test_bench_error_metric_label_for_k_swarm() -> None:
    rs = _read(ERROR_RS)
    assert '"k_swarm_above_max"' in rs


# ---------------------------------------------------------------------
# §11.4 — CLI flags
# ---------------------------------------------------------------------


def test_cli_has_k_swarm_flag() -> None:
    rs = _read(CLI_BIN)
    assert "k_swarm: Option<u32>" in rs


def test_cli_has_allow_k_above_max_flag() -> None:
    rs = _read(CLI_BIN)
    assert "allow_k_above_max: bool" in rs


def test_cli_k_swarm_only_applies_to_mode3_workloads() -> None:
    rs = _read(CLI_BIN)
    assert "--k-swarm only applies to mode3-* workloads" in rs


def test_cli_compare_subcommand_exists() -> None:
    rs = _read(CLI_BIN)
    assert "Compare {" in rs
    assert "mode1_json: PathBuf" in rs
    assert "mode3_json: PathBuf" in rs
    assert "mode3_serial_json: Option<PathBuf>" in rs


def test_cli_returns_exit_code_2_on_k_swarm_above_max() -> None:
    rs = _read(CLI_BIN)
    assert "BenchError::KSwarmAboveMax { .. } => ExitCode::from(2)" in rs


# ---------------------------------------------------------------------
# §6.3 — schema v2 k_swarm field
# ---------------------------------------------------------------------


def test_bench_report_has_k_swarm_field() -> None:
    rs = _read(REPORT_RS)
    assert "pub k_swarm: Option<u32>" in rs


def test_bench_report_k_swarm_skipped_when_none() -> None:
    """Anti-pollution : the `k_swarm` field must not bloat the JSON
    of non-Mode-3 reports."""
    rs = _read(REPORT_RS)
    assert 'skip_serializing_if = "Option::is_none"' in rs


# ---------------------------------------------------------------------
# §11.6 — derived SLOs C-3 + C-9
# ---------------------------------------------------------------------


def test_evaluate_c3_function_exists() -> None:
    rs = _read(REPORT_RS)
    assert "pub fn evaluate_c3(" in rs


def test_evaluate_c9_function_exists() -> None:
    rs = _read(REPORT_RS)
    assert "pub fn evaluate_c9(" in rs


def test_c3_latency_ratio_constant() -> None:
    rs = _read(REPORT_RS)
    assert "pub const C3_MODE3_LATENCY_RATIO: f64 = 3.0;" in rs


def test_c9_min_speedup_constant() -> None:
    rs = _read(REPORT_RS)
    assert "pub const C9_MIN_SPEEDUP: f64 = 2.5;" in rs


def test_evaluate_c3_compares_mode3_to_mode1() -> None:
    rs = _read(REPORT_RS)
    # The function takes (mode3_report, mode1_report) and computes
    # the p99 ratio. We check the parameter names appear in the
    # signature + the implementation references both reports.
    assert "mode3_report:" in rs
    assert "mode1_report:" in rs
    assert "mode3_report" in rs
    assert "mode1_report" in rs
    # The implementation walks per_op of both reports.
    assert "per_op" in rs


# ---------------------------------------------------------------------
# §11.5 — Tonic client Mode-3 dispatch
# ---------------------------------------------------------------------


def test_tonic_client_has_dispatch_mode3_execute() -> None:
    rs = _read(CLIENT_TONIC)
    assert "fn dispatch_mode3_execute(" in rs


def test_tonic_client_propagates_k_swarm_to_proto_request() -> None:
    rs = _read(CLIENT_TONIC)
    assert "k: k_swarm" in rs


def test_tonic_client_uses_mode3_service_stub() -> None:
    rs = _read(CLIENT_TONIC)
    assert "Mode3ServiceClient" in rs
    assert "mode3_stub" in rs


# ---------------------------------------------------------------------
# §11 — re-exports from lib.rs
# ---------------------------------------------------------------------


def test_evaluate_c3_c9_re_exported_from_lib() -> None:
    rs = _read(LIB_RS)
    assert "evaluate_c3" in rs
    assert "evaluate_c9" in rs


def test_validate_k_swarm_re_exported_from_lib() -> None:
    rs = _read(LIB_RS)
    assert "validate_k_swarm" in rs
