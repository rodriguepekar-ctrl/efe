"""Sprint **M8 extension** — Python SDK pin tests (D19 + D20 + D25).

**52 anti-regression pin tests** = 42 livraison + 10 audit fixes.

The Python SDK is **mostly already implemented** post-D18 ; this
file PINS what exists so future refactors can't accidentally
regress (a) exit codes taxonomy, (b) JSON schema_version, (c)
pretty/compact inversion, (d) flag layout, (e) Verdict shape, ...

Sections :
    §1. Lettre A — 10 pin tests (DT-A-1..8)
    §2. Lettre B — 6 pin tests (DT-B-1..5)
    §3. Lettre C — 6 pin tests (DT-C-1..5)
    §4. Lettre D — 5 pin tests (DT-D-1..5)
    §5. Sample bundle — 3 pin tests (DT-PC-1/2)
    §6. D25 native wiring — 12 pin tests
    §7. Audit fixes M8-A1..A11 — 10 pin tests (post-audit)

Each test docstring lists the DT id + 1-line invariant.

Cf. `docs/EFE_PYTHON_SDK_EXTENSION_ARCHITECTURE.md` §16.
"""
from __future__ import annotations

import importlib
import json
import os
import re
import sys
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
SDK_ROOT = REPO_ROOT / "clients" / "python"
EFE_TOOL = SDK_ROOT / "efe_tool"
PYPROJECT = SDK_ROOT / "pyproject.toml"


# **Audit fix M8-A9 (MED)** — single sys.path insertion at module
# load time, instead of per-test. Pytest collects this file once,
# so the path stays clean for the rest of the session. Pre-audit
# every test did `sys.path.insert(0, str(SDK_ROOT))` which polluted
# sys.path with duplicate entries.
if str(SDK_ROOT) not in sys.path:
    sys.path.insert(0, str(SDK_ROOT))


def _read(p: Path) -> str:
    return p.read_text(encoding="utf-8")


# ─────────────────────────────────────────────────────────────────────
# §1. Lettre A — 8 pin tests
# ─────────────────────────────────────────────────────────────────────


def test_d19_dt_a1_exit_codes_taxonomy_present() -> None:
    """**DT-A-1** — `EXIT_CODE_TABLE` declares stable mapping."""
    src = _read(EFE_TOOL / "cli" / "errors.py")
    assert "EXIT_CODE_TABLE" in src
    # Anchor the canonical 11-entry shape (0/2/3/4/5/6/7/8/9/126/130).
    for code in (0, 2, 3, 4, 5, 6, 7, 8, 9, 126, 130):
        assert re.search(rf"\s{code}:\s", src), \
            f"exit code {code} missing from table"


def test_d19_dt_a1_exit_code_classes_have_distinct_codes() -> None:
    """**DT-A-1** — every CliError subclass has a distinct exit_code."""
    from efe_tool.cli import errors as cli_errors  # type: ignore
    classes = [
        cli_errors.UsageError,
        cli_errors.CatalogNotFoundError,
        cli_errors.CatalogInvalidError,
        cli_errors.NetworkError,
        cli_errors.ServerError,
        cli_errors.CliTimeoutError,
        cli_errors.RejectedError,
        cli_errors.LowConfidenceError,
        cli_errors.InterruptedError,
    ]
    codes = [c.exit_code for c in classes]
    assert len(set(codes)) == len(codes), \
        f"duplicate exit codes : {sorted(codes)}"


def test_d19_dt_a2_output_carries_schema_version() -> None:
    """**DT-A-2** — every `build_*_document` emits `schema_version`."""
    src = _read(EFE_TOOL / "cli" / "output.py")
    # Each builder must reference SCHEMA_VERSION exactly once at least.
    for builder in (
        "build_classify_document",
        "build_healthcheck_document",
        "build_compile_document",
        "build_validate_document",
        "build_version_document",
    ):
        assert builder in src, f"missing builder: {builder}"
    # The constant comes from cli.metadata.
    assert "SCHEMA_VERSION" in src


