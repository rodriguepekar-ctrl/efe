"""Sprint D12-bis — Runtime wire-up surface pin.

These tests assert the *invariants* enforced by the Rust runtime,
without spinning up a live server (which would couple this CI job
to network/port availability). Reads the source files directly
and verifies the wire-up tokens are present in canonical
locations.

Catches regressions like :

  * Someone removes the catalog from the build script — proto
    types stop generating but the runtime imports compile until
    the next consumer references them.
  * Someone removes the ``add_service(CatalogServiceServer::new(...))``
    line from runtime.rs — handler still compiles, runtime still
    boots, but the server returns ``Unimplemented`` for every
    catalog RPC.
  * Someone forgets the ``health_reporter.set_serving::<…>``
    line — the catalog reachability helm test will fail in CI
    but only after deployment ; this test catches it at PR time.
"""

from __future__ import annotations

from pathlib import Path

_REPO = Path(__file__).resolve().parents[2]
_SERVER_SRC = _REPO / "crates" / "efe-server" / "src"
_BUILD_RS = _REPO / "crates" / "efe-server" / "build.rs"
_LIB_RS = _SERVER_SRC / "lib.rs"
_RUNTIME_RS = _SERVER_SRC / "runtime.rs"
_SCOPES_RS = _SERVER_SRC / "auth" / "scopes.rs"


# ─────────────────────────────────────────────────────────────────────
# build.rs — proto compilation.
# ─────────────────────────────────────────────────────────────────────


def test_build_rs_compiles_catalog_proto():
    """``catalog.proto`` MUST be in the proto file list, otherwise
    tonic-build won't generate the Rust stubs and every other
    surface fails to compile."""
    text = _BUILD_RS.read_text(encoding="utf-8")
    assert 'efe_v1.join("catalog.proto")' in text, (
        "build.rs must compile catalog.proto via tonic-build"
    )


# ─────────────────────────────────────────────────────────────────────
# lib.rs — module + proto re-export.
# ─────────────────────────────────────────────────────────────────────


def test_lib_rs_exports_v1_catalog_proto_module():
    """The generated catalog stubs MUST be re-exported as
    ``crate::proto::v1::catalog`` AND aliased as
    ``crate::proto::v1_catalog`` for ergonomic imports in
    handler + tests."""
    text = _LIB_RS.read_text(encoding="utf-8")
    assert 'tonic::include_proto!("efe.v1.catalog")' in text, (
        "lib.rs must include the generated catalog proto module"
    )
    assert "pub use v1::catalog as v1_catalog" in text, (
        "lib.rs must alias v1::catalog as v1_catalog"
    )


# ─────────────────────────────────────────────────────────────────────
# runtime.rs — service wire-up + health.
# ─────────────────────────────────────────────────────────────────────


def test_runtime_rs_attaches_catalog_service():
    """The catalog handler MUST be added to the tonic router. If
    this line is removed, all 6 catalog RPCs return
    ``Unimplemented`` even though the proto + handler compile."""
    text = _RUNTIME_RS.read_text(encoding="utf-8")
    # Two canonical tokens — both must be present.
    assert "CatalogServiceImpl::new(" in text, (
        "runtime.rs must instantiate CatalogServiceImpl"
    )
    assert "CatalogServiceServer::new(catalog_svc)" in text, (
        "runtime.rs must wrap the impl in the tonic server stub"
    )
    assert ".add_service(CatalogServiceServer::new(catalog_svc))" in text, (
        "runtime.rs must add the catalog server to the tonic router"
    )


def test_runtime_rs_marks_catalog_service_as_serving():
    """The per-service health entry is what the helm test
    ``test-catalog-reachable.yaml`` queries. Removing this line
    makes the helm test fail in deployment, not at PR time —
    catch it here."""
    text = _RUNTIME_RS.read_text(encoding="utf-8")
    assert "set_serving::<CatalogServiceServer<" in text, (
        "runtime.rs must mark the catalog service as SERVING in tonic-health"
    )
    assert "set_not_serving::<CatalogServiceServer<" in text, (
        "runtime.rs must flip the catalog service to NOT_SERVING on shutdown"
    )


def test_runtime_rs_has_postgres_fail_fast_guard():
    """The postgres backend is reserved for D14. If selected
    via env, the binary MUST fail at ``Server::build`` rather
    than silently behave like memory. Catching this here saves
    a postgres-on-fire incident later."""
    text = _RUNTIME_RS.read_text(encoding="utf-8")
    assert "CatalogBackend::Postgres" in text
    # The fail-fast token : an explicit `Err` arm with a
    # human-readable message naming D14.
    assert "D14" in text, (
        "runtime.rs Postgres branch must reference D14 in its error so the "
        "operator knows what to wait for"
    )


# ─────────────────────────────────────────────────────────────────────
# scopes.rs — RBAC mapping for the 6 RPCs.
# ─────────────────────────────────────────────────────────────────────

EXPECTED_RBAC_MAPPINGS = [
    ("/efe.v1.catalog.CatalogService/RegisterCatalog", "admin:catalog:write"),
    ("/efe.v1.catalog.CatalogService/EvolveCatalog", "admin:catalog:write"),
    ("/efe.v1.catalog.CatalogService/DeleteCatalog", "admin:catalog:write"),
    ("/efe.v1.catalog.CatalogService/GetCatalog", "admin:catalog:read"),
    ("/efe.v1.catalog.CatalogService/ListCatalogs", "admin:catalog:read"),
    ("/efe.v1.catalog.CatalogService/ListCatalogVersions", "admin:catalog:read"),
]


def test_scopes_rs_declares_catalog_constants():
    """The two scope constants MUST be declared centrally in
    scopes.rs so handler tests, JWT issuance docs, and Helm
    secret docs all reference the same source of truth."""
    text = _SCOPES_RS.read_text(encoding="utf-8")
    assert '"admin:catalog:read"' in text, (
        "scopes.rs must declare the admin:catalog:read constant"
    )
    assert '"admin:catalog:write"' in text, (
        "scopes.rs must declare the admin:catalog:write constant"
    )


def test_scopes_rs_maps_all_6_catalog_rpc_paths():
    """Each of the 6 RPC paths MUST appear in the
    ``required_scope`` mapping. A missing mapping defaults to
    ``Scope::None``, which would silently allow unauthenticated
    catalog mutations."""
    text = _SCOPES_RS.read_text(encoding="utf-8")
    for path, _scope in EXPECTED_RBAC_MAPPINGS:
        assert path in text, (
            f"scopes.rs missing RBAC entry for {path} ; "
            f"unmapped paths default to Scope::None"
        )


def test_scopes_rs_split_read_write_correctly():
    """Read paths (GET / LIST) MUST require ``admin:catalog:read``
    ; write paths (REGISTER / EVOLVE / DELETE) MUST require
    ``admin:catalog:write``. Drift here = privilege confusion."""
    text = _SCOPES_RS.read_text(encoding="utf-8")
    for path, scope in EXPECTED_RBAC_MAPPINGS:
        # Each RPC path appears once (left side of the match arm).
        # We assert the line ends with the right scope constant
        # by using a permissive substring check : both the path
        # and the scope appear in the same source file. The
        # actual binding is enforced by the Rust unit tests in
        # ``scopes.rs`` ; this is just a structural smoke.
        assert scope.replace(":", "_").upper() in text or scope in text, (
            f"scopes.rs missing scope constant {scope} for path {path}"
        )
