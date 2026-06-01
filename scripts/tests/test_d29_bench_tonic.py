"""Sprint **D29 (Φ-D)** — pin tests for the `TonicBenchClient` extension.

Anti-regression : guards the structural invariants of the M6
extension §9 :

  * `client/` sub-module exists with `mod.rs`, `mock.rs`, `tonic_client.rs`.
  * `BenchClient` trait gained the `kind()` method.
  * `ClientKind` enum exists with `Mock` + `Tonic` variants.
  * `TonicBenchClient` + `TonicConfig` declared (gated on
    `feature = "tonic"`).
  * `TonicError` typed taxonomy (Transport / Tls / Status / …).
  * `BenchReport` schema v2 carries `client_kind`.
  * CLI `--jwt-token`, `--tls-ca-file`, `--endpoint` flags present.
  * `Cargo.toml` declares the `tonic` optional dep + feature flag.
  * `build.rs` exists for proto compilation.
"""
from __future__ import annotations

from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

BENCH_CRATE = REPO_ROOT / "crates" / "efe-bench"
CLIENT_DIR = BENCH_CRATE / "src" / "client"
CLIENT_MOD = CLIENT_DIR / "mod.rs"
CLIENT_MOCK = CLIENT_DIR / "mock.rs"
CLIENT_TONIC = CLIENT_DIR / "tonic_client.rs"
ERROR_RS = BENCH_CRATE / "src" / "error.rs"
LIB_RS = BENCH_CRATE / "src" / "lib.rs"
REPORT_RS = BENCH_CRATE / "src" / "report.rs"
CLI_BIN = BENCH_CRATE / "src" / "bin" / "efe_bench.rs"
CARGO_TOML = BENCH_CRATE / "Cargo.toml"
BUILD_RS = BENCH_CRATE / "build.rs"


def _read(p: Path) -> str:
    return p.read_text(encoding="utf-8")


# ---------------------------------------------------------------------
# §9.2 — module structure
# ---------------------------------------------------------------------


def test_client_subdir_exists_with_three_files() -> None:
    """The legacy `client.rs` is split into a `client/` sub-module
    with mod / mock / tonic_client siblings."""
    assert CLIENT_DIR.is_dir(), "client/ subdir missing"
    assert CLIENT_MOD.exists(), "client/mod.rs missing"
    assert CLIENT_MOCK.exists(), "client/mock.rs missing"
    assert CLIENT_TONIC.exists(), "client/tonic_client.rs missing"
    # Ensure the legacy single-file client.rs is gone.
    assert not (BENCH_CRATE / "src" / "client.rs").exists(), (
        "legacy crates/efe-bench/src/client.rs must be removed in D29"
    )


def test_client_mod_declares_submodules() -> None:
    rs = _read(CLIENT_MOD)
    assert "pub mod mock;" in rs
    assert 'pub mod tonic_client;' in rs


def test_client_mod_re_exports_mock_and_tonic_when_feature_on() -> None:
    rs = _read(CLIENT_MOD)
    assert "pub use mock::MockBenchClient;" in rs
    assert 'pub use tonic_client::{TonicBenchClient, TonicConfig};' in rs


# ---------------------------------------------------------------------
# §9.3 — BenchClient trait + ClientKind
# ---------------------------------------------------------------------


def test_bench_client_trait_has_kind_method() -> None:
    rs = _read(CLIENT_MOD)
    assert "fn kind(&self) -> ClientKind" in rs


def test_bench_client_trait_call_accepts_worker_id() -> None:
    """**Audit fix M6-A1 (Φ-F)** — the `call` method gained a
    third parameter `worker_id: u64` so the Tonic client can
    derive the `x-efe-gpu-shard-id` metadata from the same value
    the runner uses to bucket per-shard observations. Pre-audit
    the Tonic client used a private atomic counter that rotated
    across workers, decoupling metrics from routing."""
    rs = _read(CLIENT_MOD)
    assert "worker_id: u64" in rs
    # The trait method signature must surface the parameter.
    assert "async fn call(" in rs


def test_mock_client_call_accepts_worker_id() -> None:
    """Mock impl must accept `worker_id` (ignored at runtime) so
    the trait signature stays consistent across both backends."""
    rs = _read(CLIENT_MOCK)
    assert "worker_id: u64" in rs


def test_tonic_client_call_accepts_worker_id() -> None:
    rs = _read(CLIENT_TONIC)
    assert "worker_id: u64" in rs


def test_tonic_client_separates_call_seed_from_worker_id() -> None:
    """**Audit fix M6-A1 (Φ-F)** — the Mode-3 dispatch must use a
    per-call `call_seed` (atomic counter) for the proto `seed`
    field while reserving `worker_id` for routing. Without the
    separation, every worker would request the same Mode-3 seed
    forever, defeating the parallel sub-swarm diversity."""
    rs = _read(CLIENT_TONIC)
    assert "call_seed: u64" in rs
    assert "seed: call_seed" in rs