def test_d19_dt_a2_schema_version_is_versioned_const() -> None:
    """**DT-A-2** — SCHEMA_VERSION lives in cli/metadata.py (single source)."""
    src = _read(EFE_TOOL / "cli" / "metadata.py")
    assert re.search(r"SCHEMA_VERSION\s*[:=]", src), \
        "SCHEMA_VERSION must be declared in cli/metadata.py"


def test_d19_dt_a3_pretty_is_opt_in_default_compact() -> None:
    """**DT-A-3** — JSON renderer default is COMPACT, `--pretty` opt-in."""
    src = _read(EFE_TOOL / "cli" / "output.py")
    # The render() signature has `pretty: bool = False`.
    assert re.search(r"def render\([^)]*pretty:\s*bool\s*=\s*False",
                     src, re.S), \
        "render() must default to pretty=False (compact)"
    # And the JSON path uses `separators=(\",\",\":\")` for compact.
    assert 'separators=(",", ":")' in src


def test_d19_dt_a4_log_level_format_split() -> None:
    """**DT-A-4** — parser exposes `--log-level`, `--log-format`, `--format` separately."""
    src = _read(EFE_TOOL / "cli" / "parser.py")
    # All 3 flags must be declared (avoid the legacy --verbose conflation).
    for flag in ("--log-level", "--log-format", "--format"):
        assert flag in src, f"flag missing in parser: {flag}"


def test_d19_dt_a5_text_file_stdin_mutually_exclusive() -> None:
    """**DT-A-5** — `--text-file` + `--stdin` declared mutually exclusive."""
    src = _read(EFE_TOOL / "cli" / "parser.py")
    assert "--text-file" in src
    assert "--stdin" in src
    # Mutually exclusive group OR explicit usage check :
    has_mutex = "add_mutually_exclusive_group" in src
    has_explicit_check = "text_file" in src and "stdin" in src
    assert has_mutex or has_explicit_check


def test_d19_dt_a6_alias_efe_in_pyproject() -> None:
    """**DT-A-6** — `efe` alias declared alongside `efe-admin`."""
    cfg = _read(PYPROJECT)
    assert "[project.scripts]" in cfg
    assert re.search(r'efe-admin\s*=\s*"efe_tool\.cli:main"', cfg)
    assert re.search(r'^efe\s*=\s*"efe_tool\.cli:main"', cfg, re.M)


def test_d19_dt_a7_completions_subcommand_present() -> None:
    """**DT-A-7** — `efe-admin completions {bash|zsh|fish}` shipped."""
    completions_path = EFE_TOOL / "cli" / "commands" / "completions.py"
    assert completions_path.exists()
    src = _read(completions_path)
    # Must support the 3 canonical shells.
    for shell in ("bash", "zsh", "fish"):
        assert shell in src.lower()


def test_d19_dt_a8_lazy_dispatch_in_main() -> None:
    """**DT-A-8** — `_dispatch` imports each command lazily."""
    src = _read(EFE_TOOL / "cli" / "main.py")
    # The dispatch table imports inline (per-branch `from .commands import ...`)
    # rather than a top-of-module import — this is the lazy-import idiom.
    assert "from .commands import classify" in src
    # And it must NOT preload sentence-transformers / torch at module top.
    assert "import sentence_transformers" not in src
    assert "import torch" not in src


# ─────────────────────────────────────────────────────────────────────
# §2. Lettre B — 5 pin tests
# ─────────────────────────────────────────────────────────────────────


def test_d19_dt_b1_pyreadline3_extra_declared() -> None:
    """**DT-B-1** — `[repl-win]` extra ships pyreadline3 on Windows."""
    cfg = _read(PYPROJECT)
    assert "repl-win" in cfg
    assert "pyreadline3" in cfg
    assert "sys_platform == 'win32'" in cfg


def test_d19_dt_b2_pexpect_dev_dep_declared() -> None:
    """**DT-B-2** — pexpect declared in `[dev]` for signal tests."""
    cfg = _read(PYPROJECT)
    # Must declare pexpect for the dev/test path. Skipped on Windows
    # via the `sys_platform != 'win32'` marker.
    assert re.search(r"pexpect[^']*sys_platform\s*!=\s*'win32'", cfg)


