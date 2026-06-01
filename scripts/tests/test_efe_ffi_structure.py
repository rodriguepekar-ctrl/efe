"""Sprint **D25** — `efe-ffi` structural pin tests.

Cf. `docs/EFE_FFI_ARCHITECTURE.md`.

Asserts the module file tree + Cargo.toml deps + pyproject.toml
maturin config + the public `_native.pyi` stub stays in sync with
the Rust public API. Catches the recurring "wiring forgotten"
defect class for the FFI-specific surface.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

_REPO = Path(__file__).resolve().parents[2]
_FFI = _REPO / "crates" / "efe-ffi"


# ─────────────────────────────────────────────────────────────────
# 1. Cargo.toml structural invariants
# ─────────────────────────────────────────────────────────────────


def test_cargo_toml_declares_cdylib():
    cargo = (_FFI / "Cargo.toml").read_text(encoding="utf-8")
    # `crate-type = ["cdylib", "rlib"]` MUST be present so maturin
    # can produce a Python-loadable extension.
    assert re.search(
        r'crate-type\s*=\s*\[\s*"cdylib"', cargo
    ), "efe-ffi Cargo.toml must declare crate-type cdylib"


def test_cargo_toml_pyo3_021_pinned():
    cargo = (_FFI / "Cargo.toml").read_text(encoding="utf-8")
    # PyO3 0.21+ pinned (matches the architecture commitment).
    assert re.search(
        r'pyo3\s*=\s*\{\s*version\s*=\s*"0\.21"', cargo
    ), "efe-ffi Cargo.toml must pin pyo3 = 0.21"
    assert '"extension-module"' in cargo
    assert '"abi3-py310"' in cargo


def test_cargo_toml_numpy_021_present():
    cargo = (_FFI / "Cargo.toml").read_text(encoding="utf-8")
    assert re.search(r'numpy\s*=\s*"0\.21"', cargo), (
        "efe-ffi Cargo.toml must pin numpy = 0.21"
    )


def test_cargo_toml_async_runtimes_present():
    cargo = (_FFI / "Cargo.toml").read_text(encoding="utf-8")
    assert "pyo3-async-runtimes" in cargo


# ─────────────────────────────────────────────────────────────────
# 2. pyproject.toml maturin config
# ─────────────────────────────────────────────────────────────────


def test_pyproject_toml_present():
    f = _FFI / "pyproject.toml"
    assert f.exists(), "crates/efe-ffi/pyproject.toml missing — maturin entry"


def test_pyproject_module_name_efe_native():
    pp = (_FFI / "pyproject.toml").read_text(encoding="utf-8")
    assert 'module-name = "efe._native"' in pp


def test_pyproject_python_source_python():
    pp = (_FFI / "pyproject.toml").read_text(encoding="utf-8")
    assert 'python-source = "python"' in pp


def test_pyproject_manylinux_2_17():
    pp = (_FFI / "pyproject.toml").read_text(encoding="utf-8")
    assert 'manylinux = "2_17"' in pp


def test_pyproject_requires_python_310_plus():
    pp = (_FFI / "pyproject.toml").read_text(encoding="utf-8")
    assert 'requires-python = ">=3.10"' in pp


def test_pyproject_lists_python_versions():
    pp = (_FFI / "pyproject.toml").read_text(encoding="utf-8")
    for ver in ["3.10", "3.11", "3.12", "3.13"]:
        assert f'"Programming Language :: Python :: {ver}"' in pp, (
            f"pyproject must declare Python {ver} classifier"
        )


# ─────────────────────────────────────────────────────────────────
# 3. Rust source file tree
# ─────────────────────────────────────────────────────────────────


def test_rust_modules_present():
    expected = [
        "src/lib.rs",
        "src/error.rs",
        "src/verdict.rs",
        "src/bundle.rs",
        "src/types.rs",
        "src/els.rs",
        "src/controller.rs",
        "src/client.rs",
        "src/classify.rs",
        "src/async_.rs",
    ]
    for p in expected:
        assert (_FFI / p).exists(), f"missing Rust module: {p}"


def test_lib_rs_declares_pymodule():
    rs = (_FFI / "src" / "lib.rs").read_text(encoding="utf-8")
    assert "#[pymodule]" in rs
    assert 'name = "_native"' in rs


def test_lib_rs_adds_classes():
    rs = (_FFI / "src" / "lib.rs").read_text(encoding="utf-8")
    for fragment in [
        "PyVerdict",
        "PyBundle",
        "PyEpistemicLatentSpace",
        "PyEfeNativeClient",
        "PySwarmController",
        "PySwarmSpec",
        "PyAlignmentCone",
    ]:
        assert fragment in rs, f"lib.rs no longer add_class::<{fragment}>"


def test_lib_rs_wraps_top_level_functions():
    rs = (_FFI / "src" / "lib.rs").read_text(encoding="utf-8")
    for fn in [
        "wrap_pyfunction!(version",
        "wrap_pyfunction!(classify::classify_zero_copy",
        "wrap_pyfunction!(async_::aclassify_zero_copy",
        "wrap_pyfunction!(bundle::load_bundle",
    ]:
        assert fn in rs, f"lib.rs no longer wraps {fn}"


def test_lib_rs_registers_exceptions():
    rs = (_FFI / "src" / "lib.rs").read_text(encoding="utf-8")
    assert "error::register_exceptions" in rs


# ─────────────────────────────────────────────────────────────────
# 4. Exception hierarchy
# ─────────────────────────────────────────────────────────────────


def test_error_rs_declares_10_exceptions():
    rs = (_FFI / "src" / "error.rs").read_text(encoding="utf-8")
    expected = [
        "EfeBaseError",
        "BundleError",
        "CatalogError",
        "SwarmError",
        "AlignmentBreach",
        "NumericalError",
        "ResourceExhausted",
        "ConfigError",
        "TimeoutError",
        "FfiError",
    ]
    for cls in expected:
        # Each exception must appear in a `create_exception!` macro.
        pat = re.compile(rf'create_exception!\s*\(\s*efe_native\s*,\s*{cls}\b')
        assert pat.search(rs), f"create_exception!(...{cls}...) missing in error.rs"


def test_error_rs_register_all_10():
    rs = (_FFI / "src" / "error.rs").read_text(encoding="utf-8")
    assert "register_exceptions" in rs
    # 10 m.add(...) calls — one per exception class.
    n = len(re.findall(r'm\.add\("\w+",\s*py\.get_type_bound', rs))
    assert n == 10, f"expected 10 m.add calls, got {n}"


def test_error_rs_efe_error_python_facing_name():
    """Python users see `EfeError` (not `EfeBaseError`) — the Rust
    type was renamed to avoid colliding with `efe_core::EfeError`."""
    rs = (_FFI / "src" / "error.rs").read_text(encoding="utf-8")
    assert 'm.add("EfeError"' in rs, (
        "error.rs must register the Python-facing alias 'EfeError'"
    )


# ─────────────────────────────────────────────────────────────────
# 5. Python package layout
# ─────────────────────────────────────────────────────────────────


def test_python_package_files_present():
    expected = [
        "python/efe/__init__.py",
        "python/efe/_native.pyi",
        "python/efe/py.typed",
    ]
    for p in expected:
        assert (_FFI / p).exists(), f"missing Python file: {p}"


def test_init_py_re_exports_native_surface():
    init = (_FFI / "python" / "efe" / "__init__.py").read_text(encoding="utf-8")
    expected = [
        "Verdict",
        "Bundle",
        "EpistemicLatentSpace",
        "EfeNativeClient",
        "SwarmController",
        "SwarmSpec",
        "AlignmentCone",
        "EfeError",
        "BundleError",
        "AlignmentBreach",
        "classify_zero_copy",
        "aclassify_zero_copy",
        "load_bundle",
        "version",
    ]
    for sym in expected:
        assert sym in init, f"__init__.py no longer re-exports {sym}"


def test_init_py_optional_grpc_re_export():
    init = (_FFI / "python" / "efe" / "__init__.py").read_text(encoding="utf-8")
    # The historical EfeClient gRPC import must be optional + non-fatal.
    assert "from efe_tool import EfeClient" in init
    assert "ImportError" in init


# ─────────────────────────────────────────────────────────────────
# 6. .pyi stub sync with Rust public API
# ─────────────────────────────────────────────────────────────────


def test_pyi_declares_all_classes():
    pyi = (_FFI / "python" / "efe" / "_native.pyi").read_text(encoding="utf-8")
    expected = [
        "class Verdict:",
        "class Bundle:",
        "class EpistemicLatentSpace:",
        "class EfeNativeClient:",
        "class SwarmController:",
        "class SwarmSpec:",
        "class AlignmentCone:",
        "class EfeError(Exception):",
        "class BundleError(EfeError):",
        "class AlignmentBreach(EfeError):",
    ]
    for cls in expected:
        assert cls in pyi, f"_native.pyi no longer declares: {cls}"


def test_pyi_declares_all_functions():
    pyi = (_FFI / "python" / "efe" / "_native.pyi").read_text(encoding="utf-8")
    expected = [
        "def version() -> str:",
        "def load_bundle(path: str) -> Bundle:",
        "def classify_zero_copy(",
        "def aclassify_zero_copy(",
    ]
    for fn in expected:
        assert fn in pyi, f"_native.pyi no longer declares: {fn}"


# ─────────────────────────────────────────────────────────────────
# 7. Tests directory + bench
# ─────────────────────────────────────────────────────────────────


def test_python_test_files_present():
    expected = [
        "python/efe/tests/__init__.py",
        "python/efe/tests/conftest.py",
        "python/efe/tests/test_smoke.py",
        "python/efe/tests/test_classify.py",
        "python/efe/tests/test_zero_copy.py",
        "python/efe/tests/test_async.py",
        "python/efe/tests/test_errors.py",
        "python/efe/tests/test_determinism.py",
        "python/efe/tests/test_gil_release.py",
        "python/efe/tests/bench/native_vs_grpc.py",
    ]
    for p in expected:
        assert (_FFI / p).exists(), f"missing Python test file: {p}"


def test_bench_marked_for_pytest():
    bench = (_FFI / "python" / "efe" / "tests" / "bench" / "native_vs_grpc.py").read_text(encoding="utf-8")
    assert "pytestmark = pytest.mark.bench" in bench


# ─────────────────────────────────────────────────────────────────
# 8. CI workflows
# ─────────────────────────────────────────────────────────────────


def test_ci_workflow_ffi_test_present():
    f = _REPO / ".github" / "workflows" / "ffi-test.yml"
    assert f.exists(), "ffi-test.yml CI workflow missing"
    text = f.read_text(encoding="utf-8")
    # Matrix : 3 OS × 2 Python.
    for os in ["ubuntu-22.04", "macos-14", "windows-2022"]:
        assert os in text, f"ffi-test.yml missing matrix entry: {os}"
    for py in ['"3.10"', '"3.12"']:
        assert py in text, f"ffi-test.yml missing python {py}"


def test_ci_workflow_ffi_publish_present():
    f = _REPO / ".github" / "workflows" / "ffi-publish.yml"
    assert f.exists(), "ffi-publish.yml CI workflow missing"
    text = f.read_text(encoding="utf-8")
    # 5 wheels + sdist + publish jobs.
    expected_jobs = [
        "manylinux-x86_64:",
        "manylinux-aarch64:",
        "macos-arm64:",
        "macos-x86_64:",
        "windows-x86_64:",
        "sdist:",
        "publish:",
    ]
    for j in expected_jobs:
        assert j in text, f"ffi-publish.yml missing job: {j}"


def test_ci_publish_uses_maturin_action():
    pub = (_REPO / ".github" / "workflows" / "ffi-publish.yml").read_text(encoding="utf-8")
    assert "PyO3/maturin-action@v1" in pub


def test_ci_publish_targets_match_architecture():
    pub = (_REPO / ".github" / "workflows" / "ffi-publish.yml").read_text(encoding="utf-8")
    expected_targets = [
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "x86_64-pc-windows-msvc",
    ]
    for t in expected_targets:
        assert t in pub, f"ffi-publish.yml missing target: {t}"


# ─────────────────────────────────────────────────────────────────
# 9. Audit pass 1 régression (F-A* defects from D25 audit)
# ─────────────────────────────────────────────────────────────────


def test_audit_f_a1_swarm_spec_builder_added_to_module():
    """F-A1 : `m.add_class::<PySwarmSpecBuilder>()` MUST be present
    in lib.rs ; otherwise Python `SwarmSpec.builder()` returns an
    unregistered type."""
    rs = (_FFI / "src" / "lib.rs").read_text(encoding="utf-8")
    assert "PySwarmSpecBuilder" in rs, (
        "F-A1 regression : lib.rs no longer adds PySwarmSpecBuilder ; "
        "Python users get TypeError on SwarmSpec.builder()"
    )
    assert "m.add_class::<types::PySwarmSpecBuilder>" in rs


def test_audit_f_a2_controller_latency_measured_inside_closure():
    """F-A2 : `controller.rs::execute` MUST measure `started.elapsed()`
    INSIDE `allow_threads`, not before. Pre-fix the latency was always
    ~0 ms because `started` was captured before the move."""
    rs = (_FFI / "src" / "controller.rs").read_text(encoding="utf-8")
    # We don't bind started OUTSIDE allow_threads any more.
    # Search for the bad pattern : a `let started = Instant::now()`
    # followed by `py.allow_threads(move ||` without the started
    # being declared inside.
    bad_pat = re.compile(
        r'fn execute\b[^{]*\{[^}]*?let started = Instant::now\(\);[^}]*?py\.allow_threads\(move \|\|',
        re.DOTALL,
    )
    matches = bad_pat.findall(rs)
    assert not matches, (
        "F-A2 regression : `Instant::now()` captured before "
        "`allow_threads(move || ...)` makes latency ~0 ms"
    )
    # Positive : the closure body MUST have its own `let started`.
    good_pat = re.compile(
        r'py\.allow_threads\(move \|\| \{\s*let started = Instant::now\(\);'
    )
    assert good_pat.search(rs), (
        "F-A2 regression : expected `let started = Instant::now();` "
        "inside the `allow_threads` closure"
    )


def test_audit_f_a3_controller_exposes_labels_accessor():
    """F-A3 : `PySwarmController::labels() -> Arc<Vec<String>>`
    MUST be a public accessor accessible from `client.rs` ;
    otherwise `synthesise_via_helper` falls back to fake labels."""
    rs = (_FFI / "src" / "controller.rs").read_text(encoding="utf-8")
    assert "pub fn labels(" in rs, (
        "F-A3 regression : `pub fn labels()` accessor missing from "
        "controller.rs ; client.rs fabricates label-0, label-1, …"
    )


def test_audit_f_a3_client_uses_real_labels():
    """F-A3 (positive) : client.rs::synthesise_via_helper must call
    `controller.labels()`, not `controller.n_labels()` then fabricate."""
    rs = (_FFI / "src" / "client.rs").read_text(encoding="utf-8")
    assert "controller.labels()" in rs, (
        "F-A3 regression : client.rs no longer calls controller.labels()"
    )
    # The fake-label loop must be gone.
    bad_pat = re.compile(r'\(0\.\.controller\.n_labels\(\)\)\s*\.map\(\|i\| format!\("label-')
    assert not bad_pat.search(rs), (
        "F-A3 regression : the fake `format!(\"label-{i}\")` "
        "loop resurfaced"
    )


def test_audit_f_a4_synthesise_uses_embedding_not_request_id():
    """F-A4 : determinism (SLO F-12) requires
    `synthesise_verdict(embedding, catalog_id, version, labels, seed)`
    — NOT request_id (which is unique per call).

    **Audit fix Φ-C-FFI** : visibility bumped from `pub(crate)` to
    `pub` to allow the Criterion bench (`benches/ffi_overhead.rs`)
    to call it directly. The pin test was anchored on the exact
    `pub(crate)` token and now needs to accept both. The key invariant
    preserved : the signature takes `embedding: &[f32]` first, not
    `request_id: &str`.
    """
    rs = (_FFI / "src" / "controller.rs").read_text(encoding="utf-8")
    # The signature must take an embedding (not a request_id string).
    # Accept either `pub fn` (Φ-C-FFI bench-friendly) or `pub(crate) fn`
    # (D25 original).
    pat = re.compile(
        r'pub(?:\(crate\))? fn synthesise_verdict\(\s*embedding: &\[f32\]',
    )
    assert pat.search(rs), (
        "F-A4 regression : synthesise_verdict signature no longer "
        "takes `embedding: &[f32]` as first arg ; determinism broken"
    )
    # `request_id.as_bytes()` MUST NOT appear in the body.
    assert "request_id.as_bytes()" not in rs, (
        "F-A4 regression : `hasher.update(request_id.as_bytes())` "
        "resurfaced — that breaks determinism (request_id is unique "
        "per call)"
    )


def test_audit_f_a5_no_unused_ffi_err_in_bundle():
    """F-A5 : `ffi_err` import was unused in bundle.rs ; must be
    removed."""
    rs = (_FFI / "src" / "bundle.rs").read_text(encoding="utf-8")
    # The import line must NOT include ffi_err.
    bad_pat = re.compile(r'use crate::error::\{[^}]*ffi_err[^}]*\}')
    assert not bad_pat.search(rs), (
        "F-A5 regression : `ffi_err` resurfaced in bundle.rs imports "
        "without being used"
    )


def test_audit_f_a6_no_unused_catalog_err_in_controller():
    """F-A6 : `catalog_err` import was unused in controller.rs ;
    must be removed."""
    rs = (_FFI / "src" / "controller.rs").read_text(encoding="utf-8")
    bad_pat = re.compile(r'use crate::error::\{[^}]*catalog_err[^}]*\}')
    assert not bad_pat.search(rs), (
        "F-A6 regression : `catalog_err` resurfaced in controller.rs "
        "imports without being used"
    )


def test_audit_f_a7_no_unused_pyarray_methods():
    """F-A7 : `PyArrayMethods` import was unused in els.rs ; must
    be removed."""
    rs = (_FFI / "src" / "els.rs").read_text(encoding="utf-8")
    bad_pat = re.compile(r'use numpy::\{[^}]*PyArrayMethods[^}]*\}')
    assert not bad_pat.search(rs), (
        "F-A7 regression : `PyArrayMethods` resurfaced in els.rs imports"
    )


def test_audit_f_a9_dead_code_attribute_on_synthetic():
    """F-A9 : `els::PyEpistemicLatentSpace::synthetic` must be
    annotated `#[allow(dead_code)]` until DT-F-ELS-1 (D27) wires
    a real snapshot path."""
    rs = (_FFI / "src" / "els.rs").read_text(encoding="utf-8")
    # The pattern : `#[allow(dead_code)]` immediately above
    # `pub(crate) fn synthetic`.
    pat = re.compile(
        r'#\[allow\(dead_code\)\]\s*\n\s*pub\(crate\) fn synthetic\(',
    )
    assert pat.search(rs), (
        "F-A9 regression : `#[allow(dead_code)]` attribute missing "
        "above `synthetic` ; warning resurfaces"
    )


def test_audit_f_a10_module_level_allow_missing_docs_in_error():
    """F-A10 : `#![allow(missing_docs)]` at module level in
    error.rs to silence the create_exception! macro warnings."""
    rs = (_FFI / "src" / "error.rs").read_text(encoding="utf-8")
    assert "#![allow(missing_docs)]" in rs, (
        "F-A10 regression : missing_docs allow attribute removed ; "
        "create_exception! macro warnings resurface"
    )


# ────────────────────────────────────────────────────────────────────
# Φ-C-FFI Completion (D28) — pin tests for the new artefacts.
# Cf. docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md § 10 + § 16.
# ────────────────────────────────────────────────────────────────────

import ast
import json


# Canonical symbol manifest — source of truth for the FFI surface.
# Any change here MUST be accompanied by corresponding edits in :
#   * `crates/efe-ffi/python/efe/_native.pyi`
#   * `crates/efe-ffi/python/efe/__init__.py`
#   * `crates/efe-ffi/src/lib.rs::efe_native()`
CANONICAL_FUNCTIONS_PHI_C = {
    "version",
    "load_bundle",
    "classify_zero_copy",
    "aclassify_zero_copy",
}

CANONICAL_CLASSES_PHI_C = {
    "Verdict",
    "Bundle",
    "EpistemicLatentSpace",
    "EfeNativeClient",
    "SwarmController",
    "SwarmSpec",
    "SwarmSpecBuilder",
    "AlignmentCone",
}

CANONICAL_EXCEPTIONS_PHI_C = {
    "EfeError",
    "BundleError",
    "CatalogError",
    "SwarmError",
    "AlignmentBreach",
    "NumericalError",
    "ResourceExhausted",
    "ConfigError",
    "TimeoutError",
    "FfiError",
}


def _parse_pyi_phi_c() -> tuple[set[str], set[str]]:
    pyi = _FFI / "python" / "efe" / "_native.pyi"
    if not pyi.exists():
        return set(), set()
    tree = ast.parse(pyi.read_text())
    funcs: set[str] = set()
    classes: set[str] = set()
    for node in ast.iter_child_nodes(tree):
        if isinstance(node, ast.FunctionDef):
            funcs.add(node.name)
        elif isinstance(node, ast.ClassDef):
            classes.add(node.name)
    return funcs, classes


def test_phi_c_ffi_stub_pyi_in_sync():
    """**SLO-FFI-STUB-SYNC** (Φ-C-FFI artefact #20).

    Vérifie que la surface canonique est entièrement déclarée dans
    `_native.pyi`. Sans ce gate, un dev peut ajouter un `#[pyclass]`
    côté Rust sans mettre à jour le stub → users mypy reçoivent
    une signature périmée.
    """
    funcs, classes = _parse_pyi_phi_c()
    missing_funcs = CANONICAL_FUNCTIONS_PHI_C - funcs
    missing_classes = (CANONICAL_CLASSES_PHI_C | CANONICAL_EXCEPTIONS_PHI_C) - classes
    passed = not missing_funcs and not missing_classes

    # Emit SLO compliance JSON for check_slo.py.
    out_dir = _REPO / "target" / "ffi"
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "stub_sync.json").write_text(
        json.dumps(
            {
                "slo_compliance": {
                    "SLO-FFI-STUB-SYNC": {
                        "measured_pass_count": 1 if passed else 0,
                        "measured_expected_count": 1,
                        "missing_functions": sorted(missing_funcs),
                        "missing_classes": sorted(missing_classes),
                    }
                }
            },
            indent=2,
        )
        + "\n"
    )

    assert not missing_funcs, (
        f"SLO-FFI-STUB-SYNC violated : missing functions in .pyi : "
        f"{sorted(missing_funcs)}"
    )
    assert not missing_classes, (
        f"SLO-FFI-STUB-SYNC violated : missing classes/exceptions in .pyi : "
        f"{sorted(missing_classes)}"
    )


def test_phi_c_ffi_init_py_re_exports_canonical_surface():
    """`__init__.py` must re-export every canonical symbol so
    `from efe import X` works for all surface items.
    """
    init_py = _FFI / "python" / "efe" / "__init__.py"
    src = init_py.read_text()
    m = re.search(r"from efe\._native import \(([^)]+)\)", src, re.DOTALL)
    assert m is not None, "__init__.py does not have a `from efe._native import (...)` block"
    block = m.group(1)
    imported = {
        name.strip().rstrip(",")
        for name in re.findall(r"[A-Za-z_]\w*", block)
        if not name.strip().rstrip(",").startswith("_")
    }
    expected = (
        CANONICAL_FUNCTIONS_PHI_C
        | CANONICAL_CLASSES_PHI_C
        | CANONICAL_EXCEPTIONS_PHI_C
    )
    missing = expected - imported
    assert not missing, (
        f"__init__.py does not re-export : {sorted(missing)}"
    )


def test_phi_c_ffi_lib_rs_registers_canonical_surface():
    """`lib.rs::efe_native()` must register every canonical function
    + class. Catches the case where a new pyfunction is declared but
    forgotten in the #[pymodule] block.
    """
    rs = (_FFI / "src" / "lib.rs").read_text(encoding="utf-8")
    fn_pat = re.compile(r"wrap_pyfunction!\(\s*(?:\w+::)?(\w+)\s*,")
    fns = set(fn_pat.findall(rs))
    cls_pat = re.compile(r"m\.add_class::<\s*(?:\w+::)?Py(\w+)\s*>")
    classes = set(cls_pat.findall(rs))
    missing_funcs = CANONICAL_FUNCTIONS_PHI_C - fns
    missing_classes = CANONICAL_CLASSES_PHI_C - classes
    assert not missing_funcs, (
        f"lib.rs::efe_native() missing function registrations : "
        f"{sorted(missing_funcs)}"
    )
    assert not missing_classes, (
        f"lib.rs::efe_native() missing class registrations : "
        f"{sorted(missing_classes)}"
    )


def test_phi_c_ffi_telemetry_module_exists():
    """Φ-C-FFI artefact #03 : the `ffi_instruments.rs` module must
    exist in efe-telemetry."""
    p = _REPO / "crates" / "efe-telemetry" / "src" / "metrics" / "ffi_instruments.rs"
    assert p.exists(), f"FFI telemetry instruments missing : {p}"
    src = p.read_text(encoding="utf-8")
    # Verify the 8 expected instrument helpers are declared.
    for sym in [
        "ffi_classify_total",
        "ffi_classify_seconds",
        "ffi_gil_held_seconds",
        "ffi_gil_released_seconds",
        "ffi_marshal_in_seconds",
        "ffi_marshal_out_seconds",
        "ffi_bundle_cache_hits_total",
        "ffi_wheel_load_seconds",
    ]:
        assert sym in src, (
            f"FFI instrument `{sym}` missing from ffi_instruments.rs"
        )
    # And the register_all_ffi() helper.
    assert "fn register_all_ffi" in src


def test_phi_c_ffi_cache_module_exists():
    """Φ-C-FFI artefact #05 : the `cache.rs` LRU module must exist."""
    p = _FFI / "src" / "cache.rs"
    assert p.exists(), f"FFI cache module missing : {p}"
    src = p.read_text(encoding="utf-8")
    for sym in ["BundleCache", "CacheKey", "global", "LRU_CAPACITY"]:
        assert sym in src, f"FFI cache symbol `{sym}` missing"


def test_phi_c_ffi_slo_yaml_exists_and_parses():
    """Φ-C-FFI artefact #01 : ffi_slos.yaml must exist and parse OK."""
    yaml_mod = pytest.importorskip("yaml")
    p = _REPO / "bench_baselines" / "v0.6.0" / "ffi_slos.yaml"
    assert p.exists(), f"SLO YAML missing : {p}"
    data = yaml_mod.safe_load(p.read_text())
    assert data.get("domain") == "ffi"
    slo_ids = {s["id"] for s in data["slos"]}
    expected_slo_ids = {
        "SLO-FFI-LATENCY-P99",
        "SLO-FFI-SPEEDUP",
        "SLO-FFI-COLD-START",
        "SLO-FFI-EQUIVALENCE",
        "SLO-FFI-DETERMINISM",
        "SLO-FFI-WHEEL-SIZE",
        "SLO-FFI-MEMORY-DRIFT",
        "SLO-FFI-GIL-RELEASED",
        "SLO-FFI-WHEEL-COUNT",
        "SLO-FFI-ABI3",
        "SLO-FFI-SBOM",
        "SLO-FFI-SIGSTORE",
        "SLO-FFI-STUB-SYNC",
        "SLO-FFI-CROSS-PY",
        "SLO-FFI-DROPIN",
    }
    missing = expected_slo_ids - slo_ids
    assert not missing, (
        f"FFI SLO YAML missing entries : {sorted(missing)}"
    )
    assert len(slo_ids) == 15, (
        f"expected 15 SLO entries, got {len(slo_ids)}"
    )


def test_phi_c_ffi_check_slo_parses_threshold_pass_count():
    """**Pattern Φ-D.A1 anti-regression** — `check_slo.py` must
    recognise `threshold_pass_count` and `expected_count` fields,
    sinon 6 SLO Φ-C-FFI seraient silencieusement skippés."""
    check_slo = _REPO / "eval" / "scripts" / "check_slo.py"
    src = check_slo.read_text(encoding="utf-8")
    assert "threshold_pass_count" in src, (
        "check_slo.py must parse `threshold_pass_count` (= Φ-C-FFI "
        "SLO-FFI-EQUIVALENCE/DETERMINISM/STUB-SYNC/CROSS-PY/DROPIN field)"
    )
    assert "expected_count" in src, (
        "check_slo.py must parse `expected_count` field"
    )
    # Defensive : verify there's an actual evaluation branch, not just
    # the field declaration. The branch matches against
    # `slo.threshold_pass_count is not None`.
    assert "slo.threshold_pass_count is not None" in src, (
        "check_slo.py has the field but no evaluation branch — "
        "pattern Φ-D.A1 silently skipped SLO. Reject."
    )


def test_phi_c_ffi_runbooks_exist():
    """Φ-C-FFI artefact #26-29 : 4 runbooks must exist for ops."""
    runbook_dir = _REPO / "docs" / "runbook"
    expected = [
        "ffi_gil_contention.md",
        "ffi_python_version_drift.md",
        "ffi_wheel_load_failure.md",
        "ffi_perf_regression.md",
    ]
    missing = [name for name in expected if not (runbook_dir / name).exists()]
    assert not missing, (
        f"Φ-C-FFI runbooks missing : {missing} (expected under {runbook_dir})"
    )


def test_phi_c_ffi_scripts_exist():
    """Φ-C-FFI artefact #18,19,21,22 : 4 bench scripts must exist."""
    scripts_dir = _REPO / "scripts" / "bench"
    expected = [
        "verify_wheel_size.py",
        "verify_abi3.py",
        "criterion_ffi_to_slo.py",
        "freeze_numerical_equiv.py",
    ]
    missing = [name for name in expected if not (scripts_dir / name).exists()]
    assert not missing, (
        f"Φ-C-FFI scripts missing : {missing} (expected under {scripts_dir})"
    )


def test_phi_c_ffi_workflows_exist():
    """Φ-C-FFI artefact #23 : ffi-bench.yml must exist."""
    wf_dir = _REPO / ".github" / "workflows"
    assert (wf_dir / "ffi-bench.yml").exists(), (
        "Φ-C-FFI workflow ffi-bench.yml missing"
    )


def test_phi_c_ffi_fixtures_have_100_entries():
    """Φ-C-FFI artefact #09 : numerical_equiv.json must have 100 fixtures.

    **Audit fix Φ-C-FFI.A3** : pre-fix le test faisait `pytest.skip()`
    si le fichier était absent → si quelqu'un supprime
    accidentellement le fixture file, le pin test skip silencieusement
    → CI vert mais SLO-FFI-EQUIVALENCE inopérant (= replay exact du
    pattern Φ-D.A1).

    Post-fix : `assert p.exists()` → CI rouge immédiatement. Pour
    régénérer le fichier après modification du formula stub D25 :
    `python scripts/bench/freeze_numerical_equiv.py`.
    """
    p = (
        _FFI
        / "python"
        / "efe"
        / "tests"
        / "fixtures"
        / "numerical_equiv.json"
    )
    assert p.exists(), (
        f"FATAL : numerical_equiv.json missing at {p}. "
        f"Run `python scripts/bench/freeze_numerical_equiv.py --out {p}` to regenerate."
    )
    data = json.loads(p.read_text())
    assert data["count"] == 100, f"fixture count = {data['count']}, expected 100"
    assert len(data["fixtures"]) == 100


def test_phi_c_ffi_cargo_toml_has_parking_lot_dep():
    """Φ-C-FFI artefact #08 : Cargo.toml must declare parking_lot."""
    cargo = (_FFI / "Cargo.toml").read_text(encoding="utf-8")
    assert "parking_lot" in cargo, (
        "Φ-C-FFI requires `parking_lot` for the LRU cache Mutex"
    )
    assert "efe-telemetry" in cargo, (
        "Φ-C-FFI requires `efe-telemetry` as a non-optional dep "
        "(= production caller for the 8 FFI instruments)"
    )


def test_phi_c_ffi_cargo_toml_has_bench_section():
    """Φ-C-FFI artefact #07 : `[[bench]] name = ffi_overhead` must
    be declared so `cargo bench -p efe-ffi --bench ffi_overhead` works.
    """
    cargo = (_FFI / "Cargo.toml").read_text(encoding="utf-8")
    assert re.search(
        r'\[\[bench\]\]\s*\nname\s*=\s*"ffi_overhead"',
        cargo,
    ), "Φ-C-FFI bench section missing"


# ────────────────────────────────────────────────────────────────────
# Φ-C-FFI Completion audit fix B7 — D-FFI-2 enforcement runtime.
#
# Doctrine D-FFI-2 (architecture § 0.1) : "Tout instrument Prometheus
# déclaré doit avoir un appelant production documenté (grep -r retourne
# ≥ 1 hit hors `#[cfg(test)]`)."
#
# Sans pin test runtime-enforced, un dev pourrait retirer un caller
# production (par exemple en revertant le wiring du cache LRU dans
# classify.rs) sans CI rouge → l'instrument resterait déclaré mais
# son histogramme serait plat → Grafana "no data" silencieux.
# C'est EXACTEMENT le pattern Φ-D.B1/B2 que nous avons éradiqué côté
# CUDA — on l'éradique ici aussi.
# ────────────────────────────────────────────────────────────────────

# Mapping helper → callers production attendus (grep substring, sans
# `#[cfg(test)]`). Chaque entry doit avoir ≥ 1 match cumulé sur tous
# les fichiers listés.
PHI_C_FFI_PROD_CALLERS = {
    "note_classify": ["crates/efe-ffi/src/client.rs", "crates/efe-ffi/src/async_.rs"],
    "note_gil_split": ["crates/efe-ffi/src/client.rs"],
    "note_marshal_in": ["crates/efe-ffi/src/client.rs"],
    "note_marshal_out": ["crates/efe-ffi/src/client.rs"],
    "note_bundle_cache": ["crates/efe-ffi/src/cache.rs"],
    "note_wheel_load": ["crates/efe-ffi/src/lib.rs"],
    "register_all_ffi": ["crates/efe-ffi/src/lib.rs"],
}


def _grep_production_callers(symbol: str, files: list[str]) -> int:
    """Count occurrences of `symbol` in production code (= non-test).

    Strips `#[cfg(test)]` blocks and lines inside `#[cfg(test)] mod tests`
    via heuristic : we ignore every line whose surrounding 20-line window
    contains `#[cfg(test)]` or `mod tests {`.
    """
    total = 0
    for rel in files:
        path = _REPO / rel
        if not path.exists():
            continue
        lines = path.read_text(encoding="utf-8").splitlines()
        # Find ranges marked by #[cfg(test)] or `mod tests {` until the
        # matching closing brace. Simpler heuristic : every line at and
        # after `mod tests {` to end-of-file is test scope.
        test_scope_start = None
        for i, line in enumerate(lines):
            stripped = line.strip()
            if stripped.startswith("mod tests") or stripped == "mod tests {":
                test_scope_start = i
                break
        prod_lines = lines if test_scope_start is None else lines[:test_scope_start]
        for line in prod_lines:
            # Skip comments.
            stripped = line.lstrip()
            if stripped.startswith("//") or stripped.startswith("/*") or stripped.startswith("*"):
                continue
            if symbol in line:
                total += 1
    return total


def test_phi_c_ffi_prod_callers_exist():
    """**D-FFI-2 enforcement** — every FFI instrument helper must have
    ≥ 1 production caller (= grep hit outside `#[cfg(test)]`).

    Failing this test is a hard red signal that the wiring has been
    accidentally removed → instrument déclaré mais jamais observé
    → pattern Φ-D.B1/B2 (silently flat Grafana panel).
    """
    missing = []
    for symbol, files in PHI_C_FFI_PROD_CALLERS.items():
        count = _grep_production_callers(symbol, files)
        if count == 0:
            missing.append(
                f"{symbol} : 0 production callers found in {files} "
                f"(D-FFI-2 violation : instrument has no observe path)"
            )
    assert not missing, (
        "Φ-C-FFI D-FFI-2 violated : instruments without production callers :\n  "
        + "\n  ".join(missing)
    )


def test_phi_c_ffi_async_records_endpoint_async():
    """**Audit fix Φ-C-FFI.B1 enforcement** — `aclassify_zero_copy`
    must invoke `note_classify(..., "async", ...)` for both ok and err
    paths. Sans ce wiring, 0 % des appels async sont labellés correctly
    → quadrant async du dashboard Grafana faussement plat.
    """
    async_src = (_FFI / "src" / "async_.rs").read_text(encoding="utf-8")
    # Find at least one `"async"` literal next to `note_classify`.
    assert "note_classify(" in async_src, (
        "B1 regression : async_.rs no longer calls note_classify"
    )
    assert '"async"' in async_src, (
        "B1 regression : async_.rs no longer passes the `async` endpoint label"
    )
    # Both ok and err paths must record.
    assert async_src.count('"async"') >= 2, (
        "B1 regression : async_.rs must record both ok and err branches "
        "(found < 2 'async' literals)"
    )


def test_phi_c_ffi_freeze_uses_numpy_f32():
    """**Audit fix Φ-C-FFI.B2 enforcement** — `freeze_numerical_equiv.py`
    must use numpy f32 arithmetic (NOT pure-Python f64). Sans cette
    fidelité, l'embedding bytes packed à f32 diverge de Rust →
    SHA-256 totalement différent → 0/100 fixtures match.

    Specifically check :
      * `import numpy as np` is present
      * `np.float32(` is used (= explicit f32 cast)
      * `linear sum` comment / loop is present (= not numpy default
        pairwise sum)
    """
    src = (_REPO / "scripts" / "bench" / "freeze_numerical_equiv.py").read_text(
        encoding="utf-8"
    )
    assert "import numpy as np" in src, (
        "B2 regression : freeze script no longer imports numpy"
    )
    assert "np.float32(" in src, (
        "B2 regression : freeze script no longer uses explicit f32 casts"
    )
    # The linear sum loop should still be present (= the key fix).
    assert "linear sum" in src.lower() or "for v in emb" in src, (
        "B2 regression : freeze script no longer does linear sum "
        "(numpy pairwise sum would diverge from Rust)"
    )


def test_phi_c_ffi_panic_hook_has_eprintln_fallback():
    """**Audit fix Φ-C-FFI.B3 enforcement** — install_panic_hook must
    use BOTH `tracing::error!` AND `eprintln!` to guarantee visibility
    even without a tracing subscriber.
    """
    src = (_FFI / "src" / "lib.rs").read_text(encoding="utf-8")
    # Locate the panic hook function block. The body contains
    # `Box::new(move |info| {...})` with nested braces — `[^}]` is
    # too restrictive. Strategy : take the substring from the fn
    # signature to the end of the file, then assert eprintln! is
    # in that scope AND mentions "efe-ffi rust panic".
    hook_start = src.find("fn install_panic_hook()")
    assert hook_start >= 0, "install_panic_hook function declaration missing"
    hook_scope = src[hook_start:]
    # Take everything until the next top-level `fn` or `#[cfg(test)]`.
    next_fn = re.search(r"\n(?:fn |#\[cfg\(test\)\])", hook_scope[20:])
    if next_fn:
        hook_scope = hook_scope[: 20 + next_fn.start()]
    assert "eprintln!" in hook_scope, (
        "B3 regression : install_panic_hook missing eprintln! fallback "
        "(tracing-only would silently drop panics without subscriber)"
    )
    assert "efe-ffi rust panic" in hook_scope, (
        "B3 regression : install_panic_hook eprintln! does not include "
        "the canonical panic prefix"
    )


def test_phi_c_ffi_numerical_equiv_uses_strict_comparison():
    """**Audit fix Φ-C-FFI.B6 enforcement** — `test_numerical_equiv.py`
    must use `CONFIDENCE_EPSILON = 0.0` (= strict bit-identity) post-B2.
    Pre-fix `1e-6` accepted silent precision drift.
    """
    src = (
        _FFI / "python" / "efe" / "tests" / "test_numerical_equiv.py"
    ).read_text(encoding="utf-8")
    assert "CONFIDENCE_EPSILON = 0.0" in src, (
        "B6 regression : numerical_equiv no longer enforces strict "
        "bit-identity ; tolerance should be 0.0 post-B2"
    )
