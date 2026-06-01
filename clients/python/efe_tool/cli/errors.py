"""Exit-code-mapped exception taxonomy for the ``efe-admin`` CLI.

See ``LETTER_A_ARCHITECTURE.md`` §10 for the full table and rationale.

The design rule is simple: **every fail mode is expressed as a
typed exception** that declares its own exit code. The top-level
dispatcher in :mod:`efe_tool.cli.main` never invents exit codes;
it just catches exceptions and returns ``exc.exit_code``.

Exit codes in use (canonical):

    0   OK                  — classification succeeded
    2   USAGE               — bad flags or arguments
    3   CATALOG_NOT_FOUND   — catalog path does not exist
    4   CATALOG_INVALID     — YAML malformed or schema violation
    5   SERVER_UNREACHABLE  — gRPC UNAVAILABLE, connection refused
    6   SERVER_ERROR        — server-side INTERNAL, RESOURCE_EXHAUSTED
    7   TIMEOUT             — DEADLINE_EXCEEDED (transport or budget)
    8   REJECTED            — classification ``reject=True`` + ``--strict``
    9   LOW_CONFIDENCE      — confidence < high + ``--strict``
    126 BUG                 — unhandled exception (developer bug)
    130 INTERRUPTED         — SIGINT (Ctrl-C)

Design notes
------------

* ``UsageError`` is the only exception that is **also** raised by
  argparse (through ``parser.error``). argparse by default calls
  ``sys.exit(2)`` on a parse failure, which matches our code.

* The ``from_bridge_error`` helpers adapt the lower-level
  ``efe_tool.errors.BridgeError`` family to this CLI taxonomy without
  leaking gRPC vocabulary to the end user.
"""

from __future__ import annotations

from typing import Any, Final, Mapping, Optional


# ---------------------------------------------------------------------------
# Root
# ---------------------------------------------------------------------------


class CliError(Exception):
    """Base class for any CLI-level failure.

    Subclasses override :attr:`exit_code` and :attr:`error_code`.
    ``CliError`` itself is treated as a developer-facing bug (exit
    126) — production code should always raise a subclass.
    """

    #: POSIX exit code returned by ``main()`` when this exception bubbles.
    exit_code: int = 126

    #: Machine-readable short code, embedded in JSON error docs.
    error_code: str = "BUG"

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
        self.context: Mapping[str, Any] = dict(context or {})
        # ``cause`` is stored explicitly because we sometimes want to
        # expose a truncated representation without relying on Python's
        # traceback chain.
        self._cause = cause

    # ------------------------------------------------------------------
    # Representation helpers used by cli.output.format_error
    # ------------------------------------------------------------------

    def to_payload(self) -> dict[str, Any]:
        """Render this error as a JSON-serialisable dict.

        Caller is responsible for wrapping it under ``{"error": ...}``.
        """
        payload: dict[str, Any] = {
            "code": self.error_code,
            "exit_code": self.exit_code,
            "message": self.message,
        }
        if self.hint:
            payload["hint"] = self.hint
        if self.context:
            payload["context"] = dict(self.context)
        if self._cause is not None:
            payload["cause"] = {
                "type": type(self._cause).__name__,
                "detail": str(self._cause)[:500],
            }
        return payload

    def __str__(self) -> str:  # pragma: no cover — trivial
        return self.message


# ---------------------------------------------------------------------------
# Usage — exit 2
# ---------------------------------------------------------------------------


class UsageError(CliError):
    """The user passed invalid flags or arguments.

    Raised by the parser, by post-resolution validation, or by
    subcommands when they detect logically inconsistent inputs
    (e.g. ``--text`` *and* ``--stdin``).
    """

    exit_code: int = 2
    error_code: str = "USAGE"


# ---------------------------------------------------------------------------
# Catalog — exits 3 / 4
# ---------------------------------------------------------------------------


class CatalogNotFoundError(CliError):
    """The catalog path does not exist on disk."""

    exit_code: int = 3
    error_code: str = "CATALOG_NOT_FOUND"


class CatalogInvalidError(CliError):
    """The catalog file exists but cannot be loaded or validated."""

    exit_code: int = 4
    error_code: str = "CATALOG_INVALID"


# ---------------------------------------------------------------------------
# Network / server — exits 5 / 6 / 7
# ---------------------------------------------------------------------------


class NetworkError(CliError):
    """The gRPC endpoint is unreachable (UNAVAILABLE, connection refused)."""

    exit_code: int = 5
    error_code: str = "SERVER_UNREACHABLE"


class ServerError(CliError):
    """The server responded with an error status (INTERNAL, FAILED_PRECONDITION, …)."""

    exit_code: int = 6
    error_code: str = "SERVER_ERROR"


