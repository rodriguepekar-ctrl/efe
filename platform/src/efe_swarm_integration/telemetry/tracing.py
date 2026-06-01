"""OTel span factory — with a contextmanager fallback."""

from __future__ import annotations

import contextlib
from dataclasses import dataclass
from typing import Any, Iterator

__all__ = ["Tracer"]


@dataclass(slots=True)
class _NullSpan:
    attributes: dict[str, Any]

    def set_attribute(self, key: str, value: Any) -> None:
        self.attributes[key] = value


@dataclass(slots=True)
class Tracer:
    """Minimal tracing facade.

    When ``opentelemetry`` is installed, :meth:`start_span` wraps the
    real tracer ; otherwise it yields a :class:`_NullSpan` so user code
    can always call ``span.set_attribute(...)`` without guards.
    """

    service_name: str = "efe_swarm_integration"

    @contextlib.contextmanager
    def start_span(self, name: str) -> Iterator[Any]:
        try:
            from opentelemetry import trace  # type: ignore[import-untyped]

            tracer = trace.get_tracer(self.service_name)
            with tracer.start_as_current_span(name) as span:
                yield span
        except Exception:  # pragma: no cover
            yield _NullSpan(attributes={})
