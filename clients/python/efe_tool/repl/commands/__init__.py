"""Command registry and dispatcher for the REPL.

The registry is a flat dict from ``name`` (and each alias) to a
:class:`Command` record.  Loading this package imports the themed
sub-modules, each of which registers its commands via :func:`register`.

See ``LETTER_B_ARCHITECTURE.md`` §5.3 and §23.B for the canonical
list of 26 commands.
"""

from __future__ import annotations

import difflib
from dataclasses import dataclass, field
from typing import Any, Callable, Optional

from ...cli.errors import UsageError


# ---------------------------------------------------------------------------
#  Command record
# ---------------------------------------------------------------------------


CommandFn = Callable[..., Optional[str]]
"""Signature: ``fn(state, args: list[str], log) -> str | None``."""


@dataclass(frozen=True)
class Command:
    """A single REPL command definition."""

    name: str
    run: CommandFn
    doc: str
    aliases: tuple[str, ...] = ()
    needs_catalog: bool = False
    group: str = "session"


# ---------------------------------------------------------------------------
#  Registry
# ---------------------------------------------------------------------------


REGISTRY: dict[str, Command] = {}


def register(cmd: Command) -> None:
    """Insert ``cmd`` into the global registry under its name and
    every alias. Refuses to overwrite an existing entry (so import
    order bugs surface immediately)."""
    for key in (cmd.name, *cmd.aliases):
        if key in REGISTRY:
            raise RuntimeError(f"duplicate command registration: {key!r}")
        REGISTRY[key] = cmd


def canonical_names() -> list[str]:
    """Return the list of canonical names (no aliases), sorted."""
    seen: set[str] = set()
    out: list[str] = []
    for k, v in REGISTRY.items():
        if v.name in seen:
            continue
        seen.add(v.name)
        out.append(v.name)
    return sorted(out)


def all_keys() -> list[str]:
    """Every key (names + aliases) used by the completer."""
    return sorted(REGISTRY.keys())


# ---------------------------------------------------------------------------
#  Dispatch
# ---------------------------------------------------------------------------


def dispatch(
    name: str, args: list[str], state: Any, log: Any,
) -> Optional[str]:
    """Execute the command identified by ``name``.

    Returns the string to print on stdout (or ``None``).

    Raises:
        UsageError: unknown name, missing catalog for a command that
            requires one, or argument validation error inside the
            command.
    """
    cmd = REGISTRY.get(name)
    if cmd is None:
        suggest = difflib.get_close_matches(name, canonical_names(), n=1)
        hint = f"did you mean :{suggest[0]}?" if suggest else None
        raise UsageError(f"unknown command :{name}", hint=hint)

    if cmd.needs_catalog and state.catalog is None:
        raise UsageError(
            f":{name} requires a catalog",
            hint="use `:catalog PATH` to load one first",
        )

    return cmd.run(state, list(args), log)


# ---------------------------------------------------------------------------
#  Import every command sub-module so they self-register
# ---------------------------------------------------------------------------
#
# Keep this block at the *end* of __init__.py — the sub-modules import
# ``register`` and ``Command`` from us.


from . import session     as _session          # noqa: F401,E402
from . import catalog     as _catalog          # noqa: F401,E402
from . import network     as _network          # noqa: F401,E402
from . import tuning      as _tuning           # noqa: F401,E402
from . import output      as _output           # noqa: F401,E402
from . import log         as _log              # noqa: F401,E402
from . import classify    as _classify         # noqa: F401,E402


__all__ = [
    "Command",
    "REGISTRY",
    "register",
    "dispatch",
    "canonical_names",
    "all_keys",
]
