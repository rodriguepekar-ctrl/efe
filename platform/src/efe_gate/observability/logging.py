"""Structured logging — structlog + stdlib fallback."""

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
    def __init__(
        self, logger: logging.Logger,
        bound: dict[str, Any] | None = None,
    ) -> None:
        self._log = logger
        self._bound = dict(bound or {})

    def _merge(self, kwargs: dict[str, Any]) -> dict[str, Any]:
        out = dict(self._bound)
        out.update(kwargs)
        return out

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


def get_logger(name: str = "efe_gate") -> _Logger:
    try:
        import structlog
        logger: Any = structlog.get_logger(name)
        return logger  # type: ignore[no-any-return]
    except ImportError:
        return _StdlibAdapter(logging.getLogger(name))


__all__ = ["get_logger", "init_logging"]