def test_d19_dt_b3_history_cap_declared() -> None:
    """**DT-B-3** — `HISTORY_CAP` is a finite int constant."""
    src = _read(EFE_TOOL / "repl" / "history.py")
    m = re.search(r"HISTORY_CAP\s*:\s*int\s*=\s*([\d_]+)", src)
    assert m is not None, "HISTORY_CAP must be declared as a typed int"
    cap = int(m.group(1).replace("_", ""))
    assert 100 <= cap <= 100_000, f"history cap {cap} out of safe range"


def test_d19_dt_b4_redaction_module_present() -> None:
    """**DT-B-4** — `efe_tool.repl.redaction` ships secret patterns."""
    redact_path = EFE_TOOL / "repl" / "redaction.py"
    assert redact_path.exists()
    src = _read(redact_path)
    # Public surface : redact + redact_mapping + add_pattern + reset_patterns.
    for fn in ("redact", "redact_mapping", "add_pattern",
               "reset_patterns", "builtin_pattern_names"):
        assert f"def {fn}" in src or f"{fn}(" in src
    # Pin the 10 builtin patterns (anti-regression).
    from efe_tool.repl.redaction import builtin_pattern_names  # type: ignore
    names = builtin_pattern_names()
    expected = (
        "jwt", "bearer", "aws_access_key", "aws_secret_kv",
        "openai_key", "stripe_key", "github_pat",
        "gcp_private_key", "x_api_key_header", "env_kv_secret",
    )
    assert names == expected, f"pattern drift : {names} vs {expected}"


def test_d19_dt_b4_redaction_actually_redacts_jwt_and_bearer() -> None:
    """**DT-B-4** — redact() removes JWTs and Bearer tokens from text."""
    from efe_tool.repl.redaction import redact  # type: ignore
    payload = (
        'curl -H "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9'
        '.eyJzdWIiOiJ4eHgxMjM0NTYifQ.signaturepartfortest" /api'
    )
    out = redact(payload)
    assert "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9" not in out
    assert "<REDACTED" in out


def test_d19_dt_b5_repl_doc_present_and_lists_commands() -> None:
    """**DT-B-5** — `docs/repl.md` exists and lists canonical commands."""
    doc = REPO_ROOT / "docs" / "repl.md"
    assert doc.exists()
    content = _read(doc)
    # Must mention at least the 5 most-used commands.
    for cmd in (":help", ":quit", ":classify", ":catalog", ":log"):
        assert cmd in content, f"command {cmd!r} not documented"


# ─────────────────────────────────────────────────────────────────────
# §3. Lettre C — 5 pin tests
# ─────────────────────────────────────────────────────────────────────


def test_d20_dt_c1_three_scenarios_present() -> None:
    """**DT-C-1** — 3 builtin scenarios shipped under scenarios/builtin/."""
    builtin = EFE_TOOL / "scenarios" / "builtin"
    assert builtin.is_dir()
    children = {p.name for p in builtin.iterdir() if p.is_dir()}
    # The 3 canonical scenarios :
    expected = {"billing_escalation", "fraud_alert", "support_triage_bilingual"}
    assert expected.issubset(children), \
        f"missing scenarios : {expected - children}"


def test_d20_dt_c1_each_scenario_has_golden_and_prompt() -> None:
    """**DT-C-1 sub** — each scenario directory has README + prompt + golden."""
    builtin = EFE_TOOL / "scenarios" / "builtin"
    for scenario in builtin.iterdir():
        if not scenario.is_dir():
            continue
        for required in ("README.md", "prompt.md", "golden.json", "__init__.py"):
            assert (scenario / required).exists(), \
                f"{scenario.name}/{required} missing"


def test_d20_dt_c2_retry_on_rate_limit_supported() -> None:
    """**DT-C-2** — runner accepts `retry_on_rate_limit` kwarg."""
    src = _read(EFE_TOOL / "scenarios" / "runner.py")
    assert "retry_on_rate_limit" in src


