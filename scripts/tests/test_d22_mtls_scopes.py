"""Sprint **D22** (M4 § 10) — pin tests for the 3 broad JWT scopes
(`data` / `admin` / `audit`) and the typed `MtlsConfig`.

Audit fix S-A2 — guards the recurring "wiring forgotten" pattern :
every admin / mode-1 / mode-2 / mode-3 / catalog handler MUST call
`ScopeGuard::require` (or the `broad_scope_check` helper that wraps it)
exactly once.
"""
from __future__ import annotations

from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
SCOPE_GUARD_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "auth" / "scope_guard.rs"
MTLS_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "auth" / "mtls.rs"
AUTH_MOD_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "auth" / "mod.rs"
SWARM_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "services" / "swarm.rs"
ADMIN_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "services" / "admin.rs"
MODE2_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "services" / "mode2" / "mod.rs"
MODE3_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "services" / "mode3" / "mod.rs"


def _read(p: Path) -> str:
    return p.read_text(encoding="utf-8")


def test_three_broad_scopes_declared() -> None:
    rs = _read(SCOPE_GUARD_RS)
    for s in ("DATA", "ADMIN", "AUDIT"):
        assert (
            f'pub const {s}:' in rs
        ), f"BroadScope::{s} missing"


def test_scope_guard_provides_require_helper() -> None:
    rs = _read(SCOPE_GUARD_RS)
    assert "pub fn require(claims: &Claims, scope: &str)" in rs
    assert "pub fn require_any(claims: &Claims, scopes: &[&str])" in rs


def test_scope_guard_returns_permission_denied() -> None:
    rs = _read(SCOPE_GUARD_RS)
    assert "Status::permission_denied" in rs


def test_scope_guard_re_exported_from_auth_mod() -> None:
    rs = _read(AUTH_MOD_RS)
    assert "pub use scope_guard::{BroadScope, ScopeGuard};" in rs


def test_mtls_config_validate_returns_typed_error() -> None:
    rs = _read(MTLS_RS)
    assert "MissingClientCaButRequired" in rs
    assert "MissingServerIdentity" in rs
    assert "fn validate(&self)" in rs


def test_mtls_build_server_tls_config_is_feature_gated() -> None:
    rs = _read(MTLS_RS)
    # The actual ServerTlsConfig builder must sit behind `feature = "tls"`
    # so the default build doesn't pull rustls deps.
    assert "#[cfg(feature = \"tls\")]" in rs


def test_mtls_re_exported_from_auth_mod() -> None:
    rs = _read(AUTH_MOD_RS)
    assert "pub use mtls::{MtlsConfig, MtlsError};" in rs


def test_swarm_handlers_call_scope_guard_for_data() -> None:
    """Mode-1 (SwarmService) data-plane handlers must enforce
    BroadScope::DATA in addition to the granular swarm:execute scope."""
    rs = _read(SWARM_RS)
    n_data_checks = rs.count("BroadScope::DATA")
    # 2 handlers (Execute + ExecuteStreaming) must each contain a
    # broad-scope check. DescribeSpec is a metadata RPC, not a data
    # RPC ; D22 leaves it on the granular scope only.
    assert n_data_checks >= 2, (
        f"expected ≥2 BroadScope::DATA checks in swarm.rs, found {n_data_checks}"
    )


def test_admin_handlers_call_scope_guard_for_admin_or_audit() -> None:
    """Every admin handler must check broad scope ADMIN (or AUDIT for
    ListAuditEvents). The pin test counts call sites as a coarse proxy
    — a regression that drops one would surface here."""
    rs = _read(ADMIN_RS)
    # 8 D21 RPCs ; each calls broad_scope_check(...) exactly once.
    n_admin = rs.count("BroadScope::ADMIN")
    n_audit = rs.count("BroadScope::AUDIT")
    assert n_admin >= 7, f"expected ≥7 ADMIN broad checks, found {n_admin}"
    assert n_audit >= 1, f"expected ≥1 AUDIT broad check, found {n_audit}"


def test_mode2_handlers_call_scope_guard_for_data() -> None:
    rs = _read(MODE2_RS)
    n = rs.count("BroadScope::DATA")
    assert n >= 3, f"expected ≥3 BroadScope::DATA checks in mode2, found {n}"


def test_mode3_handlers_call_scope_guard_for_data() -> None:
    rs = _read(MODE3_RS)
    n = rs.count("BroadScope::DATA")
    assert n >= 2, f"expected ≥2 BroadScope::DATA checks in mode3, found {n}"


def test_efe_runtime_mtls_required_env_var_recognised() -> None:
    rs = _read(MTLS_RS)
    assert 'EFE_RUNTIME_MTLS_REQUIRED' in rs


def test_three_pem_paths_env_vars_recognised() -> None:
    rs = _read(MTLS_RS)
    for var in (
        "EFE_TLS_SERVER_CERT_PATH",
        "EFE_TLS_SERVER_KEY_PATH",
        "EFE_TLS_CLIENT_CA_PATH",
    ):
        assert var in rs, f"env var {var} not recognised in MtlsConfig::from_env"


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-v"]))
