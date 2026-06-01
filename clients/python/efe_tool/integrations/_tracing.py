"""OpenTelemetry tracing integration for Letter D.

Soft dependency on ``opentelemetry-api``. When absent, the ``span``
context manager is a no-op — no host overhead. When present, each
``classify()`` call opens a span ``efe.classify`` with standard
attributes, child of the HTTP request span via auto-propagation.
"""

from __future__ import annotations

import contextlib
import importlib.util
from typing import Any, Iterator, Optional


_HAS_OTEL: bool = (
    importlib.util.find_spec("opentelemetry") is not None
)


class _NullSpan:
    """Spec-compatible null span — used when OpenTelemetry is unavailable."""

    def set_attribute(self, *_a: Any, **_kw: Any) -> None:
        return None

    def record_exception(self, *_a: Any, **_kw: Any) -> None:
        return None

    def set_status(self, *_a: Any, **_kw: Any) -> None:
        return None


class Tracer:
    """Wraps either the OpenTelemetry tracer or a null implementation."""

    def __init__(self, enabled: bool = True) -> None:
        self._enabled = bool(enabled and _HAS_OTEL)
        if self._enabled:
            from opentelemetry import trace
            self._tracer = trace.get_tracer("efe_tool.integrations")
        else:
            self._tracer = None

    @contextlib.contextmanager
    def span(
        self,
        name: str,
        *,
        attributes: Optional[dict[str, Any]] = None,
    ) -> Iterator[Any]:
        """Context manager opening a span. Attributes are set at entry."""
        if not self._enabled or self._tracer is None:
            yield _NullSpan()
            return
        with self._tracer.start_as_current_span(name) as span:
            if attributes:
                for k, v in attributes.items():
                    try:
                        span.set_attribute(k, v)
                    except Exception:  # pragma: no cover
                        pass
            yield span


__all__ = ["Tracer", "_HAS_OTEL"]
