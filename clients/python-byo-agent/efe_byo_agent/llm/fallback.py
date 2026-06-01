"""FallbackChain — multi-provider failover (M20).

Implements the LLMProvider Protocol so it can be plugged in
transparently anywhere a single provider is expected.

Failure semantics :
  - On `ProviderError` from a provider, record_failure on its breaker
    and try the next.
  - On `asyncio.TimeoutError`, treat as a provider failure (same as
    above).
  - If ALL providers fail (or have OPEN breakers), raise
    `AllProvidersFailedError` with the collected sub-errors.

Streaming : delegated to the first available provider — fallback within
a stream is not supported in M20 (would require buffering).
"""
from __future__ import annotations

import asyncio
import logging
import time
from typing import Any, AsyncIterator

from ..exceptions import AllProvidersFailedError, ProviderError
from ..types import CostModel, LLMResponse, ProviderHealth
from .base import LLMProvider
from .circuit_breaker import CircuitBreaker

log = logging.getLogger("efe_byo_agent.fallback")


class FallbackChain:
    """Chain of LLMProviders with per-provider circuit breakers.

    Order of `complete()` attempts :
      1. primary
      2. fallbacks[0]
      3. fallbacks[1]
      ...

    Attempts are SKIPPED when their breaker is OPEN.
    """

    def __init__(
        self,
        *,
        primary: LLMProvider,
        fallbacks: list[LLMProvider] | None = None,
        per_provider_timeout_s: float = 30.0,
        circuit_breaker_threshold: int = 5,
        circuit_breaker_open_duration_s: float = 60.0,
    ) -> None:
        if primary is None:
            raise ValueError("FallbackChain requires a primary provider")
        # Defense-in-depth : reject None entries in fallbacks too —
        # silent NoneType.attribute errors at call time would be opaque.
        if fallbacks:
            for i, f in enumerate(fallbacks):
                if f is None:
                    raise ValueError(
                        f"FallbackChain fallbacks[{i}] is None ; "
                        "all fallbacks must be LLMProvider instances"
                    )
        self._chain: list[LLMProvider] = [primary, *(fallbacks or [])]
        self._timeout = per_provider_timeout_s
        self._breakers: dict[str, CircuitBreaker] = {
            p.name: CircuitBreaker(
                threshold=circuit_breaker_threshold,
                open_duration_s=circuit_breaker_open_duration_s,
            )
            for p in self._chain
        }

    # ─── Protocol surface ─────────────────────────────────────────

    @property
    def name(self) -> str:
        return f"fallback({','.join(p.name for p in self._chain)})"

    @property
    def supports_tools(self) -> bool:
        return all(p.supports_tools for p in self._chain)

    @property
    def supports_streaming(self) -> bool:
        return all(p.supports_streaming for p in self._chain)

    @property
    def max_context_tokens(self) -> int:
        return min(p.max_context_tokens for p in self._chain)

    @property
    def cost_model(self) -> CostModel:
        return self._chain[0].cost_model

    # ─── Behaviour ────────────────────────────────────────────────

    async def complete(self, **kwargs: Any) -> LLMResponse:
        errors: list[Exception] = []
        for provider in self._chain:
            breaker = self._breakers[provider.name]
            if not breaker.is_closed():
                log.debug("circuit_open_skip", extra={"provider": provider.name})
                continue
            try:
                resp = await asyncio.wait_for(
                    provider.complete(**kwargs),
                    timeout=self._timeout,
                )
                breaker.record_success()
                return resp
            except (ProviderError, asyncio.TimeoutError) as e:
                breaker.record_failure()
                errors.append(e)
                log.warning(
                    "provider_failed",
                    extra={"provider": provider.name, "error": str(e)},
                )
        raise AllProvidersFailedError(
            f"All {len(self._chain)} providers failed",
            errors=errors,
        )

    async def stream(self, **kwargs: Any) -> AsyncIterator[LLMResponse]:
        # M20 : no mid-stream fallback. Pick the first provider whose
        # circuit is closed and stream from it.
        for provider in self._chain:
            breaker = self._breakers[provider.name]
            if breaker.is_closed():
                try:
                    async for chunk in provider.stream(**kwargs):
                        yield chunk
                    breaker.record_success()
                    return
                except (ProviderError, asyncio.TimeoutError) as e:
                    breaker.record_failure()
                    log.warning(
                        "stream_provider_failed",
                        extra={"provider": provider.name, "error": str(e)},
                    )
        raise AllProvidersFailedError("No providers available for streaming")

    async def health(self) -> ProviderHealth:
        results: list[ProviderHealth] = []
        for p in self._chain:
            try:
                results.append(await asyncio.wait_for(p.health(), timeout=5.0))
            except Exception:  # noqa: BLE001
                continue
        any_avail = any(r.available for r in results)
        return ProviderHealth(
            available=any_avail,
            last_check_ts=time.time(),
        )

    # ─── Diagnostics ──────────────────────────────────────────────

    def breaker_states(self) -> dict[str, str]:
        """Inspect circuit breaker states (for tests / dashboards)."""
        return {name: br.state.value for name, br in self._breakers.items()}


__all__ = ["FallbackChain"]
