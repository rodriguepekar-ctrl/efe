"""Sprint **D31 (Φ-D)** — pin tests for the `--gpu-shards N` flag.

Anti-regression : guards M6 extension §10 :

  * CLI exposes `--gpu-shards` flag (range 1..=16).
  * `RunnerConfig` carries `gpu_shards` field with default 1 and
    `with_gpu_shards()` builder.
  * `validate_gpu_shards()` exists in workload module and rejects
    0 + values > 16.
  * `BenchError::GpuShardOutOfRange { got: u32 }` variant declared.
  * The Tonic client propagates the `x-efe-gpu-shard-id` metadata
    header on every request.
  * `BenchReport` v2 schema includes `gpu_shards: Option<u32>`
    plus `per_shard: BTreeMap<u32, ShardReport>`.
  * Derived SLO `D31-GPU-1` is evaluated when `gpu_shards > 1`.
  * `jain_index()` helper exists for fairness computation.
"""
from __future__ import annotations

from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

BENCH_CRATE = REPO_ROOT / "crates" / "efe-bench"
WORKLOAD_RS = BENCH_CRATE / "src" / "workload.rs"
RUNNER_RS = BENCH_CRATE / "src" / "runner.rs"
ERROR_RS = BENCH_CRATE / "src" / "error.rs"
REPORT_RS = BENCH_CRATE / "src" / "report.rs"
CLIENT_TONIC = BENCH_CRATE / "src" / "client" / "tonic_client.rs"
CLI_BIN = BENCH_CRATE / "src" / "bin" / "efe_bench.rs"
LIB_RS = BENCH_CRATE / "src" / "lib.rs"


def _read(p: Path) -> str:
    return p.read_text(encoding="utf-8")


# ---------------------------------------------------------------------
# §10.3 — CLI flag
# ---------------------------------------------------------------------


def test_cli_declares_gpu_shards_flag() -> None:
    rs = _read(CLI_BIN)
    assert "gpu_shards: u32" in rs


def test_cli_gpu_shards_default_is_one() -> None:
    rs = _read(CLI_BIN)
    assert 'default_value = "1"' in rs


def test_cli_gpu_shards_range_validated_via_clap() -> None:
    rs = _read(CLI_BIN)
    # Range parser : 1..=16.
    assert "value_parser = clap::value_parser!(u32).range(1..=16)" in rs


# ---------------------------------------------------------------------
# §6.2 — invariant : validate_gpu_shards exists in workload.rs
# ---------------------------------------------------------------------


def test_validate_gpu_shards_function_exists() -> None:
    rs = _read(WORKLOAD_RS)
    assert "pub fn validate_gpu_shards(n: u32)" in rs


def test_validate_gpu_shards_rejects_zero_and_above_cap() -> None:
    rs = _read(WORKLOAD_RS)
    assert "n == 0 || n > 16" in rs
    assert "GpuShardOutOfRange" in rs


def test_validate_gpu_shards_re_exported_from_lib() -> None:
    rs = _read(LIB_RS)
    assert "validate_gpu_shards" in rs


# ---------------------------------------------------------------------
# §10.4 — runner shard orchestration
# ---------------------------------------------------------------------


def test_runner_config_has_gpu_shards_field() -> None:
    rs = _read(RUNNER_RS)
    assert "pub gpu_shards: u32" in rs


def test_runner_config_default_gpu_shards_is_one() -> None:
    rs = _read(RUNNER_RS)
    # `Self { workload, target_env: env, gpu_shards: 1 }` constructor.
    assert "gpu_shards: 1," in rs


def test_runner_config_with_gpu_shards_builder() -> None:
    rs = _read(RUNNER_RS)
    assert "pub fn with_gpu_shards(mut self, n: u32)" in rs


def test_runner_assigns_shard_id_modulo_concurrency() -> None:
    rs = _read(RUNNER_RS)
    assert "(worker_id as u32) % gpu_shards" in rs


def test_runner_passes_worker_id_to_client_call() -> None:
    """**Audit fix M6-A1 (Φ-F)** — the runner threads `worker_id`
    through every `client.call(...)` so the Tonic client can
    align the propagated `x-efe-gpu-shard-id` metadata with the
    per-shard report bucket. Pre-audit the Tonic client used a
    private atomic counter that rotated across workers."""
    rs = _read(RUNNER_RS)
    assert "client.call(op, &tenant, worker_id)" in rs


