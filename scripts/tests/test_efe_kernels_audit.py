"""Sprint **D28 (Φ-D)** — chirurgical audit régression pin tests.

Cf. `docs/EFE_KERNELS_ARCHITECTURE.md`.

Each test verrouille un fix d'audit. Toute régression future
(future refactor that drops a fix) surface en CI.

Audit-fix IDs covered :
  K-A1  — register_all_gpu wired from Server::build
  K-A2  — RuntimeConfig + EFE_RUNTIME_GPU_* env vars
  K-A3  — Helm GPU values + configmap + deployment GPU resource
  K-A4  — no `transmute` to enum in efe-kernels (UB risk)
  K-A5  — .github/workflows/test-gpu-smoke.yml exists
  K-A6  — efe-bench has --runtime gpu flag
  K-A7  — check_finite kernel + binding
  K-A8  — raw_stream helper centralizes cudarc handle access
  K-A9  — efe-kernels README present
  K-A16 — KernelCallGuard.fail() called on error path
"""

from __future__ import annotations

import re
from pathlib import Path

_REPO = Path(__file__).resolve().parents[2]


# ─────────────────────────────────────────────────────────────────────
# K-A1 : register_all_gpu() wired from Server::build
# ─────────────────────────────────────────────────────────────────────


def test_audit_k_a1_register_all_gpu_wired_in_runtime():
    rs = (_REPO / "crates" / "efe-server" / "src" / "runtime.rs").read_text(
        encoding="utf-8"
    )
    assert "register_all_gpu" in rs, (
        "K-A1 regression: efe-server::Server::build no longer "
        "calls efe_telemetry::metrics::register_all_gpu"
    )
    # Must be feature-conditional (not blindly always-on).
    assert "config.runtime.gpu_enabled" in rs, (
        "K-A1 regression: register_all_gpu must be gated on "
        "RuntimeConfig::gpu_enabled"
    )


def test_audit_k_a1_efe_kernels_optional_dep_in_efe_server():
    cargo = (
        _REPO / "crates" / "efe-server" / "Cargo.toml"
    ).read_text(encoding="utf-8")
    # The dep MUST be optional (kept off the default build for Mac CI).
    assert re.search(
        r'efe-kernels\s*=\s*\{[^}]*optional\s*=\s*true', cargo
    ), "efe-kernels must be an optional dep of efe-server"
    # The cuda feature MUST pull it in.
    assert re.search(
        r'cuda\s*=\s*\[[^\]]*"dep:efe-kernels"[^\]]*\]', cargo
    ), "efe-server `cuda` feature must pull efe-kernels"


# ─────────────────────────────────────────────────────────────────────
# K-A2 : RuntimeConfig + EFE_RUNTIME_GPU_* env vars
# ─────────────────────────────────────────────────────────────────────


def test_audit_k_a2_runtime_config_struct_present():
    cfg = (_REPO / "crates" / "efe-server" / "src" / "config.rs").read_text(
        encoding="utf-8"
    )
    assert "pub struct RuntimeConfig" in cfg
    assert "pub runtime: RuntimeConfig" in cfg
    for field in [
        "gpu_enabled",
        "gpu_device_ordinal",
        "gpu_memory_hwm_percent",
        "gpu_streams",
        "gpu_shard_ordinals",
        "gpu_fallback_policy",
        "gpu_check_finite",
    ]:
        assert field in cfg, f"RuntimeConfig missing field: {field}"


def test_audit_k_a2_runtime_env_vars_read():
    cfg = (_REPO / "crates" / "efe-server" / "src" / "config.rs").read_text(
        encoding="utf-8"
    )
    for var in [
        "EFE_RUNTIME_GPU_ENABLED",
        "EFE_RUNTIME_GPU_DEVICE_ORDINAL",
        "EFE_RUNTIME_GPU_MEMORY_HWM_PERCENT",
        "EFE_RUNTIME_GPU_STREAMS",
        "EFE_RUNTIME_GPU_SHARD_ORDINALS",
        "EFE_RUNTIME_GPU_FALLBACK_POLICY",
        "EFE_RUNTIME_GPU_CHECK_FINITE",
    ]:
        assert var in cfg, (
            f"K-A2 regression: ServerConfig::from_env_with no longer reads {var}"
        )


