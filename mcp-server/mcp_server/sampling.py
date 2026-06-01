"""MCP sampling client (M17 D45).

The mcp-server can ask the host LLM (the user's chosen model in their
MCP client) to generate text. This is the inverse direction of normal
tool calls : server → host-LLM → server.

Compliance posture : zero new sub-processor. The LLM used is the one
the user already chose for their MCP client, under their own account.
EFE infrastructure does not call any LLM provider.

Hard caps :
  - 5 sampling rounds per tool call (TM-M17-D-02)
  - 10 s timeout per round
  - 500 max tokens per request

Fallback : when the client doesn't advertise sampling capability or
the round-trip fails, callers can supply a heuristic alternative. The
exception `SamplingUnavailable` lets handlers branch to fallback.
"""
from __future__ import annotations

import asyncio
from dataclasses import dataclass
from typing import Any, Optional, Protocol

from .state import SamplingCache


class SamplingError(Exception):
    """Base class for sampling failures."""


class SamplingUnavailable(SamplingError):
    """Host client does not advertise sampling ; caller should fallback."""


class SamplingBudgetExceeded(SamplingError):
    """Per-tool-call budget of N sampling rounds is exhausted."""


class SamplingTimeout(SamplingError):
    """A single sampling round timed out."""


# ─────────────────────────────────────────────────────────────────────
# Session protocol — stripped-down view of what the MCP SDK exposes.
# ─────────────────────────────────────────────────────────────────────


class _MessageContent(Protocol):
    text: str


class _MessageResponse(Protocol):
    content: _MessageContent


class SamplingSession(Protocol):
    """Anything that can perform `sampling/createMessage`.

    Real implementation: the MCP SDK's session.create_message().
    Tests inject a fake.
    """

    async def create_message(
        self,
        *,
        messages: list[dict[str, Any]],
        max_tokens: int,
        model_preferences: Optional[list[dict[str, Any]]] = None,
    ) -> _MessageResponse: ...


# ─────────────────────────────────────────────────────────────────────
# Client
# ─────────────────────────────────────────────────────────────────────


@dataclass
class SamplingUsage:
    rounds_used: int = 0
    cache_hits: int = 0
    total_max_tokens: int = 0


class SamplingClient:
    """Wraps a SamplingSession with budget + timeout + cache."""

    DEFAULT_MAX_ROUNDS = 5
    DEFAULT_TIMEOUT = 10.0
    DEFAULT_MAX_TOKENS = 500

    def __init__(
        self,
        session: Optional[SamplingSession],
        *,
        cache: SamplingCache | None = None,
        max_rounds_per_call: int = DEFAULT_MAX_ROUNDS,
        timeout_seconds: float = DEFAULT_TIMEOUT,
        max_tokens_per_request: int = DEFAULT_MAX_TOKENS,
        client_supports_sampling: bool = True,
    ) -> None:
        self._session = session
        self._cache = cache or SamplingCache()
        self.max_rounds = max_rounds_per_call
        self.timeout = timeout_seconds
        self.max_tokens = max_tokens_per_request
        self.client_supports_sampling = client_supports_sampling
        self.usage = SamplingUsage()

    @property
    def available(self) -> bool:
        return self._session is not None and self.client_supports_sampling

    def reset_usage(self) -> None:
        self.usage = SamplingUsage()

    async def sample(
        self,
        *,
        tenant_id: str,
        prompt: str,
        model_hint: str | None = None,
        max_tokens: int | None = None,
    ) -> str:
        """Run one sampling round. Raises if unavailable / over-budget / timed out."""
        if not self.available:
            raise SamplingUnavailable(
                "host MCP client does not advertise sampling capability"
            )
        # Cache hit ?
        cached = self._cache.get(tenant_id=tenant_id, prompt=prompt, model_hint=model_hint)
        if cached is not None:
            self.usage.cache_hits += 1
            return cached
        # Budget check
        if self.usage.rounds_used >= self.max_rounds:
            raise SamplingBudgetExceeded(
                f"max sampling rounds ({self.max_rounds}) exceeded for this tool call"
            )
        max_t = max_tokens if max_tokens is not None else self.max_tokens
        max_t = min(max_t, self.max_tokens)

        prefs = (
            [{"hints": [{"name": model_hint}]}]
            if model_hint
            else None
        )
        try:
            assert self._session is not None
            resp = await asyncio.wait_for(
                self._session.create_message(
                    messages=[
                        {"role": "user", "content": {"type": "text", "text": prompt}}
                    ],
                    max_tokens=max_t,
                    model_preferences=prefs,
                ),
                timeout=self.timeout,
            )
        except asyncio.TimeoutError as exc:
            raise SamplingTimeout(
                f"sampling round timed out after {self.timeout}s"
            ) from exc
        except Exception as exc:  # noqa: BLE001
            raise SamplingError(f"sampling round failed : {exc!r}") from exc

        text = getattr(resp.content, "text", None)
        if text is None:
            raise SamplingError("sampling response has no text content")
        self.usage.rounds_used += 1
        self.usage.total_max_tokens += max_t
        self._cache.put(
            tenant_id=tenant_id, prompt=prompt, model_hint=model_hint, response=text
        )
        return text