def test_bench_client_trait_has_gpu_shards_method() -> None:
    """**D31** — gpu_shards() default returns None on the trait."""
    rs = _read(CLIENT_MOD)
    assert "fn gpu_shards(&self) -> Option<u32>" in rs


def test_client_kind_enum_declares_mock_and_tonic() -> None:
    rs = _read(CLIENT_MOD)
    assert "pub enum ClientKind" in rs
    assert "Mock," in rs
    assert "Tonic," in rs


def test_client_kind_serializes_as_snake_case() -> None:
    rs = _read(CLIENT_MOD)
    assert 'rename_all = "snake_case"' in rs


# ---------------------------------------------------------------------
# §9.4 — TonicBenchClient struct + impl
# ---------------------------------------------------------------------


def test_tonic_client_struct_declared() -> None:
    rs = _read(CLIENT_TONIC)
    assert "pub struct TonicBenchClient" in rs


def test_tonic_client_impl_bench_client() -> None:
    rs = _read(CLIENT_TONIC)
    assert "impl BenchClient for TonicBenchClient" in rs


def test_tonic_client_kind_returns_tonic() -> None:
    rs = _read(CLIENT_TONIC)
    assert "fn kind(&self) -> ClientKind" in rs
    # The implementation must return `ClientKind::Tonic`.
    assert "ClientKind::Tonic" in rs


def test_tonic_client_owns_three_stubs() -> None:
    """Swarm + Catalog + Mode3 stubs (D38 prep)."""
    rs = _read(CLIENT_TONIC)
    assert "swarm_stub" in rs
    assert "catalog_stub" in rs
    assert "mode3_stub" in rs


def test_tonic_client_propagates_authorization_header() -> None:
    rs = _read(CLIENT_TONIC)
    assert "authorization" in rs
    assert 'Bearer' in rs


def test_tonic_client_propagates_x_efe_gpu_shard_id() -> None:
    """**D31** prep — header propagation already wired in D29."""
    rs = _read(CLIENT_TONIC)
    assert "x-efe-gpu-shard-id" in rs


def test_tonic_client_supports_mtls_via_client_tls_config() -> None:
    rs = _read(CLIENT_TONIC)
    assert "ClientTlsConfig" in rs
    assert "Certificate::from_pem" in rs


# ---------------------------------------------------------------------
# §9.5 — TonicConfig builder
# ---------------------------------------------------------------------


def test_tonic_config_struct_declared() -> None:
    rs = _read(CLIENT_TONIC)
    assert "pub struct TonicConfig" in rs


def test_tonic_config_builder_methods_present() -> None:
    rs = _read(CLIENT_TONIC)
    assert "pub fn with_jwt(" in rs
    assert "pub fn with_tls_ca_file(" in rs
    assert "pub fn with_gpu_shards(" in rs
    assert "pub fn with_tls_domain_name(" in rs


def test_tonic_config_endpoint_is_string_field() -> None:
    rs = _read(CLIENT_TONIC)
    assert "pub endpoint: String" in rs


# ---------------------------------------------------------------------
# §9.6 — error.rs typed TonicError
# ---------------------------------------------------------------------


def test_tonic_error_enum_declared() -> None:
    rs = _read(ERROR_RS)
    assert "pub enum TonicError" in rs


def test_tonic_error_has_taxonomy_variants() -> None:
    rs = _read(ERROR_RS)
    for v in [
        "InvalidEndpoint",
        "Tls",
        "Transport",
        "InvalidJwtToken",
        "Status",
        "Mode3StubMissing",
    ]:
        assert v in rs, f"TonicError::{v} missing"


def test_tonic_error_status_carries_code_and_message() -> None:
    rs = _read(ERROR_RS)
    assert "code: u32" in rs
    assert "message: String" in rs


def test_bench_error_has_tonic_variant() -> None:
    rs = _read(ERROR_RS)
    assert "Tonic(#[from] TonicError)" in rs


# ---------------------------------------------------------------------
# §6.3 — schema v2 fields
# ---------------------------------------------------------------------


def test_report_version_bumped_to_v2() -> None:
    rs = _read(LIB_RS)
    assert 'pub const REPORT_VERSION: &str = "2.0.0";' in rs


def test_bench_report_includes_client_kind_field() -> None:
    rs = _read(REPORT_RS)
    assert "pub client_kind: ClientKind" in rs


def test_bench_report_client_kind_has_default_for_v1_compat() -> None:
    rs = _read(REPORT_RS)
    # `#[serde(default = "default_client_kind")]` makes v1 readers
    # land on `ClientKind::Mock` when the field is absent.
    assert 'default = "default_client_kind"' in rs


