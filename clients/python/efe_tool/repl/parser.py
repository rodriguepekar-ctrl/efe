"""Parsing of a raw input line into a typed token.

Three token kinds, matching the grammar in ``LETTER_B_ARCHITECTURE.md``
§5.2 and §23.A:

* :class:`EmptyToken`   — whitespace-only or empty line, produces no effect.
* :class:`CommandToken` — starts with ``:``. Carries a name and arg list.
* :class:`QueryToken`   — anything else, a text to classify.

Multi-line inputs (triple-quoted blocks and backslash continuations)
are assembled in :mod:`efe_tool.repl.input_loop` before being handed
to :func:`parse_line`. This module parses one *logical* line.
"""

from __future__ import annotations

import shlex
from dataclasses import dataclass
from typing import Union


@dataclass(frozen=True)
class EmptyToken:
    """Empty / whitespace-only logical line."""


@dataclass(frozen=True)
class CommandToken:
    """A REPL command (starts with ``:``)."""

    name: str
    args: tuple[str, ...] = ()

    def signature(self) -> str:
        if self.args:
            return f":{self.name} " + " ".join(self.args)
        return f":{self.name}"


@dataclass(frozen=True)
class QueryToken:
    """A classification query (free text)."""

    text: str


Token = Union[EmptyToken, CommandToken, QueryToken]


# ---------------------------------------------------------------------------
#  Parsing
# ---------------------------------------------------------------------------


def parse_line(line: str) -> Token:
    """Classify ``line`` into a token.

    Whitespace rules:

    * Stripped on both ends for command detection.
    * Preserved as-is in the query text, except for the final newline
      (which is normalised by :mod:`input_loop`).
    """
    stripped = line.strip()
    if not stripped:
        return EmptyToken()

    if stripped.startswith(":"):
        return _parse_command(stripped[1:])

    # Plain classification query — keep original line (with any internal
    # whitespace the user entered) so downstream rendering reflects it.
    return QueryToken(text=line.rstrip("\n\r"))


def _parse_command(body: str) -> CommandToken:
    """Parse the part after the leading ``:``.

    Uses shell-like splitting so users can quote arguments::

        :save "my file.json"
        :thresholds high 0.8 medium 0.6
    """
    if not body:
        raise ValueError("empty command after ':'")
    try:
        parts = shlex.split(body, posix=True)
    except ValueError as e:
        raise ValueError(f"malformed command arguments: {e}") from e
    if not parts:
        raise ValueError("empty command after ':'")
    name, *args = parts
    name = _normalise_name(name)
    return CommandToken(name=name, args=tuple(args))


def _normalise_name(raw: str) -> str:
    """Normalise a command name to its canonical form.

    * lower-case
    * underscores ↔ dashes tolerated (``:log-level`` == ``:log_level``)
    """
    return raw.lower().replace("-", "_")


__all__ = [
    "Token",
    "EmptyToken",
    "CommandToken",
    "QueryToken",
    "parse_line",
]