class CliTimeoutError(CliError):
    """The RPC exceeded its deadline (``DEADLINE_EXCEEDED``).

    Renamed from the stdlib ``TimeoutError`` to avoid clobbering
    builtins; subclasses can still catch ``TimeoutError`` if desired
    (we intentionally do *not* inherit from it to keep the taxonomy
    exit-code-driven).
    """

    exit_code: int = 7
    error_code: str = "TIMEOUT"


# ---------------------------------------------------------------------------
# Business — exits 8 / 9
# ---------------------------------------------------------------------------


class RejectedError(CliError):
    """The classification was rejected and ``--strict`` was set."""

    exit_code: int = 8
    error_code: str = "REJECTED"


class LowConfidenceError(CliError):
    """The confidence was below ``confidence_high`` and ``--strict`` was set."""

    exit_code: int = 9
    error_code: str = "LOW_CONFIDENCE"


# ---------------------------------------------------------------------------
# Signals
# ---------------------------------------------------------------------------


class InterruptedError(CliError):  # noqa: N818 — deliberately shadows builtin
    """SIGINT was delivered to the process. Rendered as exit 130."""

    exit_code: int = 130
    error_code: str = "INTERRUPTED"


# ---------------------------------------------------------------------------
# Registry — used by tests to make exhaustiveness claims
# ---------------------------------------------------------------------------


EXIT_CODE_TABLE: Final[dict[int, str]] = {
    0: "OK",
    2: UsageError.error_code,
    3: CatalogNotFoundError.error_code,
    4: CatalogInvalidError.error_code,
    5: NetworkError.error_code,
    6: ServerError.error_code,
    7: CliTimeoutError.error_code,
    8: RejectedError.error_code,
    9: LowConfidenceError.error_code,
    126: CliError.error_code,
    130: InterruptedError.error_code,
}


# ---------------------------------------------------------------------------
# Adapters: efe_tool.errors.BridgeError → CliError
# ---------------------------------------------------------------------------


def from_bridge_error(exc: BaseException) -> CliError:
    """Translate an :class:`efe_tool.errors.BridgeError` into a CliError.

    The lower-level bridge hierarchy is:

        BridgeError
          ├── ConfigError
          ├── CatalogError
          ├── EmbedderError
          ├── NetworkError            (!)  same name, different class
          └── (unspecialised)

    We flatten them onto the CLI taxonomy. The ``cause`` chain is
    preserved so ``--log-level debug`` can still surface the original
    stack.
    """
    from efe_tool.errors import (  # local to avoid a cycle at import time
        BridgeError,
        CatalogError,
        ConfigError,
        EmbedderError,
        NetworkError as BridgeNetworkError,
    )

    if isinstance(exc, CatalogError):
        return CatalogInvalidError(
            f"catalog invalid: {exc}",
            hint="run `efe-admin validate <path>` for details",
            cause=exc,
        )
    if isinstance(exc, ConfigError):
        return UsageError(f"configuration error: {exc}", cause=exc)
    if isinstance(exc, EmbedderError):
        return CatalogInvalidError(
            f"embedder failed: {exc}",
            hint="the embedder model may be missing or dimensions may mismatch",
            cause=exc,
        )
    if isinstance(exc, BridgeNetworkError):
        # Further discriminate based on message — BridgeNetworkError
        # already folds both timeout and unreachable. See client.py
        # ``_map_grpc_error`` for upstream logic.
        msg = str(exc).lower()
        if "deadline" in msg or "timeout" in msg:
            return CliTimeoutError(
                f"gRPC deadline exceeded: {exc}",
                hint=(
                    "raise --budget-ms and --timeout-s if the server is "
                    "healthy but slow"
                ),
                cause=exc,
            )
        return NetworkError(
            f"server unreachable: {exc}",
            hint=(
                "is `efe-server` running at the configured endpoint? "
                "check with `efe-admin healthcheck --endpoint …`."
            ),
            cause=exc,
        )
    if isinstance(exc, BridgeError):
        # Everything else: likely RESOURCE_EXHAUSTED, FAILED_PRECONDITION,
        # INTERNAL, or a convergence timeout surfaced as BridgeError
        # by the client. Default to ServerError but detect convergence.
        msg = str(exc).lower()
        if "did not converge" in msg or "resource budget exhausted" in msg:
            return CliTimeoutError(
                f"swarm timeout: {exc}",
                hint="raise --budget-ms or lower the catalog difficulty",
                cause=exc,
            )
        return ServerError(f"server error: {exc}", cause=exc)
    # Not a bridge error — let the generic handler upgrade to BUG.
    return CliError(f"unexpected error: {exc}", cause=exc)
