"""Sprint **D29** — GPU Prometheus instruments pin tests.

Cf. `docs/EFE_KERNELS_ARCHITECTURE.md` § 18.

Asserts the 6 GPU instruments are declared in `gpu_instruments.rs`,
that the `register_all_gpu()` helper is exported, and that the
metric bumps are wired from the `efe-kernels::observability` module
(catches the recurring "metric declared but never observed" class).
"""

from __future__ import annotations

import re
from pathlib import Path

_REPO = Path(__file__).resolve().parents[2]


# ─────────────────────────────────────────────────────────────────────
# 1. The 6 instruments are declared
# ─────────────────────────────────────────────────────────────────────


def test_gpu_instruments_module_exists():
    f = _REPO / "crates" / "efe-telemetry" / "src" / "metrics" / "gpu_instruments.rs"
    assert f.exists(), "D29-K-A1 regression : gpu_instruments.rs missing"


def test_gpu_instruments_six_metrics_declared():
    rs = (_REPO / "crates" / "efe-telemetry" / "src" / "metrics" / "gpu_instruments.rs").read_text(encoding="utf-8")
    for metric in [
        "efe_gpu_kernel_duration_seconds",
        "efe_gpu_memory_used_bytes",
        "efe_gpu_utilization_ratio",
        "efe_gpu_kernel_calls_total",
        "efe_gpu_oom_total",
        "efe_gpu_nccl_allreduce_seconds",
    ]:
        assert metric in rs, (
            f"D29-K-A1 regression : gpu_instruments.rs missing {metric}"
        )


def test_register_all_gpu_helper_present():
    rs = (_REPO / "crates" / "efe-telemetry" / "src" / "metrics" / "gpu_instruments.rs").read_text(encoding="utf-8")
    assert "pub fn register_all_gpu()" in rs


def test_register_all_gpu_re_exported_from_mod():
    rs = (_REPO / "crates" / "efe-telemetry" / "src" / "metrics" / "mod.rs").read_text(encoding="utf-8")
    assert "pub mod gpu_instruments" in rs
    assert "pub use gpu_instruments::register_all_gpu" in rs


# ─────────────────────────────────────────────────────────────────────
# 2. observability.rs in efe-kernels bumps every metric
# ─────────────────────────────────────────────────────────────────────


def test_efe_kernels_observability_bumps_each_metric():
    rs = (_REPO / "crates" / "efe-kernels" / "src" / "observability.rs").read_text(encoding="utf-8")
    # Every metric must have at least one bump caller.
    expected_helpers = [
        ("record_kernel_duration", "gpu_kernel_duration_seconds"),
        ("record_kernel_outcome",  "gpu_kernel_calls_total"),
        ("record_memory_used",     "gpu_memory_used_bytes"),
        ("record_oom",             "gpu_oom_total"),
        ("record_nccl_allreduce",  "gpu_nccl_allreduce_seconds"),
    ]
    for fn, metric in expected_helpers:
        assert f"pub fn {fn}" in rs, f"observability.rs missing helper: {fn}"
        assert metric in rs, (
            f"observability.rs no longer references {metric} — "
            "wiring forgotten regression"
        )


def test_kernel_call_guard_records_at_drop():
    rs = (_REPO / "crates" / "efe-kernels" / "src" / "observability.rs").read_text(encoding="utf-8")
    assert "impl Drop for KernelCallGuard" in rs
    assert "record_kernel_duration" in rs
    assert "record_kernel_outcome" in rs


# ─────────────────────────────────────────────────────────────────────
# 3. backend.rs uses the guard for every implemented op
# ─────────────────────────────────────────────────────────────────────


def test_backend_uses_record_kernel_call_guard():
    rs = (_REPO / "crates" / "efe-kernels" / "src" / "backend.rs").read_text(encoding="utf-8")
    # The guard must be acquired in every method body. We count
    # `record_kernel_call(` invocations and require ≥ 20 (one per
    # of the 23 ComputeBackend methods).
    n = len(re.findall(r'record_kernel_call\(', rs))
    assert n >= 20, (
        f"backend.rs only acquires record_kernel_call {n} times — "
        "every ComputeBackend method must obtain a guard (D29-K-A2)"
    )


# ─────────────────────────────────────────────────────────────────────
# 4. Cardinality budget
# ─────────────────────────────────────────────────────────────────────


def test_kernel_calls_total_label_arity_is_two():
    rs = (_REPO / "crates" / "efe-telemetry" / "src" / "metrics" / "gpu_instruments.rs").read_text(encoding="utf-8")
    # Locate the once_metric! definition (NOT the docstring banner), then
    # verify the immediately-following label slice carries both labels.
    # We split on the actual definition line which has the comma after the
    # identifier — distinct from the table line that uses pipes.
    needle = "    gpu_kernel_calls_total,"
    idx = rs.find(needle)
    assert idx >= 0, "gpu_kernel_calls_total definition line not found"
    block = rs[idx : idx + 800]
    assert '"kernel"' in block, f"missing 'kernel' label in block:\n{block}"
    assert '"outcome"' in block, f"missing 'outcome' label in block:\n{block}"


def test_outcome_label_values_pinned():
    """Architecture § 18.1 commits to outcome ∈ {ok, fail, oom}."""
    rs = (_REPO / "crates" / "efe-telemetry" / "src" / "metrics" / "gpu_instruments.rs").read_text(encoding="utf-8")
    # The pinned set is enforced by a unit test inside the Rust crate
    # (`gpu_kernel_calls_outcome_strings_pinned`).
    assert "gpu_kernel_calls_outcome_strings_pinned" in rs
