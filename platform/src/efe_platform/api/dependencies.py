"""FastAPI dependency-injection surface.

A :class:`PlatformContext` bundles the runtime pieces (bot, swarm
client, audit log, metrics). It is instantiated once in the app
lifespan and then served to every request handler via
:func:`get_context`.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import TYPE_CHECKING

from efe_platform.bot.agent import Bot
from efe_platform.catalog.models import Catalog
from efe_platform.config import PlatformConfig
from efe_platform.swarm.client import SwarmClient
from efe_platform.swarm.cone import ConeRegistryClient
from efe_platform.telemetry.audit import AuditLog
from efe_platform.telemetry.metrics import MetricsRegistry
from efe_platform.telemetry.tracing import TracingService

if TYPE_CHECKING:  # pragma: no cover
    pass


@dataclass
class PlatformContext:
    """Singleton wiring the whole platform."""

    config: PlatformConfig
    bot: Bot
    swarm: SwarmClient
    cones: ConeRegistryClient | None
    audit: AuditLog | None
    metrics: MetricsRegistry = field(default_factory=MetricsRegistry)
    tracing: TracingService | None = None
    catalog: Catalog | None = None


_CTX: PlatformContext | None = None


def set_context(ctx: PlatformContext) -> None:
    """Install the process-wide context (called from the lifespan hook)."""
    global _CTX
    _CTX = ctx


def clear_context() -> None:
    """Tear down the process-wide context."""
    global _CTX
    _CTX = None


def get_context() -> PlatformContext:
    """FastAPI dependency — raises :class:`RuntimeError` if the app
    has not been set up yet."""
    if _CTX is None:
        raise RuntimeError(
            "PlatformContext not initialized — did you call set_context()?"
        )
    return _CTX


__all__ = [
    "PlatformContext",
    "clear_context",
    "get_context",
    "set_context",
]
