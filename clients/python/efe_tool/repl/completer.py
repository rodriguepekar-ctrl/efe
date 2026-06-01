"""Tab completion for the EFE REPL.

The completion algorithm covers three situations:

1. **Beginning of the line starts with ``:``** — propose command
   names (and their short aliases) that match the prefix.
2. **Line starts with ``:<name> <something>``** and the command
   takes a *path* or *enum* argument — delegate to filename
   completion or offer the enum values.
3. **Line is free text** — no completion (language-level suggestions
   would be noise).

We register a :class:`Completer` instance as the global completer via
``readline.set_completer(...)``. Python's readline calls the completer
for each candidate at index ``state``, until we return ``None``.
"""

from __future__ import annotations

import os
import shlex
from pathlib import Path
from typing import Iterable, Optional


# ---------------------------------------------------------------------------
#  Completer
# ---------------------------------------------------------------------------


class Completer:
    """Readline completer for the REPL.

    The command registry is injected at construction so tests can
    supply a minimal subset without importing the full command package.
    """

    def __init__(
        self,
        command_names: Iterable[str],
        enum_values: Optional[dict[str, tuple[str, ...]]] = None,
        path_args: Optional[Iterable[str]] = None,
    ) -> None:
        # Normalize to a sorted tuple so completions are stable.
        self._commands: tuple[str, ...] = tuple(sorted(set(command_names)))
        self._enum_values: dict[str, tuple[str, ...]] = dict(enum_values or {})
        self._path_args: frozenset[str] = frozenset(path_args or ())
        self._matches: list[str] = []

    # ------------------------------------------------------------------
    #  readline interface
    # ------------------------------------------------------------------

    def complete(self, text: str, state: int) -> Optional[str]:
        """The function readline calls.

        It's called repeatedly with ``state=0, 1, 2, …`` until we
        return ``None``. On ``state == 0`` we build the candidates;
        on subsequent calls we serve them in order.
        """
        if state == 0:
            self._matches = list(self._candidates(text))
        if state < len(self._matches):
            return self._matches[state]
        return None

    # ------------------------------------------------------------------
    #  Candidate generation
    # ------------------------------------------------------------------

    def _candidates(self, text: str) -> Iterable[str]:
        # readline gives us ``text`` as the word being completed (not the
        # full line). We need the full line for context; readline exposes
        # it via ``readline.get_line_buffer()``.
        try:
            import readline  # type: ignore
            buf = readline.get_line_buffer()
            begin = readline.get_begidx()
        except ImportError:  # pragma: no cover
            buf = text
            begin = 0

        if buf.startswith(":") and begin == 0:
            # Completing the command *name*. Strip the leading colon
            # from the comparison.
            prefix = text[1:] if text.startswith(":") else text
            out = [f":{n}" for n in self._commands if n.startswith(prefix)]
            return out

        if buf.startswith(":"):
            # Completing an argument of the command.
            head = buf[1:begin].strip()
            parts = _safe_split(head)
            if not parts:
                return []
            name = parts[0]
            # Enum argument?
            if name in self._enum_values:
                values = self._enum_values[name]
                return [v for v in values if v.startswith(text)]
            # Path argument?
            if name in self._path_args:
                return _complete_path(text)
            return []

        # Free text — no completion.
        return []


# ---------------------------------------------------------------------------
#  Helpers
# ---------------------------------------------------------------------------


def _safe_split(s: str) -> list[str]:
    try:
        return shlex.split(s, posix=True)
    except ValueError:
        return s.split()


def _complete_path(text: str) -> list[str]:
    """Path completion mirroring the defaults of bash.

    Expands ``~`` and resolves against ``cwd``. Appends ``/`` to
    directories so the user can keep typing without a manual slash.
    """
    expanded = os.path.expanduser(text) if text else ""
    directory = os.path.dirname(expanded) or "."
    prefix = os.path.basename(expanded)
    try:
        entries = os.listdir(directory)
    except OSError:
        return []
    out: list[str] = []
    head = os.path.dirname(text)   # original (may contain ``~``) for display
    for name in entries:
        if not name.startswith(prefix):
            continue
        full = os.path.join(directory, name)
        display_head = (head + "/") if head and not head.endswith("/") else head
        display = display_head + name if display_head else name
        if os.path.isdir(full):
            out.append(display + "/")
        else:
            out.append(display)
    return sorted(out)


# ---------------------------------------------------------------------------
#  Install / uninstall
# ---------------------------------------------------------------------------


def install(completer: Completer) -> bool:
    """Register ``completer`` with ``readline``. Returns ``False`` if
    readline is unavailable on this platform (Windows without
    pyreadline3)."""
    try:
        import readline  # type: ignore
    except ImportError:  # pragma: no cover
        return False
    try:
        readline.set_completer(completer.complete)
        # On macOS the default readline is libedit, which uses a
        # different bind syntax.
        doc = getattr(readline, "__doc__", "") or ""
        if "libedit" in doc:
            readline.parse_and_bind("bind ^I rl_complete")
        else:
            readline.parse_and_bind("tab: complete")
        # Word boundary: ``:`` must be part of a word so the user can
        # complete ``:re`` into ``:reload``.
        readline.set_completer_delims(
            readline.get_completer_delims().replace(":", "")
        )
        return True
    except Exception:  # pragma: no cover
        return False


def uninstall() -> None:
    try:
        import readline  # type: ignore
        readline.set_completer(None)
    except Exception:  # pragma: no cover
        pass


__all__ = ["Completer", "install", "uninstall"]
