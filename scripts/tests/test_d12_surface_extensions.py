"""Sprint D12 — pin the SDK surface extensions.

D11 added the admin sub-package ; D12 extends it additively with :

  * 4 new types (EvolveOp, RegisteredCatalog, CatalogAuditMetadata,
    CompiledClassRef) — re-exported at both ``efe_tool.admin`` and
    ``efe_tool``.
  * 6 new methods on EfeAdminClient (register_catalog, evolve_catalog,
    get_catalog, list_catalog_versions, delete_catalog, plus the
    activated list_catalogs).

Removal of any pinned symbol is a SemVer-breaking change requiring a
deliberate test edit + CHANGELOG bump.

Pinned post-D12 (Sprint D12 § 8).
"""

from __future__ import annotations

import importlib
import sys
from pathlib import Path

_REPO = Path(__file__).resolve().parents[2]
_SDK_PATH = _REPO / "clients" / "python"
sys.path.insert(0, str(_SDK_PATH))


# ─────────────────────────────────────────────────────────────────────
# D12 symbols added to efe_tool top-level
# ─────────────────────────────────────────────────────────────────────

D12_NEW_SYMBOLS = frozenset({
    "EvolveOp",
    "RegisteredCatalog",
    "CatalogAuditMetadata",
    "CompiledClassRef",
})


def test_d12_admin_symbols_in_top_level_all():
    efe_tool = importlib.import_module("efe_tool")
    actual = set(efe_tool.__all__)
    missing = D12_NEW_SYMBOLS - actual
    assert not missing, (
        f"D12 surface regression — admin types missing from "
        f"efe_tool.__all__: {sorted(missing)}"
    )


def test_d12_admin_symbols_resolve():
    efe_tool = importlib.import_module("efe_tool")
    for name in D12_NEW_SYMBOLS:
        assert hasattr(efe_tool, name), (
            f"`efe_tool.{name}` listed in __all__ but not defined"
        )


def test_d12_admin_subpackage_exports_d12_types():
    admin = importlib.import_module("efe_tool.admin")
    for name in D12_NEW_SYMBOLS:
        assert hasattr(admin, name), \
            f"efe_tool.admin must export {name!r}"
        assert name in admin.__all__, \
            f"efe_tool.admin.__all__ must include {name!r}"


def test_d12_top_level_re_exports_are_identical():
    """`from efe_tool import EvolveOp` must yield the SAME class as
    `from efe_tool.admin import EvolveOp` (no re-wrapping)."""
    efe_tool = importlib.import_module("efe_tool")
    admin = importlib.import_module("efe_tool.admin")
    for name in D12_NEW_SYMBOLS:
        assert getattr(efe_tool, name) is getattr(admin, name), (
            f"`efe_tool.{name}` and `efe_tool.admin.{name}` must be "
            f"the SAME object (not separate imports)"
        )


# ─────────────────────────────────────────────────────────────────────
# EvolveOp shape
# ─────────────────────────────────────────────────────────────────────

def test_evolve_op_has_5_variants():
    from efe_tool.admin import EvolveOp
    expected = {"APPEND_CLASS", "REMOVE_CLASS", "UPDATE_THRESHOLDS",
                "REPLACE_CLASS_ANCHORS", "RECOMPILE"}
    actual = {v.name for v in EvolveOp}
    assert actual == expected, (
        f"EvolveOp variants drift: expected {expected}, got {actual}"
    )


def test_evolve_op_values_match_proto_oneof_names():
    """The lowercase string values must match the oneof variant names
    in catalog.proto so the audit_tag round-trips."""
    from efe_tool.admin import EvolveOp
    expected = {
        EvolveOp.APPEND_CLASS: "append_class",
        EvolveOp.REMOVE_CLASS: "remove_class",
        EvolveOp.UPDATE_THRESHOLDS: "update_thresholds",
        EvolveOp.REPLACE_CLASS_ANCHORS: "replace_class_anchors",
        EvolveOp.RECOMPILE: "recompile",
    }
    for variant, expected_value in expected.items():
        assert variant.value == expected_value, (
            f"{variant.name}.value should be {expected_value!r}, "
            f"got {variant.value!r}"
        )


