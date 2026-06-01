"""Sprint **D28** — `efe-kernels` structural pin tests.

Cf. `docs/EFE_KERNELS_ARCHITECTURE.md`.

Asserts the module file tree + extern "C" launcher consistency
between the Rust bindings and the CUDA `.cu` sources. Fires on a
future refactor that removes a kernel without removing its binding
(or vice versa) — the recurring "wiring forgotten" defect class.
"""

from __future__ import annotations

import re
from pathlib import Path

_REPO = Path(__file__).resolve().parents[2]
_KERNELS = _REPO / "crates" / "efe-kernels"


# ─────────────────────────────────────────────────────────────────────
# 1. Cargo.toml structural invariants
# ─────────────────────────────────────────────────────────────────────


def test_cargo_toml_declares_cuda_feature():
    cargo = (_KERNELS / "Cargo.toml").read_text(encoding="utf-8")
    assert "[features]" in cargo
    # The `cuda` feature MUST pull cudarc + cc.
    assert re.search(
        r'cuda\s*=\s*\[[^\]]*"dep:cudarc"[^\]]*"dep:cc"[^\]]*\]',
        cargo,
    ), "cuda feature must pull cudarc and cc as optional deps"


def test_cargo_toml_declares_optional_cudarc():
    cargo = (_KERNELS / "Cargo.toml").read_text(encoding="utf-8")
    assert re.search(
        r'cudarc\s*=\s*\{[^}]*optional\s*=\s*true', cargo
    ), "cudarc must be declared optional"


def test_cargo_toml_declares_build_rs():
    cargo = (_KERNELS / "Cargo.toml").read_text(encoding="utf-8")
    assert 'build = "build.rs"' in cargo


def test_build_rs_exists_and_handles_cuda_feature():
    f = _KERNELS / "build.rs"
    assert f.exists(), "efe-kernels/build.rs must exist (D28-K-A1)"
    text = f.read_text(encoding="utf-8")
    assert "CARGO_FEATURE_CUDA" in text
    # The feature-gated module pattern.
    assert "#[cfg(feature = \"cuda\")]" in text
    assert "cuda_build" in text


# ─────────────────────────────────────────────────────────────────────
# 2. Rust module tree
# ─────────────────────────────────────────────────────────────────────


def test_rust_modules_present():
    expected = [
        "src/lib.rs",
        "src/error.rs",
        "src/observability.rs",
        "src/backend.rs",
        "src/streams.rs",
        "src/memory.rs",
        "src/kernels.rs",
        "src/multi_gpu.rs",
        "src/fallback.rs",
    ]
    for p in expected:
        f = _KERNELS / p
        assert f.exists(), f"missing Rust module: {p}"


def test_lib_rs_exports_cuda_backend_under_feature():
    rs = (_KERNELS / "src" / "lib.rs").read_text(encoding="utf-8")
    assert "pub use backend::CudaBackend" in rs
    # Must be cfg-gated to avoid breaking workspace compile w/o feature.
    assert "#[cfg(feature = \"cuda\")]" in rs
    # `is_cuda_available` const fn ships unconditionally.
    assert "pub const fn is_cuda_available()" in rs


def test_error_taxonomy_carries_cuda_variants():
    rs = (_KERNELS / "src" / "error.rs").read_text(encoding="utf-8")
    for variant in [
        "FromCuda",
        "OutOfMemory",
        "LaunchFailed",
        "Nccl",
        "DeviceUnavailable",
        "UnsupportedCapability",
        "DeterminismViolation",
        "StreamError",
        "KernelTimeout",
        "NotYetWired",
        "InitFailed",
    ]:
        assert variant in rs, f"CudaError missing variant: {variant}"


# ─────────────────────────────────────────────────────────────────────
# 3. CUDA `.cu` files
# ─────────────────────────────────────────────────────────────────────


def test_cuda_layout_categorized():
    expected = {
        "cuda/include/efe.cuh",
        "cuda/include/efe_warp.cuh",
        "cuda/activations/softmax.cu",
        "cuda/activations/log_softmax.cu",
        "cuda/activations/gelu.cu",
        "cuda/activations/sigmoid.cu",
        "cuda/reductions/mean.cu",
        "cuda/reductions/std_dev.cu",
        "cuda/reductions/shannon_entropy.cu",
        "cuda/reductions/argmax.cu",
        "cuda/reductions/topk.cu",
        "cuda/reductions/kl_divergence.cu",
        "cuda/linalg/l2_normalize.cu",
        "cuda/linalg/l2_distance.cu",
        "cuda/linalg/cosine.cu",
        "cuda/efe_core/cone_project.cu",
        "cuda/efe_core/weighted_mean.cu",
        "cuda/efe_core/moe_reconcile.cu",
        "cuda/attention/scaled_dot_product_attention.cu",
        "cuda/sampling/multinomial_sample.cu",
    }
    for rel in expected:
        f = _KERNELS / rel
        assert f.exists(), f"missing CUDA file: {rel}"


