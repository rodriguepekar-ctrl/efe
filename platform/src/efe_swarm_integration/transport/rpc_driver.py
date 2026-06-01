"""RPC driver Protocol — decouples the SwarmClient from grpcio.

Two implementations are provided elsewhere :

* ``GrpcRpcDriver`` (transport/grpc_driver.py) — real gRPC channel.
* ``FakeSwarmServer`` (testing fixture) — in-process stub.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, AsyncIterator, Protocol

__all__ = ["RpcDriver", "SwarmRpcStub", "RpcResponse"]


@dataclass(frozen=True, slots=True)
class RpcResponse:
    """Generic gRPC-echoed response — dict payload + metadata."""

    payload: dict[str, Any]
    metadata: tuple[tuple[str, str], ...] = ()
    status: str = "OK"


class RpcDriver(Protocol):
    """Minimal driver Protocol. Consumers do not know whether the
    underlying transport is real gRPC or a fake."""

    async def unary(
        self,
        *,
        method: str,
        payload: dict[str, Any],
        metadata: tuple[tuple[str, str], ...] = (),
        deadline_s: float | None = None,
    ) -> RpcResponse: ...  # pragma: no cover

    async def server_streaming(
        self,
        *,
        method: str,
        payload: dict[str, Any],
        metadata: tuple[tuple[str, str], ...] = (),
        deadline_s: float | None = None,
    ) -> AsyncIterator[RpcResponse]: ...  # pragma: no cover


class SwarmRpcStub(Protocol):
    """Alias reserved for symmetry ; not used directly."""

    ...