# ─────────────────────────────────────────────────────────────────────
# RegisteredCatalog / CatalogAuditMetadata / CompiledClassRef shape
# ─────────────────────────────────────────────────────────────────────

def test_registered_catalog_is_frozen_dataclass():
    import dataclasses
    from efe_tool.admin import RegisteredCatalog
    assert dataclasses.is_dataclass(RegisteredCatalog)
    fields = {f.name for f in dataclasses.fields(RegisteredCatalog)}
    expected = {"key", "tenant", "name", "version", "content",
                "metadata", "compiled"}
    assert fields == expected, fields


def test_catalog_audit_metadata_is_frozen_dataclass():
    import dataclasses
    from efe_tool.admin import CatalogAuditMetadata
    assert dataclasses.is_dataclass(CatalogAuditMetadata)
    fields = {f.name for f in dataclasses.fields(CatalogAuditMetadata)}
    expected = {"created_at", "created_by", "description",
                "parent_key", "evolution_op", "labels", "content_sha256"}
    assert fields == expected, fields


def test_compiled_class_ref_is_frozen_dataclass():
    import dataclasses
    from efe_tool.admin import CompiledClassRef
    assert dataclasses.is_dataclass(CompiledClassRef)
    fields = {f.name for f in dataclasses.fields(CompiledClassRef)}
    expected = {"class_name", "num_anchors", "embedder_fingerprint"}
    assert fields == expected, fields


# ─────────────────────────────────────────────────────────────────────
# EfeAdminClient new methods (D12)
# ─────────────────────────────────────────────────────────────────────

D12_NEW_METHODS = (
    "register_catalog",
    "evolve_catalog",
    "get_catalog",
    "list_catalog_versions",
    "delete_catalog",
)


def test_efe_admin_client_has_d12_methods():
    from efe_tool.admin import EfeAdminClient
    for method in D12_NEW_METHODS:
        assert hasattr(EfeAdminClient, method), (
            f"EfeAdminClient.{method} missing — D12 surface regression"
        )
        assert callable(getattr(EfeAdminClient, method))


def test_list_catalogs_no_longer_raises_not_implemented():
    """D11 stubbed list_catalogs as raising NotImplementedError. D12
    activates the real RPC. The method must accept its documented
    keyword args without raising NotImplementedError before reaching
    the gRPC layer."""
    from efe_tool.admin import EfeAdminClient
    import inspect
    sig = inspect.signature(EfeAdminClient.list_catalogs)
    # Must accept keyword-only `tenant`, `page_size`, `page_token`.
    params = sig.parameters
    assert "tenant" in params
    assert "page_size" in params
    assert "page_token" in params


# ─────────────────────────────────────────────────────────────────────
# CLI surface
# ─────────────────────────────────────────────────────────────────────

def test_cli_catalog_subcommand_parsable():
    """`efe-admin catalog list` must parse cleanly without errors."""
    from efe_tool.cli.parser import parse_args
    ns = parse_args(["catalog", "list"])
    assert ns.cmd == "catalog"
    assert ns.catalog_action == "list"


def test_cli_catalog_register_requires_name_and_content():
    """Missing required flags must produce a UsageError."""
    from efe_tool.cli.parser import parse_args
    from efe_tool.cli.errors import UsageError
    import pytest
    with pytest.raises(UsageError):
        parse_args(["catalog", "register"])


def test_cli_catalog_evolve_op_choices():
    """The 5 evolve op flag values must match the proto oneof names
    (with hyphens substituted for underscores in the CLI)."""
    from efe_tool.cli.parser import parse_args
    from efe_tool.cli.errors import UsageError
    import pytest

    # Allowed ops :
    for op in ("append-class", "remove-class", "update-thresholds",
               "replace-class-anchors", "recompile"):
        ns = parse_args([
            "catalog", "evolve",
            "--from-key", "acme/x@v1",
            "--op", op,
            "--payload", "/etc/hosts",
        ])
        assert ns.catalog_op == op

    # Disallowed ops :
    with pytest.raises(UsageError):
        parse_args([
            "catalog", "evolve",
            "--from-key", "acme/x@v1",
            "--op", "banana",
            "--payload", "/etc/hosts",
        ])
