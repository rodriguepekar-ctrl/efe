"""Test doubles for M12."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, AsyncIterator

from efe_swarm_integration.transport.rpc_driver import RpcDriver, RpcResponse


@dataclass(slots=True)
class FakeRpcDriver:
    """In-process RPC driver — the caller pre-registers per-method
    responses, the driver echoes them."""

    unary_responses: dict[str, RpcResponse] = field(default_factory=dict)
    streaming_responses: dict[str, list[dict[str, Any]]] = field(default_factory=dict)
    unary_error: Exception | None = None
    call_log: list[tuple[str, tuple[tuple[str, str], ...]]] = field(default_factory=list)

    async def unary(
        self,
        *,
        method: str,
        payload: dict[str, Any],
        metadata: tuple[tuple[str, str], ...] = (),
        deadline_s: float | None = None,
    ) -> RpcResponse:
        self.call_log.append((method, tuple(metadata)))
        if self.unary_error is not None:
            raise self.unary_error
        return self.unary_responses.get(
            method, RpcResponse(payload={}, metadata=metadata)
        )

    async def server_streaming(
        self,
        *,
        method: str,
        payload: dict[str, Any],
        metadata: tuple[tuple[str, str], ...] = (),
        deadline_s: float | None = None,
    ) -> AsyncIterator[RpcResponse]:
        chunks = self.streaming_responses.get(method, [])

        async def iterator() -> AsyncIterator[RpcResponse]:
            for c in chunks:
                yield RpcResponse(payload=c)

        return iterator()


@dataclass(slots=True)
class FakeEmbedder:
    dim: int = 4
    fill_value: float = 0.25

    async def embed(self, *texts: str) -> list[tuple[float, ...]]:
        return [tuple([self.fill_value] * self.dim) for _ in texts]


@dataclass(slots=True)
class FakePdp:
    allow: bool = True

    async def authorise(
        self, *, actor_id: str, tenant_id: str, action: str, resource_kind: str
    ) -> bool:
        return self.allow
