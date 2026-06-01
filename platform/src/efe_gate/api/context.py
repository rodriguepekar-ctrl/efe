"""Dependency-injection bundle for the FastAPI layer."""

from __future__ import annotations

from contextvars import ContextVar
from dataclasses import dataclass

from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from efe_gate.clock import Clock, SystemClock
from efe_gate.config import GateSettings
from efe_gate.runner import EvalScorer, FakeScorer


@dataclass(frozen=True, slots=True)
class GateContext:
    settings: GateSettings
    sessionmaker: async_sessionmaker[AsyncSession]
    scorer: EvalScorer
    clock: Clock

    @classmethod
    def build(
        cls,
        settings: GateSettings,
        sessionmaker: async_sessionmaker[AsyncSession],
        *,
        scorer: EvalScorer | None = None,
        clock: Clock | None = None,
    ) -> GateContext:
        return cls(
            settings=settings,
            sessionmaker=sessionmaker,
            scorer=scorer or FakeScorer(),
            clock=clock or SystemClock(),
        )


_CTX: ContextVar[GateContext | None] = ContextVar(
    "efe_gate_context", default=None,
)


def set_context(ctx: GateContext) -> None:
    _CTX.set(ctx)


def clear_context() -> None:
    _CTX.set(None)


def get_context() -> GateContext:
    ctx = _CTX.get()
    if ctx is None:
        raise RuntimeError(
            "GateContext not set — call set_context(...) on bootstrap.",
        )
    return ctx


__all__ = [
    "GateContext",
    "clear_context",
    "get_context",
    "set_context",
]
