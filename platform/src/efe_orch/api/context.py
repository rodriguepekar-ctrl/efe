"""Dependency-injection bundle for the FastAPI layer.

One :class:`OrchContext` per service process. It carries:

* the :class:`OrchSettings` (frozen),
* the async sessionmaker,
* the K8s client (fake or real),
* optional overrides for the clock + lock backend (tests).

FastAPI's ``Depends`` picks up the current context via a ContextVar
(see :func:`get_context`) so every handler sees the same objects.
"""

from __future__ import annotations

from contextvars import ContextVar
from dataclasses import dataclass

from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from efe_orch.clock import Clock, SystemClock
from efe_orch.config import OrchSettings
from efe_orch.dispatcher import KubernetesClient, build_client
from efe_orch.state_machine.locks import InMemoryLockBackend, LockBackend


@dataclass(frozen=True, slots=True)
class OrchContext:
    settings: OrchSettings
    sessionmaker: async_sessionmaker[AsyncSession]
    k8s_client: KubernetesClient
    clock: Clock
    lock_backend: LockBackend

    @classmethod
    def build(
        cls,
        settings: OrchSettings,
        sessionmaker: async_sessionmaker[AsyncSession],
        *,
        k8s_client: KubernetesClient | None = None,
        clock: Clock | None = None,
        lock_backend: LockBackend | None = None,
    ) -> OrchContext:
        return cls(
            settings=settings,
            sessionmaker=sessionmaker,
            k8s_client=k8s_client or build_client(settings.k8s),
            clock=clock or SystemClock(),
            lock_backend=lock_backend or InMemoryLockBackend(),
        )


_CTX: ContextVar[OrchContext | None] = ContextVar(
    "efe_orch_context", default=None,
)


def set_context(ctx: OrchContext) -> None:
    _CTX.set(ctx)


def clear_context() -> None:
    _CTX.set(None)


def get_context() -> OrchContext:
    ctx = _CTX.get()
    if ctx is None:
        raise RuntimeError(
            "OrchContext not set — call set_context(...) on process bootstrap "
            "or supply one via the ``context`` arg of build_app().",
        )
    return ctx


__all__ = ["OrchContext", "clear_context", "get_context", "set_context"]