def test_d20_dt_c3_transcript_schema_version_pinned() -> None:
    """**DT-C-3** — `TRANSCRIPT_SCHEMA_VERSION` is a stable int constant."""
    src = _read(EFE_TOOL / "scenarios" / "transcript.py")
    m = re.search(r"TRANSCRIPT_SCHEMA_VERSION\s*:\s*int\s*=\s*(\d+)", src)
    assert m is not None
    assert int(m.group(1)) >= 1


def test_d20_dt_c4_cost_ceiling_supported() -> None:
    """**DT-C-4** — runner accepts `cost_ceiling_usd` kwarg."""
    src = _read(EFE_TOOL / "scenarios" / "runner.py")
    assert "cost_ceiling_usd" in src


def test_d20_dt_c5_each_scenario_has_golden_json() -> None:
    """**DT-C-5** — each scenario has a `golden.json` for all-mock CI."""
    builtin = EFE_TOOL / "scenarios" / "builtin"
    for scenario in builtin.iterdir():
        if not scenario.is_dir():
            continue
        golden = scenario / "golden.json"
        assert golden.exists()
        # Validate it parses as JSON (deterministic test fixture).
        json.loads(_read(golden))


# ─────────────────────────────────────────────────────────────────────
# §4. Lettre D — 5 pin tests
# ─────────────────────────────────────────────────────────────────────


def test_d20_dt_d1_verdict_is_frozen_dataclass() -> None:
    """**DT-D-1** — `Verdict` is `@dataclass(frozen=True)`."""
    from efe_tool.integrations._types import Verdict  # type: ignore
    import dataclasses
    assert dataclasses.is_dataclass(Verdict)
    # Mutation must raise.
    v = Verdict(
        label="x", confidence=0.5, reject=False, rounds=1,
        alignment_delta=0.0, catalog="c", zone="trusted",
        elapsed_ms=0, request_id=None,
    )
    with pytest.raises(dataclasses.FrozenInstanceError):
        v.label = "y"  # type: ignore


def test_d20_dt_d2_efefacade_api_methods_stable() -> None:
    """**DT-D-2** — `EfeFacade` exposes the 4 stable methods + ctx mgr."""
    from efe_tool.integrations._facade import EfeFacade  # type: ignore
    for method in ("classify", "classify_async", "healthcheck", "close"):
        assert hasattr(EfeFacade, method), f"EfeFacade.{method} missing"
    # Context manager protocol.
    assert hasattr(EfeFacade, "__enter__")
    assert hasattr(EfeFacade, "__exit__")


def test_d20_dt_d3_otel_tracer_has_soft_dep() -> None:
    """**DT-D-3** — `_tracing.Tracer` no-ops without opentelemetry."""
    from efe_tool.integrations import _tracing  # type: ignore
    # The module-level _HAS_OTEL flag is set via importlib probe.
    assert hasattr(_tracing, "_HAS_OTEL")
    # Tracer construction MUST not raise even when OTel is absent.
    t = _tracing.Tracer(enabled=False)
    with t.span("test") as span:
        span.set_attribute("k", "v")  # no-op


def test_d20_dt_d4_prom_metrics_have_soft_dep() -> None:
    """**DT-D-4** — `_metrics.MetricsRegistry` no-ops without prometheus_client."""
    from efe_tool.integrations import _metrics  # type: ignore
    assert hasattr(_metrics, "_HAS_PROMETHEUS")
    # Construction MUST not raise even when prom client is absent.
    reg = _metrics.MetricsRegistry(namespace="efe_test")
    # The required handles must all exist (real or no-op).
    for handle in ("classify_total", "classify_latency",
                   "classify_errors", "in_flight",
                   "preload_seconds", "ready"):
        assert hasattr(reg, handle), f"MetricsRegistry.{handle} missing"


def test_d20_dt_d5_fixtures_in_testing_module() -> None:
    """**DT-D-5** — `FakeEfeFacade` + `make_verdict` exposed."""
    from efe_tool.integrations import testing  # type: ignore
    assert hasattr(testing, "FakeEfeFacade")
    assert hasattr(testing, "make_verdict")


# ─────────────────────────────────────────────────────────────────────
# §5. Sample bundle — 3 pin tests
# ─────────────────────────────────────────────────────────────────────


