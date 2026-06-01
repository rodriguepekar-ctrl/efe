"""BackendSelector — pick a backend from a FallbackChain.

Implements I4 (deterministic fallback) : the chain is read once per
adapter construction; two identical requests with all backends green
always pick the same primary.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from efe_inference.backends.protocol import BackendAdapter
from efe_inference.backends.registry import BackendRegistry
from efe_inference.errors import (
    AllBackendsDown,
    BackendNotRegistered,
    CircuitOpen,
)
from efe_inference.resilience.circuit import CircuitBreakerRegistry


@dataclass(frozen=True, slots=True)
class FallbackEntry:
    backend_name: str
    timeout_s: float = 30.0
    weight_pct: int = 100


@dataclass(frozen=True, slots=True)
class FallbackChain:
    entries: tuple[FallbackEntry, ...]

    def __post_init__(self) -> None:
        if not self.entries:
            raise ValueError("FallbackChain must have at least one entry")


@dataclass
class BackendSelector:
    registry: BackendRegistry
    circuit: CircuitBreakerRegistry
    default_chain: FallbackChain
    per_tenant_chains: dict[tuple[str, str | None], FallbackChain] = field(
        default_factory=dict,
    )

    def resolve_chain(
        self, *, tenant_id: str, task_id: str | None,
    ) -> FallbackChain:
        """Return the FallbackChain for ``(tenant, task)``.

        Lookup order :

          1. (tenant, task)  if provided and registered.
          2. (tenant, None)  tenant-level default.
          3. default_chain   global.
        """
        if task_id is not None:
            chain = self.per_tenant_chains.get((tenant_id, task_id))
            if chain is not None:
                return chain
        chain = self.per_tenant_chains.get((tenant_id, None))
        if chain is not None:
            return chain
        return self.default_chain

    async def select(
        self, *, tenant_id: str, task_id: str | None,
    ) -> list[tuple[FallbackEntry, BackendAdapter]]:
        """Return (entry, adapter) pairs to try in order, skipping open
        circuits.

        Raises :class:`AllBackendsDown` if every entry is filtered out
        (circuit open or adapter missing from the registry).
        """
        chain = self.resolve_chain(tenant_id=tenant_id, task_id=task_id)
        result: list[tuple[FallbackEntry, BackendAdapter]] = []
        for entry in chain.entries:
            try:
                adapter = self.registry.get(entry.backend_name)
            except BackendNotRegistered:
                continue
            breaker = self.circuit.get(tenant_id, entry.backend_name)
            try:
                await breaker.allow()
            except CircuitOpen:
                continue
            result.append((entry, adapter))
        if not result:
            raise AllBackendsDown(
                f"no backend available for ({tenant_id!r}, {task_id!r})",
            )
        return result


__all__ = ["BackendSelector", "FallbackChain", "FallbackEntry"]
