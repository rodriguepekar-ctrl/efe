"""Shared helper base — direct provider invocation pattern (M20).

Each helper :
  1. Builds the prompt (system + user) via the prompts loader.
  2. Calls `agent.llm.complete(...)` directly.
  3. Validates the response via `_validation.validate_with_retry`.
  4. Records cost in `agent.cost_tracker`.
"""
from __future__ import annotations

from typing import TYPE_CHECKING, Any

from .._validation import validate_with_retry
from ..prompts import render

if TYPE_CHECKING:  # pragma: no cover
    from ..agent import Agent


class BaseHelper:
    """Mixin used by all 5 M20 helpers."""

    name: str = ""               # mcp-server tool name (e.g. "efe_summarize_classification")
    helper_id: str = ""          # validator id (e.g. "summarize_classification")
    template_id: str = ""        # prompts loader id

    # Sampling preferences passed to the provider (consistent w/
    # mcp-server tool model_preferences).
    intelligence_priority: float = 0.7
    speed_priority: float = 0.3
    cost_priority: float = 0.0

    max_tokens: int = 1024
    temperature: float = 0.0

    def __init__(self, *, agent: "Agent") -> None:
        self._agent = agent

    async def _call_llm(
        self,
        system_prompt: str,
        user_prompt: str,
        *,
        max_tokens: int | None = None,
        temperature: float | None = None,
    ):
        """Run the underlying provider with the agent's configured chain."""
        resp = await self._agent.llm.complete(
            system_prompt=system_prompt,
            messages=[{"role": "user", "content": user_prompt}],
            max_tokens=max_tokens or self.max_tokens,
            temperature=temperature if temperature is not None else self.temperature,
            timeout_s=self._agent.timeout_s,
        )
        self._agent.cost_tracker.record(resp)
        return resp

    async def _validate(
        self,
        raw_text: str,
        system_prompt: str,
        user_prompt: str,
        *,
        max_retries: int | None = None,
        **validator_kwargs: Any,
    ) -> dict:
        async def _retry_callback(errors: list[str]) -> str:
            retry_user = (
                f"{user_prompt}\n\nValidation errors : {'; '.join(errors)}. "
                f"Produce a strictly schema-conformant JSON output."
            )
            resp = await self._agent.llm.complete(
                system_prompt=system_prompt,
                messages=[{"role": "user", "content": retry_user}],
                max_tokens=self.max_tokens,
                temperature=self.temperature,
                timeout_s=self._agent.timeout_s,
            )
            self._agent.cost_tracker.record(resp)
            return resp.text

        return await validate_with_retry(
            helper=self.helper_id,
            raw_output=raw_text,
            retry_callback=_retry_callback,
            max_retries=max_retries if max_retries is not None else self._agent.max_retries,
            **validator_kwargs,
        )

    def _render(self, **kwargs) -> tuple[str, str]:
        return render(self.template_id, **kwargs)


__all__ = ["BaseHelper"]