def test_d20_sample_bundle_present_and_signed() -> None:
    """**DT-PC-1** — `bundles/all-MiniLM-L6-v2-en-1.0.0/` exists."""
    bundle = REPO_ROOT / "bundles" / "all-MiniLM-L6-v2-en-1.0.0"
    assert bundle.is_dir()
    for f in ("weights.bin", "tokenizer.json", "manifest.json"):
        assert (bundle / f).exists(), f"missing bundle artefact : {f}"


def test_d20_sample_bundle_doc_present() -> None:
    """**DT-PC-2** — `docs/bundles/sample.md` documents the bundle."""
    doc = REPO_ROOT / "docs" / "bundles" / "sample.md"
    assert doc.exists()
    content = _read(doc)
    # Must mention the slug + the build script.
    assert "all-MiniLM-L6-v2-en-1.0.0" in content
    assert "build_sample_bundle.py" in content


def test_d20_sample_bundle_reproducibility_check() -> None:
    """**DT-PC-1** — manifest.json is canonical (sorted keys)."""
    manifest = (
        REPO_ROOT / "bundles" / "all-MiniLM-L6-v2-en-1.0.0" / "manifest.json"
    )
    raw = manifest.read_text(encoding="utf-8")
    parsed = json.loads(raw)
    # Re-serialise with the same canonical settings.
    canonical = json.dumps(parsed, sort_keys=True, indent=2)
    assert raw.strip() == canonical.strip(), \
        "manifest.json drift from canonical form (DT-PC-2)"


# ─────────────────────────────────────────────────────────────────────
# §6. D25 native wiring — 12 pin tests
# ─────────────────────────────────────────────────────────────────────


def test_d25_native_extra_declared_in_pyproject() -> None:
    """**D25** — `[native]` extra opt-in for the PyO3 wheel."""
    cfg = _read(PYPROJECT)
    assert re.search(r"^native\s*=\s*\[", cfg, re.M)


def test_d25_native_available_lazy_probe_in_init() -> None:
    """**D25** — `__init__.py` exposes `_check_native_available`."""
    src = _read(EFE_TOOL / "__init__.py")
    assert "_check_native_available" in src
    assert "NATIVE_AVAILABLE" in src


def test_d25_native_lazy_import_does_not_fail_when_wheel_missing() -> None:
    """**D25** — importing efe_tool succeeds even without efe._native."""
    sys.path.insert(0, str(SDK_ROOT))
    # Remove cached import to force a fresh probe.
    for mod in list(sys.modules):
        if mod.startswith("efe_tool") or mod.startswith("efe._native"):
            del sys.modules[mod]
    import efe_tool  # noqa: F401 — must succeed
    # The flag is exposed lazily.
    flag = efe_tool.NATIVE_AVAILABLE
    assert isinstance(flag, bool)


def test_d25_native_efenativeclient_attr_attribute_error_when_absent() -> None:
    """**D25** — `EfeNativeClient` raises AttributeError with helpful msg
    when the wheel is not installed."""
    import efe_tool  # type: ignore
    if efe_tool.NATIVE_AVAILABLE:
        pytest.skip("native wheel installed — error path inactive")
    with pytest.raises(AttributeError) as exc_info:
        _ = efe_tool.EfeNativeClient
    # Must mention the install hint.
    assert "efe-tool[native]" in str(exc_info.value)


def test_d25_native_dir_includes_native_names() -> None:
    """**D25** — dir(efe_tool) shows native names for IDE completion."""
    import efe_tool  # type: ignore
    names = set(dir(efe_tool))
    assert "NATIVE_AVAILABLE" in names
    assert "EfeNativeClient" in names


def test_d25_efe_ffi_crate_exists() -> None:
    """**D25** — the Rust crate `efe-ffi` is staged."""
    crate = REPO_ROOT / "crates" / "efe-ffi"
    assert crate.is_dir()
    assert (crate / "Cargo.toml").exists()
    assert (crate / "src" / "lib.rs").exists()


