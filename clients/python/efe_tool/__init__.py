"""EFE Mode-1 client library.

Call an **ephemeral fluid swarm** from any Python agent (Claude,
OpenAI, LangChain, plain code) as if it were a local function:

    >>> from efe_tool import EfeClient, AnchorCatalog
    >>> catalog = AnchorCatalog.load("catalogs/support_triage.yaml")
    >>> client = EfeClient.from_env()
    >>> result = client.classify("my payment was charged twice", catalog)
    >>> print(result.label, result.confidence)
    fraud 0.92

Public API surface (semver-stable post-D10):

    Client:
        EfeClient                     — synchronous gRPC client
        AsyncEfeClient                — asyncio client (D10 NEW)
        classify                      — single-shot helper

    Catalog:
        AnchorCatalog                 — anchor catalog model
        ClassDef                      — single class definition
        ThresholdPolicy               — confidence thresholds

    Embedding:
        Embedder                      — protocol
        LocalSTEmbedder               — sentence-transformers wrapper

    Result types:
        ClassifyResult, SubScore

    Errors (D10 hierarchy mapped to gRPC codes):
        EfeError                      — base
        EfeAuthError                  — UNAUTHENTICATED / PERMISSION_DENIED
        EfeRateLimitError             — RESOURCE_EXHAUSTED (retryable)
        EfeInvalidArgumentError       — INVALID_ARGUMENT (not retryable)
        EfeNotFoundError              — NOT_FOUND
        EfeUnavailableError           — UNAVAILABLE / DEADLINE_EXCEEDED (retryable)
        EfeInternalError              — INTERNAL / UNKNOWN
        raise_from_grpc(rpc_error)    — translation helper

    Errors (legacy — backward compatibility):
        BridgeError, ConfigError, CatalogError, EmbedderError,
        NetworkError, LowConfidenceWarning

Underscore-prefixed modules (``_async``, ``_internal``, ``_proto``,
``_version``) are NOT public — importing them is allowed but does
NOT respect semver between minor releases.

See ``docs.efe.ai/0.1.10/`` for the full documentation.
"""

from __future__ import annotations

from ._version import __version__, __version_tuple__, server_compatible_version
from .catalog import AnchorCatalog, ClassDef, ThresholdPolicy
from .client import EfeClient
from .embedder import Embedder, LocalSTEmbedder
from .errors import (
    # Legacy hierarchy (kept for backward compat)
    BridgeError,
    CatalogError,
    ConfigError,
    EmbedderError,
    LowConfidenceWarning,
    NetworkError,
    # D10 — new hierarchy mapped to gRPC codes
    EfeError,
    EfeAuthError,
    EfeRateLimitError,
    EfeInvalidArgumentError,
    EfeNotFoundError,
    EfeUnavailableError,
    EfeInternalError,
    raise_from_grpc,
)
from .types import ClassifyResult, SubScore

# Sprint D11 — multi-tenant admin surface.
# Sprint D12 — catalog admin + versioning extensions.
from .admin import (
    AdminError,
    CatalogAuditMetadata,
    CatalogRef,
    CompiledClassRef,
    EfeAdminClient,
    EvolveOp,
    RegisteredCatalog,
)

# Async client — D10 new (skeleton stub at v0.1.10, full impl at v0.2.0).
# We expose ``AsyncEfeClient`` lazily via PEP 562 module-level
# ``__getattr__`` so importing ``efe_tool`` doesn't require
# ``grpc.aio`` (mocked test suites benefit) AND so the user-facing
# ``AsyncEfeClient`` is the REAL class — ``isinstance(obj, AsyncEfeClient)``
# works exactly as expected. Audit fix A1 (replaced an instance-proxy
# pattern that broke isinstance checks).