def test_generate_report_with_client_function_exists() -> None:
    rs = _read(REPORT_RS)
    assert "pub fn generate_report_with_client(" in rs


def test_lib_re_exports_op_stats_and_canonical_slos() -> None:
    """**Audit fix M6-A2 (Φ-F)** — `OpStats`, `canonical_slos` and
    `SloTarget` must be re-exported from the crate root so
    external consumers parsing a `RunnerOutcome` or introspecting
    the SLO targets programmatically don't have to use the long
    `efe_bench::runner::OpStats` / `efe_bench::report::SloTarget`
    paths."""
    rs = _read(LIB_RS)
    assert "OpStats" in rs
    assert "canonical_slos" in rs
    assert "SloTarget" in rs


# ---------------------------------------------------------------------
# §9.6 — CLI flags
# ---------------------------------------------------------------------


def test_cli_has_endpoint_flag() -> None:
    rs = _read(CLI_BIN)
    assert "endpoint: String" in rs
    assert "EFE_BENCH_ENDPOINT" in rs


def test_cli_has_jwt_token_flag() -> None:
    rs = _read(CLI_BIN)
    assert "jwt_token: Option<String>" in rs
    assert "EFE_BENCH_JWT_TOKEN" in rs


def test_cli_has_tls_ca_file_flag() -> None:
    rs = _read(CLI_BIN)
    assert "tls_ca_file: Option<PathBuf>" in rs


def test_cli_has_tls_domain_name_flag() -> None:
    rs = _read(CLI_BIN)
    assert "tls_domain_name: Option<String>" in rs


def test_cli_default_picks_tonic_unless_allow_mock() -> None:
    """The CLI must select Tonic by default and Mock only via
    `--allow-mock` (post-D29 contract)."""
    rs = _read(CLI_BIN)
    # The dispatch logic uses `if args.allow_mock { Mock } else { Tonic }`
    assert "if args.allow_mock" in rs
    # Build the real client when not allow_mock.
    assert "build_real_client" in rs


def test_cli_returns_exit_code_3_on_tonic_error() -> None:
    """ExitCode 3 is reserved for Tonic-side failures (transport,
    TLS, JWT) so CI gates can distinguish them from SLO failures."""
    rs = _read(CLI_BIN)
    assert "BenchError::Tonic(_) => ExitCode::from(3)" in rs


# ---------------------------------------------------------------------
# Cargo.toml + build.rs
# ---------------------------------------------------------------------


def test_cargo_toml_has_tonic_feature_flag() -> None:
    cargo = _read(CARGO_TOML)
    assert "[features]" in cargo
    assert 'tonic = [' in cargo
    assert '"dep:tonic"' in cargo
    assert '"dep:prost"' in cargo


def test_cargo_toml_has_optional_tonic_dep() -> None:
    """Tonic is declared as `optional = true` so the feature flag
    gates pulling it in. The version pin can be either workspace
    or local — D29 picks local because the bench needs to enable
    `tls` + `tls-roots` features which aren't on the workspace
    default. Both forms are accepted to keep the pin test robust
    across future workspace consolidation refactors."""
    cargo = _read(CARGO_TOML)
    # Either `workspace = true, optional = true` or
    # `version = "0.12", optional = true` is acceptable.
    assert "tonic = {" in cargo
    assert "optional = true" in cargo
    # Prost dep follows the workspace shape (no extra features needed).
    assert "prost = { workspace = true, optional = true }" in cargo


def test_cargo_toml_tonic_dep_enables_tls_feature() -> None:
    """**D29** — mTLS support requires the `tls` feature on tonic.
    Without it `Endpoint::tls_config()` doesn't compile."""
    cargo = _read(CARGO_TOML)
    assert '"tls"' in cargo


def test_cargo_toml_has_tonic_build_dep() -> None:
    cargo = _read(CARGO_TOML)
    assert "[build-dependencies]" in cargo
    assert "tonic-build" in cargo
    assert "protoc-bin-vendored" in cargo


def test_build_rs_exists_and_is_feature_gated() -> None:
    assert BUILD_RS.exists(), "crates/efe-bench/build.rs missing"
    rs = _read(BUILD_RS)
    assert '#[cfg(feature = "tonic")]' in rs
    assert '#[cfg(not(feature = "tonic"))]' in rs
    assert "tonic_build::configure()" in rs


def test_build_rs_compiles_required_protos() -> None:
    """The bench harness needs at least types.proto + efe.proto +
    catalog.proto + mode3.proto (D38)."""
    rs = _read(BUILD_RS)
    for p in ["types.proto", "efe.proto", "catalog.proto", "mode3.proto"]:
        assert p in rs, f"build.rs misses {p}"