def test_audit_k_a2_runtime_config_validate_invariants():
    cfg = (_REPO / "crates" / "efe-server" / "src" / "config.rs").read_text(
        encoding="utf-8"
    )
    # Validate must reject out-of-range HWM percent.
    assert "runtime.gpu_memory_hwm_percent" in cfg
    # Validate must reject unknown fallback policy.
    assert "runtime.gpu_fallback_policy" in cfg
    assert '"strict" | "cpu_on_error"' in cfg


# ─────────────────────────────────────────────────────────────────────
# K-A3 : Helm GPU values + configmap + deployment
# ─────────────────────────────────────────────────────────────────────


def test_audit_k_a3_helm_values_has_runtime_block():
    values = (
        _REPO / "deploy" / "helm" / "efe" / "values.yaml"
    ).read_text(encoding="utf-8")
    assert "runtime:" in values
    for k in [
        "gpuEnabled",
        "gpuDeviceOrdinal",
        "gpuMemoryHwmPercent",
        "gpuStreams",
        "gpuShardOrdinals",
        "gpuFallbackPolicy",
        "gpuCheckFinite",
        "gpuCount",
    ]:
        assert k in values, f"K-A3: helm values.yaml missing runtime.{k}"


def test_audit_k_a3_helm_configmap_renders_gpu_envvars():
    cm = (
        _REPO / "deploy" / "helm" / "efe" / "templates" / "configmap.yaml"
    ).read_text(encoding="utf-8")
    for var in [
        "EFE_RUNTIME_GPU_ENABLED",
        "EFE_RUNTIME_GPU_DEVICE_ORDINAL",
        "EFE_RUNTIME_GPU_MEMORY_HWM_PERCENT",
        "EFE_RUNTIME_GPU_STREAMS",
        "EFE_RUNTIME_GPU_SHARD_ORDINALS",
        "EFE_RUNTIME_GPU_FALLBACK_POLICY",
        "EFE_RUNTIME_GPU_CHECK_FINITE",
    ]:
        assert f"{var}:" in cm, (
            f"K-A3: configmap.yaml no longer renders {var}"
        )


def test_audit_k_a3_helm_deployment_supports_nvidia_gpu_resource():
    dep = (
        _REPO / "deploy" / "helm" / "efe" / "templates" / "deployment.yaml"
    ).read_text(encoding="utf-8")
    # The deployment must conditionally render `nvidia.com/gpu`
    # under runtime.gpuEnabled.
    assert "nvidia.com/gpu" in dep, (
        "K-A3: deployment.yaml does not declare nvidia.com/gpu resource"
    )
    assert ".Values.runtime.gpuEnabled" in dep
    # Optional NVIDIA runtimeClassName injection.
    assert "runtimeClassName" in dep


# ─────────────────────────────────────────────────────────────────────
# K-A4 : no UB transmute in efe-kernels
# ─────────────────────────────────────────────────────────────────────


def test_audit_k_a4_no_transmute_to_cuda_enum():
    err = (
        _REPO / "crates" / "efe-kernels" / "src" / "error.rs"
    ).read_text(encoding="utf-8")
    # The risky transmute has been removed.
    bad_pat = re.compile(
        r"transmute::<\s*i32\s*,\s*cudarc::driver::sys::cudaError\s*>"
    )
    assert not bad_pat.search(err), (
        "K-A4 regression : UB transmute<i32, cudaError> resurfaced"
    )


# ─────────────────────────────────────────────────────────────────────
# K-A5 : GPU CI workflow exists
# ─────────────────────────────────────────────────────────────────────


def test_audit_k_a5_gpu_ci_workflow_present():
    f = _REPO / ".github" / "workflows" / "test-gpu-smoke.yml"
    assert f.exists(), "K-A5: .github/workflows/test-gpu-smoke.yml missing"
    yaml_text = f.read_text(encoding="utf-8")
    # Must have at least 4 jobs : cargo-check-no-cuda, gpu-cross-backend-smoke,
    # gpu-cuda-memcheck, pin-tests-structure.
    for job in [
        "cargo-check-no-cuda",
        "gpu-cross-backend-smoke",
        "gpu-cuda-memcheck",
        "pin-tests-structure",
    ]:
        assert job in yaml_text, f"K-A5: workflow missing job `{job}`"
    # Must reference the self-hosted GPU runner label.
    assert "[self-hosted, gpu, t4]" in yaml_text


