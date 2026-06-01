"""Exception taxonomy for the Letter-D façade.

The host application receives ONLY subclasses of :class:`EfeFacadeError`.
No :class:`grpc.RpcError`, no :class:`anthropic.APIError`, no
``sentence_transformers`` leak escapes the façade.

Each exception carries:

* :attr:`error_code`           — short machine-readable tag.
* :attr:`suggested_http_status`— default HTTP response a web host may use.
* :attr:`hint`                 — operator-facing remediation string.
* :attr:`context`              — structured debug details (dict).
* ``__cause__``                — the original (bridge/gRPC) exception.

Mapping (canonical, see LETTER_D_ARCHITECTURE.md §10):

+--------------------------------+------+----------+---------------------+
|  class                         | HTTP | retryable| raised when         |
+--------------------------------+------+----------+---------------------+
|  EfeFacadeConfigError          | 500  | no       | config invalid      |
|  EfeFacadeInitError            | 500  | no       | preload failed      |
|  EfeFacadeClosedError          | 503  | no       | after close()       |
|  EfeFacadeCatalogError         | 500  | no       | catalog YAML issue  |
|  EfeFacadeUsageError           | 422  | no       | bad caller input    |
|  EfeFacadeTransientError       | 503  | YES      | network blip        |
|  EfeFacadeTimeoutError         | 504  | YES      | RPC deadline        |
|  EfeFacadeRejectedError        | 422  | no       | strict=True + reject|
+--------------------------------+------+----------+---------------------+
"""

from __future__ import annotations

from typing import Any, Mapping, Optional


# ---------------------------------------------------------------------------
#  Root
# ---------------------------------------------------------------------------


class EfeFacadeError(Exception):
    """Base class for every error a host application receives from EFE."""

    error_code: str = "EFE_BUG"
    suggested_http_status: int = 500
    retryable: bool = False

    def __init__(
        self,
        message: str,
        *,
        hint: Optional[str] = None,
        context: Optional[Mapping[str, Any]] = None,
        cause: Optional[BaseException] = None,
    ) -> None:
        super().__init__(message)
        self.message = message
        self.hint = hint
        self.context: dict[str, Any] = dict(context or {})
        if cause is not None:
            # Preserve the classic ``raise ... from ...`` chain.
            self.__cause__ = cause

    def to_payload(self) -> dict[str, Any]:
        """JSON-serialisable representation suitable for HTTP responses
        or log enrichment."""
        payload: dict[str, Any] = {
            "error": self.error_code,
            "http_status": self.suggested_http_status,
            "retryable": self.retryable,
            "message": self.message,
        }
        if self.hint:
            payload["hint"] = self.hint
        if self.context:
            payload["context"] = dict(self.context)
        if self.__cause__ is not None:
            payload["cause"] = {
                "type": type(self.__cause__).__name__,
                "detail": str(self.__cause__)[:500],
            }
        return payload

    def __str__(self) -> str:
        return self.message


# ---------------------------------------------------------------------------
#  Boot / lifecycle (500 family)
# ---------------------------------------------------------------------------


class EfeFacadeConfigError(EfeFacadeError):
    """The :class:`EfeConfig` is inconsistent or invalid."""

    error_code = "EFE_CONFIG_INVALID"
    suggested_http_status = 500


class EfeFacadeInitError(EfeFacadeError):
    """``preload()`` could not initialise the embedder or a catalog."""

    error_code = "EFE_INIT_FAILED"
    suggested_http_status = 500


class EfeFacadeClosedError(EfeFacadeError):
    """A method was called after :meth:`EfeFacade.close`."""

    error_code = "EFE_CLOSED"
    suggested_http_status = 503


class EfeFacadeCatalogError(EfeFacadeError):
    """A catalog YAML is malformed, missing, or its fingerprint
    disagrees with the loaded embedder."""

    error_code = "EFE_CATALOG_INVALID"
    suggested_http_status = 500


# ---------------------------------------------------------------------------
#  Caller input (422 family)
# ---------------------------------------------------------------------------


class EfeFacadeUsageError(EfeFacadeError):
    """The caller passed invalid input (empty text, unknown catalog,
    negative timeout, …). Not retryable; fix the code."""

    error_code = "EFE_USAGE"
    suggested_http_status = 422


class EfeFacadeRejectedError(EfeFacadeError):
    """``strict=True`` was requested and the swarm returned
    ``reject=True`` or a confidence below the configured high threshold."""

    error_code = "EFE_REJECTED"
    suggested_http_status = 422


