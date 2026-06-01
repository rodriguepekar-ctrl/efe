"""Protocol for serving adapters — vLLM in prod, Mock in tests."""

from __future__ import annotations

from collections.abc import AsyncIterator
from dataclasses import dataclass, field
from typing import Protocol


@dataclass(frozen=True, slots=True)
class CompletionRequest:
    request_id: str
    prompt: str
    max_tokens: int = 256
    temperature: float = 0.7
    top_p: float = 1.0
    top_k: int = -1
    stop: tuple[str, ...] = field(default_factory=tuple)
    stream: bool = False
    lora_id: str | None = None


@dataclass(frozen=True, slots=True)
class CompletionResponse:
    request_id: str
    text: str
    prompt_tokens: int
    completion_tokens: int
    finish_reason: str
    artifact_id: str
    content_hash_head: str
    latency_ms: float


class ServingAdapter(Protocol):
    artifact_id: str

    async def load(self) -> None:  # pragma: no cover
        ...

    async def ready(self) -> bool:  # pragma: no cover
        ...

    async def generate(
        self, req: CompletionRequest,
    ) -> CompletionResponse:  # pragma: no cover
        ...

    async def stream(
        self, req: CompletionRequest,
    ) -> AsyncIterator[str]:  # pragma: no cover
        ...

    async def register_lora(
        self, lora_id: str, local_path: str,
    ) -> None:  # pragma: no cover
        ...

    async def close(self) -> None:  # pragma: no cover
        ...


__all__ = [
    "CompletionRequest",
    "CompletionResponse",
    "ServingAdapter",
]
