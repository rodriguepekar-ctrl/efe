"""Explicit classify command: ``:classify TEXT``.

Used when the text to classify starts with ``:`` (which would
otherwise be parsed as a command) or when the user wants to emphasize
the distinction in a demo.

Delegates to :func:`efe_tool.repl.input_loop.run_query` via an import
at call-time to avoid circular imports (input_loop imports commands).
"""

from __future__ import annotations

from typing import Any

from ...cli.errors import UsageError
from . import Command, register


def _cmd_classify(state: Any, args: list[str], log: Any) -> str | None:
    if not args:
        raise UsageError(":classify expects a text argument")
    text = " ".join(args)
    from ..input_loop import run_query
    return run_query(text, state, log)


register(Command(
    name="classify",
    run=_cmd_classify,
    doc="Force classification of TEXT (useful if text starts with ':')",
    needs_catalog=True,
    group="classify",
))
