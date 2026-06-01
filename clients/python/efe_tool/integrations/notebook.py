"""Jupyter / notebook integration for Letter D.

Provides a singleton :class:`EfeFacade` that survives cell re-runs.
Registered with ``atexit`` so the gRPC channel is closed when the
kernel shuts down.

Usage inside a cell::

    from efe_tool.integrations.notebook import efe_notebook_singleton
    efe = efe_notebook_singleton()
    verdict = efe.classify("J'ai perdu ma carte", catalog="support_triage")
    verdict
"""

from __future__ import annotations

import atexit
import logging
import threading
from typing import Optional

from . import EfeConfig, EfeFacade


_log = logging.getLogger("efe_tool.integrations.notebook")

_FACADE: Optional[EfeFacade] = None
_LOCK = threading.Lock()


def efe_notebook_singleton(
    *, config: Optional[EfeConfig] = None, preload: bool = True,
) -> EfeFacade:
    """Return the kernel-wide facade, constructing it on first call.

    ``preload=True`` blocks on embedder + catalogs — expected in a
    notebook (you want feedback on the first cell that creates it).
    """
    global _FACADE
    with _LOCK:
        if _FACADE is not None:
            return _FACADE
        cfg = config or EfeConfig.from_env()
        facade = EfeFacade(cfg)
        if preload:
            facade.preload()
        _FACADE = facade
        atexit.register(_atexit_close)
        _log.info(
            "efe_facade_ready_in_notebook",
            extra={"endpoint": cfg.endpoint,
                   "catalogs": list(cfg.catalogs)},
        )
    return _FACADE


def _atexit_close() -> None:
    global _FACADE
    facade = _FACADE
    if facade is None:
        return
    try:
        facade.close(timeout_s=5.0)
    except Exception:  # pragma: no cover
        _log.exception("efe_facade_atexit_close_error")
    _FACADE = None


def reset_for_test() -> None:
    """Drop the notebook singleton — used by test tear-down only."""
    global _FACADE
    with _LOCK:
        if _FACADE is not None:
            try:
                _FACADE.close(timeout_s=1.0)
            except Exception:  # pragma: no cover
                pass
        _FACADE = None


__all__ = ["efe_notebook_singleton", "reset_for_test"]
