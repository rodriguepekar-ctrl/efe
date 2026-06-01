"""EFE — Ephemeral Fluid Swarm. Python bindings (M3, Sprint D25).

Cf. ``docs/EFE_FFI_ARCHITECTURE.md``.

Re-exports the public surface from the PyO3 native module
``efe._native`` so users can :

>>> import efe
>>> verdict = efe.classify_zero_copy(
...     "hello world",
...     catalog_id="router-v1",
...     bundle_path="/etc/efe/bundles/active",
...     catalog_path="/etc/efe/catalogs/router-v1.json",
... )

Co-existence : the historical gRPC client ``EfeClient`` is also
re-exported (when the ``efe_tool`` package is installed) so
existing imports keep working :

>>> from efe import EfeClient   # gRPC, multi-tenant
>>> from efe import EfeNativeClient  # PyO3, in-process (5× faster)
"""

from __future__ import annotations

from efe._native import (
    AlignmentBreach,
    AlignmentCone,
    Bundle,
    BundleError,
    CatalogError,
    ConfigError,
    EfeError,
    EfeNativeClient,
    EpistemicLatentSpace,
    FfiError,
    NumericalError,
    ResourceExhausted,
    SwarmController,
    SwarmError,
    SwarmSpec,
    SwarmSpecBuilder,  # Φ-C-FFI : registered explicitly post audit fix F-A1
    TimeoutError,
    Verdict,
    aclassify_zero_copy,
    classify_zero_copy,
    load_bundle,
    version,
)

# Optional gRPC client. Pre-D25 path ; preserved unchanged. Falls
# back gracefully if `efe_tool` (the gRPC client package) is not
# installed. The new D25 PyO3 surface lives unconditionally above.
try:
    from efe_tool import EfeClient  # type: ignore[import-not-found]

    _GRPC_EXPORTS: list[str] = ["EfeClient"]
except ImportError:
    _GRPC_EXPORTS = []

# ────────────────────────────────────────────────────────────────
# Public surface
# ────────────────────────────────────────────────────────────────

__all__ = [
    # Classes
    "AlignmentCone",
    "Bundle",
    "EfeNativeClient",
    "EpistemicLatentSpace",
    "SwarmController",
    "SwarmSpec",
    "SwarmSpecBuilder",  # Φ-C-FFI
    "Verdict",
    # Exceptions
    "AlignmentBreach",
    "BundleError",
    "CatalogError",
    "ConfigError",
    "EfeError",
    "FfiError",
    "NumericalError",
    "ResourceExhausted",
    "SwarmError",
    "TimeoutError",
    # Functions
    "aclassify_zero_copy",
    "classify_zero_copy",
    "load_bundle",
    "version",
    # Optional gRPC re-export
    *_GRPC_EXPORTS,
]

__version__: str = version()