# ─────────────────────────────────────────────────────────────────────
# K-A6 : efe-bench --runtime gpu
# ─────────────────────────────────────────────────────────────────────


def test_audit_k_a6_efe_bench_has_runtime_flag():
    rs = (
        _REPO / "crates" / "efe-bench" / "src" / "bin" / "efe_bench.rs"
    ).read_text(encoding="utf-8")
    assert "enum BenchRuntime" in rs
    assert "fn parse_runtime" in rs
    assert 'value_parser = parse_runtime' in rs


# ─────────────────────────────────────────────────────────────────────
# K-A7 : check_finite kernel + binding
# ─────────────────────────────────────────────────────────────────────


def test_audit_k_a7_check_finite_kernel_present():
    cu = _REPO / "crates" / "efe-kernels" / "cuda" / "debug" / "check_finite.cu"
    assert cu.exists(), "K-A7: cuda/debug/check_finite.cu missing"
    text = cu.read_text(encoding="utf-8")
    assert "__global__ void check_finite_kernel" in text
    assert 'extern "C" cudaError_t efe_check_finite_launch' in text


def test_audit_k_a7_rust_binding_exists():
    rs = (
        _REPO / "crates" / "efe-kernels" / "src" / "kernels.rs"
    ).read_text(encoding="utf-8")
    assert "pub fn efe_check_finite_launch" in rs


def test_audit_k_a7_observability_helper_exists():
    rs = (
        _REPO / "crates" / "efe-kernels" / "src" / "observability.rs"
    ).read_text(encoding="utf-8")
    assert "pub fn check_finite_enabled_runtime" in rs


# ─────────────────────────────────────────────────────────────────────
# K-A8 : raw_stream helper
# ─────────────────────────────────────────────────────────────────────


def test_audit_k_a8_raw_stream_helper_present():
    rs = (
        _REPO / "crates" / "efe-kernels" / "src" / "streams.rs"
    ).read_text(encoding="utf-8")
    assert "pub fn raw_stream" in rs


def test_audit_k_a8_backend_uses_raw_stream_not_handle():
    rs = (
        _REPO / "crates" / "efe-kernels" / "src" / "backend.rs"
    ).read_text(encoding="utf-8")
    # The fragile cudarc API (`stream.handle()`) MUST NOT appear
    # any more — operations route through `raw_stream(&stream)`.
    assert "stream.handle()" not in rs, (
        "K-A8 regression: stream.handle() resurfaced — use "
        "crate::streams::raw_stream(&stream) instead"
    )
    assert "crate::streams::raw_stream(" in rs


# ─────────────────────────────────────────────────────────────────────
# K-A9 : README
# ─────────────────────────────────────────────────────────────────────


def test_audit_k_a9_efe_kernels_readme_present():
    f = _REPO / "crates" / "efe-kernels" / "README.md"
    assert f.exists(), "K-A9: efe-kernels/README.md missing"
    text = f.read_text(encoding="utf-8")
    # Must mention the trait + the canonical commands.
    assert "ComputeBackend" in text
    assert "cargo build -p efe-kernels --features cuda" in text


# ─────────────────────────────────────────────────────────────────────
# K-A16 : KernelCallGuard.fail() wired on error path
# ─────────────────────────────────────────────────────────────────────


def test_audit_k_a16_check_kernel_status_takes_mut_guard():
    rs = (
        _REPO / "crates" / "efe-kernels" / "src" / "backend.rs"
    ).read_text(encoding="utf-8")
    # The signature now takes &mut KernelCallGuard.
    assert "guard: &mut crate::observability::KernelCallGuard" in rs
    assert "guard.fail();" in rs


def test_audit_k_a16_backend_uses_mut_guard_pattern():
    rs = (
        _REPO / "crates" / "efe-kernels" / "src" / "backend.rs"
    ).read_text(encoding="utf-8")
    # `let mut g = record_kernel_call(...)` MUST appear ≥ 20 times.
    n = len(re.findall(r'let mut g = record_kernel_call\(', rs))
    assert n >= 20, (
        f"K-A16 regression: only {n} mut-guard sites — every "
        "ComputeBackend method must use `let mut g = record_kernel_call(...)`"
    )
    # The bygone `let _g = record_kernel_call(...)` must be gone.
    n_immut = len(re.findall(r'let _g = record_kernel_call\(', rs))
    assert n_immut == 0, (
        f"K-A16 regression: {n_immut} immut-guard sites remain — "
        "convert to `let mut g`"
    )


