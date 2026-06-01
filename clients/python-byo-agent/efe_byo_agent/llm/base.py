"""LLMProvider Protocol — base abstraction (M20).

Every concrete provider implements this. Behaviour contract :
  - `complete()` must be idempotent w.r.t. side effects on the local
    process (network calls obviously not idempotent on the provider).
  - `complete()` MUST NOT raise on routine HTTP failures ; instead it
    raises one of the typed exceptions in `efe_byo_agent.exceptions`
    (ProviderTimeoutError, ProviderRateLimitError, etc.) so the
    FallbackChain can decide retry vs failover.
  - `health()` is best-effort, used by the fallback chain to skip
    obviously broken providers without paying the timeout.
"""
from __future__ import annotations

from typing import Any, AsyncIterator, Protocol, runtime_checkable

from ..types import CostModel, LLMResponse, ProviderHealth


@runtime_checkable
class LLMProvider(Protocol):
    """Provider abstraction implemented by `llm/{anthropic,openai,...}.py`."""

    @property
    def name(self) -> str:
        """Stable provider identifier, e.g. `'anthropic:claude-sonnet-4-5'`."""
        ...

    @property
    def supports_tools(self) -> bool:
        """True if the provider supports tool/function calling."""
        ...

    @property
    def supports_streaming(self) -> bool:
        """True if the provider supports streaming responses."""
        ...

    @property
    def max_context_tokens(self) -> int:
        """Maximum input tokens supported (for the configured model)."""
        ...

    @property
    def cost_model(self) -> CostModel:
        """Pricing parameters for cost estimation."""
        ...

    async def complete(
        self,
        *,
        system_prompt: str | None,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]] | None = None,
        max_tokens: int = 1024,
        temperature: float = 0.0,
        seed: int | None = None,
        timeout_s: float = 30.0,
    ) -> LLMResponse:
        """Single-shot completion. Raises ProviderError subclasses on failure."""
        ...

    async def stream(
        self,
        *,
        system_prompt: str | None,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]] | None = None,
        max_tokens: int = 1024,
        temperature: float = 0.0,
        timeout_s: float = 30.0,
    ) -> AsyncIterator[LLMResponse]:
        """Streaming completion. Yields partial LLMResponse objects."""
        ...

    async def health(self) -> ProviderHealth:
        """Liveness check. Should not raise."""
        ...
