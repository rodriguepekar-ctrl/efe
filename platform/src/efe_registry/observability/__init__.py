"""Observability — metrics, audit bridge, logging."""

from __future__ import annotations

from efe_registry.observability.audit import (
    AuditChannel,
    NullAuditChannel,
    RecordingAuditChannel,
)
from efe_registry.observability.logging import get_logger
from efe_registry.observability.metrics import (
    Counter,
    Gauge,
    Histogram,
    MetricsCatalog,
    get_catalog,
)

__all__ = [
    "AuditChannel",
    "Counter",
    "Gauge",
    "Histogram",
    "MetricsCatalog",
    "NullAuditChannel",
    "RecordingAuditChannel",
    "get_catalog",
    "get_logger",
]