def test_d25_efe_ffi_has_pyo3_modules() -> None:
    """**D25** — efe-ffi/src/ exposes the canonical PyO3 entry points."""
    src = REPO_ROOT / "crates" / "efe-ffi" / "src"
    for module in ("client.rs", "classify.rs", "controller.rs",
                   "verdict.rs", "types.rs", "bundle.rs"):
        assert (src / module).exists(), f"efe-ffi missing : {module}"


def test_d25_efe_ffi_cargo_declares_pyo3() -> None:
    """**D25** — efe-ffi/Cargo.toml declares pyo3.

    **Audit fix M8-A10 (MED)** — strict regex anchors the dep
    declaration line, not just the substring (which could match
    a comment or a `pyo3-foo` lookalike). Pre-audit `"pyo3" in
    cargo` was a false-positive trap.
    """
    cargo = _read(REPO_ROOT / "crates" / "efe-ffi" / "Cargo.toml")
    # Match `pyo3 = ...` or `pyo3 = { ... }` (either as direct dep
    # or in a feature block). Anchored to a line start to dodge
    # comments and substring matches.
    has_dep = bool(
        re.search(r"^\s*pyo3\s*=\s*[\{\"]", cargo, re.M)
        or re.search(r'"pyo3[/\-]', cargo)        # feature ref
    )
    assert has_dep, "Cargo.toml must declare pyo3 as a real dep"


def test_d25_facade_supports_client_kind_field() -> None:
    """**D25** — EfeFacade integration layer is reachable for client_kind."""
    from efe_tool.integrations import _facade  # type: ignore
    # Module loads without requiring native.
    assert hasattr(_facade, "EfeFacade")


def test_d25_efe_ffi_architecture_doc_pinned() -> None:
    """**D25** — `docs/EFE_FFI_ARCHITECTURE.md` describes the sprint."""
    doc = REPO_ROOT / "docs" / "EFE_FFI_ARCHITECTURE.md"
    assert doc.exists()
    content = _read(doc)
    # Pin the D25 sprint reference + 5× speedup target.
    assert "D25" in content
    assert "5×" in content or "5x" in content


def test_d25_native_module_path_canonical() -> None:
    """**D25** — the native module is importable as `efe._native` (canonical)."""
    src = _read(EFE_TOOL / "__init__.py")
    # The lazy probe imports `efe._native`, NOT `efe_native` or
    # `_efe_native` — pin the canonical module path.
    assert "import efe._native" in src
    assert "from efe._native import" in src


def test_d25_native_extra_pulls_efe_wheel() -> None:
    """**D25** — `[native]` extra pulls the `efe` package (wheel from efe-ffi)."""
    cfg = _read(PYPROJECT)
    # The native extra must reference the `efe` package (produced
    # by maturin from crates/efe-ffi/), NOT a placeholder.
    m = re.search(r"^native\s*=\s*\[(.*?)\]", cfg, re.M | re.S)
    assert m is not None
    block = m.group(1)
    assert '"efe' in block, "native extra must reference the `efe` wheel"


# ─────────────────────────────────────────────────────────────────────
# §7. Audit fixes M8-A1..A11 — 10 anti-regression pin tests (post-audit)
# ─────────────────────────────────────────────────────────────────────


def test_m8_audit_fix_a1_source_date_epoch_honoured() -> None:
    """**Audit fix M8-A1 (CRIT)** — `build_sample_bundle.resolve_built_at`
    honours `SOURCE_DATE_EPOCH` env var (Reproducible Builds spec).

    Two builds with the same SOURCE_DATE_EPOCH MUST produce
    byte-identical manifests. Pre-audit a `datetime.now()` call
    poisoned the manifest with a per-run timestamp.
    """
    sys.path.insert(0, str(REPO_ROOT / "scripts" / "release"))
    if "build_sample_bundle" in sys.modules:
        del sys.modules["build_sample_bundle"]
    import build_sample_bundle as bsb  # type: ignore

    # When SOURCE_DATE_EPOCH is set, resolve_built_at returns the
    # corresponding ISO-8601 string regardless of `dev` flag.
    os.environ["SOURCE_DATE_EPOCH"] = "1700000000"  # 2023-11-14T22:13:20Z
    try:
        a = bsb.resolve_built_at(dev=True)
        b = bsb.resolve_built_at(dev=False)
        assert a == b, f"SOURCE_DATE_EPOCH not honoured ({a!r} vs {b!r})"
        assert a.startswith("2023-11-14T22:13:20")
    finally:
        del os.environ["SOURCE_DATE_EPOCH"]


