"""Structured logger factory + PII redactor.

Any log record that slips a ``prompt_text`` / ``response_text`` /
``messages`` / ``content`` field is rejected by :func:`redact_dict`.
That is our defense-in-depth for invariant I3: even a bug that
forwards raw text into the logger cannot leak PII.
"""

from __future__ import annotations

import logging
from typing import Any

_BANNED_KEYS = frozenset({
    "prompt_text",
    "response_text",
    "messages",
    "content",
    "message_content",
})


def redact_dict(data: dict[str, Any]) -> dict[str, Any]:
    """Return a copy of ``data`` with any banned key replaced by ``[REDACTED]``.

    Applied recursively for nested dicts + lists of dicts. Any list of
    plain primitives is kept as-is.
    """
    out: dict[str, Any] = {}
    for k, v in data.items():
        if k in _BANNED_KEYS:
            out[k] = "[REDACTED]"
            continue
        if isinstance(v, dict):
            out[k] = redact_dict(v)
        elif isinstance(v, list):
            out[k] = [
                redact_dict(item) if isinstance(item, dict) else item
                for item in v
            ]
        else:
            out[k] = v
    return out


def get_logger(name: str) -> Any:
    try:
        import structlog
        return structlog.get_logger(name)
    except ImportError:  # pragma: no cover
        return logging.getLogger(name)


__all__ = ["get_logger", "redact_dict"]
