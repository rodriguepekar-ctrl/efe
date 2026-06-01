"""Purpose catalog — declared uses of personal data."""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, field

from efe_security.errors import PurposeNotDeclared

__all__ = ["Purpose", "PurposeCatalog", "default_purpose_catalog"]


@dataclass(frozen=True, slots=True)
class Purpose:
    name: str
    description: str
    required_scopes: frozenset[str] = field(default_factory=frozenset)


@dataclass(slots=True)
class PurposeCatalog:
    _rows: dict[str, Purpose] = field(default_factory=dict)
    _lock: asyncio.Lock = field(default_factory=asyncio.Lock, repr=False)

    async def register(self, p: Purpose) -> Purpose:
        async with self._lock:
            if p.name in self._rows:
                raise ValueError(f"purpose already registered: {p.name}")
            self._rows[p.name] = p
            return p

    async def get(self, name: str) -> Purpose:
        async with self._lock:
            p = self._rows.get(name)
        if p is None:
            raise PurposeNotDeclared(name)
        return p

    async def list(self) -> list[Purpose]:
        async with self._lock:
            return list(self._rows.values())

    async def exists(self, name: str) -> bool:
        async with self._lock:
            return name in self._rows


async def default_purpose_catalog() -> PurposeCatalog:
    c = PurposeCatalog()
    await c.register(Purpose("model_training", "Train / fine-tune a model", frozenset({"training:write"})))
    await c.register(Purpose("drift_analysis", "Measure drift", frozenset({"drift:write"})))
    await c.register(Purpose("quality_evaluation", "Evaluate quality", frozenset({"evaluation:write"})))
    await c.register(Purpose("rollout_canary", "Canary evaluation", frozenset({"rollout:write"})))
    await c.register(Purpose("fraud_detection", "Detect abuse", frozenset({"security:write"})))
    return c