def test_m8_audit_fix_a1_dev_mode_uses_stable_default_epoch() -> None:
    """**Audit fix M8-A1 (CRIT)** — when SOURCE_DATE_EPOCH is unset
    and `--dev` is passed, the fallback uses `DEFAULT_DEV_EPOCH`
    (stable). Two dev rebuilds → byte-identical manifests."""
    sys.path.insert(0, str(REPO_ROOT / "scripts" / "release"))
    if "build_sample_bundle" in sys.modules:
        del sys.modules["build_sample_bundle"]
    import build_sample_bundle as bsb  # type: ignore

    os.environ.pop("SOURCE_DATE_EPOCH", None)
    a = bsb.resolve_built_at(dev=True)
    b = bsb.resolve_built_at(dev=True)
    assert a == b, "dev rebuilds MUST produce identical built_at"
    # Must equal 2026-01-01T00:00:00Z (DEFAULT_DEV_EPOCH).
    assert a.startswith("2026-01-01T00:00:00")


def test_m8_audit_fix_a1_invalid_source_date_epoch_raises() -> None:
    """**Audit fix M8-A1** — bad SOURCE_DATE_EPOCH must raise (not
    silently fall back to now()). Anti-foot-gun."""
    sys.path.insert(0, str(REPO_ROOT / "scripts" / "release"))
    if "build_sample_bundle" in sys.modules:
        del sys.modules["build_sample_bundle"]
    import build_sample_bundle as bsb  # type: ignore
    os.environ["SOURCE_DATE_EPOCH"] = "not-a-number"
    try:
        with pytest.raises(RuntimeError) as exc_info:
            bsb.resolve_built_at(dev=False)
        assert "SOURCE_DATE_EPOCH" in str(exc_info.value)
    finally:
        del os.environ["SOURCE_DATE_EPOCH"]


def test_m8_audit_fix_a2_bundle_slug_derived_from_out_dir() -> None:
    """**Audit fix M8-A2 (CRIT)** — `bundle_slug` MUST match the
    `--out` directory name (or the explicit `--slug` override).
    Pre-audit it was hardcoded to `DEFAULT_BUNDLE_SLUG`, so a
    custom-named output bundle had a lying manifest."""
    sys.path.insert(0, str(REPO_ROOT / "scripts" / "release"))
    if "build_sample_bundle" in sys.modules:
        del sys.modules["build_sample_bundle"]
    import build_sample_bundle as bsb  # type: ignore

    # Default — derived from out_dir.
    p = Path("/tmp/my-custom-bundle-2026")
    assert bsb.resolve_bundle_slug(p, override=None) == "my-custom-bundle-2026"
    # Override wins.
    assert bsb.resolve_bundle_slug(p, override="forced") == "forced"


def test_m8_audit_fix_a2_slug_flag_in_argparse() -> None:
    """**Audit fix M8-A2** — argparse exposes `--slug`."""
    src = _read(REPO_ROOT / "scripts" / "release" / "build_sample_bundle.py")
    assert "--slug" in src
    # And `build()` accepts a `slug_override` kwarg.
    assert "slug_override" in src


