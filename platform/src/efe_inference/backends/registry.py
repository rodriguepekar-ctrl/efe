"""BackendRegistry — register adapters + discover by name + health-check."""

from __future__ import annotations

from dataclasses import dataclass, field

from efe_inference.backends.protocol import BackendAdapter, HealthStatus
from efe_inference.errors import BackendNotRegistered


@dataclass
class BackendRegistry:
    _by_name: dict[str, BackendAdapter] = field(default_factory=dict)

    def register(self, adapter: BackendAdapter) -> None:
        self._by_name[adapter.name] = adapter

    def unregister(self, name: str) -> None:
        self._by_name.pop(name, None)

    def get(self, name: str) -> BackendAdapter:
        if name not in self._by_name:
            raise BackendNotRegistered(
                f"backend {name!r} not registered "
                f"(known: {sorted(self._by_name)})",
            )
        return self._by_name[name]

    def names(self) -> tuple[str, ...]:
        return tuple(sorted(self._by_name))

    async def health_all(self) -> dict[str, HealthStatus]:
        out: dict[str, HealthStatus] = {}
        for name, adapter in self._by_name.items():
            try:
                out[name] = await adapter.health()
            except Exception:
                out[name] = HealthStatus.UNHEALTHY
        return out

    async def close_all(self) -> None:
        for adapter in self._by_name.values():
            try:
                await adapter.close()
            except Exception:
                pass


__all__ = ["BackendRegistry"]
