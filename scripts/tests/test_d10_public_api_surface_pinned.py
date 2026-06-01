"""Sprint D10 — pin the public API surface against accidental
breaking changes.

The ``efe_tool.__all__`` is the **contract** — every symbol there
is part of the public surface and a SemVer-bump is required to
remove it. This test fails loud the moment a PR removes a
public symbol, forcing the author to either:

    1. Restore it, OR
    2. Bump MINOR (pre-1.0) or MAJOR (post-1.0) version + add a
       ``# noqa: D10-SURFACE`` marker on this test's expected
       set with a CHANGELOG entry.

Pinned post-D10 freeze (Sprint D10 §4.1).
"""

from __future__ import annotations

import importlib
import sys
from pathlib import Path

import pytest


_REPO = Path(__file__).resolve().parents[2]
_SDK_PATH = _REPO / "clients" / "python"

# Make the SDK importable without `pip install -e .`
sys.path.insert(0, str(_SDK_PATH))


# ─────────────────────────────────────────────────────────────────────
# The expected public surface — UPDATE WITH CARE.
# ─────────────────────────────────────────────────────────────────────

EXPECTED_ALL = frozenset({
    # Version
    "__version__",
    "__version_tuple__",
    "server_compatible_version",
    # Client
    "EfeClient",
    "AsyncEfeClient",
    "classify",
    # Catalog
    "AnchorCatalog",
    "ClassDef",
    "ThresholdPolicy",
    # Embedder
    "Embedder",
    "LocalSTEmbedder",
    # Result types
    "ClassifyResult",
    "SubScore",
    # Legacy errors (backward compat)
    "BridgeError",
    "CatalogError",
    "ConfigError",
    "EmbedderError",
    "LowConfidenceWarning",
    "NetworkError",
    # D10 errors
    "EfeError",
    "EfeAuthError",
    "EfeRateLimitError",
    "EfeInvalidArgumentError",
    "EfeNotFoundError",
    "EfeUnavailableError",
    "EfeInternalError",
    "raise_from_grpc",
    # D11 — multi-tenant admin surface (added in v0.1.11)
    "EfeAdminClient",
    "CatalogRef",
    "AdminError",
    # D12 — catalog admin + versioning (added in v0.1.12)
    "EvolveOp",
    "RegisteredCatalog",
    "CatalogAuditMetadata",
    "CompiledClassRef",
})


@pytest.fixture(scope="module")
def efe_tool():
    return importlib.import_module("efe_tool")


def test_all_is_set(efe_tool):
    """`__all__` MUST be a list (not None, not undefined)."""
    assert hasattr(efe_tool, "__all__"), "`efe_tool.__all__` is missing"
    assert isinstance(efe_tool.__all__, list), \
        f"`__all__` is {type(efe_tool.__all__).__name__}, expected list"


def test_no_public_surface_removed(efe_tool):
    """Every symbol in EXPECTED_ALL must still be present."""
    actual = set(efe_tool.__all__)
    missing = EXPECTED_ALL - actual
    assert not missing, (
        f"D10 surface regression — public symbols REMOVED: {sorted(missing)}\n"
        f"Either restore them, or bump version (pre-1.0: MINOR; post-1.0: MAJOR) "
        f"AND update both CHANGELOG and EXPECTED_ALL in this file."
    )


def test_no_unannounced_additions(efe_tool):
    """New public symbols must be deliberately added to EXPECTED_ALL.

    This test fails when somebody adds a symbol to ``__all__`` without
    updating this expected set — preventing accidental public-surface
    growth that the team didn't intend.
    """
    actual = set(efe_tool.__all__)
    extras = actual - EXPECTED_ALL
    assert not extras, (
        f"D10 surface regression — public symbols ADDED without test update: "
        f"{sorted(extras)}.\n"
        f"If intentional, add them to EXPECTED_ALL in "
        f"`scripts/tests/test_d10_public_api_surface_pinned.py` AND "
        f"document in CHANGELOG."
    )


def test_each_symbol_resolves(efe_tool):
    """Every symbol in `__all__` must actually be importable."""
    for name in efe_tool.__all__:
        assert hasattr(efe_tool, name), (
            f"`efe_tool.{name}` is in __all__ but not actually defined"
        )


def test_version_matches_expected_format(efe_tool):
    v = efe_tool.__version__
    # SemVer-ish: X.Y.Z[-suffix].
    import re
    assert re.match(r"^\d+\.\d+\.\d+(-[A-Za-z0-9.\-]+)?$", v), \
        f"__version__ {v!r} is not SemVer-shaped"


def test_version_tuple_matches_string(efe_tool):
    v = efe_tool.__version__
    parts = [int(x) for x in v.split("-")[0].split(".")]
    assert tuple(parts) == efe_tool.__version_tuple__, (
        f"__version__ {v} != __version_tuple__ {efe_tool.__version_tuple__}"
    )


def test_server_compatible_version_returns_string(efe_tool):
    s = efe_tool.server_compatible_version()
    assert isinstance(s, str)
    # Pre-1.0: SDK and server share exact version.
    assert s == efe_tool.__version__


def test_efe_error_hierarchy_is_consistent(efe_tool):
    """`EfeError` must subclass `BridgeError` (backward compat) and
    every Efe* subclass must subclass `EfeError`."""
    assert issubclass(efe_tool.EfeError, efe_tool.BridgeError), (
        "EfeError must subclass BridgeError so legacy `except BridgeError` works"
    )
    for sub in (
        efe_tool.EfeAuthError,
        efe_tool.EfeRateLimitError,
        efe_tool.EfeInvalidArgumentError,
        efe_tool.EfeNotFoundError,
        efe_tool.EfeUnavailableError,
        efe_tool.EfeInternalError,
    ):
        assert issubclass(sub, efe_tool.EfeError), \
            f"{sub.__name__} must subclass EfeError"


def test_retryable_attribute_set_correctly(efe_tool):
    """The ``retryable`` class attribute must reflect the gRPC code's
    retry semantic (D10 §4.5)."""
    assert efe_tool.EfeAuthError.retryable is False
    assert efe_tool.EfeInvalidArgumentError.retryable is False
    assert efe_tool.EfeNotFoundError.retryable is False
    assert efe_tool.EfeInternalError.retryable is False
    assert efe_tool.EfeRateLimitError.retryable is True
    assert efe_tool.EfeUnavailableError.retryable is True


def test_py_typed_marker_exists():
    """PEP 561 — `py.typed` marker must be present so mypy /
    pyright detect typed library."""
    marker = _SDK_PATH / "efe_tool" / "py.typed"
    assert marker.exists(), (
        "PEP 561 marker `clients/python/efe_tool/py.typed` is missing"
    )


def test_async_client_is_lazy():
    """Importing `efe_tool` must NOT import grpc.aio — keep it lazy
    so test suites that don't need async aren't paying its cost."""
    # Reset import state.
    for mod in list(sys.modules):
        if mod.startswith("efe_tool._async"):
            del sys.modules[mod]
    import efe_tool   # noqa: F401
    assert "efe_tool._async.client" not in sys.modules, (
        "AsyncEfeClient should be a lazy proxy — actual import "
        "must be deferred until first attribute access."
    )
