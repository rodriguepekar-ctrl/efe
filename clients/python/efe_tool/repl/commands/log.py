"""Logging commands: ``:log``, ``:quiet``, ``:verbose``.

Changing the log level at runtime reconfigures the CLI logger
singleton. The new level applies to every subsequent emission.
"""

from __future__ import annotations

from typing import Any

from ...cli.errors import UsageError
from ...cli.logging import setup_logging
from ...cli.metadata import LOG_LEVELS
from . import Command, register


def _apply_level(state: Any, level: str) -> None:
    state.log_level = level
    # The CLI logger reads state via ``effective_log_level``; setup
    # with a tiny adapter that exposes the attributes it expects.
    setup_logging(_LevelProxy(state))


class _LevelProxy:
    """Adapter so ``setup_logging`` can read from a SessionState."""
    def __init__(self, s: Any):
        self._s = s

    @property
    def effective_log_level(self) -> str:
        return self._s.log_level

    @property
    def log_format(self) -> str:
        return getattr(self._s.boot_cfg, "log_format", "text")

    @property
    def no_color(self) -> bool:
        return self._s.no_color

    @property
    def request_id(self) -> str:
        return self._s.session_id


def _cmd_log(state: Any, args: list[str], log: Any) -> str:
    if not args:
        return f"log level: {state.log_level}"
    level = args[0].lower()
    if level == "reset":
        level = state.boot_cfg.log_level
    if level == "off":
        # Map "off" to the highest level to suppress emissions.
        level = "error"
    if level not in LOG_LEVELS:
        raise UsageError(
            f":log unknown level {level!r}",
            hint=f"allowed: {', '.join(LOG_LEVELS)} | off | reset",
        )
    _apply_level(state, level)
    return f"log level: {level}"


register(Command(
    name="log",
    run=_cmd_log,
    doc="Set the stderr log level: debug | info | warn | error | off | reset",
    group="logging",
))


def _cmd_quiet(state: Any, args: list[str], log: Any) -> str:
    _apply_level(state, "error")
    return "log level: error (quiet)"


register(Command(
    name="quiet",
    run=_cmd_quiet,
    doc="Shortcut for ':log error'",
    group="logging",
))


def _cmd_verbose(state: Any, args: list[str], log: Any) -> str:
    _apply_level(state, "debug")
    return "log level: debug (verbose)"


register(Command(
    name="verbose",
    run=_cmd_verbose,
    doc="Shortcut for ':log debug'",
    group="logging",
))


__all__: list[str] = []
