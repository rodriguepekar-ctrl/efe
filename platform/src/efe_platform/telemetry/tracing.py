"""OpenTelemetry tracing façade.

The façade provides a narrow, type-safe API the platform uses to
annotate work. It defaults to a no-op implementation when OTel is
not configured — the import never fails even if the collector is
offline.
"""

from __future__ import annotations

import contextlib
import logging
from collections.abc import Iterator
from dataclasses import dataclass, field
from typing import Any

_LOG = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
#  Façade
# ---------------------------------------------------------------------------


@dataclass
class TracingService:
    """Wrap the OTel tracer provider with a minimum-viable API."""

    service_name: str = "efe-platform"
    otel_endpoint: str | None = None
    _tracer: Any = field(default=None, init=False, repr=False)

    def __post_init__(self) -> None:
        self._tracer = self._make_tracer()

    # ------------------------------------------------------------- Spans

    @contextlib.contextmanager
    def span(self, name: str, **attrs: Any) -> Iterator[Any]:
        if self._tracer is None:
            yield _NoopSpan(attrs=dict(attrs))
            return
        with self._tracer.start_as_current_span(name) as s:
            for k, v in attrs.items():
                try:
                    s.set_attribute(k, v)
                except Exception:
                    pass
            yield s

    # ---------------------------------------------------------- Internal

    def _make_tracer(self) -> Any:
        try:
            from opentelemetry import trace
            from opentelemetry.sdk.resources import Resource
            from opentelemetry.sdk.trace import TracerProvider
            from opentelemetry.sdk.trace.export import (
                BatchSpanProcessor,
                ConsoleSpanExporter,
            )
        except ImportError:  # pragma: no cover — no otel installed
            _LOG.debug("OpenTelemetry not installed; tracing disabled.")
            return None

        resource = Resource.create({"service.name": self.service_name})
        provider = TracerProvider(resource=resource)
        try:
            if self.otel_endpoint:
                from opentelemetry.exporter.otlp.proto.grpc.trace_exporter import (
                    OTLPSpanExporter,
                )

                exporter: Any = OTLPSpanExporter(endpoint=self.otel_endpoint, insecure=True)
            else:
                exporter = ConsoleSpanExporter()
        except ImportError:  # pragma: no cover
            _LOG.debug("OTLP exporter missing; falling back to console.")
            exporter = ConsoleSpanExporter()
        provider.add_span_processor(BatchSpanProcessor(exporter))
        trace.set_tracer_provider(provider)
        return trace.get_tracer(self.service_name)


# ---------------------------------------------------------------------------
#  No-op span used when OTel is absent
# ---------------------------------------------------------------------------


@dataclass
class _NoopSpan:
    attrs: dict[str, Any] = field(default_factory=dict)

    def set_attribute(self, key: str, value: Any) -> None:
        self.attrs[key] = value

    def record_exception(self, exc: BaseException) -> None:  # pragma: no cover
        self.attrs["exception"] = repr(exc)

    def add_event(  # pragma: no cover
        self, name: str, attributes: dict[str, Any] | None = None
    ) -> None:
        self.attrs.setdefault("events", []).append(
            {"name": name, **(attributes or {})}
        )


# ---------------------------------------------------------------------------
#  Singleton
# ---------------------------------------------------------------------------


_TRACER_SINGLETON: TracingService | None = None


def get_tracer(
    *,
    service_name: str = "efe-platform",
    otel_endpoint: str | None = None,
) -> TracingService:
    """Return a process-wide tracer (first call seeds the singleton)."""
    global _TRACER_SINGLETON
    if _TRACER_SINGLETON is None:
        _TRACER_SINGLETON = TracingService(
            service_name=service_name, otel_endpoint=otel_endpoint
        )
    return _TRACER_SINGLETON


__all__ = ["TracingService", "get_tracer"]
