"""EFE Platform — unified bot + swarm orchestration layer.

This package composes the low-level ``efe-tool`` SDK and the Rust
``efe-server`` into a cohesive platform:

- :mod:`efe_platform.bot` — LLM orchestrator (Plan / Execute / Verify).
- :mod:`efe_platform.swarm` — async client for the Rust core.
- :mod:`efe_platform.catalog` — tasks, anchors, cone references.
- :mod:`efe_platform.telemetry` — tracing, metrics, audit JSONL.
- :mod:`efe_platform.api` — HTTP surface (FastAPI).

The canonical entry point for user code is :class:`efe_platform.Bot`.
"""

from __future__ import annotations

from efe_platform.bot import Bot, BotConfig, BotReply, TraceEvent
from efe_platform.errors import (
    BudgetExceeded,
    EfePlatformError,
    SwarmError,
    ToolInvocationError,
)
from efe_platform.swarm import (
    SwarmClient,
    SwarmEndpoint,
    SwarmOutcome,
    SwarmSpec,
)
from efe_platform.version import VERSION, __version__

__all__ = [
    "VERSION",
    "__version__",
    # Bot
    "Bot",
    "BotConfig",
    "BotReply",
    "TraceEvent",
    # Swarm
    "SwarmClient",
    "SwarmEndpoint",
    "SwarmOutcome",
    "SwarmSpec",
    # Errors
    "EfePlatformError",
    "BudgetExceeded",
    "SwarmError",
    "ToolInvocationError",
]
