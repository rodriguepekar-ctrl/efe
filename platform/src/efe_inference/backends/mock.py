"""Mock / Scripted backends — drive most tests + examples."""

from __future__ import annotations

from collections.abc import AsyncIterator, Callable
from dataclasses import dataclass, field
from typing import Any

from efe_inference.backends.protocol import (
    BackendRequest,
    BackendResponse,
    HealthStatus,
)
from efe_inference.config import BackendKind
from efe_inference.messages.response import (
    ChatChunk,
    FinishReason,
    Usage,
)


@dataclass
class MockBackend:
    """Echo-style backend — deterministic, zero I/O.

    The generated text is ``"[{name}]{prompt[::-1][:max_tokens]}"`` so
    test assertions can rely on the shape. Usage is computed from the
    character counts (a rough approximation — good enough for tests).
    """

    name: str
    kind: BackendKind = BackendKind.MOCK
    latency_ms: float = 5.0
    should_fail: bool = False
    health_status: HealthStatus = HealthStatus.HEALTHY

    async def generate(self, req: BackendRequest) -> BackendResponse:
        if self.should_fail:
            from efe_inference.errors import BackendUnavailable

            raise BackendUnavailable(
                f"mock backend {self.name!r} forced failure",
            )
        prompt = req.textual or _messages_as_text(req.messages or ())
        text = f"[{self.name}]" + prompt[::-1][: req.sampling.max_tokens]
        pt = max(1, len(prompt) // 4)
        ct = max(1, len(text) // 4)
        return BackendResponse(
            text=text,
            finish_reason=FinishReason.STOP,
            usage=Usage(
                prompt_tokens=pt, completion_tokens=ct,
                total_tokens=pt + ct,
            ),
            backend_latency_ms=self.latency_ms,
            raw={"mock": True, "name": self.name},
        )

    async def stream(
        self, req: BackendRequest,
    ) -> AsyncIterator[ChatChunk]:
        resp = await self.generate(req)
        # one token per word.
        for word in resp.text.split():
            yield ChatChunk(delta_text=word + " ")
        yield ChatChunk(
            finish_reason=resp.finish_reason, usage=resp.usage,
        )

    async def health(self) -> HealthStatus:
        return self.health_status

    async def close(self) -> None:
        return None


ScriptedResponse = Callable[[BackendRequest], BackendResponse]


@dataclass
class ScriptedBackend:
    """Backend driven by a user-provided callable — for expressive tests."""

    name: str
    generator: ScriptedResponse
    kind: BackendKind = BackendKind.MOCK
    calls: list[BackendRequest] = field(default_factory=list)

    async def generate(self, req: BackendRequest) -> BackendResponse:
        self.calls.append(req)
        return self.generator(req)

    async def stream(
        self, req: BackendRequest,
    ) -> AsyncIterator[ChatChunk]:
        resp = await self.generate(req)
        yield ChatChunk(
            delta_text=resp.text,
            finish_reason=resp.finish_reason,
            usage=resp.usage,
        )

    async def health(self) -> HealthStatus:
        return HealthStatus.HEALTHY

    async def close(self) -> None:
        return None


def _messages_as_text(messages: tuple[dict[str, Any], ...]) -> str:
    out: list[str] = []
    for m in messages:
        content = m.get("content", "")
        if isinstance(content, str):
            out.append(f"{m.get('role')}:{content}")
        elif isinstance(content, list):
            for block in content:
                if block.get("type") == "text":
                    out.append(f"{m.get('role')}:{block.get('text','')}")
    return "\n".join(out)


__all__ = ["MockBackend", "ScriptedBackend", "ScriptedResponse"]
