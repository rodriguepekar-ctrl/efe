"""CostCalculator — per-backend $/1K pricing."""

from __future__ import annotations

from dataclasses import dataclass
from decimal import Decimal


@dataclass(frozen=True, slots=True)
class CostRate:
    backend_name: str
    prompt_usd_per_1k: Decimal
    completion_usd_per_1k: Decimal

    def total_usd(
        self, prompt_tokens: int, completion_tokens: int,
    ) -> Decimal:
        if prompt_tokens < 0 or completion_tokens < 0:
            raise ValueError("token counts must be non-negative")
        p = self.prompt_usd_per_1k * Decimal(prompt_tokens) / Decimal(1000)
        c = self.completion_usd_per_1k * Decimal(completion_tokens) / Decimal(1000)
        return p + c


class CostCalculator:
    """In-memory rate book keyed by backend name."""

    def __init__(self, rates: tuple[CostRate, ...] = ()) -> None:
        self._by_name: dict[str, CostRate] = {
            r.backend_name: r for r in rates
        }

    def register(self, rate: CostRate) -> None:
        self._by_name[rate.backend_name] = rate

    def calculate(
        self,
        backend_name: str,
        *,
        prompt_tokens: int,
        completion_tokens: int,
    ) -> Decimal:
        rate = self._by_name.get(backend_name)
        if rate is None:
            return Decimal(0)
        return rate.total_usd(prompt_tokens, completion_tokens)


__all__ = ["CostCalculator", "CostRate"]
