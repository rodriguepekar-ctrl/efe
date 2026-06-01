"""Mock serving adapter — deterministic, used by tests + examples."""

from __future__ import annotations

import asyncio
from collections.abc import AsyncIterator
from dataclasses import dataclass, field
from typing import Literal

from efe_registry.serving.adapter import (
    CompletionRequest,
    CompletionResponse,
)


@dataclass
class MockServingAdapter:
    """Deterministic adapter.

    The output text is a function of the prompt length and the
    artifact id so tests can assert deterministic behaviour without
    running a GPU.
    """

    artifact_id: str
    content_hash_hex: str
    latency_ms_base: float = 5.0
    error_rate: float = 0.0  # 0.0 → always succeed; 1.0 → always raise
    loaded: bool = False
    lora_registry: dict[str, str] = field(default_factory=dict)
    state: Literal["pending", "ready", "draining"] = "pending"

    async def load(self) -> None:
        await asyncio.sleep(0)  # yield to loop
        self.loaded = True
        self.state = "ready"

    async def ready(self) -> bool:
        return self.state == "ready"

    async def generate(
        self, req: CompletionRequest,
    ) -> CompletionResponse:
        if self.state != "ready":
            raise RuntimeError(f"adapter not ready (state={self.state!r})")
        text = f"[mock:{self.artifact_id[:8]}]{req.prompt[::-1][:req.max_tokens]}"
        return CompletionResponse(
            request_id=req.request_id,
            text=text,
            prompt_tokens=len(req.prompt.split()),
            completion_tokens=min(req.max_tokens, len(text)),
            finish_reason="stop",
            artifact_id=self.artifact_id,
            content_hash_head=self.content_hash_hex[:16],
            latency_ms=self.latency_ms_base,
        )

    async def stream(
        self, req: CompletionRequest,
    ) -> AsyncIterator[str]:
        resp = await self.generate(req)
        for token in resp.text.split():
            yield token + " "

    async def register_lora(self, lora_id: str, local_path: str) -> None:
        self.lora_registry[lora_id] = local_path

    async def close(self) -> None:
        self.state = "draining"
        self.loaded = False


__all__ = ["MockServingAdapter"]
