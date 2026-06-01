"""Structured logger for the ``efe-admin`` CLI.

Two emission formats are supported:

* ``text`` — single-line, human-friendly, coloured if stderr is a TTY.
* ``json`` — one JSON object per line (NDJSON). Keys are stable.

All logs go to **stderr**. That is a hard invariant (§8.3 of the
architecture doc): stdout is reserved for the subcommand's result.

Public surface:

    setup_logging(cfg) -> Logger
    get_logger() -> Logger

The logger exposes four level methods: ``debug``, ``info``, ``warn``,
``error``. Each takes a *named* event (a short, snake_case identifier)
plus arbitrary keyword fields.

Design notes
------------

* We do **not** use :mod:`logging` from the standard library, to keep
  the log format tightly under our control and to avoid the global
  root-logger state sprawl.

* Colors use ANSI escape sequences directly (no :mod:`colorama`
  dependency on POSIX). Windows terminals usually interpret ANSI
  since Windows 10 1909.

* The logger is *process-scoped*: a single instance in module state.
  A test helper :func:`reset_logger_for_test` is provided.
"""

from __future__ import annotations

import json
import sys
import time
from dataclasses import dataclass
from typing import Any, Optional, TextIO

from .metadata import LOG_FORMATS, LOG_LEVELS


# ---------------------------------------------------------------------------
# Level numeric ordering
# ---------------------------------------------------------------------------

_LEVELS: dict[str, int] = {
    "debug": 10,
    "info":  20,
    "warn":  30,
    "error": 40,
}


# ANSI color codes — used only when TTY + color allowed.
_COLORS: dict[str, str] = {
    "debug": "\x1b[2m",      # dim
    "info":  "\x1b[34m",     # blue
    "warn":  "\x1b[33m",     # yellow
    "error": "\x1b[31m",     # red
    "reset": "\x1b[0m",
    "key":   "\x1b[36m",     # cyan — for field names
}


# ---------------------------------------------------------------------------
# Logger
# ---------------------------------------------------------------------------


@dataclass
class Logger:
    """Structured logger. Single instance per process."""

    level: str = "info"
    format: str = "text"
    stream: TextIO = sys.stderr
    request_id: Optional[str] = None
    use_color: bool = False

    # ------------------------------------------------------------------
    #  Level methods
    # ------------------------------------------------------------------

    def debug(self, event: str, **fields: Any) -> None:
        self._emit("debug", event, fields)

    def info(self, event: str, **fields: Any) -> None:
        self._emit("info", event, fields)

    def warn(self, event: str, **fields: Any) -> None:
        self._emit("warn", event, fields)

    def error(self, event: str, **fields: Any) -> None:
        self._emit("error", event, fields)

    # ------------------------------------------------------------------
    #  Core emission
    # ------------------------------------------------------------------

    def _emit(self, level: str, event: str, fields: dict[str, Any]) -> None:
        if _LEVELS[level] < _LEVELS[self.level]:
            return
        if self.request_id is not None and "request_id" not in fields:
            fields["request_id"] = self.request_id

        if self.format == "json":
            line = self._format_json(level, event, fields)
        else:
            line = self._format_text(level, event, fields)

        try:
            self.stream.write(line + "\n")
            self.stream.flush()
        except (BrokenPipeError, OSError):  # pragma: no cover
            # stderr broken — nothing to do, don't recurse.
            pass

    # ------------------------------------------------------------------
    #  Formatters
    # ------------------------------------------------------------------

    def _format_json(self, level: str, event: str, fields: dict[str, Any]) -> str:
        payload: dict[str, Any] = {
            "ts": _iso_now(),
            "lvl": level,
            "evt": event,
        }
        payload.update(fields)
        # Coerce non-JSON-serialisable values defensively so a bad
        # field type never crashes the logger itself.
        return json.dumps(payload, default=_safe_default, ensure_ascii=False)

    def _format_text(self, level: str, event: str, fields: dict[str, Any]) -> str:
        ts = _iso_now()
        if self.use_color:
            lvl_s = f"{_COLORS[level]}{level.upper():5s}{_COLORS['reset']}"
        else:
            lvl_s = f"{level.upper():5s}"

        extras: list[str] = []
        for k, v in fields.items():
            k_s = (f"{_COLORS['key']}{k}{_COLORS['reset']}"
                   if self.use_color else k)
            extras.append(f"{k_s}={_compact_value(v)}")
        return f"{ts}  {lvl_s}  {event} " + " ".join(extras)


