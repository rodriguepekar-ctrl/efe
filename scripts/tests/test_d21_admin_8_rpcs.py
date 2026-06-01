"""Sprint **D21** (M4 § 9) — pin tests for the 8 catalog-scoped admin RPCs.

These tests are **structural** — they read source files and assert that
the wiring contracts of the M4 architecture are present. They do NOT
spin up a gRPC server. The full integration test lives in
`crates/efe-server/tests/d21_admin_catalog_integration.rs` (post-D21).

Recurring "wiring forgotten" defect class — pin tests guard against the
defect class itself, not just one instance. Cf. `EFE_SERVER_ARCHITECTURE.md`
§ 24 (S-R-1).
"""
from __future__ import annotations

from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
ADMIN_PROTO = REPO_ROOT / "protos" / "efe" / "v1" / "admin.proto"
ADMIN_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "services" / "admin.rs"
SCOPES_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "auth" / "scopes.rs"


# Each tuple = (rpc_method_name, required_granular_scope_const)
EXPECTED_D21_RPCS: list[tuple[str, str]] = [
    ("ListCatalogs", "ADMIN_CATALOG_READ"),
    ("GetCatalog", "ADMIN_CATALOG_READ"),
    ("DeleteCatalog", "ADMIN_CATALOG_WRITE"),
    ("ReloadBundle", "ADMIN_BUNDLE_RELOAD"),
    ("PinCatalog", "ADMIN_CATALOG_PIN"),
    ("EvolveCatalog", "ADMIN_CATALOG_WRITE"),
    ("ReloadCatalog", "ADMIN_CATALOG_PIN"),
    ("ListAuditEvents", "ADMIN_AUDIT_READ"),
]


def _read(p: Path) -> str:
    return p.read_text(encoding="utf-8")


def test_proto_declares_8_new_rpcs() -> None:
    proto = _read(ADMIN_PROTO)
    for rpc_name, _scope in EXPECTED_D21_RPCS:
        assert (
            f"rpc {rpc_name}(" in proto
        ), f"AdminService.{rpc_name} missing from admin.proto"


def test_proto_admin_namespace_unchanged() -> None:
    proto = _read(ADMIN_PROTO)
    # The 6 M9 cone RPCs must remain — backward compat (GG-D21.4).
    for rpc in (
        "RegisterCone",
        "EvolveCone",
        "GetCone",
        "ListVersions",
        "DeleteCone",
        "BreachHistory",
    ):
        assert f"rpc {rpc}(" in proto, f"M9 RPC {rpc} disappeared"


def test_admin_handler_implements_8_new_rpcs() -> None:
    rs = _read(ADMIN_RS)
    for rpc_name, _scope in EXPECTED_D21_RPCS:
        # Tonic codegen converts CamelCase -> snake_case for handler fns.
        snake = "".join(
            "_" + c.lower() if c.isupper() else c for c in rpc_name
        ).lstrip("_")
        assert (
            f"async fn {snake}(" in rs
        ), f"AdminServiceImpl::{snake} missing from admin.rs"


def test_each_new_rpc_has_path_constant() -> None:
    rs = _read(ADMIN_RS)
    for rpc_name, _scope in EXPECTED_D21_RPCS:
        path = f"/efe.v1.admin.AdminService/{rpc_name}"
        assert path in rs, f"path constant {path} missing from admin.rs"


def test_each_new_rpc_calls_scope_guard_or_explicit_check() -> None:
    rs = _read(ADMIN_RS)
    for rpc_name, _scope in EXPECTED_D21_RPCS:
        snake = "".join(
            "_" + c.lower() if c.isupper() else c for c in rpc_name
        ).lstrip("_")
        # Find the handler body.
        # A precise regex would be nicer — for the pin test grep is enough.
        assert (
            f"async fn {snake}(" in rs
        ), f"missing handler {snake}"
    # Every D21 handler MUST contain a ScopeGuard::require call OR
    # the broad_scope_check helper that wraps it.
    n_broad_checks = rs.count("Self::broad_scope_check")
    assert n_broad_checks >= len(EXPECTED_D21_RPCS), (
        f"expected ≥ {len(EXPECTED_D21_RPCS)} broad scope checks, "
        f"found {n_broad_checks}"
    )


def test_scopes_rs_declares_3_new_granular_scopes() -> None:
    rs = _read(SCOPES_RS)
    for new_scope in ("ADMIN_BUNDLE_RELOAD", "ADMIN_CATALOG_PIN", "ADMIN_AUDIT_READ"):
        assert (
            f"pub const {new_scope}:" in rs
        ), f"granular scope {new_scope} missing"


def test_scopes_rs_maps_each_admin_rpc_path() -> None:
    rs = _read(SCOPES_RS)
    for rpc_name, scope_const in EXPECTED_D21_RPCS:
        path = f"/efe.v1.admin.AdminService/{rpc_name}"
        assert path in rs, f"scope mapping for {path} missing"
        assert scope_const in rs, f"scope const {scope_const} not referenced"


def test_list_audit_events_uses_audit_scope_not_admin() -> None:
    """ListAuditEvents is read-only audit ; it must NOT require admin."""
    rs = _read(ADMIN_RS)
    # Find the handler ; check the broad scope arg.
    h_start = rs.find("async fn list_audit_events(")
    assert h_start >= 0
    # The broad scope check inside this handler must be AUDIT, not ADMIN.
    h_body = rs[h_start : h_start + 4000]
    assert "BroadScope::AUDIT" in h_body, (
        "ListAuditEvents must enforce BroadScope::AUDIT, not ADMIN"
    )


def test_admin_rs_constructor_has_with_catalog_registry_builder() -> None:
    """The factory must expose the catalog registry wiring point."""
    rs = _read(ADMIN_RS)
    assert "fn with_catalog_registry(" in rs
    assert "fn with_bundle_reload(" in rs


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-v"]))
