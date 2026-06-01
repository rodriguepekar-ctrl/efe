"""Output commands: ``:format``, ``:pretty``, ``:diagnostics``."""

from __future__ import annotations

from typing import Any

from ...cli.errors import UsageError
from ...cli.metadata import OUTPUT_FORMATS
from . import Command, register


_VALID_FORMATS = ("human",) + OUTPUT_FORMATS  # human + json/text/table


def _cmd_format(state: Any, args: list[str], log: Any) -> str:
    if not args:
        return f"format: {state.output_format}"
    new = args[0].lower()
    if new == "reset":
        state.output_format = "human"
        return "format reset to human"
    if new not in _VALID_FORMATS:
        raise UsageError(
            f":format unknown {new!r}",
            hint=f"allowed: {', '.join(_VALID_FORMATS)}",
        )
    state.output_format = new
    return f"format: {new}"


register(Command(
    name="format",
    run=_cmd_format,
    doc="Switch stdout format: human | text | json | table",
    group="output",
))


def _cmd_pretty(state: Any, args: list[str], log: Any) -> str:
    if not args:
        return f"pretty: {'on' if state.pretty else 'off'}"
    if args[0] == "reset":
        state.pretty = state.boot_cfg.pretty
        return f"pretty reset to {'on' if state.pretty else 'off'}"
    from .tuning import _parse_on_off
    state.pretty = _parse_on_off(args[0], name="pretty")
    return f"pretty: {'on' if state.pretty else 'off'}"


register(Command(
    name="pretty",
    run=_cmd_pretty,
    doc="Toggle pretty JSON output (on|off|reset)",
    group="output",
))


def _cmd_diagnostics(state: Any, args: list[str], log: Any) -> str:
    if not args:
        return f"diagnostics: {'on' if state.diagnostics else 'off'}"
    if args[0] == "reset":
        state.diagnostics = state.boot_cfg.with_diagnostics
        return f"diagnostics reset to {'on' if state.diagnostics else 'off'}"
    from .tuning import _parse_on_off
    state.diagnostics = _parse_on_off(args[0], name="diagnostics")
    return f"diagnostics: {'on' if state.diagnostics else 'off'}"


register(Command(
    name="diagnostics",
    run=_cmd_diagnostics,
    doc="Toggle sub_scores display (on|off|reset)",
    group="output",
))


__all__: list[str] = []