# ---------------------------------------------------------------------------
#  Module-level instance
# ---------------------------------------------------------------------------

_INSTANCE: Optional[Logger] = None


def setup_logging(cfg: Any, *, stream: TextIO | None = None) -> Logger:
    """Configure (or reconfigure) the process-wide logger.

    Safe to call multiple times; each call replaces the previous
    instance. Returns the new instance for convenience.
    """
    global _INSTANCE
    level = _pick_attr(cfg, "effective_log_level", "log_level", default="info")
    fmt   = _pick_attr(cfg, "log_format", default="text")
    no_color = bool(_pick_attr(cfg, "no_color", default=False))
    request_id = _pick_attr(cfg, "request_id", default=None)
    target = stream or sys.stderr

    use_color = (not no_color) and _stream_is_tty(target)

    if level not in _LEVELS:
        # Defensive — validate_config should have caught this. But we
        # don't want the logger itself to fail here.
        level = "info"
    if fmt not in LOG_FORMATS:
        fmt = "text"

    _INSTANCE = Logger(
        level=level,
        format=fmt,
        stream=target,
        request_id=request_id,
        use_color=use_color,
    )
    return _INSTANCE


def get_logger() -> Logger:
    """Return the current logger, or a sensible default if not set up yet.

    A CLI should always call :func:`setup_logging` once at startup;
    the fallback here exists only to make unit tests of helpers that
    log defensively painless.
    """
    global _INSTANCE
    if _INSTANCE is None:
        _INSTANCE = Logger()
    return _INSTANCE


def reset_logger_for_test() -> None:  # pragma: no cover — test helper
    """Clear the module-level singleton. Only used by the pytest fixture."""
    global _INSTANCE
    _INSTANCE = None


# ---------------------------------------------------------------------------
#  Internals
# ---------------------------------------------------------------------------


def _iso_now() -> str:
    """Millisecond-precision UTC timestamp."""
    t = time.time()
    ms = int((t - int(t)) * 1000)
    gm = time.gmtime(t)
    return (
        f"{gm.tm_year:04d}-{gm.tm_mon:02d}-{gm.tm_mday:02d}"
        f"T{gm.tm_hour:02d}:{gm.tm_min:02d}:{gm.tm_sec:02d}.{ms:03d}Z"
    )


def _compact_value(v: Any) -> str:
    """Render a field value for the text format, compact and safe."""
    if isinstance(v, str):
        if any(c.isspace() for c in v) or v == "":
            return json.dumps(v, ensure_ascii=False)
        return v
    if isinstance(v, (int, float, bool)) or v is None:
        return json.dumps(v)
    try:
        return json.dumps(v, default=_safe_default, ensure_ascii=False)
    except Exception:  # pragma: no cover
        return repr(v)


def _safe_default(o: Any) -> Any:
    """JSON serialiser fallback for non-trivial objects."""
    try:
        return str(o)
    except Exception:  # pragma: no cover
        return "<unrepresentable>"


def _stream_is_tty(stream: TextIO) -> bool:
    isatty = getattr(stream, "isatty", None)
    if not callable(isatty):
        return False
    try:
        return bool(isatty())
    except Exception:  # pragma: no cover
        return False


def _pick_attr(obj: Any, *names: str, default: Any = None) -> Any:
    """Return the first non-missing attribute on ``obj`` among ``names``."""
    for n in names:
        if hasattr(obj, n):
            v = getattr(obj, n)
            if v is not None:
                return v
    return default


# ---------------------------------------------------------------------------
#  Public symbol list
# ---------------------------------------------------------------------------

__all__ = [
    "Logger",
    "setup_logging",
    "get_logger",
    "reset_logger_for_test",
    "LOG_LEVELS",
]