# ─────────────────────────────────────────────────────────────────────
# K-A17 (added during audit) : check_kernel_status passes &mut g
# ─────────────────────────────────────────────────────────────────────


def test_audit_k_a17_check_kernel_status_callsites_pass_mut_guard():
    rs = (
        _REPO / "crates" / "efe-kernels" / "src" / "backend.rs"
    ).read_text(encoding="utf-8")
    # Every `check_kernel_status(...)` call must pass `&mut g` as
    # first argument.
    bad_pat = re.compile(r'check_kernel_status\(\s*"')
    matches = bad_pat.findall(rs)
    assert not matches, (
        "K-A17 regression : a `check_kernel_status(\"op\", ...)` "
        "call lost its `&mut g` first argument"
    )
    # Allow either a string literal (`"softmax"`) OR an op variable
    # (used in `launch_rowwise`) as the second arg.
    good_pat = re.compile(r'check_kernel_status\(&mut g,\s+')
    n = len(good_pat.findall(rs))
    assert n >= 10, (
        f"K-A17 regression : only {n} check_kernel_status callsites "
        "with the &mut g first arg ; expected ≥ 10"
    )


# ─────────────────────────────────────────────────────────────────────
# Audit pass 3 — operational readiness review (C-A* defects)
#
# These tests verrouillent les défauts identifiés dans le 3ᵉ pass
# d'audit de D28 — issues that would only surface with `--features cuda`
# (which we cannot test locally without nvcc but can statically
# verify via grep + AST patterns).
# ─────────────────────────────────────────────────────────────────────


def test_audit_c_a1_streams_uses_method_not_field():
    """C-A1 : `streams.rs::raw_stream` must call the public method
    `stream.stream()`, not the private field `stream.stream` (cudarc
    0.10 marks it `pub(crate)`)."""
    rs = (
        _REPO / "crates" / "efe-kernels" / "src" / "streams.rs"
    ).read_text(encoding="utf-8")
    # Field access regex : `stream.stream` followed by a non-method
    # token (whitespace, semicolon, `as`, etc., NOT `(`).
    field_pat = re.compile(r'\bstream\.stream\b(?!\s*\()')
    # Allow comments mentioning the field — strip lines starting with
    # `//`.
    code_lines = [
        line for line in rs.splitlines()
        if not line.lstrip().startswith("//")
    ]
    code_text = "\n".join(code_lines)
    matches = field_pat.findall(code_text)
    assert not matches, (
        "C-A1 regression : `stream.stream` (field access) resurfaced "
        "in code body — must use `stream.stream()` (method call)"
    )
    # Positive : the method form must be present.
    assert "stream.stream()" in rs


def test_audit_c_a12_observability_no_prometheus_type_naming():
    """C-A12 : `observability.rs` must NOT name `prometheus::*` types
    in any signature or return type — `prometheus` is not a direct
    dep of `efe-kernels`, so naming the type breaks the
    `--features cuda` build."""
    rs = (
        _REPO / "crates" / "efe-kernels" / "src" / "observability.rs"
    ).read_text(encoding="utf-8")
    # Allow the string in comments (the explanatory audit doc) but
    # not in code.
    code_lines = [
        line for line in rs.splitlines()
        if not line.lstrip().startswith("//")
    ]
    code_text = "\n".join(code_lines)
    bad_patterns = [
        "prometheus::HistogramVec",
        "prometheus::IntCounterVec",
        "prometheus::IntGaugeVec",
        "prometheus::Histogram",
    ]
    for p in bad_patterns:
        assert p not in code_text, (
            f"C-A12 regression : `{p}` named in observability.rs code "
            f"— must be inlined via `efe_telemetry::metrics::gpu_instruments::*` calls"
        )


def test_audit_c_a12_observability_inlines_gpu_instrument_calls():
    """C-A12 (positive) : the inlined `efe_telemetry::metrics::gpu_instruments::*`
    helper calls must be present, one per metric."""
    rs = (
        _REPO / "crates" / "efe-kernels" / "src" / "observability.rs"
    ).read_text(encoding="utf-8")
    expected = [
        "gpu_kernel_duration_seconds()",
        "gpu_kernel_calls_total()",
        "gpu_oom_total()",
        "gpu_memory_used_bytes()",
        "gpu_nccl_allreduce_seconds()",
    ]
    for fn in expected:
        assert fn in rs, (
            f"C-A12 regression : `{fn}` no longer called in observability.rs"
        )