def test_runner_records_per_shard_observations() -> None:
    rs = _read(RUNNER_RS)
    # The worker_loop records into per_shard_histograms / per_shard_counters.
    assert "per_shard_histograms" in rs
    assert "per_shard_counters" in rs


def test_runner_outcome_carries_per_shard_and_gpu_shards() -> None:
    rs = _read(RUNNER_RS)
    assert "pub per_shard: std::collections::BTreeMap<u32, ShardStats>" in rs
    assert "pub gpu_shards: u32" in rs


def test_shard_stats_struct_declared() -> None:
    rs = _read(RUNNER_RS)
    assert "pub struct ShardStats" in rs


# ---------------------------------------------------------------------
# §6.2 — BenchError::GpuShardOutOfRange
# ---------------------------------------------------------------------


def test_bench_error_has_gpu_shard_out_of_range() -> None:
    rs = _read(ERROR_RS)
    assert "GpuShardOutOfRange" in rs
    assert "got: u32" in rs


def test_bench_error_metric_label_for_gpu_shard() -> None:
    rs = _read(ERROR_RS)
    assert '"gpu_shard_out_of_range"' in rs


# ---------------------------------------------------------------------
# §6.3 — JSON schema v2 gpu_shards + per_shard
# ---------------------------------------------------------------------


def test_bench_report_has_gpu_shards_field() -> None:
    rs = _read(REPORT_RS)
    assert "pub gpu_shards: Option<u32>" in rs


def test_bench_report_has_per_shard_field() -> None:
    rs = _read(REPORT_RS)
    assert "pub per_shard: std::collections::BTreeMap<u32, ShardReport>" in rs


def test_shard_report_struct_declared() -> None:
    rs = _read(REPORT_RS)
    assert "pub struct ShardReport" in rs


def test_shard_report_carries_p99_rps_success_error() -> None:
    rs = _read(REPORT_RS)
    assert "pub shard_id: u32" in rs
    assert "pub p99_ms: Option<f64>" in rs
    assert "pub rps: f64" in rs


# ---------------------------------------------------------------------
# §10.5 — D31-GPU-1 SLO + Jain index
# ---------------------------------------------------------------------


def test_jain_index_function_exists() -> None:
    rs = _read(REPORT_RS)
    assert "pub fn jain_index(values: &[f64]) -> f64" in rs


def test_d31_gpu_fairness_floor_constant_declared() -> None:
    rs = _read(REPORT_RS)
    assert "pub const D31_GPU_FAIRNESS_FLOOR: f64 = 0.95" in rs


def test_evaluate_d31_gpu_fairness_function_exists() -> None:
    rs = _read(REPORT_RS)
    assert "fn evaluate_d31_gpu_fairness(" in rs


def test_d31_slo_emitted_only_when_shards_above_one() -> None:
    rs = _read(REPORT_RS)
    # The generate_report path branches on `outcome.gpu_shards > 1`.
    assert "outcome.gpu_shards > 1" in rs


# ---------------------------------------------------------------------
# §9.4 — Tonic client propagates x-efe-gpu-shard-id
# ---------------------------------------------------------------------


def test_tonic_client_propagates_shard_metadata() -> None:
    rs = _read(CLIENT_TONIC)
    assert "x-efe-gpu-shard-id" in rs


def test_tonic_client_decorate_function_uses_gpu_shards() -> None:
    """**Audit fix M6-A1 (Φ-F)** — `decorate(req, worker_id)` derives
    the shard_id from the runner-supplied `worker_id` (per-worker
    stable) so the propagated `x-efe-gpu-shard-id` aligns with the
    per-shard metric bucket. Pre-audit a private atomic counter
    `worker_idx` rotated across all workers, decoupling routing
    from metrics."""
    rs = _read(CLIENT_TONIC)
    assert "self.gpu_shards" in rs
    # Post-audit the modulo expression keys on `worker_id`, not on
    # the rotating `call_idx` atomic.
    assert "worker_id % (shards as u64)" in rs


def test_tonic_client_kind_overrides_gpu_shards_method() -> None:
    rs = _read(CLIENT_TONIC)
    assert "fn gpu_shards(&self) -> Option<u32>" in rs


# ---------------------------------------------------------------------
# §10.5 — bench_client_trait gpu_shards default
# ---------------------------------------------------------------------


def test_client_trait_default_gpu_shards_returns_none() -> None:
    rs = _read(BENCH_CRATE / "src" / "client" / "mod.rs")
    assert "fn gpu_shards(&self) -> Option<u32>" in rs
    assert "None" in rs
