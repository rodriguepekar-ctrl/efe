"""Structured logging — structlog wrapper with a no-op fallback.

We prefer structlog because it gives us:

* JSON-formatted logs in production (``log_json=True``),
* tenant_id / run_id / worker_id carried via ``contextvars`` so every
  log line inside a request automatically carries the scope,
* bound loggers so call sites read ``log.info(...)`` without
  re-passing the context every time.

When structlog is not installed we fall back to the stdlib ``logging``
module — the call sites use :func:`get_logger` which returns an
object that implements ``debug/info/warning/error`` with the same
signature either way.
"""

from __future__ import annotations

import logging
import sys
from typing import Any, Protocol


class _Logger(Protocol):
    def debug(self, msg: str, **kwargs: Any) -> None:  # pragma: no cover
        ...

    def info(self, msg: str, **kwargs: Any) -> None:  # pragma: no cover
        ...

    def warning(self, msg: str, **kwargs: Any) -> None:  # pragma: no cover
        ...

    def error(self, msg: str, **kwargs: Any) -> None:  # pragma: no cover
        ...

    def bind(self, **kwargs: Any) -> _Logger:  # pragma: no cover
        ...


class _StdlibAdapter:
    """Adapter wrapping :class:`logging.Logger` to implement :class:`_Logger`."""

    def __init__(
        self, logger: logging.Logger, bound: dict[str, Any] | None = None,
    ) -> None:
        self._log = logger
        self._bound: dict[str, Any] = dict(bound or {})

    def _merge(self, kwargs: dict[str, Any]) -> dict[str, Any]:
        merged = dict(self._bound)
        merged.update(kwargs)
        return merged

    def debug(self, msg: str, **kwargs: Any) -> None:
        self._log.debug("%s %s", msg, self._merge(kwargs))

    def info(self, msg: str, **kwargs: Any) -> None:
        self._log.info("%s %s", msg, self._merge(kwargs))

    def warning(self, msg: str, **kwargs: Any) -> None:
        self._log.warning("%s %s", msg, self._merge(kwargs))

    def error(self, msg: str, **kwargs: Any) -> None:
        self._log.error("%s %s", msg, self._merge(kwargs))

    def bind(self, **kwargs: Any) -> _StdlibAdapter:
        return _StdlibAdapter(self._log, self._merge(kwargs))


def init_logging(*, level: str = "INFO", json: bool = True) -> None:
    """Configure the process-wide logger.

    We try structlog first; on failure we fall back to stdlib.
    """
    py_level = getattr(logging, level.upper(), logging.INFO)
    try:
        import structlog
    except ImportError:
        logging.basicConfig(
            level=py_level,
            format="%(asctime)s %(levelname)s %(name)s: %(message)s",
            stream=sys.stdout,
        )
        return

    structlog.configure(
        wrapper_class=structlog.make_filtering_bound_logger(py_level),
        processors=[
            structlog.contextvars.merge_contextvars,
            structlog.processors.add_log_level,
            structlog.processors.TimeStamper(fmt="iso"),
            structlog.processors.JSONRenderer()
            if json
            else structlog.dev.ConsoleRenderer(),
        ],
        cache_logger_on_first_use=True,
    )
    logging.basicConfig(level=py_level, stream=sys.stdout)


def get_logger(name: str = "efe_orch") -> _Logger:
    """Return a bound logger instance, using structlog when available."""
    try:
        import structlog
        logger: Any = structlog.get_logger(name)
        return logger  # type: ignore[no-any-return]
    except ImportError:
        return _StdlibAdapter(logging.getLogger(name))


__all__ = ["get_logger", "init_logging"]