def test_audit_c_a20_fallback_no_use_after_move():
    """C-A20 : `fallback.rs::probe_device` must NOT have
    `let _ = dev;` (which drops the variable then later code uses it)."""
    rs = (
        _REPO / "crates" / "efe-kernels" / "src" / "fallback.rs"
    ).read_text(encoding="utf-8")
    # Strict pattern : `let _ = dev` (not `let _dev = dev`).
    bad_pat = re.compile(r'\blet\s+_\s*=\s*dev\b')
    code_lines = [
        line for line in rs.splitlines()
        if not line.lstrip().startswith("//")
    ]
    code_text = "\n".join(code_lines)
    matches = bad_pat.findall(code_text)
    assert not matches, (
        "C-A20 regression : `let _ = dev` resurfaced — that consumes "
        "the variable so subsequent uses fail with --features cuda"
    )


def test_audit_c_a21_fallback_no_unused_e_in_closures():
    """C-A21 : `fallback.rs` map_err closures must not bind an unused
    `e` parameter — use `|_|` when `e` is not consumed in the body.
    """
    rs = (
        _REPO / "crates" / "efe-kernels" / "src" / "fallback.rs"
    ).read_text(encoding="utf-8")
    # Iterate `map_err(|e|` callsites ; for each, look at the next
    # 240 chars and verify they contain `{e}` or ` e ` or `{e:`
    # (i.e. `e` is actually consumed). Avoid the brittle nested-brace
    # parsing of the previous version.
    callsite_pat = re.compile(r'map_err\(\|e\|')
    for m in callsite_pat.finditer(rs):
        window = rs[m.end() : m.end() + 240]
        # Stop at the first balanced close of the outer paren group.
        # The simple heuristic : if `{e}` / ` e ` / `{e:` appears in
        # the window before the next `}` at column 0, OK.
        used = ("{e}" in window) or ("{e:" in window) or (" e " in window) or ("{e," in window)
        assert used, (
            "C-A21 regression : closure captures `e` but does not "
            f"use it within 240 chars ; window:\n{window}"
        )


def test_audit_c_a29_memory_no_with_observability_double_count():
    """C-A29 : `memory.rs::alloc_zeros` must NOT call
    `.with_observability("alloc_zeros")` — that double-counts the
    fail outcome with the caller's `KernelCallGuard`."""
    rs = (
        _REPO / "crates" / "efe-kernels" / "src" / "memory.rs"
    ).read_text(encoding="utf-8")
    # Allow it in audit-doc comments but not in code paths.
    code_lines = [
        line for line in rs.splitlines()
        if not line.lstrip().startswith("//")
    ]
    code_text = "\n".join(code_lines)
    bad_pat = re.compile(r'\.with_observability\("alloc_zeros"\)')
    matches = bad_pat.findall(code_text)
    assert not matches, (
        "C-A29 regression : .with_observability(\"alloc_zeros\") "
        "resurfaced in memory.rs — would double-count the fail "
        "outcome with the caller's KernelCallGuard"
    )


def test_audit_c_a30_memory_alloc_zeros_no_default_bound():
    """C-A30 : `memory.rs::alloc_zeros<T>` and `h2d_copy<T>` should
    require only `DeviceRepr` (not `+ Default`) — `Default` was a
    superfluous bound that did not match cudarc's signature and
    constrained callsites needlessly."""
    rs = (
        _REPO / "crates" / "efe-kernels" / "src" / "memory.rs"
    ).read_text(encoding="utf-8")
    # The alloc_zeros bound MUST be exactly `T: DeviceRepr,` (no `+ Default`).
    pat = re.compile(
        r'pub fn alloc_zeros<T>\([^)]*\)\s*->\s*Result<TrackedSlice<T>, CudaError>\s*'
        r'where\s+T:\s*DeviceRepr\s*,',
        re.DOTALL,
    )
    assert pat.search(rs), (
        "C-A30 regression : `alloc_zeros<T>` bound is no longer "
        "`T: DeviceRepr,` — Default was added back inadvertently"
    )
