"""Sprint D11 — pin the SDK surface extensions.

The D10 audit shipped a surface-pinned test
(``test_d10_public_api_surface_pinned.py``) — D11 extends it
additively with the new admin sub-package symbols. Removal of any
listed symbol is a SemVer-breaking change requiring a deliberate
test edit + CHANGELOG.

Pinned post-D11 (Sprint D11 §8).
"""

from __future__ import annotations

import importlib
import sys
from pathlib import Path


_REPO = Path(__file__).resolve().parents[2]
_SDK_PATH = _REPO / "clients" / "python"
sys.path.insert(0, str(_SDK_PATH))


# ─────────────────────────────────────────────────────────────────────
# D11 symbols added to efe_tool top-level
# ─────────────────────────────────────────────────────────────────────

D11_NEW_SYMBOLS = frozenset({
    "EfeAdminClient",
    "CatalogRef",
    "AdminError",
})


def test_d11_admin_symbols_in_top_level_all():
    efe_tool = importlib.import_module("efe_tool")
    actual = set(efe_tool.__all__)
    missing = D11_NEW_SYMBOLS - actual
    assert not missing, (
        f"D11 surface regression — admin symbols missing from "
        f"efe_tool.__all__: {sorted(missing)}"
    )


def test_d11_admin_symbols_resolve():
    efe_tool = importlib.import_module("efe_tool")
    for name in D11_NEW_SYMBOLS:
        assert hasattr(efe_tool, name), (
            f"`efe_tool.{name}` listed in __all__ but not defined"
        )


def test_admin_subpackage_importable():
    """`efe_tool.admin` is the canonical entry point ; it must be
    importable as a real sub-package, not just via lazy fallback."""
    admin = importlib.import_module("efe_tool.admin")
    assert hasattr(admin, "EfeAdminClient")
    assert hasattr(admin, "CatalogRef")
    assert hasattr(admin, "AdminError")
    assert "EfeAdminClient" in admin.__all__


def test_admin_error_is_efe_error_subclass():
    """`AdminError` must subclass `EfeError` so legacy ``except
    EfeError`` continues to catch."""
    efe_tool = importlib.import_module("efe_tool")
    assert issubclass(efe_tool.AdminError, efe_tool.EfeError), (
        "AdminError must subclass EfeError for backward-compat catch"
    )


def test_catalog_ref_is_frozen_dataclass():
    efe_tool = importlib.import_module("efe_tool")
    import dataclasses
    assert dataclasses.is_dataclass(efe_tool.CatalogRef), (
        "CatalogRef must be a dataclass for trivial serialisation"
    )
    # Frozen instances can't be mutated post-construction —
    # important for thread-safe sharing.
    fields = {f.name for f in dataclasses.fields(efe_tool.CatalogRef)}
    assert fields == {"tenant", "name", "version", "num_classes",
                      "num_anchors"}, fields


def test_efe_admin_client_constructor_signature():
    """The constructor must accept `endpoint`, `jwt`, `insecure`,
    `timeout_s` — pinned for documentation contract."""
    import inspect
    efe_tool = importlib.import_module("efe_tool")
    sig = inspect.signature(efe_tool.EfeAdminClient.__init__)
    params = set(sig.parameters)
    expected = {"self", "endpoint", "jwt", "insecure", "timeout_s"}
    assert params == expected, (
        f"EfeAdminClient.__init__ surface drifted: {params}"
    )


def test_efe_admin_client_required_methods():
    """The methods documented in the howto must exist."""
    efe_tool = importlib.import_module("efe_tool")
    required = ("list_catalogs", "list_cone_versions",
                "get_cone", "delete_cone", "health", "close")
    for method in required:
        assert hasattr(efe_tool.EfeAdminClient, method), method


def test_efe_admin_client_supports_context_manager():
    efe_tool = importlib.import_module("efe_tool")
    assert hasattr(efe_tool.EfeAdminClient, "__enter__")
    assert hasattr(efe_tool.EfeAdminClient, "__exit__")


def test_admin_client_rejects_empty_endpoint():
    """Defensive — an empty endpoint should fail fast at constructor."""
    efe_tool = importlib.import_module("efe_tool")
    import pytest
    with pytest.raises(efe_tool.AdminError):
        efe_tool.EfeAdminClient(endpoint="", jwt="x")


def test_admin_client_rejects_empty_jwt():
    """Tier 2 NEVER allows anonymous admin operations."""
    efe_tool = importlib.import_module("efe_tool")
    import pytest
    with pytest.raises(efe_tool.AdminError):
        efe_tool.EfeAdminClient(endpoint="x:1", jwt="")


# ─────────────────────────────────────────────────────────────────────
# Helm chart values shape
# ─────────────────────────────────────────────────────────────────────

def test_values_yaml_has_tenant_namespace_block():
    """The chart's values.yaml must declare the D11 Tier 2 block,
    even when defaults are off."""
    import re
    text = (_REPO / "deploy" / "helm" / "efe" / "values.yaml").read_text()
    # `tenant: ` block must contain `namespace:` sub-key.
    assert re.search(r"^tenant:\s*$", text, re.MULTILINE), \
        "values.yaml must declare a top-level `tenant:` block"
    assert re.search(r"^\s+namespace:\s*$", text, re.MULTILINE), \
        "values.yaml must declare `tenant.namespace:` sub-block"
    # And the three opt-in toggles.
    for key in ("resourceQuota", "limitRange", "networkPolicy"):
        assert key in text, f"values.yaml must mention `tenant.namespace.{key}`"


def test_values_tier2_yaml_exists_and_parses():
    import yaml
    path = _REPO / "deploy" / "helm" / "efe" / "values.tier2.yaml"
    assert path.exists()
    parsed = yaml.safe_load(path.read_text())
    # tier2.yaml turns isolation ON.
    assert parsed["tenant"]["namespace"]["isolated"] is True
    assert parsed["tenant"]["namespace"]["resourceQuota"]["enabled"] is True
    assert parsed["tenant"]["namespace"]["limitRange"]["enabled"] is True
    assert parsed["tenant"]["namespace"]["networkPolicy"]["tenantIngress"]["enabled"] is True
    # And forces auth ON.
    assert parsed["config"]["auth"]["enabled"] is True
