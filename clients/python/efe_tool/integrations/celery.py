"""Celery integration for Letter D.

Exports:

* :func:`setup_efe_for_worker` — call from ``worker_ready`` signal.
* :func:`shutdown_efe`         — call from ``worker_shutdown`` signal.
* :func:`get_efe`              — retrieve the worker-scoped facade.

Critical: gRPC is **fork-unsafe**. When Celery uses the prefork pool
(the default), the facade MUST be constructed inside ``worker_ready``,
not at import time. See LETTER_D_ARCHITECTURE.md §7.4.
"""

from __future__ import annotations

import logging
import threading
from typing import Any, Optional

from . import EfeConfig, EfeFacade
from ._errors import EfeFacadeConfigError, EfeFacadeError


_log = logging.getLogger("efe_tool.integrations.celery")

_FACADE: Optional[EfeFacade] = None
_LOCK = threading.Lock()


def setup_efe_for_worker(
    *, config: Optional[EfeConfig] = None, preload: bool = True,
) -> EfeFacade:
    """Construct the facade for the current worker process.

    Intended to be wired to the ``worker_ready`` signal::

        from celery.signals import worker_ready
        from efe_tool.integrations.celery import setup_efe_for_worker

        @worker_ready.connect
        def _on_ready(**_):
            setup_efe_for_worker()

    Raises :class:`EfeFacadeConfigError` if the config is invalid.
    """
    global _FACADE
    with _LOCK:
        if _FACADE is not None:
            _log.warning("efe_facade_already_initialised_in_celery_worker")
            return _FACADE
        cfg = config or EfeConfig.from_env()
        facade = EfeFacade(cfg)
        if preload:
            try:
                facade.preload()
            except Exception:
                _log.exception("efe_facade_preload_failed_in_celery_worker")
                raise
        _FACADE = facade
        _log.info(
            "efe_facade_ready_in_celery_worker",
            extra={"endpoint": cfg.endpoint,
                   "catalogs": list(cfg.catalogs)},
        )
    return _FACADE


def shutdown_efe(*, timeout_s: float = 10.0) -> None:
    """Close the worker's facade. Wire to ``worker_shutdown``."""
    global _FACADE
    with _LOCK:
        facade = _FACADE
        _FACADE = None
    if facade is None:
        return
    try:
        facade.close(timeout_s=timeout_s)
    except Exception:  # pragma: no cover
        _log.exception("efe_facade_close_error_in_celery_worker")


def get_efe() -> EfeFacade:
    """Return the worker-scoped facade.

    Raises :class:`EfeFacadeError` if not initialised — always make
    sure to register :func:`setup_efe_for_worker` with the
    ``worker_ready`` signal before any task runs.
    """
    if _FACADE is None:
        raise EfeFacadeError(
            "EfeFacade not initialised in this Celery worker",
            hint="wire setup_efe_for_worker to the worker_ready signal",
        )
    return _FACADE


# ---------------------------------------------------------------------------
#  Test support
# ---------------------------------------------------------------------------


def reset_for_test() -> None:
    global _FACADE
    with _LOCK:
        if _FACADE is not None:
            try:
                _FACADE.close(timeout_s=1.0)
            except Exception:  # pragma: no cover
                pass
        _FACADE = None


def set_facade_for_test(facade: EfeFacade) -> None:
    global _FACADE
    with _LOCK:
        _FACADE = facade


__all__ = [
    "setup_efe_for_worker",
    "shutdown_efe",
    "get_efe",
    "reset_for_test",
    "set_facade_for_test",
]
