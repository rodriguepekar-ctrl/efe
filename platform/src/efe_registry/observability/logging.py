"""Structured logger factory — thin wrapper over structlog."""

from __future__ import annotations

import logging
from typing import Any


def get_logger(name: str) -> Any:
    try:
        import structlog
        return structlog.get_logger(name)
    except ImportError:  # pragma: no cover
        return logging.getLogger(name)


__all__ = ["get_logger"]