# ---------------------------------------------------------------------------
#  Network / server (503/504 family, retryable)
# ---------------------------------------------------------------------------


class EfeFacadeTransientError(EfeFacadeError):
    """The gRPC server is temporarily unreachable or overloaded. Safe
    to retry with exponential backoff from the host."""

    error_code = "EFE_TRANSIENT"
    suggested_http_status = 503
    retryable = True


class EfeFacadeTimeoutError(EfeFacadeTransientError):
    """The classify call exceeded its deadline (server-side swarm
    timeout or client-side RPC deadline)."""

    error_code = "EFE_TIMEOUT"
    suggested_http_status = 504
    retryable = True


# ---------------------------------------------------------------------------
#  Adapters
# ---------------------------------------------------------------------------


def from_cli_error(exc: BaseException) -> EfeFacadeError:
    """Translate a CLI-level exception into a façade-level one.

    The Lettre-A CLI and the Lettre-B REPL surface :class:`CliError`
    and its subclasses. The host doesn't know about them — we adapt.
    """
    # Local import so this module stays importable without torch/grpcio.
    from ..cli.errors import (
        CatalogInvalidError,
        CatalogNotFoundError,
        CliError,
        CliTimeoutError,
        NetworkError,
        ServerError,
        UsageError,
    )

    if isinstance(exc, CliTimeoutError):
        return EfeFacadeTimeoutError(
            f"classify timed out: {exc}",
            hint="raise classify_budget_ms or check the server",
            cause=exc,
        )
    if isinstance(exc, NetworkError):
        return EfeFacadeTransientError(
            f"efe-server unreachable: {exc}",
            hint="is efe-server running at the configured endpoint?",
            cause=exc,
        )
    if isinstance(exc, ServerError):
        return EfeFacadeTransientError(
            f"efe-server internal error: {exc}",
            hint="inspect efe-server logs; retry after backoff",
            cause=exc,
        )
    if isinstance(exc, (CatalogNotFoundError, CatalogInvalidError)):
        return EfeFacadeCatalogError(str(exc), cause=exc)
    if isinstance(exc, UsageError):
        return EfeFacadeUsageError(str(exc), cause=exc)
    if isinstance(exc, CliError):
        return EfeFacadeError(str(exc), cause=exc)
    return EfeFacadeError(f"unexpected error: {type(exc).__name__}: {exc}",
                          cause=exc)


def from_bridge_error(exc: BaseException) -> EfeFacadeError:
    """Translate an efe_tool.errors.BridgeError family exception."""
    from ..errors import (
        BridgeError,
        CatalogError,
        ConfigError,
        EmbedderError,
        NetworkError as BridgeNetworkError,
    )

    if isinstance(exc, CatalogError):
        return EfeFacadeCatalogError(str(exc), cause=exc)
    if isinstance(exc, ConfigError):
        return EfeFacadeConfigError(str(exc), cause=exc)
    if isinstance(exc, EmbedderError):
        return EfeFacadeInitError(
            f"embedder failed: {exc}",
            hint="check sentence-transformers installation and the model name",
            cause=exc,
        )
    if isinstance(exc, BridgeNetworkError):
        msg = str(exc).lower()
        if "deadline" in msg or "timeout" in msg or "convergence" in msg:
            return EfeFacadeTimeoutError(str(exc), cause=exc)
        return EfeFacadeTransientError(str(exc), cause=exc)
    if isinstance(exc, BridgeError):
        return EfeFacadeError(f"bridge error: {exc}", cause=exc)
    return EfeFacadeError(f"unexpected: {type(exc).__name__}: {exc}", cause=exc)


# ---------------------------------------------------------------------------
#  Registry for tests / introspection
# ---------------------------------------------------------------------------


FACADE_EXIT_CODES: dict[str, int] = {
    cls.error_code: cls.suggested_http_status
    for cls in (
        EfeFacadeConfigError,
        EfeFacadeInitError,
        EfeFacadeClosedError,
        EfeFacadeCatalogError,
        EfeFacadeUsageError,
        EfeFacadeRejectedError,
        EfeFacadeTransientError,
        EfeFacadeTimeoutError,
    )
}


__all__ = [
    "EfeFacadeError",
    "EfeFacadeConfigError",
    "EfeFacadeInitError",
    "EfeFacadeClosedError",
    "EfeFacadeCatalogError",
    "EfeFacadeUsageError",
    "EfeFacadeRejectedError",
    "EfeFacadeTransientError",
    "EfeFacadeTimeoutError",
    "from_cli_error",
    "from_bridge_error",
    "FACADE_EXIT_CODES",
]
