"""Error taxonomy for the EFE Mode-1 SDK.

The hierarchy lets callers catch at the right level:

    try:
        client.classify(...)
    except EfeAuthError:
        # token expired/invalid — refresh and retry
    except EfeRateLimitError:
        # backoff with jitter and retry
    except EfeUnavailableError:
        # transient (network / 503) — retry
    except EfeInvalidArgumentError:
        # request shape wrong — fix code, do NOT retry
    except EfeError:
        # anything else from the SDK

Sprint D10 — public exception surface
─────────────────────────────────────

The legacy ``BridgeError``, ``ConfigError``, ``CatalogError``,
``EmbedderError``, ``NetworkError`` are kept for backward
compatibility; new code SHOULD prefer the ``Efe*Error`` family that
maps 1:1 to gRPC status codes (cf. ``reference/errors.md``).

The translation utility ``raise_from_grpc(rpc_error) -> EfeError``
wraps a raw ``grpc.RpcError`` into the right subclass with the
``retryable`` hint set so callers can implement exponential backoff
without poking at gRPC internals.
"""

from __future__ import annotations

from typing import Any


# ─────────────────────────────────────────────────────────────────────
# Legacy hierarchy (pre-D10 — kept for backward compatibility)
# ─────────────────────────────────────────────────────────────────────

class BridgeError(Exception):
    """Base class for every error emitted by the Mode-1 bridge."""


class ConfigError(BridgeError):
    """Invalid configuration (env var malformed, catalog file missing…)."""


class CatalogError(BridgeError):
    """The anchor catalog failed to load or validate."""


class EmbedderError(BridgeError):
    """The embedder could not produce a vector."""


class NetworkError(BridgeError):
    """The gRPC call to the EFE server failed at the transport level."""


class LowConfidenceWarning(UserWarning):
    """Non-fatal: the swarm produced a low-confidence verdict."""


# ─────────────────────────────────────────────────────────────────────
# Sprint D10 — public Efe* hierarchy mapped to gRPC status codes.
# ─────────────────────────────────────────────────────────────────────

class EfeError(BridgeError):
    """Base for every SDK-emitted error post-D10.

    Subclass of :class:`BridgeError` so that legacy ``except
    BridgeError`` catches still trigger.

    Attributes:
        message: human-readable error message (does not leak internals).
        grpc_code: the underlying gRPC status code integer (or None).
        details: dict carrying ``grpc_status`` (str name) and
            arbitrary metadata.
        retryable: whether the caller should retry with backoff.
    """

    #: subclasses override to declare retry semantic at class level
    retryable: bool = False

    def __init__(
        self,
        message: str,
        *,
        grpc_code: int | None = None,
        details: dict[str, Any] | None = None,
        retryable: bool | None = None,
    ) -> None:
        super().__init__(message)
        self.message = message
        self.grpc_code = grpc_code
        self.details = dict(details or {})
        # An instance-level override wins over class default
        # (lets callers force a retry on an otherwise non-retryable
        # error after manual investigation).
        if retryable is not None:
            self.retryable = retryable


class EfeAuthError(EfeError):
    """gRPC ``UNAUTHENTICATED`` (16) — token absent / invalid / expired.

    NOT retryable — the caller must refresh the token before retrying.
    """
    retryable = False


class EfeRateLimitError(EfeError):
    """gRPC ``RESOURCE_EXHAUSTED`` (8) — server-side rate limit hit.

    Retryable — the caller should backoff with jitter and retry.
    """
    retryable = True


class EfeInvalidArgumentError(EfeError):
    """gRPC ``INVALID_ARGUMENT`` (3) / ``FAILED_PRECONDITION`` (9).

    Request shape is wrong; retrying with the same payload will fail
    with the same error. NOT retryable.
    """
    retryable = False


class EfeUnavailableError(EfeError):
    """gRPC ``UNAVAILABLE`` (14) / ``DEADLINE_EXCEEDED`` (4).

    Transient — retryable with exponential backoff.
    """
    retryable = True


class EfeInternalError(EfeError):
    """gRPC ``INTERNAL`` (13) / ``UNKNOWN`` (2) — server bug or panic.

    NOT retryable — hammering a panicking server makes things worse.
    File a bug at https://github.com/efe-ai/efe/issues.
    """
    retryable = False


class EfeNotFoundError(EfeError):
    """gRPC ``NOT_FOUND`` (5) — referenced catalog / cone does not exist.

    NOT retryable.
    """
    retryable = False


# Mapping gRPC status code (int) → EfeError subclass.
# Populated lazily on first call to keep grpc import optional.
_CODE_MAP: dict[int, type[EfeError]] = {}


def _ensure_code_map() -> dict[int, type[EfeError]]:
    """Populate the gRPC code → exception map on first use.

    Lazy import of ``grpc`` keeps this module light when callers
    only use catalog/embedder helpers.
    """
    global _CODE_MAP
    if _CODE_MAP:
        return _CODE_MAP
    try:
        import grpc
    except ImportError:  # pragma: no cover — grpcio is a hard dep
        return {}
    _CODE_MAP = {
        grpc.StatusCode.UNAUTHENTICATED.value[0]: EfeAuthError,
        grpc.StatusCode.PERMISSION_DENIED.value[0]: EfeAuthError,
        grpc.StatusCode.RESOURCE_EXHAUSTED.value[0]: EfeRateLimitError,
        grpc.StatusCode.INVALID_ARGUMENT.value[0]: EfeInvalidArgumentError,
        grpc.StatusCode.FAILED_PRECONDITION.value[0]: EfeInvalidArgumentError,
        grpc.StatusCode.OUT_OF_RANGE.value[0]: EfeInvalidArgumentError,
        grpc.StatusCode.NOT_FOUND.value[0]: EfeNotFoundError,
        grpc.StatusCode.UNAVAILABLE.value[0]: EfeUnavailableError,
        grpc.StatusCode.DEADLINE_EXCEEDED.value[0]: EfeUnavailableError,
        grpc.StatusCode.INTERNAL.value[0]: EfeInternalError,
        grpc.StatusCode.UNKNOWN.value[0]: EfeInternalError,
    }
    return _CODE_MAP


def raise_from_grpc(rpc_error: Any) -> "EfeError":
    """Translate a ``grpc.RpcError`` → typed :class:`EfeError` subclass.

    The original ``grpc.RpcError`` is preserved as ``__cause__`` so
    callers retain access to the raw status / metadata if needed::

        try:
            client.execute(req)
        except grpc.RpcError as e:
            raise raise_from_grpc(e) from e

    Args:
        rpc_error: the raised ``grpc.RpcError`` (or any object with
            ``.code()`` and ``.details()`` methods).

    Returns:
        An :class:`EfeError` instance ready to ``raise``.
    """
    code_map = _ensure_code_map()
    try:
        code = rpc_error.code()
        code_int = code.value[0] if hasattr(code, "value") else int(code)
        code_name = code.name if hasattr(code, "name") else str(code)
    except Exception:  # pragma: no cover — defensive
        code_int = None
        code_name = "UNKNOWN"
    cls = code_map.get(code_int, EfeError) if code_int is not None else EfeError
    try:
        message = rpc_error.details() or str(rpc_error)
    except Exception:  # pragma: no cover
        message = str(rpc_error)
    return cls(
        message,
        grpc_code=code_int,
        details={"grpc_status": code_name},
    )