def test_efe_cuh_has_required_constants():
    cuh = (_KERNELS / "cuda" / "include" / "efe.cuh").read_text(encoding="utf-8")
    for c in ["EFE_TENSOR_ALIGNMENT", "EFE_EPS", "EFE_INIT_SIGMA",
              "EFE_KL_CAP", "EFE_TILE_MATMUL", "EFE_TILE_SOFTMAX",
              "EFE_TILE_ATTENTION", "EFE_KERNELS_ABI_VERSION"]:
        assert c in cuh, f"efe.cuh missing constant: {c}"


def test_efe_warp_cuh_has_block_reductions():
    cuh = (_KERNELS / "cuda" / "include" / "efe_warp.cuh").read_text(encoding="utf-8")
    for fn in ["warp_reduce_sum", "warp_reduce_max", "warp_reduce_argmax",
               "block_reduce_sum", "block_reduce_max", "block_reduce_argmax"]:
        assert fn in cuh, f"efe_warp.cuh missing helper: {fn}"


# ─────────────────────────────────────────────────────────────────────
# 4. extern "C" launcher consistency Rust ↔ CUDA
#    For every `extern "C" cudaError_t efe_<op>_launch(...)` in a `.cu`,
#    there must be a matching `pub fn efe_<op>_launch(...)` in
#    `kernels.rs`. Catches the "kernel added but never bound" or
#    "binding renamed but kernel not updated" defect class.
# ─────────────────────────────────────────────────────────────────────


def _list_extern_c_launchers_cuda() -> set[str]:
    """Walk the `.cu` files and collect every `extern "C"` function name."""
    names: set[str] = set()
    pat = re.compile(r'extern\s+"C"\s+cudaError_t\s+(\w+)\s*\(')
    for cu in (_KERNELS / "cuda").rglob("*.cu"):
        text = cu.read_text(encoding="utf-8")
        for m in pat.finditer(text):
            names.add(m.group(1))
    return names


def _list_extern_c_launchers_rust() -> set[str]:
    """Walk `kernels.rs` and collect every `pub fn efe_*_launch` declared."""
    rs = (_KERNELS / "src" / "kernels.rs").read_text(encoding="utf-8")
    pat = re.compile(r'pub\s+fn\s+(efe_\w+_launch)\s*\(')
    return {m.group(1) for m in pat.finditer(rs)}


def test_cuda_extern_c_launchers_match_rust_bindings():
    cuda_set = _list_extern_c_launchers_cuda()
    rust_set = _list_extern_c_launchers_rust()
    cuda_only = cuda_set - rust_set
    rust_only = rust_set - cuda_set
    assert not cuda_only, (
        f"CUDA launchers without Rust bindings (D28-K-A2 regression): {cuda_only}"
    )
    assert not rust_only, (
        f"Rust bindings without CUDA launcher (D28-K-A3 regression): {rust_only}"
    )


def test_cuda_launchers_at_least_15():
    """Sanity check : we shipped at least 15 launchers in D28."""
    cuda_set = _list_extern_c_launchers_cuda()
    # 1 noop + 4 activations + 6 reductions + 3 linalg + 3 efe_core + 1 sdpa + 1 sampling = 19
    assert len(cuda_set) >= 15, (
        f"only {len(cuda_set)} launchers found, expected ≥ 15"
    )


# ─────────────────────────────────────────────────────────────────────
# 5. ComputeBackend trait coverage
#    The `CudaBackend` impl in `backend.rs` must expose all 23 methods
#    of the trait — even if some return `NotYetWired`. Catches the
#    "trait extended but impl forgotten" defect class.
# ─────────────────────────────────────────────────────────────────────


def test_cuda_backend_implements_all_trait_methods():
    backend_rs = (_KERNELS / "src" / "backend.rs").read_text(encoding="utf-8")

    expected_fns = [
        "device", "name", "synchronize",
        "matmul", "matmul_fused_gelu", "dot",
        "l2_norm", "l2_normalize", "l2_distance",
        "softmax", "log_softmax", "gelu", "sigmoid",
        "scaled_dot_product_attention",
        "topk", "argmax",
        "cone_project", "weighted_mean", "moe_reconcile",
        "cosine_similarity_pair", "cosine_anchor_blend",
        "multinomial_sample", "kl_divergence",
        "shannon_entropy", "mean", "std_dev",
    ]
    for fn in expected_fns:
        # Match `fn <name>(` definitions, not call sites.
        assert re.search(rf'\bfn\s+{fn}\s*\(', backend_rs), (
            f"CudaBackend missing trait method: {fn}"
        )


# ─────────────────────────────────────────────────────────────────────
# 6. Tests directory
# ─────────────────────────────────────────────────────────────────────


def test_cross_backend_test_files_present():
    expected = [
        "tests/integration_smoke.rs",
        "tests/common/mod.rs",
        "tests/cross_backend_softmax.rs",
        "tests/cross_backend_cone_project.rs",
        "tests/cross_backend_weighted_mean.rs",
        "tests/cross_backend_mean_std_dev.rs",
        "tests/cross_backend_l2_normalize.rs",
        "tests/cross_backend_activations.rs",
        "tests/cross_backend_argmax_entropy.rs",
    ]
    for p in expected:
        f = _KERNELS / p
        assert f.exists(), f"missing test file: {p}"