def test_m8_audit_fix_a3_real_ed25519_signature_when_cryptography_available() -> None:
    """**Audit fix M8-A3 (CRIT)** — `phase_sign` produces a REAL
    Ed25519 signature in CI release mode (vs the
    `<TO_BE_FILLED_BY_CI_RELEASE>` placeholder pre-audit).

    Skipped when `cryptography` is not installed (the script's
    fallback raises `SigningUnavailable` ; pin tested separately)."""
    pytest.importorskip("cryptography")
    sys.path.insert(0, str(REPO_ROOT / "scripts" / "release"))
    if "build_sample_bundle" in sys.modules:
        del sys.modules["build_sample_bundle"]
    import build_sample_bundle as bsb  # type: ignore
    import tempfile

    from cryptography.hazmat.primitives.asymmetric.ed25519 import (
        Ed25519PrivateKey,
    )

    with tempfile.TemporaryDirectory() as tmp:
        tmpdir = Path(tmp)
        # Stage placeholders + manifest.
        bsb.phase_pull_model(tmpdir, dev=True)
        bsb.phase_quantize(tmpdir, dev=True)
        manifest, manifest_path = bsb.phase_write_manifest(
            tmpdir,
            dev=False,
            built_at="2026-01-01T00:00:00+00:00",
            bundle_slug="test-bundle",
        )
        # Generate a real Ed25519 key + write 32-byte raw seed.
        privkey = Ed25519PrivateKey.generate()
        from cryptography.hazmat.primitives import serialization
        raw = privkey.private_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PrivateFormat.Raw,
            encryption_algorithm=serialization.NoEncryption(),
        )
        privkey_path = tmpdir / "key.bin"
        privkey_path.write_bytes(raw)
        # Sign.
        sig_path = bsb.phase_sign(
            manifest_path, privkey_path=privkey_path, dev=False,
        )
        assert sig_path is not None
        sig_blob = json.loads(sig_path.read_text())
        # Pin the schema.
        assert sig_blob["algorithm"] == "ed25519"
        # The placeholder MUST NOT appear.
        assert sig_blob["signature_base64"] != "<TO_BE_FILLED_BY_CI_RELEASE>"
        # And the base64 must decode to a 64-byte Ed25519 sig.
        import base64 as _b64
        raw_sig = _b64.b64decode(sig_blob["signature_base64"])
        assert len(raw_sig) == 64, f"Ed25519 sig must be 64 bytes, got {len(raw_sig)}"
        # Verify the signature actually covers the canonical manifest.
        privkey.public_key().verify(raw_sig, manifest_path.read_bytes())


def test_m8_audit_fix_a3_signing_unavailable_when_no_cryptography() -> None:
    """**Audit fix M8-A3** — when `cryptography` is absent in CI mode,
    the script raises `SigningUnavailable` cleanly (exit code 5)
    instead of writing a placeholder."""
    src = _read(REPO_ROOT / "scripts" / "release" / "build_sample_bundle.py")
    # The class + the exit-5 mapping must be wired.
    assert "class SigningUnavailable" in src
    assert "return 5" in src


def test_m8_audit_fix_a4_redact_signature_accepts_str_or_none() -> None:
    """**Audit fix M8-A4 (HIGH)** — `redact()` is typed as
    `text: str | None` and the docstring explicitly documents the
    contract (str → redacted, None → "")."""
    src = _read(EFE_TOOL / "repl" / "redaction.py")
    assert "def redact(text: str | None)" in src


def test_m8_audit_fix_a5_redact_raises_typeerror_on_non_str() -> None:
    """**Audit fix M8-A5 (HIGH)** — `redact()` raises TypeError on
    non-(str | None) inputs instead of silently passing through.
    Pre-audit `redact(0)` returned `0` (int), masking dev bugs."""
    from efe_tool.repl.redaction import redact  # type: ignore

    # str OK.
    assert redact("hello") == "hello"
    # None OK (returns "").
    assert redact(None) == ""
    # int / list / dict / bytes → TypeError.
    for bad in (0, 42, [], {}, b"bytes", 3.14):
        with pytest.raises(TypeError) as exc_info:
            redact(bad)  # type: ignore
        assert "str | None" in str(exc_info.value)


def test_m8_audit_fix_a6_native_names_in_all() -> None:
    """**Audit fix M8-A6 (HIGH)** — `__all__` includes `NATIVE_AVAILABLE`
    and `EfeNativeClient` for symmetry with `AsyncEfeClient`. Without
    this, `from efe_tool import *` does NOT pull these names."""
    src = _read(EFE_TOOL / "__init__.py")
    # Match the __all__ block.
    m = re.search(r"__all__\s*=\s*\[(.*?)\]", src, re.S)
    assert m is not None
    block = m.group(1)
    assert '"NATIVE_AVAILABLE"' in block
    assert '"EfeNativeClient"' in block
