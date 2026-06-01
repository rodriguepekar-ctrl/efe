"""``efe_tool.repl`` — interactive REPL (Letter B).

See ``LETTER_B_ARCHITECTURE.md`` for the full specification.

Public surface:

    from efe_tool.repl import run_session
    run_session(cfg)

Internal layout:

* :mod:`efe_tool.repl.session`     — SessionState dataclass.
* :mod:`efe_tool.repl.parser`      — line → Token.
* :mod:`efe_tool.repl.prompt`      — prompt + banner + farewell.
* :mod:`efe_tool.repl.history`     — XDG-aware persistent history.
* :mod:`efe_tool.repl.completer`   — tab completion.
* :mod:`efe_tool.repl.render`      — human-format + delegates to cli.output.
* :mod:`efe_tool.repl.input_loop`  — orchestration; the heart of the REPL.
* :mod:`efe_tool.repl.commands`    — 26 ``:xxx`` commands in themed submodules.
"""

from __future__ import annotations

from .input_loop import run_session, run_query, read_line
from .session import SessionState

__all__ = ["run_session", "run_query", "read_line", "SessionState"]