# Sprint **D25 (Φ-C)** — `EfeNativeClient` via PyO3.
#
# `efe._native` is the binary wheel produced by `crates/efe-ffi/`
# (PyO3 zero-copy bindings, ≥ 5× speedup vs gRPC). Loaded LAZILY :
# users who haven't opted into `pip install efe-tool[native]` keep
# importing `efe_tool` for free without the binary import cost.
#
# `NATIVE_AVAILABLE` flips to ``True`` on the first successful
# import attempt (cached). `EfeFacade(client_kind="auto")` consults
# this flag to pick the right backend automatically.
#
# Pin tested by :
#   * `scripts/tests/test_d25_native_lazy_import_no_cost`
#   * `scripts/tests/test_d25_native_available_flag`

_NATIVE_AVAILABLE: bool | None = None


def _check_native_available() -> bool:
    """Cached probe : is `efe._native` importable ?

    The first call attempts the import ; subsequent calls reuse
    the cached result. Failure modes :
      * `ModuleNotFoundError`: wheel not installed → fallback gRPC.
      * `ImportError`: wheel installed but ABI mismatch → fallback gRPC.

    Both are silent — the fallback path is the safe default.
    """
    global _NATIVE_AVAILABLE
    if _NATIVE_AVAILABLE is None:
        try:
            import efe._native  # noqa: F401 — probe only.
            _NATIVE_AVAILABLE = True
        except (ModuleNotFoundError, ImportError):
            _NATIVE_AVAILABLE = False
    return _NATIVE_AVAILABLE


def __getattr__(name: str):
    """PEP 562 — lazy-load `AsyncEfeClient` + `NATIVE_AVAILABLE` on
    first attribute access.

    Caches each successful resolution as a module attribute so
    subsequent accesses (and `isinstance` checks) hit the regular
    module dict.
    """
    if name == "AsyncEfeClient":
        from ._async.client import AsyncEfeClient as _AsyncEfeClient
        globals()["AsyncEfeClient"] = _AsyncEfeClient
        return _AsyncEfeClient
    if name == "NATIVE_AVAILABLE":
        flag = _check_native_available()
        globals()["NATIVE_AVAILABLE"] = flag
        return flag
    if name == "EfeNativeClient":
        # Sprint D25 — surface the native client at the SDK root only
        # when the wheel is actually installed (lazy probe + cache).
        if _check_native_available():
            from efe._native import EfeNativeClient as _Native
            globals()["EfeNativeClient"] = _Native
            return _Native
        raise AttributeError(
            "EfeNativeClient unavailable — install efe-tool[native] "
            "to enable the PyO3 zero-copy in-process client (≥5× "
            "speedup vs gRPC). See docs/EFE_FFI_ARCHITECTURE.md."
        )
    raise AttributeError(
        f"module {__name__!r} has no attribute {name!r}"
    )


def __dir__():
    """PEP 562 — make lazy-imported names show up in `dir(efe_tool)`."""
    return sorted(set(globals()) | {
        "AsyncEfeClient",
        # Sprint D25 — lazy native surface (visible in dir() even if
        # the wheel isn't installed, so IDE auto-completion works).
        "NATIVE_AVAILABLE",
        "EfeNativeClient",
    })


__all__ = [
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
    # D11 — multi-tenant admin surface
    "EfeAdminClient",
    "CatalogRef",
    "AdminError",
    # D12 — catalog admin + versioning
    "EvolveOp",
    "RegisteredCatalog",
    "CatalogAuditMetadata",
    "CompiledClassRef",
    # **Audit fix M8-A6** — D25 native lazy-loaded surface MUST
    # appear in `__all__` for symmetry with `AsyncEfeClient` (also
    # PEP 562 lazy). Without this, `from efe_tool import *` did
    # NOT pull these names while pulling AsyncEfeClient.
    "NATIVE_AVAILABLE",
    "EfeNativeClient",
]


def classify(text: str, catalog: AnchorCatalog, **kwargs: object) -> ClassifyResult:
    """Shortcut: ``EfeClient.from_env().classify(text, catalog)``.

    Creates and tears down a client per call; for throughput use the
    long-lived :class:`EfeClient` instance directly.
    """
    client = EfeClient.from_env()
    try:
        return client.classify(text, catalog, **kwargs)
    finally:
        client.close()
