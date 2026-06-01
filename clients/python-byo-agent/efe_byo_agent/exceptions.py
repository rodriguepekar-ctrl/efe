"""Exception hierarchy for efe-byo-agent (M20).

Hierarchy :

  EfeAgentError                              (base)
    ├── ProviderError                        (base for all provider failures)
    │     ├── ProviderTimeoutError
    │     ├── ProviderAuthError
    │     ├── ProviderRateLimitError
    │     └── ProviderUnavailableError
    ├── AllProvidersFailedError              (FallbackChain exhausted)
    ├── MCPClientError                       (mcp-server RPC failure)
    │     ├── MCPRpcError
    │     ├── MCPTransportError
    │     └── MCPSamplingError
    └── ValidationFailedError                (LLM output validation failed)
"""
from __future__ import annotations


class EfeAgentError(Exception):
    """Base for all efe-byo-agent errors."""


# ─── Provider hierarchy ──────────────────────────────────────────────


class ProviderError(EfeAgentError):
    """Base for all provider-side failures."""

    def __init__(
        self,
        message: str,
        *,
        provider_name: str | None = None,
        status_code: int | None = None,
    ) -> None:
        super().__init__(message)
        self.provider_name = provider_name
        self.status_code = status_code


class ProviderTimeoutError(ProviderError):
    """Provider did not respond within the configured timeout."""


class ProviderAuthError(ProviderError):
    """401/403 from the provider."""


class ProviderRateLimitError(ProviderError):
    """429 from the provider — short-term backoff recommended."""


class ProviderUnavailableError(ProviderError):
    """5xx from the provider — fallback to a different provider."""


# ─── Aggregate failures ─────────────────────────────────────────────


class AllProvidersFailedError(EfeAgentError):
    """FallbackChain has exhausted all providers without success."""

    def __init__(
        self,
        message: str,
        *,
        errors: list[Exception] | None = None,
    ) -> None:
        super().__init__(message)
        self.errors = errors or []


# ─── MCP-side errors ─────────────────────────────────────────────────


class MCPClientError(EfeAgentError):
    """Base for MCP-related errors."""


class MCPRpcError(MCPClientError):
    """JSON-RPC error from mcp-server (returned `error` field)."""

    def __init__(
        self,
        message: str,
        *,
        code: int | None = None,
        data: dict | None = None,
    ) -> None:
        super().__init__(message)
        self.code = code
        self.data = data or {}


class MCPTransportError(MCPClientError):
    """Transport-level failure (network, TLS, etc.)."""


class MCPSamplingError(MCPClientError):
    """The mcp-server requested sampling but the client failed to handle it."""


# ─── Output validation ──────────────────────────────────────────────


class ValidationFailedError(EfeAgentError):
    """LLM output failed structured validation after retries."""

    def __init__(
        self,
        message: str,
        *,
        helper: str,
        attempts: int,
        errors: list[str] | None = None,
    ) -> None:
        super().__init__(message)
        self.helper = helper
        self.attempts = attempts
        self.errors = errors or []


__all__ = [
    "EfeAgentError",
    "ProviderError",
    "ProviderTimeoutError",
    "ProviderAuthError",
    "ProviderRateLimitError",
    "ProviderUnavailableError",
    "AllProvidersFailedError",
    "MCPClientError",
    "MCPRpcError",
    "MCPTransportError",
    "MCPSamplingError",
    "ValidationFailedError",
]
