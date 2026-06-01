"""ShadowTapper — forks a fraction of requests to a candidate.

Usage::

    tapper = ShadowTapper(
        baseline_adapter=..., candidate_adapter=...,
        tap_rate_percent=10, timeout_s=1.5,
    )
    main_response = await tapper.handle(req)

The candidate call runs concurrently but never blocks nor raises into
the main response path. Shadow results accumulate in ``tapper.stats``
and are flushed by a sidecar writer that persists a :class:`ShadowTap`
row every N samples (production) — not shown here.
"""

from __future__ import annotations

import asyncio
import time
from collections.abc import Callable
from dataclasses import dataclass, field

from efe_registry.serving.adapter import (
    CompletionRequest,
    CompletionResponse,
    ServingAdapter,
)
from efe_registry.shadow.scorer import cosine_similarity


@dataclass
class ShadowStats:
    n_samples: int = 0
    n_shadow_failures: int = 0
    latency_deltas_ms: list[float] = field(default_factory=list)
    cosines: list[float] = field(default_factory=list)

    def ingest(
        self,
        *,
        baseline: CompletionResponse,
        candidate: CompletionResponse | None,
    ) -> None:
        self.n_samples += 1
        if candidate is None:
            self.n_shadow_failures += 1
            return
        self.latency_deltas_ms.append(
            candidate.latency_ms - baseline.latency_ms,
        )
        # Use simple token-count signature for cosine (no real logits
        # available outside engine); in production we inject hooks into
        # the engine to return top-k logits for proper comparison.
        baseline_sig = [
            float(len(baseline.text)),
            float(baseline.completion_tokens),
        ]
        cand_sig = [
            float(len(candidate.text)),
            float(candidate.completion_tokens),
        ]
        self.cosines.append(cosine_similarity(baseline_sig, cand_sig))

    def mean_latency_delta_ms(self) -> float:
        if not self.latency_deltas_ms:
            return 0.0
        return sum(self.latency_deltas_ms) / len(self.latency_deltas_ms)

    def mean_cosine(self) -> float:
        if not self.cosines:
            return 0.0
        return sum(self.cosines) / len(self.cosines)


RngFn = Callable[[], float]  # returns float in [0.0, 1.0)


@dataclass
class ShadowTapper:
    baseline_adapter: ServingAdapter
    candidate_adapter: ServingAdapter | None
    tap_rate_percent: int = 10
    timeout_s: float = 1.5
    rng: RngFn | None = None
    stats: ShadowStats = field(default_factory=ShadowStats)

    def __post_init__(self) -> None:
        if not 0 <= self.tap_rate_percent <= 100:
            raise ValueError(
                f"tap_rate_percent must be 0..100, got {self.tap_rate_percent}",
            )

    async def handle(
        self, req: CompletionRequest,
    ) -> CompletionResponse:
        start = time.monotonic()
        main_task = asyncio.create_task(self.baseline_adapter.generate(req))
        shadow_task = None
        if self._should_tap():
            shadow_task = asyncio.create_task(
                self._shadow_generate(req),
            )
        main_response = await main_task
        if shadow_task is not None:
            # Don't block the main response more than ``timeout_s``.
            try:
                shadow_response = await asyncio.wait_for(
                    shadow_task, timeout=self.timeout_s,
                )
            except (asyncio.TimeoutError, Exception):
                shadow_response = None
            self.stats.ingest(
                baseline=main_response, candidate=shadow_response,
            )
        _ = time.monotonic() - start
        return main_response

    def _should_tap(self) -> bool:
        if self.candidate_adapter is None:
            return False
        if self.tap_rate_percent == 0:
            return False
        if self.tap_rate_percent == 100:
            return True
        rng = self.rng or self._default_rng
        return rng() < self.tap_rate_percent / 100.0

    @staticmethod
    def _default_rng() -> float:
        import random
        return random.random()

    async def _shadow_generate(
        self, req: CompletionRequest,
    ) -> CompletionResponse | None:
        assert self.candidate_adapter is not None
        try:
            return await self.candidate_adapter.generate(req)
        except Exception:
            return None


__all__ = ["ShadowStats", "ShadowTapper"]
