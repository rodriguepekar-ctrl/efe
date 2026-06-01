"""Observability — metrics, structured logging, tracing.

Public surface:

    MetricsRegistry    — thin facade around a prometheus_client registry
                         (or a no-op fallback when the dep is missing).
    get_logger         — structlog-backed structured logger factory
    init_logging       — apply the configured log level + renderer
"""

from __future__ import annotations

from efe_orch.observability.logging import get_logger, init_logging
from efe_orch.observability.metrics import MetricsRegistry, noop_registry

__all__ = ["MetricsRegistry", "get_logger", "init_logging", "noop_registry"]
