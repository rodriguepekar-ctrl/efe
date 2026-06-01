"""Process-wide channel pool — enforces P2 (single client, single pool)."""

from __future__ import annotations

from dataclasses import dataclass

from efe_swarm_integration.config import TransportConfig
from efe_swarm_integration.transport.channel import SwarmChannelFactory
from efe_swarm_integration.transport.credentials import CredentialsLoader

__all__ = ["ChannelPool"]


@dataclass(slots=True)
class ChannelPool:
    """Thin wrapper around :class:`SwarmChannelFactory`.

    In the MVP we only need one target ; multi-target sharding will
    extend this class.
    """

    transport: TransportConfig
    credentials: CredentialsLoader
    _factory: SwarmChannelFactory | None = None

    def get_factory(self) -> SwarmChannelFactory:
        if self._factory is None:
            self._factory = SwarmChannelFactory(self.transport, self.credentials)
        return self._factory

    def channel(self) -> object:
        return self.get_factory().channel()

    async def close(self) -> None:
        if self._factory is not None:
            await self._factory.close_all()
            self._factory = None
